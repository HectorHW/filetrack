use std::io::BufRead;

use clap::{Arg, Command};
use filetrack::TrackedReader;

fn main() -> Result<(), anyhow::Error> {
    let app = Command::new(clap::crate_name!())
        .arg(
            Arg::new("FILE_PATH")
                .long("path")
                .short('p')
                .required(true)
                .help("path to file that is possibly rotated"),
        )
        .arg(
            Arg::new("REGISTRY_FILE")
                .long("registry")
                .short('r')
                .required(true)
                .help("path to file that is used as registry to keep track of program state"),
        );

    let args = app.get_matches();

    let mut reader = TrackedReader::new(
        args.get_one::<String>("FILE_PATH").unwrap(),
        args.get_one::<String>("REGISTRY_FILE").unwrap(),
    )?;
    let mut input = String::new();
    match reader.read_line(&mut input) {
        Ok(0) => println!("reached end of file"),
        Ok(_) => println!("read line: `{}`", input.trim_end()),
        Err(e) => anyhow::bail!(e),
    };

    Ok(())
}
