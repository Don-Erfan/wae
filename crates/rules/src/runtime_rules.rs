use std::collections::HashSet;

use globset::GlobBuilder;
use wae_core::domain::{Diagnostic, ModuleId, ModuleKind, Runtime, SourceLocation};
use wae_core::rule_registry::{self, RuleDescriptor};

use crate::{DiagnosticSink, Rule, RuleContext};

pub struct BrowserToServerRule;
impl Rule for BrowserToServerRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        descriptor("RUNTIME-001")
    }

    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        for source in modules_with_runtime(context, Runtime::Browser) {
            if let Some(path) =
                context.runtime_graph.shortest_path_to_runtime(&source, &[Runtime::Server])
            {
                sink.emit(runtime_diagnostic(
                    "RUNTIME-001",
                    context,
                    path,
                    "Browser module transitively depends on server-only code",
                    "Move the server operation behind a Server Action or another explicit RPC boundary.",
                ));
            }
        }
        Ok(())
    }
}

pub struct BrowserToNodeRule;
impl Rule for BrowserToNodeRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        descriptor("RUNTIME-002")
    }

    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        for source in modules_with_runtime(context, Runtime::Browser) {
            if let Some(path) =
                context.runtime_graph.shortest_path_to_runtime(&source, &[Runtime::Node])
            {
                sink.emit(runtime_diagnostic(
                    "RUNTIME-002",
                    context,
                    path,
                    "Browser module transitively depends on Node-only code",
                    "Split the Node implementation from its browser-safe contract.",
                ));
            }
        }
        Ok(())
    }
}

pub struct BrowserIncompatiblePackageRule;
impl Rule for BrowserIncompatiblePackageRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        descriptor("RUNTIME-003")
    }

    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        let targets = incompatible_external_modules(
            context,
            &context.config.runtime.browser_incompatible_packages,
        )?;
        let reachability = context.runtime_graph.reachability_index(&targets);
        if !reachability.has_targets() {
            return Ok(());
        }
        for source in modules_with_runtime(context, Runtime::Browser) {
            if let Some(path) = context.runtime_graph.shortest_path_in_index(&source, &reachability)
            {
                let package =
                    external_name(path.last().expect("non-empty runtime path")).to_owned();
                let mut diagnostic = runtime_diagnostic(
                    "RUNTIME-003",
                    context,
                    path,
                    "Browser module reaches a browser-incompatible package",
                    "Replace the package with a browser-safe adapter or keep it behind a server boundary.",
                );
                diagnostic.metadata.insert("package".into(), package);
                sink.emit(diagnostic);
            }
        }
        Ok(())
    }
}

pub struct EdgeIncompatibleDependencyRule;
impl Rule for EdgeIncompatibleDependencyRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        descriptor("RUNTIME-004")
    }

    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        let external_targets = incompatible_external_modules(
            context,
            &context.config.runtime.edge_incompatible_packages,
        )?;
        let external_reachability = context.runtime_graph.reachability_index(&external_targets);
        for source in modules_with_runtime(context, Runtime::Edge) {
            let node_path =
                context.runtime_graph.shortest_path_to_runtime(&source, &[Runtime::Node]);
            let external_path =
                context.runtime_graph.shortest_path_in_index(&source, &external_reachability);
            if let Some(path) = shortest(node_path, external_path) {
                sink.emit(runtime_diagnostic(
                    "RUNTIME-004",
                    context,
                    path,
                    "Edge module reaches a dependency unavailable in the Edge runtime",
                    "Use an Edge-compatible implementation or move this operation to the Node runtime.",
                ));
            }
        }
        Ok(())
    }
}

pub struct AmbiguousUniversalRuntimeRule;
impl Rule for AmbiguousUniversalRuntimeRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        descriptor("RUNTIME-005")
    }

    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        if !context.runtime_graph.has_runtime_targets(&[Runtime::Browser])
            || !context.runtime_graph.has_runtime_targets(&[Runtime::Server, Runtime::Node])
        {
            return Ok(());
        }
        for source in modules_with_runtime(context, Runtime::Universal) {
            let browser =
                context.runtime_graph.shortest_path_to_runtime(&source, &[Runtime::Browser]);
            let server = context
                .runtime_graph
                .shortest_path_to_runtime(&source, &[Runtime::Server, Runtime::Node]);
            if let (Some(browser_path), Some(server_path)) = (browser, server) {
                let mut diagnostic = runtime_diagnostic(
                    "RUNTIME-005",
                    context,
                    server_path,
                    "Universal module combines incompatible browser and server requirements",
                    "Split the shared contract from browser and server implementations.",
                );
                diagnostic.metadata.insert("browserPath".into(), path_text(&browser_path));
                sink.emit(diagnostic);
            }
        }
        Ok(())
    }
}

