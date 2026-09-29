use crate::git::{Git, Result};

pub fn enable(g: &Git, remote: &str) -> Result<()> {
    let top = g.run(&["rev-parse", "--show-toplevel"])?;
    g.run(&["config", "wip.remote", remote])?;
    let repos = g
        .run(&["config", "--global", "--get-all", "wip.repo"])
        .unwrap_or_default();
    if !repos.lines().any(|l| l == top) {
        g.run(&["config", "--global", "--add", "wip.repo", &top])?;
    }
    Ok(())
}

pub fn disable(g: &Git) -> Result<()> {
    let top = g.run(&["rev-parse", "--show-toplevel"])?;
    let _ = g.run(&["config", "--unset", "wip.remote"]);
    let _ = g.run(&["config", "--global", "--fixed-value", "--unset-all", "wip.repo", &top]);
    Ok(())
}

fn remote(g: &Git) -> Option<String> {
    g.run(&["config", "--get", "wip.remote"]).ok()
}

const BUSY: [&str; 6] = [
    "rebase-merge",
    "rebase-apply",
    "MERGE_HEAD",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "BISECT_LOG",
];

fn host() -> String {
    std::env::var("GIT_WIP_HOST").unwrap_or_else(|_| {
        let out = std::process::Command::new("hostname")
            .output()
            .map(|o| o.stdout)
            .unwrap_or_default();
        String::from_utf8_lossy(&out)
            .trim()
            .split('.')
            .next()
            .unwrap_or("unknown")
            .to_string()
    })
}

fn own_ref() -> String {
    format!("refs/wip/{}", host())
}

fn busy(g: &Git) -> Result<bool> {
    if !g.ok(&["symbolic-ref", "-q", "HEAD"]) || !g.ok(&["rev-parse", "-q", "--verify", "HEAD"]) {
        return Ok(true);
    }
    let dirs = g.run(&["rev-parse", "--path-format=absolute", "--git-dir", "--git-common-dir"])?;
    if dirs.lines().next() != dirs.lines().nth(1) {
        return Ok(true); // linked worktree: refs/wip/<host> is shared with the main one
    }
    for p in BUSY {
        if g.path(p)?.exists() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Held for the whole save or restore so the timer and the cd hook never interleave.
fn lock(g: &Git) -> Result<Option<std::fs::File>> {
    let f = std::fs::File::create(g.path("wip.lock")?).map_err(|e| e.to_string())?;
    Ok(f.try_lock().is_ok().then_some(f))
}

/// Stash-shaped commit (parents HEAD, index, untracked) of the current state; touches nothing.
fn snapshot(g: &Git) -> Result<String> {
    let head = g.run(&["rev-parse", "HEAD"])?;
    let branch = g.run(&["symbolic-ref", "--short", "HEAD"])?;
    let w = g.run(&["stash", "create"])?;
    let (tree, index) = if w.is_empty() {
        let tree = g.run(&["rev-parse", "HEAD^{tree}"])?;
        let index = g.run(&["commit-tree", &tree, "-p", &head, "-m", &format!("index on {branch}")])?;
        (tree, index)
    } else {
        (
            g.run(&["rev-parse", &format!("{w}^{{tree}}")])?,
            g.run(&["rev-parse", &format!("{w}^2")])?,
        )
    };
    let untracked = untracked(g, &branch)?;
    // A host's first clean snapshot dates from its HEAD commit, so enabling an idle host never
    // looks newer than real WIP elsewhere.
    let first = !g.ok(&["rev-parse", "-q", "--verify", &own_ref()]);
    let idle = first && w.is_empty() && untracked.is_none();
    let date = if idle {
        g.run(&["log", "-1", "--format=%cI", "HEAD"])?
    } else {
        String::new()
    };
    let mut args = vec![
        "commit-tree".to_string(),
        tree,
        "-p".into(),
        head.clone(),
        "-p".into(),
        index,
    ];
    if let Some(u) = untracked {
        args.extend(["-p".into(), u]);
    }
    args.extend(["-m".into(), format!("WIP on {branch}: {head}")]);
    let env: &[(&str, &str)] = if idle { &[("GIT_COMMITTER_DATE", &date)] } else { &[] };
    g.run_with(&args.iter().map(String::as_str).collect::<Vec<_>>(), env, None)
}

fn untracked(g: &Git, branch: &str) -> Result<Option<String>> {
    let files = g.run(&["ls-files", "-z", "--others", "--exclude-standard"])?;
    if files.is_empty() {
        return Ok(None);
    }
    let idx = g.path("wip-index")?;
    let _ = std::fs::remove_file(&idx);
    let env = [("GIT_INDEX_FILE", idx.to_str().ok_or("non-utf8 git dir")?)];
    g.run_with(
        &["update-index", "--add", "-z", "--stdin"],
        &env,
        Some(files.as_bytes()),
    )?;
    let tree = g.run_with(&["write-tree"], &env, None);
    let _ = std::fs::remove_file(&idx);
    Ok(Some(g.run(&[
        "commit-tree",
        &tree?,
        "-m",
        &format!("untracked files on {branch}"),
    ])?))
}

fn state(g: &Git, c: &str) -> Result<String> {
    let mut s = g.run(&[
        "rev-parse",
        &format!("{c}^{{tree}}"),
        &format!("{c}^1"),
        &format!("{c}^2^{{tree}}"),
    ])?;
    if let Ok(u) = g.run(&["rev-parse", "-q", "--verify", &format!("{c}^3^{{tree}}")]) {
        s.push_str(&u);
    }
    Ok(s)
}

fn same(g: &Git, a: &str, b: &str) -> Result<bool> {
    Ok(state(g, a)? == state(g, b)?)
}

fn fetch(g: &Git, remote: &str) -> Result<()> {
    g.run(&[
        "fetch",
        "--quiet",
        remote,
        &format!("+refs/wip/*:refs/remotes/{remote}/wip/*"),
    ])
    .map(|_| ())
}

pub fn save(g: &Git) -> Result<()> {
    let Some(remote) = remote(g) else { return Ok(()) };
    if busy(g)? {
        return Ok(());
    }
    let Some(_lock) = lock(g)? else { return Ok(()) };
    let snap = snapshot(g)?;
    let own = own_ref();
    if let Ok(old) = g.run(&["rev-parse", "-q", "--verify", &own]) {
        if same(g, &old, &snap)? {
            return Ok(());
        }
    }
    // Push before moving the local ref so an offline run is retried next time.
    g.run(&["push", "--quiet", &remote, &format!("+{snap}:{own}")])?;
    g.run(&["update-ref", &own, &snap])?;
    Ok(())
}

fn is_ancestor(g: &Git, a: &str, b: &str) -> bool {
    g.ok(&["merge-base", "--is-ancestor", a, b])
}

fn commit_time(g: &Git, rev: &str) -> i64 {
    g.run(&["log", "-1", "--format=%ct", rev])
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(0)
}

/// (commit time, oid, host) of the newest snapshot fetched from another host.
fn newest_foreign(g: &Git, remote: &str) -> Result<Option<(i64, String, String)>> {
    let prefix = format!("refs/remotes/{remote}/wip/");
    let me = host();
    let out = g.run(&[
        "for-each-ref",
        "--format=%(committerdate:unix) %(objectname) %(refname)",
        &prefix,
    ])?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, ' ');
            let time = it.next()?.parse().ok()?;
            let oid = it.next()?.to_string();
            let from = it.next()?.strip_prefix(&prefix)?.to_string();
            (from != me).then_some((time, oid, from))
        })
        .max_by_key(|s| s.0))
}

