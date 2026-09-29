mod common;
use common::{eventually, Env};

#[test]
fn saves_after_a_file_change() {
    let env = Env::new();
    env.unregister(&env.b);
    let watch = env.watch("a", 100);
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let before = env.remote_ref("refs/wip/a");
    std::fs::write(env.a.join("file.txt"), "edited\n").unwrap();
    eventually("snapshot with the edit", || {
        env.remote_ref("refs/wip/a")
            .filter(|oid| Some(oid) != before.as_ref())
            .is_some_and(|oid| env.git(&env.a, &["show", &format!("{oid}:file.txt")]) == "edited")
    });
    let log = watch.log();
    assert!(!log.contains("error"), "{log}");
}

#[test]
fn restores_a_foreign_snapshot_in_the_background() {
    let env = Env::new();
    env.unregister(&env.a);
    std::fs::write(env.a.join("file.txt"), "from a\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    let watch = env.watch("b", 200);
    eventually("background restore", || watch.log().contains("restored state from a"));
    assert_eq!(env.read(&env.b, "file.txt"), "from a\n");
}

#[test]
fn blocked_restore_is_reported_once() {
    let env = Env::new();
    env.unregister(&env.a);
    std::fs::write(env.a.join("file.txt"), "from a\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    std::fs::write(env.b.join("file.txt"), "local b\n").unwrap();
    let watch = env.watch("b", 200);
    std::thread::sleep(std::time::Duration::from_secs(4));
    let log = watch.log();
    assert_eq!(log.matches("a has newer changes").count(), 1, "{log}");
    assert_eq!(env.read(&env.b, "file.txt"), "local b\n");
}

#[test]
fn picks_up_newly_enabled_repos() {
    let env = Env::new();
    env.unregister(&env.a);
    env.unregister(&env.b);
    let _watch = env.watch("a", 100);
    std::thread::sleep(std::time::Duration::from_millis(500));
    env.wip_ok(&env.a, "a", 100, &["enable", "origin"]);
    std::thread::sleep(std::time::Duration::from_millis(1500));
    std::fs::write(env.a.join("file.txt"), "edited\n").unwrap();
    eventually("snapshot after enable", || {
        env.remote_ref("refs/wip/a")
            .is_some_and(|oid| env.git(&env.a, &["show", &format!("{oid}:file.txt")]) == "edited")
    });
}
