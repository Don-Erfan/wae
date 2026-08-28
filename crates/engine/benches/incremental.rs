use std::fs;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use wae_engine::{AnalyzeRequest, Engine};

fn fixture(modules: usize) -> std::path::PathBuf {
    let root = std::env::temp_dir()
        .join(format!("wae-incremental-benchmark-{}-{modules}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("wae.yaml"),
        "version: 1\nresolution:\n  mode: bundler\ncache:\n  enabled: true\n  directory: .wae/cache\n",
    )
    .unwrap();
    for index in 0..modules {
        let source = if index + 1 == modules {
            format!("export const value{index} = {index};")
        } else {
            format!("import './m{}.ts'; export const value{index} = {index};", index + 1)
        };
        fs::write(root.join(format!("src/m{index}.ts")), source).unwrap();
    }
    root
}

fn incremental(criterion: &mut Criterion) {
    let root = fixture(1_000);
    let engine = Engine::default();
    criterion.bench_function("engine/1k/cold", |bencher| {
        bencher.iter(|| {
            let _ = fs::remove_dir_all(root.join(".wae/cache"));
            black_box(engine.analyze(AnalyzeRequest::new(&root)).unwrap())
        });
    });
    engine.analyze(AnalyzeRequest::new(&root)).unwrap();
    criterion.bench_function("engine/1k/warm", |bencher| {
        bencher.iter(|| {
            let analysis = engine.analyze(AnalyzeRequest::new(&root)).unwrap();
            assert_eq!(analysis.incremental.restored_modules, 1_000);
            assert!(analysis.incremental.rule_snapshot_reused);
            black_box(analysis)
        });
    });
    let _ = fs::remove_dir_all(root);
}

criterion_group!(benches, incremental);
criterion_main!(benches);
