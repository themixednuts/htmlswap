# Component Architecture

htmlswap should treat components as a target-neutral compiler concept, not as a
Web Components-only feature and not as adapter hint strings. React, Svelte,
Vue, Solid, Web Components, GPUI, and gpui-components all spell components
differently, but they share the same shape: a definition, one or more usage
sites, a contract, slots or children, input props, output events, state, effects,
context, styles, and module boundaries.

The core model is:

```text
source framework syntax
  -> ComponentDefinition + RenderComponentUse + ComponentContract
  -> adapter resolution: Native | Flatten | Reject
  -> target output
```

This keeps source dialects responsible for translating framework spelling into
compiler primitives, and keeps adapters responsible for translating compiler
primitives into target code.

## Framework Survey

The current framework landscape uses different APIs but the same component
semantics.

- React 19 components are functions or classes with props, children, callback
  props for events, refs, context, effects, Suspense, portals, fragments, and
  dynamic component expressions.
- Svelte 5 components use runes such as `$props`, snippets and `{@render}` for
  child fragments, `$state` and `$derived` for local reactivity, `$effect`,
  lifecycle helpers, context, scoped styles, and optional custom element output.
- Vue 3.5 components use props, emits, slots and scoped slots, provide/inject,
  refs, lifecycle hooks, `defineModel`, async components, Suspense, Teleport,
  scoped styles, and `defineCustomElement`.
- Solid 1.9 components are functions with props, children helpers, signals,
  stores, context, refs, lifecycle, resources, Suspense, portals, and dynamic
  components.
- Web Components use custom element definitions, registered tag names,
  attributes, properties, reflection, named slots, Shadow DOM, parts, CSS custom
  properties, custom events, and lifecycle callbacks.

The invariant is not a specific tag syntax. The invariant is a component
definition with a contract, and a usage site that binds values into that
contract.

## Core Types

Definitions live in a side graph on `RenderPlan`. Usage sites live positionally
in the render tree.

```rust
pub struct ComponentGraph {
    pub definitions: Vec<ComponentDefinition>,
    pub edges: Vec<ComponentEdge>,
}

pub struct ComponentDefinition {
    pub id: ComponentId,
    pub kind: ComponentKind,
    pub contract: ComponentContract,
    pub origin: ComponentOrigin,
    pub style: StyleBoundary,
    pub span: Option<Span>,
}

pub enum ComponentKind {
    Native,
    Template,
    CustomElement,
    Foreign,
    Dynamic,
}
```

`Native` means the target adapter provides the implementation, such as
`gpui_component::TitleBar`. `Template` means htmlswap has a render body it can
flatten or emit. `CustomElement` is the Web Components case. `Foreign` is an
imported React, Vue, Svelte, Solid, or library component whose body is not
compiled by htmlswap. `Dynamic` covers runtime component expressions such as
Vue `<component :is>`, Solid `<Dynamic>`, and React `<Tag>`.

Usages should become a real render node rather than a string in
`RenderSourceIntent`.

```rust
pub enum RenderNode {
    Element(Box<RenderElement>),
    Text(RenderText),
    Raw(RenderRaw),
    Use(Box<RenderComponentUse>),
}

pub struct RenderComponentUse {
    pub component: ComponentRef,
    pub args: Vec<PortArg>,
    pub slots: Vec<SlotContent>,
    pub key: Option<Expr>,
    pub control_flow: Option<Box<RenderControlFlow>>,
    pub source_intent: Option<Box<RenderSourceIntent>>,
    pub span: Option<Span>,
}

pub enum ComponentRef {
    Resolved(ComponentId),
    Dynamic(Expr),
    Unresolved(CompactString),
}
```

A usage is not necessarily an HTML element. Web Components are the exception
because they are tag-addressable. The general IR should not force every
component through `RenderElement`.

## Contracts And Ports

The component contract is the firewall between source dialects and target
adapters.

