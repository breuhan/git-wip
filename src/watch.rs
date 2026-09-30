use crate::git::{Git, Result};
use crate::wip;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant};

fn setting(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

struct Repo {
    path: String,
    /// Inode of the checkout, to notice a re-clone at the same path.
    ino: u64,
}

/// Saves each enabled repo shortly after its files change and fetches periodically. Restoring is
/// left to the prompt hook, where the user sees it and no editor is mid-write.
pub fn watch() -> Result<()> {
    let debounce = Duration::from_millis(setting("GIT_WIP_DEBOUNCE_MS", 2000));
    let every = Duration::from_secs(setting("GIT_WIP_FETCH_SECS", 60));
    // Logs each file event that leads to a save.
    let debug = std::env::var_os("GIT_WIP_DEBUG").is_some();
    // Repos to save on the next round: new to the watcher, or their last save failed (e.g. offline).
    // Saving every repo every round would write snapshot objects for nothing.
    let mut retry: HashSet<String> = HashSet::new();
    // Repos whose last round failed. An error is logged when a repo starts failing, not every
    // round: offline for a day would otherwise fill the log.
    let mut failing: HashSet<String> = HashSet::new();
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
                if let Ok(root) = std::fs::canonicalize(path)
                    && let Entry::Vacant(slot) = watched.entry(root)
                {
                    match watcher.watch(slot.key(), RecursiveMode::Recursive) {
                        Ok(()) => {
                            eprintln!("wip: watching {path}");
                            let ino = inode(slot.key()).unwrap_or(0);
                            slot.insert(Repo { path: path.clone(), ino });
                            retry.insert(path.clone());
                        }
                        Err(e) => eprintln!("wip: {path}: {e}"),
                    }
                }
                let g = git(path);
                let saved = if retry.remove(path) { wip::save(&g, false).map(drop) } else { Ok(()) };
                if saved.is_err() {
                    retry.insert(path.clone());
                }
                let result = saved.and_then(|()| wip::remote(&g).map_or(Ok(()), |remote| wip::fetch(&g, &remote)));
                match result {
                    Ok(()) if failing.remove(path) => eprintln!("wip: {path}: working again"),
                    Ok(()) => {}
                    Err(e) if failing.insert(path.clone()) => {
                        eprintln!("wip: {path}: {e}\nwip: {path}: not logging further failures until it works again")
                    }
                    Err(_) => {}
                }
            }
            next_sync = Instant::now() + every;
        }

        let deadline = changed.values().map(|(t, _)| *t + debounce).chain([next_sync]).min().unwrap();
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            // inotify also reports reads (Access); our own save reads the tree, so reacting would loop.
            Ok(Ok(event)) if event.kind.is_access() => {}
            Ok(Ok(event)) => {
                if event.need_rescan() {
                    // The OS dropped events (queue overflow), so changes may have gone unnoticed.
                    retry.extend(watched.values().map(|r| r.path.clone()));
                }
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
                        if debug {
                            eprintln!("wip: debug: {:?} {}", event.kind, path.display());
                        }
                        // Wait from the first change, so a file written constantly cannot postpone the save.
                        changed.entry(repo).or_insert_with(|| (Instant::now(), vec![])).1.push(path);
                    }
                }
            }
            Ok(Err(e)) => eprintln!("wip: watch: {e}"),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err("file watcher stopped".into()),
        }

        let due: Vec<String> =
            changed.iter().filter(|(_, (t, _))| t.elapsed() >= debounce).map(|(r, _)| r.clone()).collect();
        for repo in due {
            let (_, paths) = changed.remove(&repo).unwrap();
            let g = git(&repo);
            // A failure is retried, and reported, by the next round.
            if worth_saving(&g, &repo, paths) && wip::save(&g, false).is_err() {
                retry.insert(repo);
            }
        }
    }
}

/// A dead connection must not block every other repo, so ssh gets timeouts, and one connection
/// per host is kept open across rounds instead of logging in every time. Not applied when the
/// user configured ssh for git (GIT_SSH_COMMAND would override a repo's core.sshCommand).
fn git(repo: &str) -> Git {
    let g = Git::new(repo);
    if std::env::var_os("GIT_SSH_COMMAND").is_some() || g.ok(&["config", "--get", "core.sshCommand"]) {
        return g;
    }
    g.with_env(
        "GIT_SSH_COMMAND",
        "ssh -o ConnectTimeout=15 -o ServerAliveInterval=15 -o ServerAliveCountMax=2 -o BatchMode=yes \
         -o ControlMaster=auto -o ControlPath=~/.ssh/git-wip-%C -o ControlPersist=120",
    )
}

fn inode(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.ino())
}

/// The innermost watched repo containing `path`, ignoring its `.git` directory (our own git calls write there).
fn repo_of(watched: &HashMap<PathBuf, Repo>, path: &Path) -> Option<String> {
    let (root, repo) =
        watched.iter().filter(|(root, _)| path.starts_with(root)).max_by_key(|(root, _)| root.as_os_str().len())?;
    (!path.starts_with(root.join(".git"))).then(|| repo.path.clone())
}

/// Whether any changed path is neither ignored nor inside a submodule. Submodule contents are
/// not part of a snapshot, and check-ignore refuses paths inside one.
fn worth_saving(g: &Git, repo: &str, mut paths: Vec<PathBuf>) -> bool {
    let modules = g.run(&["config", "--file", ".gitmodules", "--get-regexp", r"\.path$"]).unwrap_or_default();
    let modules: Vec<PathBuf> =
        modules.lines().filter_map(|l| l.split_once(' ')).map(|(_, p)| Path::new(repo).join(p)).collect();
    paths.retain(|p| !modules.iter().any(|m| p.starts_with(m)));
    let input: Vec<u8> =
        paths.iter().flat_map(|p| p.as_os_str().as_encoded_bytes().iter().chain(b"\0")).copied().collect();
    let ignored = g.run_with(&["check-ignore", "-z", "--stdin"], &[], Some(&input)).unwrap_or_default();
    ignored.split('\0').filter(|p| !p.is_empty()).count() != paths.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_repo_wins_over_its_parent() {
        let mut watched = HashMap::new();
        watched.insert(PathBuf::from("/r"), Repo { path: "outer".into(), ino: 0 });
        watched.insert(PathBuf::from("/r/sub"), Repo { path: "inner".into(), ino: 0 });
        assert_eq!(repo_of(&watched, Path::new("/r/sub/f")).as_deref(), Some("inner"));
        assert_eq!(repo_of(&watched, Path::new("/r/f")).as_deref(), Some("outer"));
        assert_eq!(repo_of(&watched, Path::new("/r/.git/index")), None);
    }
}
