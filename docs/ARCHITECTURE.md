# WebLint / ArchLint — Architecture Contract (Phase 0)

## 1) اصل معماری

Pipeline محصول باید با این مرزبندی ثابت کار کند:

```text
Parser AST
  ↓
Internal Representation (IR)
  ↓
Module/Package/Runtime Graphs
  ↓
Rule Engine
  ↓
Diagnostics / Reporters / Integrations
```

قانون کلیدی: `Rule` نباید parser-specific API بشناسد.

## 2) هدف فنی

ساخت یک هسته deterministic که:

- روی graphهای بزرگ قابل اعتماد باشد
- برای CLI/IDE/CI خروجی هم‌ارز تولید کند
- integration logic را از business logic جدا نگه دارد

## 3) مدل ماژول‌بندی crateها

- `core`: domain model, IR contracts, diagnostics contracts
- `parser`: adapterهای TS/JS
- `resolver`: resolution pipeline
- `graph`: module/package/runtime graph engine
- `rules`: rule interfaces + rule implementations
- `config`: schema + validation + model building
- `lsp`: thin adapter روی core services
- `cli`: orchestration + reporters

## 4) Design Pattern Map (Refactoring.Guru-aligned)

### 4.1 CLI و Orchestration

- `Command`: هر دستور (`init`, `scan`, `check`, `explain`, `graph`, `doctor`) یک handler مستقل
- `Factory Method`: ساخت handler بر اساس command line
- `Facade`: یک API سطح‌بالا برای اجرای pipeline کامل

### 4.2 Parser / Resolver / Framework

- `Strategy`: انتخاب parser/resolver/framework policy بر اساس context
- `Adapter`: نرمال‌سازی خروجی parserهای بیرونی به IR داخلی
- `Chain of Responsibility`: رزولوشن مرحله‌ای import (`relative -> alias -> package -> fallback`)
- `Abstract Factory`: ساخت یک بسته سازگار از parser+resolver+classifier برای هر framework

### 4.3 Graph و Rule Engine

- `Builder`: ساخت مرحله‌ای Project/Graph با validate-on-build
- `Composite`: RuleSet به‌عنوان مجموعه ruleهای مستقل
- `Template Method`: فلو استاندارد اجرای rule (`prepare -> evaluate -> normalize diagnostics`)
- `Specification`: policyهای معماری قابل compose (`allowed/forbidden dependency`)

### 4.4 Baseline / Suppression / Config

- `Policy Object`: سیاست‌های suppression/baseline به‌صورت شیء مستقل
- `Interpreter` (سبک): تفسیر patternهای path/rule در suppressions
- `Repository`: abstraction برای cache/baseline storage

### 4.5 Integrations (LSP/VS Code/JetBrains/CI)

- `Ports & Adapters` (Hexagonal):
  - Core فقط port تعریف می‌کند
  - هر integration یک adapter نازک است
  - خروجی‌ها باید از یک مدل واحد `Diagnostic` تولید شوند

## 5) Anti-patternهای ممنوع در V1

- `Singleton` در Rule Engine/Core services
- condition branchهای framework داخل هسته (`if nextjs { ... }`)
- coupling مستقیم CLI به جزئیات parser/resolver
- duplicate business logic در integrationها

## 6) تصمیم‌های کارایی

- `adjacency list` برای graph traversal
- cache key مبتنی بر fingerprint فایل + resolver context
- deterministic sorting برای diagnostics
- اجرای ruleها روی snapshot مشترک graph (بدون parse مجدد)
- incremental invalidation بر اساس dependency impact

## 7) قرارداد Diagnostic (نسخه پایه)

هر diagnostic باید حداقل داشته باشد:

- `rule_id`
- `severity`
- `message`
- `primary_location`
- `secondary_locations`
- `dependency_path`
- `suggestion`
- `metadata`

## 8) معیار Done فاز 0

- معماری هدف و مرزها برای همه crateها مشخص و قابل ارجاع باشد
- نگاشت patternها به بخش‌های سیستم واضح باشد
- تصمیم‌های performance قابل پیگیری و قابل تست باشند
