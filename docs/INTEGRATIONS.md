# MCP, Explorer and reusable CI integration

All integrations are adapters over `wae-engine`; none parse source, resolve imports, rebuild a
graph, or evaluate rules independently.

The stable JSON reporter contract is published as `schemas/diagnostics.schema.json`; the strict
configuration/autocomplete contract is `schemas/wae.schema.json`. Both use explicit schema/version
identities and are parsed or registry-synchronized in the Rust test suite.

## MCP server

`wae-mcp` is a stdio JSON-RPC server implementing MCP protocol version `2025-06-18`. It exposes:

- `architecture_check`: versioned diagnostics and analysis timings;
- `architecture_explain`: stable rule metadata;
- `dependency_path`: the deterministic shortest path between two resolved modules;
- `architecture_model`: modules, packages, layers, runtimes, framework metadata, edges and
  diagnostics.
- `dependency_policy`: whether an existing resolved edge is allowed and the diagnostics governing it.

After installing the npm package, configure an MCP client to run the project-local executable:

```json
{
  "mcpServers": {
    "wae": {
      "command": "npx",
      "args": ["--no-install", "wae-mcp"],
      "cwd": "/absolute/path/to/project"
    }
  }
}
```

The server is confined to its startup directory by default. Add another trusted tree with
`--allow-root /absolute/path`, or use the intentionally explicit `--allow-any-root` only in an
already sandboxed environment. Canonicalization prevents `..` and symlink escapes.
Requests are limited to 1 MiB by default; local deployments can lower the bounded stdio quota with
`--max-request-bytes N`. WAE intentionally exposes no network transport, so authentication belongs
to an explicitly configured remote proxy rather than being silently omitted from a public socket.

Tool execution failures are returned as MCP tool results with `isError: true`; malformed or unknown
JSON-RPC methods use protocol errors. The server writes only protocol messages to stdout.

CI also launches a real VS Code Extension Host against the built `wae-lsp`, opens a TypeScript
workspace and waits for an `ARCH-001` diagnostic. JetBrains packages are compiled and checked with
the IntelliJ Plugin Verifier against the declared Ultimate/LSP platform.

## Architecture Explorer

Generate an offline report from the real resolved graph:

```bash
npx wae explore
npx wae explore --output artifacts/architecture.html
```

The default path is `.wae/explorer.html`. The report embeds its escaped model as
`application/json`, loads no remote resources, and supports module search plus package, layer,
runtime and violation filters. Selecting a module shows inbound/outbound counts, diagnostics and
framework metadata.

## Reusable GitHub Action

The repository root contains a composite `action.yml`. Pin a release tag and an exact npm version:

```yaml
permissions:
  contents: read
  security-events: write

steps:
  - uses: actions/checkout@v4
    with:
      fetch-depth: 0
  - uses: Don-Erfan/wae@v0.0.24
    with:
      version: 0.0.24
      changed: "true"
      base: origin/main
      format: sarif
      upload-sarif: "true"
```

Ratchet mode still requires a reviewed, committed `.wae/baseline.json`; the Action never creates
one implicitly. The Action uploads WAE SARIF through GitHub CodeQL's upload adapter and then
propagates the original WAE exit status, so annotations never hide a failed architecture gate.
