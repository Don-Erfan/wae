use std::collections::{HashMap, HashSet, VecDeque};

use wae_core::domain::{Dependency, ModuleId, PackageName, Project, Runtime};

#[derive(Clone, Debug, Default)]
pub struct ModuleGraph {
    node_indices: HashMap<ModuleId, usize>,
    nodes: Vec<ModuleId>,
    edges: Vec<Dependency>,
    outgoing: Vec<Vec<usize>>,
    incoming: Vec<Vec<usize>>,
    edge_keys: HashSet<(ModuleId, ModuleId, wae_core::domain::DependencyKind)>,
}

impl ModuleGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_project(project: &Project) -> Self {
        let mut builder = ModuleGraphBuilder::new();
        let known = project.modules.iter().map(|module| &module.id).collect::<HashSet<_>>();

        for module in &project.modules {
            builder = builder.add_node(module.id.clone());
        }

        for dependency in &project.dependencies {
            if known.contains(&dependency.from) && known.contains(&dependency.to) {
                builder = builder.add_edge(dependency.clone());
            }
        }

        builder.build()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Deterministic capacity-based estimate of heap owned directly by the graph. This is not an
    /// RSS measurement; it exists to make large-graph allocation growth observable and gateable.
    pub fn estimated_heap_bytes(&self) -> usize {
        let id_bytes = self.nodes.iter().map(|node| node.0.capacity()).sum::<usize>();
        let adjacency_bytes = self
            .outgoing
            .iter()
            .chain(&self.incoming)
            .map(|neighbors| neighbors.capacity() * std::mem::size_of::<usize>())
            .sum::<usize>();
        self.nodes.capacity() * std::mem::size_of::<ModuleId>()
            + id_bytes
            + self.edges.capacity() * std::mem::size_of::<Dependency>()
            + self.outgoing.capacity() * std::mem::size_of::<Vec<usize>>()
            + self.incoming.capacity() * std::mem::size_of::<Vec<usize>>()
            + adjacency_bytes
            + self.node_indices.capacity()
                * (std::mem::size_of::<ModuleId>() + std::mem::size_of::<usize>())
            + self.edge_keys.capacity()
                * std::mem::size_of::<(ModuleId, ModuleId, wae_core::domain::DependencyKind)>()
    }

    pub fn nodes(&self) -> &[ModuleId] {
        &self.nodes
    }

    pub fn edges(&self) -> &[Dependency] {
        &self.edges
    }

    pub fn has_node(&self, node: &ModuleId) -> bool {
        self.node_indices.contains_key(node)
    }

    pub fn neighbors(&self, node: &ModuleId) -> Vec<ModuleId> {
        self.outgoing(node)
    }

    pub fn outgoing(&self, node: &ModuleId) -> Vec<ModuleId> {
        let Some(index) = self.node_indices.get(node).copied() else {
            return Vec::new();
        };

        self.outgoing[index].iter().map(|target| self.nodes[*target].clone()).collect()
    }

    pub fn incoming(&self, node: &ModuleId) -> Vec<ModuleId> {
        let Some(index) = self.node_indices.get(node).copied() else {
            return Vec::new();
        };

        self.incoming[index].iter().map(|source| self.nodes[*source].clone()).collect()
    }

    pub fn reachable_from(&self, start: &ModuleId) -> Vec<ModuleId> {
        let Some(start_index) = self.node_indices.get(start).copied() else {
            return Vec::new();
        };

        let mut visited = vec![false; self.nodes.len()];
        let mut queue = VecDeque::new();
        let mut reachable = Vec::new();

        visited[start_index] = true;
        queue.push_back(start_index);

        while let Some(node) = queue.pop_front() {
            for &neighbor in &self.outgoing[node] {
                if visited[neighbor] {
                    continue;
                }

                visited[neighbor] = true;
                queue.push_back(neighbor);
                reachable.push(self.nodes[neighbor].clone());
            }
        }

        reachable
    }

    pub fn has_path(&self, from: &ModuleId, to: &ModuleId) -> bool {
        if from == to {
            return self.has_node(from);
        }

        let Some(start_index) = self.node_indices.get(from).copied() else {
            return false;
        };
        let Some(target_index) = self.node_indices.get(to).copied() else {
            return false;
        };

        let mut visited = vec![false; self.nodes.len()];
        let mut queue = VecDeque::new();

        visited[start_index] = true;
        queue.push_back(start_index);

        while let Some(node) = queue.pop_front() {
            for &neighbor in &self.outgoing[node] {
                if neighbor == target_index {
                    return true;
                }

                if visited[neighbor] {
                    continue;
                }

                visited[neighbor] = true;
                queue.push_back(neighbor);
            }
        }

        false
    }

