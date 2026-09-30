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
    assert_eq!(env.git(&env.b, &["status", "--porcelain"]), " M file.txt\nA  staged.txt\n?? new.txt");
}

#[test]
fn dirty_tree_refuses_and_force_restores_with_backup() {
    let env = Env::new();
    dirty_a_on_feature(&env);
    std::fs::write(env.b.join("file.txt"), "local b\n").unwrap();
    let msg = env.wip_ok(&env.b, "b", 200, &["restore"]);
    assert!(msg.contains("a has newer changes, run `git wip restore --force`"), "{msg}");
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
fn untouched_host_follows_several_saves() {
    let env = Env::new();
    dirty_a_on_feature(&env);
    env.wip_ok(&env.b, "b", 150, &["restore"]);
    std::fs::write(env.b.join("file.txt"), "from b\n").unwrap();
    env.wip_ok(&env.b, "b", 200, &["save"]);
    std::fs::write(env.b.join("file.txt"), "from b again\n").unwrap();
    env.wip_ok(&env.b, "b", 250, &["save"]);
    env.wip_ok(&env.a, "a", 300, &["restore"]);
    assert_eq!(env.read(&env.a, "file.txt"), "from b again\n");
}

#[test]
fn changes_on_both_hosts_are_not_overwritten() {
    let env = Env::new();
    std::fs::write(env.a.join("a.txt"), "only on a\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    std::fs::write(env.b.join("b.txt"), "only on b\n").unwrap();
    env.wip_ok(&env.b, "b", 110, &["save"]);
    let msg = env.wip_ok(&env.a, "a", 200, &["restore"]);
    assert!(msg.contains("b and a both have changes, run `git wip restore --merge`"), "{msg}");
    assert_eq!(env.read(&env.a, "a.txt"), "only on a\n");
    env.wip_ok(&env.a, "a", 210, &["restore", "--force"]);
    assert_eq!(env.read(&env.a, "b.txt"), "only on b\n");
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
    assert_eq!(env.git(&env.b, &["rev-parse", "main"]), env.git(&env.a, &["rev-parse", "main"]));
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
    assert_eq!(env.git(&env.b, &["rev-parse", "main"]), env.git(&env.a, &["rev-parse", "main"]));
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
    assert!(out.status.success() && out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
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
    assert!(msg.contains("fast-forwarded main to a's commits, local changes kept"), "{msg}");
    assert_eq!(env.git(&env.b, &["rev-parse", "HEAD"]), env.git(&env.a, &["rev-parse", "HEAD"]));
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
    assert!(msg.contains("a has newer changes, run `git wip restore --force`"), "{msg}");
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
    let second = env.wip_ok(&env.b, "b", 210, &["restore", "--prompt"]);
    let explicit = env.wip_ok(&env.b, "b", 220, &["restore", "--no-fetch"]);
    assert!(first.contains("a has newer changes"), "{first}");
    assert!(second.is_empty(), "the prompt hook reports once: {second}");
    assert!(explicit.contains("a has newer changes"), "a manual restore explains why: {explicit}");
}

#[test]
fn prompt_restores_once_the_refusal_is_resolved() {
    let env = Env::new();
    dirty_a_on_feature(&env);
    env.git(&env.b, &["fetch", "-q", "origin", "+refs/wip/*:refs/wip-remotes/origin/*"]);
    std::fs::write(env.b.join("file.txt"), "local b\n").unwrap();
    let refused = env.wip_ok(&env.b, "b", 200, &["restore", "--prompt"]);
    assert!(refused.contains("a has newer changes"), "{refused}");
    env.git(&env.b, &["checkout", "--", "file.txt"]);
    let msg = env.wip_ok(&env.b, "b", 210, &["restore", "--prompt"]);
    assert!(msg.contains("restored state from a"), "{msg}");
}

#[test]
fn refusal_recorded_by_an_older_version_is_rechecked() {
    let env = Env::new();
    dirty_a_on_feature(&env);
    env.wip_ok(&env.b, "b", 150, &["save-all"]);
    let snap = env.git(&env.b, &["rev-parse", "refs/wip-remotes/origin/a"]);
    std::fs::write(env.b.join(".git/wip-notified"), &snap).unwrap();
    let msg = env.wip_ok(&env.b, "b", 200, &["restore", "--prompt"]);
    assert!(msg.contains("restored state from a"), "{msg}");
}

#[test]
fn untracked_files_hidden_by_config_count_as_changes() {
    let env = Env::new();
    dirty_a_on_feature(&env);
    env.git(&env.b, &["config", "status.showUntrackedFiles", "no"]);
    std::fs::write(env.b.join("notes.txt"), "precious\n").unwrap();
    let msg = env.wip_ok(&env.b, "b", 200, &["restore"]);
    assert!(msg.contains("a has newer changes"), "{msg}");
    assert_eq!(env.read(&env.b, "notes.txt"), "precious\n");
}

#[test]
fn explicit_restore_waits_for_the_lock() {
    let env = Env::new();
    dirty_a_on_feature(&env);
    let lock = std::fs::File::create(env.b.join(".git/wip.lock")).unwrap();
    lock.lock().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(800));
        drop(lock);
    });
    let msg = env.wip_ok(&env.b, "b", 200, &["restore"]);
    release.join().unwrap();
    assert!(msg.contains("restored state from a"), "{msg}");
}

