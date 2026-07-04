# DC Source Dialect

Use `Frontend::dc()` or `htmlswap compile --source dc` for `.dc.html` files.

DC is a source dialect, not the target IR. The compiler lowers stable DC syntax into portable htmlswap primitives before adapters run.

## Root Shape

```html
<x-dc>
  <helmet>...</helmet>
  <section>render tree</section>
</x-dc>
```

- `<x-dc>` is transparent and does not become a render node.
- `<helmet>` is document metadata. It does not render as UI.
- External scripts inside `<helmet>` are preserved as `RenderScriptReference`.
- Inline `<script data-dc-script data-props="...">` is parsed by the normal JavaScript/TypeScript path, and `data-props` is preserved as `RenderAnnotationKind::SourceMetadata`.

## Holes

DC holes are intentionally small:

```html
{{ user.name }}
{{ $index }}
{{ true }}
{{ false }}
{{ null }}
{{ 42 }}
```

Supported hole grammar is dotted identifiers, `$index`, booleans, `null`, and numbers. Calls and expressions such as `{{ save() }}` or `{{ a + b }}` are rejected with diagnostics and preserved as opaque expressions.

Attribute coercion follows the source shape:

- `x="literal"` is a string attribute.
- `x="{{ path }}"` is a raw binding.
- `x="before {{ path }} after"` is an interpolated string.
- Event attributes such as `onClick="{{ handler }}"` lower through the normal event-binding path.

## Control Flow

The DC tag vocabulary still uses `sc-*` names:

```html
<sc-for list="{{ items }}" as="item" hint-placeholder-count="3">
  <button>{{ item.label }}</button>
</sc-for>

<sc-if value="{{ visible }}" hint-placeholder-val="{{ true }}">
  <span>Shown</span>
</sc-if>
```

`sc-for` binds the `as` value and `$index` in child scope. `sc-if` lowers to first-class control flow. Placeholder hints are preserved on the control-flow primitive.

`sc-switch`, `sc-case`, and `sc-default` are supported as an htmlswap extension, but they are not part of the minimal DC grammar described here.

## Components

```html
<dc-import name="Card" item="{{ item }}" hint-size="100%,120px">
  <span>Child</span>
</dc-import>

<x-import component="Chart" from="./Chart.jsx" data="{{ rows }}" dc-props="{{ props }}"></x-import>
```

- `dc-import name="Card"` lowers to a component hint for `Card`.
- `x-import component="Chart"` lowers to a component hint for `Chart`.
- `x-import from="..."` is preserved as component-source metadata and as a prop.
- Other component attributes become `data-htmlswap-prop-*`; kebab-case is normalized to lower camel case with `heck`.
- `data-htmlswap-prop-*` values follow the normal DC attribute coercion rule:
  a literal stays a string, a whole-value `{{ value }}` stays a raw binding, and
  a mixed value such as `Project {{ item.name }}` stays an interpolated string.
- `data-*` and `aria-*` prop names stay kebab-cased.
- Children remain children for adapters that model `props.children`.

## Styling

`style` is parsed by the normal CSS parser. DC pseudo-style attributes lower into style variants:

```html
<button style-hover="color: red" style-before="content: ''"></button>
```

Supported pseudo attributes include `style-hover`, `style-active`, `style-focus`, `style-before`, and `style-after`.

## Explicit Markers

DC syntax gives structural markers, but it does not standardize every UI semantic. Prefer explicit markers for source-level UI intent:

- `data-htmlswap-component` — component identity at a usage site
- `data-htmlswap-component-source` — module/path metadata for an imported component
- `data-htmlswap-slot` — named slot the parent component consumes
- `data-htmlswap-children` — child projection strategy for a component usage
- `data-htmlswap-key` — reconciliation key for list items
- `data-htmlswap-state` / `data-htmlswap-state-owner` — state identity and ownership (`source`, `external`, `target`)
- `data-htmlswap-form` — explicit form identity seed
- `data-htmlswap-prop-*` — component prop bindings
- `data-htmlswap-variant` — visual treatment: `solid`, `outline`, `ghost`, `soft`, `link`, `text`
- `data-htmlswap-tone` — intent color: `neutral`, `accent`, `success`, `warning`, `danger`, `info`
- `data-htmlswap-size` — semantic size: `xs`, `sm`, `md`, `lg`, `xl`
- `data-htmlswap-density` — spacing density: `compact`, `comfortable`, `spacious`
- `data-htmlswap-region` — dotted source namespace, inherited by descendants
- `data-htmlswap-semantic-*` — extension axes for intent that is not yet first-class
- `data-htmlswap-style` / `data-htmlswap-style-<state>` — dynamic style object bindings (whole-object and per-state)
- `data-htmlswap-raw` — escape hatch: subtree is preserved as opaque HTML
- `data-htmlswap-loading` — busy/loading state for interactive controls (also inferred from `aria-busy="true"`)

Interactive state prefers native HTML and ARIA spellings over new markers:

- `disabled`, `readonly`, `required`, `checked`, `multiple` are the native attributes.
- `aria-pressed="true"` (or bare `selected`) marks a toggle button as selected.
- `aria-selected="true"` (or bare `selected`) marks a tab as selected.
- `aria-busy="true"` marks a control as loading.

Use semantic axes for intent, not target API names. For example:

```html
<section data-htmlswap-region="project.manager">
  <button
    data-htmlswap-tone="accent"
    data-htmlswap-variant="solid"
    data-htmlswap-size="md"
    data-htmlswap-density="comfortable"
  >New Project</button>
</section>
```

`data-htmlswap-region` is a dotted source namespace, such as `project.manager`
or `editor.sidebar`. It is inherited by descendants and is useful for routing,
feature grouping, bundle policy, and generated comments. It must not be a file
path, module path, adapter layer, or output location.

`data-htmlswap-size` is a semantic size token (`xs`, `sm`, `md`, `lg`, `xl`).
It is different from DC `hint-size`, which is a rendering placeholder size.
Extra intent can use `data-htmlswap-semantic-*`, such as
`data-htmlswap-semantic-emphasis="high"`, until that axis becomes first-class.

Current gpui-components button support maps:

- `tone="accent|success|warning|danger|info"` to the matching Button variant.
- `variant="outline|ghost|link|text"` to the matching Button method.
- `size="xs|sm|md|lg"` to `Size::XSmall|Small|Medium|Large`.
- `density="compact"` to compact button padding.
- `disabled` to `.disabled(true)`.
- `aria-pressed="true"` or bare `selected` to `.selected(true)`.
- `aria-busy="true"` or `data-htmlswap-loading` to `.loading(true)`.

Default-like values such as `tone="neutral"`, `variant="solid"`, and
`density="comfortable"` intentionally emit no extra method. Lossy values such
as `size="xl"` (clamped to `Size::Large`), `variant="soft"`, or
unsupported/custom values are preserved as source comments and produce adapter
diagnostics.

Do not emit target adapter names, layer ids, output module paths, or generated
file names from DC source. Use compiler, CLI, or adapter configuration to map
semantic targets to implementation layers and output modules.

The DC dialect may infer component hints from adjacent comments or class conventions, such as titlebar comments/classes, but those are heuristics. Explicit markers are the durable contract.