    /// Returns a deterministic shortest dependency path, including both endpoints.
    pub fn shortest_path(&self, from: &ModuleId, to: &ModuleId) -> Option<Vec<ModuleId>> {
        let start = self.node_indices.get(from).copied()?;
        let target = self.node_indices.get(to).copied()?;
        let mut visited = vec![false; self.nodes.len()];
        let mut previous = vec![None; self.nodes.len()];
        let mut queue = VecDeque::from([start]);
        visited[start] = true;
        while let Some(node) = queue.pop_front() {
            if node == target {
                break;
            }
            for &neighbor in &self.outgoing[node] {
                if !visited[neighbor] {
                    visited[neighbor] = true;
                    previous[neighbor] = Some(node);
                    queue.push_back(neighbor);
                }
            }
        }
        if !visited[target] {
            return None;
        }
        let mut path = vec![target];
        let mut cursor = target;
        while cursor != start {
            cursor = previous[cursor]?;
            path.push(cursor);
        }
        path.reverse();
        Some(path.into_iter().map(|index| self.nodes[index].clone()).collect())
    }

    pub fn strongly_connected_components(&self) -> Vec<Vec<ModuleId>> {
        let order = self.finishing_order();
        let mut visited = vec![false; self.nodes.len()];
        let mut components = Vec::new();

        for &node in order.iter().rev() {
            if visited[node] {
                continue;
            }

            let mut stack = vec![node];
            visited[node] = true;
            let mut component = Vec::new();

            while let Some(current) = stack.pop() {
                component.push(self.nodes[current].clone());

                for &parent in &self.incoming[current] {
                    if visited[parent] {
                        continue;
                    }

                    visited[parent] = true;
                    stack.push(parent);
                }
            }

            components.push(component);
        }

        components
    }

    pub fn cycles(&self) -> Vec<Vec<ModuleId>> {
        let mut cycles = Vec::new();

        for component in self.strongly_connected_components() {
            let component_indices: Vec<usize> = component
                .iter()
                .filter_map(|module_id| self.node_indices.get(module_id).copied())
                .collect();

            if component_indices.is_empty() {
                continue;
            }

            if component_indices.len() == 1 {
                let index = component_indices[0];
                if self.outgoing[index].contains(&index) {
                    cycles.push(vec![self.nodes[index].clone(), self.nodes[index].clone()]);
                }
                continue;
            }

            if let Some(cycle) = self.find_cycle_in_component(&component_indices) {
                cycles.push(cycle);
            }
        }

        cycles
    }

    fn finishing_order(&self) -> Vec<usize> {
        let mut visited = vec![false; self.nodes.len()];
        let mut order = Vec::with_capacity(self.nodes.len());

        for start in 0..self.nodes.len() {
            if visited[start] {
                continue;
            }

            let mut stack: Vec<(usize, bool)> = vec![(start, false)];
            while let Some((node, expanded)) = stack.pop() {
                if expanded {
                    order.push(node);
                    continue;
                }

                if visited[node] {
                    continue;
                }

                visited[node] = true;
                stack.push((node, true));

                for &neighbor in self.outgoing[node].iter().rev() {
                    if !visited[neighbor] {
                        stack.push((neighbor, false));
                    }
                }
            }
        }

        order
    }

    fn find_cycle_in_component(&self, component: &[usize]) -> Option<Vec<ModuleId>> {
        let component_set: HashSet<usize> = component.iter().copied().collect();

        for &start in component {
            for &neighbor in &self.outgoing[start] {
                if !component_set.contains(&neighbor) {
                    continue;
                }

                if let Some(path) = self.path_between(neighbor, start, &component_set) {
                    let mut cycle = Vec::with_capacity(path.len() + 1);
                    cycle.push(self.nodes[start].clone());
                    for node in path {
                        cycle.push(self.nodes[node].clone());
                    }
                    return Some(cycle);
                }
            }
        }

        None
    }

