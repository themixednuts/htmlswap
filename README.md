# htmlswap

**Write UI once, in HTML. Compile it to native and web frameworks.**

htmlswap is a Rust compiler that parses an HTML source dialect (`.dc.html`),
lowers it into a target-neutral render plan, and emits idiomatic code for
different UI targets:

- **GPUI** — raw `gpui::div()` builder chains (Zed's UI framework)
- **gpui-components** — widget-level output (`Button`, `TabBar`, `Input`, `TitleBar`, …)

Both GPUI adapters target either Zed's `gpui` 0.2 with `gpui-component` 0.5
(the default) or GPUI Kit 0.7: `gpui-pre` 0.3, which generated code refers to
as `gpui`, with `gpui-component` 0.7 (`--gpui-kit`, or
`GpuiAdapterOptions::target = GpuiTarget::Kit`). Styles come from the typed
CSS lowering (`htmlswap::computed`) through one GPUI style plan
(`computed::gpui::plan`), which runtime renderers can apply too.
- **Svelte 5** — components with runes
- **HTML / text** — round-trip and debugging output

```text
source (.dc.html / .html / .vue)
   │  parse        html5ever + lightningcss + oxc
   ▼
HtmlDocument
   │  lower        roles, semantics, state, actions, control flow, theme
   ▼
RenderPlan        (target-neutral IR)
   │  route        RouteConfig / LayerClaim decide which layer owns each element
   ▼
adapters          GPUI · gpui-components · Svelte · HTML
   │  emit
   ▼
generated code    (.rs / .svelte / .html) + diagnostics + source maps
```

The hard part of this project is intentionally concentrated in two places:
the **planner** (`src/lower.rs`, `src/plan.rs`), which must capture UI intent
without leaking any target's API, and the **emitters** (`src/adapters/*`),
which each translate that intent into a very different domain (CSS flexbox →
GPUI style methods, HTML state → Rust entities, events → Svelte runes). The
source dialect itself is deliberately small.

## Design rules

1. **Source declares intent, never targets.** Markers describe *what* an
   element is (`tone="danger"`, `size="sm"`), never *how* a target spells it
   (`.danger()`, `Size::Small`). Output placement is CLI/adapter
   configuration, not source syntax.
2. **Native HTML first.** If HTML or ARIA already has a spelling
   (`disabled`, `aria-pressed`, `title`), use it. `data-htmlswap-*` markers
   exist only for intent HTML cannot express.
3. **Lossy mappings are loud.** When a target cannot represent an intent, the
   adapter emits a source comment *and* a diagnostic — never a silent drop.
4. **Round-trip is a feature.** Generated GPUI code can be imported back into
   a render plan (`htmlswap import`) and compared (`htmlswap roundtrip-check`).

## Quick start

```bash
# compile a DC view to gpui-components Rust
htmlswap compile src/view.dc.html --source dc --adapter gpui-components -o view.rs

# raw GPUI, Svelte, or debug text
htmlswap compile src/view.dc.html --source dc --adapter gpui -o view.rs
htmlswap compile src/view.dc.html --source dc --adapter gpui --gpui-kit -o view.rs
htmlswap compile src/view.dc.html --source dc --adapter svelte -o View.svelte
htmlswap compile src/view.dc.html --source dc --adapter text

# watch mode, import (reverse), layout debugging
htmlswap compile src/view.dc.html --source dc --adapter gpui --watch -o view.rs
htmlswap import view.rs --adapter gpui -o recovered.html
htmlswap layout-snapshot input.html -o debug.html

# Tailwind CSS v4 through a preinstalled CLI
htmlswap compile src/view.html --tailwind \
  --tailwind-cli ./node_modules/.bin/tailwindcss \
  --adapter gpui -o view.rs
```

Or from a `build.rs` (see `examples/gpui_smoke/`, and `examples/gpui_kit_smoke/`
for GPUI Kit): compile with `Frontend::dc()` + `GpuiComponentsAdapter`, write to
`OUT_DIR`, and `include!` the generated file.

Tailwind remains the authoritative CSS transformer; htmlswap feeds its output
through the ordinary CSS and render-IR pipeline. See
[docs/tailwind.md](docs/tailwind.md). The capability-based Rust extension model
is documented in [docs/plugins.md](docs/plugins.md).

## Fail-closed pure HTML policy

Embedders that need ordinary HTML without htmlswap's extension dialect can
select `SourcePolicy::pure_html()`:

```rust,ignore
let compiler = Compiler::try_with_options(
    CompilerOptions::new().with_source_policy(SourcePolicy::pure_html()),
)?;
let compilation = compiler.compile_fragment(source, &assets);
```

This policy rejects `data-htmlswap-*`, scripts, inline event handlers,
JavaScript URLs, embedded browsing/plugin elements, and remote resources before
resource or script processing. If validation emits an error, compilation
returns an empty render plan rather than exposing partially trusted output.
The default policy remains extension-compatible for existing htmlswap users.

## The source dialect in 30 seconds

```html
<x-dc>
  <helmet>
    <style>
      :root { --background: #101217; --accent: #4f8cff; }
      .toolbar { display: flex; gap: 8px; align-items: center; }
    </style>
  </helmet>
  <section class="toolbar" data-htmlswap-region="editor.toolbar">
    <button data-htmlswap-tone="accent" data-htmlswap-size="sm"
            onClick="{{ save }}">Save</button>
    <sc-for list="{{ tabs }}" as="tab" hint-placeholder-count="3">
      <button role="tab" aria-selected="{{ tab.active }}"
              onClick="{{ tab.select }}">{{ tab.label }}</button>
    </sc-for>
    <sc-if value="{{ dirty }}">
      <span>Unsaved changes</span>
    </sc-if>
  </section>
</x-dc>
```

Full dialect reference: [docs/dc-format.md](docs/dc-format.md). Component
model design: [docs/components.md](docs/components.md). Primitive roadmap:
[docs/frontend-primitives.md](docs/frontend-primitives.md).

## Repository layout

| Path | What it is |
|---|---|
| `src/parse.rs`, `src/frontend.rs` | HTML parsing and source dialects (DC, Vue) |
| `src/lower.rs`, `src/plan.rs` | The planner: HTML → `RenderPlan` |
| `src/adapters/gpui.rs` | Raw GPUI emitter (CSS → style builder methods) |
| `src/adapters/gpui_components.rs` | Widget layer over GPUI (Button, TabBar, Input, …) |
| `src/adapters/gpui_reverse.rs` | Reverse importer: generated Rust → `RenderPlan` |
| `src/adapters/svelte.rs` | Svelte 5 emitter |
| `src/adapter.rs` | Routing: `RouteConfig`, `Layer`, `LayerClaim` |
| `tests/` | Integration tests for pipeline, adapters, round-trips |
| `docs/` | Dialect and architecture design documents |

---

## Authoring UI with Claude (pre-prompt)

Paste the **core prompt** below into Claude's system prompt (or a
`CLAUDE.md` / project instructions) when you want it to design views for
htmlswap. The core prompt is target-neutral — it encodes only the source
dialect grammar. Then append the **target addendum** for each adapter you
actually compile to, so designs also land on constructs that target's widget
grammar can express. Skip addenda for targets you don't use.

