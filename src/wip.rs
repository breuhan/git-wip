use crate::git::{Git, Result};

/// Enabled repos, kept out of the global git config because that is often read-only (home-manager).
fn repos_file() -> Result<String> {
    let dir = std::env::var("XDG_STATE_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.local/state")))
        .ok_or("neither XDG_STATE_HOME nor HOME is set")?
        + "/git-wip";
    std::fs::create_dir_all(&dir).map_err(|e| format!("{dir}: {e}"))?;
    Ok(dir + "/repos")
}

/// One path per line, read without starting git: the prompt hook does this in every directory.
/// Lines of the earlier git-config format (`repo = <path>` under `[wip]`) are still understood.
pub fn repo_list() -> Result<Vec<String>> {
    let text = std::fs::read_to_string(repos_file()?).unwrap_or_default();
    let paths = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('['));
    Ok(paths.map(|l| l.strip_prefix("repo = ").unwrap_or(l).to_string()).collect())
}

fn write_repo_list(repos: &[String]) -> Result<()> {
    let text: String = repos.iter().map(|r| format!("{r}\n")).collect();
    std::fs::write(repos_file()?, text).map_err(|e| e.to_string())
}

/// The innermost enabled repo containing `dir`. The prompt hook runs in every directory and
/// must not run git in (and so trust the config of) a repo the user never enabled.
pub fn enabled_repo(dir: &std::path::Path) -> Result<Option<String>> {
    let dir = std::fs::canonicalize(dir).map_err(|e| e.to_string())?;
    let inside = |r: &String| std::fs::canonicalize(r).is_ok_and(|r| dir.starts_with(r));
    Ok(repo_list()?.into_iter().filter(inside).max_by_key(String::len))
}

pub fn enable(g: &Git, remote: &str) -> Result<()> {
    if remote.starts_with('-') {
        return Err(format!("invalid remote name: {remote}"));
    }
    let top = g.run(&["rev-parse", "--show-toplevel"])?;
    g.run(&["config", "wip.remote", remote])?;
    let mut repos = repo_list()?;
    if !repos.contains(&top) {
        repos.push(top);
    }
    write_repo_list(&repos)
}

pub fn disable(g: &Git) -> Result<()> {
    let top = g.run(&["rev-parse", "--show-toplevel"])?;
    let _ = g.run(&["config", "--unset", "wip.remote"]);
    let mut repos = repo_list()?;
    repos.retain(|r| *r != top);
    write_repo_list(&repos)
}

pub fn remote(g: &Git) -> Option<String> {
    // A value starting with a dash would be taken as an option by fetch and push.
    g.run(&["config", "--get", "wip.remote"]).ok().filter(|r| !r.starts_with('-'))
}

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

/// Trailers come from the remote: only plain host names and full object ids are accepted, since
/// the ids are passed to git as revisions.
fn parse_seen(text: &str) -> Vec<(String, String)> {
    let host_ok = |h: &str| {
        !h.is_empty() && !h.starts_with('-') && h.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    };
    let oid_ok = |o: &str| matches!(o.len(), 40 | 64) && o.chars().all(|c| c.is_ascii_hexdigit());
    text.lines()
        .filter_map(|l| l.trim().split_once(' '))
        .filter(|(h, o)| host_ok(h) && oid_ok(o))
        .map(|(h, o)| (h.to_string(), o.to_string()))
        .collect()
}

/// The branch named in a snapshot's subject. It comes from the remote and is passed to checkout.
fn snapshot_branch(g: &Git, snap: &str) -> Result<String> {
    let subject = g.run(&["log", "-1", "--format=%s", snap])?;
    let valid = |b: &&str| !b.starts_with('-') && g.ok(&["check-ref-format", "--branch", b]);
    Ok(branch_of(&subject).filter(valid).ok_or("invalid branch name in the snapshot, not restoring")?.to_string())
}

