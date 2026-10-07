use std::io::{self, Read, Write};
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

/// Runs `cmd` to completion, killing it if it outlives `timeout`.
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
