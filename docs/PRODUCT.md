# WAE — Product Contract (Phase 0)

## 1) تعریف یک‌خطی محصول

WAE یک **Web Architecture Compiler / Integrity Engine** است که graph معماری پروژه‌های وب را می‌سازد و
regressionهای معماری را قبل از merge تشخیص می‌دهد.

## 2) Problem Statement

Lintهای کلاسیک بیشتر syntax-centric هستند و روابط معماری چند-ماژولی/چند-پکیجی را به‌صورت کافی مدل نمی‌کنند.
نتیجه این است که خطاهای معماری حیاتی دیر کشف می‌شوند:

- Cycle بین moduleها و packageها
- عبور غیرمجاز از boundaryهای layer/feature/public API
- drift معماری در monorepo
- نقض مرز runtime در Next.js (`Client`, `Server`, `Edge`, `Node`)
- dependency pathهای ناسازگار با محیط اجرا

## 3) Scope (MVP)

### 3.1 پروژه‌ها و فناوری‌های پشتیبانی‌شده در MVP

- JavaScript / TypeScript
- React
- Next.js (تمرکز روی App Router و الگوهای اصلی Pages)
- single-package و monorepo (workspaceهای رایج)

### 3.2 Rule Namespaces

- `ARCH` — قوانین معماری ماژول/لایه/feature
- `RUNTIME` — سازگاری مسیر dependency با runtime
- `PACKAGE` — جهت وابستگی و policy سطح package
- `CONFIG` — اعتبارسنجی و خطاهای تنظیمات

### 3.3 CLI Contract (MVP تا Beta)

- `wae init`
- `wae scan`
- `wae check`
- `wae check --changed`
- `wae explain <RULE_ID>`
- `wae graph`
- `wae doctor`

## 4) Non-Goals (مرزهای عمدی)

WAE **عمداً** این موارد را به ابزارهای دیگر می‌سپارد:

- سبک کدنویسی، formatting و style (`ESLint`, `Biome`, `Prettier`)
- function-level correctness linting
- code transform/autofix سنگین در v1
- تحلیل security/performance اپلیکیشن در سطح runtime profiling

## 5) Detect vs Delegate (بدون ابهام)

WAE **detect می‌کند**:

- graph-level dependency violations
- boundary/rule regressions
- runtime reachability violations (direct/indirect)
- policy violations در monorepo/package graph

WAE **delegate می‌کند**:

- syntax parse errors low-level (تا حد provider)
- style/code-quality ruleهای سنتی
- type-checking کامل TypeScript compiler

## 6) خروجی‌های استاندارد محصول

- Human-readable diagnostics
- JSON / JSONL machine-readable output
- deterministic ordering
- stable violation identity برای baseline/suppression

## 7) معیار Done فاز 0

فاز 0 وقتی Done است که تیم بتواند دقیق، قابل‌تست و بدون ابهام پاسخ دهد:

1. WAE چه چیزهایی را **detect** می‌کند؟
2. چه چیزهایی را **عمداً** به ESLint/Biome/TypeScript می‌سپارد؟
3. قرارداد خروجی و کدهای خطا برای CI چیست؟
