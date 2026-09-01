mod commands;

use std::path::{Path, PathBuf};
use wae_config::{ConfigPreset, FailOn};
use wae_core::domain::DependencyKind;
use wae_engine::CancellationToken;
use wae_reporters::Format;

pub const EXIT_PASSED: i32 = 0;
pub const EXIT_VIOLATIONS: i32 = 1;
pub const EXIT_PROJECT: i32 = 2;
pub const EXIT_INTERNAL: i32 = 3;
pub const EXIT_CANCELLED: i32 = 130;

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
    pub fn cancelled() -> Self {
        Self {
            exit_code: EXIT_CANCELLED,
            stdout: String::new(),
            stderr: "analysis cancelled".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Command {
    Init {
        preset: ConfigPreset,
    },
    Scan,
    Discover {
        json: bool,
        write: bool,
        force: bool,
    },
    Check {
        changed: bool,
        format: Option<Format>,
        base: Option<String>,
        config: Option<PathBuf>,
        no_cache: bool,
        verbose: bool,
        fail_on: Option<FailOn>,
        max_warnings: Option<usize>,
    },
    BaselineCreate,
    BaselineList {
        rule: Option<String>,
    },
    BaselinePrune,
    Graph,
    Explore {
        output: PathBuf,
    },
    Doctor,
    ConfigValidate {
        show_overlaps: bool,
        show_coverage: bool,
        show_unassigned: bool,
    },
    Explain(String),
    Resolve {
        importer: PathBuf,
        specifier: String,
        kind: DependencyKind,
        config: Option<PathBuf>,
    },
    Version,
    Help,
}

pub fn run(args: &[String], cwd: &Path) -> CliOutput {
    run_with_cancellation(args, cwd, &CancellationToken::default())
}

pub fn run_with_cancellation(
    args: &[String],
    cwd: &Path,
    cancellation: &CancellationToken,
) -> CliOutput {
    let command = match parse(args) {
        Ok(command) => command,
        Err(error) => return CliOutput::project_error(format!("{error}\n\n{}", usage())),
    };
    match command {
        Command::Init { preset } => commands::init(cwd, preset),
        Command::Scan => commands::scan(cwd, cancellation),
        Command::Discover { json, write, force } => commands::discover(cwd, json, write, force),
        Command::Check {
            changed,
            format,
            base,
            config,
            no_cache,
            verbose,
            fail_on,
            max_warnings,
        } => commands::check(
            cwd,
            commands::CheckOptions {
                changed,
                format,
                base,
                config_path: config,
                no_cache,
                verbose,
                fail_on,
                max_warnings,
                cancellation: cancellation.clone(),
            },
        ),
        Command::BaselineCreate => commands::baseline_create(cwd, cancellation),
        Command::BaselineList { rule } => commands::baseline_list(cwd, rule.as_deref()),
        Command::BaselinePrune => commands::baseline_prune(cwd, cancellation),
        Command::Graph => commands::graph(cwd, cancellation),
        Command::Explore { output } => commands::explore(cwd, output, cancellation),
        Command::Doctor => commands::doctor(cwd, cancellation),
        Command::ConfigValidate { show_overlaps, show_coverage, show_unassigned } => {
            commands::config_validate(cwd, show_overlaps, show_coverage, show_unassigned)
        }
        Command::Explain(rule) => commands::explain(&rule),
        Command::Resolve { importer, specifier, kind, config } => {
            commands::resolve(cwd, importer, specifier, kind, config)
        }
        Command::Version => CliOutput::success(format!("wae {}", env!("CARGO_PKG_VERSION"))),
        Command::Help => CliOutput::success(usage()),
    }
}

fn parse(args: &[String]) -> Result<Command, String> {
    let Some(command) = args.first().map(String::as_str) else { return Ok(Command::Help) };
    match command {
        "init" => parse_init(&args[1..]),
        "scan" if args.len() == 1 => Ok(Command::Scan),
        "discover" => parse_discover(&args[1..]),
        "graph" if args.len() == 1 => Ok(Command::Graph),
        "explore" => parse_explore(&args[1..]),
        "doctor" if args.len() == 1 => Ok(Command::Doctor),
        "config" if args.get(1).map(String::as_str) == Some("validate") => {
            parse_config_validate(&args[2..])
        }
        "baseline" if args.get(1).map(String::as_str) == Some("create") && args.len() == 2 => {
            Ok(Command::BaselineCreate)
        }
        "baseline" if args.get(1).map(String::as_str) == Some("prune") && args.len() == 2 => {
            Ok(Command::BaselinePrune)
        }
        "baseline" if args.get(1).map(String::as_str) == Some("list") => {
            let rule = match &args[2..] {
                [] => None,
                [flag, rule] if flag == "--rule" => Some(rule.clone()),
                _ => return Err("baseline list accepts only `--rule RULE_ID`".into()),
            };
            Ok(Command::BaselineList { rule })
        }
        "explain" if args.len() == 2 => Ok(Command::Explain(args[1].clone())),
        "resolve" => parse_resolve(&args[1..]),
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

fn parse_discover(args: &[String]) -> Result<Command, String> {
    let mut json = false;
    let mut write = false;
    let mut force = false;
    for option in args {
        match option.as_str() {
            "--json" => json = true,
            "--write" => write = true,
            "--force" => force = true,
            value => return Err(format!("unknown discover option `{value}`")),
        }
    }
    if force && !write {
        return Err("discover --force requires --write".into());
    }
    Ok(Command::Discover { json, write, force })
}

fn parse_config_validate(args: &[String]) -> Result<Command, String> {
    let mut show_overlaps = false;
    let mut show_coverage = false;
    let mut show_unassigned = false;
    for option in args {
        match option.as_str() {
            "--show-overlaps" => show_overlaps = true,
            "--show-coverage" => show_coverage = true,
            "--show-unassigned" => show_unassigned = true,
            value => return Err(format!("unknown config validate option `{value}`")),
        }
    }
    Ok(Command::ConfigValidate { show_overlaps, show_coverage, show_unassigned })
}

fn parse_check(args: &[String]) -> Result<Command, String> {
    let mut changed = false;
    let mut format = None;
    let mut base = None;
    let mut config = None;
    let mut no_cache = false;
    let mut verbose = false;
    let mut fail_on = None;
    let mut max_warnings = None;
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
            "--config" => {
                index += 1;
                config = Some(PathBuf::from(args.get(index).ok_or("--config requires a value")?));
            }
            "--no-cache" => no_cache = true,
            "--verbose" | "-v" => verbose = true,
            "--fail-on" => {
                index += 1;
                let value = args.get(index).ok_or("--fail-on requires a value")?;
                fail_on = Some(FailOn::parse(value).ok_or_else(|| {
                    format!("unsupported failure threshold `{value}`; expected error or warning")
                })?);
            }
            "--max-warnings" => {
                index += 1;
                let value = args.get(index).ok_or("--max-warnings requires a value")?;
                max_warnings = Some(value.parse::<usize>().map_err(|_| {
                    format!(
                        "invalid --max-warnings value `{value}`; expected a non-negative integer"
                    )
                })?);
            }
            value => return Err(format!("unknown check option `{value}`")),
        }
        index += 1;
    }
    Ok(Command::Check { changed, format, base, config, no_cache, verbose, fail_on, max_warnings })
}

fn parse_resolve(args: &[String]) -> Result<Command, String> {
    if args.len() < 2 {
        return Err("resolve requires <IMPORTER> <SPECIFIER>".into());
    }
    let importer = PathBuf::from(&args[0]);
    let specifier = args[1].clone();
    let mut kind = DependencyKind::Static;
    let mut config = None;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--kind" => {
                index += 1;
                kind = match args.get(index).map(String::as_str) {
                    Some("static") => DependencyKind::Static,
                    Some("dynamic") => DependencyKind::Dynamic,
                    Some("require") => DependencyKind::Require,
                    Some("type") => DependencyKind::TypeOnly,
                    Some("re-export") => DependencyKind::ReExport,
                    Some(value) => return Err(format!("unsupported dependency kind `{value}`")),
                    None => return Err("--kind requires a value".into()),
                };
            }
            "--config" => {
                index += 1;
                config = Some(PathBuf::from(args.get(index).ok_or("--config requires a value")?));
            }
            value => return Err(format!("unknown resolve option `{value}`")),
        }
        index += 1;
    }
    Ok(Command::Resolve { importer, specifier, kind, config })
}