#[test]
fn prompt_restore_skips_while_locked() {
    let env = Env::new();
    dirty_a_on_feature(&env);
    env.wip_ok(&env.b, "b", 150, &["save-all"]);
    let lock = std::fs::File::create(env.b.join(".git/wip.lock")).unwrap();
    lock.lock().unwrap();
    let msg = env.wip_ok(&env.b, "b", 200, &["restore", "--prompt"]);
    assert!(msg.is_empty(), "{msg}");
    assert_eq!(env.read(&env.b, "file.txt"), "one\n");
    drop(lock);
    env.wip_ok(&env.b, "b", 210, &["restore", "--prompt"]);
    assert_eq!(env.read(&env.b, "file.txt"), "from a\n");
}

#[test]
fn failed_apply_puts_the_previous_state_back_and_stops_retrying() {
    let env = Env::new();
    std::fs::write(env.b.join(".git/info/exclude"), "new.txt\n").unwrap();
    std::fs::write(env.b.join("new.txt"), "ignored locally\n").unwrap();
    std::fs::write(env.b.join("file.txt"), "b work\n").unwrap();
    env.wip_ok(&env.b, "b", 50, &["save"]);
    dirty_a_on_feature(&env);
    let saved = env.remote_ref("refs/wip/b").unwrap();

    let out = env.wip(&env.b, "b", 200, &["restore", "--force"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(err.contains("could not apply a's state"), "{err}");
    assert_eq!(env.read(&env.b, "file.txt"), "b work\n", "previous state put back");
    assert_eq!(env.git(&env.b, &["symbolic-ref", "--short", "HEAD"]), "main");

    let prompt = env.wip_ok(&env.b, "b", 210, &["restore", "--prompt"]);
    assert!(prompt.is_empty(), "no retry at every prompt: {prompt}");
    env.wip_ok(&env.b, "b", 220, &["save"]);
    assert_eq!(env.remote_ref("refs/wip/b").unwrap(), saved, "b's work stays on the remote");
    let reflog = env.git(&env.b, &["reflog", "show", "refs/wip-backup/b"]);
    assert_eq!(reflog.lines().count(), 1, "{reflog}");
}

#[test]
fn fast_forward_after_a_prior_save() {
    let env = Env::new();
    std::fs::write(env.b.join("notes.txt"), "b wip\n").unwrap();
    env.wip_ok(&env.b, "b", 50, &["save"]);
    env.wip_ok(&env.a, "a", 60, &["save"]);
    a_commits_and_saves_clean(&env);
    let msg = env.wip_ok(&env.b, "b", 300, &["restore"]);
    assert!(msg.contains("fast-forwarded main to a's commits"), "{msg}");
    assert_eq!(env.read(&env.b, "notes.txt"), "b wip\n");
}

#[test]
fn handoff_back_does_not_depend_on_clocks() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "from a\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    env.wip_ok(&env.b, "b", 110, &["restore"]);
    std::fs::write(env.b.join("file.txt"), "from b, clock behind\n").unwrap();
    env.wip_ok(&env.b, "b", 50, &["save"]);
    env.wip_ok(&env.a, "a", 120, &["restore"]);
    assert_eq!(env.read(&env.a, "file.txt"), "from b, clock behind\n");
}

fn third_clone(env: &Env) -> std::path::PathBuf {
    env.git(&env.root, &["clone", "-q", "remote.git", "c"]);
    let c = env.root.join("c");
    env.wip_ok(&c, "c", 0, &["enable", "origin"]);
    c
}

#[test]
fn handoff_along_three_hosts() {
    let env = Env::new();
    let c = third_clone(&env);
    std::fs::write(env.a.join("file.txt"), "a\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    env.wip_ok(&env.b, "b", 110, &["restore"]);
    std::fs::write(env.b.join("file.txt"), "a b\n").unwrap();
    env.wip_ok(&env.b, "b", 120, &["save"]);
    env.wip_ok(&c, "c", 130, &["restore"]);
    std::fs::write(c.join("file.txt"), "a b c\n").unwrap();
    env.wip_ok(&c, "c", 140, &["save"]);
    let msg = env.wip_ok(&env.a, "a", 150, &["restore"]);
    assert!(msg.contains("restored state from c"), "{msg}");
    assert_eq!(env.read(&env.a, "file.txt"), "a b c\n");
}

#[test]
fn third_host_with_unseen_changes_is_refused() {
    let env = Env::new();
    let c = third_clone(&env);
    std::fs::write(c.join("c.txt"), "only on c\n").unwrap();
    env.wip_ok(&c, "c", 95, &["save"]);
    std::fs::write(env.a.join("file.txt"), "a\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    env.wip_ok(&env.b, "b", 110, &["restore"]);
    std::fs::write(env.b.join("file.txt"), "a b\n").unwrap();
    env.wip_ok(&env.b, "b", 120, &["save"]);
    let msg = env.wip_ok(&c, "c", 130, &["restore"]);
    assert!(msg.contains("b and c both have changes"), "{msg}");
    assert_eq!(env.read(&c, "c.txt"), "only on c\n");
}

#[test]
fn normal_handoff_after_force() {
    let env = Env::new();
    std::fs::write(env.a.join("a.txt"), "a\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    std::fs::write(env.b.join("b.txt"), "b\n").unwrap();
    env.wip_ok(&env.b, "b", 110, &["save"]);
    env.wip_ok(&env.a, "a", 120, &["restore", "--force"]);
    std::fs::write(env.b.join("b.txt"), "b again\n").unwrap();
    env.wip_ok(&env.b, "b", 130, &["save"]);
    let msg = env.wip_ok(&env.a, "a", 140, &["restore"]);
    assert!(msg.contains("restored state from b"), "{msg}");
    assert_eq!(env.read(&env.a, "b.txt"), "b again\n");
}

#[test]
fn a_commit_under_unchanged_wip_does_not_count_as_new_changes() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "a wip\n").unwrap();
    std::fs::write(env.a.join("a.txt"), "a untracked\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    env.wip_ok(&env.b, "b", 110, &["restore"]);

    // a commits something else; its WIP is the same but its snapshot is re-made on the new HEAD.
    std::fs::write(env.a.join("other.txt"), "committed\n").unwrap();
    env.git(&env.a, &["add", "other.txt"]);
    env.git_at(&env.a, 120, &["commit", "-q", "-m", "other", "other.txt"]);
    env.git(&env.a, &["push", "-q", "origin", "main"]);
    env.wip_ok(&env.a, "a", 130, &["save"]);

    env.git(&env.b, &["pull", "-q"]);
    std::fs::write(env.b.join("b.txt"), "from b\n").unwrap();
    env.wip_ok(&env.b, "b", 200, &["save"]);

    let msg = env.wip_ok(&env.a, "a", 300, &["restore"]);
    assert!(msg.contains("restored state from b"), "{msg}");
    assert_eq!(env.read(&env.a, "b.txt"), "from b\n");
    assert_eq!(env.read(&env.a, "file.txt"), "a wip\n");
}

#[test]
fn new_edits_after_being_seen_still_count() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "a wip\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    env.wip_ok(&env.b, "b", 110, &["restore"]);
    std::fs::write(env.a.join("file.txt"), "a wip, continued\n").unwrap();
    env.wip_ok(&env.a, "a", 130, &["save"]);
    std::fs::write(env.b.join("b.txt"), "from b\n").unwrap();
    env.wip_ok(&env.b, "b", 200, &["save"]);
    let msg = env.wip_ok(&env.a, "a", 300, &["restore"]);
    assert!(msg.contains("both have changes"), "{msg}");
    assert_eq!(env.read(&env.a, "file.txt"), "a wip, continued\n");
}

#[test]
fn a_restored_state_re_saved_on_a_new_head_is_not_own_work() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "a wip\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    env.wip_ok(&env.b, "b", 110, &["restore"]);

    // Both get a new commit; b's watcher re-saves the state it took from a, now on the new HEAD.
    std::fs::write(env.a.join("other.txt"), "committed\n").unwrap();
    env.git(&env.a, &["add", "other.txt"]);
    env.git_at(&env.a, 120, &["commit", "-q", "-m", "other", "other.txt"]);
    env.git(&env.a, &["push", "-q", "origin", "main"]);
    std::fs::write(env.a.join("a2.txt"), "more from a\n").unwrap();
    env.wip_ok(&env.a, "a", 150, &["save"]);
    env.git(&env.b, &["pull", "-q"]);
    env.wip_ok(&env.b, "b", 200, &["save"]);

    // b's re-save is newer by the clock but holds nothing a has not got.
    let msg = env.wip_ok(&env.a, "a", 300, &["restore"]);
    assert!(msg.is_empty(), "{msg}");
    assert_eq!(env.read(&env.a, "a2.txt"), "more from a\n");

    // b only carries a's older state, so it follows a's newer one.
    let msg = env.wip_ok(&env.b, "b", 300, &["restore"]);
    assert!(msg.contains("restored state from a"), "{msg}");
    assert_eq!(env.read(&env.b, "a2.txt"), "more from a\n");
}
