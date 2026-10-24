//! Bounded JSON lines over private pipes to a trusted native worker.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

use command_group::CommandGroup;
use serde_json::Value;

const MAX_MESSAGE: usize = 24 << 20;
static WORKERS: AtomicUsize = AtomicUsize::new(0);
const MAX_WORKERS: usize = 4;

type WriteJob = (Vec<u8>, SyncSender<Result<(), String>>);

pub(crate) struct Worker {
    input: SyncSender<WriteJob>,
    output: Receiver<Result<Value, String>>,
    stop: SyncSender<()>,
    pub(crate) deadline: Instant,
}

impl Worker {
    pub(crate) fn start(mut command: Command, lifetime: Duration) -> Result<Self, String> {
        let permit = Permit::acquire()?;
        let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
        // Workers get no server credentials or automatically loaded .env.
        command.env_clear().current_dir(directory.path());
        for name in ["PATH", "HOME", "TMPDIR", "DISPLAY", "XAUTHORITY"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .group_spawn()
            .map_err(|e| format!("couldn't start the worker: {e}"))?;
        let mut stdin = child
            .inner()
            .stdin
            .take()
            .ok_or("worker has no input pipe")?;
        let stdout = child
            .inner()
            .stdout
            .take()
            .ok_or("worker has no output pipe")?;
        let deadline = Instant::now() + lifetime;
        let (stop, stopped) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            // The timer also runs while an extension is between operations.
            let _ = stopped.recv_timeout(lifetime);
            let _ = child.kill();
            let _ = child.wait();
            drop(directory);
            drop(permit);
        });
        let (input, writes) = mpsc::sync_channel::<WriteJob>(1);
        std::thread::spawn(move || {
            while let Ok((bytes, done)) = writes.recv() {
                let result = stdin.write_all(&bytes).and_then(|()| stdin.flush());
                let failed = result.is_err();
                let _ = done.send(result.map_err(|e| e.to_string()));
                if failed {
                    break;
                }
            }
        });
        let (received, output) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                let result = reader
                    .by_ref()
                    .take(MAX_MESSAGE as u64 + 1)
                    .read_until(b'\n', &mut line)
                    .map_err(|e| e.to_string())
                    .and_then(|n| {
                        if n == 0 {
                            Err("worker closed its output".to_owned())
                        } else if n > MAX_MESSAGE || line.last() != Some(&b'\n') {
                            Err("worker response exceeds its limit".to_owned())
                        } else {
                            serde_json::from_slice(&line)
                                .map_err(|e| format!("invalid worker response: {e}"))
                        }
                    });
                let failed = result.is_err();
                if received.send(result).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            input,
            output,
            stop,
            deadline,
        })
    }

    pub(crate) fn send(&self, value: &Value, deadline: Instant) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        if bytes.len() >= MAX_MESSAGE {
            return Err("worker request exceeds its limit".to_owned());
        }
        bytes.push(b'\n');
        let (done, written) = mpsc::sync_channel(1);
        self.input
            .try_send((bytes, done))
            .map_err(|_| "worker is unavailable".to_owned())?;
        written
            .recv_timeout(remaining(deadline.min(self.deadline))?)
            .map_err(|_| "worker timed out while writing".to_owned())?
    }

    pub(crate) fn receive(&self, deadline: Instant) -> Result<Value, String> {
        self.output
            .recv_timeout(remaining(deadline.min(self.deadline))?)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => "worker timed out".to_owned(),
                mpsc::RecvTimeoutError::Disconnected => {
                    "worker stopped before responding".to_owned()
                }
            })?
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.stop.try_send(());
    }
}

pub(crate) fn remaining(deadline: Instant) -> Result<Duration, String> {
    deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(|| "worker timed out".to_owned())
}

struct Permit;

impl Permit {
    fn acquire() -> Result<Self, String> {
        WORKERS
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < MAX_WORKERS).then_some(n + 1)
            })
            .map(|_| Self)
            .map_err(|_| "all extension workers are busy; try again later".to_owned())
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        WORKERS.fetch_sub(1, Ordering::SeqCst);
    }
}
