# Contributing to WAE

Use a focused branch and include tests for every behavioral change. Before opening a pull request, run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Architecture changes should preserve the dependency direction described in `docs/ARCHITECTURE.md`. New rules require a stable rule ID, metadata, positive and negative fixtures, deterministic output, and documentation. Config and output schema changes must follow `docs/COMPATIBILITY.md`.

Do not commit generated binaries, IDE state, caches, secrets, or a baseline created from unreviewed violations.
