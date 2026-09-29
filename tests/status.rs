mod common;
use common::Env;

fn status(env: &Env, dir: &std::path::Path, host: &str) -> String {
    let out = env.wip(dir, host, 1000, &["status"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn a_saves_on_feature(env: &Env) {
    env.git(&env.a, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(env.a.join("file.txt"), "from a\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    env.wip_ok(&env.b, "b", 150, &["save-all"]);
}

#[test]
fn not_enabled() {
    let env = Env::new();
    env.wip_ok(&env.a, "a", 0, &["disable"]);
    assert_eq!(
        status(&env, &env.a, "a"),
        "not enabled, run `git wip enable <remote>`\n"
    );
}

#[test]
fn lists_hosts_and_pending_restore() {
    let env = Env::new();
    a_saves_on_feature(&env);
    let out = status(&env, &env.b, "b");
    assert!(out.starts_with("remote: origin\n"), "{out}");
    assert!(
        out.lines().any(|l| l.starts_with("a ") && l.contains("feature")),
        "{out}"
    );
    assert!(
        out.lines()
            .any(|l| l.starts_with("b ") && l.contains("main") && l.contains("(this host)")),
        "{out}"
    );
    assert!(out.ends_with("restore pending from a\n"), "{out}");
}

#[test]
fn blocked_by_local_changes() {
    let env = Env::new();
    a_saves_on_feature(&env);
    std::fs::write(env.b.join("file.txt"), "local b\n").unwrap();
    let out = status(&env, &env.b, "b");
    assert!(
        out.ends_with("a has newer changes, run `git wip restore --force`\n"),
        "{out}"
    );
}

#[test]
fn up_to_date_after_restore() {
    let env = Env::new();
    a_saves_on_feature(&env);
    env.wip_ok(&env.b, "b", 200, &["restore", "--no-fetch"]);
    assert!(status(&env, &env.b, "b").ends_with("up to date\n"));
}
