use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    process::Stdio,
};

use itertools::Itertools;
use turbopath::AbsoluteSystemPathBuf;

/// A command builder that can be used to build both regular
/// child processes and ones spawned hooked up to a PTY
pub struct Command {
    program: OsString,
    args: Vec<OsString>,
    cwd: Option<AbsoluteSystemPathBuf>,
    env: BTreeMap<OsString, OsString>,
    open_stdin: bool,
    env_clear: bool,
}

impl Command {
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        let program = program.as_ref().to_os_string();
        Self {
            program,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            open_stdin: false,
            env_clear: false,
        }
    }

    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args = args
            .into_iter()
            .map(|arg| arg.as_ref().to_os_string())
            .collect();
        self
    }

    pub fn current_dir(&mut self, dir: AbsoluteSystemPathBuf) -> &mut Self {
        self.cwd = Some(dir);
        self
    }

    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.env = vars
            .into_iter()
            .map(|(k, v)| (k.as_ref().to_os_string(), v.as_ref().to_os_string()))
            .collect();
        self
    }

    pub fn env<K, V>(&mut self, key: K, val: V) -> &mut Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.env
            .insert(key.as_ref().to_os_string(), val.as_ref().to_os_string());
        self
    }

    /// Configure if the child process should spawn with stdin or not
    pub fn open_stdin(&mut self, open_stdin: bool) -> &mut Self {
        self.open_stdin = open_stdin;
        self
    }

    /// Clears the environment variables for the child process
    pub fn env_clear(&mut self) -> &mut Self {
        self.env_clear = true;
        self
    }

    pub fn label(&self) -> String {
        format!(
            "({}) {} {}",
            self.cwd
                .as_deref()
                .map(|dir| dir.as_str())
                .unwrap_or_default(),
            self.program.to_string_lossy(),
            self.args.iter().map(|s| s.to_string_lossy()).join(" ")
        )
    }
}

impl From<Command> for tokio::process::Command {
    fn from(value: Command) -> Self {
        let Command {
            program,
            args,
            cwd,
            env,
            open_stdin,
            env_clear,
        } = value;

        let mut cmd = tokio::process::Command::new(program);
        if env_clear {
            cmd.env_clear();
        }
        cmd.args(args)
            .envs(env)
            // We always pipe stdout/stderr to allow us to capture task output
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Only open stdin if configured to do so
            .stdin(if open_stdin {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd.as_std_path());
        }
        cmd
    }
}
