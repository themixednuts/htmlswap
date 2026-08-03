# Compiler Extension Model

htmlswap uses capability traits instead of one catch-all plugin hook. A source
dialect, generated-style pass, resource resolver, module graph provider, and
target adapter have different inputs, outputs, ordering, and failure contracts;
combining them into an untyped lifecycle API would make phase ordering and
invariants implicit.

| Capability | Trait | Composition |
|---|---|---|
| Source syntax lowering | `SourceDialect` | `Frontend`, ordered `Arc` values |
| Generated target-neutral CSS | `StyleProvider` | `StyleProviderPipeline`, ordered `Arc` values |
| Resource loading | `ResourceResolver` | `ResourceResolverChain`, first capable resolver |
| JavaScript module graphs | `ModuleGraphProvider` | Replaceable provider boundary |
| Target emission | `Adapter` / `Importer` | Terminal typed operation; routing composes adapter layers |

These are Rust API extensions linked into the process, not a stable dynamic
library ABI. A host that needs runtime discovery should load configuration and
construct trait objects at its own boundary. WebAssembly or subprocess plugins
can implement the same traits through an adapter when isolation is required.

## Ownership

Composable capabilities are stored as `Arc<dyn Trait + Send + Sync>`. This
makes compiler clones cheap, permits shared immutable configuration, and avoids
forcing every plugin to implement `Clone`. Inputs are borrowed phase views;
outputs own data that crosses into later phases.

## Ordering

- Source dialects run in registration order. First-match operations such as an
  attribute rewrite stop at the first claim; additive operations are merged.
- Style providers run in registration order. Their stylesheets enter normal CSS
  source order after authored external/linked styles and before inline `<style>`
  blocks.
- Resolver chains use the first resolver whose `can_resolve` returns true. A
  capable resolver owns success and failure for that request; failures do not
  silently fall through to another interpretation.
- Adapters are terminal typed operations. `RouteConfig` composes target layers
  inside adapters without erasing their output types.

## Failure Contracts

Reusable extension points return `Compilation<T>` when partial compilation and
source diagnostics are meaningful. A style provider that emits an error has
its partial output discarded. Invalid or conflicting generated source names are
rejected before parsing.

Target adapters use `Result<Output, AdapterError>` because emission itself is a
single terminal operation. Executable/reporting code decides how diagnostics
and errors become logs or exit status.

Environmental and input failures must be returned, not panicked. A panic still
indicates a plugin bug; htmlswap does not attempt to continue through arbitrary
unwinding from in-process code.

## Adding a Style Provider

```rust,no_run
use htmlswap::{
    Compilation, GeneratedStyleSource, StyleProvider, StyleProviderInput,
    StyleProviderOutput,
};

struct DesignSystem;

impl StyleProvider for DesignSystem {
    fn name(&self) -> &str {
        "design-system"
    }

    fn expand(&self, input: StyleProviderInput<'_>) -> Compilation<StyleProviderOutput> {
        let css = if input.classes.names().any(|name| name == "card") {
            ".card { display: flex; }"
        } else {
            ""
        };

        Compilation::clean(
            StyleProviderOutput::new().with_stylesheet(GeneratedStyleSource::new(
                "design-system:generated.css",
                css,
            )),
        )
    }
}
```

Register it with `Compiler::with_style_provider`. Providers should use a stable,
unique name and stable generated source names so diagnostics, source maps, and
caches remain understandable.

## Trust and Isolation

In-process capabilities are trusted code. The compiler bounds data returned by
external Tailwind execution, but a general Rust trait implementation can still
allocate, block, access the filesystem, or panic. Hosts that accept untrusted
third-party extensions should put them behind a process or WebAssembly boundary
and implement the capability trait as the trusted bridge.
