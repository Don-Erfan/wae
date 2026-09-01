use std::collections::BTreeMap;

use wae_core::domain::{FrameworkMetadata, ModuleSemantics, Runtime};

#[derive(Clone, Debug, Default)]
pub struct ProjectEvidence {
    pub package_manifest: Option<serde_json::Value>,
    pub config_files: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct ModuleEvidence<'a> {
    pub path: &'a str,
    pub semantics: &'a ModuleSemantics,
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
        let manifest_score = if project
            .package_manifest
            .as_ref()
            .is_some_and(|manifest| manifest_has_dependency(manifest, "next"))
        {
            100
        } else {
            0
        };
        let config_score = if project.config_files.iter().any(|path| {
            matches!(
                path.as_str(),
                "next.config.js" | "next.config.mjs" | "next.config.cjs" | "next.config.ts"
            )
        }) {
            90
        } else {
            0
        };
        manifest_score.max(config_score)
    }

    fn classify(&self, module: ModuleEvidence<'_>) -> FrameworkClassification {
        let path = module.path.replace('\\', "/");
        let segments = path.split('/').collect::<Vec<_>>();
        let app_index = router_root_index(&segments, "app");
        let pages_index = router_root_index(&segments, "pages");
        let file = segments.last().copied().unwrap_or_default();
        let stem = file.split('.').next().unwrap_or(file);
        let use_client =
            module.semantics.directives.iter().any(|directive| directive == "use client");
        let use_server =
            module.semantics.directives.iter().any(|directive| directive == "use server");
        let server_only =
            module.semantics.marker_imports.iter().any(|specifier| specifier == "server-only");
        let client_only =
            module.semantics.marker_imports.iter().any(|specifier| specifier == "client-only");
        let explicit_runtime = explicit_runtime(module.semantics);

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

        let (runtime, runtime_source) = if server_only && client_only {
            (Runtime::Unknown, "conflicting-marker")
        } else if server_only {
            (Runtime::Server, "marker-package")
        } else if client_only {
            (Runtime::Browser, "marker-package")
        } else if let Some(runtime) = explicit_runtime {
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
            ("serverOnly".into(), server_only.to_string()),
            ("clientOnly".into(), client_only.to_string()),
        ]);
        FrameworkClassification {
            metadata: FrameworkMetadata { adapter_id: Some(self.id().into()), attributes },
            runtime,
        }
    }
}

fn router_root_index(segments: &[&str], router: &str) -> Option<usize> {
    segments.iter().position(|segment| *segment == router).filter(|index| match *index {
        0 => true,
        1 => segments[0] == "src",
        2 => matches!(segments[0], "apps" | "packages"),
        3 => matches!(segments[0], "apps" | "packages") && segments[2] == "src",
        _ => false,
    })
}

fn manifest_has_dependency(manifest: &serde_json::Value, dependency: &str) -> bool {
    ["dependencies", "devDependencies", "peerDependencies"]
        .into_iter()
        .filter_map(|field| manifest.get(field).and_then(serde_json::Value::as_object))
        .any(|dependencies| dependencies.contains_key(dependency))
}

fn explicit_runtime(semantics: &ModuleSemantics) -> Option<Runtime> {
    match semantics.exported_runtime.as_deref() {
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
    fn next_detection_is_stable_across_supported_manifest_versions() {
        let adapter = NextJsAdapter;
        for version in ["13.5.11", "14.2.35", "15.2.6", "16.0.0"] {
            let evidence = ProjectEvidence {
                package_manifest: Some(serde_json::json!({"dependencies":{"next":version}})),
                config_files: Vec::new(),
            };
            assert_eq!(adapter.detection_score(&evidence), 100, "Next.js {version}");
        }
    }

    #[test]
    fn classifies_app_router_client_server_route_action_and_middleware_modules() {
        let adapter = NextJsAdapter;
        let client_semantics =
            ModuleSemantics { directives: vec!["use client".into()], ..ModuleSemantics::default() };
        let client = adapter.classify(ModuleEvidence {
            path: "src/app/cart/client.tsx",
            semantics: &client_semantics,
        });
        assert_eq!(client.runtime, Runtime::Browser);
        assert_eq!(client.metadata.attributes["component"], "client");
        let empty = ModuleSemantics::default();
        let page =
            adapter.classify(ModuleEvidence { path: "src/app/cart/page.tsx", semantics: &empty });
        assert_eq!(page.runtime, Runtime::Server);
        assert_eq!(page.metadata.attributes["role"], "page");
        let edge =
            ModuleSemantics { exported_runtime: Some("edge".into()), ..ModuleSemantics::default() };
        let route =
            adapter.classify(ModuleEvidence { path: "src/app/api/route.ts", semantics: &edge });
        assert_eq!(route.runtime, Runtime::Edge);
        assert_eq!(route.metadata.attributes["role"], "route-handler");
        let server_action =
            ModuleSemantics { directives: vec!["use server".into()], ..ModuleSemantics::default() };
        let action =
            adapter.classify(ModuleEvidence { path: "src/actions.ts", semantics: &server_action });
        assert_eq!(action.metadata.attributes["role"], "server-action-module");
        let middleware =
            adapter.classify(ModuleEvidence { path: "src/middleware.ts", semantics: &empty });
        assert_eq!(middleware.runtime, Runtime::Edge);
    }

    #[test]
    fn classifies_pages_router_conventions_without_treating_late_strings_as_directives() {
        let adapter = NextJsAdapter;
        let api = adapter.classify(ModuleEvidence {
            path: "src/pages/api/user.ts",
            semantics: &ModuleSemantics::default(),
        });
        assert_eq!(api.metadata.attributes["role"], "api-route");
        assert_eq!(api.metadata.attributes["useClient"], "false");
        assert_eq!(api.runtime, Runtime::Server);
        let document = adapter.classify(ModuleEvidence {
            path: "pages/_document.tsx",
            semantics: &ModuleSemantics::default(),
        });
        assert_eq!(document.metadata.attributes["role"], "custom-document");
    }

    #[test]
    fn router_roots_are_anchored_and_marker_packages_define_runtime() {
        let adapter = NextJsAdapter;
        let nested = adapter.classify(ModuleEvidence {
            path: "src/components/app/page.tsx",
            semantics: &ModuleSemantics::default(),
        });
        assert_eq!(nested.metadata.attributes["router"], "none");
        assert_eq!(nested.metadata.attributes["role"], "module");

        let server = adapter.classify(ModuleEvidence {
            path: "src/lib/secrets.ts",
            semantics: &ModuleSemantics {
                marker_imports: vec!["server-only".into()],
                ..ModuleSemantics::default()
            },
        });
        assert_eq!(server.runtime, Runtime::Server);
        assert_eq!(server.metadata.attributes["runtimeSource"], "marker-package");

        let monorepo_page = adapter.classify(ModuleEvidence {
            path: "apps/store/src/app/page.tsx",
            semantics: &ModuleSemantics::default(),
        });
        assert_eq!(monorepo_page.metadata.attributes["router"], "app");
    }
}
