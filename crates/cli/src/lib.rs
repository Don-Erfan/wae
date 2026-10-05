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
    BaselineCreate {
        config: Option<PathBuf>,
    },
    BaselineList {
        rule: Option<String>,
        config: Option<PathBuf>,
    },
    BaselinePrune {
        config: Option<PathBuf>,
    },
    SuppressionsList,
    SuppressionsValidate,
    SuppressionsPrune,
    Graph,
    GraphModule {
        module: String,
        json: bool,
    },
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
    Rules,
    Resolve {
        importer: PathBuf,
        specifier: String,
        kind: DependencyKind,
        config: Option<PathBuf>,
    },
    Version,
    Help,
    HelpFor(&'static str),
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
        Err(error) => {
            let hint = args.first().and_then(|name| command_help(name)).map_or_else(
                || "Run `wae help` for the list of commands.".to_string(),
                str::to_string,
            );
            return CliOutput::project_error(format!("error: {error}\n\n{hint}"));
        }
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
        Command::BaselineCreate { config } => {
            commands::baseline_create(cwd, config.as_deref(), cancellation)
        }
        Command::BaselineList { rule, config } => {
            commands::baseline_list(cwd, rule.as_deref(), config.as_deref())
        }
        Command::BaselinePrune { config } => {
            commands::baseline_prune(cwd, config.as_deref(), cancellation)
        }
        Command::SuppressionsList => commands::suppressions_list(cwd),
        Command::SuppressionsValidate => commands::suppressions_validate(cwd, cancellation),
        Command::SuppressionsPrune => commands::suppressions_prune(cwd),
        Command::Graph => commands::graph(cwd, cancellation),
        Command::GraphModule { module, json } => {
            commands::graph_module(cwd, &module, json, cancellation)
        }
        Command::Explore { output } => commands::explore(cwd, output, cancellation),
        Command::Doctor => commands::doctor(cwd, cancellation),
        Command::ConfigValidate { show_overlaps, show_coverage, show_unassigned } => {
            commands::config_validate(cwd, show_overlaps, show_coverage, show_unassigned)
        }
        Command::Explain(rule) => commands::explain(&rule),
        Command::Rules => commands::rules(),
        Command::Resolve { importer, specifier, kind, config } => {
            commands::resolve(cwd, importer, specifier, kind, config)
        }
        Command::Version => CliOutput::success(format!("wae {}", env!("CARGO_PKG_VERSION"))),
        Command::Help => CliOutput::success(usage()),
        Command::HelpFor(name) => {
            CliOutput::success(command_help(name).map_or_else(usage, str::to_string))
        }
    }
}

fn parse(args: &[String]) -> Result<Command, String> {
    let Some(command) = args.first().map(String::as_str) else { return Ok(Command::Help) };
    if args.len() > 1 && args[1..].iter().any(|arg| arg == "--help" || arg == "-h") {
        if let Some((name, _)) = COMMAND_HELP.iter().find(|(name, _)| *name == command) {
            return Ok(Command::HelpFor(name));
        }
    }
    match command {
        "init" => parse_init(&args[1..]),
        "scan" if args.len() == 1 => Ok(Command::Scan),
        "discover" => parse_discover(&args[1..]),
        "graph" => parse_graph(&args[1..]),
        "explore" => parse_explore(&args[1..]),
        "doctor" if args.len() == 1 => Ok(Command::Doctor),
        "config" if args.get(1).map(String::as_str) == Some("validate") => {
            parse_config_validate(&args[2..])
        }
        "baseline" => parse_baseline(&args[1..]),
        "suppressions" if args.get(1).map(String::as_str) == Some("list") && args.len() == 2 => {
            Ok(Command::SuppressionsList)
        }
        "suppressions"
            if args.get(1).map(String::as_str) == Some("validate") && args.len() == 2 =>
        {
            Ok(Command::SuppressionsValidate)
        }
        "suppressions" if args.get(1).map(String::as_str) == Some("prune") && args.len() == 2 => {
            Ok(Command::SuppressionsPrune)
        }
        "explain" if args.len() == 2 && args[1] == "--list" => Ok(Command::Rules),
        "explain" if args.len() == 2 => Ok(Command::Explain(args[1].to_ascii_uppercase())),
        "explain" if args.len() == 1 => Err("explain requires a rule id such as ARCH-004".into()),
        "rules" if args.len() == 1 => Ok(Command::Rules),
        "resolve" => parse_resolve(&args[1..]),
        "check" => parse_check(&args[1..]),
        "--version" | "-V" if args.len() == 1 => Ok(Command::Version),
        "help" | "--help" | "-h" => match args.get(1).map(String::as_str) {
            Some(name) => COMMAND_HELP
                .iter()
                .find(|(command, _)| *command == name)
                .map(|(command, _)| Command::HelpFor(command))
                .ok_or_else(|| format!("unknown command `{name}`")),
            None => Ok(Command::Help),
        },
        _ => Err(format!("Invalid command or arguments: {}", args.join(" "))),
    }
}

