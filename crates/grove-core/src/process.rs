use std::io::{self, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) struct Output {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug)]
pub(crate) enum RunError {
    Spawn(io::Error),
    Timeout,
}

/// Runs `cmd` to completion, killing it and everything it spawned if it outlives `timeout`.
pub(crate) fn run(cmd: Command, timeout: Duration) -> Result<Output, RunError> {
    run_with_input(cmd, Vec::new(), timeout)
}

/// Like `run`, writing `input` to its stdin (closed when written).
pub(crate) fn run_with_input(
    mut cmd: Command,
    input: Vec<u8>,
    timeout: Duration,
) -> Result<Output, RunError> {
    let stdin = if input.is_empty() {
        Stdio::null()
    } else {
        Stdio::piped()
    };
    // SAFETY: setsid is async-signal-safe. Every child (git, the login-shell PATH lookup,
    // mdfind) gets its own session, so a timeout kills everything it spawned (e.g. `fetch
    // --all`'s per-remote fetches and ssh) and nothing can block on a tty prompt.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(RunError::Spawn)?;
    if let Some(mut pipe) = child.stdin.take() {
        // On its own thread so a child that fills stdout before reading all its input can't
        // deadlock us.
        thread::spawn(move || {
            let _ = pipe.write_all(&input);
        });
    }
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                // Session leader, so its pid is the group id.
                if let Ok(pgid) = libc::pid_t::try_from(child.id()) {
                    // SAFETY: plain syscall on a group we created.
                    unsafe { libc::killpg(pgid, libc::SIGKILL) };
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::Timeout);
            }
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(e) => return Err(RunError::Spawn(e)),
        }
    };
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_kills_grandchildren() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pid_file = tmp.path().join("pid");
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg("sleep 30 & echo $! > \"$1\"; wait")
            .arg("sh")
            .arg(&pid_file);
        let started = Instant::now();
        assert!(matches!(
            run(cmd, Duration::from_millis(300)),
            Err(RunError::Timeout)
        ));
        assert!(started.elapsed() < Duration::from_secs(5));
        let pid: libc::pid_t = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        // SAFETY: signal 0 only checks the pid exists.
        while unsafe { libc::kill(pid, 0) } == 0 {
            assert!(Instant::now() < deadline, "grandchild {pid} survived");
            thread::sleep(Duration::from_millis(10));
        }
    }
}
