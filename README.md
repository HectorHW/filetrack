# filetrack

Filetrack is a library for persistent reading of logs similar to the mechanisms used in Filebeat and other similar software.
It's main intention is to be used for implementation of custom log processors.

## Usage

Instantiate `TrackedReader` by passing it a path to logfile intended for reading as well as a path to file used as registry for persistent
offset storage.

```rust
fn main() -> Result<(), anyhow::Error> {
    let mut reader = TrackedReader::new("examples/file.txt", "examples/registry")?;
    let mut input = String::new();
    match reader.read_line(&mut input) {
        Ok(0) => println!("reached end of file"),
        Ok(_) => println!("read line: `{}`", input.trim_end()),
        Err(e) => anyhow::bail!(e),
    };

    Ok(())
}
```

Created structure can be used where implementation of `Read` or `BufRead` is expected. Additionally, limited `Seek` implementation
is provided (see Limitations for more info).

## Working principles

To maintain offset in a file across restarts, separate "registry" file is used for persistence. Inode is stored additionally to
offset, which allows to keep reading log file in case it was logrotate'd at MOST once. During intialization, inode of file to be read
is compared to previously known and if it differs, it means that file was rotated and a search for original file is performed by checking
a file identified by path appended by `.1` (eg. `mail.log` and `mail.log.1`). After that you are given a file-like structure that allows
buffered reading and seeking in up to two files.

## Limitations

* You can only expect this to work if logrotation happened at most once. This means that if you are creating a log processor for
example, it should be run frequently enough to keep up with logs that are written and rotated.

* Due to simple scheme of persistence, we cannot seek back into rotated file version after saving state while reading from current
log file. This means that if your program must do some conditional seeking in file, you should perform any pointer rollback before
performing final save (done by `.close()` or Drop). Overall, this library is intended to be used for mostly forward reading of
log files.
