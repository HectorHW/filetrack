use std::io::BufRead;

use filetrack::TrackedReader;

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