    fn path_between(
        &self,
        start: usize,
        target: usize,
        allowed: &HashSet<usize>,
    ) -> Option<Vec<usize>> {
        let mut queue = VecDeque::new();
        let mut visited = vec![false; self.nodes.len()];
        let mut previous = vec![None; self.nodes.len()];

        visited[start] = true;
        queue.push_back(start);

        while let Some(node) = queue.pop_front() {
            if node == target {
                break;
            }

            for &neighbor in &self.outgoing[node] {
                if !allowed.contains(&neighbor) || visited[neighbor] {
                    continue;
                }

                visited[neighbor] = true;
                previous[neighbor] = Some(node);
                queue.push_back(neighbor);
            }
        }

        if !visited[target] {
            return None;
        }

        let mut path = Vec::new();
        let mut cursor = target;
        path.push(cursor);

        while cursor != start {
            let parent = previous[cursor]?;
            cursor = parent;
            path.push(cursor);
        }

        path.reverse();
        Some(path)
    }

    fn add_node_internal(&mut self, node: ModuleId) -> usize {
        if let Some(index) = self.node_indices.get(&node).copied() {
            return index;
        }

        let index = self.nodes.len();
        self.node_indices.insert(node.clone(), index);
        self.nodes.push(node);
        self.outgoing.push(Vec::new());
        self.incoming.push(Vec::new());
        index
    }

    fn add_edge_internal(&mut self, edge: Dependency) {
        let key = (edge.from.clone(), edge.to.clone(), edge.kind.clone());
        if !self.edge_keys.insert(key) {
            return;
        }
        let from_index = self.add_node_internal(edge.from.clone());
        let to_index = self.add_node_internal(edge.to.clone());

        self.edges.push(edge);
        self.outgoing[from_index].push(to_index);
        self.incoming[to_index].push(from_index);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeRequirement {
    pub runtime: Runtime,
    pub path: Vec<ModuleId>,
}

/// Runtime-aware projection of the module graph. Framework RPC modules (currently Next.js
/// Server Actions) are terminal boundaries when reached from another module: their implementation
/// dependencies execute remotely and must not leak into the caller's runtime capability set.
#[derive(Clone, Debug)]
pub struct RuntimeGraph {
    graph: ModuleGraph,
    runtimes: HashMap<ModuleId, Runtime>,
    rpc_boundaries: HashSet<ModuleId>,
}

impl RuntimeGraph {
    pub fn from_project(project: &Project) -> Self {
        let runtimes =
            project.modules.iter().map(|module| (module.id.clone(), module.runtime)).collect();
        let rpc_boundaries = project
            .modules
            .iter()
            .filter(|module| {
                module.framework_metadata.attributes.get("role").map(String::as_str)
                    == Some("server-action-module")
            })
            .map(|module| module.id.clone())
            .collect();
        Self { graph: ModuleGraph::from_project(project), runtimes, rpc_boundaries }
    }

    pub fn runtime_of(&self, module: &ModuleId) -> Option<Runtime> {
        self.runtimes.get(module).copied()
    }

    pub fn requirements(&self, start: &ModuleId) -> Vec<RuntimeRequirement> {
        [Runtime::Browser, Runtime::Server, Runtime::Edge, Runtime::Node]
            .into_iter()
            .filter_map(|runtime| {
                self.shortest_path_to_runtime(start, &[runtime])
                    .map(|path| RuntimeRequirement { runtime, path })
            })
            .collect()
    }

    pub fn shortest_path_to_runtime(
        &self,
        start: &ModuleId,
        targets: &[Runtime],
    ) -> Option<Vec<ModuleId>> {
        self.shortest_path_matching(start, |module| {
            self.runtime_of(module).is_some_and(|runtime| targets.contains(&runtime))
        })
    }

    pub fn shortest_path_to_any(
        &self,
        start: &ModuleId,
        targets: &HashSet<ModuleId>,
    ) -> Option<Vec<ModuleId>> {
        self.shortest_path_matching(start, |module| targets.contains(module))
    }

    pub fn cycles(&self) -> Vec<Vec<ModuleId>> {
        self.graph
            .cycles()
            .into_iter()
            .filter(|cycle| !cycle.iter().any(|module| self.rpc_boundaries.contains(module)))
            .collect()
    }

    fn shortest_path_matching(
        &self,
        start: &ModuleId,
        matches: impl Fn(&ModuleId) -> bool,
    ) -> Option<Vec<ModuleId>> {
        if !self.graph.has_node(start) {
            return None;
        }
        let mut visited = HashSet::from([start.clone()]);
        let mut previous = HashMap::<ModuleId, ModuleId>::new();
        let mut queue = VecDeque::from([start.clone()]);
        while let Some(module) = queue.pop_front() {
            let reached_boundary = module != *start && self.rpc_boundaries.contains(&module);
            if !reached_boundary && matches(&module) {
                let mut path = vec![module.clone()];
                let mut cursor = module;
                while cursor != *start {
                    cursor = previous.get(&cursor)?.clone();
                    path.push(cursor.clone());
                }
                path.reverse();
                return Some(path);
            }
            if reached_boundary {
                continue;
            }
            for neighbor in self.graph.outgoing(&module) {
                if visited.insert(neighbor.clone()) {
                    previous.insert(neighbor.clone(), module.clone());
                    queue.push_back(neighbor);
                }
            }
        }
        None
    }
}

#[derive(Clone, Debug, Default)]
pub struct ModuleGraphBuilder {
    graph: ModuleGraph,
}

impl ModuleGraphBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(mut self, node: ModuleId) -> Self {
        self.graph.add_node_internal(node);
        self
    }

    pub fn add_edge(mut self, edge: Dependency) -> Self {
        self.graph.add_edge_internal(edge);
        self
    }

    pub fn build(self) -> ModuleGraph {
        let mut nodes = self.graph.nodes;
        nodes.sort_by(|left, right| left.0.cmp(&right.0));
        let mut edges = self.graph.edges;
        edges.sort_by(|left, right| {
            (&left.from.0, &left.to.0, dependency_kind_rank(&left.kind)).cmp(&(
                &right.from.0,
                &right.to.0,
                dependency_kind_rank(&right.kind),
            ))
        });
        let mut graph = ModuleGraph::new();
        for node in nodes {
            graph.add_node_internal(node);
        }
        for edge in edges {
            graph.add_edge_internal(edge);
        }
        graph
    }
}

fn dependency_kind_rank(kind: &wae_core::domain::DependencyKind) -> u8 {
    match kind {
        wae_core::domain::DependencyKind::Static => 0,
        wae_core::domain::DependencyKind::Dynamic => 1,
        wae_core::domain::DependencyKind::TypeOnly => 2,
        wae_core::domain::DependencyKind::ReExport => 3,
        wae_core::domain::DependencyKind::Require => 4,
    }
}

pub struct GraphEngine;

impl GraphEngine {
    pub fn build(project: &Project) -> ModuleGraph {
        ModuleGraph::from_project(project)
    }

