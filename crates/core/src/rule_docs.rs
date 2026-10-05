//! Long-form rule documentation.
//!
//! This is the single source for `wae explain`, LSP explanations, the MCP `architecture_explain`
//! tool, SARIF rule help and the generated `docs/RULES.md`. Keep examples minimal and accurate to
//! the engine's real behavior; every rule must have an entry (enforced by tests).

use crate::rule_registry::{RULES, RuleDescriptor};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuleDocumentation {
    /// Why the rule exists: the architectural risk it prevents.
    pub rationale: &'static str,
    /// A minimal violating example.
    pub bad_example: &'static str,
    /// The same intent written so the rule passes.
    pub good_example: &'static str,
    /// Concrete remediation steps.
    pub fix: &'static str,
    /// `wae.yaml` that enables or tunes the rule.
    pub configuration: &'static str,
    /// Known situations where the rule may be noisy and the recommended response.
    pub false_positives: &'static str,
}

pub fn documentation(id: &str) -> Option<&'static RuleDocumentation> {
    DOCUMENTATION.iter().find(|(rule, _)| *rule == id).map(|(_, documentation)| documentation)
}

/// Human-facing default behavior shared by every renderer.
pub fn default_behavior(descriptor: &RuleDescriptor) -> &'static str {
    match descriptor.id {
        "ARCH-001" => "error; enabled even without a wae.yaml",
        "SUPPRESS-001" => {
            "warning; controlled by `suppressions.require_reason` and `report_unused`"
        }
        _ if descriptor.configurable => "error once a wae.yaml exists",
        _ => "error; always enabled",
    }
}

fn language(example: &str) -> &'static str {
    if example.trim_start().starts_with('{') { "json" } else { "ts" }
}

/// Plain-text explanation for terminals (`wae explain`).
pub fn explain_text(descriptor: &RuleDescriptor) -> String {
    let mut output = format!(
        "{} — {}\n{}\n\nCategory: {}\nDefault:  {}\n",
        descriptor.id,
        descriptor.title,
        descriptor.description,
        descriptor.category,
        default_behavior(descriptor)
    );
    if let Some(docs) = descriptor.documentation() {
        let sections = [
            ("Why", docs.rationale),
            ("Bad", docs.bad_example),
            ("Good", docs.good_example),
            ("How to fix", docs.fix),
            ("Configuration", docs.configuration),
            ("False positives", docs.false_positives),
        ];
        for (heading, body) in sections {
            output.push_str(&format!("\n{heading}:\n"));
            for line in body.trim_end().lines() {
                if !line.is_empty() {
                    output.push_str("  ");
                    output.push_str(line);
                }
                output.push('\n');
            }
        }
    }
    output.push_str(&format!("\nReference: {}\n", help_uri(descriptor.id)));
    output
}

/// Markdown body for one rule, without its heading (LSP, MCP, SARIF help).
pub fn markdown_body(descriptor: &RuleDescriptor) -> String {
    let mut output = format!(
        "**{}** — {}\n\n- Category: `{}`\n- Default: {}\n",
        descriptor.title,
        descriptor.description,
        descriptor.category,
        default_behavior(descriptor)
    );
    if let Some(docs) = descriptor.documentation() {
        output.push_str(&format!("\n### Why\n\n{}\n", docs.rationale.trim_end()));
        output.push_str(&format!(
            "\n### Bad\n\n```{}\n{}\n```\n",
            language(docs.bad_example),
            docs.bad_example.trim_end()
        ));
        output.push_str(&format!(
            "\n### Good\n\n```{}\n{}\n```\n",
            language(docs.good_example),
            docs.good_example.trim_end()
        ));
        output.push_str(&format!("\n### How to fix\n\n{}\n", docs.fix.trim_end()));
        output.push_str(&format!(
            "\n### Configuration\n\n```yaml\n{}\n```\n",
            docs.configuration.trim_end()
        ));
        output.push_str(&format!("\n### False positives\n\n{}\n", docs.false_positives.trim_end()));
    }
    output
}

