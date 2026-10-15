//! `hb`: the command line for Hebra.

mod cli;
mod commands;
mod fmt;
mod mcp;

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;
use serde_json::json;

use crate::cli::{Cli, Command};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    let cwd = match cli.cwd.clone().map_or_else(std::env::current_dir, Ok) {
        Ok(cwd) => cwd,
        Err(e) => return fail(json, &anyhow::Error::from(e)),
    };
    if let Command::Mcp = cli.command {
        return match mcp::serve(Some(cwd)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(json, &e),
        };
    }
    match commands::execute(cli.command, &cwd) {
        Ok(out) => {
            let mut stdout = std::io::stdout().lock();
            let written = if json {
                writeln!(
                    stdout,
                    "{}",
                    serde_json::to_string_pretty(&out.json).unwrap_or_default()
                )
            } else {
                write!(stdout, "{}", out.text)
            };
            // A closed pipe (e.g. `hb log | head`) is not an error.
            let _ = written;
            ExitCode::SUCCESS
        }
        Err(e) => fail(json, &e),
    }
}

fn fail(json: bool, error: &anyhow::Error) -> ExitCode {
    let code = error
        .downcast_ref::<hebra_core::Error>()
        .map_or("error", hebra_core::Error::code);
    if json {
        eprintln!(
            "{}",
            json!({ "error": { "code": code, "message": format!("{error:#}") } })
        );
    } else {
        eprintln!("error: {error:#}");
    }
    ExitCode::FAILURE
}
