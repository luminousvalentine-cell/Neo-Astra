use std::io::{self, Read};
use std::process::ExitCode;

use astra_compiler_api::process_json;

fn main() -> ExitCode {
    let mut input = String::new();

    if let Err(error) = io::stdin().read_to_string(&mut input) {
        eprintln!("astra-compiler: unable to read stdin: {error}");
        return ExitCode::FAILURE;
    }

    println!("{}", process_json(&input));
    ExitCode::SUCCESS
}