### Core prompt (always include)

````markdown
# You are designing UI as htmlswap `.dc.html` source

You write declarative HTML views for htmlswap, a compiler that turns them
into code for one or more UI targets (native or web). Your HTML is compiled,
not shipped to a browser as-is. Follow this grammar exactly; anything outside
it is rejected or emitted lossily with warnings.

## File shape

```html
<x-dc>
  <helmet>
    <!-- optional: <style>, <link rel="stylesheet">, external <script src> -->
  </helmet>
  <section> <!-- exactly one render root -->
    ...
  </section>
</x-dc>
```

## Data holes — `{{ ... }}`

Holes are bindings, NOT expressions. Allowed forms only:
- dotted paths: `{{ user.name }}`, `{{ item.onSelect }}`
- loop index: `{{ $index }}`
- literals: `{{ true }}`, `{{ false }}`, `{{ null }}`, `{{ 42 }}`

NEVER write calls, arithmetic, ternaries, or comparisons inside holes
(`{{ save() }}`, `{{ a + b }}`, `{{ x ? y : z }}` are all invalid). If logic
is needed, compute it in the component script and bind the result.

Attribute value shapes:
- `x="literal"` → string
- `x="{{ path }}"` → raw binding
- `x="Hello {{ name }}"` → interpolated string

## Control flow (elements, not attributes)

