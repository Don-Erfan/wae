use std::collections::BTreeMap;

use regex::Regex;
use wae_core::domain::{FrameworkMetadata, Runtime};

#[derive(Clone, Debug, Default)]
pub struct ProjectEvidence {
    pub package_manifest: Option<serde_json::Value>,
    pub config_files: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct ModuleEvidence<'a> {
    pub path: &'a str,
    pub source: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameworkClassification {
    pub metadata: FrameworkMetadata,
    pub runtime: Runtime,
}

pub trait FrameworkAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn detection_score(&self, project: &ProjectEvidence) -> u8;
    fn classify(&self, module: ModuleEvidence<'_>) -> FrameworkClassification;
}

pub struct FrameworkRegistry {
    adapters: Vec<Box<dyn FrameworkAdapter>>,
}

impl Default for FrameworkRegistry {
    fn default() -> Self {
        Self { adapters: vec![Box::new(NextJsAdapter)] }
    }
}

impl FrameworkRegistry {
    pub fn select<'a>(
        &'a self,
        project: &ProjectEvidence,
        enabled: &[String],
        auto_detect: bool,
    ) -> Option<&'a dyn FrameworkAdapter> {
        if !auto_detect && enabled.is_empty() {
            return None;
        }
        self.adapters
            .iter()
            .filter(|adapter| enabled.is_empty() || enabled.iter().any(|id| id == adapter.id()))
            .map(|adapter| (adapter.as_ref(), adapter.detection_score(project)))
            .filter(|(_, score)| !auto_detect || *score > 0)
            .max_by_key(|(adapter, score)| (*score, adapter.id()))
            .map(|(adapter, _)| adapter)
    }
}

pub struct NextJsAdapter;

impl FrameworkAdapter for NextJsAdapter {
    fn id(&self) -> &'static str {
        "nextjs"
    }

    fn detection_score(&self, project: &ProjectEvidence) -> u8 {
        let manifest_score = project
            .package_manifest
            .as_ref()
            .is_some_and(|manifest| manifest_has_dependency(manifest, "next"))
            .then_some(100)
            .unwrap_or_default();
        let config_score = project
            .config_files
            .iter()
            .any(|path| {
                matches!(
                    path.as_str(),
                    "next.config.js" | "next.config.mjs" | "next.config.cjs" | "next.config.ts"
                )
            })
            .then_some(90)
            .unwrap_or_default();
        manifest_score.max(config_score)
    }

    fn classify(&self, module: ModuleEvidence<'_>) -> FrameworkClassification {
        let path = module.path.replace('\\', "/");
        let segments = path.split('/').collect::<Vec<_>>();
        let app_index = segments.iter().position(|part| *part == "app");
        let pages_index = segments.iter().position(|part| *part == "pages");
        let file = segments.last().copied().unwrap_or_default();
        let stem = file.split('.').next().unwrap_or(file);
        let directives = directive_prologue(module.source);
        let use_client = directives.iter().any(|directive| directive == "use client");
        let use_server = directives.iter().any(|directive| directive == "use server");
        let explicit_runtime = explicit_runtime(module.source);

        let router = if app_index.is_some() {
            "app"
        } else if pages_index.is_some() {
            "pages"
        } else {
            "none"
        };
        let role = if file.starts_with("middleware.") || stem == "middleware" {
            "middleware"
        } else if app_index.is_some() {
            match stem {
                "page" => "page",
                "layout" => "layout",
                "route" => "route-handler",
                "loading" => "loading",
                "error" => "error-boundary",
                "not-found" => "not-found",
                "template" => "template",
                _ if use_server => "server-action-module",
                _ => "module",
            }
        } else if let Some(index) = pages_index {
            if segments.get(index + 1) == Some(&"api") {
                "api-route"
            } else {
                match stem {
                    "_app" => "custom-app",
                    "_document" => "custom-document",
                    "_error" => "custom-error",
                    _ => "page",
                }
            }
        } else if use_server {
            "server-action-module"
        } else {
            "module"
        };

        let (runtime, runtime_source) = if let Some(runtime) = explicit_runtime {
            (runtime, "explicit")
        } else if use_client {
            (Runtime::Browser, "directive")
        } else if role == "middleware" {
            (Runtime::Edge, "convention")
        } else if app_index.is_some()
            || matches!(role, "api-route" | "custom-document" | "server-action-module")
        {
            (Runtime::Server, "convention")
        } else {
            (Runtime::Universal, "default")
        };

        let attributes = BTreeMap::from([
            ("router".into(), router.into()),
            ("role".into(), role.into()),
            (
                "component".into(),
                if use_client {
                    "client"
                } else if app_index.is_some() {
                    "server"
                } else {
                    "none"
                }
                .into(),
            ),
            ("runtime".into(), runtime_name(&runtime).into()),
            ("runtimeSource".into(), runtime_source.into()),
            ("useClient".into(), use_client.to_string()),
            ("useServer".into(), use_server.to_string()),
        ]);
        FrameworkClassification {
            metadata: FrameworkMetadata { adapter_id: Some(self.id().into()), attributes },
            runtime,
        }
    }
}

