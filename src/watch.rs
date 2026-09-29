use crate::git::{Git, Result};
use crate::wip;
use notify::{RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::{Duration, Instant};

fn setting(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Saves each enabled repo shortly after its files change, and fetches and restores periodically.
pub fn watch() -> Result<()> {
    let debounce = Duration::from_millis(setting("GIT_WIP_DEBOUNCE_MS", 2000));
    let every = Duration::from_secs(setting("GIT_WIP_FETCH_SECS", 30));
    let (tx, rx) = channel();
    let mut watcher = notify::recommended_watcher(tx).map_err(|e| e.to_string())?;
    // Canonical path (as reported by events) -> repo path as registered.
    let mut watched: HashMap<PathBuf, String> = HashMap::new();
    let mut changed: HashMap<String, Instant> = HashMap::new();
    let mut next_sync = Instant::now();

    loop {
        if Instant::now() >= next_sync {
            let repos = wip::repo_list()?;
            watched.retain(|path, repo| {
                let keep = repos.contains(repo);
                if !keep {
                    let _ = watcher.unwatch(path);
                }
                keep
            });
            for repo in &repos {
                let Ok(path) = std::fs::canonicalize(repo) else {
                    continue;
                };
                if let std::collections::hash_map::Entry::Vacant(slot) = watched.entry(path) {
                    match watcher.watch(slot.key(), RecursiveMode::Recursive) {
                        Ok(()) => drop(slot.insert(repo.clone())),
                        Err(e) => eprintln!("wip: {repo}: {e}"),
                    }
                }
                sync(repo);
            }
            next_sync = Instant::now() + every;
        }

        let deadline = changed
            .values()
            .map(|t| *t + debounce)
            .chain([next_sync])
            .min()
            .unwrap();
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Ok(event)) => {
                for path in event.paths {
                    if let Some(repo) = repo_of(&watched, &path) {
                        changed.insert(repo, Instant::now());
                    }
                }
            }
            Ok(Err(e)) => eprintln!("wip: watch: {e}"),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err("file watcher stopped".into()),
        }

        let due: Vec<String> = changed
            .iter()
            .filter(|(_, t)| t.elapsed() >= debounce)
            .map(|(r, _)| r.clone())
            .collect();
        for repo in due {
            changed.remove(&repo);
            if let Err(e) = wip::save(&Git::new(&repo)) {
                eprintln!("wip: {repo}: {e}");
            }
        }
    }
}

/// The repo a changed path belongs to, ignoring its `.git` directory (our own git calls write there).
fn repo_of(watched: &HashMap<PathBuf, String>, path: &Path) -> Option<String> {
    let (root, repo) = watched.iter().find(|(root, _)| path.starts_with(root))?;
    (!path.starts_with(root.join(".git"))).then(|| repo.clone())
}

fn sync(repo: &str) {
    let g = Git::new(repo);
    let result = match wip::remote(&g) {
        Some(remote) => wip::fetch(&g, &remote).and_then(|()| wip::restore(&g, false, false)),
        None => Ok(()),
    };
    if let Err(e) = result {
        eprintln!("wip: {repo}: {e}");
    }
}
