mod git;
mod watch;
mod wip;

use std::process::ExitCode;

const USAGE: &str =
    "usage: git wip enable <remote> | disable | save | save-all | status | watch | restore [--force] [--no-fetch] [--prompt]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let here = git::Git::new(".");
    let g = git::Git::new(
        here.run(&["rev-parse", "--show-toplevel"])
            .unwrap_or_else(|_| ".".into()),
    );
    let result = match args.as_slice() {
        ["enable", remote] => wip::enable(&g, remote),
        ["disable"] => wip::disable(&g),
        ["save"] => wip::save(&g, true),
        ["save-all"] => wip::save_all(),
        ["status"] => wip::status(&g),
        ["watch"] => watch::watch(),
        // --prompt is the shell hook: no network, and skip instead of waiting while busy.
        ["restore", "--prompt"] => wip::restore(&g, false, false, true),
        ["restore", flags @ ..] => wip::restore(&g, flags.contains(&"--force"), !flags.contains(&"--no-fetch"), false),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("wip: {e}");
            ExitCode::FAILURE
        }
    }
}
