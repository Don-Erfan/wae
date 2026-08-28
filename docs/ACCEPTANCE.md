# Product acceptance fixtures

The acceptance suite uses source code as input; no fixture injects diagnostics. Together the
fixtures form one deterministic synthetic product matrix:

| Contract | Fixture / test evidence |
|---|---|
| Parser and resolver failures | `broken`, `resolution-matrix` |
| Module cycles and layers | `circular`, `layers` |
| Feature/public/private boundaries | `features` and engine feature-boundary tests |
| ARCH-006..010 and PACKAGE-001..004 | `policies` |
| Next.js App/Pages semantics | `consumer-next` |
| RUNTIME-001..006 | `runtime` |
| 12-package production workspace | `monorepo-12` |
| Nx and Turborepo consumer layouts | `nx-workspace`, `turbo-workspace` |
| 500-module cold/warm/fault injection | `real_world_scale` integration test |

The large scenario creates a 500-module Next.js-style dependency chain, verifies a clean cold run,
requires an equivalent warm run restoring every module and the rule snapshot, then injects a cycle
by editing one leaf. Exactly one module may be reanalyzed, 499 must be restored, and the resulting
501-node closed cycle path must be reported.

Run it explicitly:

```bash
cargo test -p wae-engine --release --locked \
  --test real_world_scale \
  five_hundred_module_next_project_supports_warm_analysis_and_fault_injection \
  -- --ignored --exact
```
