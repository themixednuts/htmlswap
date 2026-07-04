# Frontend Primitive Design

htmlswap should treat framework syntax as source dialect sugar that lowers into portable compiler primitives. Vue, Svelte 5, and Angular share enough concepts that several should be first-class IR rather than `data-htmlswap-*` attributes or handler strings.

Component definitions and usages have their own design in
[`components.md`](components.md). The primitives below feed component contracts,
slots, model ports, and adapter resolution.

## Source Intent Markers

Some intent is useful before full component contracts exist and should still be
source-owned rather than adapter-owned:

- `data-htmlswap-variant`: visual treatment such as `solid`, `outline`,
  `ghost`, `soft`, or `link`.
- `data-htmlswap-tone`: intent such as `neutral`, `accent`, `success`,
  `warning`, `danger`, or `info`.
- `data-htmlswap-size`: semantic size such as `xs`, `sm`, `md`, `lg`, or `xl`.
- `data-htmlswap-density`: layout density such as `compact`, `comfortable`, or
  `spacious`.
- `data-htmlswap-region`: dotted source namespace for feature/region grouping,
  inherited by descendants. It is not a module path or adapter layer.
- `data-htmlswap-semantic-*`: temporary extension axes for source-owned intent
  that should be promoted when more than one adapter needs it.

Adapters may consume these directly when they have a real target API. Otherwise
they should preserve them as source metadata or diagnostics, not silently drop
them.

## Implement First

1. Typed dynamic bindings

   Add a `RenderBinding` model with targets such as `Attribute`, `Property`, `Boolean`, `Aria`, `Data`, `Class`, `Style`, and `Spread`. This maps to Angular property/attribute/class/style bindings, Svelte `class:` and `style:`, Vue `v-bind`, and JSX-style spreads.

2. Typed event listeners

   Add listener target, options, filters, and effects: element/window/document/body targets, capture/passive/once options, key/mouse filters, and `preventDefault`/`stopPropagation` effects. Vue modifiers and Angular key modifiers should lower into this directly instead of becoming JavaScript statement text.

3. Loop and control-flow metadata

   Extend `RenderControlFlow` for key/track expressions, index/context bindings, empty fallbacks, condition aliases, and switch matching metadata. Svelte each blocks and Angular `@for` both need loop identity and empty branches.

4. Raw content sinks with safety

   Model escaped text and raw HTML sinks separately, with a safety policy. Vue `v-html`, Svelte `{@html}`, and Angular `[innerHTML]` are security boundaries; adapters should warn or opt in explicitly.

## Next Layer

5. Two-way/model binding

   Model this as a linked binding plus listener over the existing state/action primitives, not a separate state system. Angular `[()]`, Vue `v-model`, and Svelte `bind:` all fit that shape.

6. Template fragments, slots, and snippets

   Add real fragment/closure primitives for Svelte snippets/render, Angular `ng-template`/`ng-content`, and Vue slots. This likely needs lexical context and fallback children.

7. Async/defer UI states

   Add descriptive metadata for pending/fulfilled/rejected and defer placeholder/loading/error states, with triggers and prefetch policy. Angular `@defer` depends on fragments and should be adapter opt-in behavior.

## Structural Notes

- `SourceDialect` should emit typed bindings/listeners through `ElementDirectives`; rewriting framework concepts back into HTML attributes is a temporary escape hatch.
- Vanilla HTML classes are not semantic component markers. Component intent should be explicit, such as `data-htmlswap-component="titlebar"`, while source dialects may translate their own conventions into that hint before lowering.
- Slots are parent-consumption hints, not independently routed components. For example, a titlebar can consume `data-htmlswap-slot="window-controls"` and replace that subtree with native window controls.
- `RenderControlFlowHost` likely needs a transparent/fragment host in addition to wrapper and element hosts, because Svelte and Angular can wrap bare text or non-emitting containers.
- Ordering rules are per target: spreads can override attributes, while class/style generally merge.
- `Expr` will need a broader portable subset. Otherwise first-class bindings still collapse into opaque strings.
- Unsupported adapter behavior should produce diagnostics, not silent comments or dropped `data-htmlswap-*` attributes.
