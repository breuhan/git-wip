mod common;
use common::Env;

fn dirty_a_on_feature(env: &Env) {
    env.git(&env.a, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(env.a.join("file.txt"), "from a\n").unwrap();
    std::fs::write(env.a.join("staged.txt"), "staged\n").unwrap();
    env.git(&env.a, &["add", "staged.txt"]);
    std::fs::write(env.a.join("new.txt"), "untracked\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
}

#[test]
fn round_trip_with_branch_staged_and_untracked() {
    let env = Env::new();
    dirty_a_on_feature(&env);
    let msg = env.wip_ok(&env.b, "b", 200, &["restore"]);
    assert!(msg.contains("restored state from a"), "{msg}");
    assert_eq!(env.git(&env.b, &["symbolic-ref", "--short", "HEAD"]), "feature");
    assert_eq!(env.read(&env.b, "file.txt"), "from a\n");
    assert_eq!(env.read(&env.b, "new.txt"), "untracked\n");
    assert_eq!(
        env.git(&env.b, &["status", "--porcelain"]),
        " M file.txt\nA  staged.txt\n?? new.txt"
    );
}

#[test]
fn dirty_tree_refuses_and_force_restores_with_backup() {
    let env = Env::new();
    dirty_a_on_feature(&env);
    std::fs::write(env.b.join("file.txt"), "local b\n").unwrap();
    let msg = env.wip_ok(&env.b, "b", 200, &["restore"]);
    assert!(
        msg.contains("a has newer changes, run `git wip restore --force`"),
        "{msg}"
    );
    assert_eq!(env.read(&env.b, "file.txt"), "local b\n");

    env.wip_ok(&env.b, "b", 200, &["restore", "--force"]);
    assert_eq!(env.read(&env.b, "file.txt"), "from a\n");
    let backup = env.git(&env.b, &["rev-parse", "refs/wip-backup/b"]);
    assert_eq!(env.git(&env.b, &["show", &format!("{backup}:file.txt")]), "local b");
}

#[test]
fn untouched_host_follows() {
    let env = Env::new();
    dirty_a_on_feature(&env);
    env.wip_ok(&env.b, "b", 150, &["restore"]);
    std::fs::write(env.b.join("file.txt"), "from b\n").unwrap();
    env.wip_ok(&env.b, "b", 200, &["save"]);
    env.wip_ok(&env.a, "a", 300, &["restore"]);
    assert_eq!(env.read(&env.a, "file.txt"), "from b\n");
}

#[test]
fn restore_is_idempotent_and_does_not_ping_pong() {
    let env = Env::new();
    dirty_a_on_feature(&env);
    env.wip_ok(&env.b, "b", 200, &["restore"]);
    let b_ref = env.git(&env.b, &["rev-parse", "refs/wip/b"]);
    env.wip_ok(&env.b, "b", 250, &["save"]);
    assert_eq!(env.remote_ref("refs/wip/b"), None, "applied state is not pushed back");
    let msg = env.wip_ok(&env.b, "b", 300, &["restore"]);
    assert!(msg.is_empty(), "{msg}");
    assert_eq!(env.git(&env.b, &["rev-parse", "refs/wip/b"]), b_ref);
}

#[test]
fn unpushed_commits_travel() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "committed\n").unwrap();
    env.git(&env.a, &["commit", "-q", "-am", "local only"]);
    std::fs::write(env.a.join("file.txt"), "committed\nplus wip\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    env.wip_ok(&env.b, "b", 200, &["restore"]);
    assert_eq!(
        env.git(&env.b, &["rev-parse", "main"]),
        env.git(&env.a, &["rev-parse", "main"])
    );
    assert_eq!(env.read(&env.b, "file.txt"), "committed\nplus wip\n");
}

#[test]
fn clean_state_supersedes_wip() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "wip\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    env.git_at(&env.a, 150, &["commit", "-q", "-am", "done"]);
    env.wip_ok(&env.a, "a", 200, &["save"]);
    env.wip_ok(&env.b, "b", 300, &["restore"]);
    assert_eq!(
        env.git(&env.b, &["rev-parse", "main"]),
        env.git(&env.a, &["rev-parse", "main"])
    );
    assert_eq!(env.git(&env.b, &["status", "--porcelain"]), "");
}

