use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Seek},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
struct State {
    offset: u64,
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

    pub fn persist(&self, file: &mut File) -> Result<(), StateSerdeError> {
        file.rewind()?;
        bincode::serialize_into(file, self)?;

        Ok(())
    }
}

pub struct TrackedReader {
    file: BufReader<File>,
    state: State,
    registry: File,
}

#[derive(Error, Debug)]
pub enum TrackedReaderError {
    #[error("while working with underlying file")]
    IO(#[from] std::io::Error),
    #[error("while working with persistent state storage")]
    Persistence(#[from] StateSerdeError),
}

impl TrackedReader {
    pub fn new(filepath: &str, registry: &str) -> Result<Self, TrackedReaderError> {
        let file = BufReader::new(File::open(filepath)?);
        let (state, registry) = if std::path::Path::new(registry).exists() {
            let mut registry = File::options().read(true).write(true).open(registry)?;
            let state = State::load(&mut registry)?;
            (state, registry)
        } else {
            let state = State { offset: 0 };
            let mut registry = File::options()
                .read(true)
                .write(true)
                .create_new(true)
                .open(registry)?;

            // I prefer to fail early
            state.persist(&mut registry)?;
            (state, registry)
        };
        let mut object = Self {
            file,
            state,
            registry,
        };

        object.seek(std::io::SeekFrom::Start(state.offset))?;

        Ok(object)
    }
}

impl Read for TrackedReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.file.read(buf) {
            Ok(size) => {
                self.state.offset += size as u64;
                Ok(size)
            }
            Err(e) => Err(e),
        }
    }
}

impl BufRead for TrackedReader {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        self.file.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.file.consume(amt);
        self.state.offset += amt as u64;
    }
}

impl Seek for TrackedReader {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        let offset_from_start = self.file.seek(pos)?;
        self.state.offset = offset_from_start;
        Ok(offset_from_start)
    }
}

impl Drop for TrackedReader {
    fn drop(&mut self) {
        self.state.persist(&mut self.registry).unwrap();
    }
}
