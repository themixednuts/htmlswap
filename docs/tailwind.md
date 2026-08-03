# Tailwind CSS v4

htmlswap runs Tailwind CSS v4 as a style provider. Tailwind remains the CSS
compiler; htmlswap consumes its generated CSS through the same Lightning CSS,
cascade, theme-token, render-IR, and adapter pipeline used for authored CSS.

```text
HTML and resolved resources
  -> literal class candidate collection
  -> TailwindProvider
  -> TailwindEngine (TailwindCli by default)
  -> generated CSS source
  -> Lightning CSS and cascade matching
  -> target-neutral render IR
  -> GPUI, gpui-components, Svelte, or another adapter
```

Adapters never contain Tailwind-specific branches.

## Why Tailwind Produces CSS

Tailwind v4 is a CSS-first build tool. Its supported CLI owns `@import`,
`@theme`, `@source`, `@utility`, `@variant`, `@custom-variant`, `@apply`, legacy
plugins, arbitrary values, and future Tailwind behavior. Reimplementing those
semantics in htmlswap would create an incomplete second Tailwind compiler and
would break project configuration and plugin compatibility.

The engine remains replaceable behind `TailwindEngine`, so a stable native
Rust integration can be added later without changing `Compiler`,
`TailwindProvider`, or adapter APIs.

## CLI

Install Tailwind CSS v4 and its CLI yourself, or download the official
standalone executable. htmlswap never installs packages or invokes a shell.

```text
htmlswap compile view.html \
  --tailwind \
  --tailwind-cli ./node_modules/.bin/tailwindcss \
  --adapter gpui
```

Use a CSS-first configuration file when the default theme is not enough:

```text
htmlswap compile view.html \
  --tailwind \
  --tailwind-css ./src/app.css \
  --tailwind-cli ./node_modules/.bin/tailwindcss
```

Relative `@import`, `@config`, `@plugin`, and `@source` paths resolve from the
configuration file's directory. `--tailwind-cli-arg` can be repeated for a
shim that needs prefix arguments. For example, an explicitly configured local
package runner can be represented as an executable plus prefix arguments
without shell parsing.

## Library API

```rust,no_run
use std::time::Duration;
use htmlswap::{
    CompileAssets, Compiler, TailwindCli, TailwindProvider,
};

let cli = TailwindCli::new("./node_modules/.bin/tailwindcss")
    .with_working_directory("./")
    .with_timeout(Duration::from_secs(20));
let tailwind = TailwindProvider::new(cli)
    .with_stylesheet("@import \"tailwindcss\" source(none);")
    .with_safelist(["sr-only", "motion-safe:animate-spin"]);
let compilation = Compiler::new()
    .with_style_provider(tailwind)
    .compile_fragment(
        r#"<main class="flex gap-4"></main>"#,
        &CompileAssets::new(),
    );

assert!(!compilation.diagnostics.has_errors());
```

The default stylesheet uses `source(none)` and supplies the parsed HTML class
tokens as an explicit isolated source. This avoids accidentally scanning the
whole repository. A custom stylesheet may opt into Tailwind's automatic
project scanning or add explicit `@source` directives.

## Hardening

- Providers execute in registration order and generated sources participate in
  normal CSS source order.
- Provider names and generated source names must be non-empty and unique.
- Output from a provider that reports an error is discarded.
- Generated sources cannot shadow authored source names.
- Tailwind is invoked directly without a shell.
- Candidate input and generated output are byte-bounded.
- Diagnostics are captured with a bounded size.
- A timed-out Tailwind process is killed.
- Candidate and output files live in an isolated temporary workspace.
- Tailwind is disabled unless the caller registers the provider or passes
  `--tailwind`.

Treat Tailwind configuration and JavaScript plugins as trusted build input:
Tailwind itself may load and execute them.

## Current Scope

Tailwind style-provider expansion currently requires an HTML-based source
frontend. JSX compilation rejects registered style providers explicitly rather
than silently ignoring them.

Literal HTML class tokens and explicit safelist entries are always supplied.
Dynamic class construction still needs complete class names in a safelist or an
explicit project source, matching Tailwind's own static detection model.

The generated CSS can use modern CSS and custom properties. htmlswap preserves
unsupported declarations and emits adapter diagnostics instead of inventing
Tailwind-specific fallbacks. Extending native target coverage belongs in the
core CSS/theme IR, not in `TailwindProvider`.

## References

- Tailwind CLI: https://tailwindcss.com/docs/installation/tailwind-cli
- Detecting classes: https://tailwindcss.com/docs/detecting-classes-in-source-files
- Functions and directives: https://tailwindcss.com/docs/functions-and-directives
- Compatibility: https://tailwindcss.com/docs/compatibility
