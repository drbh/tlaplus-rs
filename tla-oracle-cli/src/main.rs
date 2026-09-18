use std::path::Path;
use std::process::ExitCode;

use tla_oracle::corpus::{Corpus, compare_corpora};

fn run() -> Result<bool, String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, path] if command == "extract" => {
            let corpus = Corpus::read(Path::new(path))?;
            println!("{}", serde_json::to_string_pretty(&corpus).map_err(|e| e.to_string())?);
            Ok(true)
        }
        [command, expected, actual] if command == "compare" => {
            let expected = Corpus::read(Path::new(expected))?;
            let actual = Corpus::read(Path::new(actual))?;
            let report = compare_corpora(&expected, &actual);
            println!("{}", serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
            Ok(report.is_match())
        }
        _ => Err("usage: oracle extract PATH | oracle compare EXPECTED ACTUAL\nPATH: JUnit manifest, Examples .json[.gz], recording directory, or extracted snapshot".into()),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}
