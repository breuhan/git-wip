mod common;
use common::Env;

/// a hands its work over to b, so both start from a state the other has seen.
fn shared_start(env: &Env) {
    std::fs::write(env.a.join("file.txt"), "one\nshared wip\n").unwrap();
    std::fs::write(env.a.join("a.txt"), "from a\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    env.wip_ok(&env.b, "b", 110, &["restore"]);
}

fn merge_into_a(env: &Env) -> std::process::Output {
    env.wip(&env.a, "a", 300, &["restore", "--merge"])
}

#[test]
fn combines_changes_from_both_hosts() {
    let env = Env::new();
    shared_start(&env);
    std::fs::write(env.a.join("a2.txt"), "more from a\n").unwrap();
    env.wip_ok(&env.a, "a", 130, &["save"]);
    std::fs::write(env.b.join("b2.txt"), "from b\n").unwrap();
    env.wip_ok(&env.b, "b", 200, &["save"]);

    let refused = env.wip_ok(&env.a, "a", 250, &["restore"]);
    assert!(refused.contains("git wip restore --merge"), "{refused}");

    let out = merge_into_a(&env);
    let msg = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{msg}");
    assert!(msg.contains("merged state from b"), "{msg}");
    assert_eq!(env.read(&env.a, "a2.txt"), "more from a\n");
    assert_eq!(env.read(&env.a, "b2.txt"), "from b\n");
    assert_eq!(env.read(&env.a, "file.txt"), "one\nshared wip\n");
    assert_eq!(
        env.git(&env.a, &["status", "--porcelain", "--untracked-files=all"]),
        " M file.txt\n?? a.txt\n?? a2.txt\n?? b2.txt"
    );

    // The merged state has seen b's, so b follows it without a conflict.
    env.wip_ok(&env.a, "a", 310, &["save"]);
    let msg = env.wip_ok(&env.b, "b", 400, &["restore"]);
    assert!(msg.contains("restored state from a"), "{msg}");
    assert_eq!(env.read(&env.b, "a2.txt"), "more from a\n");
}

#[test]
fn applies_deletions_from_the_other_host() {
    let env = Env::new();
    shared_start(&env);
    std::fs::write(env.a.join("a2.txt"), "more from a\n").unwrap();
    env.wip_ok(&env.a, "a", 130, &["save"]);
    std::fs::remove_file(env.b.join("a.txt")).unwrap();
    env.wip_ok(&env.b, "b", 200, &["save"]);

    assert!(merge_into_a(&env).status.success());
    assert!(!env.a.join("a.txt").exists(), "b deleted a.txt after seeing it");
    assert_eq!(env.read(&env.a, "a2.txt"), "more from a\n");
}

#[test]
fn conflicting_edits_leave_markers_and_a_backup() {
    let env = Env::new();
    shared_start(&env);
    std::fs::write(env.a.join("file.txt"), "one\nedited on a\n").unwrap();
    env.wip_ok(&env.a, "a", 130, &["save"]);
    std::fs::write(env.b.join("file.txt"), "one\nedited on b\n").unwrap();
    env.wip_ok(&env.b, "b", 200, &["save"]);

    let out = merge_into_a(&env);
    let msg = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "a conflict is reported in the exit code");
    assert!(msg.contains("conflicts in: file.txt"), "{msg}");
    let merged = env.read(&env.a, "file.txt");
    assert!(merged.contains("<<<<<<<") && merged.contains("edited on a") && merged.contains("edited on b"), "{merged}");
    assert_eq!(env.git(&env.a, &["show", "refs/wip-backup/a:file.txt"]), "one\nedited on a");
}

#[test]
fn takes_the_other_hosts_commits_along() {
    let env = Env::new();
    shared_start(&env);
    std::fs::write(env.a.join("a2.txt"), "more from a\n").unwrap();
    env.wip_ok(&env.a, "a", 130, &["save"]);
    std::fs::write(env.b.join("other.txt"), "committed on b\n").unwrap();
    env.git(&env.b, &["add", "other.txt"]);
    env.git_at(&env.b, 150, &["commit", "-q", "-m", "other", "other.txt"]);
    env.wip_ok(&env.b, "b", 200, &["save"]);

    assert!(merge_into_a(&env).status.success());
    assert_eq!(env.git(&env.a, &["rev-parse", "HEAD"]), env.git(&env.b, &["rev-parse", "HEAD"]));
    assert_eq!(env.read(&env.a, "a2.txt"), "more from a\n");
    assert_eq!(
        env.git(&env.a, &["status", "--porcelain", "--untracked-files=all"]),
        " M file.txt\n?? a.txt\n?? a2.txt"
    );
}

#[test]
fn refuses_across_branches() {
    let env = Env::new();
    shared_start(&env);
    env.git(&env.b, &["checkout", "-q", "-b", "topic"]);
    std::fs::write(env.b.join("b2.txt"), "from b\n").unwrap();
    env.wip_ok(&env.b, "b", 200, &["save"]);
    std::fs::write(env.a.join("a2.txt"), "more from a\n").unwrap();

    let out = merge_into_a(&env);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("b is on topic"));
    assert!(!env.a.join("b2.txt").exists());
}

#[test]
fn nothing_to_merge_without_a_foreign_snapshot() {
    let env = Env::new();
    let out = merge_into_a(&env);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("nothing to merge"));
}

#[test]
fn a_colliding_ignored_file_stops_the_merge_without_changes() {
    let env = Env::new();
    shared_start(&env);
    std::fs::write(env.a.join("a2.txt"), "more from a\n").unwrap();
    env.wip_ok(&env.a, "a", 130, &["save"]);
    std::fs::write(env.b.join("b2.txt"), "from b\n").unwrap();
    env.wip_ok(&env.b, "b", 200, &["save"]);
    std::fs::write(env.a.join(".git/info/exclude"), "b2.txt\n").unwrap();
    std::fs::write(env.a.join("b2.txt"), "ignored on a\n").unwrap();
    let before = env.git(&env.a, &["status", "--porcelain", "--untracked-files=all"]);

    let out = merge_into_a(&env);
    let msg = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(msg.contains("nothing was changed"), "{msg}");
    assert_eq!(env.read(&env.a, "b2.txt"), "ignored on a\n");
    assert_eq!(env.git(&env.a, &["status", "--porcelain", "--untracked-files=all"]), before);
}
