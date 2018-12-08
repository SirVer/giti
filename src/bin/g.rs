use giti::git;
use giti::ErrorKind;
use std::env;
use std::process;
use self_update::cargo_crate_version;

fn update() -> Result<(), Box<dyn (::std::error::Error)>> {
    let target = self_update::get_target()?;
    self_update::backends::github::Update::configure()?
        .repo_owner("SirVer")
        .repo_name("giti")
        .target(&target)
        .bin_name("g")
        .show_download_progress(true)
        .show_output(false)
        .no_confirm(true)
        .current_version(cargo_crate_version!())
        .build()?
        .update()?;
    Ok(())
}

fn main() {
    let args_owned: Vec<String> = env::args().collect();
    let args: Vec<&str> = args_owned.iter().map(|s| s as &str).collect();

    if args.len() > 1 && args[1] == "--update" {
        update().unwrap();
        return;
    }
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