    pub fn build_package_graph(project: &Project) -> PackageGraph {
        PackageGraph::from_project(project)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PackageDependency {
    pub from: PackageName,
    pub to: PackageName,
}

#[derive(Clone, Debug, Default)]
pub struct PackageGraph {
    nodes: Vec<PackageName>,
    edges: Vec<PackageDependency>,
    outgoing: HashMap<PackageName, HashSet<PackageName>>,
    incoming: HashMap<PackageName, HashSet<PackageName>>,
}

impl PackageGraph {
    pub fn from_project(project: &Project) -> Self {
        let mut graph = Self::default();

        for package in &project.packages {
            graph.add_node(package.name.clone());
        }

        let modules_by_id: HashMap<&ModuleId, &wae_core::domain::Module> =
            project.modules.iter().map(|module| (&module.id, module)).collect();

        for dependency in &project.dependencies {
            let Some(from_module) = modules_by_id.get(&dependency.from) else {
                continue;
            };
            let Some(to_module) = modules_by_id.get(&dependency.to) else {
                continue;
            };

            let from_package = from_module.package.clone();
            let to_package = to_module.package.clone();

            if from_package == to_package {
                continue;
            }

            graph.add_edge(PackageDependency { from: from_package, to: to_package });
        }
        graph.nodes.sort_by(|left, right| left.0.cmp(&right.0));
        graph
            .edges
            .sort_by(|left, right| (&left.from.0, &left.to.0).cmp(&(&right.from.0, &right.to.0)));
        graph
    }

    pub fn nodes(&self) -> &[PackageName] {
        &self.nodes
    }

    pub fn edges(&self) -> &[PackageDependency] {
        &self.edges
    }

    pub fn outgoing(&self, package: &PackageName) -> Vec<PackageName> {
        let mut packages: Vec<PackageName> = self
            .outgoing
            .get(package)
            .map(|targets| targets.iter().cloned().collect())
            .unwrap_or_default();
        packages.sort_by(|a: &PackageName, b| a.0.cmp(&b.0));
        packages
    }

    pub fn incoming(&self, package: &PackageName) -> Vec<PackageName> {
        let mut packages: Vec<PackageName> = self
            .incoming
            .get(package)
            .map(|sources| sources.iter().cloned().collect())
            .unwrap_or_default();
        packages.sort_by(|a: &PackageName, b| a.0.cmp(&b.0));
        packages
    }

    pub fn has_edge(&self, from: &PackageName, to: &PackageName) -> bool {
        self.outgoing.get(from).map(|targets| targets.contains(to)).unwrap_or(false)
    }

    /// Returns one deterministic representative cycle for each package SCC.
    pub fn cycles(&self) -> Vec<Vec<PackageName>> {
        let mut builder = ModuleGraphBuilder::new();
        for node in &self.nodes {
            builder = builder.add_node(ModuleId(node.0.clone()));
        }
        for edge in &self.edges {
            builder = builder.add_edge(Dependency {
                from: ModuleId(edge.from.0.clone()),
                to: ModuleId(edge.to.0.clone()),
                kind: wae_core::domain::DependencyKind::Static,
                location: wae_core::domain::SourceLocation::unknown(),
            });
        }
        builder
            .build()
            .cycles()
            .into_iter()
            .map(|cycle| cycle.into_iter().map(|module| PackageName(module.0)).collect())
            .collect()
    }

    fn add_node(&mut self, package: PackageName) {
        if self.nodes.iter().any(|existing| existing == &package) {
            return;
        }

        self.nodes.push(package.clone());
        self.outgoing.entry(package.clone()).or_default();
        self.incoming.entry(package).or_default();
    }

    fn add_edge(&mut self, edge: PackageDependency) {
        self.add_node(edge.from.clone());
        self.add_node(edge.to.clone());

        if self.has_edge(&edge.from, &edge.to) {
            return;
        }

        self.outgoing.entry(edge.from.clone()).or_default().insert(edge.to.clone());
        self.incoming.entry(edge.to.clone()).or_default().insert(edge.from.clone());
        self.edges.push(edge);
    }
}

#[cfg(test)]
mod tests {
    use wae_core::domain::{
        Dependency, DependencyKind, FrameworkMetadata, LayerId, Module, ModuleId, ModuleKind,
        ModulePath, Package, PackageName, ProjectBuilder, Runtime, SourceLocation,
    };

    use crate::{ModuleGraph, PackageGraph, RuntimeGraph};

    fn module(package: &Package, id: &str) -> Module {
        Module {
            id: ModuleId(String::from(id)),
            path: ModulePath(format!("/app/src/{id}.ts")),
            package: package.name.clone(),
            kind: ModuleKind::Source,
            runtime: Runtime::Universal,
            layer: Some(LayerId("features".into())),
            framework_metadata: FrameworkMetadata::default(),
        }
    }

    fn dependency(from: &str, to: &str) -> Dependency {
        Dependency {
            from: ModuleId(String::from(from)),
            to: ModuleId(String::from(to)),
            kind: DependencyKind::Static,
            location: SourceLocation::unknown(),
        }
    }

    #[test]
    fn graph_supports_basic_queries() {
        let package =
            Package { name: PackageName(String::from("web")), root_path: String::from("/app") };

        let graph = ModuleGraph::from_project(
            &ProjectBuilder::new()
                .add_package(package.clone())
                .add_module(module(&package, "A"))
                .add_module(module(&package, "B"))
                .add_module(module(&package, "C"))
                .add_dependency(dependency("A", "B"))
                .add_dependency(dependency("A", "C"))
                .add_dependency(dependency("B", "C"))
                .build(),
        );

        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edge_count(), 3);

        let outgoing_a = graph.outgoing(&ModuleId(String::from("A")));
        assert_eq!(outgoing_a.len(), 2);
        assert_eq!(outgoing_a[0].0, "B");
        assert_eq!(outgoing_a[1].0, "C");

        let incoming_c = graph.incoming(&ModuleId(String::from("C")));
        assert_eq!(incoming_c.len(), 2);
        assert_eq!(incoming_c[0].0, "A");
        assert_eq!(incoming_c[1].0, "B");

        let reachable = graph.reachable_from(&ModuleId(String::from("A")));
        assert_eq!(reachable.len(), 2);
        assert_eq!(reachable[0].0, "B");
        assert_eq!(reachable[1].0, "C");

        assert!(graph.has_path(&ModuleId(String::from("A")), &ModuleId(String::from("C"))));
        assert!(!graph.has_path(&ModuleId(String::from("C")), &ModuleId(String::from("A"))));
    }

    #[test]
    fn shortest_path_is_deterministic_and_includes_endpoints() {
        let package = Package { name: PackageName("web".into()), root_path: "/app".into() };
        let graph = ModuleGraph::from_project(
            &ProjectBuilder::new()
                .add_package(package.clone())
                .add_module(module(&package, "A"))
                .add_module(module(&package, "B"))
                .add_module(module(&package, "C"))
                .add_dependency(dependency("A", "B"))
                .add_dependency(dependency("B", "C"))
                .build(),
        );
        assert_eq!(
            graph.shortest_path(&ModuleId("A".into()), &ModuleId("C".into())).unwrap(),
            vec![ModuleId("A".into()), ModuleId("B".into()), ModuleId("C".into())]
        );
        assert!(graph.shortest_path(&ModuleId("C".into()), &ModuleId("A".into())).is_none());
    }

    #[test]
    fn runtime_graph_propagates_requirements_with_explainable_paths() {
        let package = Package { name: PackageName("web".into()), root_path: "/app".into() };
        let mut browser = module(&package, "browser");
        browser.runtime = Runtime::Browser;
        let middle = module(&package, "middle");
        let mut server = module(&package, "server");
        server.runtime = Runtime::Server;
        let project = ProjectBuilder::new()
            .add_package(package)
            .add_module(browser)
            .add_module(middle)
            .add_module(server)
            .add_dependency(dependency("browser", "middle"))
            .add_dependency(dependency("middle", "server"))
            .build();
        let graph = RuntimeGraph::from_project(&project);
        assert_eq!(
            graph
                .shortest_path_to_runtime(&ModuleId("browser".into()), &[Runtime::Server])
                .unwrap(),
            vec![ModuleId("browser".into()), ModuleId("middle".into()), ModuleId("server".into())]
        );
    }

    #[test]
    fn server_actions_are_rpc_boundaries_for_runtime_propagation() {
        let package = Package { name: PackageName("web".into()), root_path: "/app".into() };
        let mut browser = module(&package, "browser");
        browser.runtime = Runtime::Browser;
        let mut action = module(&package, "action");
        action.runtime = Runtime::Server;
        action.framework_metadata.attributes.insert("role".into(), "server-action-module".into());
        let mut node = module(&package, "node");
        node.runtime = Runtime::Node;
        let project = ProjectBuilder::new()
            .add_package(package)
            .add_module(browser)
            .add_module(action)
            .add_module(node)
            .add_dependency(dependency("browser", "action"))
            .add_dependency(dependency("action", "node"))
            .build();
        let graph = RuntimeGraph::from_project(&project);
        assert!(
            graph
                .shortest_path_to_runtime(
                    &ModuleId("browser".into()),
                    &[Runtime::Server, Runtime::Node]
                )
                .is_none()
        );
        assert!(
            graph
                .shortest_path_to_runtime(&ModuleId("action".into()), &[Runtime::Server])
                .is_some()
        );
    }

    #[test]
    fn graph_deduplicates_edges_and_sorts_public_output() {
        let package =
            Package { name: PackageName(String::from("web")), root_path: String::from("/app") };
        let project = ProjectBuilder::new()
            .add_package(package.clone())
            .add_module(module(&package, "A"))
            .add_module(module(&package, "B"))
            .add_module(module(&package, "C"))
            .add_dependency(dependency("B", "C"))
            .add_dependency(dependency("A", "B"))
            .add_dependency(dependency("A", "B"))
            .build();
        let graph = ModuleGraph::from_project(&project);
        assert_eq!(graph.edge_count(), 2);
        assert_eq!(
            graph.nodes().iter().map(|node| node.0.as_str()).collect::<Vec<_>>(),
            vec!["A", "B", "C"]
        );
        assert_eq!(graph.edges()[0].from.0, "A");
    }

    #[test]
    fn project_graph_never_invents_nodes_for_missing_module_records() {
        let package =
            Package { name: PackageName(String::from("web")), root_path: String::from("/app") };
        let project = ProjectBuilder::new()
            .add_package(package.clone())
            .add_module(module(&package, "A"))
            .add_dependency(dependency("A", "excluded"))
            .build();
        let graph = ModuleGraph::from_project(&project);
        assert_eq!(graph.nodes(), &[ModuleId("A".into())]);
        assert!(graph.edges().is_empty());
    }

    #[test]
    fn graph_detects_scc_and_cycle_path() {
        let package =
            Package { name: PackageName(String::from("web")), root_path: String::from("/app") };

        let graph = ModuleGraph::from_project(
            &ProjectBuilder::new()
                .add_package(package.clone())
                .add_module(module(&package, "user"))
                .add_module(module(&package, "payment"))
                .add_module(module(&package, "checkout"))
                .add_dependency(dependency("user", "payment"))
                .add_dependency(dependency("payment", "checkout"))
                .add_dependency(dependency("checkout", "user"))
                .build(),
        );

        let scc = graph.strongly_connected_components();
        let large_components = scc.iter().filter(|component| component.len() > 1).count();
        assert_eq!(large_components, 1);

        let cycles = graph.cycles();
        assert_eq!(cycles.len(), 1);
        assert_eq!(
            cycles[0].iter().map(|node| node.0.as_str()).collect::<Vec<_>>(),
            vec!["checkout", "user", "payment", "checkout"]
        );
    }

    #[test]
    fn graph_handles_thousands_of_modules() {
        let package =
            Package { name: PackageName(String::from("web")), root_path: String::from("/app") };

        let mut builder = ProjectBuilder::new().add_package(package.clone());

        let total_modules = 3_000;
        for index in 0..total_modules {
            builder = builder.add_module(module(&package, &format!("m{index}")));
        }

        for index in 0..(total_modules - 1) {
            builder = builder
                .add_dependency(dependency(&format!("m{index}"), &format!("m{}", index + 1)));
        }

        let graph = ModuleGraph::from_project(&builder.build());

        assert_eq!(graph.node_count(), total_modules as usize);
        assert_eq!(graph.edge_count(), (total_modules - 1) as usize);

        let reachable = graph.reachable_from(&ModuleId(String::from("m0")));
        assert_eq!(reachable.len(), (total_modules - 1) as usize);
    }

    #[test]
    fn generated_dags_preserve_reachability_and_have_only_singleton_sccs() {
        let package = Package { name: PackageName("property".into()), root_path: "/tmp".into() };
        let mut seed = 0x5eed_u64;
        for _case in 0..40 {
            let mut builder = ProjectBuilder::new().add_package(package.clone());
            for node in 0..32 {
                builder = builder.add_module(module(&package, &format!("n{node}")));
            }
            let mut expected = vec![vec![false; 32]; 32];
            for (from, row) in expected.iter_mut().enumerate() {
                for (to, reachable) in row.iter_mut().enumerate().skip(from + 1) {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    if seed >> 61 == 0 {
                        builder = builder
                            .add_dependency(dependency(&format!("n{from}"), &format!("n{to}")));
                        *reachable = true;
                    }
                }
            }
            for pivot in 0..32 {
                for from in 0..32 {
                    for to in 0..32 {
                        expected[from][to] |= expected[from][pivot] && expected[pivot][to];
                    }
                }
            }
            let graph = ModuleGraph::from_project(&builder.build());
            assert!(graph.strongly_connected_components().iter().all(|scc| scc.len() == 1));
            for (from, row) in expected.iter().enumerate() {
                for (to, reachable) in row.iter().enumerate() {
                    assert_eq!(
                        graph.has_path(&ModuleId(format!("n{from}")), &ModuleId(format!("n{to}"))),
                        *reachable || from == to
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "release-mode CI performance regression gate"]
    fn graph_100k_performance_gate() {
        use std::time::{Duration, Instant};

        let package = Package { name: PackageName("benchmark".into()), root_path: "/bench".into() };
        let mut builder = ProjectBuilder::new().add_package(package.clone());
        for index in 0..100_000 {
            builder = builder.add_module(module(&package, &format!("m{index}")));
            if index > 0 {
                builder = builder
                    .add_dependency(dependency(&format!("m{}", index - 1), &format!("m{index}")));
            }
        }
        let project = builder.build();
        let budget_ms = std::env::var("WAE_GRAPH_BUDGET_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(15_000);
        let budget = Duration::from_millis(budget_ms);

        let started = Instant::now();
        let graph = ModuleGraph::from_project(&project);
        let build_elapsed = started.elapsed();
        assert!(
            build_elapsed < budget,
            "100k graph build took {build_elapsed:?}, budget {budget:?}"
        );
        assert!(
            graph.estimated_heap_bytes() < 256 * 1024 * 1024,
            "100k graph estimate exceeded 256 MiB: {} bytes",
            graph.estimated_heap_bytes()
        );
        let started = Instant::now();
        let components = graph.strongly_connected_components();
        let scc_elapsed = started.elapsed();
        assert!(scc_elapsed < budget, "100k SCC took {scc_elapsed:?}, budget {budget:?}");
        assert_eq!(components.len(), 100_000);
    }

    #[test]
    fn package_graph_builds_cross_package_edges() {
        let web = Package {
            name: PackageName(String::from("apps/web")),
            root_path: String::from("/repo/apps/web"),
        };
        let ui = Package {
            name: PackageName(String::from("packages/ui")),
            root_path: String::from("/repo/packages/ui"),
        };
        let auth = Package {
            name: PackageName(String::from("packages/auth")),
            root_path: String::from("/repo/packages/auth"),
        };

        let web_module = Module {
            id: ModuleId(String::from("web-home")),
            path: ModulePath(String::from("/repo/apps/web/src/home.ts")),
            package: web.name.clone(),
            kind: ModuleKind::Source,
            runtime: Runtime::Universal,
            layer: Some(LayerId("app".into())),
            framework_metadata: FrameworkMetadata::default(),
        };
        let ui_module = Module {
            id: ModuleId(String::from("ui-button")),
            path: ModulePath(String::from("/repo/packages/ui/src/button.ts")),
            package: ui.name.clone(),
            kind: ModuleKind::Source,
            runtime: Runtime::Universal,
            layer: Some(LayerId("shared".into())),
            framework_metadata: FrameworkMetadata::default(),
        };
        let auth_module = Module {
            id: ModuleId(String::from("auth-session")),
            path: ModulePath(String::from("/repo/packages/auth/src/session.ts")),
            package: auth.name.clone(),
            kind: ModuleKind::Source,
            runtime: Runtime::Universal,
            layer: Some(LayerId("shared".into())),
            framework_metadata: FrameworkMetadata::default(),
        };

        let project = ProjectBuilder::new()
            .add_package(web.clone())
            .add_package(ui.clone())
            .add_package(auth.clone())
            .add_module(web_module)
            .add_module(ui_module)
            .add_module(auth_module)
            .add_dependency(Dependency {
                from: ModuleId(String::from("web-home")),
                to: ModuleId(String::from("ui-button")),
                kind: DependencyKind::Static,
                location: SourceLocation::unknown(),
            })
            .add_dependency(Dependency {
                from: ModuleId(String::from("ui-button")),
                to: ModuleId(String::from("auth-session")),
                kind: DependencyKind::Static,
                location: SourceLocation::unknown(),
            })
            .build();

        let package_graph = PackageGraph::from_project(&project);

        assert_eq!(package_graph.nodes().len(), 3);
        assert_eq!(package_graph.edges().len(), 2);
        assert!(package_graph.has_edge(
            &PackageName(String::from("apps/web")),
            &PackageName(String::from("packages/ui"))
        ));
        assert!(package_graph.has_edge(
            &PackageName(String::from("packages/ui")),
            &PackageName(String::from("packages/auth"))
        ));
        assert!(!package_graph.has_edge(
            &PackageName(String::from("packages/auth")),
            &PackageName(String::from("apps/web"))
        ));
    }

    #[test]
    fn package_graph_detects_cross_package_cycles() {
        let a = Package { name: PackageName("a".into()), root_path: "/a".into() };
        let b = Package { name: PackageName("b".into()), root_path: "/b".into() };
        let project = ProjectBuilder::new()
            .add_package(a.clone())
            .add_package(b.clone())
            .add_module(module(&a, "a/index"))
            .add_module(module(&b, "b/index"))
            .add_dependency(dependency("a/index", "b/index"))
            .add_dependency(dependency("b/index", "a/index"))
            .build();
        let cycles = PackageGraph::from_project(&project).cycles();
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].first(), cycles[0].last());
    }
}