fn parse_baseline(args: &[String]) -> Result<Command, String> {
    // Bare `wae baseline [--config PATH]` is an explicit request to record current violations.
    let (action, options) = match args.first().map(String::as_str) {
        Some(action) if !action.starts_with('-') => (action, &args[1..]),
        _ => ("create", args),
    };
    let mut rule = None;
    let mut config = None;
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--rule" if action == "list" => {
                index += 1;
                rule = Some(options.get(index).ok_or("--rule requires a value")?.clone());
            }
            "--config" => {
                index += 1;
                config =
                    Some(PathBuf::from(options.get(index).ok_or("--config requires a value")?));
            }
            value => return Err(format!("unsupported baseline option `{value}`")),
        }
        index += 1;
    }
    match action {
        "create" => Ok(Command::BaselineCreate { config }),
        "list" => Ok(Command::BaselineList { rule, config }),
        "prune" => Ok(Command::BaselinePrune { config }),
        value => Err(format!("unknown baseline action `{value}`")),
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

fn parse_graph(args: &[String]) -> Result<Command, String> {
    let mut module = None;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--module" => {
                index += 1;
                module = Some(args.get(index).ok_or("--module requires a path")?.clone());
            }
            "--format" => {
                index += 1;
                json = match args.get(index).map(String::as_str) {
                    Some("json") => true,
                    Some("human") => false,
                    Some(value) => return Err(format!("unsupported graph format `{value}`")),
                    None => return Err("--format requires a value".into()),
                };
            }
            value => return Err(format!("unknown graph option `{value}`")),
        }
        index += 1;
    }
    match module {
        Some(module) => Ok(Command::GraphModule { module, json }),
        None if !json && args.is_empty() => Ok(Command::Graph),
        None if json => Ok(Command::Graph),
        None => Err("graph accepts `--module PATH` and `--format human|json`".into()),
    }
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
    let mut text = String::from(
        "Web Architecture Engine: deterministic architecture checks for JavaScript and TypeScript\n\nUsage: wae <COMMAND> [OPTIONS]\n\nCommands:\n",
    );
    for (name, help) in COMMAND_HELP {
        let summary = help.lines().next().unwrap_or_default();
        text.push_str(&format!("  {name:<13}{summary}\n"));
    }
    text.push_str(
        "\nOptions:\n  -V, --version  Print the installed WAE version\n  -h, --help     Print help (use `wae <COMMAND> --help` for command details)\n\nExit codes: 0 passed, 1 violations, 2 config/project error, 3 internal error, 130 cancelled",
    );
    text
}

fn command_help(name: &str) -> Option<&'static str> {
    COMMAND_HELP.iter().find(|(command, _)| *command == name).map(|(_, help)| *help)
}

