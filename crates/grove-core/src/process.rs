use std::io::{self, Read};
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
pub(crate) fn run(mut cmd: Command, timeout: Duration) -> Result<Output, RunError> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(RunError::Spawn)?;
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
