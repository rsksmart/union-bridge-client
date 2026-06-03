#![forbid(unsafe_code)]

use key_manager::cli::Cli;

fn main() {
    let cli = Cli::new();

    if let Err(e) = cli.run() {
        eprintln!("{e:?}");
        std::process::exit(1);
    }
}
