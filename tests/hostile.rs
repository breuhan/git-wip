mod common;
use common::Env;

/// Replaces b's snapshot on the remote with one carrying an attacker-chosen commit message.
fn forge(env: &Env, message: &str) {
    std::fs::write(env.b.join("b.txt"), "from b\n").unwrap();
    env.wip_ok(&env.b, "b", 500, &["save"]);
    let snap = env.git(&env.b, &["rev-parse", "refs/wip/b"]);
    let args = [
        "commit-tree",
        &format!("{snap}^{{tree}}"),
        "-p",
        &format!("{snap}^1"),
        "-p",
        &format!("{snap}^2"),
        "-p",
        &format!("{snap}^3"),
        "-m",
        message,
    ];
    let forged = env.git_at(&env.b, 500, &args);
    env.git(&env.b, &["push", "-q", "origin", &format!("+{forged}:refs/wip/b")]);
}

fn dirty_and_saved_a(env: &Env) {
    std::fs::write(env.a.join("file.txt"), "a wip\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
}

#[test]
fn seen_trailer_cannot_inject_git_options() {
    let env = Env::new();
    dirty_and_saved_a(&env);
    let target = env.root.join("pwned");
    forge(&env, &format!("WIP on main: x\n\nWip-Seen: a --output={}", target.display()));
    env.wip(&env.a, "a", 600, &["restore"]);
    assert!(!target.exists(), "diff-tree --output wrote outside the repo");
    assert_eq!(env.read(&env.a, "file.txt"), "a wip\n");
}

#[test]
fn forged_seen_trailer_with_a_garbage_oid_does_not_authorise() {
    let env = Env::new();
    dirty_and_saved_a(&env);
    forge(&env, "WIP on main: x\n\nWip-Seen: a HEAD");
    let msg = env.wip_ok(&env.a, "a", 600, &["restore"]);
    assert!(msg.contains("both have changes"), "{msg}");
    assert_eq!(env.read(&env.a, "file.txt"), "a wip\n");
}

#[test]
fn invalid_branch_name_is_refused() {
    let env = Env::new();
    forge(&env, "WIP on main~0: x");
    let out = env.wip(&env.a, "a", 600, &["restore"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("invalid branch name"));
    assert_eq!(env.git(&env.a, &["symbolic-ref", "--short", "HEAD"]), "main");
    assert!(!env.a.join("b.txt").exists());
}

#[test]
fn prompt_hook_ignores_repos_that_are_not_enabled() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "from a\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    env.unregister(&env.b);
    env.git(&env.b, &["fetch", "-q", "origin", "+refs/wip/*:refs/wip-remotes/origin/*"]);
    let msg = env.wip_ok(&env.b, "b", 200, &["restore", "--prompt"]);
    assert!(msg.is_empty(), "{msg}");
    assert_eq!(env.read(&env.b, "file.txt"), "one\n", "wip.remote alone must not enable the hook");
}

#[test]
fn prompt_hook_works_from_a_subdirectory_of_an_enabled_repo() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "from a\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    env.git(&env.b, &["fetch", "-q", "origin", "+refs/wip/*:refs/wip-remotes/origin/*"]);
    std::fs::create_dir_all(env.b.join("sub/dir")).unwrap();
    let msg = env.wip_ok(&env.b.join("sub/dir"), "b", 200, &["restore", "--prompt"]);
    assert!(msg.contains("restored state from a"), "{msg}");
}

#[test]
fn remote_name_cannot_inject_git_options() {
    let env = Env::new();
    let target = env.root.join("pwned");
    let value = format!("--upload-pack=touch {};", target.display());
    env.git(&env.a, &["config", "wip.remote", &value]);
    env.wip(&env.a, "a", 100, &["restore"]);
    env.wip(&env.a, "a", 100, &["save"]);
    assert!(!target.exists());
}

#[test]
fn fetch_does_not_import_tags() {
    let env = Env::new();
    env.git(&env.a, &["tag", "v9.9"]);
    env.git(&env.a, &["push", "-q", "origin", "v9.9"]);
    env.wip_ok(&env.a, "a", 100, &["save"]);
    env.wip_ok(&env.b, "b", 200, &["restore"]);
    assert_eq!(env.git(&env.b, &["tag"]), "");
}

#[test]
fn restore_does_not_overwrite_ignored_files() {
    let env = Env::new();
    std::fs::write(env.a.join("local.cfg"), "tracked on a\n").unwrap();
    env.git(&env.a, &["add", "local.cfg"]);
    env.git_at(&env.a, 50, &["commit", "-q", "-m", "cfg"]);
    env.wip_ok(&env.a, "a", 100, &["save"]);
    std::fs::write(env.b.join(".git/info/exclude"), "local.cfg\n").unwrap();
    std::fs::write(env.b.join("local.cfg"), "ignored on b\n").unwrap();
    env.wip(&env.b, "b", 200, &["restore"]);
    assert_eq!(env.read(&env.b, "local.cfg"), "ignored on b\n");
}