pub fn help_uri(id: &str) -> String {
    format!(
        "https://github.com/Don-Erfan/wae/blob/master/docs/RULES.md#{}",
        id.to_ascii_lowercase()
    )
}

/// The complete generated `docs/RULES.md`.
pub fn reference_markdown() -> String {
    let mut output = String::from(
        "<!-- Generated from crates/core/src/rule_docs.rs. Do not edit by hand; run\n     `WAE_UPDATE_DOCS=1 cargo test -p wae-core rules_reference_is_generated` to refresh. -->\n\n# WAE rule reference\n\nEvery rule has a stable ID that never changes meaning within a major version. `wae explain\n<RULE_ID>` prints the same text in the terminal, and editors show it from the language server.\n\nWithout a `wae.yaml`, only `ARCH-001` and the correctness diagnostics run. Once a configuration\nexists every configurable rule defaults to `error`; set a rule to `warning`, `info` or `off`, or\nuse `overrides` for path-specific rollout. Suppress an accepted exception with a documented\nreason (`// wae-ignore RULE-ID -- reason`) rather than disabling the rule.\n\n| Rule | Title | Category |\n|---|---|---|\n",
    );
    for rule in RULES {
        output.push_str(&format!(
            "| [{}](#{}) | {} | {} |\n",
            rule.id,
            rule.id.to_ascii_lowercase(),
            rule.title,
            rule.category
        ));
    }
    for rule in RULES {
        output.push_str(&format!("\n## {}\n\n{}", rule.id, markdown_body(rule)));
    }
    output
}

