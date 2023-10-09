#![doc = include_str!("../README.md")]

mod multireader;
mod tracked_reader;

pub use multireader::Multireader;
pub use tracked_reader::{State, StateSerdeError, TrackedReader, TrackedReaderError};
