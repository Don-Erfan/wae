use std::collections::BTreeMap;
use std::fs;
use std::time::{Duration, Instant};

use wae_engine::{AnalyzeRequest, Engine, WorkspaceSession};

#[test]
#[ignore = "large real-world-style acceptance scenario executed by its dedicated CI job"]
fn five_hundred_module_next_project_supports_warm_analysis_and_fault_injection() {
    let root =
        std::env::temp_dir().join(format!("wae-real-world-acceptance-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src/app/generated")).unwrap();
    fs::write(
        root.join("package.json"),
        r#"{"name":"wae-real-world","private":true,"dependencies":{"next":"15.2.6"}}"#,
    )
    .unwrap();
    fs::write(
        root.join("wae.yaml"),
        "version: 1\nproject:\n  include: ['src/**/*.ts']\nresolution:\n  mode: bundler\ncache:\n  enabled: true\nrules:\n  ARCH-001: error\n",
    )
    .unwrap();
    for index in 0..500 {
        let source = if index == 499 {
            "export const value499 = 499;".to_string()
        } else {
            format!("import './m{}.ts'; export const value{index} = {index};", index + 1)
        };
        fs::write(root.join(format!("src/app/generated/m{index}.ts")), source).unwrap();
    }

    let engine = Engine::default();
    let cold = engine.analyze(AnalyzeRequest::new(&root)).unwrap();
    assert_eq!(cold.incremental.analyzed_modules, 500);
    assert!(cold.diagnostics.is_empty(), "{:?}", cold.diagnostics);
    let warm = engine.analyze(AnalyzeRequest::new(&root)).unwrap();
    assert_eq!(warm.incremental.restored_modules, 500);
    assert!(warm.incremental.rule_snapshot_reused);
    assert_eq!(warm.diagnostics, cold.diagnostics);

    fs::write(
        root.join("src/app/generated/m499.ts"),
        "import './m0.ts'; export const value499 = 499;",
    )
    .unwrap();
    let injected = engine.analyze(AnalyzeRequest::new(&root)).unwrap();
    assert_eq!(injected.incremental.analyzed_modules, 1);
    assert_eq!(injected.incremental.restored_modules, 499);
    assert!(!injected.incremental.rule_snapshot_reused);
    let cycles = injected
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule_id.0 == "ARCH-001")
        .collect::<Vec<_>>();
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0].dependency_path.len(), 501);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "10k all-rules cold-path regression gate executed by dedicated CI"]
fn ten_thousand_universal_modules_keep_cold_analysis_near_linear() {
    const MODULES: usize = 10_000;
    let root = std::env::temp_dir().join(format!("wae-universal-10k-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src/generated")).unwrap();
    fs::write(root.join("package.json"), r#"{"name":"wae-universal-10k","private":true}"#).unwrap();
    fs::write(
        root.join("wae.yaml"),
        "version: 1\nproject:\n  include: ['src/**/*.ts']\nresolution:\n  mode: bundler\ncache:\n  enabled: true\n",
    )
    .unwrap();
    for index in 0..MODULES {
        let source = if index + 1 == MODULES {
            format!("export const value{index} = {index};")
        } else {
            format!("import './m{}.ts'; export const value{index} = {index};", index + 1)
        };
        fs::write(root.join(format!("src/generated/m{index}.ts")), source).unwrap();
    }

    let engine = Engine::default();
    let mut cold_samples = Vec::new();
    let mut analysis = None;
    for _ in 0..5 {
        let started = Instant::now();
        let result = engine.analyze(AnalyzeRequest::new(&root).without_cache()).unwrap();
        cold_samples.push(started.elapsed());
        analysis = Some(result);
    }
    let analysis = analysis.unwrap();
    let elapsed = median(&mut cold_samples);
    let budget = duration_env("WAE_ENGINE_10K_BUDGET_MS", 3_000);
    assert_eq!(analysis.project.modules.len(), MODULES);
    assert_eq!(analysis.incremental.analyzed_modules, MODULES);
    assert!(
        elapsed <= budget,
        "10k universal cold analysis took {elapsed:?}, budget {budget:?}; timings={:?}",
        analysis.timings
    );
    engine.analyze(AnalyzeRequest::new(&root)).unwrap();
    let mut warm_samples = Vec::new();
    let mut warm = None;
    for _ in 0..5 {
        let started = Instant::now();
        let result = engine.analyze(AnalyzeRequest::new(&root)).unwrap();
        warm_samples.push(started.elapsed());
        warm = Some(result);
    }
    let warm = warm.unwrap();
    let warm_elapsed = median(&mut warm_samples);
    assert_eq!(warm.incremental.restored_modules, MODULES);
    assert!(warm.incremental.rule_snapshot_reused);
    let session = WorkspaceSession::new(&root);
    session.analyze(&session.begin_analysis(), &BTreeMap::new()).unwrap();
    let mut edit_samples = Vec::new();
    let mut edited = None;
    for revision in 0..5 {
        let source = format!("export const value9999 = {};", 10_000 + revision);
        let started = Instant::now();
        let overlays = BTreeMap::from([("src/generated/m9999.ts".into(), source)]);
        let result = session.analyze_changes(&session.begin_analysis(), &overlays, false).unwrap();
        edit_samples.push(started.elapsed());
        edited = Some(result);
    }
    let edited = edited.unwrap();
    let edit_elapsed = median(&mut edit_samples);
    assert_eq!(edited.incremental.analyzed_modules, 1);
    assert_eq!(edited.incremental.restored_modules, MODULES - 1);
    let peak_rss = peak_rss_kb();
    eprintln!(
        "WAE_ENGINE_10K cold_median_ms={} warm_median_ms={} edit_median_ms={} cold_samples_ms={:?} warm_samples_ms={:?} edit_samples_ms={:?} peak_rss_kb={:?} cold_timings={:?} warm_timings={:?} edit_timings={:?}",
        elapsed.as_millis(),
        warm_elapsed.as_millis(),
        edit_elapsed.as_millis(),
        milliseconds(&cold_samples),
        milliseconds(&warm_samples),
        milliseconds(&edit_samples),
        peak_rss,
        analysis.timings,
        warm.timings,
        edited.timings,
    );
    let warm_budget = duration_env("WAE_ENGINE_10K_WARM_BUDGET_MS", 650);
    let edit_budget = duration_env("WAE_ENGINE_10K_EDIT_BUDGET_MS", 750);
    assert!(warm_elapsed <= warm_budget, "10k warm median took {warm_elapsed:?}");
    assert!(edit_elapsed <= edit_budget, "10k single-edit median took {edit_elapsed:?}");
    assert_relative_budget(
        "cold",
        elapsed,
        release_baseline("coldMedianMs", "WAE_ENGINE_10K_BASELINE_COLD_MS", 1_116),
        125,
    );
    assert_relative_budget(
        "warm",
        warm_elapsed,
        release_baseline("warmMedianMs", "WAE_ENGINE_10K_BASELINE_WARM_MS", 344),
        125,
    );
    assert_relative_budget(
        "edit",
        edit_elapsed,
        release_baseline("editMedianMs", "WAE_ENGINE_10K_BASELINE_EDIT_MS", 397),
        120,
    );
    let rss_budget_kb = std::env::var("WAE_ENGINE_10K_RSS_BUDGET_KB")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(256 * 1024);
    if let Some(peak_rss) = peak_rss {
        assert!(
            peak_rss <= rss_budget_kb,
            "10k analysis peak RSS was {peak_rss} KiB; budget is {rss_budget_kb} KiB"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "50k/100k full-engine scale gate executed by dedicated CI workflows"]
fn large_full_engine_cold_warm_and_edit_are_bounded() {
    let modules = std::env::var("WAE_ENGINE_LARGE_MODULES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(50_000);
    let root =
        std::env::temp_dir().join(format!("wae-full-engine-{modules}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src/generated")).unwrap();
    fs::write(root.join("package.json"), r#"{"name":"wae-full-engine","private":true}"#).unwrap();
    fs::write(
        root.join("wae.yaml"),
        "version: 1\nproject:\n  include: ['src/**/*.ts']\nresolution:\n  mode: bundler\ncache:\n  enabled: true\nrules:\n  ARCH-001: error\n",
    )
    .unwrap();
    for index in 0..modules {
        let source = if index + 1 == modules {
            format!("export const value{index} = {index};")
        } else {
            format!("import './m{}.ts'; export const value{index} = {index};", index + 1)
        };
        fs::write(root.join(format!("src/generated/m{index}.ts")), source).unwrap();
    }

    let engine = Engine::default();
    let cold_started = Instant::now();
    let cold = engine.analyze(AnalyzeRequest::new(&root)).unwrap();
    let cold_elapsed = cold_started.elapsed();
    let warm_started = Instant::now();
    let warm = engine.analyze(AnalyzeRequest::new(&root)).unwrap();
    let warm_elapsed = warm_started.elapsed();
    let session = WorkspaceSession::new(&root);
    session.analyze(&session.begin_analysis(), &BTreeMap::new()).unwrap();
    let edited_module = format!("src/generated/m{}.ts", modules - 1);
    let overlays = BTreeMap::from([(edited_module, "export const finalValue = true;".into())]);
    let edit_started = Instant::now();
    let edit = session.analyze_changes(&session.begin_analysis(), &overlays, false).unwrap();
    let edit_elapsed = edit_started.elapsed();

    assert_eq!(cold.project.modules.len(), modules);
    assert_eq!(cold.incremental.analyzed_modules, modules);
    assert_eq!(warm.incremental.restored_modules, modules);
    assert!(warm.incremental.rule_snapshot_reused);
    assert_eq!(edit.incremental.analyzed_modules, 1);
    assert_eq!(edit.incremental.restored_modules, modules - 1);
    let cold_budget = duration_env("WAE_ENGINE_LARGE_COLD_BUDGET_MS", 30_000);
    let warm_budget = duration_env("WAE_ENGINE_LARGE_WARM_BUDGET_MS", 5_000);
    let edit_budget = duration_env("WAE_ENGINE_LARGE_EDIT_BUDGET_MS", 5_000);
    assert!(cold_elapsed <= cold_budget, "large cold analysis took {cold_elapsed:?}");
    assert!(warm_elapsed <= warm_budget, "large warm analysis took {warm_elapsed:?}");
    assert!(edit_elapsed <= edit_budget, "large single edit took {edit_elapsed:?}");
    let peak_rss = peak_rss_kb();
    let rss_budget_kb = std::env::var("WAE_ENGINE_LARGE_RSS_BUDGET_KB")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(768 * 1024);
    if let Some(peak_rss) = peak_rss {
        assert!(peak_rss <= rss_budget_kb, "large analysis used {peak_rss} KiB peak RSS");
    }
    eprintln!(
        "WAE_ENGINE_LARGE modules={modules} cold_ms={} warm_ms={} edit_ms={} peak_rss_kb={peak_rss:?} cold_timings={:?} warm_timings={:?} edit_timings={:?}",
        cold_elapsed.as_millis(),
        warm_elapsed.as_millis(),
        edit_elapsed.as_millis(),
        cold.timings,
        warm.timings,
        edit.timings,
    );
    fs::remove_dir_all(root).unwrap();
}

fn duration_env(name: &str, default_ms: u64) -> Duration {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_millis(default_ms))
}

fn release_baseline(key: &str, override_name: &str, default_ms: u64) -> Duration {
    if std::env::var_os(override_name).is_some() {
        return duration_env(override_name, default_ms);
    }
    let value: serde_json::Value =
        serde_json::from_str(include_str!("../../../performance/baselines/v0.0.26.json"))
            .expect("valid checked-in release performance baseline");
    Duration::from_millis(value[key].as_u64().unwrap_or(default_ms))
}

fn median(samples: &mut [Duration]) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn milliseconds(samples: &[Duration]) -> Vec<u128> {
    samples.iter().map(Duration::as_millis).collect()
}

fn assert_relative_budget(label: &str, actual: Duration, baseline: Duration, percent: u32) {
    let allowed_ms = baseline.as_millis().saturating_mul(u128::from(percent)) / 100;
    assert!(
        actual.as_millis() <= allowed_ms,
        "{label} median {}ms exceeds release baseline {}ms × {percent}% ({}ms)",
        actual.as_millis(),
        baseline.as_millis(),
        allowed_ms,
    );
}

#[cfg(target_os = "linux")]
fn peak_rss_kb() -> Option<u64> {
    fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

#[cfg(not(target_os = "linux"))]
fn peak_rss_kb() -> Option<u64> {
    None
}
