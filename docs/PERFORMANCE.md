# Performance and regression policy

WAE has two Criterion suites:

- `wae-graph/graph_scaling` measures graph construction at 1k, 10k, 50k and 100k modules and
  SCC, reachability and estimated owned heap capacity at 1k, 10k and 50k.
- `wae-engine/incremental` measures cold, warm, syntax-only, edge-local and global-cycle edits at
  1k and 10k modules. Every edit profile must restore unchanged modules and analyze only its
  overlay; syntax-only edits must restore every rule partition.

Run the complete measurement suite with:

```bash
cargo bench -p wae-graph --bench graph_scaling --locked
cargo bench -p wae-engine --bench incremental --locked
```

Criterion results belong in local/CI artifacts, not source control: processor model, runner load and
toolchain affect absolute latency. Every benchmark target is compiled by CI to prevent silent rot.

CI runs five complete 10,000-module cold, warm and syntax-only samples plus explicit edge-local and
global-cycle edits. The 50k and scheduled 100k gates exercise the same semantic profiles. It
checks both tight absolute budgets and relative envelopes against the checked-in latest-release
baseline in `performance/baselines/`. A 50,000-module full-engine gate runs on every change; a
scheduled/manual workflow records the equivalent 100,000-module cold, warm, edit and peak-RSS
contract. Artifacts retain raw sample arrays, phase timings, RSS, commit SHA, run URL, runner
image/CPU, Rust version, job conclusion and edit-profile names.

`wae check --verbose` includes a per-rule profile (nanoseconds and emitted diagnostic count), so
slow rules can be attributed rather than hidden inside an aggregate rules bucket. Cache module
shards are loaded on demand; discovered file membership avoids per-dependency filesystem stats;
semantic graph hashing feeds the hasher directly without constructing a project-sized JSON buffer.
Rule results are persisted per rule and keyed by the rule's declared `Edge`, `Closure` or `Global`
semantic input scope. Cache misses are parsed and resolved in bounded parallel batches (at most
eight workers), which improves cold throughput without making thread-stack memory scale with the
host's CPU count.

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
targets and newly satisfiable candidate paths invalidate their importers. Rule partitions are
reused independently when the semantic identity for their declared scope is unchanged; an
environment change invalidates every partition.

The persisted partitions avoid repeated rule work when their semantic input is unchanged. WAE
still rebuilds the immutable module graph after a semantic edge change; incremental SCC and
in-place adjacency deltas remain future work and are not part of the current latency guarantee.