/// First line: one-line summary for `wae help`. The full text is printed by `wae <cmd> --help`.
const COMMAND_HELP: &[(&str, &str)] = &[
    (
        "init",
        "Create a safe, explicit wae.yaml\n\nUsage: wae init [--preset blank|fsd|next|nx]\n\nThe default `blank` preset assigns no layers, so nothing is guessed. `fsd`, `next` and `nx`\nwrite repository-anchored layer patterns. Use `wae discover` to infer a proposal instead.",
    ),
    (
        "discover",
        "Infer an evidence-backed architecture proposal\n\nUsage: wae discover [--json] [--write [--force]]\n\nOptions:\n  --json     Print the proposal as JSON\n  --write    Write the proposal to wae.yaml (refuses to overwrite)\n  --force    Overwrite an existing wae.yaml (requires --write)\n\nThe proposal lists evidence, confidence and unknown decisions to review before adoption.",
    ),
    ("scan", "Analyze the project and report module and dependency counts\n\nUsage: wae scan"),
    (
        "check",
        "Run every enabled rule and fail on violations\n\nUsage: wae check [OPTIONS]\n\nOptions:\n  --format human|json|jsonl|sarif  Output format (default: output.format or human)\n  --changed                       Fail only on violations new since the committed baseline,\n                                  limited to changed files and their importers\n  --base REF                      Git base for --changed (default: WAE_BASE_REF or merge base)\n  --config PATH                   Use another configuration file\n  --fail-on error|warning         Lowest severity that fails the check\n  --max-warnings N                Fail when more than N warnings are reported\n  --no-cache                      Neither read nor write the incremental cache\n  -v, --verbose                   Print timing and cache details to stderr\n\nExit codes: 0 passed, 1 violations, 2 config/project error, 3 internal error, 130 cancelled",
    ),
    (
        "baseline",
        "Record, review or prune accepted existing violations\n\nUsage: wae baseline [create|list|prune] [--rule RULE_ID] [--config PATH]\n\n  create (default)  Record current fail-level violations to the baseline file\n  list              Show baseline entries (optionally filtered by --rule)\n  prune             Remove expired or already-fixed entries\n\nCommit the baseline, then use `wae check --changed` to fail only on new violations.",
    ),
    (
        "explain",
        "Explain a rule: why it exists, bad and good examples, configuration\n\nUsage: wae explain <RULE_ID>\n       wae explain --list\n\nExample: wae explain ARCH-004",
    ),
    ("rules", "List every rule with its title and category\n\nUsage: wae rules"),
    (
        "graph",
        "Print the dependency graph, or inspect one module\n\nUsage: wae graph [--format json]\n       wae graph --module PATH [--format human|json]\n\nWith --module, prints the module's package, layer, runtime (and why it has that runtime),\nits dependencies, its dependents and every diagnostic that involves it.",
    ),
    (
        "resolve",
        "Trace how one import specifier resolves\n\nUsage: wae resolve <IMPORTER> <SPECIFIER> [--kind static|dynamic|require|type|re-export] [--config PATH]\n\nPrints every resolver handler attempt, active package conditions and the final outcome.",
    ),
    (
        "explore",
        "Write a self-contained interactive architecture explorer (HTML)\n\nUsage: wae explore [--output PATH]   (default: .wae/explorer.html)",
    ),
    (
        "config",
        "Validate configuration, layer ownership and coverage\n\nUsage: wae config validate [--show-overlaps] [--show-coverage] [--show-unassigned]",
    ),
    (
        "suppressions",
        "List, audit or prune config-level suppressions\n\nUsage: wae suppressions list|validate|prune",
    ),
    (
        "doctor",
        "Check project, configuration and tooling with actionable advice\n\nUsage: wae doctor",
    ),
];

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
    fn every_command_has_help_and_help_flags_never_run_the_command() {
        for (name, _) in COMMAND_HELP {
            for args in
                [vec![name.to_string(), "--help".into()], vec!["help".into(), name.to_string()]]
            {
                let output = run(&args, Path::new("/nonexistent-wae-root"));
                assert_eq!(output.exit_code, EXIT_PASSED, "{args:?}: {}", output.stderr);
                assert!(output.stdout.contains("Usage: wae"), "{args:?}");
            }
        }
        let nested = run(&["config".into(), "validate".into(), "-h".into()], Path::new("."));
        assert!(nested.stdout.contains("--show-overlaps"));
        assert!(usage().contains("graph"));
    }

    #[test]
    fn invalid_arguments_print_the_command_help_instead_of_the_full_usage() {
        let output = run(&["check".into(), "--frmat".into(), "json".into()], Path::new("."));
        assert_eq!(output.exit_code, EXIT_PROJECT);
        assert!(output.stderr.starts_with("error: unknown check option `--frmat`"));
        assert!(output.stderr.contains("Usage: wae check"));
        assert!(!output.stderr.contains("Usage: wae init"));
        let unknown = run(&["chek".into()], Path::new("."));
        assert!(unknown.stderr.contains("Run `wae help`"));
    }

    #[test]
    fn explain_prints_full_documentation_and_rules_lists_the_registry() {
        let output = run(&["explain".into(), "arch-004".into()], Path::new("."));
        assert_eq!(output.exit_code, EXIT_PASSED);
        for section in ["ARCH-004 — Feature boundary", "Why:", "Bad:", "Good:", "Configuration:"]
        {
            assert!(output.stdout.contains(section), "missing {section}");
        }
        assert!(!output.stdout.lines().any(|line| line.ends_with(' ')));
        let rules = run(&["rules".into()], Path::new("."));
        assert_eq!(rules.exit_code, EXIT_PASSED);
        for rule in wae_core::rule_registry::RULES {
            assert!(rules.stdout.contains(rule.id));
        }
        assert_eq!(run(&["explain".into(), "--list".into()], Path::new(".")).stdout, rules.stdout);
        let unknown = run(&["explain".into(), "ARCH-999".into()], Path::new("."));
        assert_eq!(unknown.exit_code, EXIT_PROJECT);
        assert!(unknown.stderr.contains("wae rules"));
    }

    #[test]
    fn graph_module_explains_edges_runtime_and_diagnostics() {
        let human = run(
            &["graph".into(), "--module".into(), "./src/app/server.ts".into()],
            &fixture("runtime"),
        );
        assert_eq!(human.exit_code, EXIT_PASSED, "{}", human.stderr);
        assert!(human.stdout.starts_with("src/app/server.ts\n"));
        assert!(human.stdout.contains("Dependents ("));
        assert!(human.stdout.contains("RUNTIME-001"));
        let json = run(
            &[
                "graph".into(),
                "--module".into(),
                "src/app/server.ts".into(),
                "--format".into(),
                "json".into(),
            ],
            &fixture("runtime"),
        );
        let value: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
        assert_eq!(value["runtime"]["runtime"], "server");
        assert!(
            value["dependents"]
                .as_array()
                .unwrap()
                .iter()
                .any(|edge| edge["module"] == "src/app/client.tsx")
        );
        let missing =
            run(&["graph".into(), "--module".into(), "src/nope.ts".into()], &fixture("runtime"));
        assert_eq!(missing.exit_code, EXIT_PROJECT);
        assert!(parse(&["graph".into()]).is_ok_and(|command| command == Command::Graph));
    }

    #[test]
    fn bare_baseline_is_an_explicit_create_request() {
        assert_eq!(parse(&["baseline".into()]).unwrap(), Command::BaselineCreate { config: None });
        assert_eq!(
            parse(&["baseline".into(), "--config".into(), "custom.yaml".into()]).unwrap(),
            Command::BaselineCreate { config: Some(PathBuf::from("custom.yaml")) }
        );
        assert!(
            parse(&["baseline".into(), "list".into(), "--rule".into(), "ARCH-001".into()]).is_ok()
        );
        assert!(parse(&["baseline".into(), "--rule".into(), "ARCH-001".into()]).is_err());
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
    fn suppression_registry_commands_list_validate_and_prune_atomically() {
        let root =
            std::env::temp_dir().join(format!("wae-suppressions-cli-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/app.ts"), "export const value = true;").unwrap();
        std::fs::write(
            root.join("wae.yaml"),
            "version: 1\nsuppressions:\n  paths:\n    - pattern: 'src/unused/**'\n      rules: [ARCH-003]\n      reason: migration\n      owner: platform\n  fingerprints:\n    - fingerprint: expired\n      reason: temporary\n      expires_at: '2020-01-01'\n",
        )
        .unwrap();
        let listed = run(&["suppressions".into(), "list".into()], &root);
        assert_eq!(listed.exit_code, EXIT_PASSED);
        assert!(listed.stdout.contains("platform"));
        let validated = run(&["suppressions".into(), "validate".into()], &root);
        assert_eq!(validated.exit_code, EXIT_VIOLATIONS);
        assert!(validated.stdout.contains("Expired config fingerprint suppression"));
        let pruned = run(&["suppressions".into(), "prune".into()], &root);
        assert_eq!(pruned.exit_code, EXIT_PASSED);
        assert_eq!(wae_config::Config::load(&root).unwrap().suppressions.fingerprints.len(), 0);
        assert!(
            !std::fs::read_dir(&root).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("tmp-"))
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn suppression_prune_preserves_leaf_composition_and_reports_parent_ownership() {
        let root = std::env::temp_dir()
            .join(format!("wae-suppressions-inheritance-cli-{}", std::process::id()));
        std::fs::create_dir_all(root.join("config")).unwrap();
        let parent = "version: 1\nsuppressions:\n  paths:\n    - pattern: 'src/parent/**'\n      rules: [ARCH-003]\n      reason: parent migration\n      expires_at: '2020-01-01'\n";
        let leaf = "# local architecture policy\nextends: config/base.yaml\nversion: 1\nsuppressions:\n  fingerprints: # local exceptions\n    - fingerprint: expired-local\n      reason: local migration\n      expires_at: '2020-01-01'\noutput:\n  format: human\n";
        std::fs::write(root.join("config/base.yaml"), parent).unwrap();
        std::fs::write(root.join("wae.yaml"), leaf).unwrap();

        let listed = run(&["suppressions".into(), "list".into()], &root);
        assert_eq!(listed.exit_code, EXIT_PASSED);
        assert!(listed.stdout.contains("\"inherited\": true"));
        assert!(listed.stdout.contains("config/base.yaml"));
        let pruned = run(&["suppressions".into(), "prune".into()], &root);
        assert_eq!(pruned.exit_code, EXIT_PASSED, "{}", pruned.stderr);
        assert!(pruned.stdout.contains("1 expired inherited entries remain"));
        let updated = std::fs::read_to_string(root.join("wae.yaml")).unwrap();
        assert!(updated.contains("# local architecture policy"));
        assert!(updated.contains("extends: config/base.yaml"));
        assert!(updated.contains("fingerprints: [] # local exceptions"));
        assert_eq!(std::fs::read_to_string(root.join("config/base.yaml")).unwrap(), parent);
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
    fn baseline_commands_accept_the_same_custom_config_selector_as_check() {
        assert!(matches!(
            parse(&[
                "baseline".into(),
                "list".into(),
                "--rule".into(),
                "ARCH-003".into(),
                "--config".into(),
                "custom.yaml".into(),
            ])
            .unwrap(),
            Command::BaselineList { rule: Some(rule), config: Some(config) }
                if rule == "ARCH-003" && config.as_os_str() == "custom.yaml"
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
    fn changed_check_uses_baseline_from_the_selected_custom_config() {
        let root = std::env::temp_dir().join(format!("wae-custom-baseline-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.ts"), "export const value = 1;").unwrap();
        std::fs::write(
            root.join("custom.yaml"),
            "version: 1\nbaseline:\n  file: baseline-custom.json\n",
        )
        .unwrap();
        std::fs::write(
            root.join("baseline-custom.json"),
            r#"{"schemaVersion":3,"createdAtUnix":0,"entries":[]}"#,
        )
        .unwrap();
        let output = run(
            &["check".into(), "--changed".into(), "--config".into(), "custom.yaml".into()],
            &root,
        );
        assert!(!output.stderr.contains(".wae/baseline.json"), "{}", output.stderr);
        assert!(!output.stderr.contains("baseline is missing"), "{}", output.stderr);
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
