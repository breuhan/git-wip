use crate::git::{Git, Result};

/// Enabled repos, kept out of the global git config because that is often read-only (home-manager).
fn repos_file() -> Result<String> {
    let dir = std::env::var("XDG_STATE_HOME")
        .or_else(|_| std::env::var("HOME").map(|h| format!("{h}/.local/state")))
        .map_err(|_| "neither XDG_STATE_HOME nor HOME is set")?
        + "/git-wip";
    std::fs::create_dir_all(&dir).map_err(|e| format!("{dir}: {e}"))?;
    Ok(dir + "/repos")
}

pub fn repo_list() -> Result<Vec<String>> {
    Ok(repos(&Git::new("."))?.lines().map(str::to_string).collect())
}

fn repos(g: &Git) -> Result<String> {
    Ok(g.run(&["config", "--file", &repos_file()?, "--get-all", "wip.repo"])
        .unwrap_or_default())
}

pub fn enable(g: &Git, remote: &str) -> Result<()> {
    let top = g.run(&["rev-parse", "--show-toplevel"])?;
    g.run(&["config", "wip.remote", remote])?;
    if !repos(g)?.lines().any(|l| l == top) {
        g.run(&["config", "--file", &repos_file()?, "--add", "wip.repo", &top])?;
    }
    Ok(())
}

pub fn disable(g: &Git) -> Result<()> {
    let top = g.run(&["rev-parse", "--show-toplevel"])?;
    let _ = g.run(&["config", "--unset", "wip.remote"]);
    let _ = g.run(&[
        "config",
        "--file",
        &repos_file()?,
        "--fixed-value",
        "--unset-all",
        "wip.repo",
        &top,
    ]);
    Ok(())
}

pub fn remote(g: &Git) -> Option<String> {
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
        let name = gethostname::gethostname().to_string_lossy().to_lowercase();
        name.split('.').next().unwrap_or_default().to_string()
    })
}

/// Per host, the newest snapshot of it this host has taken in, directly or via another host's
/// snapshot. Recorded in every snapshot as `Wip-Seen: <host> <oid>` trailers, so a host can tell
/// whether its own last save was seen before it gets replaced.
fn seen(g: &Git) -> Result<Vec<(String, String)>> {
    let text = std::fs::read_to_string(g.path("wip-seen")?).unwrap_or_default();
    Ok(parse_seen(&text))
}

fn parse_seen(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| l.trim().split_once(' ').map(|(h, o)| (h.to_string(), o.to_string())))
        .collect()
}

/// The `Wip-Seen` entries of a snapshot.
fn seen_in(g: &Git, snap: &str) -> Result<Vec<(String, String)>> {
    Ok(parse_seen(&g.run(&[
        "log",
        "-1",
        "--format=%(trailers:key=Wip-Seen,valueonly,separator=%x0A)",
        snap,
    ])?))
}

fn record_seen(g: &Git, from: &str, snap: &str) -> Result<()> {
    let mut entries = seen_in(g, snap)?;
    entries.retain(|(h, _)| h != from);
    entries.push((from.to_string(), snap.to_string()));
    let text: String = entries.iter().map(|(h, o)| format!("{h} {o}\n")).collect();
    std::fs::write(g.path("wip-seen")?, text).map_err(|e| e.to_string())
}

fn own_ref() -> String {
    format!("refs/wip/{}", host())
}

fn busy(g: &Git) -> Result<bool> {
    if !g.ok(&["symbolic-ref", "-q", "HEAD"]) || !g.ok(&["rev-parse", "-q", "--verify", "HEAD"]) {
        return Ok(true);
    }
    let mut args = vec!["rev-parse", "--path-format=absolute", "--git-dir", "--git-common-dir"];
    for p in BUSY {
        args.extend(["--git-path", p]);
    }
    let out = g.run(&args)?;
    let mut paths = out.lines();
    if paths.next() != paths.next() {
        return Ok(true); // linked worktree: refs/wip/<host> is shared with the main one
    }
    Ok(paths.any(|p| std::path::Path::new(p).exists()))
}