static DOCUMENTATION: &[(&str, RuleDocumentation)] = &[
    (
        "ARCH-001",
        RuleDocumentation {
            rationale: "A dependency cycle means no module in it can be understood, tested, reused or extracted on its own.\nIn ES modules and CommonJS, cycles also cause initialization-order bugs: a binding may still be\n`undefined` (or in its temporal dead zone) when the other module reads it. Cycles tend to grow\nbecause every new edge inside the strongly connected component is \"free\". WAE reports one\ndiagnostic per strongly connected component with a closed, shortest representative path.",
            bad_example: "// src/cart/cart.ts\nimport { formatPrice } from \"../pricing/price\";\nexport const total = (items: Item[]) => items.map(formatPrice);\n\n// src/pricing/price.ts\nimport { total } from \"../cart/cart\"; // closes the cycle\nexport const formatPrice = (item: Item) => `${item.price} of ${total([item])}`;",
            good_example: "// src/pricing/price.ts — depends on nothing in cart\nexport const formatPrice = (item: Item) => `${item.price}`;\n\n// src/cart/cart.ts\nimport { formatPrice } from \"../pricing/price\";\nexport const total = (items: Item[]) => items.map(formatPrice);",
            fix: "Read the reported path and find the edge that points \"upward\" (from a lower-level module to a\nhigher-level one). Remove it by extracting the shared code into a new lower-level module, by\npassing the dependency in as a parameter, or by introducing a small interface (port) owned by\nthe lower module. `wae graph --module <file>` lists both directions for any module on the path.",
            configuration: "rules:\n  ARCH-001: error   # or warning / off\n  # Also report cycles closed only by `import type` (design-time coupling):\n  # ARCH-001:\n  #   include_type_only: true",
            false_positives: "By default a cycle closed only by `import type` / `export type` is not reported, because\nTypeScript erases those imports and the cycle never executes; set `include_type_only: true` to\nreport design-time cycles too. Barrel files (`index.ts` re-exporting everything) often create\ncycles that disappear when importers use the concrete module. Accept an intentional cycle with\na fingerprint suppression in `wae.yaml`.",
        },
    ),
    (
        "ARCH-002",
        RuleDocumentation {
            rationale: "Some dependencies are wrong regardless of layers: UI importing a database client, a shared\nlibrary importing an application, test utilities leaking into production code. `ARCH-002`\nturns those decisions into explicit, reviewable policy instead of tribal knowledge.",
            bad_example: "// src/components/user-card.tsx\nimport { db } from \"../server/db\"; // forbidden: components -> server",
            good_example: "// src/components/user-card.tsx\nimport type { User } from \"../entities/user\";\nexport function UserCard({ user }: { user: User }) { /* ... */ }",
            fix: "Route the dependency through an allowed module (for example a Server Action, API client or\napplication service), or, if the policy is outdated, change the policy in `wae.yaml` in the same\nreviewed pull request.",
            configuration: "architecture:\n  forbidden_dependencies:\n    - from: \"src/components/**\"\n      to: \"src/server/**\"\n  presets:\n    monorepo_boundaries: true   # also forbid packages/** -> apps/**",
            false_positives: "Globs match project-relative module paths with literal `/` separators, so `src/*` does not\nmatch nested files; use `src/**`. The rule never fires without a configured policy or preset.",
        },
    ),
    (
        "ARCH-003",
        RuleDocumentation {
            rationale: "Layered architectures (for example app → features → entities → shared) only stay layered if\nthe import direction is enforced. Without it, lower layers start importing higher ones and the\nlayers collapse into one tangled module.",
            bad_example: "# wae.yaml: shared may not import features\n// src/shared/ui/button.tsx\nimport { useCart } from \"../../features/cart\"; // shared -> features",
            good_example: "// src/features/cart/ui/add-button.tsx\nimport { Button } from \"../../../shared/ui/button\"; // features -> shared is allowed\nimport { useCart } from \"../model\";",
            fix: "Move the code to the layer that is allowed to depend on both sides, invert the dependency with\na callback or prop, or add the target layer to `canImport` if the architecture genuinely\nchanged. Imports within the same layer are always allowed.",
            configuration: "architecture:\n  layers:\n    app:      { patterns: [\"src/app/**\"],      canImport: [features, entities, shared] }\n    features: { patterns: [\"src/features/**\"], canImport: [entities, shared] }\n    entities: { patterns: [\"src/entities/**\"], canImport: [shared] }\n    shared:   { patterns: [\"src/shared/**\"],   canImport: [] }",
            false_positives: "Modules outside every layer are ignored by this rule (see `ARCH-010`). A module matching two\nlayers is a configuration error, not a diagnostic; run `wae config validate --show-overlaps`\nand anchor broad patterns such as `**/shared/**` to `src/shared/**`.",
        },
    ),
    (
        "ARCH-004",
        RuleDocumentation {
            rationale: "A feature's public entrypoint (`index.ts`) is its contract. Importing files behind it couples\nthe importer to implementation details, so the owning team cannot refactor without breaking\nother features.",
            bad_example: "// src/features/checkout/pay.ts\nimport { sessionStore } from \"../user/model/session\"; // reaches into feature `user`",
            good_example: "// src/features/user/index.ts\nexport { getSession } from \"./model/session\";\n\n// src/features/checkout/pay.ts\nimport { getSession } from \"../user\";",
            fix: "Export what the importer needs from the target feature's public entrypoint and import that\nentrypoint instead. If the importer needs a lot of internals, the code probably belongs in the\ntarget feature or in a lower shared layer.",
            configuration: "architecture:\n  features:\n    roots: [\"src/features\"]\n    public_entrypoints: [\"index.ts\", \"index.tsx\"]\nrules:\n  ARCH-004: error",
            false_positives: "Files inside the same feature may import each other freely. Code outside every feature (for\nexample `src/app`) is still an importer and must use the entrypoint. Add extra public files such\nas `public-api.ts` to `public_entrypoints` instead of suppressing.",
        },
    ),
    (
        "ARCH-005",
        RuleDocumentation {
            rationale: "Directories named `internal` or `private` declare code that is not part of any contract.\n`ARCH-005` makes that naming convention enforceable, even outside configured features.",
            bad_example: "// src/features/checkout/pay.ts\nimport { tokenCache } from \"../user/internal/token-cache\";",
            good_example: "// src/features/user/index.ts\nexport { refreshToken } from \"./internal/token-cache\";\n\n// src/features/checkout/pay.ts\nimport { refreshToken } from \"../user\";",
            fix: "Import the owner's public entrypoint, or move the shared helper out of the private directory\ninto a module that is meant to be shared.",
            configuration: "architecture:\n  features:\n    private_segments: [internal, private]\nrules:\n  ARCH-005: error",
            false_positives: "A path segment must equal a configured private segment exactly; `internals` or `private-api`\ndo not match. Imports from inside the owning feature are allowed.",
        },
    ),
    (
        "ARCH-006",
        RuleDocumentation {
            rationale: "Long dependency chains from an entrypoint (page, route, CLI) make start-up cost, bundle size\nand change impact hard to reason about. A depth budget surfaces chains that keep growing.",
            bad_example: "# max_depth: 3\n// src/app/page.tsx -> features/a -> features/b -> entities/c -> shared/d (depth 4)",
            good_example: "// src/app/page.tsx -> features/a -> shared/d (depth 2)",
            fix: "Flatten the chain: let the entrypoint compose features directly, remove pass-through modules,\nor move deep helpers closer to their callers.",
            configuration: "rules:\n  ARCH-006:\n    severity: warning\n    max_depth: 8\n    entrypoints: [\"src/app/**/page.tsx\", \"src/pages/**/*.tsx\"]",
            false_positives: "Depth is the shortest path from any configured entrypoint, so a module reachable through both\na short and a long chain is measured by the short one. Without `max_depth` the rule emits nothing.",
        },
    ),
    (
        "ARCH-007",
        RuleDocumentation {
            rationale: "A module that imports many distinct modules (high fan-out) usually mixes responsibilities and\nchanges whenever any of its dependencies change.",
            bad_example: "# max_fan_out: 3\n// src/app/dashboard.tsx imports 12 different feature and entity modules",
            good_example: "// src/app/dashboard.tsx imports one composed widget per area\nimport { SalesWidget } from \"../features/sales\";\nimport { UsersWidget } from \"../features/users\";",
            fix: "Split the module by responsibility or introduce a composition module per area.",
            configuration: "rules:\n  ARCH-007:\n    severity: warning\n    max_fan_out: 20",
            false_positives: "Multiple imports of the same module count once. Barrel and composition-root files are\nexpected to have high fan-out; exempt them with a path override set to `off`.",
        },
    ),
    (
        "ARCH-008",
        RuleDocumentation {
            rationale: "A module that very many modules depend on (high fan-in) is a change hotspot: every edit risks\nbreaking a large part of the codebase.",
            bad_example: "# max_fan_in: 2\n// src/utils/index.ts is imported by 140 modules and keeps growing",
            good_example: "// small, focused modules: src/shared/lib/format-date.ts, src/shared/lib/money.ts",
            fix: "Split the hotspot into smaller cohesive modules, or keep it deliberately stable and exempt it.",
            configuration: "rules:\n  ARCH-008:\n    severity: warning\n    max_fan_in: 50",
            false_positives: "Stable foundations (design-system primitives, type definitions) legitimately have high fan-in;\nexempt them with `overrides` rather than raising the global threshold.",
        },
    ),
    (
        "ARCH-009",
        RuleDocumentation {
            rationale: "Source modules that no entrypoint can reach are dead code or forgotten migrations. They still\ncost review, type-checking and maintenance time.",
            bad_example: "# entrypoints: [\"src/app/**/page.tsx\"]\n// src/features/legacy-banner/index.ts — nothing imports it",
            good_example: "// delete the module, or import it from a page/route that should use it",
            fix: "Delete unreachable code, or add the missing entrypoint (for example a script or route) to\n`entrypoints`.",
            configuration: "rules:\n  ARCH-009:\n    severity: warning\n    entrypoints: [\"src/app/**/page.tsx\", \"src/pages/**/*.tsx\", \"scripts/*.ts\"]",
            false_positives: "Framework files loaded by convention (middleware, instrumentation, config files) and files\nloaded through non-static mechanisms are unreachable unless listed as entrypoints. The rule never\nguesses entrypoints and emits nothing until at least one is configured.",
        },
    ),
    (
        "ARCH-010",
        RuleDocumentation {
            rationale: "Layer rules only protect modules that belong to a layer. `ARCH-010` makes sure new directories\ndo not silently escape the architecture.",
            bad_example: "# layers cover src/app, src/features and src/shared\n// src/helpers/date.ts — belongs to no layer",
            good_example: "// src/shared/lib/date.ts — owned by the shared layer",
            fix: "Move the module under an existing layer, add a pattern to the right layer, or list the path in\n`architecture.coverage.allow_unassigned` when it is intentionally outside the architecture.",
            configuration: "architecture:\n  coverage:\n    allow_unassigned: [\"scripts/**\", \"generated/**\"]\nrules:\n  ARCH-010: warning",
            false_positives: "The rule is silent when no layers are configured. Use `wae config validate --show-coverage\n--show-unassigned` to review the full list before enabling it as an error.",
        },
    ),
    (
        "ARCH-011",
        RuleDocumentation {
            rationale: "Strict per-module ownership (`ARCH-010`) can be too much on day one. `ARCH-011` enforces an\naggregate coverage floor so adoption can ratchet upward.",
            bad_example: "# coverage.minimum: 90\n# 70% of non-exempt source modules belong to exactly one layer",
            good_example: "# 92% assigned: passes, and the minimum can be raised over time",
            fix: "Assign more modules to layers or exempt intentional areas; then raise `minimum`.",
            configuration: "architecture:\n  coverage:\n    minimum: 80\nrules:\n  ARCH-010: off      # threshold-only adoption\n  ARCH-011: error",
            false_positives: "Exempt (`allow_unassigned`) modules are excluded from the percentage. The rule emits nothing\nwithout `coverage.minimum`.",
        },
    ),
    (
        "PACKAGE-001",
        RuleDocumentation {
            rationale: "A cycle between workspace packages makes independent versioning, building and publishing\nimpossible and usually breaks task-graph tools such as Turborepo and Nx.",
            bad_example: "// packages/ui/src/button.tsx\nimport { track } from \"@acme/analytics\";\n// packages/analytics/src/index.ts\nimport { Button } from \"@acme/ui\"; // @acme/ui <-> @acme/analytics",
            good_example: "// move the shared contract into a lower package both depend on\nimport { type TrackEvent } from \"@acme/contracts\";",
            fix: "Move the shared contract into a lower-level package, or invert one dependency through a port\nthat the higher package implements.",
            configuration: "rules:\n  PACKAGE-001: error",
            false_positives: "Only declared workspace packages participate; external npm packages never form package cycles.",
        },
    ),
    (
        "PACKAGE-002",
        RuleDocumentation {
            rationale: "Package direction policies (shared packages never import apps, UI never imports server\npackages) keep a monorepo layered at the package level.",
            bad_example: "# from: \"@acme/ui-*\" to: \"@acme/app-*\"\n// packages/ui-kit/src/header.tsx\nimport { routes } from \"@acme/app-web\";",
            good_example: "// pass routes in from the app instead\nexport function Header({ routes }: { routes: Route[] }) { /* ... */ }",
            fix: "Invert the dependency (props, callbacks, dependency injection) or move the code into a package\nthat is allowed to depend on both.",
            configuration: "architecture:\n  forbidden_package_dependencies:\n    - from: \"@acme/ui-*\"\n      to: \"@acme/app-*\"",
            false_positives: "Patterns match package names, not paths. The rule never fires without a configured policy.",
        },
    ),
    (
        "PACKAGE-003",
        RuleDocumentation {
            rationale: "Importing a workspace package that is not declared in `package.json` works locally through\nhoisting but breaks isolated installs, pruned Docker builds and publishing.",
            bad_example: "// apps/web/src/page.tsx\nimport { Button } from \"@acme/ui\";\n// apps/web/package.json has no \"@acme/ui\" entry",
            good_example: "// apps/web/package.json\n{ \"dependencies\": { \"@acme/ui\": \"workspace:*\" } }",
            fix: "Add the package to `dependencies`, `devDependencies`, `peerDependencies` or\n`optionalDependencies` of the importing package.",
            configuration: "rules:\n  PACKAGE-003: error",
            false_positives: "Only cross-workspace imports are checked; third-party packages are left to your package\nmanager and tools such as `depcheck`.",
        },
    ),
    (
        "PACKAGE-004",
        RuleDocumentation {
            rationale: "A relative import such as `../../packages/ui/src/button` bypasses the target package's\n`exports`, its build and its dependency declaration.",
            bad_example: "// apps/web/src/page.tsx\nimport { Button } from \"../../../packages/ui/src/button\";",
            good_example: "// apps/web/src/page.tsx\nimport { Button } from \"@acme/ui\";",
            fix: "Import the package by name and expose what you need through its `exports` entrypoints.",
            configuration: "rules:\n  PACKAGE-004: error",
            false_positives: "Relative imports within one package are always allowed.",
        },
    ),
    (
        "RUNTIME-001",
        RuleDocumentation {
            rationale: "Code that reaches the browser must never pull in server-only modules: it leaks secrets and\nserver logic into the client bundle or fails the build. The violation is often indirect, so WAE\nreports the shortest transitive path.",
            bad_example: "// src/app/profile/client.tsx\n\"use client\";\nimport { getUser } from \"../../lib/user\"; // lib/user imports \"server-only\" db code",
            good_example: "// src/app/profile/actions.ts\n\"use server\";\nexport async function getUser() { /* db access */ }\n\n// src/app/profile/client.tsx\n\"use client\";\nimport { getUser } from \"./actions\"; // Server Action = explicit RPC boundary",
            fix: "Follow the reported path to the first server-only module and cut the edge before it: call the\nserver through a Server Action, a route handler or an API client, and keep shared modules free\nof server imports.",
            configuration: "rules:\n  RUNTIME-001: error",
            false_positives: "Type-only imports are ignored. Modules are server-only through `import \"server-only\"`, Next.js\nconventions or an explicit runtime export; `wae graph --module <file>` shows why a module got its\nruntime. Server Action modules (`\"use server\"`) stop propagation by design.",
        },
    ),
    (
        "RUNTIME-002",
        RuleDocumentation {
            rationale: "Browser bundles cannot use Node APIs (`fs`, `child_process`, `node:*`). Reaching them through a\nshared helper fails at build time or ships broken polyfills.",
            bad_example: "\"use client\";\nimport { readConfig } from \"../lib/config\"; // lib/config imports \"node:fs\"",
            good_example: "\"use client\";\nimport type { Config } from \"../lib/config-types\"; // read the file on the server and pass data down",
            fix: "Split the Node implementation from its browser-safe contract, and load the data on the server.",
            configuration: "rules:\n  RUNTIME-002: error",
            false_positives: "Node classification comes from Node builtins and explicit `runtime = \"nodejs\"` exports. Type-only\nimports never count.",
        },
    ),
    (
        "RUNTIME-003",
        RuleDocumentation {
            rationale: "Some npm packages only work on the server (native addons, database drivers, SDKs holding\nsecrets). Listing them makes accidental client imports a build-time error.",
            bad_example: "# browser_incompatible_packages: [\"pg\", \"@acme/server-*\"]\n\"use client\";\nimport { query } from \"../lib/db\"; // lib/db imports \"pg\"",
            good_example: "// keep pg behind a Server Action or route handler and call that from the client",
            fix: "Replace the package with a browser-safe client, or keep it behind a server boundary.",
            configuration: "runtime:\n  browser_incompatible_packages: [\"node:*\", \"pg\", \"@acme/server-*\"]\nrules:\n  RUNTIME-003: error",
            false_positives: "The list is opt-in; an empty list produces no diagnostics. Keep entries specific to packages that\nreally require server APIs.",
        },
    ),
    (
        "RUNTIME-004",
        RuleDocumentation {
            rationale: "Edge runtimes (Next.js middleware, `runtime = \"edge\"` routes) do not provide Node APIs. A\ntransitive Node import fails only at deploy time unless it is caught statically.",
            bad_example: "// src/middleware.ts (Edge)\nimport { verify } from \"./lib/auth\"; // lib/auth imports \"node:crypto\"",
            good_example: "// src/middleware.ts\nimport { verify } from \"./lib/auth-edge\"; // uses Web Crypto (crypto.subtle)",
            fix: "Use an Edge-compatible implementation (Web APIs), or move the work to a Node runtime route.",
            configuration: "runtime:\n  edge_incompatible_packages: [\"node:*\", \"*-native\"]\nrules:\n  RUNTIME-004: error",
            false_positives: "Middleware is Edge by convention; a route is Edge only with an explicit `runtime = \"edge\"` export.",
        },
    ),
    (
        "RUNTIME-005",
        RuleDocumentation {
            rationale: "A \"universal\" module that transitively needs both browser and server capabilities cannot run\ncorrectly anywhere. It is usually a shared file that grew imports from both sides.",
            bad_example: "// src/lib/session.ts (no runtime marker)\nimport { useSyncExternalStore } from \"./client-store\"; // browser\nimport { cookies } from \"./server-cookies\";             // server",
            good_example: "// src/lib/session-types.ts      — shared contract\n// src/lib/session.client.ts     — browser implementation\n// src/lib/session.server.ts     — server implementation",
            fix: "Split the module at the boundary the diagnostic reports: a runtime-neutral contract plus one\nimplementation per runtime.",
            configuration: "rules:\n  RUNTIME-005: error",
            false_positives: "Only declared browser boundaries (`\"use client\"`, `client-only`, an explicit runtime) count as\nbrowser requirements; modules that are merely bundled for the browser because some client\ncomponent imports them do not. The diagnostic includes both paths as evidence; if a module was\nmisclassified, fix its marker rather than suppressing.",
        },
    ),
    (
        "RUNTIME-006",
        RuleDocumentation {
            rationale: "A cycle that joins browser and server (or Edge and Node) code cannot be split into valid\nbundles; every module in it inherits both runtimes.",
            bad_example: "// client.tsx (\"use client\") -> server.ts -> client.tsx",
            good_example: "// client.tsx -> contract.ts <- server.ts (no cycle across the boundary)",
            fix: "Break the cycle at a runtime-neutral contract or an explicit transport boundary such as a\nServer Action.",
            configuration: "rules:\n  RUNTIME-006: error",
            false_positives: "Cycles through Server Action (RPC) boundaries are excluded. A plain cycle is also reported as\n`ARCH-001`; fixing the cycle resolves both.",
        },
    ),
    (
        "PARSE-001",
        RuleDocumentation {
            rationale: "WAE cannot see the imports of a file it cannot parse, so the architecture model would be\nincomplete. The file is reported instead of silently skipped.",
            bad_example: "// src/broken.ts\nexport const value = ;",
            good_example: "// src/broken.ts\nexport const value = 1;",
            fix: "Fix the syntax error (your compiler reports the details), or exclude generated or intentionally\ninvalid files with `project.exclude`.",
            configuration: "project:\n  exclude: [\"**/fixtures/invalid/**\"]",
            false_positives: "Tree-sitter accepts all standard JS/TS/JSX/TSX syntax. Experimental syntax not yet supported by\nthe grammar may be reported; exclude those files and open an issue.",
        },
    ),
    (
        "RESOLVE-001",
        RuleDocumentation {
            rationale: "An unresolved relative or aliased import is a broken edge in the architecture graph: rules\ncannot evaluate what they cannot see, and the import usually fails at build time too.",
            bad_example: "import { Button } from \"@/components/buton\"; // typo",
            good_example: "import { Button } from \"@/components/button\";",
            fix: "Fix the path, or teach WAE the alias through `tsconfig.json`/`jsconfig.json` `paths`. Run\n`wae resolve <importer> <specifier>` to see every resolver step that was tried.",
            configuration: "# aliases come from tsconfig.json / jsconfig.json; conditions and generated modules from:\nresolution:\n  mode: bundler\n  custom_conditions: [browser]\n  virtual_modules: [\"contentlayer/generated\"]   # created by a build step",
            false_positives: "Bare npm package imports are treated as external and are never reported. Bundler-only aliases\n(webpack/Vite `resolve.alias`) must be mirrored in tsconfig `paths`. Imports of code generated by\na build step (contentlayer, codegen output) fail before that step runs: run WAE after generation,\nor list the specifiers in `resolution.virtual_modules` to treat them as opaque modules.",
        },
    ),
    (
        "RESOLVE-002",
        RuleDocumentation {
            rationale: "Absolute filesystem specifiers and similar forms escape the project analysis boundary and make\nresults depend on the machine running the analysis.",
            bad_example: "import config from \"/Users/alice/project/src/config\";",
            good_example: "import config from \"@/config\"; // or a relative path",
            fix: "Use a relative path, a tsconfig alias or a package name.",
            configuration: "# not configurable",
            false_positives: "None known; URL-style and absolute specifiers are intentionally rejected.",
        },
    ),
    (
        "SUPPRESS-001",
        RuleDocumentation {
            rationale: "Suppressions without a reason, for unknown rules, or that no longer match anything hide real\nproblems and rot over time.",
            bad_example: "// wae-ignore ARCH-003\nimport { legacy } from \"../legacy\"; // no reason given",
            good_example: "// wae-ignore ARCH-003 -- legacy adapter; remove after ARC-142\nimport { legacy } from \"../legacy\";",
            fix: "Add a concrete reason after `--`, correct the rule ID, or delete the unused directive. `wae\nsuppressions validate` and `wae suppressions prune` audit config-level suppressions.",
            configuration: "suppressions:\n  require_reason: true\n  report_unused: true",
            false_positives: "A directive applies to its own line or the next line only; move it directly above the import.",
        },
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_rule_is_documented_completely() {
        for rule in RULES {
            let docs = rule.documentation().unwrap_or_else(|| panic!("{} lacks docs", rule.id));
            for (field, value) in [
                ("rationale", docs.rationale),
                ("bad_example", docs.bad_example),
                ("good_example", docs.good_example),
                ("fix", docs.fix),
                ("configuration", docs.configuration),
                ("false_positives", docs.false_positives),
            ] {
                assert!(!value.trim().is_empty(), "{}.{field} is empty", rule.id);
            }
            let text = explain_text(rule);
            assert!(text.contains("Bad:") && text.contains("Good:") && text.contains("Why:"));
        }
        assert_eq!(DOCUMENTATION.len(), RULES.len(), "documentation for unregistered rule");
    }

    #[test]
    fn rules_reference_is_generated() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/RULES.md");
        let generated = reference_markdown();
        if std::env::var_os("WAE_UPDATE_DOCS").is_some() {
            std::fs::write(path, &generated).unwrap();
        }
        let checked_in = std::fs::read_to_string(path).unwrap().replace("\r\n", "\n");
        assert!(
            checked_in == generated,
            "docs/RULES.md is stale; run `WAE_UPDATE_DOCS=1 cargo test -p wae-core rules_reference_is_generated`"
        );
        for rule in RULES {
            assert!(generated.contains(&format!("\n## {}\n", rule.id)), "{} anchor", rule.id);
        }
    }
}
