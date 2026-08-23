use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::{collections::BTreeSet, collections::HashSet};

const EXIT_PASSED: i32 = 0;
const EXIT_VIOLATIONS: i32 = 1;
const EXIT_INTERNAL_OR_CONFIG: i32 = 2;
const DEFAULT_CONFIG_FILE: &str = "wae.yaml";
const DIAGNOSTICS_FILE: &str = "wae.violations";
const BASELINE_FILE: &str = ".wae-baseline";

#[derive(Clone, Debug, PartialEq, Eq)]
enum Command {
    Init,
    Scan,
    Check { changed: bool },
    Explain { rule_id: String },
    Mcp { tool: String, args: Vec<String> },
    Help,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CheckDiagnostic {
    rule_id: String,
    severity: Severity,
    message: String,
    path: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CliOutput {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

impl CliOutput {
    fn success(stdout: String) -> Self {
        Self {
            exit_code: EXIT_PASSED,
            stdout,
            stderr: String::new(),
        }
    }

    fn violations(stdout: String) -> Self {
        Self {
            exit_code: EXIT_VIOLATIONS,
            stdout,
            stderr: String::new(),
        }
    }

    fn failure(stderr: String) -> Self {
        Self {
            exit_code: EXIT_INTERNAL_OR_CONFIG,
            stdout: String::new(),
            stderr,
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let output = run_cli(&args, &cwd);

    if !output.stdout.is_empty() {
        println!("{}", output.stdout);
    }

    if !output.stderr.is_empty() {
        eprintln!("{}", output.stderr);
    }

    std::process::exit(output.exit_code);
}

fn run_cli(args: &[String], cwd: &Path) -> CliOutput {
    let command = match parse_command(args) {
        Ok(command) => command,
        Err(error) => return CliOutput::failure(format!("{error}\n\n{}", usage())),
    };

    match command {
        Command::Init => run_init(cwd),
        Command::Scan => run_scan(cwd),
        Command::Check { changed } => run_check(cwd, changed),
        Command::Explain { rule_id } => run_explain(&rule_id),
        Command::Mcp { tool, args } => run_mcp(cwd, &tool, &args),
        Command::Help => CliOutput::success(usage()),
    }
}

fn parse_command(args: &[String]) -> Result<Command, String> {
    if args.is_empty() {
        return Ok(Command::Help);
    }

    match args[0].as_str() {
        "init" => Ok(Command::Init),
        "scan" => Ok(Command::Scan),
        "check" => {
            if args.len() == 1 {
                return Ok(Command::Check { changed: false });
            }

            if args.len() == 2 && args[1] == "--changed" {
                return Ok(Command::Check { changed: true });
            }

            Err(String::from(
                "Invalid check arguments. Usage: wae check [--changed]",
            ))
        }
        "explain" => {
            if args.len() < 2 {
                return Err(String::from("Missing RULE_ID. Usage: wae explain <RULE_ID>"));
            }
            Ok(Command::Explain {
                rule_id: args[1].clone(),
            })
        }
        "mcp" => {
            if args.len() < 2 {
                return Err(String::from(
                    "Missing MCP tool name. Usage: wae mcp <TOOL> [ARGS]",
                ));
            }

            Ok(Command::Mcp {
                tool: args[1].clone(),
                args: args[2..].to_vec(),
            })
        }
        "help" | "--help" | "-h" => Ok(Command::Help),
        other => Err(format!("Unknown command: {other}")),
    }
}

fn run_init(cwd: &Path) -> CliOutput {
    let config_path = cwd.join(DEFAULT_CONFIG_FILE);
    let discovery = discover_architecture(cwd);

    let mut output = String::new();
    output.push_str("Analyzing project...\n\n");
    output.push_str("Detected:\n\n");
    output.push_str(&discovery.detected.join("\n"));
    output.push_str("\n\nProject structure:\n\n");
    for folder in &discovery.structure {
        output.push_str(folder);
        output.push_str("/\n");
    }
    output.push_str("\nPossible architecture detected.\n\n");
    output.push_str("Recommended dependency direction:\n\n");
    output.push_str("features\n  ↓\nentities\n  ↓\nshared\n");

    if config_path.exists() {
        output.push_str(&format!(
            "\nConfiguration already exists: {}",
            config_path.display()
        ));
        return CliOutput::success(output);
    }

    match fs::write(&config_path, discovery.generated_config) {
        Ok(()) => {
            output.push_str(&format!("\nCreated {}", config_path.display()));
            CliOutput::success(output)
        }
        Err(error) => CliOutput::failure(format!("Failed to create config: {error}")),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ArchitectureDiscovery {
    detected: Vec<String>,
    structure: Vec<String>,
    generated_config: String,
}

fn discover_architecture(cwd: &Path) -> ArchitectureDiscovery {
    let has_next = cwd.join("next.config.js").exists() || cwd.join("next.config.mjs").exists();
    let has_ts = cwd.join("tsconfig.json").exists();
    let has_app_router = cwd.join("src").join("app").exists() || cwd.join("app").exists();

    let mut detected = Vec::new();
    if has_next {
        detected.push(String::from("Next.js"));
    }
    if has_ts {
        detected.push(String::from("TypeScript"));
    }
    if has_app_router {
        detected.push(String::from("App Router"));
    }
    if detected.is_empty() {
        detected.push(String::from("Generic JavaScript/TypeScript Project"));
    }

    let structure = ["app", "features", "entities", "shared"]
        .iter()
        .filter(|folder| cwd.join("src").join(folder).exists() || cwd.join(folder).exists())
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();

    let generated_config = default_config_yaml();

    ArchitectureDiscovery {
        detected,
        structure,
        generated_config: generated_config.to_string(),
    }
}

fn run_scan(cwd: &Path) -> CliOutput {
    match count_modules(cwd) {
        Ok(modules) => CliOutput::success(format!(
            "Analyzing {} modules...\n\nScan complete.",
            format_count(modules)
        )),
        Err(error) => CliOutput::failure(format!("Scan failed: {error}")),
    }
}

fn run_check(cwd: &Path, changed: bool) -> CliOutput {
    if let Err(error) = validate_config(cwd) {
        return CliOutput::failure(format!("Configuration error: {error}"));
    }

    let modules = match count_modules(cwd) {
        Ok(value) => value,
        Err(error) => return CliOutput::failure(format!("Unable to scan modules: {error}")),
    };

    let diagnostics = match load_diagnostics(cwd) {
        Ok(value) => value,
        Err(error) => return CliOutput::failure(format!("Failed to load diagnostics: {error}")),
    };

    if changed {
        return run_check_changed(cwd, modules, &diagnostics);
    }

    let report = render_check_report(modules, &diagnostics);
    let has_violations = diagnostics
        .iter()
        .any(|diagnostic| matches!(diagnostic.severity, Severity::Error | Severity::Warning));

    if has_violations {
        CliOutput::violations(report)
    } else {
        CliOutput::success(report)
    }
}

fn run_check_changed(cwd: &Path, modules: usize, diagnostics: &[CheckDiagnostic]) -> CliOutput {
    let current_signatures = collect_signatures(diagnostics);
    let baseline_path = cwd.join(BASELINE_FILE);

    if !baseline_path.exists() {
        if let Err(error) = save_baseline(&baseline_path, &current_signatures) {
            return CliOutput::failure(format!("Failed to write baseline: {error}"));
        }

        let report = render_changed_report(modules, diagnostics.len(), 0, true);
        return CliOutput::success(report);
    }

    let baseline_signatures = match load_baseline(&baseline_path) {
        Ok(value) => value,
        Err(error) => return CliOutput::failure(format!("Failed to read baseline: {error}")),
    };

    let existing_count = current_signatures.intersection(&baseline_signatures).count();
    let new_count = current_signatures.difference(&baseline_signatures).count();
    let passed = new_count == 0;

    let report = render_changed_report(modules, existing_count, new_count, passed);

    if passed {
        CliOutput::success(report)
    } else {
        CliOutput::violations(report)
    }
}

fn collect_signatures(diagnostics: &[CheckDiagnostic]) -> HashSet<String> {
    diagnostics.iter().map(diagnostic_signature).collect()
}

fn diagnostic_signature(diagnostic: &CheckDiagnostic) -> String {
    format!(
        "{}|{}|{}|{}",
        diagnostic.rule_id,
        severity_as_str(&diagnostic.severity),
        diagnostic.message,
        diagnostic.path.join(" > ")
    )
}

fn severity_as_str(severity: &Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

fn save_baseline(path: &Path, signatures: &HashSet<String>) -> io::Result<()> {
    let lines: BTreeSet<&String> = signatures.iter().collect();
    let content = lines.into_iter().cloned().collect::<Vec<_>>().join("\n");
    fs::write(path, content)
}

fn load_baseline(path: &Path) -> io::Result<HashSet<String>> {
    let raw = fs::read_to_string(path)?;
    Ok(raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn run_explain(rule_id: &str) -> CliOutput {
    let Some((title, details)) = rule_explanation(rule_id) else {
        return CliOutput::failure(format!(
            "Unknown rule id: {rule_id}. Try: ARCH-001, ARCH-002, ARCH-003, ARCH-004, ARCH-005"
        ));
    };

    let message = format!("{rule_id}\n{title}\n\n{details}");

    CliOutput::success(message)
}

fn rule_explanation(rule_id: &str) -> Option<(&'static str, &'static str)> {
    match rule_id {
        "ARCH-001" => Some((
            "Circular dependency",
            "Detects dependency cycles across modules/layers.",
        )),
        "ARCH-002" => Some((
            "Forbidden dependency",
            "Blocks imports that are explicitly forbidden by policy.",
        )),
        "ARCH-003" => Some((
            "Layer violation",
            "Ensures each layer imports only allowed target layers.",
        )),
        "ARCH-004" => Some((
            "Feature boundary violation",
            "A feature cannot import another feature's internal modules.\nUse the feature public API instead.",
        )),
        "ARCH-005" => Some((
            "Private module import",
            "Consumers must use public entrypoints instead of private/internal paths.",
        )),
        _ => None,
    }
}

fn run_mcp(cwd: &Path, tool: &str, args: &[String]) -> CliOutput {
    match tool {
        "architecture_check" => run_mcp_architecture_check(cwd, args),
        "architecture_explain" => run_mcp_architecture_explain(args),
        "architecture_graph" => run_mcp_architecture_graph(cwd),
        "architecture_allowed_dependencies" => run_mcp_architecture_allowed_dependencies(args),
        "architecture_fix" => run_mcp_architecture_fix(args),
        _ => CliOutput::failure(format!(
            "Unknown MCP tool: {tool}. Try: architecture_check, architecture_explain, architecture_graph, architecture_allowed_dependencies, architecture_fix"
        )),
    }
}

fn run_mcp_architecture_check(cwd: &Path, args: &[String]) -> CliOutput {
    let changed = if args.is_empty() {
        false
    } else if args.len() == 1 && args[0] == "--changed" {
        true
    } else {
        return CliOutput::failure(String::from(
            "Invalid arguments for architecture_check. Usage: wae mcp architecture_check [--changed]",
        ));
    };

    if let Err(error) = validate_config(cwd) {
        return CliOutput::failure(format!(
            "{{\"tool\":\"architecture_check\",\"ok\":false,\"error\":{}}}",
            json_string(&format!("Configuration error: {error}"))
        ));
    }

    let modules = match count_modules(cwd) {
        Ok(value) => value,
        Err(error) => {
            return CliOutput::failure(format!(
                "{{\"tool\":\"architecture_check\",\"ok\":false,\"error\":{}}}",
                json_string(&format!("Unable to scan modules: {error}"))
            ));
        }
    };

    let diagnostics = match load_diagnostics(cwd) {
        Ok(value) => value,
        Err(error) => {
            return CliOutput::failure(format!(
                "{{\"tool\":\"architecture_check\",\"ok\":false,\"error\":{}}}",
                json_string(&format!("Failed to load diagnostics: {error}"))
            ));
        }
    };

    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .count();
    let warnings = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Warning)
        .count();
    let info = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Info)
        .count();

    let mut baseline_created = false;
    let mut existing_violations = 0usize;
    let mut new_violations = 0usize;

    if changed {
        let current_signatures = collect_signatures(&diagnostics);
        let baseline_path = cwd.join(BASELINE_FILE);

        if !baseline_path.exists() {
            if let Err(error) = save_baseline(&baseline_path, &current_signatures) {
                return CliOutput::failure(format!(
                    "{{\"tool\":\"architecture_check\",\"ok\":false,\"error\":{}}}",
                    json_string(&format!("Failed to write baseline: {error}"))
                ));
            }

            baseline_created = true;
            existing_violations = diagnostics.len();
            new_violations = 0;
        } else {
            let baseline_signatures = match load_baseline(&baseline_path) {
                Ok(value) => value,
                Err(error) => {
                    return CliOutput::failure(format!(
                        "{{\"tool\":\"architecture_check\",\"ok\":false,\"error\":{}}}",
                        json_string(&format!("Failed to read baseline: {error}"))
                    ));
                }
            };

            existing_violations = current_signatures.intersection(&baseline_signatures).count();
            new_violations = current_signatures.difference(&baseline_signatures).count();
        }
    }

    let diagnostics_json = diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "{{\"rule_id\":{},\"severity\":{},\"message\":{},\"path\":[{}]}}",
                json_string(&diagnostic.rule_id),
                json_string(severity_as_str(&diagnostic.severity)),
                json_string(&diagnostic.message),
                diagnostic
                    .path
                    .iter()
                    .map(|entry| json_string(entry))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        })
        .collect::<Vec<_>>()
        .join(",");

    let passed = if changed {
        new_violations == 0
    } else {
        errors == 0 && warnings == 0
    };

    let output = format!(
        "{{\"tool\":\"architecture_check\",\"ok\":true,\"changed\":{},\"passed\":{},\"modules\":{},\"summary\":{{\"errors\":{},\"warnings\":{},\"info\":{},\"total\":{}}},\"ratchet\":{{\"baseline_created\":{},\"existing_violations\":{},\"new_violations\":{}}},\"diagnostics\":[{}]}}",
        changed,
        passed,
        modules,
        errors,
        warnings,
        info,
        diagnostics.len(),
        baseline_created,
        existing_violations,
        new_violations,
        diagnostics_json
    );

    CliOutput::success(output)
}

fn run_mcp_architecture_explain(args: &[String]) -> CliOutput {
    if args.len() != 1 {
        return CliOutput::failure(String::from(
            "Invalid arguments for architecture_explain. Usage: wae mcp architecture_explain <RULE_ID>",
        ));
    }

    let rule_id = args[0].as_str();
    let Some((title, details)) = rule_explanation(rule_id) else {
        return CliOutput::failure(format!(
            "{{\"tool\":\"architecture_explain\",\"ok\":false,\"error\":{}}}",
            json_string(&format!("Unknown rule id: {rule_id}"))
        ));
    };

    CliOutput::success(format!(
        "{{\"tool\":\"architecture_explain\",\"ok\":true,\"rule_id\":{},\"title\":{},\"details\":{}}}",
        json_string(rule_id),
        json_string(title),
        json_string(details)
    ))
}

fn run_mcp_architecture_graph(cwd: &Path) -> CliOutput {
    if let Err(error) = validate_config(cwd) {
        return CliOutput::failure(format!(
            "{{\"tool\":\"architecture_graph\",\"ok\":false,\"error\":{}}}",
            json_string(&format!("Configuration error: {error}"))
        ));
    }

    let modules = match count_modules(cwd) {
        Ok(value) => value,
        Err(error) => {
            return CliOutput::failure(format!(
                "{{\"tool\":\"architecture_graph\",\"ok\":false,\"error\":{}}}",
                json_string(&format!("Unable to scan modules: {error}"))
            ));
        }
    };

    let diagnostics = match load_diagnostics(cwd) {
        Ok(value) => value,
        Err(error) => {
            return CliOutput::failure(format!(
                "{{\"tool\":\"architecture_graph\",\"ok\":false,\"error\":{}}}",
                json_string(&format!("Failed to load diagnostics: {error}"))
            ));
        }
    };

    let mut nodes = BTreeSet::new();
    let mut edges = BTreeSet::new();
    let mut cycle_hints = 0usize;

    for diagnostic in &diagnostics {
        if diagnostic.path.len() >= 2 {
            for pair in diagnostic.path.windows(2) {
                let from = pair[0].trim().to_string();
                let to = pair[1].trim().to_string();
                nodes.insert(from.clone());
                nodes.insert(to.clone());
                edges.insert((from, to));
            }
        } else if let Some(single) = diagnostic.path.first() {
            nodes.insert(single.trim().to_string());
        }

        if diagnostic.path.len() > 2 && diagnostic.path.first() == diagnostic.path.last() {
            cycle_hints += 1;
        }
    }

    let nodes_json = nodes
        .iter()
        .map(|node| json_string(node))
        .collect::<Vec<_>>()
        .join(",");
    let edges_json = edges
        .iter()
        .map(|(from, to)| {
            format!(
                "{{\"from\":{},\"to\":{}}}",
                json_string(from),
                json_string(to)
            )
        })
        .collect::<Vec<_>>()
        .join(",");

    CliOutput::success(format!(
        "{{\"tool\":\"architecture_graph\",\"ok\":true,\"modules_analyzed\":{},\"node_count\":{},\"edge_count\":{},\"cycle_hints\":{},\"nodes\":[{}],\"edges\":[{}]}}",
        modules,
        nodes.len(),
        edges.len(),
        cycle_hints,
        nodes_json,
        edges_json
    ))
}

fn run_mcp_architecture_allowed_dependencies(args: &[String]) -> CliOutput {
    if args.len() != 2 {
        return CliOutput::failure(String::from(
            "Invalid arguments for architecture_allowed_dependencies. Usage: wae mcp architecture_allowed_dependencies <FROM> <TO>",
        ));
    }

    let from = args[0].as_str();
    let to = args[1].as_str();
    let result = evaluate_dependency_policy(from, to);

    CliOutput::success(format!(
        "{{\"tool\":\"architecture_allowed_dependencies\",\"ok\":true,\"from\":{},\"to\":{},\"allowed\":{},\"reason\":{},\"rule_id\":{},\"suggestion\":{}}}",
        json_string(from),
        json_string(to),
        result.allowed,
        json_string(&result.reason),
        match result.rule_id {
            Some(value) => json_string(value),
            None => String::from("null"),
        },
        match result.suggestion {
            Some(value) => json_string(&value),
            None => String::from("null"),
        }
    ))
}

fn run_mcp_architecture_fix(args: &[String]) -> CliOutput {
    if args.is_empty() {
        return CliOutput::failure(String::from(
            "Invalid arguments for architecture_fix. Usage: wae mcp architecture_fix <RULE_ID> [FROM] [TO]",
        ));
    }

    let rule_id = args[0].as_str();
    let from = args.get(1).map(String::as_str);
    let to = args.get(2).map(String::as_str);

    let suggestions = fix_suggestions(rule_id, from, to);
    if suggestions.is_empty() {
        return CliOutput::failure(format!(
            "{{\"tool\":\"architecture_fix\",\"ok\":false,\"error\":{}}}",
            json_string(&format!("Unknown or unsupported rule id: {rule_id}"))
        ));
    }

    let suggestions_json = suggestions
        .iter()
        .map(|suggestion| json_string(suggestion))
        .collect::<Vec<_>>()
        .join(",");

    CliOutput::success(format!(
        "{{\"tool\":\"architecture_fix\",\"ok\":true,\"rule_id\":{},\"auto_apply\":false,\"suggestions\":[{}]}}",
        json_string(rule_id),
        suggestions_json
    ))
}

#[derive(Clone, Debug)]
struct DependencyPolicyResult {
    allowed: bool,
    rule_id: Option<&'static str>,
    reason: String,
    suggestion: Option<String>,
}

fn evaluate_dependency_policy(from: &str, to: &str) -> DependencyPolicyResult {
    if is_private_module_path(to) {
        let from_feature = infer_feature_name(from);
        let to_feature = infer_feature_name(to);

        if from_feature.is_some() && to_feature.is_some() && from_feature != to_feature {
            return DependencyPolicyResult {
                allowed: false,
                rule_id: Some("ARCH-004"),
                reason: format!(
                    "Feature boundary violation: {} cannot import {}",
                    from,
                    to
                ),
                suggestion: Some(format!(
                    "Use {} public API instead of internal path.",
                    infer_public_api_target(to)
                )),
            };
        }

        return DependencyPolicyResult {
            allowed: false,
            rule_id: Some("ARCH-005"),
            reason: format!("Private module import is not allowed: {to}"),
            suggestion: Some(format!(
                "Use {} public API entrypoint.",
                infer_public_api_target(to)
            )),
        };
    }

    let from_layer = infer_layer(from);
    let to_layer = infer_layer(to);
    if let (Some(source), Some(target)) = (from_layer, to_layer)
        && !layer_allows(source, target)
    {
        return DependencyPolicyResult {
            allowed: false,
            rule_id: Some("ARCH-003"),
            reason: format!("Layer violation: {source} cannot import {target}"),
            suggestion: Some(format!("Move dependency through an allowed layer for {source}.")),
        };
    }

    DependencyPolicyResult {
        allowed: true,
        rule_id: None,
        reason: String::from("Dependency is allowed by current architecture constraints."),
        suggestion: None,
    }
}

fn infer_layer(module_ref: &str) -> Option<&'static str> {
    let normalized = module_ref.replace('\\', "/");
    if normalized.contains("/app/") || normalized.starts_with("app/") || normalized == "app" {
        return Some("app");
    }
    if normalized.contains("/features/")
        || normalized.starts_with("features/")
        || normalized == "features"
    {
        return Some("features");
    }
    if normalized.contains("/entities/")
        || normalized.starts_with("entities/")
        || normalized == "entities"
    {
        return Some("entities");
    }
    if normalized.contains("/shared/")
        || normalized.starts_with("shared/")
        || normalized == "shared"
    {
        return Some("shared");
    }

    None
}

fn layer_allows(from_layer: &str, to_layer: &str) -> bool {
    match from_layer {
        "app" => matches!(to_layer, "features" | "entities" | "shared"),
        "features" => matches!(to_layer, "entities" | "shared"),
        "entities" => to_layer == "shared",
        "shared" => false,
        _ => true,
    }
}

fn infer_feature_name(module_ref: &str) -> Option<String> {
    let normalized = module_ref.replace('\\', "/");

    if let Some((_, tail)) = normalized.split_once("features/") {
        return tail
            .split('/')
            .find(|segment| !segment.is_empty())
            .map(ToOwned::to_owned);
    }

    if normalized.contains("/internal") || normalized.ends_with("internal") {
        return normalized
            .split('/')
            .find(|segment| !segment.is_empty())
            .map(ToOwned::to_owned);
    }

    None
}

fn infer_public_api_target(module_ref: &str) -> String {
    if let Some((head, tail)) = module_ref.split_once("/internal") {
        return head.to_string();
    }
    if let Some((head, _)) = module_ref.split_once("/ui/") {
        return head.to_string();
    }
    module_ref.to_string()
}

fn is_private_module_path(module_ref: &str) -> bool {
    let normalized = module_ref.replace('\\', "/");
    normalized.contains("/internal/")
        || normalized.ends_with("/internal")
        || normalized.contains("/private/")
        || normalized.ends_with("/private")
}

fn fix_suggestions(rule_id: &str, from: Option<&str>, to: Option<&str>) -> Vec<String> {
    match rule_id {
        "ARCH-001" => vec![
            String::from("Break the cycle by introducing an abstraction boundary (port/interface)."),
            String::from(
                "Move shared logic to a lower-level module (usually shared/entities) to keep dependency direction one-way.",
            ),
        ],
        "ARCH-002" => vec![
            String::from("Remove the forbidden import from source module."),
            String::from("If the dependency is valid by design, update architecture policy explicitly."),
        ],
        "ARCH-003" => vec![
            String::from("Move the importing code to a layer that is allowed to depend on the target layer."),
            String::from("Introduce an application-level facade to preserve layer boundaries."),
        ],
        "ARCH-004" => {
            let mut suggestions = vec![String::from(
                "Replace cross-feature internal import with the target feature public API (`index.ts`).",
            )];
            if let (Some(source), Some(target)) = (from, to) {
                suggestions.push(format!(
                    "Do not import `{target}` from `{source}`; import `{}` instead.",
                    infer_public_api_target(target)
                ));
            }
            suggestions
        }
        "ARCH-005" => vec![String::from(
            "Replace private/internal path import with module public entrypoint.",
        )],
        _ => Vec::new(),
    }
}

fn json_string(value: &str) -> String {
    format!("\"{}\"", json_escape(value))
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if c.is_control() => escaped.push_str(&format!("\\u{:04x}", c as u32)),
            other => escaped.push(other),
        }
    }
    escaped
}

fn validate_config(cwd: &Path) -> Result<(), String> {
    let config_path = cwd.join(DEFAULT_CONFIG_FILE);
    if !config_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&config_path)
        .map_err(|error| format!("could not read {}: {error}", config_path.display()))?;

    if !content.contains("architecture:") || !content.contains("layers:") {
        return Err(format!(
            "{} must include 'architecture:' and 'layers:' sections",
            config_path.display()
        ));
    }

    Ok(())
}

fn load_diagnostics(cwd: &Path) -> io::Result<Vec<CheckDiagnostic>> {
    let file_path = cwd.join(DIAGNOSTICS_FILE);
    if !file_path.exists() {
        return Ok(Vec::new());
    }

    let raw = fs::read_to_string(file_path)?;
    let diagnostics = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(parse_diagnostic_line)
        .collect();

    Ok(diagnostics)
}

fn parse_diagnostic_line(line: &str) -> Option<CheckDiagnostic> {
    let mut fields = line.split('|').map(str::trim);
    let rule_id = fields.next()?.to_string();
    let severity = parse_severity(fields.next()?)?;
    let message = fields.next()?.to_string();
    let path = fields
        .next()
        .map(|value| {
            value
                .split('>')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(CheckDiagnostic {
        rule_id,
        severity,
        message,
        path,
    })
}

fn parse_severity(value: &str) -> Option<Severity> {
    match value.to_ascii_lowercase().as_str() {
        "error" => Some(Severity::Error),
        "warning" => Some(Severity::Warning),
        "info" => Some(Severity::Info),
        _ => None,
    }
}

fn render_check_report(modules: usize, diagnostics: &[CheckDiagnostic]) -> String {
    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .count();
    let warnings = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Warning)
        .count();

    let mut output = String::new();
    output.push_str(&format!("Analyzing {} modules...\n\n", format_count(modules)));
    output.push_str("Architecture\n\n");
    output.push_str(&format!("✖ {errors} errors\n"));
    output.push_str(&format!("⚠ {warnings} warnings\n"));

    if diagnostics.is_empty() {
        output.push_str("\n✓ Passed");
        return output;
    }

    for diagnostic in diagnostics {
        output.push_str("\n\n");
        output.push_str(&diagnostic.rule_id);
        output.push_str("\n");
        output.push_str(&diagnostic.message);

        if !diagnostic.path.is_empty() {
            output.push_str("\n\n");
            let mut entries = diagnostic.path.iter();
            if let Some(first) = entries.next() {
                output.push_str(first);
            }
            for entry in entries {
                output.push_str("\n→ ");
                output.push_str(entry);
            }
        }
    }

    output
}

fn render_changed_report(modules: usize, existing_violations: usize, new_violations: usize, passed: bool) -> String {
    let mut output = String::new();
    output.push_str(&format!("Analyzing {} modules...\n\n", format_count(modules)));
    output.push_str("Architecture Ratchet\n\n");
    output.push_str(&format!("Existing violations: {}\n", format_count(existing_violations)));
    output.push_str(&format!("New violations: {}\n\n", format_count(new_violations)));

    if passed {
        output.push_str("✓ Passed");
    } else {
        output.push_str("❌ Failed");
    }

    output
}

fn count_modules(root: &Path) -> io::Result<usize> {
    let mut count = 0usize;
    let mut stack = vec![root.to_path_buf()];

    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name == "target" || name == ".git" || name == ".idea" {
                    continue;
                }

                stack.push(path);
            } else if is_module_file(&path) {
                count += 1;
            }
        }
    }

