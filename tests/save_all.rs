mod common;
use common::Env;

#[test]
fn save_all_saves_and_fetches_every_registered_repo() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "a\n").unwrap();
    std::fs::write(env.b.join("file.txt"), "b\n").unwrap();
    env.wip_ok(&env.root, "a", 100, &["save-all"]);
    assert!(env.remote_ref("refs/wip/a").is_some());
    assert!(env
        .git(&env.a, &["for-each-ref", "refs/wip-remotes/origin/"])
        .contains("origin/a"));
}

#[test]
fn snapshots_deleted_on_the_remote_disappear_locally() {
    let env = Env::new();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    env.wip_ok(&env.root, "b", 150, &["save-all"]);
    assert!(!env
        .git(&env.b, &["for-each-ref", "refs/wip-remotes/origin/a"])
        .is_empty());
    env.git(&env.a, &["push", "-q", "origin", ":refs/wip/a"]);
    env.wip_ok(&env.root, "b", 200, &["save-all"]);
    assert_eq!(env.git(&env.b, &["for-each-ref", "refs/wip-remotes/origin/a"]), "");
    assert!(!env
        .git(&env.b, &["for-each-ref", "refs/remotes/origin/main"])
        .is_empty());
}

#[test]
fn save_all_continues_after_a_failing_repo() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "a\n").unwrap();
    env.git(&env.b, &["remote", "set-url", "origin", "/nonexistent/remote.git"]);
    let out = env.wip(&env.root, "a", 100, &["save-all"]);
    assert!(!out.status.success(), "a failing repo is reported in the exit code");
    assert!(String::from_utf8_lossy(&out.stderr).contains("/b:"));
    assert!(env.remote_ref("refs/wip/a").is_some());
}
