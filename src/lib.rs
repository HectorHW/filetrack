#![doc = include_str!("../README.md")]

mod multireader;

pub use multireader::Multireader;

use std::{
    fs::File,
    io::{BufReader, Seek},
    ops::{Deref, DerefMut},
    os::unix::prelude::MetadataExt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct State {
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
    inner: Multireader<BufReader<File>>,
    inodes: Vec<u64>,
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
    pub fn new(
        filepath: impl AsRef<Path>,
        registry: impl AsRef<Path>,
    ) -> Result<Self, TrackedReaderError> {
        let state_from_disk = maybe_read_state(registry.as_ref())?;
        let (files, inodes) = open_files(PathBuf::from(filepath.as_ref()), state_from_disk)?;
        let initial_offset = state_from_disk
            .map(|state| state.offset)
            .unwrap_or_default();
        // now that we know that open_files did not fail, we can create registry file
        let registry = open_state_file(registry)?;
        let mut reader = Self {
            inner: Multireader::new(files)?,
            inodes,
            registry,
            already_freed: false,
        };
        if let Some(state) = state_from_disk {
            reader.seek(std::io::SeekFrom::Start(state.offset))?;
        } else {
            // If state did not exist previously, registry file is created empty. We should additionally initialize file content.
            // This will make struct work correctly even if close/Drop will never happen (eg in case of mem::forget).
            reader.persist()?;
        }
        reader.seek(std::io::SeekFrom::Start(initial_offset))?;

        Ok(reader)
    }

    /// Explicitly save current state into registry file and return any errors generated
    pub fn persist(&mut self) -> std::io::Result<()> {
        self.get_persistent_state().persist(&mut self.registry)
    }

    /// Explicitly finalize structure, returning any errors that were produced in the process. Alternative to relying on `Drop`.
    pub fn close(mut self) -> std::io::Result<()> {
        self.persist()?;
        self.already_freed = true;
        Ok(())
    }

    /// Get current state for possible manual handling
    pub fn get_persistent_state(&self) -> State {
        if self.len() == 1 {
            State {
                offset: self.get_global_offset(),
                inode: self.inodes[0],
            }
        } else {
            State {
                offset: self.get_local_offset(),
                inode: self.inodes[self.get_current_item_index()],
            }
        }
    }
}

fn maybe_read_state(path: &Path) -> Result<Option<State>, TrackedReaderError> {
    if !path.exists() {
        return Ok(None);
    }

    let mut file = File::options().read(true).open(path)?;
    let state = State::load(&mut file)?;
    Ok(Some(state))
}

fn open_files(
    path: PathBuf,
    state: Option<State>,
) -> Result<(Vec<BufReader<File>>, Vec<u64>), TrackedReaderError> {
    match state {
        None => {
            let current_file_meta = std::fs::metadata(&path)?;
            let reader = BufReader::new(File::open(path)?);
            Ok((vec![reader], vec![current_file_meta.ino()]))
        }
        Some(state) => {
            let current_file_meta = std::fs::metadata(&path)?;
            let current_file = BufReader::new(File::open(&path)?);
            if current_file_meta.ino() == state.inode {
                Ok((vec![current_file], vec![current_file_meta.ino()]))
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
                Ok((
                    vec![older, current_file],
                    vec![older_path_meta.ino(), current_file_meta.ino()],
                ))
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

fn open_state_file(path: impl AsRef<Path>) -> std::io::Result<File> {
    File::options()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
}

impl Deref for TrackedReader {
    type Target = Multireader<BufReader<File>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for TrackedReader {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
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
