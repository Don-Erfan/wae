use std::env;

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let cwd = env::current_dir().unwrap_or_default();
    let cancellation = wae_engine::CancellationToken::default();
    let signal_token = cancellation.clone();
    if let Err(error) = ctrlc::set_handler(move || signal_token.cancel()) {
        eprintln!("could not install Ctrl+C handler: {error}");
        std::process::exit(wae_cli::EXIT_INTERNAL);
    }
    let output = wae_cli::run_with_cancellation(&args, &cwd, &cancellation);
    if !output.stdout.is_empty() {
        println!("{}", output.stdout);
    }
    if !output.stderr.is_empty() {
        eprintln!("{}", output.stderr);
    }
    std::process::exit(output.exit_code);
}
