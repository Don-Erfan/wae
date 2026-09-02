use wae_parser::ParserAdapter;

use super::*;

/// Immutable input context for a single pipeline execution.
struct AnalysisContext<'engine, P> {
    engine: &'engine Engine<P>,
    request: AnalyzeRequest,
}

/// Typed boundary between project loading/discovery and semantic analysis. Keeping this state
/// explicit prevents later stages from quietly repeating filesystem discovery.
struct DiscoveredWorkspace {
    root: PathBuf,
    config: Config,
    files: Vec<PathBuf>,
    analysis_inputs: Vec<PathBuf>,
}

fn discover_stage(
    requested_root: PathBuf,
    config_path: Option<PathBuf>,
    cache_enabled: Option<bool>,
    known_files: Option<Vec<PathBuf>>,
    environment_is_known: bool,
    overlay_modules: impl Iterator<Item = String>,
) -> Result<DiscoveredWorkspace, AnalysisError> {
    let root = requested_root
        .canonicalize()
        .map_err(|error| AnalysisError::Project(format!("cannot open project root: {error}")))?;
    let mut config = match config_path {
        Some(path) => {
            let path = if path.is_absolute() { path } else { root.join(path) };
            Config::load_file(&path).map_err(AnalysisError::Config)?
        }
        None => Config::load(&root).map_err(AnalysisError::Config)?,
    };
    if let Some(enabled) = cache_enabled {
        config.cache.enabled = enabled;
    }
    let (mut files, analysis_inputs) = match known_files {
        Some(files) if environment_is_known => (files, Vec::new()),
        Some(files) => (files, discovery::discover_analysis_inputs(&root, &config)?),
        None => {
            let discovery = discovery::discover_project(&root, &config)?;
            (discovery.modules, discovery.analysis_inputs)
        }
    };
    for module in overlay_modules {
        let path = root.join(module);
        if !files.contains(&path) {
            files.push(path);
        }
    }
    files.sort();
    files.dedup();
    Ok(DiscoveredWorkspace { root, config, files, analysis_inputs })
}

impl<P: ParserAdapter> AnalysisContext<'_, P> {
    fn run(self) -> Result<Analysis, AnalysisError> {
        execute(self.engine, self.request)
    }
}

/// Executes the ordered analysis phases for one immutable request context.
pub(crate) fn analyze<P: ParserAdapter>(
    engine: &Engine<P>,
    request: AnalyzeRequest,
) -> Result<Analysis, AnalysisError> {
    AnalysisContext { engine, request }.run()
}