fn parse_explore(args: &[String]) -> Result<Command, String> {
    match args {
        [] => Ok(Command::Explore { output: PathBuf::from(".wae/explorer.html") }),
        [option, path] if option == "--output" => {
            Ok(Command::Explore { output: PathBuf::from(path) })
        }
        _ => Err("explore accepts only `--output PATH`".into()),
    }
}

fn usage() -> String {
    "Usage: wae <COMMAND>\n\nCommands:\n  init [--preset blank|fsd|next|nx]\n                               Create a safe, explicit wae.yaml\n  discover [--json] [--write] [--force]\n                               Infer an evidence-backed architecture proposal\n  scan                         Analyze and report module/dependency counts\n  check [--changed] [--base REF] [--format human|json|jsonl|sarif]\n        [--config PATH] [--no-cache] [--verbose]\n        [--fail-on error|warning] [--max-warnings N]\n  resolve <IMPORTER> <SPECIFIER> [--kind static|dynamic|require|type|re-export]\n                               Trace every resolver handler and active condition\n  baseline create              Explicitly record current violations\n  config validate [--show-overlaps] [--show-coverage] [--show-unassigned]\n                               Validate config, ownership and coverage\n  graph                        Print the real dependency graph as JSON\n  explore [--output PATH]      Build a self-contained interactive architecture explorer\n  doctor                       Validate project/config/tooling with actionable errors\n  explain <RULE_ID>            Explain an architecture rule\n\nOptions:\n  -V, --version                Print the installed WAE version\n  -h, --help                   Print help\n\nExit codes: 0 passed, 1 violations, 2 config/project error, 3 internal error, 130 cancelled"
    .replace(
        "baseline create              Explicitly record current violations",
        "baseline create|list|prune   Create, inspect, or remove stale baseline entries",
    )
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
    fn warnings_are_visible_but_only_fail_when_configured_or_over_budget() {
        let root = std::env::temp_dir().join(format!("wae-warning-policy-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.ts"), "import './b';").unwrap();
        std::fs::write(root.join("src/b.ts"), "import './a';").unwrap();
        std::fs::write(root.join("wae.yaml"), "version: 1\nrules:\n  ARCH-001: warning\n").unwrap();

        let visible = run(&["check".into(), "--format".into(), "json".into()], &root);
        assert_eq!(visible.exit_code, EXIT_PASSED);
        assert!(visible.stdout.contains("\"warningCount\": 1"));
        assert_eq!(
            run(&["check".into(), "--fail-on".into(), "warning".into()], &root).exit_code,
            EXIT_VIOLATIONS
        );
        assert_eq!(
            run(&["check".into(), "--max-warnings".into(), "0".into()], &root).exit_code,
            EXIT_VIOLATIONS
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancellation_has_the_conventional_signal_exit_code() {
        let cancellation = CancellationToken::default();
        cancellation.cancel();
        let output = run_with_cancellation(&["check".into()], &fixture("basic"), &cancellation);
        assert_eq!(output.exit_code, EXIT_CANCELLED);
        assert_eq!(output.stderr, "analysis cancelled");
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
    fn check_supports_custom_config_no_cache_and_verbose_timing() {
        let root = std::env::temp_dir().join(format!("wae-observability-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.ts"), "export const value = 1;").unwrap();
        std::fs::write(
            root.join("architecture.yml"),
            "version: 1\noutput:\n  format: json\ncache:\n  enabled: true\n",
        )
        .unwrap();
        let output = run(
            &[
                "check".into(),
                "--config".into(),
                "architecture.yml".into(),
                "--no-cache".into(),
                "--verbose".into(),
            ],
            &root,
        );
        assert_eq!(output.exit_code, EXIT_PASSED);
        assert!(output.stdout.contains("\"schemaVersion\""));
        assert!(output.stderr.contains("WAE timing:"));
        assert!(output.stderr.contains("enabled=false"));
        assert!(!root.join(".wae/cache").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolve_command_explains_alias_handler_conditions_and_outcome() {
        let output = run(
            &[
                "resolve".into(),
                "src/app/page.tsx".into(),
                "@/features/cart".into(),
                "--kind".into(),
                "static".into(),
            ],
            &fixture("consumer-next"),
        );
        assert_eq!(output.exit_code, EXIT_PASSED, "{}", output.stderr);
        let trace: serde_json::Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(trace["resolutionKind"], "Import");
        assert!(
            trace["activeConditions"].as_array().unwrap().iter().any(|value| value == "import")
        );
        assert!(trace["attempts"].as_array().unwrap().iter().any(|attempt| {
            attempt["handler"] == "tsconfig-alias"
                && attempt["outcome"].as_str().is_some_and(|value| value.starts_with("module:"))
        }));
        assert_eq!(trace["outcome"], "module:src/features/cart/index.ts");
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
            Command::ConfigValidate {
                show_overlaps: true,
                show_coverage: false,
                show_unassigned: false,
            }
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

    #[test]
    fn config_coverage_reports_unassigned_modules_and_enforces_minimum() {
        let root = std::env::temp_dir().join(format!("wae-coverage-cli-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src/app")).unwrap();
        std::fs::create_dir_all(root.join("scripts")).unwrap();
        std::fs::write(root.join("src/app/page.ts"), "export const page = true;").unwrap();
        std::fs::write(root.join("src/orphan.ts"), "export const orphan = true;").unwrap();
        std::fs::write(root.join("scripts/generate.ts"), "export const generated = true;").unwrap();
        std::fs::write(
            root.join("wae.yaml"),
            "version: 1\narchitecture:\n  coverage:\n    minimum: 90\n    allow_unassigned: ['scripts/**']\n  layers:\n    app:\n      patterns: ['src/app/**']\n",
        )
        .unwrap();

        let output = run(
            &[
                "config".into(),
                "validate".into(),
                "--show-coverage".into(),
                "--show-unassigned".into(),
            ],
            &root,
        );
        assert_eq!(output.exit_code, EXIT_PROJECT);
        assert!(output.stderr.contains("Coverage: 50% (1 assigned, 1 unassigned, 1 exempt)"));
        assert!(output.stderr.contains("src/orphan.ts"));
        assert!(!output.stderr.contains("scripts/generate.ts\n"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn doctor_treats_missing_git_as_changed_mode_advice() {
        let root = std::env::temp_dir().join(format!("wae-doctor-no-git-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.ts"), "export const value = true;").unwrap();
        std::fs::write(root.join("wae.yaml"), "version: 1\n").unwrap();
        let output = run(&["doctor".into()], &root);
        assert_eq!(output.exit_code, EXIT_PASSED, "{}", output.stderr);
        assert!(output.stdout.contains("only `wae check --changed` is disabled"));
        assert!(output.stdout.contains("no layers configured"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
