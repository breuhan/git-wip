mod git;
mod ui;
mod watch;
mod wip;

use std::process::ExitCode;

const USAGE: &str = "\
usage: git wip <command>

  enable <remote>   sync this repository through <remote>
  disable           stop syncing this repository
  status            snapshot per host, and what a restore would do
  restore           take the newest state from another host
    --merge           combine it with local changes
    --force           replace local changes (kept in refs/wip-backup/<host>)
    --no-fetch        use what was fetched last
  save              snapshot and push now
  save-all          save and fetch every enabled repository
  watch             save on file changes, fetch periodically (run by the service)";

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
    let has = |flag: &str| args[1..].contains(&flag);
    let result = match args.as_slice() {
        ["help" | "-h" | "--help"] => {
            println!("{USAGE}");
            Ok(())
        }
        ["enable", remote] => wip::enable(&g, remote),
        ["disable"] => wip::disable(&g),
        ["save"] => wip::save(&g, true).map(|outcome| ui::say(&outcome)),
        ["save-all"] => wip::save_all(),
        ["status"] => wip::status(&g),
        ["watch"] => watch::watch(),
        // A mistyped flag must not quietly run a plain restore.
        ["restore", flags @ ..] if flags.iter().any(|f| !["--merge", "--force", "--no-fetch"].contains(f)) => {
            Err(USAGE.to_string())
        }
        ["restore", ..] if has("--merge") => wip::merge(&g, !has("--no-fetch")),
        ["restore", ..] => wip::restore(&g, has("--force"), !has("--no-fetch"), false),
        _ => Err(USAGE.to_string()),
    };
    finish(result)
}

fn finish(result: git::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            ui::say(&e);
            ExitCode::FAILURE
        }
    }
}
