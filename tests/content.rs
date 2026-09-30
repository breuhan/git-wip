//! Round trips of working-tree content that is easy to lose or mangle: a saves, b restores.
mod common;
use common::Env;
use std::os::unix::fs::PermissionsExt;

fn hand_over(env: &Env) {
    env.wip_ok(&env.a, "a", 100, &["save"]);
    let msg = env.wip_ok(&env.b, "b", 200, &["restore"]);
    assert!(msg.contains("restored state from a"), "{msg}");
}

#[test]
fn staged_and_unstaged_versions_of_one_file() {
    let env = Env::new();
    std::fs::write(env.a.join("file.txt"), "staged\n").unwrap();
    env.git(&env.a, &["add", "file.txt"]);
    std::fs::write(env.a.join("file.txt"), "unstaged\n").unwrap();
    hand_over(&env);
    assert_eq!(env.git(&env.b, &["show", ":file.txt"]), "staged");
    assert_eq!(env.read(&env.b, "file.txt"), "unstaged\n");
}

#[test]
fn deletions_and_renames() {
    let env = Env::new();
    std::fs::write(env.a.join("other.txt"), "other\n").unwrap();
    env.git(&env.a, &["add", "other.txt"]);
    env.git(&env.a, &["commit", "-q", "-m", "other"]);
    env.git(&env.a, &["push", "-q", "origin", "main"]);
    env.git(&env.b, &["pull", "-q"]);
    env.git(&env.a, &["mv", "file.txt", "renamed.txt"]);
    std::fs::remove_file(env.a.join("other.txt")).unwrap();
    hand_over(&env);
    assert_eq!(env.git(&env.b, &["status", "--porcelain"]), " D other.txt\nR  file.txt -> renamed.txt");
}

#[test]
fn unusual_file_names() {
    let env = Env::new();
    for name in ["with space.txt", "ünïcödé.txt", "-dash.txt", "tab\there.txt"] {
        std::fs::write(env.a.join(name), format!("{name}\n")).unwrap();
    }
    hand_over(&env);
    for name in ["with space.txt", "ünïcödé.txt", "-dash.txt", "tab\there.txt"] {
        assert_eq!(env.read(&env.b, name), format!("{name}\n"));
    }
}

#[test]
fn binary_content_and_executable_bit() {
    let env = Env::new();
    let bytes: Vec<u8> = (0..=255).collect();
    std::fs::write(env.a.join("blob.bin"), &bytes).unwrap();
    std::fs::write(env.a.join("run.sh"), "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(env.a.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    hand_over(&env);
    assert_eq!(std::fs::read(env.b.join("blob.bin")).unwrap(), bytes);
    let mode = std::fs::metadata(env.b.join("run.sh")).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111, "executable bit lost: {mode:o}");
}

#[test]
fn symlinks_stay_links() {
    let env = Env::new();
    std::os::unix::fs::symlink("file.txt", env.a.join("link")).unwrap();
    hand_over(&env);
    assert_eq!(std::fs::read_link(env.b.join("link")).unwrap().to_str(), Some("file.txt"));
}

#[test]
fn branch_that_exists_at_an_older_commit_is_fast_forwarded() {
    let env = Env::new();
    env.git(&env.a, &["checkout", "-q", "-b", "topic"]);
    env.git(&env.a, &["push", "-q", "origin", "topic"]);
    env.git(&env.b, &["fetch", "-q"]);
    env.git(&env.b, &["branch", "-q", "topic", "origin/topic"]);
    std::fs::write(env.a.join("file.txt"), "topic work\n").unwrap();
    env.git(&env.a, &["commit", "-q", "-am", "topic commit"]);
    std::fs::write(env.a.join("file.txt"), "topic wip\n").unwrap();
    hand_over(&env);
    assert_eq!(env.git(&env.b, &["symbolic-ref", "--short", "HEAD"]), "topic");
    assert_eq!(env.git(&env.b, &["rev-parse", "topic"]), env.git(&env.a, &["rev-parse", "topic"]));
    assert_eq!(env.read(&env.b, "file.txt"), "topic wip\n");
}