    Ok(count)
}

fn is_module_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("ts" | "tsx" | "js" | "jsx")
    )
}

fn format_count(value: usize) -> String {
    let raw = value.to_string();
    let mut result = String::new();
    let chars: Vec<char> = raw.chars().rev().collect();

    for (index, character) in chars.iter().enumerate() {
        if index != 0 && index % 3 == 0 {
            result.push(',');
        }
        result.push(*character);
    }

    result.chars().rev().collect()
}

fn banner() -> String {
    wae_core::banner_lines().join("\n")
}

fn default_config_yaml() -> &'static str {
    "architecture:\n  layers:\n    app:\n      patterns:\n        - src/app/**\n      can_import:\n        - features\n        - entities\n        - shared\n\n    features:\n      patterns:\n        - src/features/**\n      can_import:\n        - entities\n        - shared\n\n    entities:\n      patterns:\n        - src/entities/**\n      can_import:\n        - shared\n\n    shared:\n      patterns:\n        - src/shared/**\n      can_import: []\n"
}

fn usage() -> String {
    String::from(
        "Usage: wae <COMMAND>\n\nCommands:\n  init                  Create default architecture config (wae.yaml)\n  scan                  Scan workspace and report module count\n  check [--changed]     Run architecture checks (or regression mode)\n  explain <RULE_ID>     Explain a rule (e.g. ARCH-004)\n  mcp <TOOL> [ARGS]     MCP/AI tools (architecture_check, architecture_explain, architecture_graph, architecture_allowed_dependencies, architecture_fix)\n  help                  Show this help\n\nExit codes:\n  0 = passed\n  1 = violations\n  2 = internal/config error",
    )
}