fn seen_in(g: &Git, snap: &str) -> Result<Vec<(String, String)>> {
    Ok(parse_seen(&g.run(&["log", "-1", "--format=%(trailers:key=Wip-Seen,valueonly,separator=%x0A)", snap])?))
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
    const IN_PROGRESS: [&str; 6] =
        ["rebase-merge", "rebase-apply", "MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD", "BISECT_LOG"];
    if !g.ok(&["symbolic-ref", "-q", "HEAD"]) || g.rev("HEAD").is_none() {
        return Ok(true);
    }
    let mut args = vec!["rev-parse", "--path-format=absolute", "--git-dir", "--git-common-dir"];
    for p in IN_PROGRESS {
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
        (g.run(&["rev-parse", &format!("{w}^{{tree}}")])?, g.run(&["rev-parse", &format!("{w}^2")])?)
    };
    let untracked = untracked(g, &branch)?;
    // A host's first clean snapshot dates from its HEAD commit, so enabling an idle host never
    // looks newer than real WIP elsewhere.
    let idle = g.rev(&own_ref()).is_none() && w.is_empty() && untracked.is_none();
    let date = if idle { g.run(&["log", "-1", "--format=%cI", "HEAD"])? } else { String::new() };
    let subject = format!("WIP on {branch}: {head}");
    let trailers = seen(g)?.iter().map(|(h, o)| format!("Wip-Seen: {h} {o}")).collect::<Vec<_>>().join("\n");
    let mut args = vec!["commit-tree", &tree, "-p", &head, "-p", &index];
    if let Some(u) = &untracked {
        args.extend(["-p", u]);
    }
    args.extend(["-m", &subject]);
    if !trailers.is_empty() {
        args.extend(["-m", &trailers]);
    }
    let env: &[(&str, &str)] = if idle { &[("GIT_COMMITTER_DATE", &date)] } else { &[] };
    g.run_with(&args, env, None)
}

fn untracked(g: &Git, branch: &str) -> Result<Option<String>> {
    let files = g.run(&["ls-files", "-z", "--others", "--exclude-standard"])?;
    if files.is_empty() {
        return Ok(None);
    }
    // The index is kept between saves: git then skips re-reading untracked files that did not
    // change, which matters for large ones. Entries for files no longer untracked are dropped.
    let idx = g.path("wip-untracked-index")?;
    let env = [("GIT_INDEX_FILE", idx.to_str().ok_or("non-utf8 git dir")?)];
    let now: std::collections::HashSet<&str> = files.split('\0').collect();
    let tree = || -> Result<String> {
        let before = g.run_with(&["ls-files", "-z"], &env, None)?;
        let gone: String =
            before.split('\0').filter(|p| !p.is_empty() && !now.contains(p)).flat_map(|p| [p, "\0"]).collect();
        g.run_with(&["update-index", "--force-remove", "-z", "--stdin"], &env, Some(gone.as_bytes()))?;
        g.run_with(&["update-index", "--add", "-z", "--stdin"], &env, Some(files.as_bytes()))?;
        g.run_with(&["write-tree"], &env, None)
    };
    // A damaged index must not fail every later save; the next one starts from scratch.
    let tree = tree().inspect_err(|_| drop(std::fs::remove_file(&idx)))?;
    Ok(Some(g.run(&["commit-tree", &tree, "-m", &format!("untracked files on {branch}")])?))
}

