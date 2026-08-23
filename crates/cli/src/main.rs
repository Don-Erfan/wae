use std::env;

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let cwd = env::current_dir().unwrap_or_default();
    let output = wae_cli::run(&args, &cwd);
    if !output.stdout.is_empty() {
        println!("{}", output.stdout);
    }
    if !output.stderr.is_empty() {
        eprintln!("{}", output.stderr);
    }
    std::process::exit(output.exit_code);
}
