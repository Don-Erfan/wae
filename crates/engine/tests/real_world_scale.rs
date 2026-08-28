use std::fs;

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
