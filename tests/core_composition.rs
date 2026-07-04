use htmlswap::{
    AdapterArtifact, AdapterContext, ArtifactBuilder, BundlePlan, ClassIndex, ComponentId,
    DependencySet, GeneratedFile, GeneratedStyleSource, LayerClaim, LayerId, ModuleChunk,
    ModuleEdge, ModuleEdgeKind, ModuleGraph, ModuleGraphOptions, ModuleNode, ModuleNodeId,
    OutputBoundary, OutputDependency, OutputSource, OutputUnit, OutputUnitId, OutputUnitKind,
    RouteConfig, RouteTarget, RouteTargetRef, Severity, SourceId, StyleOrigin, StyleProperty,
    StyleProviderOutput, TargetDependency, UiRole, parse_fragment,
};

#[test]
fn base_layer_resolves_any_target() {
    let routes = RouteConfig::new().with_base("core");

    assert_eq!(
        routes
            .resolve(&RouteTarget::Style(StyleProperty::Display))
            .map(LayerId::as_str),
        Some("core")
    );
}

#[test]
fn specialized_layer_wins_over_base_for_claimed_targets() {
    let routes = RouteConfig::new().with_base("core").with_layer(
        "components",
        LayerClaim::Targets(vec![RouteTarget::Role(UiRole::Button)]),
    );

    assert_eq!(
        routes
            .resolve(&RouteTarget::Role(UiRole::Button))
            .map(LayerId::as_str),
        Some("components")
    );
    assert_eq!(
        routes
            .resolve(&RouteTarget::Role(UiRole::Link))
            .map(LayerId::as_str),
        Some("core")
    );
}

#[test]
fn later_overlapping_layer_wins() {
    let routes = RouteConfig::new()
        .with_base("core")
        .with_layer(
            "component-v1",
            LayerClaim::Targets(vec![RouteTarget::Role(UiRole::Button)]),
        )
        .with_layer(
            "component-v2",
            LayerClaim::Targets(vec![RouteTarget::Role(UiRole::Button)]),
        );

    assert_eq!(
        routes
            .resolve(&RouteTarget::Role(UiRole::Button))
            .map(LayerId::as_str),
        Some("component-v2")
    );
}

#[test]
fn component_targets_route_like_other_targets() {
    let routes = RouteConfig::new().with_base("core").with_layer(
        "components",
        LayerClaim::Targets(vec![RouteTarget::component("titlebar")]),
    );

    assert_eq!(
        routes
            .resolve(&RouteTarget::component("titlebar"))
            .map(LayerId::as_str),
        Some("components")
    );
    let titlebar = ComponentId::new("titlebar");
    assert_eq!(
        routes
            .resolve_ref(RouteTargetRef::Component(&titlebar))
            .map(LayerId::as_str),
        Some("components")
    );

    let title_bar = ComponentId::new("Title-Bar");
    assert_eq!(
        routes
            .resolve_ref(RouteTargetRef::Component(&title_bar))
            .map(LayerId::as_str),
        Some("components")
    );
    assert_eq!(
        routes
            .resolve(&RouteTarget::component("Title-Bar"))
            .map(LayerId::as_str),
        Some("components")
    );
}

#[test]
fn region_targets_route_like_semantic_namespaces_not_paths() {
    let routes = RouteConfig::new().with_base("core").with_layer(
        "project-region",
        LayerClaim::Targets(vec![
            RouteTarget::region("editor.project").expect("valid region"),
        ]),
    );
    let region = htmlswap::RegionId::parse("Editor.Project").expect("valid region");

    assert_eq!(
        routes
            .resolve_ref(RouteTargetRef::Region(&region))
            .map(LayerId::as_str),
        Some("project-region")
    );
    assert!(RouteTarget::region("src/view.rs").is_none());
}

#[test]
fn class_targets_route_source_shapes_without_claiming_the_tag() {
    let routes = RouteConfig::new().with_base("core").with_layer(
        "components",
        LayerClaim::Targets(vec![RouteTarget::class("ms")]),
    );

    assert_eq!(
        routes
            .resolve_ref(RouteTargetRef::Class("ms"))
            .map(LayerId::as_str),
        Some("components")
    );
    assert_eq!(
        routes
            .resolve_first_ref(&[RouteTargetRef::Tag("span"), RouteTargetRef::Class("ms"),])
            .map(LayerId::as_str),
        Some("components")
    );
    assert_eq!(
        routes
            .resolve_ref(RouteTargetRef::Tag("span"))
            .map(LayerId::as_str),
        Some("core")
    );
}

