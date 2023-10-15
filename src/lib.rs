//! # Filetrack
//!
//! Filetrack is a library for persistent reading of logs similar to the mechanisms used in Filebeat and other software alike.
//! It provides a few useful primitives for working with IO and its main intention is to be used for implementation of custom log processors.
//!
//! * `Multireader` that lets you work with a list of readers as if you had one single buffer
//!
//! ```rust
//! # use std::io::{Cursor, Read};
//! # use filetrack::Multireader;
//! let inner_items = vec![Cursor::new(vec![1, 2, 3]), Cursor::new(vec![4, 5])];
//! // we get result here because Multireader performs seek
//! // (fallible operation) under the hood to determine sizes
//! let mut reader = Multireader::new(inner_items)?;
//! # let mut buf = vec![];
//! reader.read_to_end(&mut buf)?;
//! assert_eq!(buf, vec![1, 2, 3, 4, 5]);
//! # Ok::<(), std::io::Error>(())
//! ```
//!
//! * `InodeAwareReader` that allows working with rotated logs and maintating persistent offset inside them. Scheme of persistence is
//! to be implemented by user.
//!
//! ```rust no_run
//! # use std::io::{Read, BufRead, self};
//! # use filetrack::{InodeAwareOffset, InodeAwareReader};
//! # fn load_state() -> io::Result<InodeAwareOffset> {Ok(InodeAwareOffset{inode: 0, offset: 0})}
//! # fn save_state(state: InodeAwareOffset) -> io::Result<()> {Ok(())}
//! let mut reader = InodeAwareReader::from_rotated_logs("/var/log/mail.log")?;
//! reader.seek_persistent(load_state()?)?;
//! # let mut buf = vec![];
//! reader.read_exact(& mut buf)?;
//! save_state(reader.get_persistent_offset())?;
//! # Ok::<(), std::io::Error>(())
//! ```
//!
//! * `TrackedReader` that allows to read logs or any other content from rotated files with offset persisted across restarts inside a file
//! in case you want a ready-to-use structure.
//!
//! ```rust no_run
//! # use filetrack::{TrackedReader, TrackedReaderError};
//! # use std::io::BufRead;
//! // running this script will fetch and print new lines on each execution
//! # fn main() -> Result<(), TrackedReaderError> {
//! let mut reader = TrackedReader::new("examples/file.txt", "examples/registry")?;
//! let mut input = String::new();
//! loop {
//!     match reader.read_line(&mut input)? {
//!         0 => break Ok(()),
//!         _ => println!("read line: `{}`", input.trim_end()),
//!     };
//! }
//! # }
//! ```
//!

mod inode_aware;
mod multireader;
pub mod path_utils;
mod tracked_reader;

pub use inode_aware::{InodeAwareOffset, InodeAwareReader};
pub use multireader::Multireader;
pub use tracked_reader::{State, StateSerdeError, TrackedReader, TrackedReaderError};
