use std::fs;
use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
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
    let mut group = criterion.benchmark_group("engine/full_pipeline");
    group.sample_size(10).measurement_time(Duration::from_secs(5));
    for modules in [1_000_usize, 10_000] {
        let root = fixture(modules);
        let engine = Engine::default();
        group.throughput(Throughput::Elements(modules as u64));
        group.bench_with_input(BenchmarkId::new("cold", modules), &root, |bencher, root| {
            bencher.iter(|| {
                let _ = fs::remove_dir_all(root.join(".wae/cache"));
                black_box(engine.analyze(AnalyzeRequest::new(root)).unwrap())
            });
        });
        engine.analyze(AnalyzeRequest::new(&root)).unwrap();
        group.bench_with_input(BenchmarkId::new("warm", modules), &root, |bencher, root| {
            bencher.iter(|| {
                let analysis = engine.analyze(AnalyzeRequest::new(root)).unwrap();
                assert_eq!(analysis.incremental.restored_modules, modules);
                assert!(analysis.incremental.rule_snapshot_reused);
                black_box(analysis)
            });
        });
        let revision = std::cell::Cell::new(0_u64);
        group.bench_with_input(BenchmarkId::new("single_edit", modules), &root, |bencher, root| {
            bencher.iter(|| {
                let next = revision.get() + 1;
                revision.set(next);
                let analysis = engine
                    .analyze(AnalyzeRequest::new(root).with_overlay(
                        "src/m0.ts",
                        format!("import './m1.ts'; export const edited = {next};"),
                    ))
                    .unwrap();
                assert_eq!(analysis.incremental.analyzed_modules, 1);
                assert_eq!(analysis.incremental.restored_modules, modules - 1);
                black_box(analysis)
            });
        });
        let _ = fs::remove_dir_all(root);
    }
    group.finish();
}

criterion_group!(benches, incremental);
criterion_main!(benches);
