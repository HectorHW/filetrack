use filetrack::TrackedReader;
use std::io::{BufRead, Seek};

fn main() -> Result<(), anyhow::Error> {
    let mut reader = TrackedReader::new("examples/file.txt", "examples/registry")?;

    let mut input = String::new();

    let size = reader.read_line(&mut input)?;
    let input = input.trim_end();
    if input == "third" {
        println!("stumbled upon a third line, performing rollback");
        let offset = -(size as i64);
        reader.seek(std::io::SeekFrom::Current(offset))?;
    } else {
        println!("read `{input}`");
    }

    Ok(())
}
