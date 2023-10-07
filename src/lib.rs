use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Seek, Write},
    os::unix::prelude::MetadataExt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
struct State {
    pub offset: u64,
    pub inode: u64,
}

#[derive(Error, Debug)]
pub enum StateSerdeError {
    #[error("while working with underlying file")]
    IO(#[from] std::io::Error),

    #[error("while trying to (de)serialize state")]
    Serde(#[from] bincode::Error),
}

impl State {
    pub fn load(file: &mut File) -> Result<Self, StateSerdeError> {
        file.rewind()?;
        let state = bincode::deserialize_from(file)?;
        Ok(state)
    }

    pub fn persist(&self, file: &mut File) -> std::io::Result<()> {
        file.rewind()?;
        match bincode::serialize_into(file, self) {
            Ok(_) => {}
            Err(e) => match *e {
                bincode::ErrorKind::Io(ioerr) => return Err(ioerr),
                _ => unreachable!(),
            },
        }
        Ok(())
    }
}

enum Files {
    Current {
        file: BufReader<File>,
        inode: u64,
    },

    CurrentAndPrevious {
        previous: BufReader<File>,
        previous_inode: u64,
        previous_size: u64,
        current: BufReader<File>,
        current_inode: u64,
    },
}

pub struct TrackedReader {
    files: Files,
    buffer: Vec<u8>,
    global_offset: u64,
    registry: File,
}

#[derive(Error, Debug)]
pub enum TrackedReaderError {
    #[error("while working with underlying file")]
    IO(#[from] std::io::Error),
    #[error("while working with persistent state storage")]
    Persistence(#[from] StateSerdeError),
    #[error("trying to resolve logrotated file")]
    RotationResolution(String),
}

impl TrackedReader {
    pub fn with_capacity(
        filepath: &str,
        registry: &str,
        capacity: usize,
    ) -> Result<Self, TrackedReaderError> {
        let (state, registry) = maybe_read_state(Path::new(registry))?;
        let files = open_files(PathBuf::from(filepath), state)?;

        Ok(Self {
            files,
            buffer: Vec::with_capacity(capacity),
            global_offset: state.map(|state| state.offset).unwrap_or_default(),
            registry,
        })
    }

    pub fn new(filepath: &str, registry: &str) -> Result<Self, TrackedReaderError> {
        Self::with_capacity(filepath, registry, 8 * 1024)
    }

    fn extract_state(&self) -> State {
        match &self.files {
            Files::Current { inode, .. } => State {
                offset: self.global_offset,
                inode: *inode,
            },
            Files::CurrentAndPrevious {
                previous_inode,
                previous_size,
                current_inode,
                ..
            } => {
                if self.global_offset >= *previous_size {
                    State {
                        offset: self.global_offset - previous_size,
                        inode: *current_inode,
                    }
                } else {
                    State {
                        offset: self.global_offset,
                        inode: *previous_inode,
                    }
                }
            }
        }
    }
}

fn maybe_read_state(path: &Path) -> Result<(Option<State>, File), TrackedReaderError> {
    if !path.exists() {
        return Ok((
            None,
            File::options()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path)?,
        ));
    }

    let mut file = File::options().read(true).write(true).open(path)?;
    let state = State::load(&mut file)?;
    Ok((Some(state), file))
}

fn open_files(path: PathBuf, state: Option<State>) -> Result<Files, TrackedReaderError> {
    match state {
        None => {
            let current_file_meta = std::fs::metadata(&path)?;
            let reader = BufReader::new(File::open(path)?);
            Ok(Files::Current {
                file: reader,
                inode: current_file_meta.ino(),
            })
        }
        Some(state) => {
            let current_file_meta = std::fs::metadata(&path)?;
            let mut current_file = BufReader::new(File::open(&path)?);
            current_file.seek(std::io::SeekFrom::Start(state.offset))?;
            if current_file_meta.ino() == state.inode {
                Ok(Files::Current {
                    file: current_file,
                    inode: state.inode,
                })
            } else {
                let older_path = get_rotated_filename(&path);
                let older_path_meta = std::fs::metadata(&older_path)?;

                if older_path_meta.ino() != state.inode {
                    return Err(TrackedReaderError::RotationResolution(
                        "failed to resolve rotated file: previous file's inode does not match"
                            .to_string(),
                    ));
                }
                let mut older = BufReader::new(File::open(older_path)?);
                older.seek(std::io::SeekFrom::Start(state.offset))?;
                Ok(Files::CurrentAndPrevious {
                    previous: older,
                    previous_inode: older_path_meta.ino(),
                    previous_size: older_path_meta.size(),
                    current: current_file,
                    current_inode: current_file_meta.ino(),
                })
            }
        }
    }
}

fn get_rotated_filename(path: &Path) -> PathBuf {
    append_ext("1", path.to_path_buf())
}

pub fn append_ext(ext: impl AsRef<std::ffi::OsStr>, path: PathBuf) -> PathBuf {
    let mut os_string: std::ffi::OsString = path.into();
    os_string.push(".");
    os_string.push(ext.as_ref());
    os_string.into()
}

impl Read for TrackedReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match &mut self.files {
            Files::Current { file, .. } => {
                let read = file.read(buf)?;
                self.global_offset += read as u64;
                Ok(read)
            }
            Files::CurrentAndPrevious {
                previous,
                previous_inode,
                previous_size,
                current,
                current_inode,
            } => todo!(),
        }
    }
}

impl BufRead for TrackedReader {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        match &mut self.files {
            Files::Current { file: file, .. } => {
                let buffer = file.fill_buf()?;
                self.buffer.clear();
                self.buffer.write_all(buffer)?;
                Ok(self.buffer.as_slice())
            }
            Files::CurrentAndPrevious {
                previous, current, ..
            } => todo!(),
        }
    }

    fn consume(&mut self, amt: usize) {
        match &mut self.files {
            Files::Current { file: file, .. } => {
                file.consume(amt);
                self.global_offset += amt as u64;
            }
            Files::CurrentAndPrevious {
                previous,
                previous_inode,
                previous_size,
                current,
                current_inode,
            } => todo!(),
        }
    }
}

impl Seek for TrackedReader {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        match &mut self.files {
            Files::Current { file, .. } => {
                let new_pos = file.seek(pos)?;
                self.global_offset = new_pos;
                Ok(new_pos)
            }
            Files::CurrentAndPrevious {
                previous,
                previous_inode,
                previous_size,
                current,
                current_inode,
            } => todo!(),
        }
    }
}

impl Drop for TrackedReader {
    fn drop(&mut self) {
        self.extract_state().persist(&mut self.registry).unwrap();
    }
}
