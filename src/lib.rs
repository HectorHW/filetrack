#![doc = include_str!("../README.md")]

use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Seek},
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

/// possible errors that could happen while working with persistent state storage
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

/// Structure that implements `Read`, `ReadBuf` and `Seek` while working with persistent offset in up to two underlying files.
/// External file is used to persist offset across restarts.
///
/// ## Cleanup
///
/// There are two distinct ways to perform cleanup for this structure:
///
/// * **explicit** by calling `.close()`. This will allow you to handle any errors that may happen in the process
/// * **implicitly** by relying on `Drop`. Note that errors generated while working with the filesystem cannot be handled and will
/// cause a panic in this case.
pub struct TrackedReader {
    files: Files,
    global_offset: u64,
    registry: File,
    already_freed: bool,
}

/// possible errors that could happen while working with `TrackedReader`
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
    /// Creates a new `TrackedReader` possibly loading current offset from a registry file. On a first execution registry file most
    /// likely will not exist and in that case it will be created with zero offset.
    ///
    /// # Arguments
    ///
    /// * `filepath` - a path to log file to be read. `TrackedReader` will additionally search for logrotated file under `{filepath}.1`
    /// * `registry` - path to registry file used to persist offset and other metadata
    pub fn new(filepath: &str, registry: &str) -> Result<Self, TrackedReaderError> {
        let (state, registry) = maybe_read_state(Path::new(registry))?;
        let files = open_files(PathBuf::from(filepath), state)?;
        let initial_offset = state.map(|state| state.offset).unwrap_or_default();
        let mut reader = Self {
            files,
            global_offset: initial_offset,
            registry,
            already_freed: false,
        };
        reader.seek(std::io::SeekFrom::Start(initial_offset))?;
        Ok(reader)
    }

    /// Explicitly save current state into registry file and return any errors generated
    pub fn persist(&mut self) -> std::io::Result<()> {
        self.extract_state().persist(&mut self.registry)
    }

    /// Explicitly finalize structure, returning any errors that were produced in the process. Alternative to relying on `Drop`.
    pub fn close(mut self) -> std::io::Result<()> {
        self.persist()?;
        self.already_freed = true;
        Ok(())
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
                let older = BufReader::new(File::open(older_path)?);
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

fn append_ext(ext: impl AsRef<std::ffi::OsStr>, path: PathBuf) -> PathBuf {
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
                previous_size,
                current,
                ..
            } => {
                // because we read forward, we can use current offset to determine file we are in
                let read = if self.global_offset < *previous_size {
                    previous.read(buf)?
                } else {
                    current.read(buf)?
                };
                self.global_offset += read as u64;
                Ok(read)
            }
        }
    }
}

impl BufRead for TrackedReader {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        match &mut self.files {
            Files::Current { file, .. } => file.fill_buf(),
            Files::CurrentAndPrevious {
                previous,
                current,
                previous_size,
                ..
            } => {
                // firstly, we determine if we are reading from the first or the second file
                if self.global_offset < *previous_size {
                    // then we can simply ask appropriate file to fill the buffer
                    //
                    // previous file is not over yet because we've read less than size and we trust
                    // that previous file won't change
                    previous.fill_buf()
                } else {
                    current.fill_buf()
                }
                // because of consume, we will shift file pointer into correct location
            }
        }
    }

    fn consume(&mut self, amt: usize) {
        match &mut self.files {
            Files::Current { file, .. } => {
                file.consume(amt);
                self.global_offset += amt as u64;
            }
            Files::CurrentAndPrevious {
                previous,
                previous_size,
                current,
                ..
            } => {
                // the proper file returned some nonzero buf result, so we just need to forward `amt` to it
                if self.global_offset < *previous_size {
                    previous.consume(amt);
                } else {
                    current.consume(amt)
                }
                self.global_offset += amt as u64;
            }
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
                previous_size,
                current,
                ..
            } => match pos {
                std::io::SeekFrom::Start(offset) => {
                    let previous_size = *previous_size;
                    self.global_offset = offset;
                    if offset > previous_size {
                        let offset_in_new =
                            current.seek(std::io::SeekFrom::Start(offset - previous_size))?;
                        Ok(previous_size + offset_in_new)
                    } else {
                        // here we seek both files - because we initially read from the first one,
                        // second should also be reset to read from start when we get here
                        current.rewind()?;
                        previous.seek(pos)
                    }
                }
                std::io::SeekFrom::Current(offset) => {
                    let new_position = self.global_offset as i64 + offset;
                    if new_position < 0 {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "negative real offset after seek",
                        ));
                    }
                    let new_position = new_position as u64;
                    self.seek(std::io::SeekFrom::Start(new_position))
                }
                std::io::SeekFrom::End(offset) => {
                    //lets suppose that both files do not change for the duration of seek
                    let current_file_size = current.seek(std::io::SeekFrom::End(0))?;
                    let total_offset = (*previous_size + current_file_size) as i64 + offset;
                    if total_offset < 0 {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "negative real offset after seek",
                        ));
                    }
                    let total_offset = total_offset as u64;
                    self.seek(std::io::SeekFrom::Start(total_offset))
                }
            },
        }
    }
}

/// Executes destructor. If `.close()` was not called previously, will write state to disk, possibly panicking if any error happens.
/// If panic is not what you want, use `.close()` and handle errors manually instead.
impl Drop for TrackedReader {
    fn drop(&mut self) {
        if !self.already_freed {
            self.persist().unwrap()
        }
    }
}
