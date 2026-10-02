use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use compact_str::CompactString;
use heck::{ToLowerCamelCase, ToUpperCamelCase};

use crate::adapter::{Adapter, AdapterArtifact, AdapterContext, AdapterError, GeneratedFile};
use crate::compiler::CompiledFragment;
use crate::diagnostics::Diagnostic;
use crate::expr::{Expr, ExprLiteral, TemplateSegment, TemplateString};
use crate::plan::{
    ActionBinding, ActionPayload, RenderActionArgument, RenderActionHandlerEffect,
    RenderControlFlow, RenderControlFlowKind, RenderElement, RenderFormControlType,
    RenderHeadElement, RenderNode, RenderPlan, RenderPseudoElement, RenderRaw, RenderScriptKind,
    RenderScriptReference, RenderSourceCallback, RenderSourceDerived, RenderSourceEffect,
    RenderSourceIntent, RenderSourceLocal, RenderSourceLogic, RenderSourceLogicItem,
    RenderSourceMount, RenderSourceRef, RenderSourceSnippet, RenderSourceState, RenderStateBinding,
    RenderStateKind, RenderStateOwner, RenderStyleCondition, RenderStyleVariant, RenderText,
    UiRole,
};
use crate::source::{SourceFile, SourceKind, SourceMap, Span};
use crate::style::{StyleDeclaration, StyleProperty};

pub const SVELTE_LAYER_ID: &str = "svelte";
pub const SVELTE_PACKAGE_VERSION: &str = "5.56.4";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvelteAdapterOptions {
    pub component_name: CompactString,
    pub typescript: bool,
    pub include_dependency_header: bool,
    pub emit_source_comments: bool,
}