/// Held for the whole save or restore so the watcher, the prompt hook and manual commands never
/// interleave. Background callers skip when it is taken; explicit commands wait.
fn lock(g: &Git, wait: bool) -> Result<Option<std::fs::File>> {
    let f = std::fs::File::create(g.path("wip.lock")?).map_err(|e| e.to_string())?;
    if wait {
        f.lock().map_err(|e| e.to_string())?;
        return Ok(Some(f));
    }
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
    let trailers: Vec<String> = seen(g)?.iter().map(|(h, o)| format!("Wip-Seen: {h} {o}")).collect();
    if !trailers.is_empty() {
        args.extend(["-m".into(), trailers.join("\n")]);
    }
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

pub fn fetch(g: &Git, remote: &str) -> Result<()> {
    g.run(&[
        "fetch",
        "--quiet",
        // Only prunes within the refspec's destination, refs/wip-remotes/<remote>/.
        "--prune",
        remote,
        &format!("+refs/wip/*:refs/wip-remotes/{remote}/*"),
    ])
    .map(|_| ())
}

pub fn save(g: &Git, wait: bool) -> Result<()> {
    let Some(remote) = remote(g) else { return Ok(()) };
    if busy(g)? {
        return Ok(());
    }
    let Some(_lock) = lock(g, wait)? else { return Ok(()) };
    let snap = snapshot(g)?;
    let own = own_ref();
    if let Ok(old) = g.run(&["rev-parse", "-q", "--verify", &own]) {
        if same(g, &old, &snap)? {
            return Ok(());
        }
    }
    // Push before moving the local ref so an offline run is retried next time.
    // Snapshots are not real pushes; hooks such as CI checks would run every minute.
    g.run(&["push", "--quiet", "--no-verify", &remote, &format!("+{snap}:{own}")])?;
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
    let prefix = format!("refs/wip-remotes/{remote}/");
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

fn branch_of(subject: &str) -> Option<&str> {
    subject.strip_prefix("WIP on ")?.split_once(": ").map(|(b, _)| b)
}

enum Plan {
    UpToDate,
    Blocked {
        msg: String,
        snap: String,
    },
    /// The foreign host only has newer commits on our branch; like `git pull`, local changes stay.
    FastForward {
        from: String,
        snap: String,
        branch: String,
        base: String,
    },
    Ready {
        from: String,
        snap: String,
        current: String,
        branch: String,
        base: String,
        exists: bool,
        behind: bool,
    },
}

/// What `restore` would do with the refs fetched so far. `prompt`: the shell hook, which gives up
/// early on a snapshot it already reported.
fn plan(g: &Git, remote: &str, force: bool, prompt: bool) -> Result<Plan> {
    let own = own_ref();
    let own_oid = g.run(&["rev-parse", "-q", "--verify", &own]).ok();
    let Some((time, snap, from)) = newest_foreign(g, remote)? else {
        return Ok(Plan::UpToDate);
    };
    let me = host();
    // A snapshot made after seeing our own last save is newer whatever the clocks say.
    let causal = own_oid
        .as_ref()
        .is_some_and(|o| seen_in(g, &snap).is_ok_and(|s| s.contains(&(me.clone(), o.clone()))));
    if !causal && time <= commit_time(g, &own).max(commit_time(g, "HEAD")) {
        return Ok(Plan::UpToDate);
    }
    if prompt && !force && std::fs::read_to_string(g.path("wip-refused")?).ok() == Some(refusal_key(g, &snap)?) {
        // Refused before and nothing has changed since; skip the snapshot below, which would
        // otherwise be redone at every prompt.
        let msg = String::new();
        return Ok(Plan::Blocked { msg, snap });
    }
    // -uall: untracked files count even with status.showUntrackedFiles=no, or clean -fd deletes them.
    let clean = g.run(&["status", "--porcelain", "--untracked-files=all"])?.is_empty();
    let current = snapshot(g)?;
    let unchanged = match &own_oid {
        Some(o) => same(g, o, &current)?,
        None => false,
    };
    // Replacing our state loses nothing if the tree is clean, or if it is our last save and the
    // other host has seen that save (or it holds no changes).
    let untouched = clean || (unchanged && seen_by(g, own_oid.as_deref().unwrap_or_default(), &snap)?);
    let subject = g.run(&["log", "-1", "--format=%s", &snap])?;
    let branch = branch_of(&subject)
        .ok_or(format!("unexpected snapshot message: {subject}"))?
        .to_string();
    let base = g.run(&["rev-parse", &format!("{snap}^1")])?;
    if !force && !untouched {
        let head = g.run(&["rev-parse", "HEAD"])?;
        let on_branch = g.run(&["symbolic-ref", "--short", "HEAD"])? == branch;
        if on_branch && !has_changes(g, &snap)? {
            if is_ancestor(g, &base, &head) {
                return Ok(Plan::UpToDate); // we already have its commits and it has no changes
            }
            if is_ancestor(g, &head, &base) {
                return Ok(Plan::FastForward {
                    from,
                    snap,
                    branch,
                    base,
                });
            }
        }
        let msg = if unchanged {
            format!(
                "{from} and {me} both have changes, run `git wip restore --merge` to combine them \
                 or `git wip restore --force` to take {from}'s (yours is kept in refs/wip-backup/{me})"
            )
        } else {
            format!("{from} has newer changes, run `git wip restore --force`")
        };
        return Ok(Plan::Blocked { msg, snap });
    }
    let local = format!("refs/heads/{branch}");
    let exists = g.ok(&["rev-parse", "-q", "--verify", &local]);
    let behind = exists && is_ancestor(g, &local, &base);
    if exists && !behind && !is_ancestor(g, &base, &local) {
        let msg = format!("{branch} has diverged from {from}, not restoring");
        return Ok(Plan::Blocked { msg, snap });
    }
    Ok(Plan::Ready {
        from,
        snap,
        current,
        branch,
        base,
        exists,
        behind,
    })
}

/// Whether `foreign` was made after seeing our own last save `own`, so replacing it loses nothing.
/// True as well when `own` is a snapshot we restored rather than made, or holds no changes.
fn seen_by(g: &Git, own: &str, foreign: &str) -> Result<bool> {
    if seen(g)?.iter().any(|(_, o)| o == own) || !has_changes(g, own)? {
        return Ok(true);
    }
    let me = host();
    let Some((_, theirs)) = seen_in(g, foreign)?.into_iter().find(|(h, _)| *h == me) else {
        return Ok(false);
    };
    // A commit or pull re-makes our snapshot on the new HEAD with the same changes, so compare those.
    Ok(theirs == own || changes(g, &theirs).is_ok_and(|seen| Some(seen) == changes(g, own).ok()))
}

/// What a snapshot changes relative to its base: resulting mode, content and path of every
/// staged and unstaged file, plus the untracked files.
fn changes(g: &Git, snap: &str) -> Result<String> {
    let mut out = String::new();
    for side in ["", "^2"] {
        let diff = g.run(&["diff-tree", "-r", &format!("{snap}^1"), &format!("{snap}{side}")])?;
        for line in diff.lines() {
            let (meta, path) = line.split_once('\t').unwrap_or((line, ""));
            let f: Vec<&str> = meta.split(' ').collect();
            out += &format!(
                "{side} {} {} {} {path}\n",
                f.get(1).unwrap_or(&""),
                f.get(3).unwrap_or(&""),
                f.get(4).unwrap_or(&"")
            );
        }
    }
    out += &g
        .run(&["rev-parse", "-q", "--verify", &format!("{snap}^3^{{tree}}")])
        .unwrap_or_default();
    Ok(out)
}

/// Everything a refusal depends on: the foreign snapshot, our own snapshot, HEAD and the working
/// tree. The prompt hook re-checks as soon as any of it changes.
fn refusal_key(g: &Git, snap: &str) -> Result<String> {
    let own = g.run(&["rev-parse", "-q", "--verify", &own_ref()]).unwrap_or_default();
    let head = g.run(&["rev-parse", "HEAD"])?;
    let status = g.run(&["status", "--porcelain", "--untracked-files=all"])?;
    Ok(format!("{snap} {own} {head}\n{status}"))
}

fn record_refusal(g: &Git, snap: &str) -> Result<()> {
    std::fs::write(g.path("wip-refused")?, refusal_key(g, snap)?).map_err(|e| e.to_string())
}

/// The foreign snapshot a refusal was last printed for.
fn notified(g: &Git) -> Result<String> {
    Ok(std::fs::read_to_string(g.path("wip-notified")?).unwrap_or_default())
}

/// Prints a refusal only once per foreign snapshot, since the prompt hook runs restore at every prompt.
fn notify_once(g: &Git, snap: &str, msg: &str) -> Result<()> {
    if notified(g)? == snap {
        return Ok(());
    }
    eprintln!("wip: {msg}");
    std::fs::write(g.path("wip-notified")?, snap).map_err(|e| e.to_string())
}

/// `prompt`: the shell hook, which never waits for the lock and reports each refusal once.
pub fn restore(g: &Git, force: bool, fetch_first: bool, prompt: bool) -> Result<()> {
    let Some(remote) = remote(g) else { return Ok(()) };
    if busy(g)? {
        return Ok(());
    }
    let Some(_lock) = lock(g, !prompt)? else { return Ok(()) };
    if fetch_first {
        fetch(g, &remote)?;
    }
    // A manual restore always explains; the prompt hook only says it once per foreign snapshot.
    let refuse = |snap: &str, msg: &str| {
        if !msg.is_empty() {
            record_refusal(g, snap)?;
        }
        if prompt {
            return notify_once(g, snap, msg);
        }
        eprintln!("wip: {msg}");
        std::fs::write(g.path("wip-notified")?, snap).map_err(|e| e.to_string())
    };
    let (from, snap, current, branch, base, exists, behind) = match plan(g, &remote, force, prompt)? {
        Plan::UpToDate => return Ok(()),
        Plan::Blocked { msg, snap } => return refuse(&snap, &msg),
        Plan::FastForward {
            from,
            snap,
            branch,
            base,
        } => {
            // --ff-only refuses when the new commits touch locally changed files.
            if g.run(&["merge", "--ff-only", "-q", &base]).is_ok() {
                eprintln!("wip: fast-forwarded {branch} to {from}'s commits, local changes kept");
                return Ok(());
            }
            return refuse(
                &snap,
                &format!("{from} has newer changes, run `git wip restore --force`"),
            );
        }
        Plan::Ready {
            from,
            snap,
            current,
            branch,
            base,
            exists,
            behind,
        } => (from, snap, current, branch, base, exists, behind),
    };

    let backup = format!("refs/wip-backup/{}", host());
    g.run(&[
        "update-ref",
        "--create-reflog",
        "-m",
        "git wip restore",
        &backup,
        &current,
    ])?;
    let old_branch = g.run(&["symbolic-ref", "--short", "HEAD"])?;
    let apply = || -> Result<()> {
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
        Ok(())
    };
    if let Err(e) = apply() {
        // Put our state back, and remember the refusal so the prompt hook does not retry (and
        // re-backup a half-restored tree) at every prompt.
        let undo = || -> Result<()> {
            g.run(&["reset", "--hard", "-q"])?;
            g.run(&["clean", "-fdq"])?;
            g.run(&["checkout", "-q", &old_branch])?;
            g.run(&["reset", "--hard", "-q", &format!("{current}^1")])?;
            if has_changes(g, &current)? {
                g.run(&["stash", "apply", "--index", "-q", &current])?;
            }
            Ok(())
        };
        let undone = match undo() {
            Ok(()) => "your state was put back".to_string(),
            Err(u) => format!("putting your state back failed too ({u})"),
        };
        std::fs::write(g.path("wip-notified")?, &snap).map_err(|e| e.to_string())?;
        record_refusal(g, &snap)?;
        return Err(format!(
            "could not apply {from}'s state: {e}\n{undone}; it is also in {backup}"
        ));
    }
    g.run(&["update-ref", &own_ref(), &snap])?;
    record_seen(g, &from, &snap)?;
    eprintln!("wip: restored state from {from} (previous state in {backup})");
    Ok(())
}

/// Tree of a snapshot's working tree including its untracked files (or of a plain commit).
fn full_tree(g: &Git, snap: &str) -> Result<String> {
    let idx = g.path("wip-index")?;
    let _ = std::fs::remove_file(&idx);
    let env = [("GIT_INDEX_FILE", idx.to_str().ok_or("non-utf8 git dir")?)];
    let build = || -> Result<String> {
        g.run_with(&["read-tree", &format!("{snap}^{{tree}}")], &env, None)?;
        if let Ok(files) = g.run(&["ls-tree", "-r", "-z", &format!("{snap}^3")]) {
            g.run_with(&["update-index", "-z", "--index-info"], &env, Some(files.as_bytes()))?;
        }
        g.run_with(&["write-tree"], &env, None)
    };
    let tree = build();
    let _ = std::fs::remove_file(&idx);
    tree
}

/// Three-way merge of the newest foreign working state into ours, like `git merge` for
/// uncommitted work: the ancestor is our last state the other host had seen, conflicts get
/// markers in the files. Nothing is staged afterwards.
pub fn merge(g: &Git, fetch_first: bool) -> Result<()> {
    let Some(remote) = remote(g) else { return Ok(()) };
    if busy(g)? {
        return Err("detached HEAD or an operation in progress, not merging".into());
    }
    let Some(_lock) = lock(g, true)? else { return Ok(()) };
    if fetch_first {
        fetch(g, &remote)?;
    }
    let foreign = newest_foreign(g, &remote)?;
    let Some((_, snap, from)) = foreign.filter(|(_, s, _)| !seen(g).is_ok_and(|v| v.iter().any(|(_, o)| o == s)))
    else {
        eprintln!("wip: nothing to merge");
        return Ok(());
    };
    let me = host();
    let subject = g.run(&["log", "-1", "--format=%s", &snap])?;
    let branch = branch_of(&subject).ok_or(format!("unexpected snapshot message: {subject}"))?;
    let ours = g.run(&["symbolic-ref", "--short", "HEAD"])?;
    if branch != ours {
        return Err(format!(
            "{from} is on {branch}, {me} on {ours}; merging needs the same branch"
        ));
    }
    let base = g.run(&["rev-parse", &format!("{snap}^1")])?;
    let head = g.run(&["rev-parse", "HEAD"])?;
    let behind = is_ancestor(g, &head, &base);
    if !behind && !is_ancestor(g, &base, &head) {
        return Err(format!(
            "{branch} has diverged from {from}; merge or rebase the commits first"
        ));
    }
    let current = snapshot(g)?;
    let ancestor = seen_in(g, &snap)?
        .into_iter()
        .find(|(h, o)| *h == me && g.ok(&["cat-file", "-e", &format!("{o}^{{commit}}")]))
        .map_or(base.clone(), |(_, o)| o);

    let ours_tree = full_tree(g, &current)?;
    let commit = |tree: &str| g.run(&["commit-tree", tree, "-m", "git wip merge"]);
    let sides = [
        commit(&full_tree(g, &ancestor)?)?,
        commit(&ours_tree)?,
        commit(&full_tree(g, &snap)?)?,
    ];
    let merge_base = format!("--merge-base={}", sides[0]);
    let args = [
        "merge-tree",
        "--write-tree",
        "--name-only",
        "--no-messages",
        &merge_base,
        &sides[1],
        &sides[2],
    ];
    let (code, out, err) = g.run_code(&args, &[], None)?;
    let mut lines = out.lines();
    let merged = match (code, lines.next()) {
        (0 | 1, Some(tree)) => tree,
        _ => return Err(err),
    };
    let conflicts: Vec<&str> = lines.filter(|l| !l.is_empty()).collect();

    // Git would overwrite an ignored file that a file from the other host collides with, and
    // ignored files are in no backup.
    let added = g.run(&[
        "diff-tree",
        "-r",
        "-z",
        "--name-only",
        "--diff-filter=A",
        &ours_tree,
        merged,
    ])?;
    let top = std::path::PathBuf::from(g.run(&["rev-parse", "--show-toplevel"])?);
    if let Some(path) = added
        .split('\0')
        .find(|p| !p.is_empty() && top.join(p).symlink_metadata().is_ok())
    {
        return Err(format!(
            "{path} from {from} exists here as an ignored file\nnothing was changed"
        ));
    }

    let backup = format!("refs/wip-backup/{me}");
    g.run(&[
        "update-ref",
        "--create-reflog",
        "-m",
        "git wip restore --merge",
        &backup,
        &current,
    ])?;
    // With everything in the index, the two-tree read-tree also removes files the merge deleted.
    g.run(&["read-tree", &ours_tree])?;
    g.run(&["update-index", "-q", "--refresh"])?;
    if let Err(e) = g.run(&["read-tree", "-u", "-m", &ours_tree, merged]) {
        let _ = g.run(&["read-tree", &format!("{current}^2")]);
        return Err(format!("{e}\nnothing was changed"));
    }
    if behind {
        g.run(&["reset", "--soft", &base])?;
    }
    g.run(&["reset", "-q"])?;
    record_seen(g, &from, &snap)?;
    if conflicts.is_empty() {
        eprintln!("wip: merged state from {from} (previous state in {backup})");
        return Ok(());
    }
    Err(format!(
        "merged state from {from} with conflicts in: {} (in the markers yours comes first; \
         previous state in {backup})",
        conflicts.join(", ")
    ))
}

/// Snapshots per host as of the last fetch, and what `restore` would do.
pub fn status(g: &Git) -> Result<()> {
    let Some(remote) = remote(g) else {
        println!("not enabled, run `git wip enable <remote>`");
        return Ok(());
    };
    println!("remote: {remote}");
    let prefix = format!("refs/wip-remotes/{remote}/");
    let me = host();
    let refs = g.run(&[
        "for-each-ref",
        "--format=%(refname)%00%(committerdate:relative)%00%(subject)",
        &prefix,
    ])?;
    for line in refs.lines() {
        let mut it = line.split('\0');
        let (Some(name), Some(date), Some(subject)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let name = name.strip_prefix(&prefix).unwrap_or(name);
        let mark = if name == me { " (this host)" } else { "" };
        println!("{name:<12} {:<24} {date}{mark}", branch_of(subject).unwrap_or("?"));
    }
    if busy(g)? {
        println!("busy (detached HEAD or an operation in progress), nothing is saved or restored");
        return Ok(());
    }
    match plan(g, &remote, false, false)? {
        Plan::UpToDate => println!("up to date"),
        Plan::Blocked { msg, .. } => println!("{msg}"),
        Plan::Ready { from, .. } => println!("restore pending from {from}"),
        Plan::FastForward { from, .. } => println!("fast-forward pending from {from}, local changes kept"),
    }
    Ok(())
}

pub fn save_all() -> Result<()> {
    let mut failed = 0;
    for dir in repos(&Git::new("."))?.lines() {
        let g = Git::new(dir);
        let result = save(&g, true).and_then(|()| remote(&g).map_or(Ok(()), |r| fetch(&g, &r)));
        if let Err(e) = result {
            eprintln!("wip: {dir}: {e}");
            failed += 1;
        }
    }
    match failed {
        0 => Ok(()),
        n => Err(format!("{n} repo(s) failed")),
    }
}
