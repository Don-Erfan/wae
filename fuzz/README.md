# Fuzzing

Install `cargo-fuzz`, then run either target from the repository root:

```bash
cargo fuzz run parser_imports -- -max_total_time=300
cargo fuzz run config_yaml -- -max_total_time=300
```

CI compiles both targets and runs bounded smoke sessions; longer corpora belong in scheduled jobs.