fn execute<P: ParserAdapter>(
    engine: &Engine<P>,
    request: AnalyzeRequest,
) -> Result<Analysis, AnalysisError> {
    let total_started = std::time::Instant::now();
    let mut telemetry = PipelineTelemetry::default();
    let AnalyzeRequest {
        root: requested_root,
        config_path,
        cache_enabled,
        overlays,
        known_files,
        known_environment_hash,
        cancellation,
    } = request;
    if cancellation.is_cancelled() {
        return Err(AnalysisError::Cancelled);
    }
    let discovery_started = std::time::Instant::now();
    let discovered = discover_stage(
        requested_root,
        config_path,
        cache_enabled,
        known_files,
        known_environment_hash.is_some(),
        overlays.keys().cloned(),
    )?;
    let DiscoveredWorkspace { root, config, files, analysis_inputs } = discovered;
    let architecture = CompiledArchitectureModel::compile(&config)?;
    let framework_registry = FrameworkRegistry::default();
    let framework_evidence = framework_project_evidence(&root)?;
    let framework_adapter = framework_registry.select(
        &framework_evidence,
        &config.framework.enabled,
        config.framework.auto_detect,
    );
    let tsconfigs = TsConfigIndex::discover(&root).map_err(AnalysisError::Project)?;
    let workspace_resolver =
        WorkspacePackageIndex::discover(&root).map_err(AnalysisError::Project)?;
    let package_scopes =
        PackageScopeIndex::from_importers(&root, &files).map_err(AnalysisError::Project)?;
    let workspace_packages = workspace_resolver.packages().to_vec();
    let declared_package_dependencies = workspace_packages
        .iter()
        .map(|package| {
            (
                PackageName(package.name.clone()),
                package
                    .declared_dependencies
                    .iter()
                    .cloned()
                    .map(PackageName)
                    .collect::<HashSet<_>>(),
            )
        })
        .collect::<HashMap<_, _>>();
    let module_formats = ModuleFormatResolver::new(&package_scopes);
    let resolver = ResolverPipeline::indexed_node_with_workspaces(
        tsconfigs,
        workspace_resolver,
        config.resolution.mode,
    );
    let default_package =
        Package { name: PackageName(project_name(&root)), root_path: normalize(&root) };
    let mut project = Project::default();
    let mut discovered_packages = HashMap::<PackageName, Package>::new();
    let mut layers = HashMap::new();
    let mut ownership = ArchitectureOwnershipIndex::default();
    let mut features = HashMap::new();
    let mut feature_roots = HashMap::new();
    let live_cache_files = files.iter().map(|path| relative_path(&root, path)).collect();
    let mut cache = PipelineTelemetry::measure(&mut telemetry.cache, || {
        AnalysisCache::load(&root, &config, live_cache_files)
    })?;
    let environment_hash = match known_environment_hash {
        Some(hash) => hash,
        None => analysis_environment_hash(&root, &config, analysis_inputs)?,
    };
    let mut incremental =
        IncrementalStats { cache_enabled: cache.enabled(), environment_hash, ..Default::default() };
    let mut suppressions = Vec::new();
    telemetry.discovery = discovery_started.elapsed().saturating_sub(telemetry.cache);

    for path in &files {
        if cancellation.is_cancelled() {
            return Err(AnalysisError::Cancelled);
        }
        let relative = relative_path(&root, path);
        let id = ModuleId(relative.clone());
        ownership.insert(id.clone(), architecture.ownership(&relative));
        let package = infer_package(&root, path, &workspace_packages, &default_package);
        discovered_packages.entry(package.name.clone()).or_insert_with(|| package.clone());
        let layer_name = architecture.layer(&relative)?;
        if let Some(value) = &layer_name {
            layers.insert(id.clone(), value.clone());
        }
        let package_root = relative_resolved_path(&root, &package.root_path);
        if let Some((feature, feature_root)) =
            architecture.feature(&relative, &package, &package_root)
        {
            features.insert(id.clone(), feature);
            feature_roots.insert(id.clone(), feature_root);
        }
        project.modules.push(Module {
            id: id.clone(),
            path: ModulePath(id.0.clone()),
            package: package.name.clone(),
            kind: ModuleKind::Source,
            runtime: Runtime::Universal,
            layer: layer_name.map(LayerId),
            framework_metadata: FrameworkMetadata::default(),
        });
    }

    project.packages = discovered_packages.into_values().collect();
    project.packages.sort_by(|a, b| a.name.0.cmp(&b.name.0));
    let mut project_index = ProjectIndex::from_project(&project);

    for path in &files {
        if cancellation.is_cancelled() {
            return Err(AnalysisError::Cancelled);
        }
        let module_path = ModulePath(normalize(path));
        let module_id = ModuleId(relative_path(&root, path));
        let source = match overlays
            .get(&module_id.0)
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| fs::read_to_string(path))
        {
            Ok(source) => source,
            Err(error) => {
                project.diagnostics.push(simple_diagnostic(
                    "PARSE-001",
                    format!("Cannot read source: {error}"),
                    &module_id.0,
                ));
                continue;
            }
        };
        let source_hash = stable_hash(source.as_bytes());
        suppression::collect(
            &module_id.0,
            &source,
            config.suppressions.require_reason,
            &mut suppressions,
            &mut project.diagnostics,
        );
        if let Some(cached) = cache.get(&module_id.0, source_hash, environment_hash) {
            incremental.restored_modules += 1;
            PipelineTelemetry::measure(&mut telemetry.classification, || {
                apply_framework_classification(
                    &mut project,
                    &module_id,
                    framework_adapter,
                    &cached.semantics,
                );
            });
            restore_cached_module(
                cached,
                &root,
                &workspace_packages,
                &default_package,
                framework_adapter,
                &architecture,
                &mut project,
                &mut project_index,
                &mut layers,
                &mut features,
                &mut feature_roots,
            )?;
            continue;
        }
        incremental.analyzed_modules += 1;
        let imports_start = project.imports.len();
        let dependencies_start = project.dependencies.len();
        let resolved_start = project.resolved_dependencies.len();
        let diagnostics_start = project.diagnostics.len();
        let parsed = PipelineTelemetry::measure(&mut telemetry.parsing, || {
            engine.parser.parse_module(&module_path, &source)
        });
        let semantics = match parsed {
            Ok(parsed) => {
                PipelineTelemetry::measure(&mut telemetry.classification, || {
                    apply_framework_classification(
                        &mut project,
                        &module_id,
                        framework_adapter,
                        &parsed.semantics,
                    );
                });
                for mut import in parsed.imports {
                    if cancellation.is_cancelled() {
                        return Err(AnalysisError::Cancelled);
                    }
                    import.module_id = module_id.clone();
                    import.location.file = module_id.0.clone();
                    let candidate = wae_core::domain::DependencyCandidate::from(import.clone());
                    let importer_format = module_formats.resolve(path);
                    let resolution_kind = resolution_kind_for(
                        config.resolution.mode,
                        &candidate.kind,
                        importer_format,
                    );
                    let resolution_request = ResolutionRequest {
                        importer: &module_path,
                        specifier: &import.specifier,
                        dependency_kind: candidate.kind.clone(),
                        resolution_kind,
                        importer_format,
                        mode: config.resolution.mode,
                        custom_conditions: &config.resolution.custom_conditions,
                    };
                    let (candidate_paths, resolution) =
                        PipelineTelemetry::measure(&mut telemetry.resolution, || {
                            let candidates = resolver
                                .candidate_paths(&resolution_request)
                                .into_iter()
                                .map(|path| relative_resolved_path(&root, &path.0))
                                .collect::<Vec<_>>();
                            (candidates, resolver.resolve(&resolution_request))
                        });
                    match resolution {
                        Resolution::Module(target) => {
                            let target_id = ModuleId(relative_resolved_path(&root, &target.0));
                            let target_kind = workspace_packages
                                .iter()
                                .filter(|package| {
                                    normalized_path_is_within(&target.0, &package.root)
                                })
                                .max_by_key(|package| package.root.components().count())
                                .map_or_else(
                                    || DependencyTarget::Internal(target_id.clone()),
                                    |package| DependencyTarget::WorkspacePackage {
                                        package: PackageName(package.name.clone()),
                                        module: target_id.clone(),
                                    },
                                );
                            if project_index.insert_module(target_id.clone()) {
                                let target_path = root.join(&target_id.0);
                                let package = infer_package(
                                    &root,
                                    &target_path,
                                    &workspace_packages,
                                    &default_package,
                                );
                                if project_index.insert_package(package.name.clone()) {
                                    project.packages.push(package.clone());
                                }
                                let layer = architecture.layer(&target_id.0)?;
                                if let Some(value) = &layer {
                                    layers.insert(target_id.clone(), value.clone());
                                }
                                let package_root =
                                    relative_resolved_path(&root, &package.root_path);
                                if let Some((feature, feature_root)) =
                                    architecture.feature(&target_id.0, &package, &package_root)
                                {
                                    features.insert(target_id.clone(), feature);
                                    feature_roots.insert(target_id.clone(), feature_root);
                                }
                                let semantics = ModuleSemantics::default();
                                let classification = PipelineTelemetry::measure(
                                    &mut telemetry.classification,
                                    || {
                                        framework_adapter.map(|adapter| {
                                            adapter.classify(ModuleEvidence {
                                                path: &target_id.0,
                                                semantics: &semantics,
                                            })
                                        })
                                    },
                                );
                                project.modules.push(Module {
                                    id: target_id.clone(),
                                    path: ModulePath(target_id.0.clone()),
                                    package: package.name,
                                    kind: ModuleKind::Excluded,
                                    runtime: classification
                                        .as_ref()
                                        .map_or(Runtime::Unknown, |value| value.runtime),
                                    layer: layer.map(LayerId),
                                    framework_metadata: classification
                                        .map_or_else(FrameworkMetadata::default, |value| {
                                            value.metadata
                                        }),
                                });
                            }
                            project.resolved_dependencies.push(ResolvedDependency {
                                from: module_id.clone(),
                                specifier: import.specifier.clone(),
                                kind: candidate.kind.clone(),
                                target: target_kind,
                                location: import.location.clone(),
                            });
                            project.dependencies.push(Dependency {
                                from: module_id.clone(),
                                to: target_id,
                                kind: candidate.kind,
                                location: import.location.clone(),
                            });
                        }
                        Resolution::External(name) => {
                            let external_id = ModuleId(format!("external:{name}"));
                            if project_index.insert_module(external_id.clone()) {
                                let external_package = PackageName(name.clone());
                                project.modules.push(Module {
                                    id: external_id.clone(),
                                    path: ModulePath(format!("external:{name}")),
                                    package: external_package.clone(),
                                    kind: ModuleKind::External,
                                    runtime: Runtime::Unknown,
                                    layer: None,
                                    framework_metadata: FrameworkMetadata::default(),
                                });
                                if project_index.insert_package(external_package.clone()) {
                                    project.packages.push(Package {
                                        name: external_package,
                                        root_path: String::new(),
                                    });
                                }
                            }
                            project.resolved_dependencies.push(ResolvedDependency {
                                from: module_id.clone(),
                                specifier: import.specifier.clone(),
                                kind: candidate.kind.clone(),
                                target: DependencyTarget::ExternalPackage(PackageName(name)),
                                location: import.location.clone(),
                            });
                            project.dependencies.push(Dependency {
                                from: module_id.clone(),
                                to: external_id,
                                kind: candidate.kind,
                                location: import.location.clone(),
                            });
                        }
                        Resolution::Builtin(name) => {
                            let builtin_id = ModuleId(format!("builtin:{name}"));
                            if project_index.insert_module(builtin_id.clone()) {
                                let builtin_package = PackageName("node".into());
                                project.modules.push(Module {
                                    id: builtin_id.clone(),
                                    path: ModulePath(format!("builtin:{name}")),
                                    package: builtin_package.clone(),
                                    kind: ModuleKind::External,
                                    runtime: Runtime::Node,
                                    layer: None,
                                    framework_metadata: FrameworkMetadata::default(),
                                });
                                if project_index.insert_package(builtin_package.clone()) {
                                    project.packages.push(Package {
                                        name: builtin_package,
                                        root_path: String::new(),
                                    });
                                }
                            }
                            project.resolved_dependencies.push(ResolvedDependency {
                                from: module_id.clone(),
                                specifier: import.specifier.clone(),
                                kind: candidate.kind.clone(),
                                target: DependencyTarget::Builtin(name),
                                location: import.location.clone(),
                            });
                            project.dependencies.push(Dependency {
                                from: module_id.clone(),
                                to: builtin_id,
                                kind: candidate.kind,
                                location: import.location.clone(),
                            });
                        }
                        Resolution::Unresolved => {
                            project.resolved_dependencies.push(ResolvedDependency {
                                from: module_id.clone(),
                                specifier: import.specifier.clone(),
                                kind: candidate.kind,
                                target: DependencyTarget::Unresolved {
                                    specifier: import.specifier.clone(),
                                    reason: "no resolver in the configured chain produced a module"
                                        .into(),
                                },
                                location: import.location.clone(),
                            });
                            let mut diagnostic = unresolved_diagnostic(&import);
                            if !candidate_paths.is_empty() {
                                diagnostic.metadata.insert(
                                    "candidatePaths".into(),
                                    serde_json::to_string(&candidate_paths).map_err(|error| {
                                        AnalysisError::Internal(error.to_string())
                                    })?,
                                );
                                diagnostic.refresh_fingerprint();
                            }
                            project.diagnostics.push(diagnostic)
                        }
                        Resolution::Invalid(reason) => {
                            project.resolved_dependencies.push(ResolvedDependency {
                                from: module_id.clone(),
                                specifier: import.specifier.clone(),
                                kind: candidate.kind.clone(),
                                target: DependencyTarget::Unresolved {
                                    specifier: import.specifier.clone(),
                                    reason: reason.clone(),
                                },
                                location: import.location.clone(),
                            });
                            let mut diagnostic =
                                simple_diagnostic("RESOLVE-002", reason, &module_id.0);
                            diagnostic.primary_location = Some(import.location.clone());
                            diagnostic.refresh_fingerprint();
                            project.diagnostics.push(diagnostic);
                        }
                        Resolution::Redirect(target) => {
                            return Err(AnalysisError::Internal(format!(
                                "resolver leaked redirect `{target}` out of its pipeline"
                            )));
                        }
                    }
                    project.dependency_candidates.push(import.clone().into());
                    project.imports.push(import);
                }
                parsed.semantics
            }
            Err(error) => {
                PipelineTelemetry::measure(&mut telemetry.classification, || {
                    apply_framework_classification(
                        &mut project,
                        &module_id,
                        framework_adapter,
                        &ModuleSemantics::default(),
                    );
                });
                let mut diagnostic = simple_diagnostic("PARSE-001", error.message, &module_id.0);
                diagnostic.primary_location = error.location.or_else(|| {
                    Some(SourceLocation { file: module_id.0.clone(), line: 1, column: 1 })
                });
                diagnostic.refresh_fingerprint();
                project.diagnostics.push(diagnostic);
                ModuleSemantics::default()
            }
        };
        cache.insert(
            module_id.0.clone(),
            source_hash,
            environment_hash,
            CachedModuleAnalysis {
                hash: source_hash,
                environment_hash,
                imports: project.imports[imports_start..].to_vec(),
                dependencies: project.dependencies[dependencies_start..].to_vec(),
                resolved_dependencies: project.resolved_dependencies[resolved_start..].to_vec(),
                diagnostics: project.diagnostics[diagnostics_start..].to_vec(),
                semantics,
                resolved_paths: Vec::new(),
            },
        );
    }

    project.packages.sort_by(|a, b| a.name.0.cmp(&b.name.0));
    project.modules.sort_by(|a, b| a.id.0.cmp(&b.id.0));
    project.dependencies.sort_by(|a, b| (&a.from.0, &a.to.0).cmp(&(&b.from.0, &b.to.0)));
    if cancellation.is_cancelled() {
        return Err(AnalysisError::Cancelled);
    }
    let (graph, package_graph, runtime_graph) =
        PipelineTelemetry::measure(&mut telemetry.graph_build, || {
            (
                ModuleGraph::from_project(&project),
                PackageGraph::from_project(&project),
                RuntimeGraph::from_project(&project),
            )
        });
    let rule_policies = PipelineTelemetry::measure(&mut telemetry.rule_evaluation, || {
        CompiledRulePolicies::compile(&config).map_err(AnalysisError::Internal)
    })?;
    let context = RuleContext {
        project: &project,
        graph: &graph,
        package_graph: &package_graph,
        runtime_graph: &runtime_graph,
        config: &config,
        module_layers: &layers,
        ownership: &ownership,
        module_features: &features,
        module_feature_roots: &feature_roots,
        policies: &rule_policies,
        declared_package_dependencies: &declared_package_dependencies,
    };
    let mut diagnostics = project.diagnostics.clone();
    let graph_hash = analysis_graph_hash(&project, environment_hash)?;
    let (rule_diagnostics, rule_profiles) = PipelineTelemetry::measure(
        &mut telemetry.rule_evaluation,
        || -> Result<_, AnalysisError> {
            if let Some(diagnostics) = cache.rule_diagnostics(graph_hash) {
                incremental.rule_snapshot_reused = true;
                Ok((diagnostics, Vec::new()))
            } else {
                let evaluation =
                    engine.rules.evaluate_profiled(&context).map_err(AnalysisError::Internal)?;
                cache.set_rule_diagnostics(graph_hash, evaluation.diagnostics.clone());
                Ok((evaluation.diagnostics, evaluation.profiles))
            }
        },
    )?;
    if cancellation.is_cancelled() {
        return Err(AnalysisError::Cancelled);
    }
    PipelineTelemetry::measure(&mut telemetry.rule_evaluation, || {
        diagnostics.extend(rule_diagnostics);
        diagnostics = DiagnosticArbitrator::arbitrate(std::mem::take(&mut diagnostics));
        suppression::apply(&mut diagnostics, &mut suppressions, config.suppressions.report_unused);
        suppression::apply_config(&mut diagnostics, &config.suppressions);
        diagnostics.sort_by(|a, b| diagnostic_key(a).cmp(&diagnostic_key(b)));
    });
    if cancellation.is_cancelled() {
        return Err(AnalysisError::Cancelled);
    }
    PipelineTelemetry::measure(&mut telemetry.cache, || cache.save())?;
    let mut timings = telemetry.finish(total_started.elapsed());
    timings.rules = rule_profiles
        .into_iter()
        .map(|profile| {
            (
                profile.rule_id.to_owned(),
                RuleTiming { elapsed_ns: profile.elapsed_ns, diagnostics: profile.diagnostics },
            )
        })
        .collect();
    Ok(Analysis {
        schema_version: OUTPUT_SCHEMA_VERSION,
        project,
        graph,
        ownership,
        diagnostics,
        failure_policy: crate::FailurePolicy::from_output(&config.output),
        incremental,
        timings,
    })
}
