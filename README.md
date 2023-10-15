# Filetrack

Filetrack is a library for persistent reading of logs similar to the mechanisms used in Filebeat and other software alike.
It provides a few useful primitives for working with IO and its main intention is to be used for implementation of custom log processors.

* `Multireader` that lets you work with a list of readers as if you had one single buffer

* `InodeAwareReader` that allows working with rotated logs and maintating persistent offset inside them. Scheme of persistence is
to be implemented by user.

* `TrackedReader` that allows to read logs or any other content from rotated files with offset persisted across restarts inside a file
in case you want a ready-to-use structure.

See [documentation](https://docs.rs/filetrack/latest/filetrack/) for examples and working principles.
