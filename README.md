# Filetrack

Filetrack is a library for persistent reading of logs similar to the mechanisms used in Filebeat and other software alike.
It provides a few useful primitives for working with IO and its main intention is to be used for implementation of custom log processors.

* `Multireader` that lets you work with a list of readers as if you had one single buffer

```rust
# use std::io::{Cursor, Read};
# use filetrack::Multireader;
let inner_items = vec![Cursor::new(vec![1, 2, 3]), Cursor::new(vec![4, 5])];
// we get result here because Multireader performs seek
// (fallible operation) under the hood to determine sizes
let mut reader = Multireader::new(inner_items)?;
# let mut buf = vec![];
reader.read_to_end(&mut buf)?;
assert_eq!(buf, vec![1, 2, 3, 4, 5]);
# Ok::<(), std::io::Error>(())
```

* `InodeAwareReader` that allows working with rotated logs and maintating persistent offset inside them. Scheme of persistence is to be
implemented by user.

* `TrackedReader` that allows to read logs or any other content from rotated files with offset persisted across restarts inside a file
in case you want a ready-to-use structure.

```rust no_run
// running this program multiple times will output next line on each execution
# use std::io::BufRead;
# use filetrack::{TrackedReader, TrackedReaderError};
let mut reader = TrackedReader::new("examples/file.txt", "examples/registry")?;
# let mut input = String::new();
match reader.read_line(&mut input)? {
    0 => println!("reached end of file"),
    _ => println!("read line: `{}`", input.trim_end()),
};
# Ok::<(), TrackedReaderError>(())
```

See [documentation](https://docs.rs/filetrack/latest/filetrack/) for examples and working principles.
