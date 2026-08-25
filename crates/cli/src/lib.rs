mod commands;

use std::path::Path;
use wae_config::ConfigPreset;
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
    Init { preset: ConfigPreset },
    Scan,
    Check { changed: bool, format: Option<Format>, base: Option<String> },
    BaselineCreate,
    Graph,
    Doctor,
    ConfigValidate { show_overlaps: bool },
    Explain(String),
    Version,
    Help,
}

pub fn run(args: &[String], cwd: &Path) -> CliOutput {
    let command = match parse(args) {
        Ok(command) => command,
        Err(error) => return CliOutput::project_error(format!("{error}\n\n{}", usage())),
    };
    match command {
        Command::Init { preset } => commands::init(cwd, preset),
        Command::Scan => commands::scan(cwd),
        Command::Check { changed, format, base } => commands::check(cwd, changed, format, base),
        Command::BaselineCreate => commands::baseline_create(cwd),
        Command::Graph => commands::graph(cwd),
        Command::Doctor => commands::doctor(cwd),
        Command::ConfigValidate { show_overlaps } => commands::config_validate(cwd, show_overlaps),
        Command::Explain(rule) => commands::explain(&rule),
        Command::Version => CliOutput::success(format!("wae {}", env!("CARGO_PKG_VERSION"))),
        Command::Help => CliOutput::success(usage()),
    }
}

fn parse(args: &[String]) -> Result<Command, String> {
    let Some(command) = args.first().map(String::as_str) else { return Ok(Command::Help) };
    match command {
        "init" => parse_init(&args[1..]),
        "scan" if args.len() == 1 => Ok(Command::Scan),
        "graph" if args.len() == 1 => Ok(Command::Graph),
        "doctor" if args.len() == 1 => Ok(Command::Doctor),
        "config" if args.get(1).map(String::as_str) == Some("validate") => {
            parse_config_validate(&args[2..])
        }
        "baseline" if args.get(1).map(String::as_str) == Some("create") && args.len() == 2 => {
            Ok(Command::BaselineCreate)
        }
        "explain" if args.len() == 2 => Ok(Command::Explain(args[1].clone())),
        "check" => parse_check(&args[1..]),
        "--version" | "-V" if args.len() == 1 => Ok(Command::Version),
        "help" | "--help" | "-h" => Ok(Command::Help),
        _ => Err(format!("Invalid command or arguments: {}", args.join(" "))),
    }
}

fn parse_init(args: &[String]) -> Result<Command, String> {
    if args.is_empty() {
        return Ok(Command::Init { preset: ConfigPreset::Blank });
    }
    if args.len() != 2 || args[0] != "--preset" {
        return Err("init accepts only `--preset blank|fsd|next|nx`".into());
    }
    let preset = match args[1].as_str() {
        "blank" => ConfigPreset::Blank,
        "fsd" => ConfigPreset::Fsd,
        "next" => ConfigPreset::Next,
        "nx" => ConfigPreset::Nx,
        value => return Err(format!("unknown init preset `{value}`")),
    };
    Ok(Command::Init { preset })
}

fn parse_config_validate(args: &[String]) -> Result<Command, String> {
    match args {
        [] => Ok(Command::ConfigValidate { show_overlaps: false }),
        [option] if option == "--show-overlaps" => {
            Ok(Command::ConfigValidate { show_overlaps: true })
        }
        _ => Err("config validate accepts only `--show-overlaps`".into()),
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
    "Usage: wae <COMMAND>\n\nCommands:\n  init [--preset blank|fsd|next|nx]\n                               Create a safe, explicit wae.yaml\n  scan                         Analyze and report module/dependency counts\n  check [--changed] [--base REF] [--format human|json|jsonl|sarif]\n  baseline create              Explicitly record current violations\n  config validate [--show-overlaps]\n                               Validate config and layer ownership\n  graph                        Print the real dependency graph as JSON\n  doctor                       Validate project/config/tooling with actionable errors\n  explain <RULE_ID>            Explain an architecture rule\n\nOptions:\n  -V, --version                Print the installed WAE version\n  -h, --help                   Print help\n\nExit codes: 0 passed, 1 violations, 2 config/project error, 3 internal error"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(name)
    }
    #[test]
    fn version_flags_report_the_package_version() {
        for flag in ["-V", "--version"] {
            let output = run(&[flag.into()], Path::new("."));
            assert_eq!(output.exit_code, EXIT_PASSED);
            assert_eq!(output.stdout, format!("wae {}", env!("CARGO_PKG_VERSION")));
        }
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

    #[test]
    fn init_defaults_to_blank_and_accepts_explicit_presets() {
        assert_eq!(parse(&["init".into()]).unwrap(), Command::Init { preset: ConfigPreset::Blank });
        assert_eq!(
            parse(&["init".into(), "--preset".into(), "fsd".into()]).unwrap(),
            Command::Init { preset: ConfigPreset::Fsd }
        );
    }

    #[test]
    fn config_validation_parses_overlap_reporting() {
        assert_eq!(
            parse(&["config".into(), "validate".into(), "--show-overlaps".into()]).unwrap(),
            Command::ConfigValidate { show_overlaps: true }
        );
    }

    #[test]
    fn init_blank_is_safe_and_fsd_is_anchored() {
        for (preset, expected) in [("blank", "layers: {}"), ("fsd", "src/shared/**")] {
            let root =
                std::env::temp_dir().join(format!("wae-init-{preset}-{}", std::process::id()));
            std::fs::create_dir_all(&root).unwrap();
            let output = run(&["init".into(), "--preset".into(), preset.into()], &root);
            assert_eq!(output.exit_code, EXIT_PASSED);
            let config = std::fs::read_to_string(root.join("wae.yaml")).unwrap();
            assert!(config.contains(expected), "generated config: {config}");
            assert!(!config.contains("**/shared/**"));
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn config_validate_and_doctor_report_actionable_layer_overlaps() {
        let root = std::env::temp_dir().join(format!("wae-overlap-cli-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src/components/app/quests/components/shared")).unwrap();
        std::fs::write(
            root.join("src/components/app/quests/components/shared/file.ts"),
            "export const value = true;",
        )
        .unwrap();
        std::fs::write(
            root.join("wae.yaml"),
            "version: 1\narchitecture:\n  layers:\n    app:\n      patterns: ['**/app/**']\n    shared:\n      patterns: ['**/shared/**']\n",
        )
        .unwrap();
        let validation =
            run(&["config".into(), "validate".into(), "--show-overlaps".into()], &root);
        assert_eq!(validation.exit_code, EXIT_PROJECT);
        assert!(validation.stderr.contains("app, shared"));
        assert!(validation.stderr.contains("Anchor `shared` to `src/shared/**`"));

        let doctor = run(&["doctor".into()], &root);
        assert_eq!(doctor.exit_code, EXIT_PROJECT);
        assert!(doctor.stderr.contains("matches multiple architecture layers"));
        assert!(doctor.stderr.contains("wae config validate --show-overlaps"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