```html
<sc-if value="{{ visible }}"> ... </sc-if>
<sc-else-if value="{{ other }}"> ... </sc-else-if>
<sc-else> ... </sc-else>

<sc-for list="{{ items }}" as="item" hint-placeholder-count="3">
  <div>{{ item.label }} — {{ $index }}</div>
</sc-for>

<sc-switch value="{{ mode }}">
  <sc-case value="edit"> ... </sc-case>
  <sc-default> ... </sc-default>
</sc-switch>
```

`hint-placeholder-*` attributes give static preview hints (e.g. how many
placeholder rows to render); use them on every `sc-for`/`sc-if` whose data is
runtime-only.

## Events

Bind handlers with `on*` attributes whose value is a single hole:
`onClick`, `onDoubleClick`, `onInput`, `onChange`, `onSubmit`,
`onMouseEnter`, `onMouseDown`.

```html
<button onClick="{{ handleSave }}">Save</button>
<input value="{{ name }}" onInput="{{ onName }}">
```

## State

- Form controls (`input`, `textarea`, `select`, checkboxes, radios) own their
  state by default (the target framework manages it).
- Add `data-htmlswap-state-owner="source"` when your script owns the value
  and the control should reflect it.
- Use native attributes for control state: `disabled`, `readonly`,
  `required`, `checked`, `multiple`, `placeholder`, `value`, `min`, `max`,
  `step`, `pattern`, `minlength`, `maxlength`, `rows`.

## Semantic intent markers (the only `data-htmlswap-*` you may use)

| Attribute | Allowed values | Meaning |
|---|---|---|
| `data-htmlswap-tone` | `neutral` `accent` `success` `warning` `danger` `info` | intent color |
| `data-htmlswap-variant` | `solid` `outline` `ghost` `soft` `link` `text` | visual treatment |
| `data-htmlswap-size` | `xs` `sm` `md` `lg` `xl` | semantic size |
| `data-htmlswap-density` | `compact` `comfortable` `spacious` | spacing density |
| `data-htmlswap-region` | dotted namespace, e.g. `editor.sidebar` | feature grouping, inherited |
| `data-htmlswap-component` | kebab-case name, e.g. `gem-card` | component identity |
| `data-htmlswap-slot` | slot name, e.g. `header-actions` | slot the parent consumes |
| `data-htmlswap-prop-*` | any | props for a component usage |
| `data-htmlswap-key` | binding | list reconciliation key |
| `data-htmlswap-state` / `-state-owner` | id / `source` `external` `target` | state identity/ownership |
| `data-htmlswap-loading` | boolean attribute | busy state (or use `aria-busy="true"`) |
| `data-htmlswap-semantic-*` | any | escape hatch for intent not listed above |
| `data-htmlswap-raw` | boolean attribute | preserve subtree as opaque HTML |

Do NOT invent other `data-htmlswap-*` attributes. Use
`data-htmlswap-semantic-<axis>` for anything not covered.

Interactive state uses ARIA, not custom markers:
- toggle button pressed → `aria-pressed="true"` (or bare `selected`)
- selected tab → `aria-selected="true"` on the `role="tab"` element
- loading control → `aria-busy="true"`
- tooltips → the native `title` attribute

## Structure conveys widgets

Adapters recognize widgets from standard HTML tags and ARIA roles, so use
them precisely: `button`, `a`, `label`, `input`/`textarea`/`select` with
correct `type`, `form`/`fieldset`/`legend`, list markup for lists, and
`role="tablist"`/`role="tab"` for tabs. Icons are written as Material Symbol
spans with the icon name as text:
`<span class="material-symbols-outlined">save</span>`.

## Styling

- Use `style="..."` and `<style>` blocks with a flexbox-only layout model:
  `display:flex`, `flex-direction`, `flex-wrap`, `flex`, `gap`, `padding`,
  `margin`, `width/height/min-*/max-*`, `border*`, `border-radius`,
  `background`, `color`, `font-size`, `font-weight`, `opacity`, `overflow`.
- Do NOT use: `position: absolute/fixed/sticky`, `z-index`, `float`,
  `display: inline/inline-block/grid`, CSS animations. Targets that cannot
  express them preserve them as source comments instead of rendering them.
- Theme through CSS custom properties in `:root` (`--background`,
  `--foreground`, `--accent`, …) and reference them with `var(--token)`.
- Pseudo states via attributes: `style-hover="..."`, `style-active`,
  `style-focus`, `style-before`, `style-after`.