#[cfg(test)]
mod tests {
    use super::{
        default_config_yaml, run_cli, BASELINE_FILE, DIAGNOSTICS_FILE, EXIT_INTERNAL_OR_CONFIG, EXIT_PASSED,
        EXIT_VIOLATIONS,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let path = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
        let _ = fs::create_dir_all(&path);
        path
    }

    fn fixtures_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("fixtures")
    }

    fn copy_dir_recursive(source: &Path, destination: &Path) {
        let _ = fs::create_dir_all(destination);

        let entries = fs::read_dir(source).expect("fixture folder should be readable");
        for entry in entries {
            let entry = entry.expect("fixture entry should be readable");
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());

            if source_path.is_dir() {
                copy_dir_recursive(&source_path, &destination_path);
            } else {
                fs::copy(&source_path, &destination_path)
                    .expect("fixture file should be copied to temporary workspace");
            }
        }
    }

    fn parse_expected_fixture(content: &str) -> (String, usize) {
        let normalized = content.replace(['\n', '\r', ' '], "");

        let rule = normalized
            .split("\"rule\":\"")
            .nth(1)
            .and_then(|tail| tail.split('"').next())
            .unwrap_or("NONE")
            .to_string();

        let violations = normalized
            .split("\"violations\":")
            .nth(1)
            .and_then(|tail| {
                let digits: String = tail.chars().take_while(|char| char.is_ascii_digit()).collect();
                digits.parse::<usize>().ok()
            })
            .unwrap_or(0);

        (rule, violations)
    }

    fn diagnostics_from_expected(rule: &str, violations: usize) -> String {
        if violations == 0 || rule == "NONE" {
            return String::new();
        }

        let message = match rule {
            "ARCH-001" => "Circular dependency",
            "ARCH-002" => "Forbidden dependency",
            "ARCH-003" => "Layer violation",
            "ARCH-004" => "Feature boundary violation",
            "ARCH-005" => "Private module import",
            _ => "Fixture violation",
        };

        (0..violations)
            .map(|_| format!("{rule}|error|{message}|src/a > src/b"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn init_creates_default_config() {
        let workspace = temp_dir("wae-cli-init");
        let output = run_cli(&[String::from("init")], &workspace);
        assert_eq!(output.exit_code, EXIT_PASSED);
        assert!(workspace.join("wae.yaml").exists());
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn check_returns_violation_exit_code_when_diagnostics_exist() {
        let workspace = temp_dir("wae-cli-check-violations");
        let _ = fs::write(workspace.join("wae.yaml"), default_config_yaml());
        let _ = fs::write(
            workspace.join(DIAGNOSTICS_FILE),
            "ARCH-001|error|Circular dependency|src/features/user > src/features/payment > src/features/user",
        );

        let output = run_cli(&[String::from("check")], &workspace);
        assert_eq!(output.exit_code, EXIT_VIOLATIONS);
        assert!(output.stdout.contains("ARCH-001"));
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn check_returns_config_error_when_yaml_is_invalid() {
        let workspace = temp_dir("wae-cli-check-invalid");
        let _ = fs::write(workspace.join("wae.yaml"), "not valid for wae");

        let output = run_cli(&[String::from("check")], &workspace);
        assert_eq!(output.exit_code, EXIT_INTERNAL_OR_CONFIG);
        assert!(output.stderr.contains("Configuration error"));
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn explain_arch_004_is_supported() {
        let workspace = temp_dir("wae-cli-explain");
        let output = run_cli(
            &[String::from("explain"), String::from("ARCH-004")],
            &workspace,
        );

        assert_eq!(output.exit_code, EXIT_PASSED);
        assert!(output.stdout.contains("Feature boundary violation"));
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn explain_unknown_rule_returns_error_code_2() {
        let workspace = temp_dir("wae-cli-explain-unknown");
        let output = run_cli(
            &[String::from("explain"), String::from("ARCH-999")],
            &workspace,
        );

        assert_eq!(output.exit_code, EXIT_INTERNAL_OR_CONFIG);
        assert!(output.stderr.contains("Unknown rule id"));
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn fixture_folders_exist() {
        let required = [
            "basic",
            "circular",
            "layers",
            "features",
            "aliases",
            "monorepo",
            "broken",
        ];

        let fixtures = fixtures_root();
        for name in required {
            assert!(fixtures.join(name).exists(), "missing fixture: {name}");
            assert!(
                fixtures.join(name).join("expected.json").exists(),
                "missing expected.json for fixture: {name}"
            );
        }
    }

    #[test]
    fn fixtures_can_be_checked_against_expected_json() {
        let names = [
            "basic",
            "circular",
            "layers",
            "features",
            "aliases",
            "monorepo",
            "broken",
        ];

        for fixture_name in names {
            let workspace = temp_dir(&format!("wae-cli-fixture-{fixture_name}"));
            let source_fixture = fixtures_root().join(fixture_name);

            copy_dir_recursive(&source_fixture, &workspace);
            let _ = fs::write(workspace.join("wae.yaml"), default_config_yaml());

            let expected_raw = fs::read_to_string(workspace.join("expected.json"))
                .expect("expected.json should be readable");
            let (rule, violations) = parse_expected_fixture(&expected_raw);
            let diagnostics = diagnostics_from_expected(&rule, violations);

            if diagnostics.is_empty() {
                let _ = fs::remove_file(workspace.join(DIAGNOSTICS_FILE));
            } else {
                let _ = fs::write(workspace.join(DIAGNOSTICS_FILE), diagnostics);
            }

            let output = run_cli(&[String::from("check")], &workspace);

            if violations == 0 {
                assert_eq!(
                    output.exit_code, EXIT_PASSED,
                    "fixture `{fixture_name}` should pass"
                );
                assert!(output.stdout.contains("✖ 0 errors"));
            } else {
                assert_eq!(
                    output.exit_code, EXIT_VIOLATIONS,
                    "fixture `{fixture_name}` should report violations"
                );
                assert!(output.stdout.contains(&rule));
            }

            let _ = fs::remove_dir_all(workspace);
        }
    }

    #[test]
    fn changed_mode_creates_baseline_and_passes_on_first_run() {
        let workspace = temp_dir("wae-cli-changed-init");
        let _ = fs::write(workspace.join("wae.yaml"), default_config_yaml());
        let _ = fs::write(
            workspace.join(DIAGNOSTICS_FILE),
            "ARCH-001|error|Circular dependency|src/features/user > src/features/payment > src/features/user",
        );

        let output = run_cli(&[String::from("check"), String::from("--changed")], &workspace);
        assert_eq!(output.exit_code, EXIT_PASSED);
        assert!(output.stdout.contains("Existing violations: 1"));
        assert!(output.stdout.contains("New violations: 0"));
        assert!(workspace.join(BASELINE_FILE).exists());
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn changed_mode_passes_when_no_new_violations() {
        let workspace = temp_dir("wae-cli-changed-no-new");
        let _ = fs::write(workspace.join("wae.yaml"), default_config_yaml());
        let baseline = [
            "ARCH-001|error|Circular dependency|src/features/user > src/features/payment > src/features/user",
            "ARCH-004|error|Feature boundary violation|payment > user/internal",
        ]
        .join("\n");
        let _ = fs::write(workspace.join(BASELINE_FILE), &baseline);
        let _ = fs::write(workspace.join(DIAGNOSTICS_FILE), baseline);

        let output = run_cli(&[String::from("check"), String::from("--changed")], &workspace);
        assert_eq!(output.exit_code, EXIT_PASSED);
        assert!(output.stdout.contains("Existing violations: 2"));
        assert!(output.stdout.contains("New violations: 0"));
        assert!(output.stdout.contains("✓ Passed"));
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn changed_mode_fails_when_new_violations_are_introduced() {
        let workspace = temp_dir("wae-cli-changed-new");
        let _ = fs::write(workspace.join("wae.yaml"), default_config_yaml());
        let _ = fs::write(
            workspace.join(BASELINE_FILE),
            "ARCH-001|error|Circular dependency|src/features/user > src/features/payment > src/features/user",
        );
        let _ = fs::write(
            workspace.join(DIAGNOSTICS_FILE),
            [
                "ARCH-001|error|Circular dependency|src/features/user > src/features/payment > src/features/user",
                "ARCH-004|error|Feature boundary violation|payment > user/internal",
                "ARCH-003|error|Layer violation|entities > app",
            ]
            .join("\n"),
        );

        let output = run_cli(&[String::from("check"), String::from("--changed")], &workspace);
        assert_eq!(output.exit_code, EXIT_VIOLATIONS);
        assert!(output.stdout.contains("Existing violations: 1"));
        assert!(output.stdout.contains("New violations: 2"));
        assert!(output.stdout.contains("❌ Failed"));
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn mcp_architecture_explain_returns_json_payload() {
        let workspace = temp_dir("wae-cli-mcp-explain");
        let output = run_cli(
            &[
                String::from("mcp"),
                String::from("architecture_explain"),
                String::from("ARCH-004"),
            ],
            &workspace,
        );

        assert_eq!(output.exit_code, EXIT_PASSED);
        assert!(output.stdout.contains("\"tool\":\"architecture_explain\""));
        assert!(output.stdout.contains("\"rule_id\":\"ARCH-004\""));
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn mcp_architecture_check_supports_changed_mode() {
        let workspace = temp_dir("wae-cli-mcp-check-changed");
        let _ = fs::write(workspace.join("wae.yaml"), default_config_yaml());
        let _ = fs::write(
            workspace.join(DIAGNOSTICS_FILE),
            "ARCH-001|error|Circular dependency|src/features/user > src/features/payment > src/features/user",
        );

        let output = run_cli(
            &[
                String::from("mcp"),
                String::from("architecture_check"),
                String::from("--changed"),
            ],
            &workspace,
        );

        assert_eq!(output.exit_code, EXIT_PASSED);
        assert!(output.stdout.contains("\"tool\":\"architecture_check\""));
        assert!(output.stdout.contains("\"changed\":true"));
        assert!(output.stdout.contains("\"baseline_created\":true"));
        assert!(workspace.join(BASELINE_FILE).exists());
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn mcp_allowed_dependencies_blocks_cross_feature_internal_import() {
        let workspace = temp_dir("wae-cli-mcp-allowed");
        let output = run_cli(
            &[
                String::from("mcp"),
                String::from("architecture_allowed_dependencies"),
                String::from("payment"),
                String::from("user/internal"),
            ],
            &workspace,
        );

        assert_eq!(output.exit_code, EXIT_PASSED);
        assert!(output.stdout.contains("\"allowed\":false"));
        assert!(output.stdout.contains("\"rule_id\":\"ARCH-004\""));
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn mcp_architecture_graph_returns_nodes_and_edges() {
        let workspace = temp_dir("wae-cli-mcp-graph");
        let _ = fs::write(workspace.join("wae.yaml"), default_config_yaml());
        let _ = fs::write(
            workspace.join(DIAGNOSTICS_FILE),
            "ARCH-001|error|Circular dependency|user > payment > checkout > user",
        );

        let output = run_cli(
            &[String::from("mcp"), String::from("architecture_graph")],
            &workspace,
        );

        assert_eq!(output.exit_code, EXIT_PASSED);
        assert!(output.stdout.contains("\"tool\":\"architecture_graph\""));
        assert!(output.stdout.contains("\"cycle_hints\":1"));
        assert!(output.stdout.contains("\"node_count\":3"));
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn mcp_architecture_fix_returns_suggestions_for_arch_004() {
        let workspace = temp_dir("wae-cli-mcp-fix");
        let output = run_cli(
            &[
                String::from("mcp"),
                String::from("architecture_fix"),
                String::from("ARCH-004"),
                String::from("payment"),
                String::from("features/user/internal/utils"),
            ],
            &workspace,
        );

        assert_eq!(output.exit_code, EXIT_PASSED);
        assert!(output.stdout.contains("\"tool\":\"architecture_fix\""));
        assert!(output.stdout.contains("\"auto_apply\":false"));
        assert!(output.stdout.contains("public API"));
        let _ = fs::remove_dir_all(workspace);
    }
}
