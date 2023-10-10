#![doc = include_str!("../README.md")]

mod inode_aware;
mod multireader;
mod path_utils;
mod tracked_reader;

pub use multireader::Multireader;
pub use tracked_reader::{State, StateSerdeError, TrackedReader, TrackedReaderError};