#[test]
fn diverged_branch_is_refused() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "a commit\n").unwrap();
    env.git_at(&env.a, 100, &["commit", "-q", "-am", "a"]);
    env.wip_ok(&env.a, "a", 100, &["save"]);
    std::fs::write(env.b.join("other.txt"), "b commit\n").unwrap();
    env.git(&env.b, &["add", "other.txt"]);
    env.git(&env.b, &["commit", "-q", "-m", "b"]);
    let head = env.git(&env.b, &["rev-parse", "HEAD"]);
    let msg = env.wip_ok(&env.b, "b", 200, &["restore"]);
    assert!(msg.contains("diverged"), "{msg}");
    assert_eq!(env.git(&env.b, &["rev-parse", "HEAD"]), head);
}

#[test]
fn restore_from_subdirectory() {
    let env = Env::new();
    std::fs::create_dir_all(env.a.join("sub")).unwrap();
    std::fs::write(env.a.join("sub/keep.txt"), "keep\n").unwrap();
    env.git(&env.a, &["add", "sub"]);
    env.git(&env.a, &["commit", "-q", "-m", "sub"]);
    env.git(&env.a, &["push", "-q", "origin", "main"]);
    env.git(&env.b, &["pull", "-q"]);
    dirty_a_on_feature(&env);
    env.wip_ok(&env.b.join("sub"), "b", 200, &["restore"]);
    assert_eq!(env.read(&env.b, "file.txt"), "from a\n");
}

#[test]
fn no_fetch_uses_already_fetched_refs() {
    let env = Env::new();
    dirty_a_on_feature(&env);
    env.wip_ok(&env.b, "b", 200, &["restore", "--no-fetch"]);
    assert_eq!(env.read(&env.b, "file.txt"), "one\n", "nothing fetched yet");
    env.wip_ok(&env.b, "b", 200, &["save-all"]);
    env.wip_ok(&env.b, "b", 300, &["restore", "--no-fetch"]);
    assert_eq!(env.read(&env.b, "file.txt"), "from a\n");
}

#[test]
fn force_from_subdirectory_keeps_root_untracked_in_backup() {
    let env = Env::new();
    std::fs::create_dir_all(env.a.join("sub")).unwrap();
    std::fs::write(env.a.join("sub/keep.txt"), "keep\n").unwrap();
    env.git(&env.a, &["add", "sub"]);
    env.git(&env.a, &["commit", "-q", "-m", "sub"]);
    env.git(&env.a, &["push", "-q", "origin", "main"]);
    env.git(&env.b, &["pull", "-q"]);
    dirty_a_on_feature(&env);
    std::fs::write(env.b.join("notes.txt"), "precious\n").unwrap();
    env.wip_ok(&env.b.join("sub"), "b", 200, &["restore", "--force"]);
    let backup = env.git(&env.b, &["rev-parse", "refs/wip-backup/b"]);
    assert_eq!(env.git(&env.b, &["show", &format!("{backup}^3:notes.txt")]), "precious");
}

#[test]
fn backup_keeps_history() {
    let env = Env::new();
    dirty_a_on_feature(&env);
    std::fs::write(env.b.join("file.txt"), "first b\n").unwrap();
    env.wip_ok(&env.b, "b", 200, &["restore", "--force"]);
    std::fs::write(env.a.join("file.txt"), "a again\n").unwrap();
    env.wip_ok(&env.a, "a", 300, &["save"]);
    std::fs::write(env.b.join("file.txt"), "second b\n").unwrap();
    env.wip_ok(&env.b, "b", 400, &["restore", "--force"]);
    assert_eq!(env.git(&env.b, &["show", "refs/wip-backup/b@{1}:file.txt"]), "first b");
}

