# Tailwind Composition

Tailwind support should be modeled as a style expansion pass, not as a UI adapter.

The compiler pipeline should stay shaped like this:

```text
HTML, CSS, and JavaScript resources
  -> parsers and source maps
  -> class and source candidate collection
  -> Tailwind style provider
  -> generated CSS sources
  -> Lightning CSS parsing
  -> selector/style mapping onto render IR
  -> target adapters such as GPUI and gpui-components
```

Adapters should not need Tailwind-specific branches. GPUI and gpui-components should receive the same target-neutral style declarations, style variants, pseudo-elements, annotations, and source metadata they already consume for authored CSS.

## Core Primitive

Add a style provider abstraction that can contribute generated stylesheet sources before the existing stylesheet graph is parsed and matched.

```rust
pub trait StyleProvider {
    fn expand(
        &self,
        input: StyleProviderInput<'_>,
        cx: &mut CompileContext,
    ) -> Compilation<StyleProviderOutput>;
}

pub struct StyleProviderInput<'a> {
    pub sources: &'a SourceMap,
    pub document: &'a HtmlDocument,
    pub classes: &'a ClassIndex,
    pub resources: &'a dyn ResourceResolver,
}

pub struct StyleProviderOutput {
    pub stylesheets: Vec<GeneratedStyleSource>,
    pub annotations: Vec<RenderAnnotation>,
}
```

`GeneratedStyleSource` should be registered in `SourceMap` with a stable source name, then parsed through Lightning CSS like any other stylesheet. This keeps cascade, source maps, comments, media queries, pseudo-classes, and pseudo-elements on the same path as authored CSS.

## Tailwind Provider

Tailwind should be one implementation of `StyleProvider`.

```rust
pub struct TailwindProvider {
    pub config: TailwindConfigSource,
    pub safelist: Vec<String>,
    pub source_mode: TailwindSourceMode,
}
```

The provider should collect candidates from:

- HTML `class` attributes, including each token span.
- CSS or Tailwind directives that affect generation.
- JavaScript string literals when configured to scan scripts.
- Explicit safelist entries for generated class names that do not appear literally in source.
- Explicit source roots or source files when the CLI expands compilation beyond the primary HTML graph.

## Provenance

Tailwind expands one source token into generated CSS, so a single declaration span is not enough for high-quality diagnostics and target source maps. The style IR should grow a richer origin model:

```rust
pub struct StyleOrigin {
    pub declaration_span: Option<Span>,
    pub selector_span: Option<Span>,
    pub candidate_span: Option<Span>,
}
```

Authored CSS can populate declaration and selector spans. Tailwind output should additionally point `candidate_span` at the original class token that caused the generated declaration.

## Escape Hatches

Tailwind needs first-class escape hatches because many projects generate class names dynamically.

- `safelist`: explicit class candidates to always generate.
- `source_roots`: extra files or directories to scan for candidates.
- `source_mode`: whether to scan only the HTML graph or the broader project.
- `config`: file path, inline config, or default config.

Adapters should still receive plain render IR. Tailwind-specific metadata should remain in source maps, annotations, and style provenance.

## Non-Goal

Do not add Tailwind logic to GPUI or gpui-components adapters. If Tailwind support requires adapter-specific behavior, that is a sign the core style IR is missing a primitive.
