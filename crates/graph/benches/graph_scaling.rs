use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use wae_core::domain::{
    Dependency, DependencyKind, FrameworkMetadata, Module, ModuleId, ModuleKind, ModulePath,
    Package, PackageName, Project, Runtime, SourceLocation,
};
use wae_graph::ModuleGraph;

#[derive(Clone, Copy)]
enum Topology {
    Chain,
    Star,
    ClusteredScc,
}

impl Topology {
    fn name(self) -> &'static str {
        match self {
            Self::Chain => "chain",
            Self::Star => "star",
            Self::ClusteredScc => "clustered-scc",
        }
    }
}

fn synthetic_project(modules: usize, topology: Topology) -> Project {
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
    let endpoints = match topology {
        Topology::Chain => (1..modules).map(|index| (index - 1, index)).collect::<Vec<_>>(),
        Topology::Star => (1..modules).map(|index| (0, index)).collect::<Vec<_>>(),
        Topology::ClusteredScc => (0..modules)
            .map(|index| {
                let cluster_start = (index / 20) * 20;
                let cluster_end = (cluster_start + 20).min(modules);
                let next = if index + 1 == cluster_end { cluster_start } else { index + 1 };
                (index, next)
            })
            .collect::<Vec<_>>(),
    };
    let dependencies = endpoints
        .into_iter()
        .map(|(from, to)| Dependency {
            from: ModuleId(format!("src/m{from}.ts")),
            to: ModuleId(format!("src/m{to}.ts")),
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
        let project = synthetic_project(modules, Topology::Chain);
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

    let mut topologies = criterion.benchmark_group("module_graph/topologies-10k");
    topologies.sample_size(10).measurement_time(Duration::from_secs(3));
    for topology in [Topology::Chain, Topology::Star, Topology::ClusteredScc] {
        let project = synthetic_project(10_000, topology);
        topologies.throughput(Throughput::Elements(10_000));
        topologies.bench_with_input(topology.name(), &project, |bencher, project| {
            bencher.iter(|| {
                let graph = ModuleGraph::from_project(black_box(project));
                black_box(graph.strongly_connected_components())
            });
        });
    }
    topologies.finish();

    let mut algorithms = criterion.benchmark_group("module_graph/algorithms");
    algorithms.sample_size(10).measurement_time(Duration::from_secs(3));
    for modules in [1_000_usize, 10_000, 50_000] {
        let graph = ModuleGraph::from_project(&synthetic_project(modules, Topology::Chain));
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