#[test]
fn explicit_override_wins_over_layer_claims() {
    let routes = RouteConfig::new()
        .with_base("core")
        .with_layer(
            "components",
            LayerClaim::Targets(vec![RouteTarget::Role(UiRole::Button)]),
        )
        .with_override(RouteTarget::Role(UiRole::Button), "core");

    assert_eq!(
        routes
            .resolve(&RouteTarget::Role(UiRole::Button))
            .map(LayerId::as_str),
        Some("core")
    );
}

#[test]
fn resolve_first_uses_the_first_target_that_resolves() {
    let routes = RouteConfig::new().with_layer(
        "custom-elements",
        LayerClaim::Targets(vec![RouteTarget::tag("x-action")]),
    );

    assert_eq!(
        routes
            .resolve_first(&[
                RouteTarget::Role(UiRole::Button),
                RouteTarget::tag("x-action"),
            ])
            .map(LayerId::as_str),
        Some("custom-elements")
    );
}

#[test]
fn resolve_first_prefers_specific_claim_over_base_fallback() {
    let routes = RouteConfig::new().with_base("core").with_layer(
        "layout",
        LayerClaim::Targets(vec![RouteTarget::Style(StyleProperty::Display)]),
    );

    assert_eq!(
        routes
            .resolve_first(&[
                RouteTarget::Role(UiRole::Container),
                RouteTarget::Style(StyleProperty::Display),
            ])
            .map(LayerId::as_str),
        Some("layout")
    );
}

#[test]
fn borrowed_targets_resolve_like_owned_targets() {
    let routes = RouteConfig::new()
        .with_base("core")
        .with_layer(
            "layout",
            LayerClaim::Targets(vec![RouteTarget::Style(StyleProperty::Display)]),
        )
        .with_override(RouteTarget::event("click"), "core");
    let role = UiRole::Button;
    let style = StyleProperty::Display;
    let event = "click";

    assert_eq!(
        routes
            .resolve_first_ref(&[RouteTargetRef::Role(&role), RouteTargetRef::Style(&style),])
            .map(LayerId::as_str),
        Some("layout")
    );
    assert_eq!(
        routes
            .resolve_ref(RouteTargetRef::Event(event))
            .map(LayerId::as_str),
        Some("core")
    );
}

#[test]
fn unresolved_target_without_base_returns_none() {
    let routes = RouteConfig::new().with_layer(
        "buttons",
        LayerClaim::Targets(vec![RouteTarget::Role(UiRole::Button)]),
    );

    assert_eq!(routes.resolve(&RouteTarget::Role(UiRole::Link)), None);
}

#[test]
fn route_validation_warns_for_duplicate_layers_and_unknown_override_targets() {
    let routes = RouteConfig::new()
        .with_base("core")
        .with_base("core")
        .with_override(RouteTarget::event("click"), "missing");
    let mut cx = AdapterContext::new();

    routes.validate(&mut cx);

    let messages = diagnostic_messages(&cx);
    assert_eq!(cx.diagnostics().len(), 2);
    assert!(
        messages
            .iter()
            .any(|message| message.contains("duplicate layer"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("unknown layer"))
    );
    assert!(
        cx.diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.severity == Severity::Warning)
    );
}

#[test]
fn route_validation_warns_when_no_base_layer_exists() {
    let routes = RouteConfig::new().with_layer(
        "buttons",
        LayerClaim::Targets(vec![RouteTarget::Role(UiRole::Button)]),
    );
    let mut cx = AdapterContext::new();

    routes.validate(&mut cx);

    assert!(
        diagnostic_messages(&cx)
            .iter()
            .any(|message| message.contains("no base layer"))
    );
}

#[test]
fn dependency_set_merges_compatible_requirements() {
    let mut cx = AdapterContext::new();
    let mut dependencies = DependencySet::new();

    dependencies.insert(
        TargetDependency::crates_io("gpui", "0.2.2")
            .with_default_features(false)
            .with_features(["serde", "image"]),
        &mut cx,
    );
    dependencies.insert(
        TargetDependency::crates_io("gpui", "0.2.2").with_features(["image", "debug"]),
        &mut cx,
    );

    let dependencies = dependencies.into_vec();
    assert!(cx.diagnostics().is_empty());
    assert_eq!(dependencies.len(), 1);
    assert_eq!(dependencies[0].package, "gpui");
    assert!(!dependencies[0].default_features);
    assert_eq!(dependencies[0].features, ["debug", "image", "serde"]);
}

#[test]
fn dependency_set_warns_and_keeps_first_conflicting_requirement() {
    let mut cx = AdapterContext::new();
    let mut dependencies = DependencySet::new();

    dependencies.insert(TargetDependency::crates_io("gpui", "0.2.2"), &mut cx);
    dependencies.insert(TargetDependency::crates_io("gpui", "0.3.0"), &mut cx);

    let dependencies = dependencies.into_vec();
    assert_eq!(dependencies.len(), 1);
    assert_eq!(dependencies[0].version_req, "0.2.2");
    assert!(
        diagnostic_messages(&cx)
            .iter()
            .any(|message| message.contains("conflicting requirements"))
    );
}