fn manifest_has_dependency(manifest: &serde_json::Value, dependency: &str) -> bool {
    ["dependencies", "devDependencies", "peerDependencies"]
        .into_iter()
        .filter_map(|field| manifest.get(field).and_then(serde_json::Value::as_object))
        .any(|dependencies| dependencies.contains_key(dependency))
}

fn explicit_runtime(source: &str) -> Option<Runtime> {
    let pattern = Regex::new(r#"(?m)\bexport\s+const\s+runtime\s*=\s*['\"](edge|nodejs)['\"]"#)
        .expect("static Next.js runtime regex");
    match pattern.captures(source).and_then(|captures| captures.get(1)).map(|value| value.as_str())
    {
        Some("edge") => Some(Runtime::Edge),
        Some("nodejs") => Some(Runtime::Node),
        _ => None,
    }
}

fn runtime_name(runtime: &Runtime) -> &'static str {
    match runtime {
        Runtime::Browser => "browser",
        Runtime::Server => "server",
        Runtime::Edge => "edge",
        Runtime::Node => "node",
        Runtime::Universal => "universal",
        Runtime::Unknown => "unknown",
    }
}

/// Reads only the ECMAScript directive prologue. Strings later in a module are not directives.
fn directive_prologue(source: &str) -> Vec<String> {
    let mut rest = source.trim_start_matches('\u{feff}');
    let mut directives = Vec::new();
    loop {
        rest = trim_leading_trivia(rest);
        let Some(quote) =
            rest.as_bytes().first().copied().filter(|byte| matches!(byte, b'\'' | b'"'))
        else {
            break;
        };
        let Some(end) = rest[1..].find(char::from(quote)) else { break };
        let value = &rest[1..end + 1];
        let after = rest[end + 2..].trim_start();
        if !after.starts_with(';') {
            break;
        }
        directives.push(value.to_string());
        rest = &after[1..];
    }
    directives
}

fn trim_leading_trivia(mut source: &str) -> &str {
    loop {
        source = source.trim_start();
        if let Some(end) = source.strip_prefix("//").and_then(|value| value.find('\n')) {
            source = &source[end + 3..];
            continue;
        }
        if let Some(end) = source.strip_prefix("/*").and_then(|value| value.find("*/")) {
            source = &source[end + 4..];
            continue;
        }
        return source;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_detection_requires_authoritative_project_evidence() {
        let adapter = NextJsAdapter;
        assert_eq!(adapter.detection_score(&ProjectEvidence::default()), 0);
        let manifest = serde_json::json!({"dependencies":{"next":"15.2.6"}});
        assert_eq!(
            adapter.detection_score(&ProjectEvidence {
                package_manifest: Some(manifest),
                config_files: vec![]
            }),
            100
        );
        assert_eq!(
            adapter.detection_score(&ProjectEvidence {
                package_manifest: None,
                config_files: vec!["next.config.ts".into()]
            }),
            90
        );
    }

    #[test]
    fn classifies_app_router_client_server_route_action_and_middleware_modules() {
        let adapter = NextJsAdapter;
        let classify = |path, source| adapter.classify(ModuleEvidence { path, source });
        let client =
            classify("src/app/cart/client.tsx", "// lead\n'use client';\nexport default 1;");
        assert_eq!(client.runtime, Runtime::Browser);
        assert_eq!(client.metadata.attributes["component"], "client");
        let page = classify("src/app/cart/page.tsx", "export default function Page() {}");
        assert_eq!(page.runtime, Runtime::Server);
        assert_eq!(page.metadata.attributes["role"], "page");
        let route = classify("src/app/api/route.ts", "export const runtime = 'edge';");
        assert_eq!(route.runtime, Runtime::Edge);
        assert_eq!(route.metadata.attributes["role"], "route-handler");
        let action = classify("src/actions.ts", "'use server'; export async function save() {}");
        assert_eq!(action.metadata.attributes["role"], "server-action-module");
        let middleware = classify("src/middleware.ts", "export function middleware() {}");
        assert_eq!(middleware.runtime, Runtime::Edge);
    }

    #[test]
    fn classifies_pages_router_conventions_without_treating_late_strings_as_directives() {
        let adapter = NextJsAdapter;
        let api = adapter.classify(ModuleEvidence {
            path: "src/pages/api/user.ts",
            source: "export default () => 'use client';",
        });
        assert_eq!(api.metadata.attributes["role"], "api-route");
        assert_eq!(api.metadata.attributes["useClient"], "false");
        assert_eq!(api.runtime, Runtime::Server);
        let document = adapter
            .classify(ModuleEvidence { path: "pages/_document.tsx", source: "export default 1;" });
        assert_eq!(document.metadata.attributes["role"], "custom-document");
    }
}
