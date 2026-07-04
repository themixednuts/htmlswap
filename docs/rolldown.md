# Rolldown Integration

Rolldown should be the preferred JavaScript and TypeScript bundler backend for htmlswap when we need real bundler behavior.

It should not replace htmlswap's core bundle plan. Core owns target-neutral source graphs, output units, source maps, and adapter boundaries. Rolldown is an optional implementation for JS/TS module graph resolution and bundling.

## Why Rolldown

Rolldown is a Rust-based JavaScript/TypeScript bundler with Rollup-compatible APIs and plugin behavior. It is also positioned as Vite's future bundler and is powered by the same Oxc ecosystem that htmlswap already uses for script parsing.

That makes it a good fit for:

- JS/TS module graph resolution.
- ESM and CJS interop.
- Code splitting and chunk metadata.
- Source maps for bundled JavaScript.
- Rollup-compatible plugin behavior.
- Future web-target adapters.
- Reverse/import workflows involving bundled JS artifacts.

## Boundary

Use Rolldown behind a neutral core trait:

```rust
pub trait ModuleGraphProvider {
    fn graph(&self, input: ModuleGraphInput<'_>) -> Compilation<ModuleGraph>;
}

pub struct ModuleGraphInput<'a> {
    pub entries: &'a [SourceId],
    pub sources: &'a SourceMap,
    pub resources: &'a dyn ResourceResolver,
    pub options: &'a ModuleGraphOptions,
}

pub struct ModuleGraph {
    pub modules: Vec<ModuleNode>,
    pub edges: Vec<ModuleEdge>,
    pub chunks: Vec<ModuleChunk>,
}
```

The compiler should depend on `ModuleGraphProvider`, not directly on Rolldown APIs. A `RolldownGraphProvider` can implement the trait behind a feature flag.

```rust
#[cfg(feature = "rolldown")]
pub struct RolldownGraphProvider {
    pub options: RolldownOptions,
}
```

## Feature Shape

Use an optional feature:

```toml
[features]
rolldown = ["dep:rolldown"]
```

Prefer the native Rust crate when its API is stable enough for our needs. If a required capability is only exposed through the Node/CLI API at the time we implement it, keep that behind the same provider boundary so the integration can move native later without changing compiler or adapter APIs.

## Forward Compilation

During normal HTML-to-target compilation, Rolldown should be used only when the job requires bundler semantics:

- Multiple JS/TS entrypoints.
- Package imports.
- CJS/ESM interop.
- Code splitting decisions.
- Plugin-driven transforms.
- Adapter output that needs bundled web assets.

Simple inline scripts and directly linked local scripts can stay on the existing Oxc parser path.

## Import / Reverse Workflows

For target-to-HTML import, Rolldown helps when the target artifact includes bundled JavaScript or sourcemaps:

```text
target artifact
  -> TargetArtifact files
  -> Rolldown or sourcemap-aware module graph provider
  -> ModuleGraph
  -> action/resource provenance
  -> HtmlArtifact or CompiledFragment reconstruction
```

Rolldown should not be expected to reverse GPUI Rust source into HTML. That path needs Rust source analysis. Rolldown is for JavaScript and TypeScript assets that participate in the imported target artifact.

## Bundle Plan Relationship

`BundlePlan` remains the source of truth for output units. Rolldown can propose or resolve JS chunks, but core decides how those chunks attach to htmlswap output units.

Example:

```text
BundlePlan OutputUnit::Script
  -> Rolldown entry
  -> ModuleGraph chunks
  -> GeneratedFile assets
  -> AdapterArtifact
```

Adapters should receive the resolved bundle plan and generated files. They should not call Rolldown directly unless an adapter is explicitly taking ownership of a target-specific web bundling step.

## Non-Goals

- Do not make Rolldown a hard dependency of core.
- Do not model all htmlswap resources as Rolldown modules.
- Do not use Rolldown for Rust/GPUI source import.
- Do not leak Rolldown-specific chunk structures into adapter APIs.

## References

- Rolldown docs: https://rolldown.rs/
- Rolldown bundler API: https://rolldown.rs/apis/bundler-api
- Rolldown plugin API: https://rolldown.rs/apis/plugin-api
- Rolldown GitHub: https://github.com/rolldown/rolldown
- Vite Rolldown guide: https://vite.dev/guide/rolldown
