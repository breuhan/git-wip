use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub type Result<T> = std::result::Result<T, String>;

pub struct Git {
    dir: PathBuf,
    env: Vec<(String, String)>,
}

impl Git {
    pub fn new(dir: impl Into<PathBuf>) -> Git {
        Git {
            dir: dir.into(),
            env: vec![],
        }
    }

    /// Sets an environment variable for every git call.
    pub fn with_env(mut self, key: &str, value: &str) -> Git {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn run(&self, args: &[&str]) -> Result<String> {
        self.run_with(args, &[], None)
    }

    pub fn ok(&self, args: &[&str]) -> bool {
        self.run(args).is_ok()
    }

    pub fn run_with(&self, args: &[&str], env: &[(&str, &str)], input: Option<&[u8]>) -> Result<String> {
        match self.run_code(args, env, input)? {
            (0, out, _) => Ok(out),
            (_, _, err) => Err(err),
        }
    }

    /// (exit code, stdout, error message), for commands whose exit code 1 is an answer
    /// (merge-tree: conflicts).
    pub fn run_code(&self, args: &[&str], env: &[(&str, &str)], input: Option<&[u8]>) -> Result<(i32, String, String)> {
        let mut child = Command::new("git")
            .current_dir(&self.dir)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .envs(env.iter().copied())
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("git: {e}"))?;
        if let Some(data) = input {
            child.stdin.take().unwrap().write_all(data).map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
        let err = format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
        Ok((out.status.code().unwrap_or(-1), stdout, err))
    }

    pub fn path(&self, git_path: &str) -> Result<PathBuf> {
        Ok(self.dir.join(self.run(&["rev-parse", "--git-path", git_path])?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_env_reaches_git() {
        let g = Git::new(std::env::temp_dir()).with_env("GIT_WIP_TEST_VAR", "passed");
        assert_eq!(
            g.run(&["-c", "alias.v=!echo $GIT_WIP_TEST_VAR", "v"]).unwrap(),
            "passed"
        );
    }
}
