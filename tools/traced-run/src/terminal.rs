//! Inherited controlling-terminal handoff for the POSIX process-group backend.
//!
//! Processkit provides containment but no inherited-terminal handoff hook. A
//! private reentry takes the foreground before exec (not after spawning the
//! user's program, which races its first read). No extra PTY or stream pumps.

use nix::sys::signal::{SigSet, SigmaskHow, Signal, pthread_sigmask};
use nix::unistd::{Pid, getpgrp, tcgetpgrp, tcsetpgrp};
use std::io;

pub(crate) const REENTRY: &str = "--__traced-run-terminal-exec";

fn set_foreground(group: Pid) -> nix::Result<()> {
    // tcsetpgrp from a background group would otherwise stop the caller. Block
    // only on this thread, and restore the mask before returning or execing.
    let mut blocked = SigSet::empty();
    blocked.add(Signal::SIGTTOU);
    let mut previous = SigSet::empty();
    pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&blocked), Some(&mut previous))?;
    let result = tcsetpgrp(io::stdin(), group);
    let restored = pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&previous), None);
    result.and(restored)
}

pub(crate) struct Foreground(Pid);

impl Foreground {
    pub(crate) fn acquire(group: &processkit::ProcessGroup) -> Option<Self> {
        if group.mechanism() != processkit::Mechanism::ProcessGroup {
            return None;
        }
        let foreground = tcgetpgrp(io::stdin()).ok()?;
        // Do not steal a terminal from another foreground job. Pipes, files,
        // null stdin and non-controlling terminals all retain the ordinary path.
        (foreground == getpgrp()).then_some(Self(foreground))
    }

    pub(crate) fn group(&self) -> String {
        self.0.to_string()
    }
}

impl Drop for Foreground {
    fn drop(&mut self) {
        if let Err(error) = set_foreground(self.0) {
            eprintln!("traced-run: warning: restoring terminal foreground failed: {error}");
        }
    }
}

pub(crate) fn exec(args: &[String]) -> ! {
    use std::os::unix::process::CommandExt;
    let result = (|| -> Result<(), String> {
        let (expected, argv) = args.split_first().ok_or("missing terminal owner")?;
        let expected = expected.parse::<i32>().map_err(|e| e.to_string())?;
        super::validate_argv(argv).map_err(|e| e.message)?;
        if tcgetpgrp(io::stdin()).map_err(|e| e.to_string())? != Pid::from_raw(expected) {
            return Err("terminal foreground changed before handoff".into());
        }
        set_foreground(getpgrp()).map_err(|e| e.to_string())?;
        let error = std::process::Command::new(&argv[0]).args(&argv[1..]).exec();
        Err(error.to_string())
    })();
    if let Err(error) = result {
        eprintln!("traced-run: terminal exec: {error}");
    }
    std::process::exit(64);
}
