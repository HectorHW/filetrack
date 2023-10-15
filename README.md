# Filetrack

[![Rust](https://github.com/HectorHW/filetrack/workflows/Rust/badge.svg)](https://github.com/HectorHW/filetrack/actions)
[![Latest version](https://img.shields.io/crates/v/filetrack.svg)](https://crates.io/crates/filetrack)
[![Documentation](https://docs.rs/filetrack/badge.svg)](https://docs.rs/filetrack)
![License](https://img.shields.io/crates/l/filetrack.svg)

Filetrack is a library for persistent reading of logs similar to the mechanisms used in Filebeat and other software alike.
It provides a few useful primitives for working with IO and its main intention is to be used for implementation of custom log processors.

* `Multireader` that lets you work with a list of readers as if you had one single buffer.

* `InodeAwareReader` that allows working with rotated logs and maintating persistent offset inside them. Scheme of persistence is
to be implemented by user.

* `TrackedReader` that allows to read logs or any other content from rotated files with offset persisted across restarts inside a file
in case you want a ready-to-use structure.

## Example

Read a file line-by-line keeping track of current offset so that you could start where you left off next time. Note that the library
does not force you to use this scheme, you can implement your own persistence scheme using `InodeAwareReader`.

```rust
use filetrack::{TrackedReader, TrackedReaderError};
use std::io::BufRead;

// running this script will fetch and print new lines on each execution
fn main() -> Result<(), TrackedReaderError> {
    let mut reader = TrackedReader::new("examples/file.txt", "examples/registry")?;
    let mut input = String::new();
    loop {
        match reader.read_line(&mut input)? {
            0 => break Ok(()),
            _ => println!("read line: `{}`", input.trim_end()),
        };
    }
}
```

See [documentation](https://docs.rs/filetrack/latest/filetrack/) for more examples and working principles.
