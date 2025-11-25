//! Subprocess execution with proper signal handling.
//!
//! This module provides utilities to run external commands while properly
//! forwarding signals (SIGINT, SIGTERM, etc.) to the child process.
//! This ensures that Ctrl+C is handled by the child, not by m87.

use anyhow::{Context, Result};
use std::process::{Command, Stdio};

/// Builder for running external commands with signal forwarding.
pub struct SubprocessBuilder {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
}

impl SubprocessBuilder {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
        }
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Run the command with signal forwarding.
    /// This function does NOT return on success - it calls std::process::exit().
    pub fn exec(self) -> Result<()> {
        #[cfg(unix)]
        {
            self.exec_unix()
        }
        #[cfg(windows)]
        {
            self.exec_windows()
        }
    }
}

#[cfg(unix)]
impl SubprocessBuilder {
    fn exec_unix(self) -> Result<()> {
        use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};

        // Ignore SIGINT in the parent process BEFORE spawning child.
        // The child inherits the TTY and will receive SIGINT directly from the terminal.
        // We ignore it here so the parent waits for the child to handle it.
        // SAFETY: Setting signal handler to SIG_IGN is safe
        unsafe {
            let ignore = SigAction::new(SigHandler::SigIgn, SaFlags::empty(), SigSet::empty());
            let _ = sigaction(Signal::SIGINT, &ignore);
            let _ = sigaction(Signal::SIGQUIT, &ignore);
        }

        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        for (k, v) in &self.env {
            cmd.env(k, v);
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("Failed to spawn {}", self.program))?;

        let status = child.wait()?;

        std::process::exit(status.code().unwrap_or(1));
    }
}

#[cfg(windows)]
impl SubprocessBuilder {
    fn exec_windows(self) -> Result<()> {
        // Windows: no signal forwarding, just run normally
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        for (k, v) in &self.env {
            cmd.env(k, v);
        }

        let status = cmd
            .status()
            .with_context(|| format!("Failed to execute {}", self.program))?;

        std::process::exit(status.code().unwrap_or(1));
    }
}
