# RFC-001: Intermediate Representation (IR) for WebLint

## وضعیت

- Status: Draft (Phase 0)
- Owners: Core Architecture Team
- Target: V1 foundation

## 1) مسئله

Ruleها نباید به parser یا framework خاص وابسته باشند. اگر Rule مستقیماً با AST خام کار کند:

- توسعه rule دشوار می‌شود
- portability پایین می‌آید
- کارایی کاهش می‌یابد (تبدیل داده در هر rule)

بنابراین به یک IR پایدار، کم‌هزینه و parser-agnostic نیاز داریم.

## 2) اهداف IR

- مستقل از parser/bundler
- قابل serialize به JSON
- حداقل داده لازم برای ruleهای V1
- قابل گسترش برای runtime/monorepo/metrics

## 3) Non-Goals

- نگه‌داری AST کامل در IR
- تبدیل IR به فرم executable JS/TS
- پشتیبانی کامل DSL rule در فاز اول

## 4) مدل مفهومی

### 4.1 ModuleNode

```yaml
ModuleNode:
  id: string                 # canonical id
  path: string               # normalized absolute/virtual path
  package: string|null       # monorepo package/app
  layer: string|null         # app/features/entities/shared/...
  feature: string|null       # e.g. payment, user
  visibility: public|private
  runtime:
    browser: boolean
    server: boolean
    edge: boolean
    node: boolean
  tags: [string]
```

### 4.2 DependencyEdge

```yaml
DependencyEdge:
  from: ModuleId
  to: ModuleId | ExternalPackageId
  kind: static|dynamic|type_only|re_export
  loc:
    file: string
    line: number
    column: number
  via: [string]              # optional chain for expanded paths
```

### 4.3 GraphIR

```yaml
GraphIR:
  modules: ModuleNode[]
  edges: DependencyEdge[]
  externals:
    - id: string
      runtime_compat:
        browser: compatible|incompatible|unknown
        edge: compatible|incompatible|unknown
```

## 5) Diagnostic Contract

```yaml
Diagnostic:
  id: string                 # e.g. ARCH-001
  severity: error|warning|info
  message: string
  primary_location:
    file: string
    line: number
    column: number
  path: [string]             # dependency path/cycle path
  metadata: object
```

## 6) Rule ID Taxonomy (V1 baseline)

- `ARCH-001`: Circular dependency detected
- `ARCH-003`: Layer violation
- `ARCH-004`: Feature boundary violation
- `ARCH-005`: Private module import
- `RUNTIME-001`: Client → Server violation
- `RUNTIME-003`: Browser-incompatible dependency path
- `RUNTIME-004`: Edge-incompatible dependency path

## 7) الگوریتم پایه برای تولید IR

1. کشف moduleها از source roots
2. parse فایل‌ها و استخراج import/export/directives
3. resolve dependency targetها
4. classify layer/feature/runtime/visibility
5. ساخت `GraphIR` نهایی

## 8) ملاحظات کارایی

- intern کردن path/id برای کاهش حافظه
- cache resolver بر اساس `(importer, specifier)`
- shared graph برای اجرای چند rule
- امکان incremental update در آینده نزدیک

## 9) سازگاری با CLI اولیه

- `weblint graph --format json` خروجی مستقیم از `GraphIR`
- `weblint check` و `weblint check --changed` مصرف‌کننده IR در Rule Engine

## 10) نمونه Diagnostic

```yaml
id: ARCH-004
severity: error
message: Feature boundary violation
primary_location:
  file: src/features/payment/service.ts
  line: 12
  column: 8
path:
  - src/features/payment/service.ts
  - src/features/user/internal/utils.ts
metadata:
  allowed_import_root: src/features/user/index.ts
```

## 11) Open Questions

- دقت runtime classification برای کتابخانه‌های third-party چگونه افزایش یابد؟
- سیاست versioning برای IR (`ir_version`) از چه زمانی رسمی شود؟
- آیا در V1 نیاز به source-map ارتباط IR↔AST داریم یا در V2 کافی است؟

## 12) جمله نهایی فاز ۰

این ابزار graph معماری یک Web Application را می‌سازد و architectural regressions را قبل از merge شدن پیدا می‌کند.
