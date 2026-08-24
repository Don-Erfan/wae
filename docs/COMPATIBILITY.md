# WAE — Compatibility Policy

## 1) اصل سیاست سازگاری

پشتیبانی رسمی فقط برای مواردی اعلام می‌شود که:

1. در این سند ذکر شده باشند
2. در CI تست شوند
3. برای آن‌ها معیار رفتار تعریف شده باشد

هر چیزی خارج از این لیست، best-effort است و ممکن است شکست بخورد.

## 2) نسخه‌بندی قراردادها

- `Config schema`: versioned (`version: 1`)
- `JSON output schema`: versioned (`schemaVersion: 1`)
- `Baseline schema`: writer uses version 2; readers migrate version 1
- `Rule IDs`: پایدار در minor/patch؛ تغییر breaking فقط در major
- `CLI exit codes`: پایدار و contract-based

## 3) پشتیبانی رسمی MVP

### زبان/فریم‌ورک

- JavaScript
- TypeScript
- React

تشخیص اختصاصی Next.js App Router/Pages و runtime rules فعلاً experimental است و جزو پشتیبانی رسمی نیست.

### ساختار پروژه

- single package
- npm/yarn workspaces و `pnpm-workspace.yaml` بر اساس declarationهای صریح
- package `exports`/`imports` با encapsulation، condition، array، null و subpath pattern در سطح MVP

### module system

- ESM و mappingهای TypeScript NodeNext (`.js` → `.ts`, `.mjs` → `.mts`, `.cjs` → `.cts`)
- CommonJS (پوشش MVP resolver)
- JSONC `tsconfig.json`, relative `extends`, `baseUrl`, and longest-match `paths`

فایل resolve‌شده‌ای که با `project.exclude` از discovery کنار گذاشته شده، به‌صورت node صریح
`Excluded` و opaque در مدل نگه‌داری می‌شود؛ dependencyهای transitive آن تحلیل نمی‌شوند.

## 4) ماتریس سازگاری هدف (Beta → v1)

- TypeScript (چند نسخه اصلی فعال)
- Node ecosystem: `pnpm`, `npm`, `yarn`
- monorepo tooling structures: `turborepo`, `nx-like`
- Next.js نسخه‌های پشتیبانی‌شده (به‌صورت explicit)

> نسخه‌های دقیق پس از اضافه‌شدن تست CI برای هر محور در همین سند تثبیت می‌شوند.

## 5) سیاست تغییرات (Compatibility Guarantees)

- Patch releases: بدون breaking change در config/CLI/JSON schema
- Minor releases: افزودن قابلیت backward-compatible
- Major releases: اجازه breaking change با migration guide اجباری

## 6) خروجی CLI و Exit Codes

- `0`: موفق (بدون violation fail-level)
- `1`: violation شناسایی شد
- `2`: خطای config/project
- `3`: خطای داخلی سیستم

این قرارداد برای CI ثابت فرض می‌شود.

## 7) Deprecation Policy

- هر قابلیت deprecate شده حداقل یک minor نسخه هشدار می‌دهد
- قبل از حذف، جایگزین رسمی و migration guide ارائه می‌شود

## 8) معیار Done این سند

- هر ادعای سازگاری، تست/CI متناظر داشته باشد
- سند با وضعیت واقعی پروژه و roadmap sync بماند
