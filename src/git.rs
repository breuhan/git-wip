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
        Git { dir: dir.into(), env: vec![] }
    }

    pub fn with_env(mut self, key: &str, value: &str) -> Git {
        self.env.push((key.into(), value.into()));
        self
    }

    /// The object id of a revision, if it exists.
    pub fn rev(&self, rev: &str) -> Option<String> {
        self.run(&["rev-parse", "-q", "--verify", rev]).ok()
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
        // Write the input while reading the output: git may fill its output pipe before it has
        // read everything, and then both sides would wait forever.
        let stdin = child.stdin.take().zip(input);
        let out = std::thread::scope(|s| {
            s.spawn(|| stdin.map(|(mut pipe, data)| pipe.write_all(data)));
            child.wait_with_output().map_err(|e| e.to_string())
        })?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
        let err = format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
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
        assert_eq!(g.run(&["-c", "alias.v=!echo $GIT_WIP_TEST_VAR", "v"]).unwrap(), "passed");
    }

    #[test]
    fn large_input_with_large_output_does_not_deadlock() {
        let g = Git::new(std::env::temp_dir());
        let data = vec![b'x'; 1 << 20];
        let out = g.run_with(&["-c", "alias.c=!cat", "c"], &[], Some(&data)).unwrap();
        assert_eq!(out.len(), data.len());
    }
}
