#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

pub struct Env {
    pub root: PathBuf,
    pub remote: PathBuf,
    pub a: PathBuf,
    pub b: PathBuf,
}

impl Env {
    /// Bare remote with one commit on main, cloned as `a` and `b`, both enabled.
    pub fn new() -> Env {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("git-wip-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("home")).unwrap();
        let env = Env { remote: root.join("remote.git"), a: root.join("a"), b: root.join("b"), root };
        env.git(&env.root, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
        env.git(&env.root, &["clone", "-q", "remote.git", "a"]);
        std::fs::write(env.a.join("file.txt"), "one\n").unwrap();
        env.git(&env.a, &["add", "file.txt"]);
        env.git(&env.a, &["commit", "-q", "-m", "init"]);
        env.git(&env.a, &["push", "-q", "origin", "main"]);
        env.git(&env.root, &["clone", "-q", "remote.git", "b"]);
        env.wip_ok(&env.a, "a", 0, &["enable", "origin"]);
        env.wip_ok(&env.b, "b", 0, &["enable", "origin"]);
        env
    }

    pub fn envs(&self, date: i64) -> Vec<(String, String)> {
        let d = format!("@{} +0000", 1_700_000_000 + date);
        vec![
            ("HOME".into(), self.root.join("home").display().to_string()),
            ("XDG_STATE_HOME".into(), self.root.join("home/.local/state").display().to_string()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ("GIT_AUTHOR_NAME".into(), "t".into()),
            ("GIT_AUTHOR_EMAIL".into(), "t@t".into()),
            ("GIT_COMMITTER_NAME".into(), "t".into()),
            ("GIT_COMMITTER_EMAIL".into(), "t@t".into()),
            ("GIT_AUTHOR_DATE".into(), d.clone()),
            ("GIT_COMMITTER_DATE".into(), d),
        ]
    }

    pub fn git(&self, dir: &Path, args: &[&str]) -> String {
        self.git_at(dir, 0, args)
    }

    pub fn git_at(&self, dir: &Path, date: i64, args: &[&str]) -> String {
        let out = Command::new("git").current_dir(dir).args(args).envs(self.envs(date)).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim_end().to_string()
    }

    /// Runs git-wip as `host` with commit time offset `date` (seconds).
    pub fn wip(&self, dir: &Path, host: &str, date: i64, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_git-wip"))
            .current_dir(dir)
            .args(args)
            .envs(self.envs(date))
            .env("GIT_WIP_HOST", host)
            .output()
            .unwrap()
    }

    pub fn wip_ok(&self, dir: &Path, host: &str, date: i64, args: &[&str]) -> String {
        let out = self.wip(dir, host, date, args);
        assert!(out.status.success(), "git wip {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stderr).to_string()
    }

    pub fn remote_ref(&self, name: &str) -> Option<String> {
        let out = self.git(&self.root, &["--git-dir", "remote.git", "for-each-ref", "--format=%(objectname)", name]);
        (!out.is_empty()).then_some(out)
    }

    /// Repos registered by `git wip enable`.
    pub fn repos(&self) -> Vec<String> {
        let file = self.root.join("home/.local/state/git-wip/repos");
        if !file.exists() {
            return vec![];
        }
        let out = self.git(&self.root, &["config", "--file", file.to_str().unwrap(), "--get-all", "wip.repo"]);
        out.lines().map(str::to_string).collect()
    }

    /// Removes `dir` from the repo list but keeps its `wip.remote`, so only one clone is watched.
    pub fn unregister(&self, dir: &Path) {
        let file = self.root.join("home/.local/state/git-wip/repos");
        let top = self.git(dir, &["rev-parse", "--show-toplevel"]);
        self.git(
            &self.root,
            &["config", "--file", file.to_str().unwrap(), "--fixed-value", "--unset-all", "wip.repo", &top],
        );
    }

    /// Starts `git wip watch` as `host` with short debounce and fetch intervals; `None` uses the real clock.
    pub fn watch(&self, host: &str, date: Option<i64>) -> Watch {
        let envs = self.envs(date.unwrap_or(0));
        let envs = envs.into_iter().filter(|(k, _)| date.is_some() || !k.ends_with("_DATE"));
        let mut child = Command::new(env!("CARGO_BIN_EXE_git-wip"))
            .current_dir(&self.root)
            .arg("watch")
            .envs(envs)
            .env("GIT_WIP_HOST", host)
            .env("GIT_WIP_DEBOUNCE_MS", "300")
            .env("GIT_WIP_FETCH_SECS", "1")
            .env("GIT_WIP_DEBUG", "1")
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let log = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let (stderr, sink) = (child.stderr.take().unwrap(), log.clone());
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
                sink.lock().unwrap().push_str(&(line + "\n"));
            }
        });
        Watch { child, log }
    }

    pub fn read(&self, dir: &Path, file: &str) -> String {
        std::fs::read_to_string(dir.join(file)).unwrap()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub struct Watch {
    child: std::process::Child,
    log: std::sync::Arc<std::sync::Mutex<String>>,
}

impl Watch {
    /// What the watcher logged so far.
    pub fn log(&self) -> String {
        self.log.lock().unwrap().clone()
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Polls `f` for up to 15 seconds.
pub fn eventually(what: &str, f: impl Fn() -> bool) {
    for _ in 0..150 {
        if f() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("timed out waiting for {what}");
}
