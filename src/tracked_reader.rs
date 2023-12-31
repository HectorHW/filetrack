use std::{
    fs::File,
    io::Seek,
    ops::{Deref, DerefMut},
    path::Path,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::inode_aware::{InodeAwareOffset, InodeAwareReader};

/// Structure used by `TrackedReader` for simple file persistence.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct State {
    pub offset: InodeAwareOffset,
}

/// Possible errors that could happen while working with persistent state storage.
#[derive(Error, Debug)]
pub enum StateSerdeError {
    #[error("while working with underlying file")]
    IO(#[from] std::io::Error),

    #[error("while trying to (de)serialize state")]
    Serde(#[from] bincode::Error),
}

impl State {
    /// deserialize State from a file
    pub fn load(file: &mut File) -> Result<Self, StateSerdeError> {
        file.rewind()?;
        let state = bincode::deserialize_from(file)?;
        Ok(state)
    }

    /// serialize and write State to a file
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

/// Structure that implements `Read`, `ReadBuf` and `Seek` while working with persistent offset in underlying logrotated files.
/// External file is used to persist offset across restarts.
///
/// ## Usage
///
/// Instantiate `TrackedReader` by passing it a path to logfile intended for reading as well as a path to file used as a registry for
/// persistent offset storage. Created structure can be used where implementation of `Read` or `BufRead` is expected. Additionally,
/// limited `Seek` implementation is provided (see Limitations for more info).
///
/// ```rust no_run
/// # use filetrack::{TrackedReader, TrackedReaderError};
/// # use std::io::BufRead;
///
/// // running this script will fetch and print new lines on each execution
/// # fn main() -> Result<(), TrackedReaderError> {
/// let mut reader = TrackedReader::new("examples/file.txt", "examples/registry")?;
/// let mut input = String::new();
/// loop {
///     match reader.read_line(&mut input)? {
///         0 => break Ok(()),
///         _ => println!("read line: `{}`", input.trim_end()),
///     };
/// }
/// # }
/// ```
///
/// ## Cleanup
///
/// There are two distinct ways to perform cleanup for this structure:
///
/// * **explicit** by calling `.close()`. This will allow you to handle any errors that may happen in the process
/// * **implicitly** by relying on `Drop`. Note that errors generated while working with the filesystem cannot be handled and will
/// cause a panic in this case.
///
///
/// ## Working principles
///
/// To maintain offset in a file across restarts, separate "registry" file is used for persistence. Inode is stored additionally to
/// offset, which allows to keep reading log file in case it was logrotate'd. During intialization, inode of file to be read
/// is compared to previously known and if it differs, it means that file was rotated and a search for original file is performed by checking
/// a file identified by path appended by `.1` (eg. `mail.log` and `mail.log.1`) and so on. After that you are given a file-like structure that allows
/// buffered reading and seeking in up to specified number of files files.
///
/// ## Limitations
///
/// * You can only expect this to work if logrotation happened not more than the number you specified as search_depth. This means that if you are
/// creating a log processor for example, it should be run frequently enough to keep up with logs that are written and rotated.
///
/// * Due to simple scheme of persistence, we cannot seek back into rotated file version after saving state while reading from current
/// log file. This means that if your program must do some conditional seeking in a file, you should perform any pointer rollback before
/// performing final save (done by `.close()` or Drop). Overall, this library is intended to be used for mostly forward reading of
/// log files.
pub struct TrackedReader {
    inner: InodeAwareReader,
    registry: File,
    already_freed: bool,
}

/// Possible errors that could happen while working with `TrackedReader`.
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
    /// * `filepath` - a path to log file to be read. `TrackedReader` will additionally search for logrotated file under `{filepath}.1`.
    /// * `registry` - path to registry file used to persist offset and other metadata.
    pub fn new(
        filepath: impl AsRef<Path>,
        registry: impl AsRef<Path>,
    ) -> Result<Self, TrackedReaderError> {
        Self::with_search_depth(filepath, registry, 1)
    }

    /// Like `::new` but allows specifying how many rotated items to check.
    ///
    /// `search_depth` of 2 means that apart from `log` file we will check for `log.1` and `log.2`.
    pub fn with_search_depth(
        filepath: impl AsRef<Path>,
        registry: impl AsRef<Path>,
        search_depth: usize,
    ) -> Result<Self, TrackedReaderError> {
        let state_from_disk = maybe_read_state(registry.as_ref())?;
        let reader = InodeAwareReader::from_rotated_logs_with_depth(filepath, search_depth)?;
        // now that we know that open_files did not fail, we can create registry file
        let registry = open_state_file(registry)?;
        let mut reader = Self {
            inner: reader,
            registry,
            already_freed: false,
        };
        if let Some(state) = state_from_disk {
            reader.seek_persistent(state.offset)?;
        } else {
            // If state did not exist previously, registry file is created empty. We should additionally initialize file content.
            // This will make struct work correctly even if close/Drop will never happen (eg in case of mem::forget).
            reader.persist()?;
        }

        Ok(reader)
    }

    /// Explicitly save current state into registry file and return any errors generated.
    pub fn persist(&mut self) -> std::io::Result<()> {
        self.get_persistent_state().persist(&mut self.registry)
    }

    /// Explicitly finalize structure, returning any errors that were produced in the process. Alternative to relying on `Drop`.
    pub fn close(mut self) -> std::io::Result<()> {
        self.persist()?;
        self.already_freed = true;
        Ok(())
    }

    /// Get current state for possible manual handling.
    pub fn get_persistent_state(&self) -> State {
        State {
            offset: self.get_persistent_offset(),
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

fn open_state_file(path: impl AsRef<Path>) -> std::io::Result<File> {
    File::options()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
}

impl Deref for TrackedReader {
    type Target = InodeAwareReader;

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
