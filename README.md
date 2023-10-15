# Filetrack

[![Rust](https://github.com/HectorHW/filetrack/workflows/Rust/badge.svg)](https://github.com/HectorHW/filetrack/actions)
[![Latest version](https://img.shields.io/crates/v/filetrack.svg)](https://crates.io/crates/filetrack)
[![Documentation](https://docs.rs/filetrack/badge.svg)](https://docs.rs/filetrack)
![License](https://img.shields.io/crates/l/filetrack.svg)

Filetrack is a library for persistent reading of logs similar to the mechanisms used in Filebeat and other software alike.
It provides a few useful primitives for working with IO and its main intention is to be used for implementation of custom log processors.

* `Multireader` that lets you work with a list of readers as if you had one single buffer

* `InodeAwareReader` that allows working with rotated logs and maintating persistent offset inside them. Scheme of persistence is
to be implemented by user.

* `TrackedReader` that allows to read logs or any other content from rotated files with offset persisted across restarts inside a file
in case you want a ready-to-use structure.

See [documentation](https://docs.rs/filetrack/latest/filetrack/) for examples and working principles.