- Dynamic styles: `style="{{ computedStyle }}"` binds a style object;
  `data-htmlswap-style-hover="{{ hoverStyle }}"` binds per-state.

## Components

```html
<!-- source-defined component usage -->
<dc-import name="Card" item="{{ item }}" hint-size="100%,120px">
  <span>projected child</span>
</dc-import>

<!-- foreign component (body not compiled here) -->
<x-import component="Chart" from="./Chart.jsx" data="{{ rows }}"></x-import>
```

Kebab-case prop attributes become camelCase props (`item-name` →
`itemName`); `data-*`/`aria-*` props stay kebab-case. `hint-size` is a
placeholder render size ("width,height"), unrelated to
`data-htmlswap-size`.

## Never do

- No JavaScript expressions in holes — bindings only.
- No target names, adapter names, module paths, file paths, or output
  locations anywhere in source (`data-htmlswap-region` is a dotted
  namespace like `project.manager`, never a path).
- No absolute positioning or overlay hacks; express layout as nested flex.
- No inventing tags or attributes outside this document.
- Do not silently work around a limitation — if the design needs something
  this grammar cannot express, say so explicitly and propose the closest
  expressible alternative.

## Self-check before you finish

1. Every `{{ }}` is a dotted path, `$index`, or literal.
2. Every interactive element has an `id` and an `on*` handler bound to a hole.
3. Every `sc-for` has `list`, `as`, and a `hint-placeholder-count`.
4. Semantic markers only use the allowed values in the table above.
5. Layout is pure flexbox; theme colors come from `var(--token)`.
6. If a target addendum follows, honor its lossy-value warnings.
````

### Target addenda (append only the ones you compile to)

**gpui-components** (widget-level native desktop output):

````markdown
# Target addendum: gpui-components

This design compiles to the gpui-component widget library. Design within its
grammar:

- Buttons: tones `accent/success/warning/danger/info` and variants
  `outline/ghost/link/text` map 1:1 to real widget methods, as do `disabled`,
  `aria-pressed` (selected), and `aria-busy` (loading). `variant="soft"`,
  `density="spacious"`, and `size="xl"` are LOSSY on this target (warned;
  `xl` clamps to large) — avoid them unless another target needs them.
- Tabs: mark the container `role="tablist"` (or
  `data-htmlswap-component="tabs"`) and children `role="tab"`. Tab-bar
  variants are `outline`, `pill`, `segmented`, `underline`.
- Title bars: `data-htmlswap-component="titlebar"` on a header element; a
  child with `data-htmlswap-slot="window-controls"` is replaced by native
  window controls.
- Text inputs, selects, checkboxes, radios, links, labels, list items, forms
  and fieldsets map to native widgets automatically from tags and roles.
- Material Symbol spans render as native icons.
````

**Raw GPUI** (no widget library — everything becomes `gpui::div()` chains):

````markdown
# Target addendum: raw GPUI

This design compiles to bare GPUI style-builder chains with no widget
library. Consequences:

- Semantic markers (`tone`, `variant`, `size`, `density`) produce no visuals
  here; they are preserved as metadata. Carry all appearance in CSS
  (colors, borders, padding) so the design survives without a widget layer.
- Text inputs become generated stateful entities; other form controls are
  styled divs, so draw their states (checked, focus) explicitly in CSS.
- The flexbox-only rule is strict: unsupported CSS becomes comments, not
  rendering.
- Material Symbol spans render as native icons.
````

**Svelte 5** (web output):

````markdown
# Target addendum: Svelte 5

This design compiles to a Svelte component (runes for state, `{#if}`/`{#each}`
for control flow, scoped styles).

- Semantic markers pass through as `data-htmlswap-*` attributes; style them
  yourself with selectors such as `[data-htmlswap-tone="danger"]`.
- The full browser CSS engine is available, but stay inside the portable
  flexbox subset if this view also targets a native adapter.
- Material Symbol spans stay as spans — include the Material Symbols
  stylesheet via `<helmet>`.
````

**Verification loop.** After Claude produces a view, compile it against each
of your targets and feed the diagnostics back:

```bash
htmlswap compile view.dc.html --source dc --adapter gpui-components --out-dir ./generated
```

Warnings name the exact attribute and span that could not be mapped; asking
Claude to "fix all compiler diagnostics" converges quickly because every
lossy construct is reported rather than dropped.
