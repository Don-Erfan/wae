use std::io::{self, BufRead, Write};

fn main() {
    let root = std::env::current_dir().unwrap_or_default();
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let response = serde_json::from_str(&line)
            .ok()
            .and_then(|message| wae_mcp::handle_message(message, &root));
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
