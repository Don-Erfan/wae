use std::fs;
use std::time::{Duration, Instant};

use wae_engine::{AnalyzeRequest, Engine};

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

    let budget = std::env::var("WAE_ENGINE_10K_BUDGET_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(15));
    let started = Instant::now();
    let analysis = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
    let elapsed = started.elapsed();
    assert_eq!(analysis.project.modules.len(), MODULES);
    assert_eq!(analysis.incremental.analyzed_modules, MODULES);
    assert!(
        elapsed <= budget,
        "10k universal cold analysis took {elapsed:?}, budget {budget:?}; timings={:?}",
        analysis.timings
    );
    let incremental_budget = std::env::var("WAE_ENGINE_10K_EDIT_BUDGET_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(1));
    let warm_started = Instant::now();
    let warm = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
    let warm_elapsed = warm_started.elapsed();
    assert_eq!(warm.incremental.restored_modules, MODULES);
    assert!(warm.incremental.rule_snapshot_reused);
    let edit_started = Instant::now();
    let edited = Engine::default()
        .analyze(
            AnalyzeRequest::new(&root)
                .with_overlay("src/generated/m9999.ts", "export const value9999 = 10000;"),
        )
        .unwrap();
    let edit_elapsed = edit_started.elapsed();
    assert_eq!(edited.incremental.analyzed_modules, 1);
    assert_eq!(edited.incremental.restored_modules, MODULES - 1);
    assert!(warm_elapsed <= incremental_budget, "10k warm analysis took {warm_elapsed:?}");
    assert!(edit_elapsed <= incremental_budget, "10k single edit took {edit_elapsed:?}");
    let peak_rss = peak_rss_kb();
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
    eprintln!(
        "WAE_ENGINE_10K cold_ms={} warm_ms={} edit_ms={} peak_rss_kb={:?} cold_timings={:?} warm_timings={:?} edit_timings={:?}",
        elapsed.as_millis(),
        warm_elapsed.as_millis(),
        edit_elapsed.as_millis(),
        peak_rss,
        analysis.timings,
        warm.timings,
        edited.timings,
    );
    fs::remove_dir_all(root).unwrap();
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