```rust
pub struct ComponentContract {
    pub version: ContractVersion,
    pub ports: Vec<Port>,
    pub consumes_context: Vec<ContextKey>,
    pub provides_context: Vec<ContextKey>,
    pub extras: BTreeMap<CapabilityKey, ContractValue>,
}

pub enum Port {
    Prop {
        name: PortName,
        value_kind: ValueKind,
        surface: PropSurface,
        required: bool,
        default: Option<Expr>,
    },
    Event {
        name: PortName,
        payload: ValueKind,
    },
    Model {
        prop: PortName,
        event: PortName,
    },
    Slot {
        name: Option<SlotId>,
        scoped_params: Vec<ValueKind>,
        fallback: Option<FragmentId>,
    },
    Expose {
        name: PortName,
        signature: ValueKind,
    },
}

pub enum PropSurface {
    Attribute,
    Property,
    Both,
}
```

This single shape covers framework-specific concepts:

- React props, callback props, `children`, refs, and render props.
- Svelte `$props`, snippets, bindable props, and component exports.
- Vue props, emits, `v-model`, slots, scoped slots, template refs, and expose.
- Solid props, children functions, signal pairs, and refs.
- Web Component attributes, properties, reflected attributes, slots, custom
  events, parts, and imperative element methods.

Two-way binding is not a separate state system. It is a model port linking an
input prop and an output event.

Scoped slots, Svelte snippets, Vue scoped slots, React render props, and Solid
function children should all lower to slot content whose body is a fragment and
whose parameters describe the values supplied by the component.

## Style And Module Boundaries

Component definitions need style and module ownership.

```rust
pub struct ComponentOrigin {
    pub source: OutputSource,
    pub module: Option<ModuleRef>,
    pub export: Option<CompactString>,
}

pub struct StyleBoundary {
    pub scope: StyleScope,
    pub module: Option<CompactString>,
}

pub enum StyleScope {
    Global,
    Scoped,
    Shadow,
    CssModule,
}
```

Svelte scoped styles, Vue scoped styles, CSS Modules, and Shadow DOM styles are
all style boundary strategies. CSS custom properties remain in the existing
theme/custom-property pipeline; they should not become component-only data.

`BundlePlan` should use `OutputUnitKind::Component` for component definitions
and should add dependency edges from usage sites and component bodies to their
definitions, scripts, state, and styles.

## Adapter Resolution

Adapters should not know every source framework. They should resolve neutral
component definitions and usages.

```rust
pub enum ComponentResolution {
    Native(TargetComponentSpec),
    Flatten,
    Reject(Diagnostic),
}
```

- `Native` maps a contract to a target component, such as gpui-components
  `TitleBar`.
- `Flatten` inlines a source-defined template component body at the usage site,
  projecting slot content and binding props.
- `Reject` emits a diagnostic when a component cannot be mapped or flattened.

GPUI and gpui-components should consume the same neutral contracts. They should
not learn Vue, Svelte, React, Solid, or Shadow DOM syntax. A future
`WebComponentAdapter`, `ReactAdapter`, `SvelteAdapter`, `VueAdapter`, or
`SolidAdapter` can emit framework-native code from the same graph.

`RouteConfig`, `LayerClaim`, and `RouteTarget::Component` remain useful. Routing
decides which layer owns a component. Component resolution decides whether that
layer emits native code, flattens, or rejects.

## Registry And Plugins

Components should be registered as data, not hard-coded match arms.

```rust
pub trait ComponentProvider {
    fn provide(&self, registry: &mut ComponentRegistryBuilder, cx: &mut ProviderContext);
}

pub struct ComponentRegistry {
    definitions: BTreeMap<ComponentId, ComponentDefinition>,
}
```

Source dialects can provide definitions found in source files. Adapters can
provide target-native definitions they know how to emit. External contract files
can provide `Foreign` definitions for imported packages.

Contracts should be versioned. A component provider can declare support for a
contract version and required ports. Missing required ports should produce
precise diagnostics.

Use runtime reflection carefully:

1. Prefer typed ports and typed contract fields.
2. Use `extras: BTreeMap<CapabilityKey, ContractValue>` for adapter-specific
   data that is still serializable.
3. Use `dyn Any` only behind a named capability key for Rust-only plugin escape
   hatches.
4. If two adapters need the same extension, promote it to a typed field or port.

## Script Analysis

Oxc should connect JavaScript and TypeScript to the component graph. This should
be AST-driven, not string scraping.

The script collector should detect:

- `customElements.define("my-card", MyCard)` and classes extending
  `HTMLElement`.
