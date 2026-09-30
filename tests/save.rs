mod common;
use common::Env;

#[test]
fn save_pushes_stash_commit_with_untracked_and_staged() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "changed\n").unwrap();
    std::fs::write(env.a.join("staged.txt"), "staged\n").unwrap();
    env.git(&env.a, &["add", "staged.txt"]);
    std::fs::write(env.a.join("new.txt"), "untracked\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);

    let oid = env.remote_ref("refs/wip/a").expect("refs/wip/a pushed");
    let msg = env.git(&env.a, &["log", "-1", "--format=%s", &oid]);
    assert!(msg.starts_with("WIP on main: "), "{msg}");
    assert_eq!(env.git(&env.a, &["show", &format!("{oid}:file.txt")]), "changed");
    assert_eq!(env.git(&env.a, &["show", &format!("{oid}^2:staged.txt")]), "staged");
    assert_eq!(env.git(&env.a, &["show", &format!("{oid}^3:new.txt")]), "untracked");
    assert_eq!(env.git(&env.a, &["status", "--porcelain"]), " M file.txt\nA  staged.txt\n?? new.txt");
}

#[test]
fn save_leaves_worktree_index_and_stash_list_untouched() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "changed\n").unwrap();
    std::fs::write(env.a.join("new.txt"), "untracked\n").unwrap();
    let before = env.git(&env.a, &["status", "--porcelain"]);
    env.wip_ok(&env.a, "a", 100, &["save"]);
    assert_eq!(env.git(&env.a, &["status", "--porcelain"]), before);
    assert_eq!(env.git(&env.a, &["stash", "list"]), "");
}

#[test]
fn clean_tree_is_saved() {
    let env = Env::new();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    let oid = env.remote_ref("refs/wip/a").expect("clean state pushed");
    let head = env.git(&env.a, &["rev-parse", "HEAD"]);
    assert_eq!(env.git(&env.a, &["rev-parse", &format!("{oid}^1")]), head);
}

#[test]
fn clean_snapshot_takes_head_commit_time() {
    let env = Env::new();
    env.wip_ok(&env.a, "a", 500, &["save"]);
    let oid = env.remote_ref("refs/wip/a").unwrap();
    let head_time = env.git(&env.a, &["log", "-1", "--format=%ct", "HEAD"]);
    assert_eq!(env.git(&env.a, &["log", "-1", "--format=%ct", &oid]), head_time);
}

#[test]
fn no_push_when_unchanged() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "changed\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    let first = env.remote_ref("refs/wip/a").unwrap();
    env.wip_ok(&env.a, "a", 200, &["save"]);
    assert_eq!(env.remote_ref("refs/wip/a").unwrap(), first);
    std::fs::write(env.a.join("file.txt"), "again\n").unwrap();
    env.wip_ok(&env.a, "a", 300, &["save"]);
    assert_ne!(env.remote_ref("refs/wip/a").unwrap(), first);
}

#[test]
fn ignored_files_not_pushed() {
    let env = Env::new();
    std::fs::write(env.a.join(".gitignore"), ".env\n").unwrap();
    std::fs::write(env.a.join(".env"), "SECRET=1\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    let oid = env.remote_ref("refs/wip/a").unwrap();
    let files = env.git(&env.a, &["ls-tree", "-r", "--name-only", &format!("{oid}^3")]);
    assert_eq!(files, ".gitignore");
}

#[test]
fn busy_repo_is_skipped() {
    let env = Env::new();
    std::fs::create_dir_all(env.a.join(".git/rebase-merge")).unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    assert_eq!(env.remote_ref("refs/wip/a"), None);
}

#[test]
fn detached_head_is_skipped() {
    let env = Env::new();
    env.git(&env.a, &["checkout", "-q", "--detach"]);
    env.wip_ok(&env.a, "a", 100, &["save"]);
    assert_eq!(env.remote_ref("refs/wip/a"), None);
}

#[test]
fn offline_save_retries() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "changed\n").unwrap();
    env.git(&env.a, &["remote", "set-url", "origin", "/nonexistent/remote.git"]);
    assert!(!env.wip(&env.a, "a", 100, &["save"]).status.success());
    assert!(!env.git(&env.a, &["for-each-ref", "refs/wip/"]).contains("refs/wip/a"));
    let remote = env.remote.display().to_string();
    env.git(&env.a, &["remote", "set-url", "origin", &remote]);
    env.wip_ok(&env.a, "a", 200, &["save"]);
    assert!(env.remote_ref("refs/wip/a").is_some());
}

#[test]
fn explicit_save_waits_for_the_lock() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "changed\n").unwrap();
    let lock = std::fs::File::create(env.a.join(".git/wip.lock")).unwrap();
    lock.try_lock().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(800));
        drop(lock);
    });
    env.wip_ok(&env.a, "a", 100, &["save"]);
    release.join().unwrap();
    assert!(env.remote_ref("refs/wip/a").is_some());
}

#[test]
fn save_ignores_pre_push_hooks() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    let hook = env.a.join(".git/hooks/pre-push");
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(env.a.join("file.txt"), "changed\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    assert!(env.remote_ref("refs/wip/a").is_some());
}

#[test]
fn host_name_does_not_depend_on_path() {
    let name = gethostname::gethostname().to_string_lossy().to_lowercase();
    let expected = name.split('.').next().unwrap().to_string();
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "changed\n").unwrap();
    let git = std::process::Command::new("sh").args(["-c", "command -v git"]).output().unwrap();
    let git_bin = std::path::Path::new(String::from_utf8_lossy(&git.stdout).trim()).parent().unwrap().to_path_buf();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_git-wip"))
        .current_dir(&env.a)
        .arg("save")
        .envs(env.envs(100))
        .env("PATH", git_bin)
        .env_remove("GIT_WIP_HOST")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(env.remote_ref(&format!("refs/wip/{expected}")).is_some(), "expected refs/wip/{expected}");
    assert_eq!(env.remote_ref("refs/wip/unknown"), None);
}

#[test]
fn untracked_files_that_are_gone_or_tracked_leave_the_snapshot() {
    let env = Env::new();
    for name in ["u1.txt", "u2.txt", "u3.txt"] {
        std::fs::write(env.a.join(name), "untracked\n").unwrap();
    }
    env.wip_ok(&env.a, "a", 100, &["save"]);
    std::fs::remove_file(env.a.join("u1.txt")).unwrap();
    env.git(&env.a, &["add", "u2.txt"]);
    env.wip_ok(&env.a, "a", 200, &["save"]);
    let oid = env.remote_ref("refs/wip/a").unwrap();
    assert_eq!(env.git(&env.a, &["ls-tree", "-r", "--name-only", &format!("{oid}^3")]), "u3.txt");
}
