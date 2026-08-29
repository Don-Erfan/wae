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
        let response = serde_json::from_str(&line)
            .ok()
            .and_then(|message| wae_mcp::handle_message_with_policy(message, &root, &policy));
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
