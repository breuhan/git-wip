mod git;
mod watch;
mod wip;

use std::process::ExitCode;

const USAGE: &str = "usage: git wip enable <remote> | disable | save | save-all | status | watch | restore [--merge | --force] [--no-fetch] [--prompt]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    // The prompt hook runs in every directory. Outside the repos the user enabled it must not run
    // git at all, since git would act on that repository's own config (core.fsmonitor and such).
    if args == ["restore", "--prompt"] {
        return match wip::enabled_repo(std::path::Path::new(".")) {
            // No network, and skip instead of waiting while the watcher holds the lock.
            Ok(Some(repo)) => finish(wip::restore(&git::Git::new(repo), false, false, true)),
            _ => ExitCode::SUCCESS,
        };
    }
    let here = git::Git::new(".");
    let g = git::Git::new(here.run(&["rev-parse", "--show-toplevel"]).unwrap_or_else(|_| ".".into()));
    let result = match args.as_slice() {
        ["enable", remote] => wip::enable(&g, remote),
        ["disable"] => wip::disable(&g),
        ["save"] => wip::save(&g, true),
        ["save-all"] => wip::save_all(),
        ["status"] => wip::status(&g),
        ["watch"] => watch::watch(),
        ["restore", flags @ ..] if flags.contains(&"--merge") => wip::merge(&g, !flags.contains(&"--no-fetch")),
        ["restore", flags @ ..] => wip::restore(&g, flags.contains(&"--force"), !flags.contains(&"--no-fetch"), false),
        _ => Err(USAGE.to_string()),
    };
    finish(result)
}

fn finish(result: git::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("wip: {e}");
            ExitCode::FAILURE
        }
    }
}