pub struct IncompatibleRuntimeCycleRule;
impl Rule for IncompatibleRuntimeCycleRule {
    fn metadata(&self) -> &'static RuleDescriptor {
        descriptor("RUNTIME-006")
    }

    fn evaluate(
        &self,
        context: &RuleContext<'_>,
        sink: &mut dyn DiagnosticSink,
    ) -> Result<(), String> {
        for cycle in context.runtime_graph.cycles() {
            let runtimes = cycle
                .iter()
                .filter_map(|module| context.runtime_graph.runtime_of(module))
                .collect::<HashSet<_>>();
            let browser_conflict = runtimes.contains(&Runtime::Browser)
                && (runtimes.contains(&Runtime::Server) || runtimes.contains(&Runtime::Node));
            let edge_conflict =
                runtimes.contains(&Runtime::Edge) && runtimes.contains(&Runtime::Node);
            if browser_conflict || edge_conflict {
                sink.emit(runtime_diagnostic(
                    "RUNTIME-006",
                    context,
                    cycle,
                    "Dependency cycle crosses incompatible runtime domains",
                    "Break the cycle at a runtime-neutral contract or an explicit transport boundary.",
                ));
            }
        }
        Ok(())
    }
}

fn descriptor(id: &str) -> &'static RuleDescriptor {
    rule_registry::descriptor(id).expect("registered built-in runtime rule")
}

fn modules_with_runtime(context: &RuleContext<'_>, runtime: Runtime) -> Vec<ModuleId> {
    context
        .project
        .modules
        .iter()
        .filter(|module| {
            module.kind != ModuleKind::External
                && module.runtime == runtime
                && !(runtime == Runtime::Browser
                    && module
                        .framework_metadata
                        .attributes
                        .get("runtimeSource")
                        .is_some_and(|source| source == "propagated"))
        })
        .map(|module| module.id.clone())
        .collect()
}

fn incompatible_external_modules(
    context: &RuleContext<'_>,
    patterns: &[String],
) -> Result<HashSet<ModuleId>, String> {
    let matchers = patterns
        .iter()
        .map(|pattern| {
            GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()
                .map(|glob| glob.compile_matcher())
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(context
        .project
        .modules
        .iter()
        .filter(|module| {
            module.kind == ModuleKind::External
                && matchers.iter().any(|matcher| matcher.is_match(&module.package.0))
        })
        .map(|module| module.id.clone())
        .collect())
}

fn shortest(left: Option<Vec<ModuleId>>, right: Option<Vec<ModuleId>>) -> Option<Vec<ModuleId>> {
    match (left, right) {
        (Some(left), Some(right)) => Some(if left.len() <= right.len() { left } else { right }),
        (left @ Some(_), None) => left,
        (None, right) => right,
    }
}

fn runtime_diagnostic(
    rule: &str,
    context: &RuleContext<'_>,
    path: Vec<ModuleId>,
    message: &str,
    suggestion: &str,
) -> Diagnostic {
    let primary_location =
        path.windows(2).next().and_then(|edge| edge_location(context, &edge[0], &edge[1]));
    let mut diagnostic = Diagnostic::new(rule, message);
    diagnostic.primary_location = primary_location;
    diagnostic.metadata.insert("runtimePath".into(), path_text(&path));
    diagnostic.dependency_path = path;
    diagnostic.suggestion = Some(suggestion.into());
    diagnostic
}

fn edge_location(
    context: &RuleContext<'_>,
    from: &ModuleId,
    to: &ModuleId,
) -> Option<SourceLocation> {
    context
        .project
        .dependencies
        .iter()
        .find(|dependency| dependency.from == *from && dependency.to == *to)
        .map(|dependency| dependency.location.clone())
}

fn path_text(path: &[ModuleId]) -> String {
    path.iter().map(|module| module.0.as_str()).collect::<Vec<_>>().join(" -> ")
}

fn external_name(module: &ModuleId) -> &str {
    module.0.strip_prefix("external:").unwrap_or(&module.0)
}