fn has_changes(g: &Git, snap: &str) -> Result<bool> {
    let base = g.run(&["rev-parse", &format!("{snap}^1^{{tree}}")])?;
    let trees = g.run(&["rev-parse", &format!("{snap}^{{tree}}"), &format!("{snap}^2^{{tree}}")])?;
    let untracked = g.ok(&["rev-parse", "-q", "--verify", &format!("{snap}^3")]);
    Ok(untracked || trees.lines().any(|t| t != base))
}

pub fn restore(g: &Git, force: bool, fetch_first: bool) -> Result<()> {
    let Some(remote) = remote(g) else { return Ok(()) };
    if busy(g)? {
        return Ok(());
    }
    let Some(_lock) = lock(g)? else { return Ok(()) };
    if fetch_first {
        fetch(g, &remote)?;
    }
    let own = own_ref();
    let Some((time, snap, from)) = newest_foreign(g, &remote)? else {
        return Ok(());
    };
    if time <= commit_time(g, &own).max(commit_time(g, "HEAD")) {
        return Ok(());
    }
    let current = snapshot(g)?;
    let untouched = g.run(&["status", "--porcelain"])?.is_empty()
        || g.run(&["rev-parse", "-q", "--verify", &own])
            .map_or(Ok(false), |o| same(g, &o, &current))?;
    if !force && !untouched {
        eprintln!("wip: {from} has newer changes, run `git wip restore --force`");
        return Ok(());
    }

    let subject = g.run(&["log", "-1", "--format=%s", &snap])?;
    let branch = subject
        .strip_prefix("WIP on ")
        .and_then(|s| s.split_once(": "))
        .map(|(b, _)| b.to_string())
        .ok_or(format!("unexpected snapshot message: {subject}"))?;
    let base = g.run(&["rev-parse", &format!("{snap}^1")])?;
    let local = format!("refs/heads/{branch}");
    let exists = g.ok(&["rev-parse", "-q", "--verify", &local]);
    let behind = exists && is_ancestor(g, &local, &base);
    if exists && !behind && !is_ancestor(g, &base, &local) {
        eprintln!("wip: {branch} has diverged from {from}, not restoring");
        return Ok(());
    }

    let backup = format!("refs/wip-backup/{}", host());
    g.run(&[
        "update-ref",
        "--create-reflog",
        "-m",
        "git wip restore",
        &backup,
        &current,
    ])?;
    g.run(&["reset", "--hard", "-q"])?;
    g.run(&["clean", "-fdq"])?;
    if exists {
        g.run(&["checkout", "-q", &branch])?;
        if behind {
            g.run(&["merge", "--ff-only", "-q", &base])?;
        }
    } else {
        g.run(&["checkout", "-q", "-b", &branch, &base])?;
    }
    if has_changes(g, &snap)? {
        g.run(&["stash", "apply", "--index", "-q", &snap])?;
    }
    g.run(&["update-ref", &own, &snap])?;
    eprintln!("wip: restored state from {from} (previous state in {backup})");
    Ok(())
}

pub fn save_all() -> Result<()> {
    let repos = Git::new(".")
        .run(&["config", "--global", "--get-all", "wip.repo"])
        .unwrap_or_default();
    for dir in repos.lines() {
        let g = Git::new(dir);
        let result = save(&g).and_then(|()| remote(&g).map_or(Ok(()), |r| fetch(&g, &r)));
        if let Err(e) = result {
            eprintln!("wip: {dir}: {e}");
        }
    }
    Ok(())
}
