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
        .git(&env.a, &["for-each-ref", "refs/remotes/origin/wip/"])
        .contains("wip/a"));
}

#[test]
fn save_all_continues_after_a_failing_repo() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "a\n").unwrap();
    env.git(&env.b, &["remote", "set-url", "origin", "/nonexistent/remote.git"]);
    let out = env.wip(&env.root, "a", 100, &["save-all"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("/b:"));
    assert!(env.remote_ref("refs/wip/a").is_some());
}
