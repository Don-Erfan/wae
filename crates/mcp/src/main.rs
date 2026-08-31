use std::io::{self, BufRead, Write};

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
            _ => {
                eprintln!("wae-mcp: unknown option `{argument}`");
                std::process::exit(2);
            }
        }
    }
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let response = wae_mcp::handle_line(&line, &root, &policy);
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