fn state(g: &Git, c: &str) -> Result<String> {
    let mut s = g.run(&["rev-parse", &format!("{c}^{{tree}}"), &format!("{c}^1"), &format!("{c}^2^{{tree}}")])?;
    s.push_str(&g.rev(&format!("{c}^3^{{tree}}")).unwrap_or_default());
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
        "--no-tags",
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
    if let Some(old) = g.rev(&own)
        && same(g, &old, &snap)?
    {
        return Ok(());
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
    g.run(&["log", "-1", "--format=%ct", rev]).ok().and_then(|t| t.parse().ok()).unwrap_or(0)
}

/// (commit time, oid, host) of the newest snapshot fetched from another host.
fn newest_foreign(g: &Git, remote: &str) -> Result<Option<(i64, String, String)>> {
    let prefix = format!("refs/wip-remotes/{remote}/");
    let me = host();
    let out = g.run(&["for-each-ref", "--format=%(committerdate:unix) %(objectname) %(refname)", &prefix])?;
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
    Ok(g.rev(&format!("{snap}^3")).is_some() || trees.lines().any(|t| t != base))
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
    Ready(Ready),
}

struct Ready {
    from: String,
    snap: String,
    /// Snapshot of our own state, for the backup.
    current: String,
    branch: String,
    base: String,
    /// Whether the branch exists here already.
    exists: bool,
}

fn newer_changes(from: &str) -> String {
    format!("{from} has newer changes, run `git wip restore --force`")
}

/// What `restore` would do with the refs fetched so far. `prompt`: the shell hook, which gives up
/// early on a snapshot it already reported.
fn plan(g: &Git, remote: &str, force: bool, prompt: bool) -> Result<Plan> {
    let own = own_ref();
    let own_oid = g.rev(&own);
    let Some((time, snap, from)) = newest_foreign(g, remote)? else {
        return Ok(Plan::UpToDate);
    };
    let me = host();
    // A snapshot made after seeing our own last save is newer whatever the clocks say.
    let causal =
        own_oid.as_ref().is_some_and(|o| seen_in(g, &snap).is_ok_and(|s| s.iter().any(|(h, s)| *h == me && s == o)));
    // Our own save may only repeat a state we restored from another host; then it is neither
    // our work nor any newer than that state.
    let foreign_seen = |seen: Vec<(String, String)>| seen.into_iter().filter(|(h, _)| *h != me).map(|(_, o)| o);
    let restored = own_oid.as_deref().and_then(|o| repeats(g, o, foreign_seen(seen(g).ok()?)));
    let own_time = commit_time(g, restored.as_deref().unwrap_or(&own));
    if !causal && time <= own_time.max(commit_time(g, "HEAD")) {
        return Ok(Plan::UpToDate);
    }
    // Refused before and nothing has changed since: skip the snapshot below, which would
    // otherwise be redone at every prompt.
    if prompt && !force && refusal(g)? == refusal_key(g, &snap)? {
        return Ok(Plan::UpToDate);
    }
    // -uall: untracked files count even with status.showUntrackedFiles=no, or clean -fd deletes them.
    let clean = g.run(&["status", "--porcelain", "--untracked-files=all"])?.is_empty();
    let current = snapshot(g)?;
    let unchanged = own_oid.as_deref().map_or(Ok(false), |o| same(g, o, &current))?;
    // Replacing our state loses nothing if the tree is clean, or if it is our last save and that
    // save is a restored state, holds no changes, or has been seen by the other host.
    let untouched =
        clean || (unchanged && (restored.is_some() || seen_by_them(g, own_oid.as_deref().unwrap_or_default(), &snap)?));
    let branch = snapshot_branch(g, &snap)?;
    let base = g.run(&["rev-parse", &format!("{snap}^1")])?;
    if !force && !untouched {
        let head = g.run(&["rev-parse", "HEAD"])?;
        let on_branch = g.run(&["symbolic-ref", "--short", "HEAD"])? == branch;
        // The other host has nothing of its own if its snapshot only repeats one of ours it saw.
        let ours_it_saw = seen_in(g, &snap)?.into_iter().filter(|(h, _)| *h == me).map(|(_, o)| o);
        let theirs_new = has_changes(g, &snap)? && repeats(g, &snap, ours_it_saw).is_none();
        if on_branch && !theirs_new {
            if is_ancestor(g, &base, &head) {
                return Ok(Plan::UpToDate); // we already have its commits and it has nothing new
            }
            if is_ancestor(g, &head, &base) {
                return Ok(Plan::FastForward { from, snap, branch, base });
            }
        }
        let msg = if unchanged {
            format!(
                "{from} and {me} both have changes, run `git wip restore --merge` to combine them \
                 or `git wip restore --force` to take {from}'s (yours is kept in refs/wip-backup/{me})"
            )
        } else {
            newer_changes(&from)
        };
        return Ok(Plan::Blocked { msg, snap });
    }
    let local = format!("refs/heads/{branch}");
    let exists = g.rev(&local).is_some();
    if exists && !is_ancestor(g, &local, &base) && !is_ancestor(g, &base, &local) {
        let msg = format!("{branch} has diverged from {from}, not restoring");
        return Ok(Plan::Blocked { msg, snap });
    }
    Ok(Plan::Ready(Ready { from, snap, current, branch, base, exists }))
}

/// Whether `foreign` was made after seeing our own last save `own`, so replacing it loses nothing.
/// True as well when `own` holds no changes.
fn seen_by_them(g: &Git, own: &str, foreign: &str) -> Result<bool> {
    let me = host();
    let ours_it_saw = seen_in(g, foreign)?.into_iter().filter(|(h, _)| *h == me).map(|(_, o)| o);
    Ok(!has_changes(g, own)? || repeats(g, own, ours_it_saw).is_some())
}

/// The snapshot among `candidates` that `snap` is, or whose changes it repeats. A commit or pull
/// re-makes a snapshot on the new HEAD: same changes, but a new id and a new time.
fn repeats(g: &Git, snap: &str, mut candidates: impl Iterator<Item = String>) -> Option<String> {
    let changed = changes(g, snap).ok()?;
    candidates.find(|c| c == snap || changes(g, c).is_ok_and(|other| other == changed))
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
    out += &g.rev(&format!("{snap}^3^{{tree}}")).unwrap_or_default();
    Ok(out)
}

/// Everything a refusal depends on: the foreign snapshot, our own snapshot, HEAD and the working
/// tree. The prompt hook re-checks as soon as any of it changes.
fn refusal_key(g: &Git, snap: &str) -> Result<String> {
    let own = g.rev(&own_ref()).unwrap_or_default();
    let head = g.run(&["rev-parse", "HEAD"])?;
    let status = g.run(&["status", "--porcelain", "--untracked-files=all"])?;
    Ok(format!("{snap} {own} {head}\n{status}"))
}

/// The key of the last refusal.
fn refusal(g: &Git) -> Result<String> {
    Ok(std::fs::read_to_string(g.path("wip-refused")?).unwrap_or_default())
}

fn record_refusal(g: &Git, snap: &str) -> Result<()> {
    std::fs::write(g.path("wip-refused")?, refusal_key(g, snap)?).map_err(|e| e.to_string())
}

/// Stores our state in the backup ref (with a reflog) before it gets replaced.
fn backup(g: &Git, current: &str, action: &str) -> Result<String> {
    let backup = format!("refs/wip-backup/{}", host());
    g.run(&["update-ref", "--create-reflog", "-m", action, &backup, current])?;
    Ok(backup)
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
        if !prompt || !refusal(g)?.starts_with(snap) {
            eprintln!("wip: {msg}");
        }
        record_refusal(g, snap)
    };
    let Ready { from, snap, current, branch, base, exists } = match plan(g, &remote, force, prompt)? {
        Plan::UpToDate => return Ok(()),
        Plan::Blocked { msg, snap } => return refuse(&snap, &msg),
        Plan::FastForward { from, snap, branch, base } => {
            // --ff-only refuses when the new commits touch locally changed files.
            if g.run(&["merge", "--ff-only", "--no-overwrite-ignore", "-q", &base]).is_ok() {
                eprintln!("wip: fast-forwarded {branch} to {from}'s commits, local changes kept");
                return Ok(());
            }
            return refuse(&snap, &newer_changes(&from));
        }
        Plan::Ready(ready) => ready,
    };

    let backup = backup(g, &current, "git wip restore")?;
    let old_branch = g.run(&["symbolic-ref", "--short", "HEAD"])?;
    let apply = || -> Result<()> {
        g.run(&["reset", "--hard", "-q"])?;
        g.run(&["clean", "-fdq"])?;
        // Ignored files are in no backup, so git must not overwrite them with incoming ones.
        if exists {
            g.run(&["checkout", "-q", "--no-overwrite-ignore", &branch])?;
            g.run(&["merge", "--ff-only", "--no-overwrite-ignore", "-q", &base])?;
        } else {
            g.run(&["checkout", "-q", "--no-overwrite-ignore", "-b", &branch, &base])?;
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
            g.run(&["checkout", "-q", "-B", &old_branch, &format!("{current}^1")])?;
            if has_changes(g, &current)? {
                g.run(&["stash", "apply", "--index", "-q", &current])?;
            }
            Ok(())
        };
        let undone = match undo() {
            Ok(()) => "your state was put back".to_string(),
            Err(u) => format!("putting your state back failed too ({u})"),
        };
        record_refusal(g, &snap)?;
        return Err(format!("could not apply {from}'s state: {e}\n{undone}; it is also in {backup}"));
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
    let branch = snapshot_branch(g, &snap)?;
    let ours = g.run(&["symbolic-ref", "--short", "HEAD"])?;
    if branch != ours {
        return Err(format!("{from} is on {branch}, {me} on {ours}; merging needs the same branch"));
    }
    let base = g.run(&["rev-parse", &format!("{snap}^1")])?;
    let head = g.run(&["rev-parse", "HEAD"])?;
    let behind = is_ancestor(g, &head, &base);
    if !behind && !is_ancestor(g, &base, &head) {
        return Err(format!("{branch} has diverged from {from}; merge or rebase the commits first"));
    }
    let current = snapshot(g)?;
    let ancestor = seen_in(g, &snap)?
        .into_iter()
        .find(|(h, o)| *h == me && g.ok(&["cat-file", "-e", &format!("{o}^{{commit}}")]))
        .map_or(base.clone(), |(_, o)| o);

    let ours_tree = full_tree(g, &current)?;
    let commit = |tree: &str| g.run(&["commit-tree", tree, "-m", "git wip merge"]);
    let sides = [commit(&full_tree(g, &ancestor)?)?, commit(&ours_tree)?, commit(&full_tree(g, &snap)?)?];
    let merge_base = format!("--merge-base={}", sides[0]);
    let args = ["merge-tree", "--write-tree", "--name-only", "--no-messages", &merge_base, &sides[1], &sides[2]];
    let (code, out, err) = g.run_code(&args, &[], None)?;
    let mut lines = out.lines();
    let (0 | 1, Some(merged)) = (code, lines.next()) else { return Err(err) };
    let conflicts: Vec<&str> = lines.filter(|l| !l.is_empty()).collect();

    // Git would overwrite an ignored file that a file from the other host collides with, and
    // ignored files are in no backup.
    let added = g.run(&["diff-tree", "-r", "-z", "--name-only", "--diff-filter=A", &ours_tree, merged])?;
    let top = std::path::PathBuf::from(g.run(&["rev-parse", "--show-toplevel"])?);
    if let Some(path) = added.split('\0').find(|p| !p.is_empty() && top.join(p).symlink_metadata().is_ok()) {
        return Err(format!("{path} from {from} exists here as an ignored file\nnothing was changed"));
    }

    let backup = backup(g, &current, "git wip restore --merge")?;
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
    let refs = g.run(&["for-each-ref", "--format=%(refname)%00%(committerdate:relative)%00%(subject)", &prefix])?;
    for line in refs.lines() {
        let mut it = line.split('\0');
        let (Some(name), Some(date), Some(subject)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let name = name.strip_prefix(&prefix).unwrap_or(name);
        let mark = if name == me { " (this host)" } else { "" };
        // The subject comes from the remote; keep terminal control characters out of the output.
        let branch: String = branch_of(subject).unwrap_or("?").chars().filter(|c| !c.is_control()).collect();
        println!("{name:<12} {branch:<24} {date}{mark}");
    }
    if busy(g)? {
        println!("busy (detached HEAD or an operation in progress), nothing is saved or restored");
        return Ok(());
    }
    // plan() snapshots through the same temporary index as a concurrent save.
    let _lock = lock(g, true)?;
    match plan(g, &remote, false, false)? {
        Plan::UpToDate => println!("up to date"),
        Plan::Blocked { msg, .. } => println!("{msg}"),
        Plan::Ready(Ready { from, .. }) => println!("restore pending from {from}"),
        Plan::FastForward { from, .. } => println!("fast-forward pending from {from}, local changes kept"),
    }
    Ok(())
}

pub fn save_all() -> Result<()> {
    let mut failed = 0;
    for dir in repo_list()? {
        let g = Git::new(&dir);
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
