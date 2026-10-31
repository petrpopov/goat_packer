//! Stream native subprocess diagnostics without buffering untrusted output in memory.
use std::{io::{self, Read}, process::{Command, Output, Stdio}, thread, time::{Duration, Instant}};

const TAIL: usize = 64 * 1024;
const LINE: usize = 8192;
const STREAM_LOG_LIMIT: usize = 5 * 1024 * 1024;
/// Structured mode: a `slicer_progress` heartbeat this often while the slicer runs. The
/// slicers print no progress of their own, and a 5-minute final slice must not look frozen.
const HEARTBEAT: Duration = Duration::from_secs(2);

fn drain(mut input: impl Read, stream: &str, structured: bool) -> io::Result<Vec<u8>> {
    let mut tail = Vec::with_capacity(TAIL);
    let mut line = Vec::with_capacity(LINE);
    let mut emitted = 0;
    let mut buffer = [0; 4096];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 { break; }
        if tail.len() + n > TAIL { tail.drain(..tail.len() + n - TAIL); }
        tail.extend_from_slice(&buffer[..n]);
        for &byte in &buffer[..n] {
            if byte == b'\n' {
                if structured && emitted < STREAM_LOG_LIMIT {
                    eprintln!("{}", serde_json::json!({"event":"slicer_output","stream":stream,"message":String::from_utf8_lossy(&line)}));
                    emitted += line.len() + 1;
                }
                line.clear();
            } else if line.len() < LINE { line.push(byte); }
        }
    }
    if structured && !line.is_empty() && emitted < STREAM_LOG_LIMIT {
        eprintln!("{}", serde_json::json!({"event":"slicer_output","stream":stream,"message":String::from_utf8_lossy(&line)}));
    }
    Ok(tail)
}

pub fn bounded_output(command: &mut Command) -> io::Result<Output> {
    let started = Instant::now();
    let structured = std::env::var("GOAT_LOG_FORMAT").as_deref() == Ok("json");
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    if structured { eprintln!("{}",serde_json::json!({"event":"slicer_started","pid":child.id()})); }
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out = thread::spawn(move || drain(stdout, "stdout", structured));
    let err = thread::spawn(move || drain(stderr, "stderr", structured));
    let status = if structured {
        let mut last = Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if last.elapsed() >= HEARTBEAT {
                last = Instant::now();
                eprintln!("{}",serde_json::json!({"event":"slicer_progress","elapsed_ms":started.elapsed().as_millis()}));
            }
            thread::sleep(Duration::from_millis(100));
        }
    } else {
        child.wait()?
    };
    let stdout = out.join().map_err(|_| io::Error::other("stdout reader panicked"))??;
    let stderr = err.join().map_err(|_| io::Error::other("stderr reader panicked"))??;
    if structured { eprintln!("{}",serde_json::json!({"event":"slicer_finished","exit_code":status.code(),"elapsed_ms":started.elapsed().as_millis()})); }
    Ok(Output { status, stdout, stderr })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_huge_line_and_preserves_tail() {
        let mut bytes = vec![b'x'; 2_000_000]; bytes.extend_from_slice(b"\nlast diagnostic\n");
        let output = drain(&bytes[..], "test", false).unwrap();
        assert_eq!(output.len(), TAIL);
        assert!(output.ends_with(b"last diagnostic\n"));
    }
    #[cfg(unix)]
    #[test]
    fn drains_both_pipes_without_deadlock() {
        let output = bounded_output(Command::new("sh").args(["-c", "i=0; while [ $i -lt 12000 ]; do echo out; echo err >&2; i=$((i+1)); done; exit 7"])).unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert!(output.stdout.ends_with(b"out\n")); assert!(output.stderr.ends_with(b"err\n"));
    }
}
