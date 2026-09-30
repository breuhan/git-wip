use crate::git::{Git, Result};
use crate::wip;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::{Duration, Instant};

fn setting(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

struct Repo {
    path: String,
    /// Inode of the checkout, to notice a re-clone at the same path.
    ino: u64,
}

/// Saves each enabled repo shortly after its files change, and saves and fetches periodically.
/// Restoring is left to the prompt hook, where the user sees it and no editor is mid-write.
pub fn watch() -> Result<()> {
    let debounce = Duration::from_millis(setting("GIT_WIP_DEBOUNCE_MS", 2000));
    let every = Duration::from_secs(setting("GIT_WIP_FETCH_SECS", 30));
    set_ssh_timeouts();
    let (tx, rx) = channel();
    // Following symlinks would watch e.g. .direnv/flake-inputs into /nix/store (an inotify watch per dir).
    let config = Config::default().with_follow_symlinks(false);
    let mut watcher = RecommendedWatcher::new(tx, config).map_err(|e| e.to_string())?;
    // Canonical path (as reported by events) -> repo.
    let mut watched: HashMap<PathBuf, Repo> = HashMap::new();
    // Repo -> time of the first unsaved change and the changed paths.
    let mut changed: HashMap<String, (Instant, Vec<PathBuf>)> = HashMap::new();
    let mut next_sync = Instant::now();

    loop {
        if Instant::now() >= next_sync {
            let repos = wip::repo_list()?;
            watched.retain(|root, repo| {
                let keep = repos.contains(&repo.path) && inode(root) == Some(repo.ino);
                if !keep {
                    let _ = watcher.unwatch(root);
                }
                keep
            });
            for path in &repos {
                if let Ok(root) = std::fs::canonicalize(path) {
                    if let Entry::Vacant(slot) = watched.entry(root) {
                        match watcher.watch(slot.key(), RecursiveMode::Recursive) {
                            Ok(()) => {
                                eprintln!("wip: watching {path}");
                                let ino = inode(slot.key()).unwrap_or(0);
                                slot.insert(Repo {
                                    path: path.clone(),
                                    ino,
                                });
                            }
                            Err(e) => eprintln!("wip: {path}: {e}"),
                        }
                    }
                }
                sync(path);
            }
            next_sync = Instant::now() + every;
        }

        let deadline = changed
            .values()
            .map(|(t, _)| *t + debounce)
            .chain([next_sync])
            .min()
            .unwrap();
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Ok(event)) => {
                if event.kind.is_remove() {
                    // A deleted checkout: forget it so the next round watches a re-clone at the same path
                    // (Linux may give the new directory the same inode, so the inode check can miss it).
                    for path in &event.paths {
                        if watched.remove(path).is_some() {
                            let _ = watcher.unwatch(path);
                        }
                    }
                }
                for path in event.paths {
                    if let Some(repo) = repo_of(&watched, &path) {
                        // Wait from the first change, so a file written constantly cannot postpone the save.
                        changed
                            .entry(repo)
                            .or_insert_with(|| (Instant::now(), vec![]))
                            .1
                            .push(path);
                    }
                }
            }
            Ok(Err(e)) => eprintln!("wip: watch: {e}"),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err("file watcher stopped".into()),
        }

        let due: Vec<String> = changed
            .iter()
            .filter(|(_, (t, _))| t.elapsed() >= debounce)
            .map(|(r, _)| r.clone())
            .collect();
        for repo in due {
            let (_, paths) = changed.remove(&repo).unwrap();
            let g = Git::new(&repo);
            if all_ignored(&g, &paths) {
                continue;
            }
            if let Err(e) = wip::save(&g, false) {
                eprintln!("wip: {repo}: {e}");
            }
        }
    }
}

fn inode(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.ino())
}

/// The innermost watched repo containing `path`, ignoring its `.git` directory (our own git calls write there).
fn repo_of(watched: &HashMap<PathBuf, Repo>, path: &Path) -> Option<String> {
    let (root, repo) = watched
        .iter()
        .filter(|(root, _)| path.starts_with(root))
        .max_by_key(|(root, _)| root.as_os_str().len())?;
    (!path.starts_with(root.join(".git"))).then(|| repo.path.clone())
}

fn all_ignored(g: &Git, paths: &[PathBuf]) -> bool {
    let input: Vec<u8> = paths
        .iter()
        .flat_map(|p| p.as_os_str().as_encoded_bytes().iter().chain(b"\0"))
        .copied()
        .collect();
    let ignored = g
        .run_with(&["check-ignore", "-z", "--stdin"], &[], Some(&input))
        .unwrap_or_default();
    ignored.split('\0').filter(|p| !p.is_empty()).count() == paths.len()
}

/// Saving here too retries pushes that failed (e.g. offline) and covers changes made while not watching.
fn sync(repo: &str) {
    let g = Git::new(repo);
    let result = match wip::remote(&g) {
        Some(remote) => wip::save(&g, false).and_then(|()| wip::fetch(&g, &remote)),
        None => Ok(()),
    };
    if let Err(e) = result {
        eprintln!("wip: {repo}: {e}");
    }
}

/// A dead connection must not block every other repo; only applies when ssh is not configured through git.
fn set_ssh_timeouts() {
    let configured =
        std::env::var_os("GIT_SSH_COMMAND").is_some() || Git::new(".").ok(&["config", "--get", "core.sshCommand"]);
    if !configured {
        // SAFETY: single-threaded here, before the watcher starts its threads.
        unsafe {
            std::env::set_var(
                "GIT_SSH_COMMAND",
                "ssh -o ConnectTimeout=15 -o ServerAliveInterval=15 -o ServerAliveCountMax=2 -o BatchMode=yes",
            )
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_repo_wins_over_its_parent() {
        let mut watched = HashMap::new();
        watched.insert(
            PathBuf::from("/r"),
            Repo {
                path: "outer".into(),
                ino: 0,
            },
        );
        watched.insert(
            PathBuf::from("/r/sub"),
            Repo {
                path: "inner".into(),
                ino: 0,
            },
        );
        assert_eq!(repo_of(&watched, Path::new("/r/sub/f")).as_deref(), Some("inner"));
        assert_eq!(repo_of(&watched, Path::new("/r/f")).as_deref(), Some("outer"));
        assert_eq!(repo_of(&watched, Path::new("/r/.git/index")), None);
    }
}
