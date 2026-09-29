mod common;
use common::Env;

#[test]
fn enable_sets_remote_and_registers_repo() {
    let env = Env::new();
    assert_eq!(env.git(&env.a, &["config", "wip.remote"]), "origin");
    let repos = env.git(&env.a, &["config", "--global", "--get-all", "wip.repo"]);
    let a = env.git(&env.a, &["rev-parse", "--show-toplevel"]);
    assert_eq!(repos.lines().filter(|l| *l == a).count(), 1);
    env.wip_ok(&env.a, "a", 0, &["enable", "origin"]);
    let repos = env.git(&env.a, &["config", "--global", "--get-all", "wip.repo"]);
    assert_eq!(
        repos.lines().filter(|l| *l == a).count(),
        1,
        "enable twice registers once"
    );
}

#[test]
fn disable_removes_remote_and_registration() {
    let env = Env::new();
    env.wip_ok(&env.a, "a", 0, &["disable"]);
    assert!(env.wip(&env.a, "a", 0, &["save"]).status.success());
    let out = std::process::Command::new("git")
        .current_dir(&env.a)
        .args(["config", "wip.remote"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let repos = env.git(&env.a, &["config", "--global", "--get-all", "wip.repo"]);
    let a = env.git(&env.a, &["rev-parse", "--show-toplevel"]);
    assert!(!repos.lines().any(|l| l == a));
}

#[test]
fn noop_outside_enabled_repo() {
    let env = Env::new();
    for args in [&["restore", "--no-fetch"][..], &["save"][..]] {
        let out = env.wip(&env.root, "a", 0, args);
        assert!(out.status.success());
        assert!(
            out.stderr.is_empty() && out.stdout.is_empty(),
            "{args:?} printed output"
        );
    }
}

#[test]
fn usage_error_on_unknown_command() {
    let env = Env::new();
    let out = env.wip(&env.a, "a", 0, &["bogus"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage: git wip"));
}
