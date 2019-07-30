/// Tools to shell out to external commands.
use super::error::{Error, Result};
use std::path::Path;
use std::process;

use term;

#[derive(Clone, Copy)]
enum PrintCommands {
    YES,
    NO,
}

pub fn run_editor(path: &Path) -> Result<()> {
    let editor = default_editor::get()?;
    let mut it = editor.split(" ");
    let cmd = it.next().unwrap();
    let mut args: Vec<String> = it.map(|s| s.to_string()).collect();
    args.push(path.to_str().unwrap().to_string());
    let _ = process::Command::new(cmd).args(&args).spawn()?.wait();
    Ok(())
}

/// Dispatches to 'command' without echoing.
pub fn dispatch_to(command: &str, args: &[&str]) -> Result<()> {
    shell_out(command, args, PrintCommands::NO)
}

/// Runs the command and echoing the command line.
pub fn run_command(args: &[&str]) -> Result<()> {
    shell_out(args[0], &args[1..], PrintCommands::YES)
}

/// Runs the command, but captures stdout & stdin. Named after the python function.
pub fn communicate(args: &[&str]) -> Result<process::Output> {
    Ok(process::Command::new(&args[0]).args(&args[1..]).output()?)
}

/// Dispatches to 'program' with 'str'. 'print' decides if the command lines are echoed.
fn shell_out(program: &str, args: &[&str], print: PrintCommands) -> Result<()> {
    match print {
        PrintCommands::YES => {
            let mut terminal = term::stdout().unwrap();
            terminal.fg(term::color::CYAN).unwrap();
            write!(terminal, "=> Running: {} {}", program, args.join(" ")).unwrap();
            terminal.reset().unwrap();
            writeln!(terminal, "").unwrap();
        }
        PrintCommands::NO => {}
    }

    let mut child = process::Command::new(program)
        .args(args)
        .stdin(process::Stdio::inherit())
        .stdout(process::Stdio::inherit())
        .stderr(process::Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to execute: {}", e));

    let result = match child.wait().unwrap().code() {
        Some(0) => Ok(()),
        Some(a) => Err(Error::subcommand_fail(program, a)),
        None => Err(Error::general(format!(
            "{} was terminated by a signal.",
            program
        ))),
    };

    match print {
        PrintCommands::YES => {
            // An empty line to separate the different commands.
            println!()
        }
        PrintCommands::NO => (),
    }
    result
}