#[test]
fn artifact_builder_deduplicates_files_and_warns_on_conflicting_contents() {
    let mut cx = AdapterContext::new();
    let mut builder = ArtifactBuilder::new();

    builder.add_file(GeneratedFile::new("view.rs", "first"), &mut cx);
    builder.add_file(GeneratedFile::new("view.rs", "first"), &mut cx);
    builder.add_file(GeneratedFile::new("view.rs", "second"), &mut cx);

    let artifact = builder.finish();
    assert_eq!(artifact.files.len(), 1);
    assert_eq!(artifact.files[0].contents, "first");
    assert!(
        diagnostic_messages(&cx)
            .iter()
            .any(|message| message.contains("conflicting contents"))
    );
}

#[test]
fn artifact_builder_merges_artifact_dependencies() {
    let mut cx = AdapterContext::new();
    let mut builder = AdapterArtifact::builder();

    builder.add_artifact(
        AdapterArtifact::new(
            vec![GeneratedFile::new("a.rs", "pub struct A;")],
            vec![TargetDependency::crates_io("gpui", "0.2.2").with_features(["a"])],
        ),
        &mut cx,
    );
    builder.add_dependency(
        TargetDependency::crates_io("gpui", "0.2.2").with_features(["b"]),
        &mut cx,
    );

    let artifact = builder.finish();
    assert!(cx.diagnostics().is_empty());
    assert_eq!(artifact.files.len(), 1);
    assert_eq!(artifact.dependencies.len(), 1);
    assert_eq!(artifact.dependencies[0].features, ["a", "b"]);
}

#[test]
fn bundle_plan_tracks_output_units_and_edges() {
    let view = OutputUnit::new(
        OutputUnitId::new(0),
        OutputUnitKind::View,
        OutputBoundary::SameModule,
        OutputSource::single(SourceId::primary()),
    );
    let action = OutputUnit::new(
        OutputUnitId::new(1),
        OutputUnitKind::Action,
        OutputBoundary::SeparateModule {
            path_hint: Some("actions".to_owned()),
        },
        OutputSource::default(),
    );
    let mut plan = BundlePlan::new();

    plan.push_unit(view);
    plan.push_unit(action);
    plan.push_edge(OutputDependency::new(
        OutputUnitId::new(0),
        OutputUnitId::new(1),
    ));

    assert_eq!(plan.units.len(), 2);
    assert_eq!(plan.edges[0].from.index(), 0);
    assert!(matches!(
        plan.units[1].boundary,
        OutputBoundary::SeparateModule { .. }
    ));
}

#[test]
fn class_index_collects_html_class_candidates() {
    let parsed = parse_fragment(r#"<div class="card primary"><span class="label"></span></div>"#);
    let classes = ClassIndex::collect(&parsed.value, SourceId::primary());

    assert_eq!(
        classes.names().collect::<Vec<_>>(),
        ["card", "primary", "label"]
    );
    assert!(
        classes
            .candidates()
            .iter()
            .all(|candidate| candidate.span.is_some())
    );
}

#[test]
fn module_graph_and_style_provider_outputs_are_plain_data() {
    let module_a = ModuleNodeId::new(0);
    let module_b = ModuleNodeId::new(1);
    let graph = ModuleGraph {
        modules: vec![
            ModuleNode::new(module_a, SourceId::primary(), Some("app.ts".to_owned())),
            ModuleNode::new(module_b, SourceId::new(1), Some("./actions.ts".to_owned())),
        ],
        edges: vec![ModuleEdge::new(module_a, module_b, ModuleEdgeKind::Import)],
        chunks: vec![ModuleChunk::new(
            Some("app".to_owned()),
            Some(module_a),
            vec![module_a, module_b],
        )],
    };
    let options = ModuleGraphOptions::new().with_code_splitting(true);
    let styles = StyleProviderOutput::new().with_stylesheet(GeneratedStyleSource::new(
        "tailwind:generated.css",
        ".card{display:flex}",
    ));
    let origin = StyleOrigin {
        declaration_span: None,
        selector_span: None,
        candidate_span: Some(htmlswap::Span::primary(12, 16)),
    };

    assert!(options.code_splitting);
    assert_eq!(graph.edges[0].kind, ModuleEdgeKind::Import);
    assert_eq!(styles.stylesheets[0].name, "tailwind:generated.css");
    assert_eq!(origin.candidate_span.expect("candidate span").start, 12);
}

fn diagnostic_messages(cx: &AdapterContext) -> Vec<&str> {
    cx.diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect()
}
