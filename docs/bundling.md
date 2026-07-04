# Module And Bundling Plan

htmlswap should be more than a parser-to-emitter compiler. Once compilation can follow HTML, CSS, JavaScript, images, generated CSS, and adapter outputs, it also needs bundling primitives.

This should not mean becoming a full JavaScript bundler. Core should own the target-neutral resource graph and output planning. Adapters should own target-language code generation.

## Compiler Boundary

Core should produce:

- A render IR.
- A component graph for reusable definitions, usage sites, contracts, and
  source or target ownership.
- A source graph for HTML, CSS, JavaScript, and resolved resources.
- A style graph after authored and generated CSS are parsed.
- An action graph from inline and external scripts.
- A bundle plan that says which output units exist and how they depend on each other.

Adapters should consume that plan and emit target-specific artifacts.

## Output Units

Adapters need to know whether a thing should be emitted inline, in the current file, or in a separate module. That should be represented directly.

```rust
pub struct BundlePlan {
    pub units: Vec<OutputUnit>,
    pub edges: Vec<OutputDependency>,
}

pub struct OutputUnit {
    pub id: OutputUnitId,
    pub kind: OutputUnitKind,
    pub boundary: OutputBoundary,
    pub source: OutputSource,
}

pub enum OutputUnitKind {
    View,
    Component,
    Style,
    Script,
    Asset,
    State,
    Action,
}

pub enum OutputBoundary {
    Inline,
    SameModule,
    SeparateModule { path_hint: Option<String> },
    External { specifier: String },
}
```

The key idea is that module splitting is a plan decision. The GPUI adapter may turn `SeparateModule` into a Rust module file. Another adapter may turn the same boundary into a package file, component file, or external artifact.

## Planning Inputs

The planner should consider:

- HTML source boundaries and file names.
- Semantic source hints such as `data-htmlswap-component`, slots, roles, keys,
  variants, tones, sizes, densities, and inherited `data-htmlswap-region`
  namespaces.
- Script imports and action ownership.
- Stateful controls and generated state structs.
- CSS source files, generated Tailwind CSS, and whether styles are target-native or need comments/fallbacks.
- CLI options such as single-file output, preserve source modules, output directory, module naming, and asset copying.

## Boundary Config

HTML should not request output files or target adapter layers. Source only
describes the UI; bundling configuration decides output placement.

`BundleConfig` maps route targets to output boundaries:

```rust
BundleConfig::new()
    .with_boundary(
        RouteTarget::region("project.manager").expect("valid source region"),
        OutputUnitKind::Action,
        OutputBoundary::SeparateModule {
            path_hint: Some("project_manager_actions.rs".to_owned()),
        },
    )
    .with_boundary(
        RouteTarget::component("profile-card"),
        OutputUnitKind::Component,
        OutputBoundary::SeparateModule {
            path_hint: Some("profile_card.rs".to_owned()),
        },
    )
    .with_boundary(
        RouteTarget::event("click"),
        OutputUnitKind::Action,
        OutputBoundary::SeparateModule {
            path_hint: Some("actions.rs".to_owned()),
        },
    );
```

This keeps paths, module names, and output partitioning in compiler/CLI config
where target-specific knowledge belongs.

Component definitions and usages are covered in
[`components.md`](components.md). Component output units should be planned from
that graph rather than inferred from adapter-specific strings.

## Bundler Scope

Core should bundle:

- Resolved source files.
- Generated target files.
- Source maps.
- Dependency metadata.
- Asset copy/link decisions.
- Module import/export relationships.

Core should not try to replace mature JS bundlers. If JavaScript needs target-specific bundling, minification, tree shaking, or package resolution beyond the HTML graph, expose the graph and let a dedicated tool or adapter integration handle it.

## Relation To Existing Adapters

GPUI and gpui-components already return `AdapterArtifact`. That is the final artifact container. `BundlePlan` should be the missing compiler-side plan that tells adapters how to populate that artifact cleanly.

The current first-class `Importer` API should eventually consume the same artifact and bundle metadata so target-to-HTML import can recover module boundaries when possible.