impl Default for SvelteAdapterOptions {
    fn default() -> Self {
        Self {
            component_name: "HtmlswapView".into(),
            typescript: true,
            include_dependency_header: true,
            emit_source_comments: true,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct SvelteAdapter {
    options: SvelteAdapterOptions,
}

impl SvelteAdapter {
    #[must_use]
    pub fn new(options: SvelteAdapterOptions) -> Self {
        Self { options }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvelteOutput {
    pub artifact: AdapterArtifact,
}

impl SvelteOutput {
    #[must_use]
    pub fn code(&self) -> &str {
        self.artifact
            .files
            .first()
            .map_or("", |file| file.contents.as_str())
    }
}

impl Adapter for SvelteAdapter {
    type Output = SvelteOutput;

    fn adapt(
        &self,
        fragment: &CompiledFragment,
        cx: &mut AdapterContext,
    ) -> Result<Self::Output, AdapterError> {
        let mut emitter = SvelteEmitter::new(&self.options, &fragment.sources, cx);
        let code = emitter.emit(&fragment.plan)?;
        let mut files = vec![GeneratedFile::new(
            format!(
                "{}.svelte",
                svelte_component_file_stem(&self.options.component_name)
            ),
            code,
        )];
        files.extend(emitter.script_files);
        Ok(SvelteOutput {
            artifact: AdapterArtifact::new(files, Vec::new()),
        })
    }
}

#[derive(Debug, Clone, Default)]
struct Scope {
    locals: BTreeSet<String>,
}

impl Scope {
    fn with_local(&self, local: impl Into<String>) -> Self {
        let mut next = self.clone();
        next.locals.insert(local.into());
        next
    }

    fn is_local(&self, name: &str) -> bool {
        self.locals.contains(name)
    }
}

#[derive(Debug, Clone)]
struct PropBinding {
    name: String,
    default: Option<String>,
    snippet: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct StyleRuleOrderKey {
    source: usize,
    start: usize,
    end: usize,
    fallback: usize,
}

impl StyleRuleOrderKey {
    fn new(span: Option<Span>, fallback: usize) -> Self {
        span.map_or(
            Self {
                source: usize::MAX,
                start: fallback,
                end: usize::MAX,
                fallback,
            },
            |span| Self {
                source: span.source.index(),
                start: span.start,
                end: span.end,
                fallback,
            },
        )
    }
}

#[derive(Debug, Default)]
struct SveltePrelude {
    props: BTreeMap<String, PropBinding>,
    states: BTreeMap<String, String>,
    component_imports: BTreeMap<String, String>,
    module_imports: Vec<String>,
    head_scripts: Vec<String>,
    runtime_classic_scripts: Vec<String>,
    source_logic: Option<RenderSourceLogic>,
    uses_snippet: bool,
    uses_attachment: bool,
    uses_style_helper: bool,
    style_classes: BTreeMap<String, String>,
    stylesheet_rules: BTreeMap<StyleRuleOrderKey, String>,
    style_rules: Vec<String>,
    inline_style_blocks: Vec<String>,
}

impl SveltePrelude {
    fn prop(&mut self, name: impl Into<String>) {
        let name = name.into();
        if should_skip_prop_name(&name) {
            return;
        }
        self.props.entry(name.clone()).or_insert(PropBinding {
            name,
            default: None,
            snippet: false,
        });
    }

    fn prop_with_default(&mut self, name: impl Into<String>, default: impl Into<String>) {
        let name = name.into();
        let default = default.into();
        if should_skip_prop_name(&name) {
            return;
        }
        self.props
            .entry(name.clone())
            .and_modify(|prop| prop.default = Some(default.clone()))
            .or_insert(PropBinding {
                name,
                default: Some(default),
                snippet: false,
            });
    }

    fn snippet_prop(&mut self, name: impl Into<String>) {
        let name = name.into();
        self.uses_snippet = true;
        self.props
            .entry(name.clone())
            .and_modify(|prop| prop.snippet = true)
            .or_insert(PropBinding {
                name,
                default: None,
                snippet: true,
            });
    }

    fn insert_stylesheet_rule_once(&mut self, key: StyleRuleOrderKey, rule_text: String) {
        self.stylesheet_rules.entry(key).or_insert(rule_text);
    }

    fn insert_deduped_stylesheet_rule(&mut self, span: Option<Span>, rule_text: String) {
        if self
            .stylesheet_rules
            .values()
            .any(|existing| existing == &rule_text)
        {
            return;
        }

        let mut fallback = self.stylesheet_rules.len();
        loop {
            let key = StyleRuleOrderKey::new(span, fallback);
            if let std::collections::btree_map::Entry::Vacant(entry) =
                self.stylesheet_rules.entry(key)
            {
                entry.insert(rule_text);
                break;
            }
            fallback += 1;
        }
    }
}

struct SvelteEmitter<'a, 'cx> {
    options: &'a SvelteAdapterOptions,
    sources: &'a SourceMap,
    cx: &'cx mut AdapterContext,
    prelude: SveltePrelude,
    script_files: Vec<GeneratedFile>,
}

impl<'a, 'cx> SvelteEmitter<'a, 'cx> {
    fn new(
        options: &'a SvelteAdapterOptions,
        sources: &'a SourceMap,
        cx: &'cx mut AdapterContext,
    ) -> Self {
        Self {
            options,
            sources,
            cx,
            prelude: SveltePrelude::default(),
            script_files: Vec::new(),
        }
    }

    fn emit(&mut self, plan: &RenderPlan) -> Result<String, AdapterError> {
        self.collect_plan(plan);
        self.collect_scripts(&plan.scripts);

        let mut output = String::new();
        if self.options.include_dependency_header {
            writeln!(
                output,
                "<!-- htmlswap: requires svelte@^{SVELTE_PACKAGE_VERSION} -->"
            )
            .expect("writing to String cannot fail");
        }
        self.write_script(&mut output);
        self.write_head(&mut output, &plan.head);
        self.write_style(&mut output, plan);
        self.write_jsx_snippets(&mut output, 0);
        for annotation in &plan.annotations {
            writeln!(
                output,
                "<!-- htmlswap-{}: {} -->",
                annotation.kind.name(),
                sanitize_html_comment(&annotation.value)
            )
            .expect("writing to String cannot fail");
        }
        self.write_nodes(&mut output, &plan.nodes, 0, &Scope::default(), "n");
        Ok(output)
    }

    fn collect_plan(&mut self, plan: &RenderPlan) {
        self.collect_source_logic(&plan.source_logic);
        for state in &plan.state.bindings {
            if state.owner != RenderStateOwner::Target {
                continue;
            }
            let name = sanitize_js_identifier(&state.id, "state");
            self.prelude
                .states
                .insert(name, svelte_state_initial_value(state));
        }
        for (index, node) in plan.nodes.iter().enumerate() {
            self.collect_node(node, &Scope::default(), &format!("n_{index}"));
        }
    }

    fn collect_source_logic(&mut self, source_logic: &[RenderSourceLogic]) {
        self.prelude.source_logic = source_logic
            .iter()
            .find(|logic| {
                logic.dialect.eq_ignore_ascii_case("dc")
                    || logic.dialect.eq_ignore_ascii_case("jsx")
            })
            .cloned();
        if let Some(logic) = self.prelude.source_logic.clone()
            && logic.dialect.eq_ignore_ascii_case("jsx")
        {
            self.collect_jsx_source_logic(&logic);
        }
    }

    fn collect_jsx_source_logic(&mut self, logic: &RenderSourceLogic) {
        let Some(component) = &logic.component else {
            return;
        };
        for item in &component.items {
            if let RenderSourceLogicItem::Snippet(snippet) = item {
                let mut scope = Scope::default();
                for param in &snippet.params {
                    for local in binding_pattern_locals(param) {
                        scope = scope.with_local(local);
                    }
                }
                for (index, node) in snippet.nodes.iter().enumerate() {
                    self.collect_node(node, &scope, &format!("snippet_{}_{}", snippet.name, index));
                }
            }
        }
    }

    fn collect_node(&mut self, node: &RenderNode, scope: &Scope, path: &str) {
        match node {
            RenderNode::Element(element) => self.collect_element(element, scope, path),
            RenderNode::Text(text) => {
                if let Some(template) = &text.template {
                    self.collect_template_props(template, scope);
                }
            }
            RenderNode::Raw(_) => {}
        }
    }
    fn collect_element(&mut self, element: &RenderElement, scope: &Scope, path: &str) {
        if let Some(component) = jsx_component_placeholder_name(element) {
            self.cx.push(Diagnostic::warning(
                format!(
                    "JSX component tag `{component}` is outside Phase 1 scope; emitted placeholder"
                ),
                element.span,
            ));
            return;
        }
        if let Some(style_block) = inline_style_block(element) {
            self.prelude.inline_style_blocks.push(style_block);
            return;
        }
        let scope =
            scope_for_control_flow(element.control_flow.as_deref(), scope, &mut self.prelude);
        if let Some(control_flow) = element.control_flow.as_deref()
            && let Some(expression) = &control_flow.expression
        {
            self.collect_expr_props(
                expression,
                &scope,
                control_flow.kind == RenderControlFlowKind::For,
            );
        }
        let mut element_scope = scope.clone();
        if let Some(control_flow) = element.control_flow.as_deref() {
            if let Some(key) = &control_flow.key {
                self.collect_expr_props(key, &scope, false);
            }
            for local in &control_flow.locals {
                self.collect_expr_props(&local.value, &element_scope, false);
                for name in binding_pattern_locals(&local.name) {
                    element_scope = element_scope.with_local(name);
                }
            }
        }
        self.collect_browser_only_style_diagnostics(element);
        let emits_dom_node = element_emits_dom_node(element);
        if emits_dom_node {
            self.collect_stylesheet_rules(element);
            self.collect_stylesheet_variant_rules(element);
        }
        if element_needs_style_class(element) && emits_dom_node {
            let class = format!("hs_{}", self.prelude.style_classes.len());
            self.prelude
                .style_classes
                .insert(path.to_owned(), class.clone());
            self.collect_element_style_rules(element, &class);
        }
        if element_is_render_slot(element) {
            self.prelude.snippet_prop(slot_prop_name(element));
        }
        if let Some(source_intent) = element.source_intent.as_deref() {
            self.collect_source_intent(element, source_intent, &element_scope);
        }
        for attribute in &element.attributes {
            if attribute.name == "data-htmlswap-attach" {
                self.prelude.uses_attachment = true;
                for root in loose_expression_roots(&attribute.value) {
                    if !element_scope.is_local(&root) && !self.prelude.states.contains_key(&root) {
                        self.prelude.prop(root);
                    }
                }
            }
            if let Some(template) = &attribute.template {
                self.collect_template_props(template, &element_scope);
            }
        }
        for style in &element.dynamic_styles {
            self.prelude.uses_style_helper = true;
            self.collect_template_props(&style.expression, &element_scope);
        }
        for action in &element.actions {
            self.collect_action_props(action, &element_scope);
        }
        for (index, child) in element.children.iter().enumerate() {
            self.collect_node(child, &element_scope, &format!("{path}_{index}"));
        }
    }

    fn collect_source_intent(
        &mut self,
        element: &RenderElement,
        source_intent: &RenderSourceIntent,
        scope: &Scope,
    ) {
        let emits_component = should_emit_svelte_component(element);
        if emits_component {
            if let Some(component) = &source_intent.component {
                let component_name = component_name(component.as_str());
                if let Some(source) = &source_intent.component_source {
                    if !component_source_is_external(source) {
                        self.prelude
                            .component_imports
                            .entry(component_name)
                            .or_insert_with(|| svelte_component_import_specifier(source));
                    }
                } else if element.source_tag == "dc-import" {
                    self.prelude
                        .component_imports
                        .entry(component_name.clone())
                        .or_insert_with(|| {
                            js_string_literal(&format!("./{component_name}.svelte"))
                        });
                } else {
                    self.prelude.prop(component_name);
                }
            }
            for prop in &source_intent.props {
                if let Some(template) = &prop.template {
                    self.collect_template_props(template, scope);
                }
            }
        }
    }

    fn collect_template_props(&mut self, template: &TemplateString, scope: &Scope) {
        for expression in template.expressions() {
            self.collect_expr_props(expression, scope, false);
        }
    }

    fn collect_expr_props(&mut self, expression: &Expr, scope: &Scope, default_array: bool) {
        for root in expression_roots(expression) {
            if self.prelude.states.contains_key(&root)
                || scope.is_local(&root)
                || should_skip_prop_name(&root)
                || self
                    .prelude
                    .source_logic
                    .as_ref()
                    .is_some_and(|logic| jsx_source_logic_declares(logic, &root))
            {
                continue;
            }
            if default_array {
                self.prelude.prop_with_default(root, "[]");
            } else {
                self.prelude.prop(root);
            }
        }
    }

    fn collect_action_props(&mut self, action: &ActionBinding, scope: &Scope) {
        for root in action_expression_roots(action) {
            if !scope.is_local(&root) && !self.prelude.states.contains_key(&root) {
                self.prelude.prop(root);
            }
        }
        for invocation in &action.handler.invocations {
            if !scope.is_local(&invocation.action)
                && !self.prelude.states.contains_key(invocation.action.as_str())
            {
                self.prelude.prop(invocation.action.to_string());
            }
        }
    }

    fn collect_stylesheet_rules(&mut self, element: &RenderElement) {
        for rule in &element.stylesheet_rules {
            if let Some(rule_text) = conditioned_style_rule(
                rule.selector.to_string(),
                &rule.conditions,
                &rule.declarations,
            ) {
                self.prelude.insert_stylesheet_rule_once(
                    StyleRuleOrderKey::new(rule.span, rule.source_order),
                    rule_text,
                );
            }
        }
    }

    fn collect_stylesheet_variant_rules(&mut self, element: &RenderElement) {
        for variant in &element.style_variants {
            if let Some(rule) = stylesheet_variant_rule(variant) {
                self.prelude
                    .insert_deduped_stylesheet_rule(variant.span, rule);
            }
        }
        for pseudo in &element.pseudo_elements {
            if let Some(rule) = stylesheet_pseudo_element_rule(pseudo) {
                self.prelude
                    .insert_deduped_stylesheet_rule(pseudo.span, rule);
            }
        }
    }

    fn collect_element_style_rules(&mut self, element: &RenderElement, class: &str) {
        for variant in &element.style_variants {
            if !uses_generated_style_class(&variant.selector) {
                continue;
            }
            if let Some(rule) = style_variant_rule(&format!(".{class}"), variant) {
                self.prelude.style_rules.push(rule);
            }
        }
        for pseudo in &element.pseudo_elements {
            if !uses_generated_style_class(&pseudo.selector) {
                continue;
            }
            if let Some(rule) = pseudo_element_rule(&format!(".{class}"), pseudo) {
                self.prelude.style_rules.push(rule);
            }
        }
    }

    fn collect_browser_only_style_diagnostics(&mut self, element: &RenderElement) {
        for declaration in &element.styles {
            let Some(description) = browser_only_style_description(declaration) else {
                continue;
            };
            self.cx.push(Diagnostic::warning(
                format!(
                    "Svelte adapter emitted browser-only CSS declaration `{description}`; this violates the flexbox-only authoring contract"
                ),
                element.span,
            ));
        }
    }

    fn collect_scripts(&mut self, scripts: &[RenderScriptReference]) {
        for script in scripts {
            let load_before_lifecycle = script.kind != RenderScriptKind::Module
                && self
                    .prelude
                    .source_logic
                    .as_ref()
                    .is_some_and(source_logic_uses_mount);
            if let Some(source) = script
                .resolved_source
                .and_then(|source_id| self.sources.file(source_id))
                .filter(|source| source.kind() == SourceKind::JavaScript)
            {
                let path = svelte_script_output_path(script, source);
                self.script_files
                    .push(GeneratedFile::new(path, source.text().to_owned()));
                if script.kind == RenderScriptKind::Module {
                    let alias = module_alias_for_script(script);
                    self.prelude.module_imports.push(format!(
                        "import * as {alias} from {};",
                        js_string_literal(&svelte_script_import_specifier(script))
                    ));
                } else {
                    self.cx.push(Diagnostic::warning(
                        format!(
                            "Svelte adapter copied classic script `{}`; classic scripts are loaded before source lifecycle hooks",
                            script.src
                        ),
                        script.span,
                    ));
                    if load_before_lifecycle {
                        self.prelude
                            .runtime_classic_scripts
                            .push(script.src.to_string());
                    } else {
                        self.prelude.head_scripts.push(head_script_tag(script));
                    }
                }
            } else if load_before_lifecycle {
                self.prelude
                    .runtime_classic_scripts
                    .push(script.src.to_string());
            } else {
                self.prelude.head_scripts.push(head_script_tag(script));
            }
        }
    }

    fn write_script(&self, output: &mut String) {
        let lang = if self.options.typescript {
            " lang=\"ts\""
        } else {
            ""
        };
        writeln!(output, "<script{lang}>").expect("writing to String cannot fail");
        let svelte_imports = self.svelte_runtime_imports();
        if !svelte_imports.is_empty() {
            writeln!(
                output,
                "\timport {{ {} }} from 'svelte';",
                svelte_imports.join(", ")
            )
            .expect("writing to String cannot fail");
        }
        if self.prelude.uses_snippet {
            writeln!(output, "\timport type {{ Snippet }} from 'svelte';")
                .expect("writing to String cannot fail");
        }
        for (name, source) in &self.prelude.component_imports {
            writeln!(output, "\timport {name} from {source};")
                .expect("writing to String cannot fail");
        }
        for import in &self.prelude.module_imports {
            writeln!(output, "\t{import}").expect("writing to String cannot fail");
        }
        if self.prelude.uses_attachment {
            writeln!(
                output,
                "\t// Attachments use Svelte 5 {{@attach ...}} syntax in markup."
            )
            .expect("writing to String cannot fail");
        }
        if let Some(logic) = &self.prelude.source_logic {
            if logic.dialect.eq_ignore_ascii_case("jsx") {
                self.write_jsx_source_logic(output, logic);
            } else {
                self.write_source_logic_bridge(output, logic);
            }
        } else if !self.prelude.props.is_empty() {
            self.write_props(output);
        }
        for (name, value) in &self.prelude.states {
            writeln!(output, "\tlet {name} = $state({value});")
                .expect("writing to String cannot fail");
        }
        if self.prelude.uses_style_helper {
            self.write_style_helper(output);
        }
        if !self.prelude.runtime_classic_scripts.is_empty() {
            self.write_runtime_script_loader(output);
        }
        writeln!(output, "</script>").expect("writing to String cannot fail");
    }

    fn svelte_runtime_imports(&self) -> Vec<&'static str> {
        let mut imports = Vec::new();
        if let Some(logic) = &self.prelude.source_logic {
            if source_logic_uses_mount(logic) {
                imports.push("onMount");
            }
            if source_logic_uses_did_update(logic) {
                imports.push("tick");
            }
        }
        imports
    }

    fn write_props(&self, output: &mut String) {
        let names = self
            .prelude
            .props
            .values()
            .map(|prop| match &prop.default {
                Some(default) => format!("{} = {default}", prop.name),
                None => prop.name.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        if self.options.typescript {
            let fields = self
                .prelude
                .props
                .values()
                .map(|prop| {
                    let ty = if prop.snippet { "Snippet" } else { "any" };
                    format!("{}?: {ty}", prop.name)
                })
                .collect::<Vec<_>>()
                .join("; ");
            writeln!(output, "\tlet {{ {names} }}: {{ {fields} }} = $props();")
                .expect("writing to String cannot fail");
        } else {
            writeln!(output, "\tlet {{ {names} }} = $props();")
                .expect("writing to String cannot fail");
        }
    }

    fn write_jsx_source_logic(&self, output: &mut String, logic: &RenderSourceLogic) {
        self.write_jsx_props(output, logic);
        self.write_indented_source_logic_body(output, logic);
        self.write_jsx_component_logic(output, logic);
        if !logic.body.trim().is_empty() {
            writeln!(output).expect("writing to String cannot fail");
        }
    }

    fn write_jsx_component_logic(&self, output: &mut String, logic: &RenderSourceLogic) {
        let Some(component) = &logic.component else {
            return;
        };
        if !logic.body.trim().is_empty() {
            writeln!(output).expect("writing to String cannot fail");
        }
        for item in &component.items {
            match item {
                RenderSourceLogicItem::Local(local) => self.write_jsx_local(output, local),
                RenderSourceLogicItem::State(state) => self.write_jsx_state(output, state),
                RenderSourceLogicItem::Derived(derived) => self.write_jsx_derived(output, derived),
                RenderSourceLogicItem::Ref(reference) => self.write_jsx_ref(output, reference),
                RenderSourceLogicItem::Callback(callback) => {
                    self.write_jsx_callback(output, callback)
                }
                RenderSourceLogicItem::Mount(mount) => self.write_jsx_mount(output, mount),
                RenderSourceLogicItem::Effect(effect) => self.write_jsx_effect(output, effect),
                RenderSourceLogicItem::Snippet(_) => {}
            }
        }
    }

    fn write_jsx_local(&self, output: &mut String, local: &RenderSourceLocal) {
        write_indented_block(output, local.body.as_str(), 1);
    }

    fn write_jsx_state(&self, output: &mut String, state: &RenderSourceState) {
        writeln!(output, "\tlet {} = $state({});", state.name, state.initial)
            .expect("writing to String cannot fail");
    }

    fn write_jsx_derived(&self, output: &mut String, derived: &RenderSourceDerived) {
        if derived.by {
            writeln!(output, "\tlet {} = $derived.by(() => {{", derived.name)
                .expect("writing to String cannot fail");
            write_indented_block(output, derived.body.as_str(), 2);
            writeln!(output, "\t}});").expect("writing to String cannot fail");
        } else {
            writeln!(
                output,
                "\tlet {} = $derived({});",
                derived.name, derived.body
            )
            .expect("writing to String cannot fail");
        }
    }

    fn write_jsx_ref(&self, output: &mut String, reference: &RenderSourceRef) {
        if reference.dom && matches!(reference.initial.as_str(), "null" | "undefined") {
            writeln!(output, "\tlet {};", reference.name).expect("writing to String cannot fail");
        } else {
            writeln!(output, "\tlet {} = {};", reference.name, reference.initial)
                .expect("writing to String cannot fail");
        }
    }

    fn write_jsx_callback(&self, output: &mut String, callback: &RenderSourceCallback) {
        writeln!(output, "\tconst {} = {};", callback.name, callback.body)
            .expect("writing to String cannot fail");
    }

    fn write_jsx_mount(&self, output: &mut String, mount: &RenderSourceMount) {
        writeln!(output, "\tonMount(() => {{").expect("writing to String cannot fail");
        write_indented_block(output, mount.body.as_str(), 2);
        writeln!(output, "\t}});").expect("writing to String cannot fail");
    }

    fn write_jsx_effect(&self, output: &mut String, effect: &RenderSourceEffect) {
        writeln!(output, "\t$effect(() => {{").expect("writing to String cannot fail");
        for dependency in &effect.dependencies {
            writeln!(output, "\t\t{dependency};").expect("writing to String cannot fail");
        }
        write_indented_block(output, effect.body.as_str(), 2);
        writeln!(output, "\t}});").expect("writing to String cannot fail");
    }

    fn write_jsx_props(&self, output: &mut String, logic: &RenderSourceLogic) {
        let mut entries = Vec::new();
        if let Some(data_props) = &logic.data_props {
            let data_props = data_props.trim();
            if let Some(inner) = data_props
                .strip_prefix('{')
                .and_then(|value| value.strip_suffix('}'))
            {
                let inner = inner.trim();
                if !inner.is_empty() {
                    entries.push(inner.to_owned());
                }
            } else if !data_props.is_empty() {
                writeln!(output, "\tlet {data_props} = $props();")
                    .expect("writing to String cannot fail");
            }
        }
        for prop in self.prelude.props.values() {
            if jsx_props_entries_contain(&entries, &prop.name)
                || jsx_source_logic_declares(logic, &prop.name)
            {
                continue;
            }
            match &prop.default {
                Some(default) => entries.push(format!("{} = {default}", prop.name)),
                None => entries.push(prop.name.clone()),
            }
        }
        if entries.is_empty() {
            return;
        }
        writeln!(output, "\tlet {{ {} }} = $props();", entries.join(", "))
            .expect("writing to String cannot fail");
    }

    fn write_source_logic_bridge(&self, output: &mut String, logic: &RenderSourceLogic) {
        let uses_mount = source_logic_uses_mount(logic);
        let uses_did_update = source_logic_uses_did_update(logic);
        if self.options.typescript {
            writeln!(
                output,
                "\tlet __htmlswap_props: Record<string, any> = $props();"
            )
            .expect("writing to String cannot fail");
        } else {
            writeln!(output, "\tlet __htmlswap_props = $props();")
                .expect("writing to String cannot fail");
        }
        writeln!(output, "\tlet __htmlswap_state_version = $state(0);")
            .expect("writing to String cannot fail");
        if uses_mount || uses_did_update {
            writeln!(output, "\tlet __htmlswap_mounted = false;")
                .expect("writing to String cannot fail");
        }
        writeln!(output, "\tclass DCLogic {{").expect("writing to String cannot fail");
        if self.options.typescript {
            writeln!(output, "\t\tprops: any;").expect("writing to String cannot fail");
            writeln!(output, "\t\tstate: any = {{}};").expect("writing to String cannot fail");
            writeln!(output, "\t\tconstructor(props: any = {{}}) {{")
                .expect("writing to String cannot fail");
        } else {
            writeln!(output, "\t\tstate = {{}};").expect("writing to String cannot fail");
            writeln!(output, "\t\tconstructor(props = {{}}) {{")
                .expect("writing to String cannot fail");
        }
        writeln!(output, "\t\t\tthis.props = props;").expect("writing to String cannot fail");
        writeln!(output, "\t\t}}").expect("writing to String cannot fail");
        if self.options.typescript {
            writeln!(
                output,
                "\t\tsetState(update: any, callback?: () => void) {{"
            )
            .expect("writing to String cannot fail");
        } else {
            writeln!(output, "\t\tsetState(update, callback) {{")
                .expect("writing to String cannot fail");
        }
        writeln!(output, "\t\t\tconst prevState = this.state;")
            .expect("writing to String cannot fail");
        writeln!(
            output,
            "\t\t\tconst patch = typeof update === 'function' ? update(this.state, this.props) : update;"
        )
        .expect("writing to String cannot fail");
        writeln!(
            output,
            "\t\t\tif (patch && typeof patch === 'object') this.state = {{ ...this.state, ...patch }};"
        )
        .expect("writing to String cannot fail");
        writeln!(output, "\t\t\t__htmlswap_state_version += 1;")
            .expect("writing to String cannot fail");
        writeln!(output, "\t\t\tcallback?.();").expect("writing to String cannot fail");
        if uses_did_update {
            if self.options.typescript {
                writeln!(
                    output,
                    "\t\t\tif (__htmlswap_mounted && typeof (this as any).componentDidUpdate === 'function') tick().then(() => {{ if (__htmlswap_mounted) (this as any).componentDidUpdate(this.props, prevState); }});"
                )
                .expect("writing to String cannot fail");
            } else {
                writeln!(
                    output,
                    "\t\t\tif (__htmlswap_mounted && typeof this.componentDidUpdate === 'function') tick().then(() => {{ if (__htmlswap_mounted) this.componentDidUpdate(this.props, prevState); }});"
                )
                .expect("writing to String cannot fail");
            }
        }
        writeln!(output, "\t\t}}").expect("writing to String cannot fail");
        writeln!(
            output,
            "\t\tforceUpdate() {{ __htmlswap_state_version += 1; }}"
        )
        .expect("writing to String cannot fail");
        writeln!(output, "\t}}").expect("writing to String cannot fail");
        self.write_indented_source_logic_body(output, logic);
        writeln!(
            output,
            "\tconst __htmlswap_component = (() => new Component(__htmlswap_props))();"
        )
        .expect("writing to String cannot fail");
        if uses_mount {
            writeln!(output, "\tonMount(() => {{").expect("writing to String cannot fail");
            writeln!(output, "\t\tlet __htmlswap_cancelled = false;")
                .expect("writing to String cannot fail");
            writeln!(output, "\t\t__htmlswap_mounted = true;")
                .expect("writing to String cannot fail");
            if self.prelude.runtime_classic_scripts.is_empty() {
                writeln!(output, "\t\t__htmlswap_component.componentDidMount?.();")
                    .expect("writing to String cannot fail");
            } else {
                writeln!(
                    output,
                    "\t\t__htmlswap_load_classic_scripts().then(() => {{"
                )
                .expect("writing to String cannot fail");
                writeln!(
                    output,
                    "\t\t\tif (!__htmlswap_cancelled) __htmlswap_component.componentDidMount?.();"
                )
                .expect("writing to String cannot fail");
                writeln!(output, "\t\t}}).catch((error) => console.error(error));")
                    .expect("writing to String cannot fail");
            }
            writeln!(output, "\t\treturn () => {{").expect("writing to String cannot fail");
            writeln!(output, "\t\t\t__htmlswap_cancelled = true;")
                .expect("writing to String cannot fail");
            writeln!(output, "\t\t\t__htmlswap_mounted = false;")
                .expect("writing to String cannot fail");
            writeln!(
                output,
                "\t\t\t__htmlswap_component.componentWillUnmount?.();"
            )
            .expect("writing to String cannot fail");
            writeln!(output, "\t\t}};").expect("writing to String cannot fail");
            writeln!(output, "\t}});").expect("writing to String cannot fail");
        }
        writeln!(output, "\tlet __htmlswap_vals = $derived.by(() => {{")
            .expect("writing to String cannot fail");
        writeln!(output, "\t\t__htmlswap_state_version;").expect("writing to String cannot fail");
        writeln!(output, "\t\t__htmlswap_component.props = __htmlswap_props;")
            .expect("writing to String cannot fail");
        writeln!(
            output,
            "\t\treturn __htmlswap_component.renderVals?.() ?? {{}};"
        )
        .expect("writing to String cannot fail");
        writeln!(output, "\t}});").expect("writing to String cannot fail");
        writeln!(
            output,
            "\tconst __htmlswap_has = Object.prototype.hasOwnProperty;"
        )
        .expect("writing to String cannot fail");
        if self.options.typescript {
            writeln!(
                output,
                "\tconst __htmlswap_value = (name: string) => __htmlswap_has.call(__htmlswap_vals, name) ? __htmlswap_vals[name] : __htmlswap_props[name];"
            )
            .expect("writing to String cannot fail");
        } else {
            writeln!(
                output,
                "\tconst __htmlswap_value = (name) => __htmlswap_has.call(__htmlswap_vals, name) ? __htmlswap_vals[name] : __htmlswap_props[name];"
            )
            .expect("writing to String cannot fail");
        }
        for prop in self.prelude.props.values() {
            let default = prop
                .default
                .as_ref()
                .map(|default| format!(" ?? {default}"))
                .unwrap_or_default();
            writeln!(
                output,
                "\tlet {} = $derived(__htmlswap_value({}){});",
                prop.name,
                js_string_literal(&prop.name),
                default
            )
            .expect("writing to String cannot fail");
        }
    }

    fn write_indented_source_logic_body(&self, output: &mut String, logic: &RenderSourceLogic) {
        let body = logic.body.trim();
        if body.is_empty() {
            return;
        }
        for line in body.lines() {
            writeln!(output, "\t{line}").expect("writing to String cannot fail");
        }
    }

    fn write_style_helper(&self, output: &mut String) {
        if self.options.typescript {
            writeln!(output, "\tfunction __htmlswap_style(value: any): string {{")
                .expect("writing to String cannot fail");
        } else {
            writeln!(output, "\tfunction __htmlswap_style(value) {{")
                .expect("writing to String cannot fail");
        }
        writeln!(
            output,
            "\t\tif (value == null || value === false) return '';"
        )
        .expect("writing to String cannot fail");
        writeln!(output, "\t\tif (typeof value === 'string') return value;")
            .expect("writing to String cannot fail");
        writeln!(
            output,
            "\t\tif (typeof value !== 'object') return String(value);"
        )
        .expect("writing to String cannot fail");
        writeln!(
            output,
            "\t\treturn Object.entries(value).filter(([, item]) => item != null && item !== false).map(([key, item]) => `${{__htmlswap_kebab(key)}}:${{String(item)}}`).join(';');"
        )
        .expect("writing to String cannot fail");
        writeln!(output, "\t}}").expect("writing to String cannot fail");
        if self.options.typescript {
            writeln!(
                output,
                "\tfunction __htmlswap_kebab(value: string): string {{"
            )
            .expect("writing to String cannot fail");
        } else {
            writeln!(output, "\tfunction __htmlswap_kebab(value) {{")
                .expect("writing to String cannot fail");
        }
        writeln!(
            output,
            "\t\treturn value.replace(/[A-Z]/g, (ch) => `-${{ch.toLowerCase()}}`);"
        )
        .expect("writing to String cannot fail");
        writeln!(output, "\t}}").expect("writing to String cannot fail");
    }

    fn write_runtime_script_loader(&self, output: &mut String) {
        let scripts = self
            .prelude
            .runtime_classic_scripts
            .iter()
            .map(|src| js_string_literal(src))
            .collect::<Vec<_>>()
            .join(", ");
        if self.options.typescript {
            writeln!(
                output,
                "\tconst __htmlswap_classic_scripts: string[] = [{scripts}];"
            )
            .expect("writing to String cannot fail");
            writeln!(
                output,
                "\tfunction __htmlswap_load_classic_scripts(): Promise<void> {{"
            )
            .expect("writing to String cannot fail");
            writeln!(
                output,
                "\t\treturn __htmlswap_classic_scripts.reduce((chain, src) => chain.then(() => __htmlswap_load_script(src)), Promise.resolve());"
            )
            .expect("writing to String cannot fail");
            writeln!(output, "\t}}").expect("writing to String cannot fail");
            writeln!(
                output,
                "\tfunction __htmlswap_load_script(src: string): Promise<void> {{"
            )
            .expect("writing to String cannot fail");
        } else {
            writeln!(output, "\tconst __htmlswap_classic_scripts = [{scripts}];")
                .expect("writing to String cannot fail");
            writeln!(output, "\tfunction __htmlswap_load_classic_scripts() {{")
                .expect("writing to String cannot fail");
            writeln!(
                output,
                "\t\treturn __htmlswap_classic_scripts.reduce((chain, src) => chain.then(() => __htmlswap_load_script(src)), Promise.resolve());"
            )
            .expect("writing to String cannot fail");
            writeln!(output, "\t}}").expect("writing to String cannot fail");
            writeln!(output, "\tfunction __htmlswap_load_script(src) {{")
                .expect("writing to String cannot fail");
        }
        writeln!(
            output,
            "\t\tconst url = new URL(src, document.baseURI).href;"
        )
        .expect("writing to String cannot fail");
        writeln!(
            output,
            "\t\tconst existing = Array.from(document.scripts).find((script) => script.src === url);"
        )
        .expect("writing to String cannot fail");
        writeln!(
            output,
            "\t\tif (existing?.dataset.htmlswapLoaded === 'true') return Promise.resolve();"
        )
        .expect("writing to String cannot fail");
        writeln!(output, "\t\treturn new Promise((resolve, reject) => {{")
            .expect("writing to String cannot fail");
        writeln!(
            output,
            "\t\t\tconst script = existing ?? document.createElement('script');"
        )
        .expect("writing to String cannot fail");
        writeln!(
            output,
            "\t\t\tscript.addEventListener('load', () => {{ script.dataset.htmlswapLoaded = 'true'; resolve(); }}, {{ once: true }});"
        )
        .expect("writing to String cannot fail");
        writeln!(
            output,
            "\t\t\tscript.addEventListener('error', () => reject(new Error(`Failed to load script ${{src}}`)), {{ once: true }});"
        )
        .expect("writing to String cannot fail");
        writeln!(output, "\t\t\tif (!existing) {{").expect("writing to String cannot fail");
        writeln!(output, "\t\t\t\tscript.src = src;").expect("writing to String cannot fail");
        writeln!(output, "\t\t\t\tdocument.head.appendChild(script);")
            .expect("writing to String cannot fail");
        writeln!(output, "\t\t\t}}").expect("writing to String cannot fail");
        writeln!(output, "\t\t}});").expect("writing to String cannot fail");
        writeln!(output, "\t}}").expect("writing to String cannot fail");
    }

    fn write_head(&self, output: &mut String, head: &[RenderHeadElement]) {
        if head.is_empty() && self.prelude.head_scripts.is_empty() {
            return;
        }
        writeln!(output, "<svelte:head>").expect("writing to String cannot fail");
        for element in head {
            writeln!(output, "\t{}", element.html).expect("writing to String cannot fail");
        }
        for script in &self.prelude.head_scripts {
            writeln!(output, "\t{script}").expect("writing to String cannot fail");
        }
        writeln!(output, "</svelte:head>").expect("writing to String cannot fail");
    }

    fn write_style(&self, output: &mut String, plan: &RenderPlan) {
        let has_root = !plan.root.styles.is_empty() || !plan.root.style_variants.is_empty();
        if !has_root
            && self.prelude.stylesheet_rules.is_empty()
            && self.prelude.style_rules.is_empty()
            && self.prelude.inline_style_blocks.is_empty()
        {
            return;
        }
        writeln!(output, "<style>").expect("writing to String cannot fail");
        if !plan.root.styles.is_empty() {
            writeln!(output, "\t:global(:root) {{").expect("writing to String cannot fail");
            for declaration in &plan.root.styles {
                write_style_declaration(output, declaration, 2);
            }
            writeln!(output, "\t}}").expect("writing to String cannot fail");
        }
        for variant in &plan.root.style_variants {
            if let Some(rule) =
                style_variant_rule(&format!(":global({})", variant.selector), variant)
            {
                write_indented_block(output, &rule, 1);
            }
        }
        for rule in self.prelude.stylesheet_rules.values() {
            write_indented_block(output, rule, 1);
        }
        for rule in &self.prelude.style_rules {
            write_indented_block(output, rule, 1);
        }
        for block in &self.prelude.inline_style_blocks {
            write_indented_style_block(output, block, 1);
        }
        writeln!(output, "</style>").expect("writing to String cannot fail");
    }

    fn write_jsx_snippets(&self, output: &mut String, depth: usize) {
        let Some(logic) = &self.prelude.source_logic else {
            return;
        };
        if !logic.dialect.eq_ignore_ascii_case("jsx") {
            return;
        }
        let Some(component) = &logic.component else {
            return;
        };
        for item in &component.items {
            let RenderSourceLogicItem::Snippet(snippet) = item else {
                continue;
            };
            self.write_jsx_snippet(output, snippet, depth);
        }
    }

    fn write_jsx_snippet(&self, output: &mut String, snippet: &RenderSourceSnippet, depth: usize) {
        let params = snippet
            .params
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            output,
            "{}{{#snippet {}({params})}}",
            indent(depth),
            snippet.name
        )
        .expect("writing to String cannot fail");
        let mut scope = Scope::default();
        for param in &snippet.params {
            for local in binding_pattern_locals(param) {
                scope = scope.with_local(local);
            }
        }
        self.write_nodes(
            output,
            &snippet.nodes,
            depth + 1,
            &scope,
            &format!("snippet_{}", snippet.name),
        );
        writeln!(output, "{}{{/snippet}}", indent(depth)).expect("writing to String cannot fail");
    }
    fn write_nodes(
        &self,
        output: &mut String,
        nodes: &[RenderNode],
        depth: usize,
        scope: &Scope,
        path: &str,
    ) {
        let mut index = 0usize;
        while index < nodes.len() {
            if let RenderNode::Element(element) = &nodes[index]
                && let Some(control_flow) = element.control_flow.as_deref()
            {
                match control_flow.kind {
                    RenderControlFlowKind::If => {
                        index = self.write_if_chain(output, nodes, index, depth, scope, path);
                        continue;
                    }
                    RenderControlFlowKind::ElseIf | RenderControlFlowKind::Else => {
                        self.write_control_flow_as_if(
                            output,
                            element,
                            depth,
                            scope,
                            &format!("{path}_{index}"),
                        );
                        index += 1;
                        continue;
                    }
                    RenderControlFlowKind::For => {
                        self.write_for(
                            output,
                            element,
                            control_flow,
                            depth,
                            scope,
                            &format!("{path}_{index}"),
                        );
                        index += 1;
                        continue;
                    }
                    RenderControlFlowKind::Switch
                    | RenderControlFlowKind::Case
                    | RenderControlFlowKind::Default => {
                        self.write_control_flow_as_if(
                            output,
                            element,
                            depth,
                            scope,
                            &format!("{path}_{index}"),
                        );
                        index += 1;
                        continue;
                    }
                }
            }
            self.write_node(
                output,
                &nodes[index],
                depth,
                scope,
                &format!("{path}_{index}"),
                false,
            );
            index += 1;
        }
    }

    fn write_if_chain(
        &self,
        output: &mut String,
        nodes: &[RenderNode],
        start: usize,
        depth: usize,
        scope: &Scope,
        path: &str,
    ) -> usize {
        let mut index = start;
        let mut first = true;
        while index < nodes.len() {
            let RenderNode::Element(element) = &nodes[index] else {
                break;
            };
            let Some(control_flow) = element.control_flow.as_deref() else {
                break;
            };
            match control_flow.kind {
                RenderControlFlowKind::If if first => {
                    let expr = control_flow
                        .expression
                        .as_ref()
                        .map(svelte_expr)
                        .unwrap_or_else(|| "false".to_owned());
                    writeln!(output, "{}{{#if {expr}}}", indent(depth))
                        .expect("writing to String cannot fail");
                }
                RenderControlFlowKind::ElseIf => {
                    let expr = control_flow
                        .expression
                        .as_ref()
                        .map(svelte_expr)
                        .unwrap_or_else(|| "false".to_owned());
                    writeln!(output, "{}{{:else if {expr}}}", indent(depth))
                        .expect("writing to String cannot fail");
                }
                RenderControlFlowKind::Else => {
                    writeln!(output, "{}{{:else}}", indent(depth))
                        .expect("writing to String cannot fail");
                }
                _ if first => break,
                _ => break,
            }
            let mut inner_scope =
                scope_for_control_flow(Some(control_flow), scope, &mut SveltePrelude::default());
            self.write_control_flow_locals(output, control_flow, depth + 1, &mut inner_scope);
            self.write_element_or_wrapper(
                output,
                element,
                depth + 1,
                &inner_scope,
                &format!("{path}_{index}"),
                true,
            );
            first = false;
            index += 1;
        }
        writeln!(output, "{}{{/if}}", indent(depth)).expect("writing to String cannot fail");
        index
    }

    fn write_for(
        &self,
        output: &mut String,
        element: &RenderElement,
        control_flow: &RenderControlFlow,
        depth: usize,
        scope: &Scope,
        path: &str,
    ) {
        let expr = control_flow
            .expression
            .as_ref()
            .map(svelte_expr)
            .unwrap_or_else(|| "[]".to_owned());
        let binding_pattern = control_flow
            .binding
            .as_ref()
            .map(|binding| svelte_binding_pattern(&binding.name))
            .unwrap_or_else(|| "item".to_owned());
        let index_binding = control_flow
            .index_binding
            .as_ref()
            .map(|binding| sanitize_js_identifier(&binding.name, "index"));
        let key = control_flow
            .key
            .as_ref()
            .map(|key| format!(" ({})", svelte_expr(key)))
            .unwrap_or_default();
        let each_binding = if let Some(index_binding) = &index_binding {
            format!("{binding_pattern}, {index_binding}")
        } else if control_flow.key.is_some() {
            binding_pattern.clone()
        } else {
            format!("{binding_pattern}, $index")
        };
        writeln!(
            output,
            "{}{{#each {expr} as {each_binding}{key}}}",
            indent(depth)
        )
        .expect("writing to String cannot fail");
        let mut inner_scope = scope.with_local("$index");
        if let Some(binding) = &control_flow.binding {
            for local in binding_pattern_locals(&binding.name) {
                inner_scope = inner_scope.with_local(local);
            }
        } else {
            inner_scope = inner_scope.with_local("item");
        }
        if let Some(index_binding) = index_binding {
            inner_scope = inner_scope.with_local(index_binding);
        }
        self.write_control_flow_locals(output, control_flow, depth + 1, &mut inner_scope);
        self.write_element_or_wrapper(output, element, depth + 1, &inner_scope, path, true);
        writeln!(output, "{}{{/each}}", indent(depth)).expect("writing to String cannot fail");
    }

    fn write_control_flow_locals(
        &self,
        output: &mut String,
        control_flow: &RenderControlFlow,
        depth: usize,
        scope: &mut Scope,
    ) {
        for local in &control_flow.locals {
            let pattern = svelte_binding_pattern(&local.name);
            writeln!(
                output,
                "{}{{@const {} = {}}}",
                indent(depth),
                pattern,
                svelte_expr(&local.value)
            )
            .expect("writing to String cannot fail");
            for name in binding_pattern_locals(&local.name) {
                *scope = scope.clone().with_local(name);
            }
        }
    }

    fn write_control_flow_as_if(
        &self,
        output: &mut String,
        element: &RenderElement,
        depth: usize,
        scope: &Scope,
        path: &str,
    ) {
        let expr = element
            .control_flow
            .as_deref()
            .and_then(|control_flow| control_flow.expression.as_ref())
            .map(svelte_expr)
            .unwrap_or_else(|| "true".to_owned());
        writeln!(output, "{}{{#if {expr}}}", indent(depth)).expect("writing to String cannot fail");
        let mut inner_scope = scope.clone();
        if let Some(control_flow) = element.control_flow.as_deref() {
            self.write_control_flow_locals(output, control_flow, depth + 1, &mut inner_scope);
        }
        self.write_element_or_wrapper(output, element, depth + 1, &inner_scope, path, true);
        writeln!(output, "{}{{/if}}", indent(depth)).expect("writing to String cannot fail");
    }

    fn write_node(
        &self,
        output: &mut String,
        node: &RenderNode,
        depth: usize,
        scope: &Scope,
        path: &str,
        suppress_control_flow: bool,
    ) {
        match node {
            RenderNode::Element(element) => self.write_element_or_wrapper(
                output,
                element,
                depth,
                scope,
                path,
                suppress_control_flow,
            ),
            RenderNode::Text(text) => self.write_text(output, text, depth),
            RenderNode::Raw(raw) => self.write_raw(output, raw, depth),
        }
    }

    fn write_element_or_wrapper(
        &self,
        output: &mut String,
        element: &RenderElement,
        depth: usize,
        scope: &Scope,
        path: &str,
        suppress_control_flow: bool,
    ) {
        if !suppress_control_flow && element.control_flow.is_some() {
            self.write_nodes(
                output,
                &[RenderNode::Element(Box::new(element.clone()))],
                depth,
                scope,
                path,
            );
            return;
        }
        if is_control_flow_wrapper(element) {
            self.write_nodes(output, &element.children, depth, scope, path);
            return;
        }
        if element.source_tag == "jsx-render" {
            if let Some(call) = attribute_value(element, "data-htmlswap-render") {
                writeln!(output, "{}{{@render {call}}}", indent(depth))
                    .expect("writing to String cannot fail");
            }
            return;
        }
        if let Some(component) = jsx_component_placeholder_name(element) {
            self.write_jsx_component_placeholder(output, component, depth);
            return;
        }
        if inline_style_block(element).is_some() {
            return;
        }
        if element_is_render_slot(element) {
            writeln!(
                output,
                "{}{{@render {}?.()}}",
                indent(depth),
                slot_prop_name(element)
            )
            .expect("writing to String cannot fail");
            return;
        }
        self.write_element(output, element, depth, scope, path);
    }

    fn write_jsx_component_placeholder(&self, output: &mut String, component: &str, depth: usize) {
        writeln!(
            output,
            "{}<div class=\"htmlswap-jsx-component-placeholder\" data-htmlswap-jsx-component-placeholder=\"{}\">",
            indent(depth),
            escape_html_attribute(component)
        )
        .expect("writing to String cannot fail");
        writeln!(
            output,
            "{}[unsupported JSX component: {}]",
            indent(depth + 1),
            escape_svelte_text(component)
        )
        .expect("writing to String cannot fail");
        writeln!(output, "{}</div>", indent(depth)).expect("writing to String cannot fail");
    }

    fn write_element(
        &self,
        output: &mut String,
        element: &RenderElement,
        depth: usize,
        scope: &Scope,
        path: &str,
    ) {
        if self.options.emit_source_comments
            && let Some(comment) = source_comment(self.sources, element.span)
        {
            writeln!(
                output,
                "{}<!-- htmlswap-source: {comment} -->",
                indent(depth)
            )
            .expect("writing to String cannot fail");
        }
        let tag = tag_for_element(element);
        write!(output, "{}<{tag}", indent(depth)).expect("writing to String cannot fail");
        self.write_attributes(output, element, scope, path);
        if is_void_html_tag(&tag) && element.children.is_empty() {
            writeln!(output, " />").expect("writing to String cannot fail");
            return;
        }
        writeln!(output, ">").expect("writing to String cannot fail");
        self.write_nodes(output, &element.children, depth + 1, scope, path);
        writeln!(output, "{}</{tag}>", indent(depth)).expect("writing to String cannot fail");
    }

    fn write_attributes(
        &self,
        output: &mut String,
        element: &RenderElement,
        scope: &Scope,
        path: &str,
    ) {
        let emits_component = should_emit_svelte_component(element);
        let mut classes = if emits_component {
            Vec::new()
        } else {
            element
                .classes
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        };
        if !emits_component && let Some(class) = self.prelude.style_classes.get(path) {
            classes.push(class.clone());
        }
        let mut written_attributes = BTreeSet::new();
        for action in &element.actions {
            written_attributes.insert(attribute_key(&svelte_event_name(&action.event)));
        }
        if element_has_static_or_dynamic_style(element) {
            written_attributes.insert(attribute_key("style"));
        }
        self.write_class_attribute(output, &classes, element, scope);
        if !classes.is_empty() || element_has_attribute(element, "class") {
            written_attributes.insert(attribute_key("class"));
        }
        if let Some(binding) = state_binding_attribute_name(element) {
            self.write_state_binding(output, element);
            written_attributes.insert(attribute_key(binding));
        }
        if let Some(accessibility) = &element.accessibility {
            if let Some(role) = &accessibility.role {
                write!(output, " role=\"{}\"", escape_html_attribute(role))
                    .expect("writing to String cannot fail");
                written_attributes.insert(attribute_key("role"));
            }
            if let Some(label) = &accessibility.label {
                write!(output, " aria-label=\"{}\"", escape_html_attribute(label))
                    .expect("writing to String cannot fail");
                written_attributes.insert(attribute_key("aria-label"));
            }
            if accessibility.hidden {
                write!(output, " aria-hidden=\"true\"").expect("writing to String cannot fail");
                written_attributes.insert(attribute_key("aria-hidden"));
            }
        }
        if needs_keyboard_activation_fallback(element) {
            if !element_has_role(element) {
                write!(output, " role=\"button\"").expect("writing to String cannot fail");
                written_attributes.insert(attribute_key("role"));
            }
            if !element_has_attribute(element, "tabindex") {
                write!(output, " tabindex=\"0\"").expect("writing to String cannot fail");
                written_attributes.insert(attribute_key("tabindex"));
            }
        }
        for attribute in &element.attributes {
            let name = attribute.name.as_str();
            if name == "{...}" {
                if let Some(template) = &attribute.template
                    && let Some(expression) = template.single_expression()
                {
                    write!(output, " {{...{}}}", svelte_expr(expression))
                        .expect("writing to String cannot fail");
                }
                continue;
            }
            if name == "data-htmlswap-attach" {
                if !written_attributes.insert(attribute_key(name)) {
                    continue;
                }
                if !attribute.value.is_empty() {
                    write!(output, " {{@attach {}}}", attribute.value)
                        .expect("writing to String cannot fail");
                }
                continue;
            }
            if name == "bind:this" {
                if !written_attributes.insert(attribute_key(name)) {
                    continue;
                }
                if !attribute.value.is_empty() {
                    write!(output, " bind:this={{{}}}", attribute.value)
                        .expect("writing to String cannot fail");
                }
                continue;
            }
            if name == "class" {
                continue;
            }
            if should_skip_svelte_attribute(name) {
                continue;
            }
            if has_target_state(element) && matches!(name, "value" | "checked") {
                continue;
            }
            if !written_attributes.insert(attribute_key(name)) {
                continue;
            }
            write!(output, " {name}").expect("writing to String cannot fail");
            if let Some(template) = &attribute.template {
                self.write_template_attribute_value(output, template, scope);
            } else if !attribute.value.is_empty() {
                write!(output, "=\"{}\"", escape_html_attribute(&attribute.value))
                    .expect("writing to String cannot fail");
            }
        }
        if should_emit_svelte_component(element)
            && let Some(source_intent) = element.source_intent.as_deref()
        {
            for prop in &source_intent.props {
                let name = prop.name.to_string().to_lower_camel_case();
                write!(output, " {name}").expect("writing to String cannot fail");
                if let Some(template) = &prop.template {
                    self.write_template_attribute_value(output, template, scope);
                } else if !prop.value.is_empty() {
                    write!(output, "=\"{}\"", escape_html_attribute(&prop.value))
                        .expect("writing to String cannot fail");
                }
            }
        }
        if !emits_component {
            if let Some(component) = element
                .source_intent
                .as_deref()
                .and_then(|intent| intent.component.as_ref())
                && written_attributes.insert(attribute_key("data-htmlswap-component"))
            {
                write!(
                    output,
                    " data-htmlswap-component=\"{}\"",
                    escape_html_attribute(component.as_str())
                )
                .expect("writing to String cannot fail");
            }
            self.write_style_attribute(output, element, scope);
            for action in &element.actions {
                self.write_event_attribute(output, element, action);
            }
            self.write_keyboard_activation_attribute(output, element);
        }
    }

    fn write_class_attribute(
        &self,
        output: &mut String,
        classes: &[String],
        element: &RenderElement,
        _scope: &Scope,
    ) {
        let class_attributes = element
            .attributes
            .iter()
            .filter(|attribute| attribute.name == "class")
            .collect::<Vec<_>>();
        if classes.is_empty() && class_attributes.is_empty() {
            return;
        }
        if class_attributes.is_empty() {
            write!(
                output,
                " class=\"{}\"",
                escape_html_attribute(&classes.join(" "))
            )
            .expect("writing to String cannot fail");
            return;
        }
        let static_classes = classes.join(" ");
        let mut segments = Vec::new();
        if !static_classes.is_empty() {
            segments.push(TemplateSegment::Literal(static_classes.into()));
        }
        for attribute in class_attributes {
            if let Some(template) = &attribute.template {
                if !segments.is_empty() {
                    segments.push(TemplateSegment::Literal(" ".into()));
                }
                segments.extend(template.segments.iter().cloned());
            } else if !attribute.value.is_empty() {
                if !segments.is_empty() {
                    segments.push(TemplateSegment::Literal(" ".into()));
                }
                segments.push(TemplateSegment::Literal(attribute.value.clone()));
            }
        }
        let template = TemplateString::new("", segments, None);
        if let Some(expression) = template.single_expression() {
            write!(output, " class={{{}}}", svelte_expr(expression))
                .expect("writing to String cannot fail");
        } else {
            write!(
                output,
                " class={{`{}`}}",
                svelte_template_literal(&template)
            )
            .expect("writing to String cannot fail");
        }
    }

    fn write_template_attribute_value(
        &self,
        output: &mut String,
        template: &TemplateString,
        _scope: &Scope,
    ) {
        if let Some(expression) = template.single_expression() {
            write!(output, "={{{}}}", svelte_expr(expression))
                .expect("writing to String cannot fail");
        } else {
            write!(output, "={{`{}`}}", svelte_template_literal(template))
                .expect("writing to String cannot fail");
        }
    }

    fn write_state_binding(&self, output: &mut String, element: &RenderElement) {
        let Some(binding) = state_binding_attribute_name(element) else {
            return;
        };
        let state = element
            .state
            .as_deref()
            .expect("state binding attribute requires state");
        let name = sanitize_js_identifier(&state.id, "state");
        write!(output, " {binding}={{{name}}}").expect("writing to String cannot fail");
    }

    fn write_style_attribute(&self, output: &mut String, element: &RenderElement, _scope: &Scope) {
        let static_style = format_styles(svelte_inline_styles(element));
        let dynamic = element
            .dynamic_styles
            .iter()
            .map(|style| svelte_style_binding(&style.expression))
            .collect::<Vec<_>>();
        match (static_style.is_empty(), dynamic.is_empty()) {
            (true, true) => {}
            (false, true) => write!(
                output,
                " style=\"{}\"",
                escape_html_attribute(&static_style)
            )
            .expect("writing to String cannot fail"),
            (true, false) => write!(output, " style={{`{}`}}", dynamic.join("; "))
                .expect("writing to String cannot fail"),
            (false, false) => write!(
                output,
                " style={{`{}; {}`}}",
                escape_js_template(&static_style),
                dynamic.join("; ")
            )
            .expect("writing to String cannot fail"),
        }
    }

    fn write_event_attribute(
        &self,
        output: &mut String,
        element: &RenderElement,
        action: &ActionBinding,
    ) {
        let event = svelte_event_name(&action.event);
        let body = action_handler_body(element, action);
        write!(output, " {event}={{{body}}}").expect("writing to String cannot fail");
    }

    fn write_keyboard_activation_attribute(&self, output: &mut String, element: &RenderElement) {
        if !needs_keyboard_activation_fallback(element) {
            return;
        }
        let Some(action) = element
            .actions
            .iter()
            .find(|action| action.event == "click")
        else {
            return;
        };
        let body = action_handler_body(element, action);
        write!(
            output,
            " onkeydown={{(event) => {{ if (event.key === 'Enter' || event.key === ' ') {{ event.preventDefault(); const handler = {body}; handler?.(event); }} }}}}"
        )
        .expect("writing to String cannot fail");
    }

    fn write_text(&self, output: &mut String, text: &RenderText, depth: usize) {
        write!(output, "{}", indent(depth)).expect("writing to String cannot fail");
        if let Some(template) = &text.template {
            write!(output, "{}", svelte_template_markup(template))
                .expect("writing to String cannot fail");
        } else {
            write!(output, "{}", escape_svelte_text(&text.value))
                .expect("writing to String cannot fail");
        }
        writeln!(output).expect("writing to String cannot fail");
    }

    fn write_raw(&self, output: &mut String, raw: &RenderRaw, depth: usize) {
        writeln!(
            output,
            "{}{{@html {}}}",
            indent(depth),
            js_string_literal(&raw.html)
        )
        .expect("writing to String cannot fail");
    }
}

fn scope_for_control_flow(
    control_flow: Option<&RenderControlFlow>,
    scope: &Scope,
    prelude: &mut SveltePrelude,
) -> Scope {
    let Some(control_flow) = control_flow else {
        return scope.clone();
    };
    if control_flow.kind != RenderControlFlowKind::For {
        return scope.clone();
    }
    if let Some(binding) = &control_flow.binding {
        let index = control_flow
            .index_binding
            .as_ref()
            .map(|binding| sanitize_js_identifier(&binding.name, "index"))
            .unwrap_or_else(|| "$index".to_owned());
        if let Some(root) = control_flow.expression.as_ref().and_then(simple_expr_root)
            && !scope.is_local(&root)
            && !prelude.states.contains_key(&root)
            && !prelude
                .source_logic
                .as_ref()
                .is_some_and(|logic| jsx_source_logic_declares(logic, &root))
        {
            prelude.prop_with_default(root, "[]");
        }
        let mut scope = scope.with_local(index).with_local("$index");
        for local in binding_pattern_locals(&binding.name) {
            scope = scope.with_local(local);
        }
        return scope;
    }
    scope.clone()
}

fn svelte_binding_pattern(value: &str) -> String {
    let value = value.trim();
    if is_destructuring_binding_pattern(value) {
        value.to_owned()
    } else {
        sanitize_js_identifier(value, "item")
    }
}

fn binding_pattern_locals(value: &str) -> Vec<String> {
    let value = value.trim();
    if !is_destructuring_binding_pattern(value) {
        return vec![sanitize_js_identifier(value, "item")];
    }

    let mut locals = BTreeSet::new();
    let mut current = String::new();
    for ch in value.chars() {
        if current.is_empty() {
            if is_identifier_start(ch) {
                current.push(ch);
            }
        } else if is_identifier_continue(ch) {
            current.push(ch);
        } else {
            push_binding_local(&mut locals, &current);
            current.clear();
        }
    }
    push_binding_local(&mut locals, &current);
    locals.into_iter().collect()
}

fn push_binding_local(locals: &mut BTreeSet<String>, name: &str) {
    if !name.is_empty()
        && !JS_KEYWORDS.contains(&name)
        && !matches!(name, "undefined" | "null" | "true" | "false")
    {
        locals.insert(name.to_owned());
    }
}

fn is_destructuring_binding_pattern(value: &str) -> bool {
    value.starts_with('[') || value.starts_with('{')
}

fn element_emits_dom_node(element: &RenderElement) -> bool {
    !is_control_flow_wrapper(element)
        && !element_is_render_slot(element)
        && !should_emit_svelte_component(element)
}
fn element_needs_style_class(element: &RenderElement) -> bool {
    element
        .style_variants
        .iter()
        .any(|variant| uses_generated_style_class(&variant.selector))
        || element
            .pseudo_elements
            .iter()
            .any(|pseudo| uses_generated_style_class(&pseudo.selector))
}

fn tag_for_element(element: &RenderElement) -> String {
    if should_emit_svelte_component(element)
        && let Some(source_intent) = element.source_intent.as_deref()
        && let Some(component) = &source_intent.component
    {
        return component_name(component.as_str());
    }
    if !element.source_tag.is_empty()
        && !element.source_tag.starts_with("sc-")
        && element.source_tag != "x-dc"
    {
        return element.source_tag.to_string();
    }
    match element.role {
        UiRole::Container | UiRole::Unknown => "div",
        UiRole::Inline => "span",
        UiRole::Paragraph => "p",
        UiRole::Button => "button",
        UiRole::TextInput => "input",
        UiRole::Select => "select",
        UiRole::Option => "option",
        UiRole::Link => "a",
        UiRole::Image => "img",
        UiRole::Heading(level) => match level {
            1 => "h1",
            2 => "h2",
            3 => "h3",
            4 => "h4",
            5 => "h5",
            6 => "h6",
            _ => "h2",
        },
        UiRole::List { ordered: false } => "ul",
        UiRole::List { ordered: true } => "ol",
        UiRole::ListItem => "li",
        UiRole::Form => "form",
        UiRole::Fieldset => "fieldset",
        UiRole::Legend => "legend",
        UiRole::Label => "label",
    }
    .to_owned()
}

fn is_control_flow_wrapper(element: &RenderElement) -> bool {
    matches!(
        element.source_tag.as_str(),
        "sc-for" | "sc-if" | "sc-else" | "sc-else-if" | "x-dc" | "jsx-fragment"
    ) || matches!(
        element
            .control_flow
            .as_deref()
            .map(|control_flow| &control_flow.kind),
        Some(
            RenderControlFlowKind::For
                | RenderControlFlowKind::If
                | RenderControlFlowKind::ElseIf
                | RenderControlFlowKind::Else
        )
    ) && element.role == UiRole::Unknown
        && element.attributes.is_empty()
        && element.classes.is_empty()
        && element.styles.is_empty()
}

fn element_is_render_slot(element: &RenderElement) -> bool {
    element.source_tag == "slot"
}

fn slot_prop_name(element: &RenderElement) -> String {
    element
        .source_intent
        .as_deref()
        .and_then(|intent| intent.slot.as_ref())
        .map(|slot| slot.as_str().to_lower_camel_case())
        .unwrap_or_else(|| "children".to_owned())
}

fn should_emit_svelte_component(element: &RenderElement) -> bool {
    let Some(source_intent) = element.source_intent.as_deref() else {
        return false;
    };
    source_intent.component.is_some()
        && (matches!(element.source_tag.as_str(), "dc-import" | "x-import")
            || element.source_tag == "jsx-component"
            || source_intent.component_source.is_some())
}

fn has_target_state(element: &RenderElement) -> bool {
    element
        .state
        .as_deref()
        .is_some_and(|state| state.owner == RenderStateOwner::Target)
}

fn state_binding_attribute_name(element: &RenderElement) -> Option<&'static str> {
    let state = element.state.as_deref()?;
    if state.owner != RenderStateOwner::Target {
        return None;
    }
    match (
        &state.kind,
        element
            .form_control
            .as_deref()
            .map(|control| &control.control_type),
    ) {
        (RenderStateKind::Toggle(_), _)
        | (_, Some(RenderFormControlType::Checkbox | RenderFormControlType::Radio)) => {
            Some("bind:checked")
        }
        _ => Some("bind:value"),
    }
}

fn element_has_static_or_dynamic_style(element: &RenderElement) -> bool {
    element
        .styles
        .iter()
        .any(|style| !is_stylesheet_declaration(element, style))
        || !element.dynamic_styles.is_empty()
}

fn jsx_component_placeholder_name(element: &RenderElement) -> Option<&str> {
    if element.source_tag == "jsx-component" {
        if element
            .source_intent
            .as_deref()
            .and_then(|intent| intent.component_source.as_ref())
            .is_some()
        {
            return None;
        }
        return element
            .source_intent
            .as_deref()
            .and_then(|intent| intent.component.as_ref())
            .map(|component| component.as_str());
    }
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name == "data-htmlswap-jsx-component-placeholder")
        .map(|attribute| attribute.value.as_str())
}

fn inline_style_block(element: &RenderElement) -> Option<String> {
    if element.source_tag != "style" {
        return None;
    }

    let mut block = String::new();
    for child in &element.children {
        match child {
            RenderNode::Text(text) => block.push_str(&text.value),
            RenderNode::Raw(raw) => block.push_str(&raw.html),
            RenderNode::Element(_) => return None,
        }
    }
    let block = block.trim();
    (!block.is_empty()).then(|| block.to_owned())
}

fn svelte_inline_styles(element: &RenderElement) -> Vec<&StyleDeclaration> {
    if !element.source_inline_styles.is_empty() {
        return element.source_inline_styles.iter().collect();
    }
    element
        .styles
        .iter()
        .filter(|style| !is_stylesheet_declaration(element, style))
        .collect()
}

fn is_stylesheet_declaration(element: &RenderElement, declaration: &StyleDeclaration) -> bool {
    element
        .stylesheet_declarations
        .iter()
        .any(|stylesheet| style_declarations_match(stylesheet, declaration))
}

fn style_declarations_match(left: &StyleDeclaration, right: &StyleDeclaration) -> bool {
    left.property == right.property
        && left.value == right.value
        && left.important == right.important
}

fn attribute_key(name: &str) -> String {
    if name.eq_ignore_ascii_case("role") || name.to_ascii_lowercase().starts_with("aria-") {
        name.to_ascii_lowercase()
    } else {
        name.to_owned()
    }
}

fn should_skip_svelte_attribute(name: &str) -> bool {
    matches!(name, "class" | "style" | "data-htmlswap-attach")
        || name.starts_with("data-htmlswap-prop-")
        || matches!(
            name,
            "data-htmlswap-component"
                | "data-htmlswap-component-source"
                | "data-htmlswap-children"
                | "data-htmlswap-slot"
                | "data-htmlswap-state"
                | "data-htmlswap-state-owner"
                | "data-htmlswap-render"
        )
}

fn is_void_html_tag(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "source"
            | "track"
            | "wbr"
    )
}

fn needs_keyboard_activation_fallback(element: &RenderElement) -> bool {
    let tag = tag_for_element(element);
    element_has_action(element, "click")
        && !element_has_action(element, "keydown")
        && !is_native_keyboard_click_tag(&tag)
}

fn is_native_keyboard_click_tag(tag: &str) -> bool {
    matches!(
        tag,
        "button" | "a" | "input" | "select" | "textarea" | "option"
    )
}

fn element_has_action(element: &RenderElement, event: &str) -> bool {
    element.actions.iter().any(|action| action.event == event)
}

fn element_has_role(element: &RenderElement) -> bool {
    element
        .accessibility
        .as_deref()
        .and_then(|accessibility| accessibility.role.as_ref())
        .is_some()
        || element_has_attribute(element, "role")
}

fn element_has_attribute(element: &RenderElement, name: &str) -> bool {
    element
        .attributes
        .iter()
        .any(|attribute| attribute.name.eq_ignore_ascii_case(name))
}

fn attribute_value<'a>(element: &'a RenderElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.eq_ignore_ascii_case(name))
        .map(|attribute| attribute.value.as_str())
}

fn svelte_event_name(event: &str) -> String {
    let event = match event {
        "doubleclick" => "dblclick",
        value => value,
    };
    format!("on{}", event.replace('-', ""))
}

fn action_handler_body(element: &RenderElement, action: &ActionBinding) -> String {
    let mut statements = Vec::new();
    for effect in &action.handler.effects {
        match effect {
            RenderActionHandlerEffect::PreventDefault { .. } => {
                statements.push("event.preventDefault();".to_owned())
            }
            RenderActionHandlerEffect::StopPropagation { .. } => {
                statements.push("event.stopPropagation();".to_owned())
            }
        }
    }
    match &action.payload {
        ActionPayload::None => {}
        ActionPayload::ElementState { state_id } => {
            if should_mutate_local_state_from_action(element, state_id) {
                statements.push(format!(
                    "{} = event.currentTarget?.value ?? '';",
                    sanitize_js_identifier(state_id, "state")
                ));
            }
        }
        ActionPayload::FormData { .. } => {}
    }
    let has_invocations = !action.handler.invocations.is_empty();
    for invocation in &action.handler.invocations {
        let name = sanitize_js_identifier(&invocation.action, "handler");
        let args = invocation
            .arguments
            .iter()
            .map(action_argument)
            .collect::<Vec<_>>()
            .join(", ");
        statements.push(format!("{name}?.({args});"));
    }
    if !has_invocations && let Some(expression) = normalized_action_expression(action) {
        if statements.is_empty() {
            if is_simple_handler_reference(&expression) {
                return expression;
            }
            if expression.contains("=>") {
                return expression;
            }
        }
        push_normalized_action_statement(&mut statements, &expression);
    }
    if statements.is_empty() {
        return "() => {}".to_owned();
    }
    format!("(event) => {{ {} }}", statements.join(" "))
}

fn should_mutate_local_state_from_action(element: &RenderElement, state_id: &str) -> bool {
    element.state.as_deref().is_some_and(|state| {
        state.owner == RenderStateOwner::Target && state.id.as_str() == state_id
    })
}

fn push_normalized_action_statement(statements: &mut Vec<String>, expression: &str) {
    if is_simple_handler_reference(expression) {
        statements.push(format!("{expression}?.(event);"));
    } else if expression.contains("=>") {
        statements.push(format!("({expression})(event);"));
    } else {
        statements.push(format!("{expression};"));
    }
}
fn normalized_action_expression(action: &ActionBinding) -> Option<String> {
    if let Some(template) = &action.template
        && let Some(expression) = template.single_expression()
    {
        return Some(svelte_expr(expression));
    }
    let expression = action.expression.trim();
    if expression.is_empty() {
        return None;
    }
    if let Some(inner) = expression
        .strip_prefix("{{")
        .and_then(|value| value.strip_suffix("}}"))
    {
        let inner = inner.trim();
        if inner.is_empty() {
            None
        } else {
            Some(inner.to_owned())
        }
    } else {
        Some(expression.to_owned())
    }
}

fn is_simple_handler_reference(value: &str) -> bool {
    value
        .split('.')
        .all(|segment| is_simple_identifier(segment.trim()))
}
fn action_argument(argument: &RenderActionArgument) -> String {
    match argument {
        RenderActionArgument::Event => "event".to_owned(),
        RenderActionArgument::Element => "event.currentTarget".to_owned(),
        RenderActionArgument::ElementValue => "event.currentTarget?.value".to_owned(),
        RenderActionArgument::ElementChecked => "event.currentTarget?.checked".to_owned(),
        RenderActionArgument::Literal(value) => js_string_literal(value),
        RenderActionArgument::Unknown(value) => value.to_string(),
    }
}
fn svelte_state_initial_value(state: &RenderStateBinding) -> String {
    match &state.kind {
        RenderStateKind::TextInput(input) => input
            .initial_template
            .as_ref()
            .and_then(|template| template.single_expression().map(svelte_expr))
            .or_else(|| {
                input
                    .initial_value
                    .as_ref()
                    .map(|value| js_string_literal(value))
            })
            .unwrap_or_else(|| "''".to_owned()),
        RenderStateKind::Choice(choice) => {
            if choice.multiple {
                "[]".to_owned()
            } else {
                choice
                    .selected_index
                    .and_then(|index| choice.options.get(index))
                    .map(|option| js_string_literal(&option.value))
                    .unwrap_or_else(|| "''".to_owned())
            }
        }
        RenderStateKind::Toggle(toggle) => toggle.checked.to_string(),
    }
}

fn svelte_template_markup(template: &TemplateString) -> String {
    let mut output = String::new();
    for segment in &template.segments {
        match segment {
            TemplateSegment::Literal(value) => output.push_str(&escape_svelte_text(value)),
            TemplateSegment::Expression(expression) => {
                write!(output, "{{{}}}", svelte_expr(expression))
                    .expect("writing to String cannot fail");
            }
        }
    }
    output
}

fn svelte_template_literal(template: &TemplateString) -> String {
    let mut output = String::new();
    for segment in &template.segments {
        match segment {
            TemplateSegment::Literal(value) => output.push_str(&escape_js_template(value)),
            TemplateSegment::Expression(expression) => {
                write!(output, "${{{}}}", svelte_expr(expression))
                    .expect("writing to String cannot fail");
            }
        }
    }
    output
}

fn svelte_style_binding(template: &TemplateString) -> String {
    if let Some(expression) = template.single_expression() {
        format!("${{__htmlswap_style({})}}", svelte_expr(expression))
    } else {
        svelte_template_literal(template)
    }
}

fn svelte_expr(expression: &Expr) -> String {
    match expression {
        Expr::Path(segments) => segments
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("."),
        Expr::Member { object, property } => format!("{}.{}", svelte_expr(object), property),
        Expr::Index { object, index } => format!("{}[{}]", svelte_expr(object), svelte_expr(index)),
        Expr::Call { callee, arguments } => format!(
            "{}({})",
            svelte_expr(callee),
            arguments
                .iter()
                .map(svelte_expr)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expr::Binary {
            left,
            operator,
            right,
        }
        | Expr::Logical {
            left,
            operator,
            right,
        } => format!("{} {} {}", svelte_expr(left), operator, svelte_expr(right)),
        Expr::Conditional {
            test,
            consequent,
            alternate,
        } => format!(
            "{} ? {} : {}",
            svelte_expr(test),
            svelte_expr(consequent),
            svelte_expr(alternate)
        ),
        Expr::TemplateLiteral { segments } => {
            let mut output = String::from("`");
            for segment in segments {
                match segment {
                    TemplateSegment::Literal(value) => output.push_str(value),
                    TemplateSegment::Expression(expression) => {
                        write!(output, "${{{}}}", svelte_expr(expression))
                            .expect("writing to String cannot fail");
                    }
                }
            }
            output.push('`');
            output
        }
        Expr::Array(items) => format!(
            "[{}]",
            items.iter().map(svelte_expr).collect::<Vec<_>>().join(", ")
        ),
        Expr::Object(entries) => format!(
            "{{ {} }}",
            entries
                .iter()
                .map(|entry| format!("{}: {}", entry.key, svelte_expr(&entry.value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expr::Literal(literal) => match literal {
            ExprLiteral::String(value) => js_string_literal(value),
            ExprLiteral::Bool(value) => value.to_string(),
            ExprLiteral::Number(value) => value.to_string(),
            ExprLiteral::Null => "null".to_owned(),
        },
        Expr::Opaque(value) => value.to_string(),
    }
}

fn simple_expr_root(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Path(segments) => segments.first().map(ToString::to_string),
        Expr::Member { object, .. } | Expr::Index { object, .. } => simple_expr_root(object),
        _ => None,
    }
}

fn expression_roots(expression: &Expr) -> BTreeSet<String> {
    let mut roots = BTreeSet::new();
    collect_expression_roots(expression, &mut roots);
    roots
}

fn collect_expression_roots(expression: &Expr, roots: &mut BTreeSet<String>) {
    match expression {
        Expr::Path(segments) => {
            if let Some(root) = segments.first() {
                roots.insert(root.to_string());
            }
        }
        Expr::Member { object, .. } => collect_expression_roots(object, roots),
        Expr::Index { object, index } => {
            collect_expression_roots(object, roots);
            collect_expression_roots(index, roots);
        }
        Expr::Call { callee, arguments } => {
            collect_expression_roots(callee, roots);
            for argument in arguments {
                collect_expression_roots(argument, roots);
            }
        }
        Expr::Binary { left, right, .. } | Expr::Logical { left, right, .. } => {
            collect_expression_roots(left, roots);
            collect_expression_roots(right, roots);
        }
        Expr::Conditional {
            test,
            consequent,
            alternate,
        } => {
            collect_expression_roots(test, roots);
            collect_expression_roots(consequent, roots);
            collect_expression_roots(alternate, roots);
        }
        Expr::TemplateLiteral { segments } => {
            for expression in segments.iter().filter_map(|segment| match segment {
                TemplateSegment::Literal(_) => None,
                TemplateSegment::Expression(expression) => Some(expression),
            }) {
                collect_expression_roots(expression, roots);
            }
        }
        Expr::Array(items) => {
            for item in items {
                collect_expression_roots(item, roots);
            }
        }
        Expr::Object(entries) => {
            for entry in entries {
                collect_expression_roots(&entry.value, roots);
            }
        }
        Expr::Literal(_) | Expr::Opaque(_) => {}
    }
}

fn loose_expression_roots(expression: &str) -> BTreeSet<String> {
    expression
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'))
        .filter(|part| !part.is_empty())
        .filter(|part| !should_skip_prop_name(part))
        .map(ToOwned::to_owned)
        .collect()
}
fn action_expression_roots(action: &ActionBinding) -> BTreeSet<String> {
    if let Some(template) = &action.template {
        return template.expressions().flat_map(expression_roots).collect();
    }
    let Some(expression) = normalized_action_expression(action) else {
        return BTreeSet::new();
    };
    loose_expression_roots(&expression)
}
fn format_styles<'a>(styles: impl IntoIterator<Item = &'a StyleDeclaration>) -> String {
    styles
        .into_iter()
        .map(|style| {
            let important = if style.important { " !important" } else { "" };
            format!(
                "{}: {}{important}",
                style.property.as_str(),
                style.value.as_str()
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn browser_only_style_description(declaration: &StyleDeclaration) -> Option<String> {
    let value = declaration.value.as_str().trim();
    let value_lower = value.to_ascii_lowercase();
    let violates_contract = match declaration.property {
        StyleProperty::Position => matches!(value_lower.as_str(), "fixed" | "absolute" | "sticky"),
        StyleProperty::ZIndex => true,
        StyleProperty::Display => value_lower == "inline-block",
        _ => declaration.property.as_str().eq_ignore_ascii_case("float"),
    };
    violates_contract.then(|| format!("{}: {value}", declaration.property.as_str()))
}

fn uses_generated_style_class(selector: &str) -> bool {
    let selector = selector.trim();
    selector.is_empty() || selector.starts_with("[style-")
}

fn stylesheet_variant_rule(variant: &RenderStyleVariant) -> Option<String> {
    if uses_generated_style_class(&variant.selector) {
        return None;
    }
    conditioned_style_rule(
        variant.selector.to_string(),
        &variant.conditions,
        &variant.declarations,
    )
}

fn stylesheet_pseudo_element_rule(pseudo: &RenderPseudoElement) -> Option<String> {
    if uses_generated_style_class(&pseudo.selector) {
        return None;
    }
    let declarations = pseudo_element_declarations(pseudo);
    conditioned_style_rule(
        pseudo.selector.to_string(),
        &pseudo.conditions,
        &declarations,
    )
}

fn style_variant_rule(base_selector: &str, variant: &RenderStyleVariant) -> Option<String> {
    let selector = if variant.selector.is_empty() || variant.selector.starts_with("[style-") {
        base_selector.to_owned()
    } else {
        format!("{base_selector}{}", selector_suffix(&variant.selector))
    };
    conditioned_style_rule(selector, &variant.conditions, &variant.declarations)
}

fn pseudo_element_rule(base_selector: &str, pseudo: &RenderPseudoElement) -> Option<String> {
    let selector = if pseudo.selector.is_empty() {
        format!("{base_selector}::{}", pseudo.kind)
    } else {
        format!("{base_selector}{}", selector_suffix(&pseudo.selector))
    };
    let declarations = pseudo_element_declarations(pseudo);
    conditioned_style_rule(selector, &pseudo.conditions, &declarations)
}

fn pseudo_element_declarations(pseudo: &RenderPseudoElement) -> Vec<StyleDeclaration> {
    let mut declarations = pseudo.styles.clone();
    if !pseudo.children.is_empty()
        && !declarations
            .iter()
            .any(|style| style.property == StyleProperty::Content)
    {
        declarations.push(StyleDeclaration::new("content", "\"\"", false, pseudo.span));
    }
    declarations
}

fn selector_suffix(selector: &str) -> String {
    let selector = selector.trim();
    if selector.starts_with(':')
        || selector.starts_with('[')
        || selector.starts_with('.')
        || selector.starts_with('#')
    {
        selector.to_owned()
    } else {
        format!(" {selector}")
    }
}

fn conditioned_style_rule(
    selector: String,
    conditions: &[RenderStyleCondition],
    declarations: &[StyleDeclaration],
) -> Option<String> {
    if declarations.is_empty() {
        return None;
    }
    let mut selector = selector;
    let mut wrappers = Vec::new();
    for condition in conditions {
        match condition {
            RenderStyleCondition::PseudoClass(value) => {
                push_trailing_pseudo_if_missing(&mut selector, ":", value);
            }
            RenderStyleCondition::PseudoElement(value) => {
                push_trailing_pseudo_if_missing(&mut selector, "::", value);
            }
            RenderStyleCondition::Media(value) => wrappers.push(format!("@media {value}")),
            RenderStyleCondition::Supports(value) => wrappers.push(format!("@supports {value}")),
            RenderStyleCondition::Container(value) => wrappers.push(format!("@container {value}")),
            RenderStyleCondition::StartingStyle => wrappers.push("@starting-style".to_owned()),
            RenderStyleCondition::ActiveViewTransitionType(types) => {
                selector = format!(
                    ":global(:root:active-view-transition-type({})) {}",
                    types.join(", "),
                    selector.trim_start()
                );
            }
            RenderStyleCondition::ElementState {
                pseudo,
                ancestor: 0,
                negated: true,
            } => selector.push_str(&format!(":not(:{pseudo})")),
            // A state on an ancestor needs that ancestor's selector, which a
            // per-element rule does not carry, so the rule is not emitted
            // rather than styling the element on its own state.
            RenderStyleCondition::ElementState { .. } => return None,
        }
    }
    let mut rule = String::new();
    writeln!(rule, "{selector} {{").expect("writing to String cannot fail");
    for declaration in declarations {
        write_style_declaration(&mut rule, declaration, 1);
    }
    write!(rule, "}}").expect("writing to String cannot fail");
    for wrapper in wrappers.into_iter().rev() {
        rule = format!("{wrapper} {{\n{}\n}}", indent_block(&rule, 1));
    }
    Some(rule)
}

fn push_trailing_pseudo_if_missing(selector: &mut String, prefix: &str, value: &str) {
    let pseudo = format!("{prefix}{value}");
    if !selector.trim_end().ends_with(&pseudo) {
        selector.push_str(&pseudo);
    }
}

fn write_style_declaration(output: &mut String, declaration: &StyleDeclaration, depth: usize) {
    let important = if declaration.important {
        " !important"
    } else {
        ""
    };
    writeln!(
        output,
        "{}{}: {}{important};",
        indent(depth),
        declaration.property.as_str(),
        declaration.value.as_str()
    )
    .expect("writing to String cannot fail");
}

fn write_indented_block(output: &mut String, block: &str, depth: usize) {
    for line in block.lines() {
        writeln!(output, "{}{}", indent(depth), line).expect("writing to String cannot fail");
    }
}

fn write_indented_style_block(output: &mut String, block: &str, depth: usize) {
    write_indented_block(output, block.trim(), depth);
}

fn indent_block(block: &str, depth: usize) -> String {
    block
        .lines()
        .map(|line| format!("{}{line}", indent(depth)))
        .collect::<Vec<_>>()
        .join("\n")
}
fn component_name(value: &str) -> String {
    let name = svelte_component_identifier(value);
    if name.is_empty() {
        "HtmlswapComponent".to_owned()
    } else {
        name
    }
}

fn jsx_props_entries_contain(entries: &[String], name: &str) -> bool {
    entries.iter().any(|entry| {
        entry
            .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'))
            .any(|part| part == name)
    })
}

fn jsx_source_logic_declares(logic: &RenderSourceLogic, name: &str) -> bool {
    if !logic.dialect.eq_ignore_ascii_case("jsx") {
        return false;
    }
    if logic
        .data_props
        .as_ref()
        .is_some_and(|props| jsx_props_entries_contain(&[props.to_string()], name))
    {
        return true;
    }
    let body = logic.body.as_str();
    jsx_source_logic_declared_names(body).contains(name)
        || jsx_source_logic_component_declares(logic, name)
        || body.contains(&format!(", {name} ="))
        || body.contains(&format!(" {name} ="))
}

fn jsx_source_logic_component_declares(logic: &RenderSourceLogic, name: &str) -> bool {
    let Some(component) = &logic.component else {
        return false;
    };
    component.items.iter().any(|item| match item {
        RenderSourceLogicItem::State(item) => item.name == name,
        RenderSourceLogicItem::Derived(item) => item.name == name,
        RenderSourceLogicItem::Ref(item) => item.name == name,
        RenderSourceLogicItem::Callback(item) => item.name == name,
        RenderSourceLogicItem::Snippet(item) => item.name == name,
        RenderSourceLogicItem::Local(local) => {
            jsx_source_logic_declared_names(local.body.as_str()).contains(name)
        }
        RenderSourceLogicItem::Mount(_) | RenderSourceLogicItem::Effect(_) => false,
    })
}

fn jsx_source_logic_declared_names(body: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    collect_jsx_named_declaration_names(body, "function", &mut names);
    collect_jsx_named_declaration_names(body, "class", &mut names);
    collect_jsx_variable_declaration_names(body, "const", &mut names);
    collect_jsx_variable_declaration_names(body, "let", &mut names);
    collect_jsx_variable_declaration_names(body, "var", &mut names);
    names
}

fn collect_jsx_named_declaration_names(body: &str, keyword: &str, names: &mut BTreeSet<String>) {
    let mut offset = 0;
    while let Some(index) = find_js_keyword(body, keyword, offset) {
        let cursor = skip_js_whitespace(body, index + keyword.len());
        if let Some((name, end)) = scan_js_identifier_at(body, cursor) {
            names.insert(name.to_owned());
            offset = end;
        } else {
            offset = index + keyword.len();
        }
    }
}

fn collect_jsx_variable_declaration_names(body: &str, keyword: &str, names: &mut BTreeSet<String>) {
    let mut offset = 0;
    while let Some(index) = find_js_keyword(body, keyword, offset) {
        let mut cursor = skip_js_whitespace(body, index + keyword.len());
        loop {
            cursor = skip_js_whitespace(body, cursor);
            let Some(ch) = char_at(body, cursor) else {
                break;
            };
            if matches!(ch, '{' | '[') {
                let Some(end) = scan_js_balanced(body, cursor) else {
                    break;
                };
                for local in binding_pattern_locals(&body[cursor..end]) {
                    names.insert(local);
                }
                cursor = end;
            } else if let Some((name, end)) = scan_js_identifier_at(body, cursor) {
                names.insert(name.to_owned());
                cursor = end;
            } else {
                break;
            }

            let (next, separator) = scan_js_declarator_separator(body, cursor);
            cursor = next;
            if separator == Some(',') {
                cursor += 1;
            } else {
                break;
            }
        }
        offset = cursor.max(index + keyword.len());
    }
}

fn find_js_keyword(body: &str, keyword: &str, mut offset: usize) -> Option<usize> {
    while let Some(relative) = body[offset..].find(keyword) {
        let index = offset + relative;
        let end = index + keyword.len();
        let before_ok = body[..index]
            .chars()
            .next_back()
            .is_none_or(|ch| !is_identifier_continue(ch));
        let after_ok = body[end..]
            .chars()
            .next()
            .is_none_or(|ch| !is_identifier_continue(ch));
        if before_ok && after_ok {
            return Some(index);
        }
        offset = end;
    }
    None
}

fn scan_js_identifier_at(body: &str, start: usize) -> Option<(&str, usize)> {
    let mut chars = body[start..].char_indices();
    let (_, first) = chars.next()?;
    if !is_identifier_start(first) {
        return None;
    }
    let mut end = start + first.len_utf8();
    for (relative, ch) in chars {
        if !is_identifier_continue(ch) {
            break;
        }
        end = start + relative + ch.len_utf8();
    }
    Some((&body[start..end], end))
}

fn scan_js_balanced(body: &str, start: usize) -> Option<usize> {
    let mut stack = Vec::new();
    let mut cursor = start;
    let mut quote = None;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = false;

    while let Some(ch) = char_at(body, cursor) {
        let next = cursor + ch.len_utf8();
        if line_comment {
            line_comment = ch != '\n';
            cursor = next;
            continue;
        }
        if block_comment {
            if ch == '*' && char_at(body, next) == Some('/') {
                block_comment = false;
                cursor = next + 1;
            } else {
                cursor = next;
            }
            continue;
        }
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            cursor = next;
            continue;
        }
        match ch {
            '/' if char_at(body, next) == Some('/') => {
                line_comment = true;
                cursor = next + 1;
            }
            '/' if char_at(body, next) == Some('*') => {
                block_comment = true;
                cursor = next + 1;
            }
            '\'' | '"' | '`' => {
                quote = Some(ch);
                cursor = next;
            }
            '(' => {
                stack.push(')');
                cursor = next;
            }
            '[' => {
                stack.push(']');
                cursor = next;
            }
            '{' => {
                stack.push('}');
                cursor = next;
            }
            ')' | ']' | '}' if stack.last() == Some(&ch) => {
                stack.pop();
                cursor = next;
                if stack.is_empty() {
                    return Some(cursor);
                }
            }
            _ => cursor = next,
        }
    }
    None
}

fn scan_js_declarator_separator(body: &str, start: usize) -> (usize, Option<char>) {
    let mut cursor = start;
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = false;

    while let Some(ch) = char_at(body, cursor) {
        let next = cursor + ch.len_utf8();
        if line_comment {
            line_comment = ch != '\n';
            cursor = next;
            continue;
        }
        if block_comment {
            if ch == '*' && char_at(body, next) == Some('/') {
                block_comment = false;
                cursor = next + 1;
            } else {
                cursor = next;
            }
            continue;
        }
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            cursor = next;
            continue;
        }
        match ch {
            '/' if char_at(body, next) == Some('/') => {
                line_comment = true;
                cursor = next + 1;
            }
            '/' if char_at(body, next) == Some('*') => {
                block_comment = true;
                cursor = next + 1;
            }
            '\'' | '"' | '`' => {
                quote = Some(ch);
                cursor = next;
            }
            '(' | '[' | '{' => {
                depth += 1;
                cursor = next;
            }
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                cursor = next;
            }
            ',' | ';' if depth == 0 => return (cursor, Some(ch)),
            _ => cursor = next,
        }
    }
    (cursor, None)
}

fn skip_js_whitespace(body: &str, mut cursor: usize) -> usize {
    while let Some(ch) = char_at(body, cursor) {
        if !ch.is_whitespace() {
            break;
        }
        cursor += ch.len_utf8();
    }
    cursor
}

fn char_at(body: &str, cursor: usize) -> Option<char> {
    body.get(cursor..)?.chars().next()
}

fn source_logic_uses_mount(logic: &RenderSourceLogic) -> bool {
    if logic.dialect.eq_ignore_ascii_case("jsx") {
        return logic.component.as_ref().is_some_and(|component| {
            component
                .items
                .iter()
                .any(|item| matches!(item, RenderSourceLogicItem::Mount(_)))
        });
    }
    if !logic.dialect.eq_ignore_ascii_case("dc") {
        return false;
    }
    let body = logic.body.as_str();
    body.contains("componentDidMount") || body.contains("componentWillUnmount")
}

fn source_logic_uses_did_update(logic: &RenderSourceLogic) -> bool {
    if !logic.dialect.eq_ignore_ascii_case("dc") {
        return false;
    }
    logic.body.as_str().contains("componentDidUpdate")
}

fn svelte_component_identifier(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        let mut output = String::with_capacity(value.len());
        for (index, ch) in value.chars().enumerate() {
            if index == 0 {
                if ch.is_ascii_digit() {
                    output.push('_');
                    output.push(ch);
                } else {
                    output.push(ch.to_ascii_uppercase());
                }
            } else {
                output.push(ch);
            }
        }
        return output;
    }

    value.to_upper_camel_case()
}

fn component_source_is_external(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://") || source.starts_with("//")
}

fn svelte_component_import_specifier(source: &str) -> String {
    let mut source = source.replace('\\', "/");
    if source.ends_with(".dc.html") {
        source.truncate(source.len() - ".dc.html".len());
        source.push_str(".svelte");
    }
    if !source.starts_with('.') && !source.starts_with('/') {
        source = format!("./{source}");
    }
    js_string_literal(&source)
}

fn svelte_component_file_stem(name: &str) -> String {
    let stem = name.to_upper_camel_case();
    if stem.is_empty() {
        "HtmlswapView".to_owned()
    } else {
        stem
    }
}

fn svelte_script_output_path(script: &RenderScriptReference, source: &SourceFile) -> String {
    let name = source.name().unwrap_or(script.src.as_str());
    let file_name = name
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("htmlswap-script.ts");
    format!("scripts/{file_name}")
}

fn svelte_script_import_specifier(script: &RenderScriptReference) -> String {
    format!("./{}", svelte_script_output_path_for_src(&script.src))
}

fn svelte_script_output_path_for_src(src: &str) -> String {
    let file_name = src
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("htmlswap-script.ts");
    format!("scripts/{file_name}")
}

fn module_alias_for_script(script: &RenderScriptReference) -> String {
    let base = script
        .src
        .rsplit(['/', '\\', '.'])
        .find(|part| !part.is_empty())
        .unwrap_or("script");
    sanitize_js_identifier(&format!("htmlswap_{base}"), "htmlswapScript")
}

fn head_script_tag(script: &RenderScriptReference) -> String {
    let kind = if script.kind == RenderScriptKind::Module {
        " type=\"module\""
    } else {
        ""
    };
    let async_attr = if script.async_script { " async" } else { "" };
    let defer_attr = if script.defer { " defer" } else { "" };
    format!(
        "<script src=\"{}\"{kind}{async_attr}{defer_attr}></script>",
        escape_html_attribute(&script.src)
    )
}

fn source_comment(sources: &SourceMap, span: Option<Span>) -> Option<String> {
    let span = span?;
    let source = sources.source_text(span)?.trim();
    if source.is_empty() {
        return None;
    }
    let first_line = source.lines().next().unwrap_or(source).trim();
    Some(sanitize_html_comment(first_line))
}

fn sanitize_html_comment(value: &str) -> String {
    value.replace("--", "- -").replace('>', "&gt;")
}

fn escape_html_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_svelte_text(value: &str) -> String {
    escape_html_text(value)
        .replace('{', "&#123;")
        .replace('}', "&#125;")
}

fn escape_html_attribute(value: &str) -> String {
    escape_html_text(value).replace('"', "&quot;")
}

fn escape_js_template(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace("${", "\\${")
}

fn js_string_literal(value: &str) -> String {
    format!("{value:?}")
}

fn sanitize_js_identifier(value: &str, fallback: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for (index, ch) in value.chars().enumerate() {
        if (index == 0 && is_identifier_start(ch)) || (index > 0 && is_identifier_continue(ch)) {
            output.push(ch);
        } else if matches!(ch, '-' | '_' | ' ' | '.') {
            output.push('_');
        }
    }
    let output = output.trim_matches('_');
    if output.is_empty() || JS_KEYWORDS.contains(&output) {
        fallback.to_owned()
    } else {
        output.to_owned()
    }
}

fn is_simple_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    is_identifier_start(first) && chars.all(is_identifier_continue)
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_ascii_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    is_identifier_start(ch) || ch.is_ascii_digit()
}

fn should_skip_prop_name(name: &str) -> bool {
    name.is_empty()
        || name.starts_with('$')
        || matches!(
            name,
            "event" | "undefined" | "null" | "true" | "false" | "this"
        )
        || JS_GLOBALS.contains(&name)
        || JS_KEYWORDS.contains(&name)
}

fn indent(depth: usize) -> String {
    "\t".repeat(depth)
}

const JS_KEYWORDS: &[&str] = &[
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "return",
    "super",
    "switch",
    "throw",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

const JS_GLOBALS: &[&str] = &[
    "Array", "Boolean", "Date", "Error", "JSON", "Math", "Number", "Object", "Promise", "String",
    "console", "document", "window",
];
