use astra_nopm::run;
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    exit(run(&args));
}
