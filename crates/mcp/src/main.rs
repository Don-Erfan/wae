use std::io::{self, BufRead, Write};
use std::time::Duration;

fn main() {
    let root = std::env::current_dir().unwrap_or_default();
    let mut policy = wae_mcp::ServerPolicy::confined(&root);
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--allow-root" => {
                let Some(path) = arguments.next() else {
                    eprintln!("wae-mcp: --allow-root requires a path");
                    std::process::exit(2);
                };
                policy = policy.with_allowed_root(path.into());
            }
            "--allow-any-root" => policy = policy.allow_any_root(),
            "--max-request-bytes" => {
                let Some(bytes) = arguments.next().and_then(|value| value.parse().ok()) else {
                    eprintln!("wae-mcp: --max-request-bytes requires a positive integer");
                    std::process::exit(2);
                };
                policy = policy.with_max_request_bytes(bytes);
            }
            "--max-sessions" => {
                let Some(sessions) = arguments
                    .next()
                    .and_then(|value| value.parse().ok())
                    .filter(|sessions: &usize| *sessions > 0)
                else {
                    eprintln!("wae-mcp: --max-sessions requires a positive integer");
                    std::process::exit(2);
                };
                policy = policy.with_max_sessions(sessions);
            }
            "--session-ttl-seconds" => {
                let Some(seconds) = arguments.next().and_then(|value| value.parse().ok()) else {
                    eprintln!("wae-mcp: --session-ttl-seconds requires a non-negative integer");
                    std::process::exit(2);
                };
                policy = policy.with_session_ttl(Duration::from_secs(seconds));
            }
            _ => {
                eprintln!("wae-mcp: unknown option `{argument}`");
                std::process::exit(2);
            }
        }
    }
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let mut stdout = io::stdout().lock();
    let server = wae_mcp::McpServer::new(&root, policy.clone());
    while let Ok(Some(line)) = read_bounded_line(&mut stdin, policy.max_request_bytes()) {
        let line = String::from_utf8_lossy(&line).into_owned();
        if line.trim().is_empty() {
            continue;
        }
        let response = server.handle_line(&line);
        if let Some(response) = response {
            if serde_json::to_writer(&mut stdout, &response).is_err()
                || writeln!(stdout).is_err()
                || stdout.flush().is_err()
            {
                break;
            }
        }
    }
}

fn read_bounded_line(reader: &mut impl BufRead, maximum: usize) -> io::Result<Option<Vec<u8>>> {
    let mut output = Vec::with_capacity(maximum.min(8 * 1024));
    let mut saw_input = false;
    let mut overflow = false;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(saw_input.then_some(output));
        }
        saw_input = true;
        let consumed =
            buffer.iter().position(|byte| *byte == b'\n').map_or(buffer.len(), |i| i + 1);
        let ended = buffer.get(consumed.saturating_sub(1)) == Some(&b'\n');
        let payload = &buffer[..consumed];
        let payload = payload.strip_suffix(b"\n").unwrap_or(payload);
        let remaining = maximum.saturating_add(1).saturating_sub(output.len());
        output.extend_from_slice(&payload[..payload.len().min(remaining)]);
        overflow |= payload.len() > remaining;
        reader.consume(consumed);
        if ended {
            if overflow && output.len() <= maximum {
                output.push(0);
            }
            return Ok(Some(output));
        }
    }
}
