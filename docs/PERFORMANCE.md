# Performance and regression policy

WAE has two Criterion suites:

- `wae-graph/graph_scaling` measures graph construction at 1k, 10k, 50k and 100k modules and
  SCC, reachability and estimated owned heap capacity at 1k, 10k and 50k.
- `wae-engine/incremental` measures cold, warm and single-edit complete-engine analysis at 1k and
  10k modules. Single-edit runs must restore every unchanged module and analyze exactly one overlay.

Run the complete measurement suite with:

```bash
cargo bench -p wae-graph --bench graph_scaling --locked
cargo bench -p wae-engine --bench incremental --locked
```

Criterion results belong in local/CI artifacts, not source control: processor model, runner load and
toolchain affect absolute latency. Every benchmark target is compiled by CI to prevent silent rot.

CI runs five complete 10,000-module cold, warm and single-edit samples and gates their median. It
checks both tight absolute budgets and relative envelopes against the checked-in latest-release
baseline in `performance/baselines/`. A 50,000-module full-engine gate runs on every change; a
scheduled/manual workflow records the equivalent 100,000-module cold, warm, edit and peak-RSS
contract. All results are retained as performance artifacts.

`wae check --verbose` includes a per-rule profile (nanoseconds and emitted diagnostic count), so
slow rules can be attributed rather than hidden inside an aggregate rules bucket. Cache module
shards are loaded on demand; discovered file membership avoids per-dependency filesystem stats;
semantic graph hashing streams directly into the hasher without constructing a project-sized JSON
buffer.

## Required CI gate

The release-mode gate constructs a 100,000-module chain, computes all strongly connected
components and checks the graph's deterministic capacity-based heap estimate. Defaults:

- graph construction: less than 15 seconds;
- SCC: less than 15 seconds;
- directly owned estimated graph heap: less than 256 MiB.

```bash
cargo test -p wae-graph --release --locked \
  tests::graph_100k_performance_gate -- --ignored --exact
```

`WAE_GRAPH_BUDGET_MS` can lower the time budget on dedicated benchmark hardware, but pull-request
CI must not raise it. The heap estimate is intentionally not advertised as process RSS: it covers
the capacities and string buffers directly owned by `ModuleGraph`, making allocation-complexity
regressions deterministic across runs.

## Incremental acceptance contract

A warm analysis is valid only when its diagnostics are byte-for-byte equivalent to the cold
analysis. Source hashes invalidate individual module snapshots. Config, package manifests,
ts/jsconfig and framework configuration invalidate the resolution environment. Deleted resolution
targets and newly satisfiable candidate paths invalidate their importers. A rule snapshot is reused
only when the full semantic graph identity is unchanged.