- React and Solid function components exported from modules.
- Vue `defineComponent`, `defineProps`, `defineEmits`, and `defineModel`.
- Svelte component module exports and runes where the source parser supports
  them.
- Imports, default exports, named exports, dynamic imports, and CDN/bare module
  specifiers.

Imported or CDN components should become `ComponentKind::Foreign` when their
body is unavailable. Their contracts can come from usage inference, TypeScript
declarations, generated metadata, or user-provided component contract files.

Action handler analysis should stay unified. Event ports bind to existing
action/event primitives. Web Component `CustomEvent.detail` should become an
event payload source, not a separate event system.

## Existing Htmlswap Syntax

Existing `data-htmlswap-*` attributes should remain source syntax, but lower
into the component model:

- `data-htmlswap-component` becomes a `RenderComponentUse`.
- `data-htmlswap-slot` becomes `SlotContent`.
- `data-htmlswap-prop-*` becomes `PortArg`; whole-value template holes should be
  treated as raw values, while mixed holes remain interpolated strings.
- `data-htmlswap-variant`, `data-htmlswap-tone`, `data-htmlswap-size`, and
  `data-htmlswap-density` become typed source semantics.
- `data-htmlswap-region` becomes an inherited source namespace that can drive
  routing, bundle policy, diagnostics, or adapter selection without naming an
  output module.

Output module placement is not source syntax. `BundleConfig`, CLI options, or
adapter configuration should map semantic targets to `BundlePlan` boundaries.

`RenderSourceIntent` is a temporary source-intent bridge for current behavior.
It should not contain target policy, adapter names, output paths, or generated
file names, and it should not remain the primary representation for components,
slots, props, events, or models once component contracts land.

## Implementation Plan

### Slice 1: Native Component Usage

Prove the design on existing behavior before adding framework-specific support.

- Add `ComponentGraph`, `ComponentDefinition`, `ComponentContract`, and
  `RenderNode::Use`.
- Register current gpui-components primitives as `ComponentKind::Native`.
- Lower `data-htmlswap-component`, `data-htmlswap-slot`, and
  `data-htmlswap-prop-*` into component usages.
- Keep generated output behavior-equivalent for existing tests.
- Add diagnostics for unknown components that cannot be mapped or flattened.

### Slice 2: Slots And Fragments

- Add `SlotContent` and named/default slot projection.
- Add a fragment or transparent control-flow host.
- Lower native `<slot>`, `slot=""`, existing `data-htmlswap-slot`, Vue slots,
  and later Svelte snippets into the same model.

### Slice 3: Typed Ports

- Add `PortArg`, typed prop surfaces, event ports, and model ports.
- Lower Vue `v-model`, SC templates, and event bindings into typed ports where
  possible.
- Keep string/template fallbacks with diagnostics where expressions cannot be
  typed.

### Slice 4: Template Definitions

- Allow source-defined components with `ComponentKind::Template`.
- Flatten template components for adapters that do not emit components natively.
- Add bundle edges for component definitions and recursive/shared component
  usage.

### Slice 5: Script-Discovered Definitions

- Extend Oxc script analysis to collect exports, imports, component functions,
  `defineComponent`, and `customElements.define`.
- Resolve component usages against imports and exported definitions.
- Treat external/CDN modules as `Foreign` definitions with opaque contracts.

### Slice 6: Framework-Native Emitters

- Add optional adapters for Web Components, React, Vue, Svelte, or Solid.
- Emit from `ComponentGraph` and `RenderComponentUse`, not from source-specific
  syntax.

## Avoid

- Do not make Web Components the base component primitive. They are
  `ComponentKind::CustomElement`.
- Do not keep components as `RenderSourceIntent` strings.
- Do not overload `ComponentId` as a non-lossy custom element tag name.
- Do not inline component definitions into every usage site.
- Do not make adapters parse Vue, React, Svelte, Solid, or Web Component syntax.
- Do not evaluate framework reactivity in core. Carry expressions and
  dependencies as data.
- Do not lead with `dyn Any`. Use it only as a keyed escape hatch.
- Do not silently turn unsupported components into `div`.
- Do not replace JavaScript bundlers. Keep external modules in the module graph
  and expose them to adapters or dedicated bundler integrations.
