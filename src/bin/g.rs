extern crate giti;
extern crate git2;

use giti::git;
use giti::ErrorKind;
use std::env;
use std::process;

fn main() {
    let args_owned: Vec<String> = env::args().collect();
    let args: Vec<&str> = args_owned.iter().map(|s| s as &str).collect();

    let result = git::handle_repository(&args[1..]);

    let exit_code = match result {
        Err(error) => {
            match error.kind {
                ErrorKind::GeneralError => println!("{}", error.description()),
                ErrorKind::SubcommandFailed => {}
            };
            1
        }
        Ok(()) => 0,
    };
    process::exit(exit_code);
}
