# WAE — Engineering Roadmap (Phase 0 → Phase 40)

این roadmap مرجع اجرای پروژه است. اصل اجرایی:

1. بازنویسی از صفر ممنوع
2. هر فاز به‌صورت audit در برابر Definition of Done بررسی می‌شود
3. اگر فاز پاس بود به فاز بعد برو، اگر نه فقط gap همان فاز بسته شود

## Milestoneها

### MVP (Phase 0 تا 15)

- Product contract, workspace foundation, IR/diagnostics
- discovery/parser/resolver/graphs
- config/rules/rule-engine
- suppression+baseline, CLI v1, git-aware regression

### Beta (Phase 16 تا 29)

- monorepo production support
- framework adapter + Next.js + runtime model/rules
- incremental/cache/perf/debugging
- LSP + VS Code + JetBrains + GitHub Action

### v1 (Phase 30 تا 40)

- machine-readable APIs + MCP
- reliability hardening + compatibility matrix
- docs کامل + dogfooding
- synthetic + real-world validation + false-positive audit
- release gate

## فازها و معیارهای Done

1. **Phase 0 — Product Contract**: خروجی‌های `PRODUCT.md`, `ARCHITECTURE.md`, `ROADMAP.md`, `COMPATIBILITY.md` کامل و بدون ابهام detect/delegate.
2. **Phase 1 — Rust Workspace Foundation**: `cargo fmt --check`, `cargo clippy`, `cargo test` سبز.
3. **Phase 2 — Core Domain / IR**: ruleها بدون وابستگی به parser، امکان ساخت graph مصنوعی و اجرای rule.
4. **Phase 3 — Error & Diagnostics**: تفکیک `Parse/Resolution/Config/Internal` و خروجی deterministic.
5. **Phase 4 — Project Discovery**: مدل درست برای single-package و monorepo root.
6. **Phase 5 — JS/TS Parser**: matrix syntaxهای import/export/require/dynamic برای JS/JSX/TS/TSX.
7. **Phase 6 — Resolver**: matrix مستقل برای relative/alias/package/subpath/ESM/CJS.
8. **Phase 7 — Module Graph**: reachability/SCC/cycle/path دقیق.
9. **Phase 8 — Package Graph**: monorepo fixture تحلیل‌شده با dependency policy.
10. **Phase 9 — Architecture Model**: دسته‌بندی deterministic moduleها از config معتبر.
11. **Phase 10 — Config Engine**: schema versioned + validation خوانا (بدون panic).
12. **Phase 11 — Core Rules**: هر rule fixture مثبت/منفی + false-positive tests.
13. **Phase 12 — Rule Engine**: enable/disable, severity override, ordering deterministic, filtering/suppression.
14. **Phase 13 — Suppression & Baseline**: existing pass / new fail با identity پایدار violation.
15. **Phase 14 — CLI v1**: commands/options/exit-codeهای استاندارد CI.
16. **Phase 15 — Git Regression**: `check --changed` روی branch/diff واقعی.
17. **Phase 16 — Monorepo Production Support**: fixture واقعی 10+ package.
18. **Phase 17 — Framework Adapter System**: core بدون شاخه framework-specific.
19. **Phase 18 — Next.js Adapter**: fixtureهای واقعی App/Pages/runtime conventions.
20. **Phase 19 — Runtime Model**: explainable runtime dependency path.
21. **Phase 20 — Runtime Rules**: direct + transitive violations covered.
22. **Phase 21 — Architecture Discovery**: `wae init` پیشنهاد معماری معقول ارائه دهد.
23. **Phase 22 — Cache & Incremental**: تغییر یک فایل full parse ایجاد نکند.
24. **Phase 23 — Performance**: benchmark regression در CI قابل مشاهده.
25. **Phase 24 — Observability/Debugging**: ابزار explain/debug برای bug reportها کافی.
26. **Phase 25 — LSP**: editor مستقل diagnostics قابل اتکا دریافت کند.
27. **Phase 26 — VS Code Extension**: live diagnostics + quick fixes.
28. **Phase 27 — JetBrains Plugin**: parity رفتاری با VS Code.
29. **Phase 28 — Architecture Explorer**: UI صرفاً consumer داده core، بدون logic duplication.
30. **Phase 29 — GitHub Action**: PR خراب fail + annotations.
31. **Phase 30 — Machine-readable API**: schema versioned JSON/JSONL پایدار.
32. **Phase 31 — MCP Integration**: agent بدون parse متن CLI به مدل معماری دسترسی بگیرد.
33. **Phase 32 — Reliability Hardening**: zero known panic paths روی ورودی‌های رایج.
34. **Phase 33 — Compatibility Matrix**: ماتریس پشتیبانی شفاف و sync با CI.
35. **Phase 34 — Documentation**: onboarding کامل بدون کمک نویسنده ابزار.
36. **Phase 35 — Dogfooding**: repo خودش با ruleهای خودش clean باشد.
37. **Phase 36 — Synthetic Test App**: violationهای عمدی با diagnostics کاملاً predictable.
38. **Phase 37 — Real-world Test Project**: flow کامل CLI + IDE + CI روی پروژه واقعی.
39. **Phase 38 — False Positive Audit**: rule set با signal بالا و noise پایین.
40. **Phase 39/40 — v1 Release Gate**: فقط پس از عبور کامل معیارهای پایداری/سازگاری/کارایی/یکپارچگی.
