mod commands;

use std::path::Path;
use wae_reporters::Format;

pub const EXIT_PASSED: i32 = 0;
pub const EXIT_VIOLATIONS: i32 = 1;
pub const EXIT_PROJECT: i32 = 2;
pub const EXIT_INTERNAL: i32 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}
impl CliOutput {
    pub fn success(stdout: impl Into<String>) -> Self {
        Self { exit_code: EXIT_PASSED, stdout: stdout.into(), stderr: String::new() }
    }
    pub fn violations(stdout: impl Into<String>) -> Self {
        Self { exit_code: EXIT_VIOLATIONS, stdout: stdout.into(), stderr: String::new() }
    }
    pub fn project_error(stderr: impl Into<String>) -> Self {
        Self { exit_code: EXIT_PROJECT, stdout: String::new(), stderr: stderr.into() }
    }
    pub fn internal_error(stderr: impl Into<String>) -> Self {
        Self { exit_code: EXIT_INTERNAL, stdout: String::new(), stderr: stderr.into() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Command {
    Init,
    Scan,
    Check { changed: bool, format: Option<Format>, base: Option<String> },
    BaselineCreate,
    Graph,
    Doctor,
    Explain(String),
    Help,
}

pub fn run(args: &[String], cwd: &Path) -> CliOutput {
    let command = match parse(args) {
        Ok(command) => command,
        Err(error) => return CliOutput::project_error(format!("{error}\n\n{}", usage())),
    };
    match command {
        Command::Init => commands::init(cwd),
        Command::Scan => commands::scan(cwd),
        Command::Check { changed, format, base } => commands::check(cwd, changed, format, base),
        Command::BaselineCreate => commands::baseline_create(cwd),
        Command::Graph => commands::graph(cwd),
        Command::Doctor => commands::doctor(cwd),
        Command::Explain(rule) => commands::explain(&rule),
        Command::Help => CliOutput::success(usage()),
    }
}

fn parse(args: &[String]) -> Result<Command, String> {
    let Some(command) = args.first().map(String::as_str) else { return Ok(Command::Help) };
    match command {
        "init" if args.len() == 1 => Ok(Command::Init),
        "scan" if args.len() == 1 => Ok(Command::Scan),
        "graph" if args.len() == 1 => Ok(Command::Graph),
        "doctor" if args.len() == 1 => Ok(Command::Doctor),
        "baseline" if args.get(1).map(String::as_str) == Some("create") && args.len() == 2 => {
            Ok(Command::BaselineCreate)
        }
        "explain" if args.len() == 2 => Ok(Command::Explain(args[1].clone())),
        "check" => parse_check(&args[1..]),
        "help" | "--help" | "-h" => Ok(Command::Help),
        _ => Err(format!("Invalid command or arguments: {}", args.join(" "))),
    }
}

fn parse_check(args: &[String]) -> Result<Command, String> {
    let mut changed = false;
    let mut format = None;
    let mut base = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--changed" => changed = true,
            "--format" => {
                index += 1;
                let value = args.get(index).ok_or("--format requires a value")?;
                format = Some(
                    Format::parse(value).ok_or_else(|| format!("unsupported format `{value}`"))?,
                );
            }
            "--base" => {
                index += 1;
                base = Some(args.get(index).ok_or("--base requires a value")?.clone());
            }
            value => return Err(format!("unknown check option `{value}`")),
        }
        index += 1;
    }
    Ok(Command::Check { changed, format, base })
}

fn usage() -> &'static str {
    "Usage: wae <COMMAND>\n\nCommands:\n  init                         Create wae.yaml\n  scan                         Analyze and report module count\n  check [--changed] [--base REF] [--format human|json|jsonl|sarif]\n  baseline create              Explicitly record current violations\n  graph                        Print the real dependency graph as JSON\n  doctor                       Validate project/config/tooling\n  explain <RULE_ID>            Explain an architecture rule\n\nExit codes: 0 passed, 1 violations, 2 config/project error, 3 internal error"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(name)
    }
    #[test]
    fn circular_fixture_runs_end_to_end_without_a_diagnostic_input_file() {
        let output = run(&["check".into(), "--format".into(), "json".into()], &fixture("circular"));
        assert_eq!(output.exit_code, EXIT_VIOLATIONS);
        assert!(output.stdout.contains("ARCH-001"));
        assert!(!fixture("circular").join("wae.violations").exists());
    }
    #[test]
    fn basic_fixture_passes() {
        assert_eq!(run(&["check".into()], &fixture("basic")).exit_code, EXIT_PASSED);
    }
    #[test]
    fn malformed_config_uses_project_exit_code() {
        let root = std::env::temp_dir().join(format!("wae-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("wae.yaml"), "bad: [").unwrap();
        assert_eq!(run(&["check".into()], &root).exit_code, EXIT_PROJECT);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_mode_never_creates_a_missing_baseline() {
        let root = fixture("basic");
        let output = run(&["check".into(), "--changed".into()], &root);
        assert_eq!(output.exit_code, EXIT_PROJECT);
        assert!(output.stderr.contains("baseline is missing"));
        assert!(!root.join(".wae/baseline.json").exists());
    }

    #[test]
    fn check_uses_the_configured_output_format_by_default() {
        let root = std::env::temp_dir().join(format!("wae-output-test-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.ts"), "export const value = 1;").unwrap();
        std::fs::write(root.join("wae.yaml"), "version: 1\noutput:\n  format: json\n").unwrap();
        let output = run(&["check".into()], &root);
        assert_eq!(output.exit_code, EXIT_PASSED);
        assert!(output.stdout.contains("\"schemaVersion\""));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_check_accepts_an_explicit_base() {
        let command =
            parse(&["check".into(), "--changed".into(), "--base".into(), "origin/main".into()])
                .unwrap();
        assert!(matches!(
            command,
            Command::Check { changed: true, base: Some(base), .. } if base == "origin/main"
        ));
    }
}
