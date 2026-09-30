mod common;
use common::{Env, Watch, eventually};

fn snapshot_file(env: &Env, file: &str) -> Option<String> {
    let oid = env.remote_ref("refs/wip/a")?;
    Some(env.git(&env.a, &["show", &format!("{oid}:{file}")]))
}

fn watching(env: &Env, host: &str, date: Option<i64>) -> Watch {
    let watch = env.watch(host, date);
    eventually("watcher to start", || watch.log().contains("watching"));
    watch
}

#[test]
fn saves_after_a_file_change() {
    let env = Env::new();
    env.unregister(&env.b);
    let watch = watching(&env, "a", Some(100));
    std::fs::write(env.a.join("file.txt"), "edited\n").unwrap();
    eventually("snapshot with the edit", || snapshot_file(&env, "file.txt").as_deref() == Some("edited"));
    assert!(!watch.log().contains("error"), "{}", watch.log());
}

#[test]
fn idle_rounds_write_no_objects() {
    let env = Env::new();
    env.unregister(&env.b);
    std::fs::write(env.a.join("file.txt"), "unsaved\n").unwrap();
    // Real clock: with fixed test dates repeated snapshots are identical objects and nothing grows.
    let watch = watching(&env, "a", None);
    eventually("startup save", || snapshot_file(&env, "file.txt").as_deref() == Some("unsaved"));
    let loose = || env.git(&env.a, &["count-objects"]);
    let before = loose();
    std::thread::sleep(std::time::Duration::from_millis(3500));
    assert_eq!(loose(), before, "fetch rounds must not snapshot unchanged repos, log:\n{}", watch.log());
}

#[test]
fn fetches_but_leaves_restoring_to_the_prompt() {
    let env = Env::new();
    env.unregister(&env.a);
    std::fs::write(env.a.join("file.txt"), "from a\n").unwrap();
    env.wip_ok(&env.a, "a", 100, &["save"]);
    let _watch = watching(&env, "b", Some(200));
    eventually("fetch", || !env.git(&env.b, &["for-each-ref", "refs/wip-remotes/origin/a"]).is_empty());
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(env.read(&env.b, "file.txt"), "one\n");
    let msg = env.wip_ok(&env.b, "b", 300, &["restore", "--prompt"]);
    assert!(msg.contains("restored state from a"), "{msg}");
}

#[test]
fn ignored_writes_do_not_delay_saves() {
    let env = Env::new();
    env.unregister(&env.b);
    std::fs::write(env.a.join(".gitignore"), "busy.log\n").unwrap();
    let _watch = watching(&env, "a", Some(100));
    let log = env.a.join("busy.log");
    let writer = std::thread::spawn(move || {
        for i in 0..40 {
            std::fs::write(&log, format!("{i}\n")).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
    std::fs::write(env.a.join("file.txt"), "edited\n").unwrap();
    let start = std::time::Instant::now();
    eventually("snapshot while ignored file keeps changing", || {
        snapshot_file(&env, "file.txt").as_deref() == Some("edited")
    });
    assert!(start.elapsed() < std::time::Duration::from_secs(3), "{:?}", start.elapsed());
    writer.join().unwrap();
}

#[test]
fn retries_a_failed_push_without_new_edits() {
    let env = Env::new();
    env.unregister(&env.b);
    env.git(&env.a, &["remote", "set-url", "origin", "/nonexistent/remote.git"]);
    std::fs::write(env.a.join("file.txt"), "offline edit\n").unwrap();
    let watch = watching(&env, "a", Some(100));
    eventually("failed push", || watch.log().contains("nonexistent"));
    let remote = env.remote.display().to_string();
    env.git(&env.a, &["remote", "set-url", "origin", &remote]);
    eventually("retried push", || snapshot_file(&env, "file.txt").as_deref() == Some("offline edit"));
}

#[test]
fn picks_up_newly_enabled_repos() {
    let env = Env::new();
    env.unregister(&env.a);
    env.unregister(&env.b);
    let watch = env.watch("a", Some(100));
    std::thread::sleep(std::time::Duration::from_millis(300));
    env.wip_ok(&env.a, "a", 100, &["enable", "origin"]);
    eventually("watching the new repo", || watch.log().contains("watching"));
    std::fs::write(env.a.join("file.txt"), "edited\n").unwrap();
    eventually("snapshot after enable", || snapshot_file(&env, "file.txt").as_deref() == Some("edited"));
}

#[test]
fn rewatches_a_recloned_repo() {
    let env = Env::new();
    env.unregister(&env.b);
    let watch = watching(&env, "a", Some(100));
    std::fs::remove_dir_all(&env.a).unwrap();
    env.git(&env.root, &["clone", "-q", "remote.git", "a"]);
    env.git(&env.a, &["config", "wip.remote", "origin"]);
    eventually("rewatch", || watch.log().matches("watching").count() >= 2);
    std::fs::write(env.a.join("file.txt"), "after reclone\n").unwrap();
    eventually("snapshot after reclone", || snapshot_file(&env, "file.txt").as_deref() == Some("after reclone"));
}
