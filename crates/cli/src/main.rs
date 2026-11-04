//! `hb`: the command line for Hebra.

mod account;
mod bridge;
mod cli;
mod client;
mod collab;
mod commands;
mod extras;
mod fmt;
mod global;
mod heddle;
mod hooks;
mod inspect;
mod mcp;
mod porcelain;
mod remote;
mod runner;
mod search;
mod settings;
mod sign;
mod transfer;
mod verify;
mod verify_log;
mod work;

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;
use serde_json::json;

use crate::cli::{Cli, Command};

/// Replaces a command `alias.<name>` defines, like Git.
fn expand_alias(args: Vec<String>) -> Vec<String> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let Some(word) = args.get(1) else {
        return args;
    };
    let key = format!("alias.{word}");
    let Some(expansion) = hebra_core::Workspace::open(&cwd)
        .ok()
        .and_then(|ws| ws.config_get(&key).ok().flatten())
        .or_else(|| settings::get(&key))
    else {
        return args;
    };
    let mut out = vec![args[0].clone()];
    out.extend(expansion.split_whitespace().map(str::to_owned));
    out.extend(args.into_iter().skip(2));
    out
}

/// `hb log -5` is `hb log -n 5`, as in Git.
fn numeric_limit(args: Vec<String>) -> Vec<String> {
    if !args.iter().any(|a| a == "log") {
        return args;
    }
    let mut out = Vec::with_capacity(args.len() + 1);
    for arg in args {
        match arg.strip_prefix('-') {
            Some(digits) if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => {
                out.push("-n".to_owned());
                out.push(digits.to_owned());
            }
            _ => out.push(arg),
        }
    }
    out
}

/// `hb commit -Skey` is `--gpg-sign=key`, as in Git. Only in the commands that
/// sign: `hb log -Sword` looks for a word.
fn attached_key(args: Vec<String>) -> Vec<String> {
    let command = args.iter().skip(1).find(|a| {
        matches!(
            a.as_str(),
            "log" | "am" | "cherry-pick" | "commit" | "merge" | "pull" | "rebase" | "revert"
        )
    });
    if command.is_none_or(|c| c == "log") {
        return args;
    }
    let mut literal = false;
    args.into_iter()
        .map(|arg| {
            literal |= arg == "--";
            match arg.strip_prefix("-S") {
                Some(key) if !literal && !key.is_empty() && !key.starts_with('=') => {
                    format!("--gpg-sign={key}")
                }
                _ => arg,
            }
        })
        .collect()
}

fn parse() -> Cli {
    let args = attached_key(numeric_limit(std::env::args().collect()));
    match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(error) if error.kind() == clap::error::ErrorKind::InvalidSubcommand => {
            match Cli::try_parse_from(attached_key(expand_alias(args))) {
                Ok(cli) => cli,
                Err(_) => error.exit(),
            }
        }
        Err(error) => error.exit(),
    }
}

/// Parsing a command line this rich takes more stack than the main thread
/// has on Windows.
const STACK_BYTES: usize = 16 * 1024 * 1024;

fn main() -> ExitCode {
    std::thread::Builder::new()
        .stack_size(STACK_BYTES)
        .spawn(run)
        .ok()
        .and_then(|thread| thread.join().ok())
        .unwrap_or(ExitCode::FAILURE)
}

fn run() -> ExitCode {
    let cli = parse();
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
    let result = match global::execute(cli.command, &cwd, json) {
        Ok(result) => result,
        Err(command) => commands::execute(*command, &cwd),
    };
    match result {
        Ok(out) => {
            let mut stdout = std::io::stdout().lock();
            let written = if json {
                writeln!(
                    stdout,
                    "{}",
                    serde_json::to_string_pretty(&out.json).unwrap_or_default()
                )
            } else if let Some(raw) = &out.raw {
                stdout.write_all(raw)
            } else if out.diff && use_color(&stdout) {
                write!(stdout, "{}", fmt::color_diff(&out.text))
            } else {
                write!(stdout, "{}", out.text)
            };
            // A closed pipe (e.g. `hb log | head`) is not an error.
            let _ = written;
            ExitCode::from(out.exit)
        }
        Err(e) => fail(json, &e),
    }
}

/// Color only for a person at a terminal who has not asked otherwise.
fn use_color(stdout: &std::io::StdoutLock<'_>) -> bool {
    use std::io::IsTerminal;
    stdout.is_terminal() && std::env::var_os("NO_COLOR").is_none()
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
