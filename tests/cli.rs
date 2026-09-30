mod common;
use common::Env;

#[test]
fn enable_sets_remote_and_registers_repo() {
    let env = Env::new();
    assert_eq!(env.git(&env.a, &["config", "wip.remote"]), "origin");
    let a = env.git(&env.a, &["rev-parse", "--show-toplevel"]);
    assert_eq!(env.repos().iter().filter(|l| **l == a).count(), 1);
    env.wip_ok(&env.a, "a", 0, &["enable", "origin"]);
    assert_eq!(env.repos().iter().filter(|l| **l == a).count(), 1, "enable twice registers once");
}

#[test]
fn disable_removes_remote_and_registration() {
    let env = Env::new();
    env.wip_ok(&env.a, "a", 0, &["disable"]);
    assert!(env.wip(&env.a, "a", 0, &["save"]).status.success());
    let out = std::process::Command::new("git").current_dir(&env.a).args(["config", "wip.remote"]).output().unwrap();
    assert!(!out.status.success());
    let a = env.git(&env.a, &["rev-parse", "--show-toplevel"]);
    assert!(!env.repos().contains(&a));
}

#[test]
fn noop_outside_enabled_repo() {
    let env = Env::new();
    for args in [&["restore", "--no-fetch"][..], &["save"][..]] {
        let out = env.wip(&env.root, "a", 0, args);
        assert!(out.status.success());
        assert!(out.stderr.is_empty() && out.stdout.is_empty(), "{args:?} printed output");
    }
}

#[test]
fn usage_error_on_unknown_command() {
    let env = Env::new();
    let out = env.wip(&env.a, "a", 0, &["bogus"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage: git wip"));
}

#[test]
fn enable_works_with_read_only_global_git_config() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    let _ = std::fs::remove_file(env.root.join("home/.gitconfig"));
    let dir = env.root.join("home/.config/git");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config"), "[user]\n\tname = t\n").unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let disable = env.wip(&env.a, "a", 0, &["disable"]);
    let enable = env.wip(&env.a, "a", 0, &["enable", "origin"]);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(disable.status.success(), "{}", String::from_utf8_lossy(&disable.stderr));
    assert!(enable.status.success(), "{}", String::from_utf8_lossy(&enable.stderr));
}

#[test]
fn repo_list_in_the_old_git_config_format_is_still_read() {
    let env = Env::new();
    let a = env.git(&env.a, &["rev-parse", "--show-toplevel"]);
    std::fs::write(env.repos_file(), format!("[wip]\n\trepo = {a}\n")).unwrap();
    std::fs::write(env.a.join("file.txt"), "changed\n").unwrap();
    env.wip_ok(&env.root, "a", 100, &["save-all"]);
    assert!(env.remote_ref("refs/wip/a").is_some());
    env.wip_ok(&env.b, "b", 0, &["enable", "origin"]);
    env.wip_ok(&env.a, "a", 0, &["disable"]);
    assert_eq!(env.repos(), vec![env.git(&env.b, &["rev-parse", "--show-toplevel"])]);
}
