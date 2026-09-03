use crate::{Resolution, ResolutionHandler, ResolutionRequest};

/// Node builtin catalog with the same bare/prefix-only distinction exposed by
/// `node:module.builtinModules`. The snapshot is generated from Node 24.14.1 and intentionally
/// stores complete specifiers so unsupported subpaths cannot be accepted accidentally.
#[derive(Clone, Copy, Debug, Default)]
pub struct BuiltinCatalog;

impl BuiltinCatalog {
    pub fn resolve(self, specifier: &str) -> Option<String> {
        if let Some(prefixed) = specifier.strip_prefix("node:") {
            return (BARE_BUILTINS.contains(&prefixed) || PREFIX_ONLY_BUILTINS.contains(&prefixed))
                .then(|| format!("node:{prefixed}"));
        }
        BARE_BUILTINS.contains(&specifier).then(|| format!("node:{specifier}"))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BuiltinResolver;

impl ResolutionHandler for BuiltinResolver {
    fn name(&self) -> &'static str {
        "node-builtin"
    }

    fn try_resolve(&self, request: &ResolutionRequest<'_>) -> Option<Resolution> {
        BuiltinCatalog.resolve(request.specifier).map(Resolution::Builtin)
    }
}

const BARE_BUILTINS: &[&str] = &[
    "_http_agent",
    "_http_client",
    "_http_common",
    "_http_incoming",
    "_http_outgoing",
    "_http_server",
    "_stream_duplex",
    "_stream_passthrough",
    "_stream_readable",
    "_stream_transform",
    "_stream_wrap",
    "_stream_writable",
    "_tls_common",
    "_tls_wrap",
    "assert",
    "assert/strict",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "dns/promises",
    "domain",
    "events",
    "fs",
    "fs/promises",
    "http",
    "http2",
    "https",
    "inspector",
    "inspector/promises",
    "module",
    "net",
    "os",
    "path",
    "path/posix",
    "path/win32",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "readline/promises",
    "repl",
    "stream",
    "stream/consumers",
    "stream/promises",
    "stream/web",
    "string_decoder",
    "sys",
    "timers",
    "timers/promises",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "util/types",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

const PREFIX_ONLY_BUILTINS: &[&str] = &["sea", "sqlite", "test", "test/reporters"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_bare_and_prefix_only_builtins() {
        let catalog = BuiltinCatalog;
        assert_eq!(catalog.resolve("fs/promises").as_deref(), Some("node:fs/promises"));
        assert_eq!(catalog.resolve("node:fs/promises").as_deref(), Some("node:fs/promises"));
        assert_eq!(catalog.resolve("node:test").as_deref(), Some("node:test"));
        assert_eq!(catalog.resolve("node:test/reporters").as_deref(), Some("node:test/reporters"));
        assert_eq!(catalog.resolve("node:sqlite").as_deref(), Some("node:sqlite"));
        assert_eq!(catalog.resolve("node:sea").as_deref(), Some("node:sea"));
        assert_eq!(catalog.resolve("test"), None);
        assert_eq!(catalog.resolve("sqlite"), None);
        assert_eq!(catalog.resolve("node:test/unknown"), None);
    }
}
