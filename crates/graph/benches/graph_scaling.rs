use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use wae_core::domain::{
    Dependency, DependencyKind, FrameworkMetadata, Module, ModuleId, ModuleKind, ModulePath,
    Package, PackageName, Project, Runtime, SourceLocation,
};
use wae_graph::ModuleGraph;

fn synthetic_project(modules: usize) -> Project {
    let package = Package { name: PackageName("benchmark".into()), root_path: "/bench".into() };
    let nodes = (0..modules)
        .map(|index| Module {
            id: ModuleId(format!("src/m{index}.ts")),
            path: ModulePath(format!("src/m{index}.ts")),
            package: package.name.clone(),
            kind: ModuleKind::Source,
            runtime: Runtime::Universal,
            layer: None,
            framework_metadata: FrameworkMetadata::default(),
        })
        .collect();
    let dependencies = (1..modules)
        .map(|index| Dependency {
            from: ModuleId(format!("src/m{}.ts", index - 1)),
            to: ModuleId(format!("src/m{index}.ts")),
            kind: DependencyKind::Static,
            location: SourceLocation::unknown(),
        })
        .collect();
    Project { packages: vec![package], modules: nodes, dependencies, ..Project::default() }
}

fn graph_scaling(criterion: &mut Criterion) {
    let mut build = criterion.benchmark_group("module_graph/build");
    build.sample_size(10).measurement_time(Duration::from_secs(3));
    for modules in [1_000_usize, 10_000, 50_000, 100_000] {
        let project = synthetic_project(modules);
        build.throughput(Throughput::Elements(modules as u64));
        build.bench_with_input(
            BenchmarkId::from_parameter(modules),
            &project,
            |bencher, project| {
                bencher.iter(|| black_box(ModuleGraph::from_project(black_box(project))));
            },
        );
    }
    build.finish();

    let mut algorithms = criterion.benchmark_group("module_graph/algorithms");
    algorithms.sample_size(10).measurement_time(Duration::from_secs(3));
    for modules in [1_000_usize, 10_000, 50_000] {
        let graph = ModuleGraph::from_project(&synthetic_project(modules));
        algorithms.throughput(Throughput::Elements(modules as u64));
        algorithms.bench_with_input(BenchmarkId::new("scc", modules), &graph, |bencher, graph| {
            bencher.iter(|| black_box(graph.strongly_connected_components()))
        });
        algorithms.bench_with_input(
            BenchmarkId::new("reachable", modules),
            &graph,
            |bencher, graph| {
                bencher.iter(|| black_box(graph.reachable_from(&ModuleId("src/m0.ts".into()))));
            },
        );
        algorithms.bench_with_input(
            BenchmarkId::new("estimated_heap_bytes", modules),
            &graph,
            |bencher, graph| bencher.iter(|| black_box(graph.estimated_heap_bytes())),
        );
    }
    algorithms.finish();
}

criterion_group!(benches, graph_scaling);
criterion_main!(benches);
