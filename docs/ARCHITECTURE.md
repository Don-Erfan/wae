# WAE — Architecture Contract

## 1) اصل معماری

Pipeline محصول باید با این مرزبندی ثابت کار کند:

```text
JS/TS source
  ↓
Dependency-oriented parser adapter → normalized IR
  ↓
Resolver chain → module/package graphs
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
- `engine`: public Facade and pipeline orchestration
- `reporters`: human/JSON/JSONL/SARIF presentation strategies
- `lsp`: thin adapter روی core services
- `cli`: command parsing and thin delivery adapter

## 4) Design Pattern Map (Refactoring.Guru-aligned)

### 4.1 الگوهای پیاده‌شده

- `Facade`: `wae-engine::Engine` تنها API سطح‌بالای pipeline برای CLI و integrationهای آینده است.
- `Strategy / Adapter`: قرارداد `ParserAdapter` جزئیات parser را از IR و engine جدا می‌کند.
- `Chain of Responsibility`: `ResolverPipeline` handlerهای relative، alias، workspace و external package را به‌ترتیب اجرا می‌کند.
- `Strategy`: `ResolutionKindProvider` نوع `import`/`require` را از syntax و mode تعیین می‌کند و `ConditionSetProvider` conditionهای Node10، Node16، NodeNext و Bundler را مستقل و قابل‌آزمون نگه می‌دارد.
- `Composite`: `RuleSet` ruleهای مستقل را روی یک `RuleContext` و graph مشترک اجرا می‌کند.
- `Repository`: baseline storage پشت command صریح `baseline create` قرار دارد و `check --changed` هرگز آن را ایجاد نمی‌کند.
- `Repository`: cache store snapshot را بدون قفل می‌خواند و قفل advisory را فقط برای reload→merge→prune→atomic-write می‌گیرد؛ پس از crash نیز نیازمند پاک‌کردن lock-file نیست.
- `Ports & Adapters`: قرارداد `VcsPort` و `ChangeSet` در engine قرار دارد و Git/CLI در لبه می‌مانند؛ ruleها به command یا reporter وابسته نیستند.

این انتخاب‌ها با تعریف‌های Refactoring.Guru هم‌راستا هستند: Facade سطح ساده‌ای روی subsystem می‌دهد، Strategy الگوریتم‌های قابل‌تعویض را جدا می‌کند، Chain درخواست را در handlerهای مرتب عبور می‌دهد، و Composite مجموعه‌ای از اجزا را پشت قرارداد مشترک قرار می‌دهد.

منابع: [Facade](https://refactoring.guru/design-patterns/facade)، [Strategy](https://refactoring.guru/design-patterns/strategy)، [Chain of Responsibility](https://refactoring.guru/design-patterns/chain-of-responsibility)، [Composite](https://refactoring.guru/design-patterns/composite).

### 4.2 الگوهای تعمداً به‌تعویق‌افتاده

`Abstract Factory` تا زمانی که provider/framework دوم وجود ندارد اضافه نمی‌شود. `Template Method` و hierarchy برای commandها نیز تا وقتی variation واقعی نداشته باشند ارزش کافی ندارند. Specification برای policyهای پیچیده‌تر و framework adapter برای Next.js در milestone مربوطه اضافه می‌شوند.

## 5) Anti-patternهای ممنوع در V1

- `Singleton` در Rule Engine/Core services
- condition branchهای framework داخل هسته (`if nextjs { ... }`)
- coupling مستقیم CLI به جزئیات parser/resolver
- duplicate business logic در integrationها

## 6) تصمیم‌های کارایی

- `adjacency list` برای graph traversal
- cache key مبتنی بر hash پایدار محتوای فایل؛ cache به‌صورت opt-in و atomic ذخیره می‌شود
- deterministic sorting برای diagnostics
- اجرای ruleها روی snapshot مشترک graph (بدون parse مجدد)
- incremental invalidation بر اساس dependency impact
- `ProjectIndex` مبتنی بر hash set برای lookupهای O(1) هنگام ساخت graph
- `TsConfigIndex` با انتخاب نزدیک‌ترین config والد برای monorepoها
- `PackageScopeIndex` محدود به ancestorهای importerهای کشف‌شده، بدون crawl کردن خروجی‌های excluded

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
- `fingerprint` semantic و مستقل از message/severity/line/column
- `schemaVersion` در envelope خروجی machine-readable

پس از اجرای مستقل ruleها، `DiagnosticArbitrator` فقط خانواده‌های overlap ثبت‌شده را براساس edge
semantic گروه‌بندی می‌کند. بالاترین severity حفظ می‌شود، tie با specificity صریح شکسته می‌شود و
تمام ruleهای مرتبط در `metadata.related_rules` باقی می‌مانند. سپس suppression و `FailurePolicy`
اعمال می‌شوند؛ CLI و reporterها همین policy واحد را مصرف می‌کنند.

`DiagnosticIdentity` یک Value Object دامنه‌ای مستقل از representation است. `related_rules` و سایر
metadataهای نمایشی هرگز identity را بازنویسی نمی‌کنند؛ baseline repository برای مهاجرت aliasهای
fingerprint نسخه‌های `0.0.10` و `0.0.11` را نیز می‌پذیرد.

## 8) معیار Done فاز 0

- معماری هدف و مرزها برای همه crateها مشخص و قابل ارجاع باشد
- نگاشت patternها به بخش‌های سیستم واضح باشد
- تصمیم‌های performance قابل پیگیری و قابل تست باشند