#[test]
fn discarding_wip_does_not_resurrect_older_foreign_wip() {
    let env = Env::new();
    std::fs::write(env.b.join("file.txt"), "old b wip\n").unwrap();
    env.wip_ok(&env.b, "b", 50, &["save"]);
    std::fs::write(env.a.join("file.txt"), "a wip\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    env.git(&env.a, &["checkout", "--", "file.txt"]);
    env.wip_ok(&env.a, "a", 200, &["save"]);
    env.wip_ok(&env.a, "a", 300, &["restore"]);
    assert_eq!(env.read(&env.a, "file.txt"), "one\n");
}

#[test]
fn local_commit_newer_than_foreign_wip_is_kept() {
    let env = Env::new();
    std::fs::write(env.b.join("file.txt"), "b wip\n").unwrap();
    env.wip_ok(&env.b, "b", 100, &["save"]);
    std::fs::write(env.a.join("file.txt"), "committed\n").unwrap();
    env.git_at(&env.a, 150, &["commit", "-q", "-am", "newer"]);
    env.wip_ok(&env.a, "a", 200, &["restore"]);
    assert_eq!(env.read(&env.a, "file.txt"), "committed\n");
}

#[test]
fn linked_worktree_is_skipped() {
    let env = Env::new();
    dirty_a_on_feature(&env);
    let wt = env.root.join("wt");
    env.git(&env.b, &["worktree", "add", "-q", "-b", "other", wt.to_str().unwrap()]);
    let out = env.wip(&wt, "b", 200, &["restore"]);
    assert!(
        out.status.success() && out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(env.read(&wt, "file.txt"), "one\n");
}

#[test]
fn fetched_snapshots_survive_a_pruning_fetch() {
    let env = Env::new();
    dirty_a_on_feature(&env);
    env.wip_ok(&env.b, "b", 150, &["save-all"]);
    env.git(&env.b, &["fetch", "-q", "--prune", "origin"]);
    assert!(env.git(&env.b, &["branch", "-r"]).lines().all(|l| !l.contains("wip/")));
    env.wip_ok(&env.b, "b", 200, &["restore", "--no-fetch"]);
    assert_eq!(env.read(&env.b, "file.txt"), "from a\n");
}

fn a_commits_and_saves_clean(env: &Env) {
    std::fs::write(env.a.join("file.txt"), "a commit\n").unwrap();
    env.git_at(&env.a, 150, &["commit", "-q", "-am", "a"]);
    env.wip_ok(&env.a, "a", 200, &["save"]);
}

#[test]
fn clean_foreign_commits_fast_forward_under_local_changes() {
    let env = Env::new();
    a_commits_and_saves_clean(&env);
    std::fs::write(env.b.join("notes.txt"), "b wip\n").unwrap();
    let msg = env.wip_ok(&env.b, "b", 300, &["restore"]);
    assert!(
        msg.contains("fast-forwarded main to a's commits, local changes kept"),
        "{msg}"
    );
    assert_eq!(
        env.git(&env.b, &["rev-parse", "HEAD"]),
        env.git(&env.a, &["rev-parse", "HEAD"])
    );
    assert_eq!(env.read(&env.b, "notes.txt"), "b wip\n");
    assert_eq!(env.read(&env.b, "file.txt"), "a commit\n");
}

#[test]
fn fast_forward_refused_when_local_changes_conflict() {
    let env = Env::new();
    a_commits_and_saves_clean(&env);
    std::fs::write(env.b.join("file.txt"), "b wip\n").unwrap();
    let head = env.git(&env.b, &["rev-parse", "HEAD"]);
    let msg = env.wip_ok(&env.b, "b", 300, &["restore"]);
    assert!(
        msg.contains("a has newer changes, run `git wip restore --force`"),
        "{msg}"
    );
    assert_eq!(env.git(&env.b, &["rev-parse", "HEAD"]), head);
    assert_eq!(env.read(&env.b, "file.txt"), "b wip\n");
}

#[test]
fn fast_forward_is_quiet_on_the_next_cd() {
    let env = Env::new();
    env.wip_ok(&env.a, "a", 50, &["save"]);
    a_commits_and_saves_clean(&env);
    std::fs::write(env.b.join("notes.txt"), "b wip\n").unwrap();
    env.wip_ok(&env.b, "b", 300, &["restore"]);
    let msg = env.wip_ok(&env.b, "b", 310, &["restore", "--no-fetch"]);
    assert!(msg.is_empty(), "{msg}");
}

#[test]
fn blocked_message_is_shown_once_per_snapshot() {
    let env = Env::new();
    dirty_a_on_feature(&env);
    std::fs::write(env.b.join("file.txt"), "local b\n").unwrap();
    let first = env.wip_ok(&env.b, "b", 200, &["restore"]);
    let second = env.wip_ok(&env.b, "b", 210, &["restore", "--no-fetch"]);
    assert!(first.contains("a has newer changes"), "{first}");
    assert!(second.is_empty(), "{second}");
}
