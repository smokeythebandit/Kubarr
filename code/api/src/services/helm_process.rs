//! Bounded Helm execution. Call from `block_in_place` in asynchronous handlers.
use std::io::Read;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, Result};

const OUTPUT_LIMIT: u64 = 64 * 1024;

pub struct HelmOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

fn drain(mut pipe: impl Read) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    // Continue draining after the limit so Helm cannot block on full pipes.
    let mut buffer = [0; 8192];
    loop {
        let count = pipe.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let remaining = OUTPUT_LIMIT as usize - output.len();
        output.extend_from_slice(&buffer[..count.min(remaining)]);
    }
    Ok(output)
}

pub fn run(args: &[&str], timeout: Duration) -> Result<HelmOutput> {
    run_with_stop(args, timeout, None)
}

pub fn run_with_stop(
    args: &[&str],
    timeout: Duration,
    stop: Option<&CancellationToken>,
) -> Result<HelmOutput> {
    run_command_with_stop(Command::new("helm"), args, timeout, stop)
}

#[cfg(test)]
fn run_command(command: Command, args: &[&str], timeout: Duration) -> Result<HelmOutput> {
    run_command_with_stop(command, args, timeout, None)
}

fn run_command_with_stop(
    mut command: Command,
    args: &[&str],
    timeout: Duration,
    stop: Option<&CancellationToken>,
) -> Result<HelmOutput> {
    if stop.is_some_and(CancellationToken::is_cancelled) {
        return Err(AppError::Internal(
            "Stop requested; external outcome indeterminate".into(),
        ));
    }
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|e| AppError::Internal(format!("Failed to run helm: {e}")))?;
    let stdout = std::thread::spawn({
        let pipe = child.stdout.take().unwrap();
        move || drain(pipe)
    });
    let stderr = std::thread::spawn({
        let pipe = child.stderr.take().unwrap();
        move || drain(pipe)
    });
    let start = Instant::now();
    let mut stopped = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if stop.is_some_and(CancellationToken::is_cancelled) => {
                stopped = true;
                break None;
            }
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(50)),
            _ => break None,
        }
    };
    // Close pipes held by any remaining descendants even when Helm itself exits.
    // The child is its own process group; otherwise a lingering helper can hold a
    // reader thread indefinitely after a successful exit.
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let stdout = stdout
        .join()
        .map_err(|_| AppError::Internal("Helm stdout reader panicked".into()))?
        .map_err(|e| AppError::Internal(format!("Helm stdout read failed: {e}")))?;
    let stderr = stderr
        .join()
        .map_err(|_| AppError::Internal("Helm stderr reader panicked".into()))?
        .map_err(|e| AppError::Internal(format!("Helm stderr read failed: {e}")))?;
    if stopped {
        return Err(AppError::Internal(
            "Stop requested; Helm process group killed and reaped; external outcome indeterminate"
                .into(),
        ));
    }
    let status = status.ok_or_else(|| AppError::Internal(format!(
        "Helm deadline exceeded after {} seconds; process killed and reaped; external outcome indeterminate", timeout.as_secs()
    )))?;
    Ok(HelmOutput {
        status,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deadline_kills_and_reaps_child() {
        let start = Instant::now();
        let error = run_command(
            Command::new("sh"),
            &["-c", "sleep 10"],
            Duration::from_millis(100),
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("outcome indeterminate"));
        assert!(start.elapsed() < Duration::from_secs(3));
    }
    #[test]
    fn output_is_capped_while_draining() {
        let output = run_command(
            Command::new("sh"),
            &["-c", "yes x | head -c 200000"],
            Duration::from_secs(3),
        )
        .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), OUTPUT_LIMIT as usize);
    }
    #[test]
    fn stop_kills_running_process_group() {
        let stop = CancellationToken::new();
        let trigger = stop.clone();
        let signal = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            trigger.cancel();
        });
        let start = Instant::now();
        let error = run_command_with_stop(
            Command::new("sh"),
            &["-c", "sleep 10"],
            Duration::from_secs(15),
            Some(&stop),
        )
        .err()
        .unwrap();
        signal.join().unwrap();
        assert!(error.to_string().contains("killed and reaped"));
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}
