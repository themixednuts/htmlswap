use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use compact_str::CompactString;
use heck::{ToKebabCase, ToLowerCamelCase};
use syn::parse::Parser;
use syn::{Expr, ExprCall, ExprMethodCall, File, Item, Lit, Member, Stmt};

use crate::adapter::{AdapterContext, AdapterError, HtmlArtifact, TargetArtifact};
use crate::adapters::gpui_semantics::{component_size_value, semantic_attribute};
use crate::css::parse_style_attribute;
use crate::diagnostics::Diagnostic;
use crate::emit::{EmitContext, Emitter, HtmlEmitter};
use crate::expr::TemplateString;
use crate::plan::{
    ActionBinding, ActionPayload, RenderActionArgument, RenderActionHandler,
    RenderActionInvocation, RenderAttribute, RenderDynamicStyleBinding, RenderElement,
    RenderHeadElement, RenderNode, RenderPlan, RenderPseudoElement, RenderRaw, RenderStatePlan,
    RenderStyleCondition, RenderStyleVariant, RenderText, UiRole,
};
use crate::style::{StyleDeclaration, StyleProperty};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GpuiImportMode {
    Gpui,
    GpuiComponents,
}

const INTERNAL_SOURCE_ORDER_ATTRIBUTE: &str = "data-htmlswap-source-order";

pub(crate) fn import_gpui_target(
    source: &TargetArtifact,
    cx: &mut AdapterContext,
    mode: GpuiImportMode,
) -> Result<HtmlArtifact, AdapterError> {
    let plan = GpuiReader::new(mode).read(source, cx)?;
    let mut emit_cx = EmitContext::new();
    HtmlEmitter
        .emit(&plan, &mut emit_cx)
        .map_err(|error| AdapterError::message(error.to_string()))
}

struct GpuiReader {
    mode: GpuiImportMode,
    text_inputs: BTreeMap<CompactString, TextInputMetadata>,
    source_hints: RefCell<VecDeque<SourceHint>>,
    control_flow_hints: RefCell<VecDeque<ControlFlowHint>>,
}

#[derive(Debug, Clone, Default)]
struct TextInputMetadata {
    value: Option<CompactString>,
    placeholder: Option<CompactString>,
    password: bool,
    multiline: bool,
    rows: Option<u16>,
    action: Option<TextInputAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TextInputAction {
    event: CompactString,
    handler: CompactString,
}

#[derive(Debug, Clone, Default)]
struct SourceHint {
    source_order: Option<usize>,
    source_tag: Option<CompactString>,
    attributes: Vec<(CompactString, CompactString)>,
    classes: Vec<CompactString>,
    styles: Vec<StyleDeclaration>,
    style_variants: Vec<RenderStyleVariant>,
    pseudo_elements: Vec<RenderPseudoElement>,
    dynamic_styles: Vec<(Option<CompactString>, CompactString)>,
    actions: Vec<ActionBinding>,
    raw_html: Option<CompactString>,
    raw_child_html: Vec<CompactString>,
}

#[derive(Debug, Clone, Default)]
struct ControlFlowHint {
    kind: CompactString,
    source_order: Option<usize>,
    expression: Option<CompactString>,
    binding: Option<CompactString>,
    placeholder: Option<CompactString>,
}

impl SourceHint {
    fn is_empty(&self) -> bool {
        self.source_order.is_none()
            && self.source_tag.is_none()
            && self.attributes.is_empty()
            && self.classes.is_empty()
            && self.styles.is_empty()
            && self.style_variants.is_empty()
            && self.pseudo_elements.is_empty()
            && self.dynamic_styles.is_empty()
            && self.actions.is_empty()
            && self.raw_html.is_none()
            && self.raw_child_html.is_empty()
    }
}

impl GpuiReader {
    fn new(mode: GpuiImportMode) -> Self {
        Self {
            mode,
            text_inputs: BTreeMap::new(),
            source_hints: RefCell::new(VecDeque::new()),
            control_flow_hints: RefCell::new(VecDeque::new()),
        }
    }

    fn read(
        mut self,
        source: &TargetArtifact,
        cx: &mut AdapterContext,
    ) -> Result<RenderPlan, AdapterError> {
        let Some(file) = source
            .files
            .iter()
            .find(|file| file.path.ends_with(".rs"))
            .or_else(|| source.files.first())
        else {
            return Err(AdapterError::unsupported(
                "GPUI-to-HTML import requires at least one Rust source file",
            ));
        };

        let parsed = syn::parse_file(file.contents.as_str()).map_err(|error| {
            AdapterError::message(format!(
                "failed to parse GPUI target source `{}`: {error}",
                file.path
            ))
        })?;
        self.collect_generated_state_fields(&parsed);
        self.collect_preserved_comment_metadata(file.contents.as_str());
        self.source_hints
            .replace(collect_source_hints(file.contents.as_str()));
        self.control_flow_hints
            .replace(collect_control_flow_hints(file.contents.as_str()));

        let Some(render_expr) = find_render_expression(&parsed) else {
            return Err(AdapterError::unsupported(format!(
                "GPUI target source `{}` does not contain an impl Render::render body",
                file.path
            )));
        };

        let mut nodes = self.nodes_from_expression(render_expr, cx);
        restore_source_order_hints(&mut nodes);
        mark_generated_dc_body_wrapper(&mut nodes);
        Ok(RenderPlan::new(nodes)
            .with_head(collect_source_head_elements(file.contents.as_str()))
            .with_state(RenderStatePlan::default()))
    }

    fn collect_generated_state_fields(&mut self, file: &File) {
        for item in &file.items {
            let Item::Struct(item_struct) = item else {
                continue;
            };
            let syn::Fields::Named(fields) = &item_struct.fields else {
                continue;
            };
            for field in &fields.named {
                let Some(name) = &field.ident else {
                    continue;
                };
                if type_contains_path(&field.ty, "HtmlswapGpuiTextInput")
                    || type_contains_path(&field.ty, "gpui_component::input::InputState")
                {
                    self.text_inputs.entry(name.to_string().into()).or_default();
                }
            }
        }

        for item in &file.items {
            let Item::Impl(item_impl) = item else {
                continue;
            };
            for item in &item_impl.items {
                let syn::ImplItem::Fn(method) = item else {
                    continue;
                };
                if method.sig.ident == "new" {
                    self.collect_state_initializers_from_block(&method.block);
                }
            }
        }
    }

    fn collect_preserved_comment_metadata(&mut self, source: &str) {
        let mut pending_action = None;
        for line in source.lines().map(str::trim) {
            if let Some((event, handler)) = ["input", "change"].into_iter().find_map(|event| {
                preserved_template_action(line, event).map(|handler| (event, handler))
            }) {
                pending_action = Some(TextInputAction {
                    event: event.into(),
                    handler,
                });
                continue;
            }
            if let Some(field) = preserved_action_payload_state(line)
                && let Some(action) = pending_action.take()
                && let Some(metadata) = self.text_inputs.get_mut(field.as_str())
            {
                metadata.action = Some(action);
            }
        }
    }

    fn nodes_from_expression(&self, expression: &Expr, cx: &mut AdapterContext) -> Vec<RenderNode> {
        match expression {
            Expr::Block(block) => block_expr(&block.block)
                .map(|expr| self.nodes_from_expression(expr, cx))
                .unwrap_or_default(),
            Expr::Array(array) => array
                .elems
                .iter()
                .flat_map(|expr| self.nodes_from_expression(expr, cx))
                .collect(),
            Expr::If(if_expr) => self.nodes_from_if(if_expr, cx),
            _ => {
                if let Some(raw) = self.take_next_raw_hint(expression) {
                    return vec![RenderNode::Raw(raw)];
                }

                self.element_from_expression(expression, cx)
                    .map(|element| vec![RenderNode::Element(Box::new(element))])
                    .unwrap_or_else(|| {
                        if let Some(text) = text_from_expression(expression) {
                            return vec![RenderNode::Text(RenderText {
                                value: text,
                                template: None,
                                span: None,
                            })];
                        }
                        cx.push(Diagnostic::warning(
                            format!(
                                "GPUI import skipped an unsupported render expression: {}",
                                expression_summary(expression)
                            ),
                            None,
                        ));
                        Vec::new()
                    })
            }
        }
    }

    fn take_next_raw_hint(&self, expression: &Expr) -> Option<RenderRaw> {
        if !expression_starts_recovered_element(expression) {
            return None;
        }

        let mut hints = self.source_hints.borrow_mut();
        let raw_html = hints.front()?.raw_html.as_ref()?.clone();
        hints.pop_front();
        Some(RenderRaw {
            html: raw_html.to_string(),
            span: None,
        })
    }

    fn element_from_expression(
        &self,
        expression: &Expr,
        cx: &mut AdapterContext,
    ) -> Option<RenderElement> {
        let expression = final_expression(expression);
        let (base, methods) = flatten_method_chain(expression);
        let mut element = self.element_from_base(base)?;
        let source_hint = self.source_hints.borrow_mut().pop_front();

        for method in methods {
            self.apply_method(&mut element, method, cx);
        }

        if let Some(source_hint) = source_hint {
            apply_source_hint(&mut element, source_hint);
        }
        self.collapse_source_text_input_wrapper(&mut element);
        collapse_source_material_symbol_wrapper(&mut element);
        reconcile_generated_flex_fallbacks(&mut element);
        self.infer_element_after_methods(&mut element);
        Some(element)
    }

    fn element_from_base(&self, base: &Expr) -> Option<RenderElement> {
        let Expr::Call(call) = final_expression(base) else {
            return None;
        };
        let path = call_path(call)?;

        let element = if self.mode == GpuiImportMode::GpuiComponents
            && let Some(element) = self.component_element_from_call(&path, call)
        {
            element
        } else {
            match path.as_str() {
                "gpui::div" | "div" => Some(generated_container_element()),
                "gpui::img" | "img" => {
                    let mut element = element("img", UiRole::Image);
                    if let Some(src) = call.args.first().and_then(string_literal) {
                        element.attributes.push(attribute("src", src));
                    }
                    Some(element)
                }
                "htmlswap_material_symbol_icon" => {
                    let mut element = element("span", UiRole::Inline);
                    element.classes.push("ms".into());
                    if let Some(icon) = call.args.first().and_then(text_from_expression) {
                        element.children.push(text_node(icon));
                    }
                    Some(element)
                }
                _ => None,
            }?
        };
        Some(element)
    }

    fn component_element_from_call(&self, path: &str, call: &ExprCall) -> Option<RenderElement> {
        match path {
            "gpui_component::button::Button::new" => {
                let mut element = element("button", UiRole::Button);
                push_id_arg(&mut element, call);
                Some(element)
            }
            "gpui_component::link::Link::new" => {
                let mut element = element("a", UiRole::Link);
                push_id_arg(&mut element, call);
                Some(element)
            }
            "gpui_component::label::Label::new" => {
                let mut element = element("label", UiRole::Label);
                if let Some(text) = call.args.first().and_then(text_from_expression) {
                    element.children.push(text_node(text));
                }
                Some(element)
            }
            "gpui_component::TitleBar::new" => {
                let mut element = element("header", UiRole::Container);
                element
                    .attributes
                    .push(attribute("data-htmlswap-component", "titlebar"));
                Some(element)
            }
            "gpui_component::tab::TabBar::new" => {
                let mut element = element("div", UiRole::Container);
                element
                    .attributes
                    .push(attribute("data-htmlswap-component", "tabs"));
                element.attributes.push(attribute("role", "tablist"));
                push_id_arg(&mut element, call);
                Some(element)
            }
            "gpui_component::tab::Tab::new" => {
                let mut element = element("button", UiRole::Button);
                element
                    .attributes
                    .push(attribute("data-htmlswap-component", "tab"));
                element.attributes.push(attribute("role", "tab"));
                Some(element)
            }
            "gpui_component::input::Input::new" => {
                let mut element = element("input", UiRole::TextInput);
                if let Some(field) = call.args.first().and_then(text_input_state_reference) {
                    self.apply_text_input_metadata(&mut element, &field);
                }
                Some(element)
            }
            "gpui_component::checkbox::Checkbox::new" => {
                let mut element = element("input", UiRole::TextInput);
                element.attributes.push(attribute("type", "checkbox"));
                push_id_arg(&mut element, call);
                Some(element)
            }
            "gpui_component::radio::Radio::new" => {
                let mut element = element("input", UiRole::TextInput);
                element.attributes.push(attribute("type", "radio"));
                push_id_arg(&mut element, call);
                Some(element)
            }
            "gpui_component::select::Select::new" => Some(element("select", UiRole::Select)),
            "gpui_component::list::ListItem::new" => {
                let mut element = element("li", UiRole::ListItem);
                push_id_arg(&mut element, call);
                Some(element)
            }
            "gpui_component::form::Form::vertical" => Some(element("form", UiRole::Form)),
            "gpui_component::form::Form::horizontal" => {
                let mut element = element("form", UiRole::Form);
                element
                    .attributes
                    .push(attribute("data-htmlswap-form-layout", "horizontal"));
                Some(element)
            }
            "gpui_component::group_box::GroupBox::new" => {
                Some(element("fieldset", UiRole::Fieldset))
            }
            _ => None,
        }
    }

    fn apply_method(
        &self,
        element: &mut RenderElement,
        method: &ExprMethodCall,
        cx: &mut AdapterContext,
    ) {
        let method_name = method.method.to_string();
        match method_name.as_str() {
            "child" => {
                if let Some(argument) = method.args.first() {
                    if self.try_collapse_text_input_child(element, argument) {
                        return;
                    }
                    element.children.extend(unwrap_transparent_generated_nodes(
                        self.nodes_from_expression(argument, cx),
                    ));
                }
            }
            "children" => {
                if let Some(argument) = method.args.first() {
                    match final_expression(argument) {
                        Expr::Array(array) => {
                            for child in &array.elems {
                                element.children.extend(unwrap_transparent_generated_nodes(
                                    self.nodes_from_expression(child, cx),
                                ));
                            }
                        }
                        _ if let Some(loop_element) =
                            self.loop_element_from_children_expression(argument, cx) =>
                        {
                            element
                                .children
                                .push(RenderNode::Element(Box::new(loop_element)));
                        }
                        _ => cx.push(Diagnostic::warning(
                            "GPUI import skipped dynamic children; loop recovery is not implemented yet",
                            None,
                        )),
                    }
                }
            }
            "id" => {
                if let Some(value) = method.args.first().and_then(string_literal) {
                    push_or_replace_attribute(element, "id", value);
                }
            }
            "href" => {
                if let Some(value) = method.args.first().and_then(string_literal) {
                    push_or_replace_attribute(element, "href", value);
                }
            }
            "label" | "title" => {
                if let Some(value) = method.args.first().and_then(text_from_expression) {
                    element.children.push(text_node(value));
                }
            }
            "placeholder" => {
                if let Some(value) = method.args.first().and_then(string_literal) {
                    push_or_replace_attribute(element, "placeholder", value);
                }
            }
            "tooltip" => {
                if let Some(value) = method.args.first().and_then(tooltip_title_from_expression) {
                    push_or_replace_attribute(element, "title", value);
                }
            }
            "disabled" => push_or_replace_attribute(element, "disabled", ""),
            "selected" => {
                if has_attribute_value(element, "data-htmlswap-component", "tab") {
                    push_or_replace_attribute(element, "aria-selected", "true");
                } else {
                    push_or_replace_attribute(element, "selected", "");
                }
            }
            "checked" => push_or_replace_attribute(element, "checked", ""),
            "mask_toggle" => push_or_replace_attribute(element, "type", "password"),
            _ if let Some((attribute, value)) = semantic_attribute(
                &method_name,
                has_attribute_value(element, "data-htmlswap-component", "tabs"),
            ) =>
            {
                push_or_replace_attribute(element, attribute, value);
            }
            "with_size" => apply_component_size_method(element, method),
            "on_click" | "on_mouse_up" => {
                if let Some(action) = method.args.iter().find_map(find_this_invocation) {
                    let event = if method_name == "on_click"
                        && method.args.iter().any(expression_has_double_click_guard)
                    {
                        "doubleclick"
                    } else {
                        "click"
                    };
                    element.actions.push(action_binding(event, action));
                }
            }
            "on_double_click" => {
                if let Some(action) = method.args.iter().find_map(find_this_invocation) {
                    element.actions.push(action_binding("doubleclick", action));
                }
            }
            "on_hover" | "on_mouse_enter" => {
                if let Some(action) = method.args.iter().find_map(find_this_invocation) {
                    element.actions.push(action_binding("mouseenter", action));
                }
            }
            "on_any_mouse_down" => {
                if let Some(action) = method.args.iter().find_map(find_this_invocation) {
                    element.actions.push(action_binding("mousedown", action));
                }
            }
            "on_mouse_move" => {
                if let Some(action) = method.args.iter().find_map(find_this_invocation) {
                    element.actions.push(action_binding("mousemove", action));
                }
            }
            "on_scroll_wheel" => {
                if let Some(action) = method.args.iter().find_map(find_this_invocation) {
                    element.actions.push(action_binding("wheel", action));
                }
            }
            "on_key_down" => {
                if let Some(action) = method.args.iter().find_map(find_this_invocation) {
                    element.actions.push(action_binding("keydown", action));
                }
            }
            "on_key_up" => {
                if let Some(action) = method.args.iter().find_map(find_this_invocation) {
                    element.actions.push(action_binding("keyup", action));
                }
            }
            "on_input" => {
                if let Some(action) = method.args.iter().find_map(find_this_invocation) {
                    element.actions.push(action_binding("input", action));
                }
            }
            "on_change" => {
                if let Some(action) = method.args.iter().find_map(find_this_invocation) {
                    element.actions.push(action_binding("change", action));
                }
            }
            "on_submit" => {
                if let Some(action) = method.args.iter().find_map(find_this_invocation) {
                    element.actions.push(action_binding("submit", action));
                }
            }
            "hover" | "focus" | "active" => {
                if let Some(style) = style_variant_from_method_argument(method) {
                    push_or_replace_attribute(element, format!("style-{method_name}"), style);
                }
            }
            "when" => self.apply_when_method(element, method, cx),
            _ => {
                if is_generated_component_input_fill_method(element, method_name.as_str()) {
                    return;
                }
                if let Some(styles) = styles_from_method(&method_name, method) {
                    element.styles.extend(styles);
                }
            }
        }
    }

    fn try_collapse_text_input_child(
        &self,
        element: &mut RenderElement,
        expression: &Expr,
    ) -> bool {
        let Some(field) = self_field_clone(expression) else {
            return false;
        };
        if !self.text_inputs.contains_key(field.as_str()) {
            return false;
        }

        element.role = UiRole::TextInput;
        element.source_tag = "input".into();
        element.children.clear();
        self.apply_text_input_metadata(element, &field);
        true
    }

    fn collect_state_initializers_from_block(&mut self, block: &syn::Block) {
        let mut locals = BTreeMap::new();
        for statement in &block.stmts {
            if let Some((name, value)) = local_text_binding(statement) {
                locals.insert(name, value);
            }
            if let Some((field, value)) = text_input_value_update(statement, &locals) {
                self.text_inputs.entry(field.into()).or_default().value = Some(value);
            }
            if let Some((field, placeholder)) = text_input_placeholder_update(statement, &locals) {
                self.text_inputs
                    .entry(field.into())
                    .or_default()
                    .placeholder = Some(placeholder);
            }
            if let Some((field, action)) = text_input_subscription(statement) {
                self.text_inputs.entry(field.into()).or_default().action = Some(TextInputAction {
                    event: "input".into(),
                    handler: action,
                });
            }
            self.collect_state_initializers_from_statement(statement);
        }
    }

    fn collect_state_initializers_from_statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Local(local) => {
                if let Some(init) = &local.init {
                    if let syn::Pat::Ident(ident) = &local.pat
                        && let Some(metadata) = text_input_metadata_from_initializer(&init.expr)
                    {
                        self.text_inputs
                            .entry(ident.ident.to_string().into())
                            .and_modify(|existing| existing.merge(metadata.clone()))
                            .or_insert(metadata);
                    }
                    self.collect_state_initializers_from_expression(&init.expr);
                }
            }
            Stmt::Expr(expression, _) => {
                self.collect_state_initializers_from_expression(expression)
            }
            _ => {}
        }
    }

    fn collect_state_initializers_from_expression(&mut self, expression: &Expr) {
        match final_expression(expression) {
            Expr::Struct(struct_expr) => {
                for field in &struct_expr.fields {
                    let Some(name) = member_name(&field.member) else {
                        continue;
                    };
                    let Some(metadata) = text_input_metadata_from_initializer(&field.expr) else {
                        continue;
                    };
                    self.text_inputs
                        .entry(name.into())
                        .and_modify(|existing| existing.merge(metadata.clone()))
                        .or_insert(metadata);
                }
                if let Some(rest) = &struct_expr.rest {
                    self.collect_state_initializers_from_expression(rest);
                }
            }
            Expr::Block(block) => self.collect_state_initializers_from_block(&block.block),
            Expr::Call(call) => {
                for argument in &call.args {
                    self.collect_state_initializers_from_expression(argument);
                }
            }
            Expr::MethodCall(method) => {
                self.collect_state_initializers_from_expression(&method.receiver);
                for argument in &method.args {
                    self.collect_state_initializers_from_expression(argument);
                }
            }
            Expr::Closure(closure) => {
                self.collect_state_initializers_from_expression(&closure.body);
            }
            Expr::If(if_expr) => {
                self.collect_state_initializers_from_block(&if_expr.then_branch);
                if let Some((_, else_expr)) = &if_expr.else_branch {
                    self.collect_state_initializers_from_expression(else_expr);
                }
            }
            Expr::Match(match_expr) => {
                self.collect_state_initializers_from_expression(&match_expr.expr);
                for arm in &match_expr.arms {
                    self.collect_state_initializers_from_expression(&arm.body);
                }
            }
            _ => {}
        }
    }

    fn apply_text_input_metadata(&self, element: &mut RenderElement, field: &str) {
        let Some(metadata) = self.text_inputs.get(field) else {
            return;
        };

        if !has_attribute(element, "id")
            && let Some(id) = inferred_id_from_state_field(field)
        {
            push_or_replace_attribute(element, "id", id);
        }
        if let Some(value) = &metadata.value {
            push_or_replace_attribute(element, "value", value.clone());
        }
        if let Some(placeholder) = &metadata.placeholder {
            push_or_replace_attribute(element, "placeholder", placeholder.clone());
        }
        if metadata.password {
            push_or_replace_attribute(element, "type", "password");
        }
        if metadata.multiline {
            element.source_tag = "textarea".into();
        }
        if let Some(rows) = metadata.rows {
            push_or_replace_attribute(element, "rows", rows.to_string());
        }
        if let Some(action) = &metadata.action {
            element.actions.push(action_binding(
                action.event.as_str(),
                action.handler.clone(),
            ));
        }
    }

    fn infer_element_after_methods(&self, element: &mut RenderElement) {
        if is_div_like_source(element)
            && element.role == UiRole::Container
            && has_click_like_action(element)
            && !has_attribute(element, "href")
            && !has_attribute(element, "data-htmlswap-component")
            && has_only_button_phrasing_children(element)
        {
            element.role = UiRole::Button;
        }
    }

    fn collapse_source_text_input_wrapper(&self, element: &mut RenderElement) {
        if element.source_tag != "input" || element.role == UiRole::TextInput {
            return;
        }

        let Some(input) = collapsed_wrapped_text_input(element) else {
            return;
        };
        *element = input;
    }

    fn nodes_from_if(&self, if_expr: &syn::ExprIf, cx: &mut AdapterContext) -> Vec<RenderNode> {
        let mut nodes = Vec::new();
        let condition = template_binding_expression(&if_expr.cond);
        let then_hint = self.take_matching_control_flow_hint("if", Some(condition.as_str()), None);
        let then_children = block_expr(&if_expr.then_branch)
            .map(|expr| self.nodes_from_expression(expr, cx))
            .unwrap_or_default();
        if !then_children.is_empty() {
            let mut element = control_flow_element("sc-if", "value", condition, then_children);
            if let Some(hint) = then_hint {
                apply_control_flow_hint(&mut element, "if", hint);
            }
            nodes.push(RenderNode::Element(Box::new(element)));
        }

        if let Some((_, else_expr)) = &if_expr.else_branch {
            let fallback_condition = format!(
                "{{{{ !{} }}}}",
                rust_binding_to_template_path(&compact_expr_name(&if_expr.cond))
            );
            let else_hint =
                self.take_matching_control_flow_hint("if", Some(fallback_condition.as_str()), None);
            let else_children = self.nodes_from_expression(else_expr, cx);
            if !else_children.is_empty() {
                let mut element =
                    control_flow_element("sc-if", "value", fallback_condition, else_children);
                if let Some(hint) = else_hint {
                    apply_control_flow_hint(&mut element, "if", hint);
                }
                nodes.push(RenderNode::Element(Box::new(element)));
            }
        }
        nodes
    }

    fn loop_element_from_children_expression(
        &self,
        expression: &Expr,
        cx: &mut AdapterContext,
    ) -> Option<RenderElement> {
        let Expr::MethodCall(top_call) = final_expression(expression) else {
            return None;
        };
        let map_expression = if top_call.method == "collect" {
            final_expression(&top_call.receiver)
        } else {
            final_expression(expression)
        };

        let Expr::MethodCall(map_call) = map_expression else {
            return None;
        };
        if map_call.method != "map" {
            return None;
        }

        let iter_call = loop_iter_call_from_map_receiver(&map_call.receiver)?;

        let Expr::Closure(closure) = map_call.args.first().map(final_expression)? else {
            return None;
        };
        let binding = closure.inputs.first().and_then(loop_item_pattern_ident)?;
        let list = template_binding_expression(&iter_call.receiver);
        let hint = self.take_matching_control_flow_hint(
            "for",
            Some(list.as_str()),
            Some(binding.as_str()),
        );
        let children = unwrap_transparent_loop_body(self.nodes_from_expression(&closure.body, cx));
        let mut loop_element = element("sc-for", UiRole::Container);
        loop_element.attributes.push(attribute("list", list));
        loop_element.attributes.push(attribute("as", binding));
        loop_element.children = children;
        if let Some(hint) = hint {
            apply_control_flow_hint(&mut loop_element, "for", hint);
        }
        Some(loop_element)
    }

    fn take_matching_control_flow_hint(
        &self,
        expected_kind: &str,
        expression: Option<&str>,
        binding: Option<&str>,
    ) -> Option<ControlFlowHint> {
        let mut hints = self.control_flow_hints.borrow_mut();
        let expression = expression.map(normalize_control_flow_expression);
        let binding = binding.map(normalize_control_flow_binding);
        let index = hints.iter().position(|hint| {
            if hint.kind != expected_kind {
                return false;
            }
            if let Some(expression) = &expression
                && hint
                    .expression
                    .as_deref()
                    .is_none_or(|hint| normalize_control_flow_expression(hint) != *expression)
            {
                return false;
            }
            if let Some(binding) = &binding
                && hint
                    .binding
                    .as_deref()
                    .is_none_or(|hint| normalize_control_flow_binding(hint) != *binding)
            {
                return false;
            }
            true
        })?;
        hints.remove(index)
    }

    fn apply_when_method(
        &self,
        parent: &mut RenderElement,
        method: &ExprMethodCall,
        cx: &mut AdapterContext,
    ) {
        let Some(condition) = method.args.first().map(template_binding_expression) else {
            return;
        };
        let Some(Expr::Closure(closure)) = method.args.iter().nth(1).map(final_expression) else {
            return;
        };

        let hint = self.take_matching_control_flow_hint("if", Some(condition.as_str()), None);
        let mut branch = element("sc-if", UiRole::Container);
        if !self.apply_this_chain(&mut branch, &closure.body, cx) {
            return;
        }

        let mut condition_element =
            control_flow_element("sc-if", "value", condition, branch.children);
        if let Some(hint) = hint {
            apply_control_flow_hint(&mut condition_element, "if", hint);
        }
        parent
            .children
            .push(RenderNode::Element(Box::new(condition_element)));
    }

    fn apply_this_chain(
        &self,
        element: &mut RenderElement,
        expression: &Expr,
        cx: &mut AdapterContext,
    ) -> bool {
        let (base, methods) = flatten_method_chain(expression);
        if !is_path_ident(base, "this") {
            return false;
        }
        for method in methods {
            self.apply_method(element, method, cx);
        }
        true
    }
}

impl TextInputMetadata {
    fn merge(&mut self, other: Self) {
        if self.value.is_none() {
            self.value = other.value;
        }
        if self.placeholder.is_none() {
            self.placeholder = other.placeholder;
        }
        self.password |= other.password;
        self.multiline |= other.multiline;
        if self.rows.is_none() {
            self.rows = other.rows;
        }
        if self.action.is_none() {
            self.action = other.action;
        }
    }
}

fn apply_control_flow_hint(
    element: &mut RenderElement,
    expected_kind: &str,
    hint: ControlFlowHint,
) {
    match expected_kind {
        "for" => {
            if let Some(expression) = hint.expression {
                push_or_replace_attribute(
                    element,
                    "list",
                    format_template_binding(expression.as_str()),
                );
            }
            if let Some(binding) = hint.binding {
                push_or_replace_attribute(element, "as", binding);
            }
        }
        "if" | "else-if" => {
            if let Some(expression) = hint.expression {
                push_or_replace_attribute(
                    element,
                    "value",
                    format_template_binding(expression.as_str()),
                );
            }
        }
        _ => {}
    }
    if let Some(source_order) = hint.source_order {
        push_or_replace_attribute(
            element,
            INTERNAL_SOURCE_ORDER_ATTRIBUTE,
            source_order.to_string(),
        );
    }
    if let Some(placeholder) = hint.placeholder
        && let Some((name, value)) = parse_control_flow_placeholder(placeholder.as_str())
    {
        push_or_replace_attribute(element, name, value);
    }
}

fn restore_source_order_hints(nodes: &mut Vec<RenderNode>) {
    for node in nodes.iter_mut() {
        let RenderNode::Element(element) = node else {
            continue;
        };
        restore_source_order_hints(&mut element.children);
    }

    if nodes.iter().any(node_source_order_hint) {
        let mut indexed = std::mem::take(nodes)
            .into_iter()
            .enumerate()
            .collect::<Vec<_>>();
        indexed.sort_by_key(|(index, node)| {
            (owned_node_source_order_hint(node).unwrap_or(*index), *index)
        });
        nodes.extend(indexed.into_iter().map(|(_, node)| node));
    }

    for node in nodes.iter_mut() {
        strip_source_order_hint(node);
    }
}

fn node_source_order_hint(node: &RenderNode) -> bool {
    let RenderNode::Element(element) = node else {
        return false;
    };
    element_source_order_hint(element).is_some()
}

fn owned_node_source_order_hint(node: &RenderNode) -> Option<usize> {
    let RenderNode::Element(element) = node else {
        return None;
    };
    element_source_order_hint(element)
}

fn element_source_order_hint(element: &RenderElement) -> Option<usize> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name == INTERNAL_SOURCE_ORDER_ATTRIBUTE)
        .and_then(|attribute| attribute.value.parse::<usize>().ok())
}

fn strip_source_order_hint(node: &mut RenderNode) {
    let RenderNode::Element(element) = node else {
        return;
    };
    element
        .attributes
        .retain(|attribute| attribute.name != INTERNAL_SOURCE_ORDER_ATTRIBUTE);
}

fn find_render_expression(file: &File) -> Option<&Expr> {
    let mut candidate = None;
    for item in &file.items {
        let Item::Impl(item_impl) = item else {
            continue;
        };
        if !item_impl
            .trait_
            .as_ref()
            .is_some_and(|(_, path, _)| path_ends_with(path, "Render"))
        {
            continue;
        }

        for item in &item_impl.items {
            let syn::ImplItem::Fn(method) = item else {
                continue;
            };
            if method.sig.ident == "render" {
                candidate = block_expr(&method.block);
            }
        }
    }
    candidate
}

fn block_expr(block: &syn::Block) -> Option<&Expr> {
    match block.stmts.last()? {
        Stmt::Expr(expr, _) => Some(expr),
        Stmt::Local(local) => local.init.as_ref().map(|init| &*init.expr),
        _ => None,
    }
}

fn final_expression(expression: &Expr) -> &Expr {
    match expression {
        Expr::Block(block) => block_expr(&block.block).map_or(expression, final_expression),
        Expr::Paren(paren) => final_expression(&paren.expr),
        Expr::Group(group) => final_expression(&group.expr),
        _ => expression,
    }
}

fn flatten_method_chain(expression: &Expr) -> (&Expr, Vec<&ExprMethodCall>) {
    let expression = final_expression(expression);
    if let Expr::MethodCall(method) = expression {
        let (base, mut methods) = flatten_method_chain(&method.receiver);
        methods.push(method);
        return (base, methods);
    }
    (expression, Vec::new())
}

fn type_contains_path(ty: &syn::Type, needle: &str) -> bool {
    match ty {
        syn::Type::Path(path) => path.path.segments.iter().any(|segment| {
            segment.ident == needle
                || segment.ident == "InputState"
                || segment_arguments_contain_path(&segment.arguments, needle)
        }),
        _ => false,
    }
}

fn segment_arguments_contain_path(arguments: &syn::PathArguments, needle: &str) -> bool {
    let syn::PathArguments::AngleBracketed(arguments) = arguments else {
        return false;
    };

    arguments.args.iter().any(|argument| match argument {
        syn::GenericArgument::Type(ty) => type_contains_path(ty, needle),
        _ => false,
    })
}

fn path_ends_with(path: &syn::Path, segment: &str) -> bool {
    path.segments
        .last()
        .is_some_and(|last| last.ident == segment)
}

fn call_path(call: &ExprCall) -> Option<String> {
    let Expr::Path(path) = final_expression(&call.func) else {
        return None;
    };
    Some(
        path.path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
    )
}

fn string_literal(expression: &Expr) -> Option<String> {
    let Expr::Lit(lit) = final_expression(expression) else {
        return None;
    };
    let Lit::Str(value) = &lit.lit else {
        return None;
    };
    Some(value.value())
}

fn text_from_expression(expression: &Expr) -> Option<String> {
    if let Some(value) = string_literal(expression) {
        return Some(value);
    }

    if let Expr::MethodCall(method) = final_expression(expression)
        && method.args.is_empty()
    {
        match method.method.to_string().as_str() {
            "to_uppercase" => {
                return text_from_expression(&method.receiver)
                    .map(|value| transform_template_literal_text(&value, TextTransform::Upper));
            }
            "to_lowercase" => {
                return text_from_expression(&method.receiver)
                    .map(|value| transform_template_literal_text(&value, TextTransform::Lower));
            }
            "to_string" => return text_from_expression(&method.receiver),
            _ => {}
        }
    }

    if let Expr::Macro(macro_expr) = final_expression(expression) {
        return text_from_format_macro(macro_expr);
    }

    let Expr::Call(call) = final_expression(expression) else {
        return None;
    };
    if call_path(call).as_deref() != Some("format") {
        return None;
    }
    if call.args.len() == 2 && string_literal(&call.args[0]).as_deref() == Some("{}") {
        return Some(template_binding_expression(&call.args[1]));
    }
    None
}

#[derive(Debug, Clone, Copy)]
enum TextTransform {
    Upper,
    Lower,
}

fn transform_template_literal_text(text: &str, transform: TextTransform) -> String {
    let mut output = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let (literal, template) = rest.split_at(start);
        push_transformed_literal(&mut output, literal, transform);
        if let Some(end) = template.find("}}") {
            let (binding, tail) = template.split_at(end + 2);
            output.push_str(binding);
            rest = tail;
        } else {
            push_transformed_literal(&mut output, template, transform);
            return output;
        }
    }
    push_transformed_literal(&mut output, rest, transform);
    output
}

fn push_transformed_literal(output: &mut String, literal: &str, transform: TextTransform) {
    match transform {
        TextTransform::Upper => output.push_str(&literal.to_uppercase()),
        TextTransform::Lower => output.push_str(&literal.to_lowercase()),
    }
}

fn text_from_format_macro(expression: &syn::ExprMacro) -> Option<String> {
    if !path_ends_with(&expression.mac.path, "format") {
        return None;
    }
    let parser = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated;
    let arguments = parser.parse2(expression.mac.tokens.clone()).ok()?;
    let mut arguments = arguments.iter();
    let mut text = string_literal(arguments.next()?)?;
    for argument in arguments {
        let replacement = template_binding_expression(argument);
        if text.contains("{}") {
            text = text.replacen("{}", &replacement, 1);
        }
    }
    Some(text)
}

fn tooltip_title_from_expression(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Paren(paren) => return tooltip_title_from_expression(&paren.expr),
        Expr::Group(group) => return tooltip_title_from_expression(&group.expr),
        Expr::Block(block) => {
            for statement in &block.block.stmts {
                if let Some(value) = tooltip_title_from_local(statement) {
                    return Some(value);
                }
            }
            return block_expr(&block.block).and_then(tooltip_title_from_expression);
        }
        _ => {}
    }

    text_from_expression(expression).or_else(|| match final_expression(expression) {
        Expr::Closure(closure) => tooltip_title_from_expression(&closure.body),
        _ => None,
    })
}

fn tooltip_title_from_local(statement: &Stmt) -> Option<String> {
    let Stmt::Local(local) = statement else {
        return None;
    };
    let syn::Pat::Ident(ident) = &local.pat else {
        return None;
    };
    if ident.ident != "htmlswap_tooltip" {
        return None;
    }

    text_from_expression(&local.init.as_ref()?.expr)
}

fn template_binding_expression(expression: &Expr) -> String {
    let name = compact_expr_name(expression);
    let name = rust_binding_to_template_path(&name);
    format!("{{{{ {name} }}}}")
}

fn rust_binding_to_template_path(name: &str) -> String {
    name.trim_start_matches("self.")
        .trim_start_matches("this.")
        .split('.')
        .map(|segment| {
            let segment = segment.strip_suffix("()").unwrap_or(segment);
            let segment = segment.strip_prefix("r#").unwrap_or(segment);
            let segment = segment.strip_prefix("htmlswap_capture_").unwrap_or(segment);
            if segment.contains('_') {
                segment.to_lower_camel_case()
            } else {
                segment.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn compact_expr_name(expression: &Expr) -> String {
    match final_expression(expression) {
        Expr::MethodCall(method) if method.args.is_empty() => {
            let receiver = compact_expr_name(&method.receiver);
            if receiver == "self" {
                format!("{}()", method.method)
            } else {
                format!("{receiver}.{}()", method.method)
            }
        }
        Expr::Reference(reference) => compact_expr_name(&reference.expr),
        Expr::Field(field) => {
            let base = compact_expr_name(&field.base);
            if base.is_empty() {
                field.member.to_token_stream_string()
            } else {
                format!("{base}.{}", field.member.to_token_stream_string())
            }
        }
        Expr::Path(path) => path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("."),
        _ => "value".to_owned(),
    }
}

fn self_field_clone(expression: &Expr) -> Option<String> {
    let Expr::MethodCall(clone_call) = final_expression(expression) else {
        return None;
    };
    if clone_call.method != "clone" {
        return None;
    }
    let Expr::Field(field) = final_expression(&clone_call.receiver) else {
        return None;
    };
    let Expr::Path(receiver) = final_expression(&field.base) else {
        return None;
    };
    if !receiver.path.is_ident("self") {
        return None;
    }
    match &field.member {
        syn::Member::Named(name) => Some(name.to_string()),
        syn::Member::Unnamed(_) => None,
    }
}

fn self_field_reference(expression: &Expr) -> Option<String> {
    receiver_field_reference(expression, "self")
}

fn this_field_reference(expression: &Expr) -> Option<String> {
    receiver_field_reference(expression, "this")
}

fn text_input_state_reference(expression: &Expr) -> Option<String> {
    self_field_reference(expression).or_else(|| local_value_reference(expression))
}

fn receiver_field_reference(expression: &Expr, receiver_name: &str) -> Option<String> {
    match final_expression(expression) {
        Expr::Reference(reference) => receiver_field_reference(&reference.expr, receiver_name),
        Expr::MethodCall(method)
            if matches!(
                method.method.to_string().as_str(),
                "clone" | "into" | "to_owned"
            ) =>
        {
            receiver_field_reference(&method.receiver, receiver_name)
        }
        Expr::Field(field) => {
            let Expr::Path(receiver) = final_expression(&field.base) else {
                return None;
            };
            if !receiver.path.is_ident(receiver_name) {
                return None;
            }
            member_name(&field.member)
        }
        _ => None,
    }
}

fn local_text_binding(statement: &Stmt) -> Option<(String, CompactString)> {
    let Stmt::Local(local) = statement else {
        return None;
    };
    let syn::Pat::Ident(ident) = &local.pat else {
        return None;
    };
    let init = local.init.as_ref()?;
    text_from_expression(&init.expr).map(|value| (ident.ident.to_string(), value.into()))
}

fn text_input_value_update(
    statement: &Stmt,
    locals: &BTreeMap<String, CompactString>,
) -> Option<(String, CompactString)> {
    text_input_text_update(statement, locals, "set_value")
}

fn text_input_placeholder_update(
    statement: &Stmt,
    locals: &BTreeMap<String, CompactString>,
) -> Option<(String, CompactString)> {
    text_input_text_update(statement, locals, "set_placeholder")
}

fn text_input_text_update(
    statement: &Stmt,
    locals: &BTreeMap<String, CompactString>,
    setter: &str,
) -> Option<(String, CompactString)> {
    let expression = statement_expression(statement)?;
    let Expr::MethodCall(update) = final_expression(expression) else {
        return None;
    };
    if update.method != "update" {
        return None;
    }
    let field = this_field_reference(&update.receiver)
        .or_else(|| local_value_reference(&update.receiver))?;
    let closure = update
        .args
        .iter()
        .find_map(|argument| match final_expression(argument) {
            Expr::Closure(closure) => Some(closure),
            _ => None,
        })?;
    let value_expression = find_method_call_argument(&closure.body, setter)?;
    let value = local_value_reference(value_expression)
        .and_then(|name| locals.get(name.as_str()).cloned())
        .or_else(|| text_from_expression(value_expression).map(Into::into))?;
    Some((field, value))
}

fn text_input_subscription(statement: &Stmt) -> Option<(String, CompactString)> {
    let expression = statement_expression(statement)?;
    let Expr::MethodCall(subscription) = final_expression(expression) else {
        return None;
    };
    if subscription.method != "subscribe_in" {
        return None;
    }
    let field = subscription.args.first().and_then(|argument| {
        this_field_reference(argument).or_else(|| local_value_reference(argument))
    })?;
    let action =
        subscription
            .args
            .iter()
            .find_map(|argument| match final_expression(argument) {
                Expr::Closure(closure) => find_this_invocation(&closure.body),
                _ => None,
            })?;
    Some((field, action))
}

fn preserved_template_action(line: &str, event: &str) -> Option<CompactString> {
    let prefix = format!("// template action: {event}=\"{{{{");
    let start = line.find(&prefix)? + prefix.len();
    let end = line[start..].find("}}")? + start;
    Some(rust_binding_to_template_path(line[start..end].trim()).into())
}

fn preserved_action_payload_state(line: &str) -> Option<CompactString> {
    let prefix = "// action payload: element state `";
    let start = line.find(prefix)? + prefix.len();
    let end = line[start..].find('`')? + start;
    Some(line[start..end].into())
}

fn collect_source_hints(source: &str) -> VecDeque<SourceHint> {
    let source = render_source_for_hints(source);
    let mut hints = VecDeque::new();
    let mut pending = SourceHint::default();
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(source_order) = source_hint_source_order(trimmed) {
            pending.source_order = Some(source_order);
            continue;
        }
        if let Some((name, value)) = source_hint_attribute(trimmed) {
            pending.attributes.push((name, value));
            continue;
        }
        if let Some((name, value)) = source_hint_generated_text_input_attribute(trimmed) {
            pending.attributes.push((name, value));
            continue;
        }
        if let Some(tag) = source_hint_tag(trimmed) {
            pending.source_tag = Some(tag);
            continue;
        }
        if let Some(classes) = source_hint_classes(trimmed) {
            pending.classes.extend(classes);
            continue;
        }
        if let Some(raw_html) = source_hint_raw_html(trimmed) {
            pending.raw_html = Some(raw_html);
            continue;
        }
        if let Some(raw_child_html) = source_hint_raw_child_html(trimmed) {
            pending.raw_child_html.push(raw_child_html);
            continue;
        }
        if let Some(style) = source_hint_style(trimmed) {
            pending.styles.push(style);
            continue;
        }
        if let Some(variant) = source_hint_style_variant(trimmed) {
            pending.style_variants.push(variant);
            continue;
        }
        if let Some(pseudo_element) = source_hint_pseudo_element(trimmed) {
            pending.pseudo_elements.push(pseudo_element);
            continue;
        }
        if let Some(dynamic_style) = source_hint_dynamic_style(trimmed) {
            pending.dynamic_styles.push(dynamic_style);
            continue;
        }
        if let Some(action) = source_hint_action(trimmed) {
            pending.actions.push(action);
            continue;
        }

        let recovered_element_count = line_recovered_element_start_count(trimmed);
        if recovered_element_count > 0 {
            hints.push_back(std::mem::take(&mut pending));
            for _ in 1..recovered_element_count {
                hints.push_back(SourceHint::default());
            }
        } else if !trimmed.starts_with("//")
            && !trimmed.is_empty()
            && !pending.is_empty()
            && !line_is_hint_scaffolding(trimmed)
        {
            pending = SourceHint::default();
        }
    }
    hints
}

fn collect_control_flow_hints(source: &str) -> VecDeque<ControlFlowHint> {
    let mut hints = VecDeque::new();
    let mut pending_order = None;
    for line in render_source_for_hints(source).lines() {
        let trimmed = line.trim();
        if let Some(source_order) = source_hint_control_flow_source_order(trimmed) {
            pending_order = Some(source_order);
            continue;
        }
        if let Some(mut hint) = source_control_flow_hint(trimmed) {
            hint.source_order = pending_order.take();
            hints.push_back(hint);
            continue;
        }
        if !trimmed.starts_with("//")
            && !trimmed.is_empty()
            && pending_order.is_some()
            && !line_is_hint_scaffolding(trimmed)
        {
            pending_order = None;
        }
    }
    hints
}

fn collect_source_head_elements(source: &str) -> Vec<RenderHeadElement> {
    let mut seen = BTreeSet::new();
    let mut head = Vec::new();
    for line in source.lines().map(str::trim) {
        let Some(html) = source_head_html(line) else {
            continue;
        };
        if !seen.insert(html.clone()) {
            if is_source_head_group_boundary(&html) {
                break;
            }
            continue;
        }
        head.push(RenderHeadElement { html, span: None });
    }
    head
}

fn source_head_html(line: &str) -> Option<String> {
    line.trim_start_matches("//")
        .trim()
        .strip_prefix("source head: ")
        .map(str::trim)
        .filter(|html| !html.is_empty())
        .map(str::to_owned)
}

fn is_source_head_group_boundary(html: &str) -> bool {
    let html = html.trim_start().to_ascii_lowercase();
    html.starts_with("<meta charset") || html.starts_with("<meta name=\"viewport\"")
}

fn source_control_flow_hint(line: &str) -> Option<ControlFlowHint> {
    let line = line.trim_start_matches("//").trim();
    let rest = line.strip_prefix("source control-flow: ")?;
    let mut parts = rest.split(", ").map(str::trim);
    let kind = parts.next()?.into();
    let mut hint = ControlFlowHint {
        kind,
        ..Default::default()
    };
    for part in parts {
        if let Some(value) = part.strip_prefix("expr=") {
            hint.expression = Some(value.into());
        } else if let Some(value) = part.strip_prefix("item=") {
            hint.binding = Some(value.into());
        } else if let Some(value) = part.strip_prefix("placeholder=") {
            hint.placeholder = Some(value.into());
        }
    }
    Some(hint)
}

fn render_source_for_hints(source: &str) -> &str {
    source
        .rfind("fn render")
        .map_or(source, |offset| &source[offset..])
}

fn source_hint_source_order(line: &str) -> Option<usize> {
    source_order_hint(line, "htmlswap source-order: ")
}

fn source_hint_control_flow_source_order(line: &str) -> Option<usize> {
    source_order_hint(line, "htmlswap control-flow source-order: ")
}

fn source_order_hint(line: &str, prefix: &str) -> Option<usize> {
    let line = line.trim_start_matches("//").trim();
    let value = line.strip_prefix(prefix)?.split_whitespace().next()?;
    value.parse::<usize>().ok()
}

fn line_is_hint_scaffolding(line: &str) -> bool {
    let line = line.trim();
    matches!(line, "{" | "}" | ")" | ");" | "})" | "}),")
        || line.starts_with('}')
        || line.ends_with('{')
        || matches!(
            line,
            "input," | "event," | "window," | "_window," | "cx," | "cx| {"
        )
        || line.starts_with(".child(")
        || line.starts_with(".children(")
        || line.starts_with(".when(")
        || line.starts_with(".default_value(")
        || line.starts_with(".placeholder(")
        || (line.starts_with("let ") && line.contains(".clone()"))
        || line.starts_with("move |")
        || line.starts_with("event:")
        || line.starts_with("input.read(")
        || line.starts_with("state.set_")
        || line.starts_with("if matches!(")
        || line.starts_with("if let ")
        || line == "if self"
        || line.contains("InputState::new(")
        || line.contains("HtmlswapGpuiTextInput::new(")
        || line.contains("gpui_component::input::InputEvent")
        || line.contains("htmlswap_")
        || line.contains("_htmlswap_")
}

fn line_recovered_element_start_count(line: &str) -> usize {
    if line.starts_with("fn ") {
        return 0;
    }
    if line.starts_with("//") {
        return 0;
    }

    let qualified_count: usize = [
        "gpui::div()",
        "gpui::img(",
        "htmlswap_material_symbol_icon(",
        "gpui_component::button::Button::new",
        "gpui_component::link::Link::new",
        "gpui_component::label::Label::new",
        "gpui_component::TitleBar::new",
        "gpui_component::tab::TabBar::new",
        "gpui_component::tab::Tab::new",
        "gpui_component::input::Input::new",
        "gpui_component::checkbox::Checkbox::new",
        "gpui_component::radio::Radio::new",
        "gpui_component::select::Select::new",
        "gpui_component::list::ListItem::new",
        "gpui_component::form::Form::vertical",
        "gpui_component::form::Form::horizontal",
        "gpui_component::group_box::GroupBox::new",
    ]
    .into_iter()
    .map(|needle| line.matches(needle).count())
    .sum();

    qualified_count + unqualified_call_count(line, "div") + unqualified_call_count(line, "img")
}

fn unqualified_call_count(line: &str, name: &str) -> usize {
    let needle = format!("{name}(");
    line.match_indices(&needle)
        .filter(|(index, _)| {
            let before = &line[..*index];
            !before.ends_with("::")
                && before
                    .chars()
                    .last()
                    .is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
        })
        .count()
}

fn expression_starts_recovered_element(expression: &Expr) -> bool {
    let expression = final_expression(expression);
    let (base, _) = flatten_method_chain(expression);
    let Expr::Call(call) = final_expression(base) else {
        return false;
    };
    let Some(path) = call_path(call) else {
        return false;
    };

    matches!(
        path.as_str(),
        "gpui::div"
            | "div"
            | "gpui::img"
            | "img"
            | "htmlswap_material_symbol_icon"
            | "gpui_component::button::Button::new"
            | "gpui_component::link::Link::new"
            | "gpui_component::label::Label::new"
            | "gpui_component::TitleBar::new"
            | "gpui_component::tab::TabBar::new"
            | "gpui_component::tab::Tab::new"
            | "gpui_component::input::Input::new"
            | "gpui_component::checkbox::Checkbox::new"
            | "gpui_component::radio::Radio::new"
            | "gpui_component::select::Select::new"
            | "gpui_component::list::ListItem::new"
            | "gpui_component::form::Form::vertical"
            | "gpui_component::form::Form::horizontal"
            | "gpui_component::group_box::GroupBox::new"
    )
}

fn source_hint_attribute(line: &str) -> Option<(CompactString, CompactString)> {
    let line = line.trim_start_matches("//").trim();
    let (name, value) = if let Some(value) = line.strip_prefix("source region: ") {
        ("data-htmlswap-region", value)
    } else if let Some(value) = line.strip_prefix("source variant: ") {
        ("data-htmlswap-variant", value)
    } else if let Some(value) = line.strip_prefix("source tone: ") {
        ("data-htmlswap-tone", value)
    } else if let Some(value) = line.strip_prefix("source size: ") {
        ("data-htmlswap-size", value)
    } else if let Some(value) = line.strip_prefix("source density: ") {
        ("data-htmlswap-density", value)
    } else if let Some(value) = line.strip_prefix("source key: ") {
        ("data-htmlswap-key", value)
    } else if let Some(value) = line.strip_prefix("source state id: ") {
        ("data-htmlswap-state", value)
    } else if let Some(value) = line.strip_prefix("source state owner: ") {
        ("data-htmlswap-state-owner", value)
    } else if let Some(value) = line.strip_prefix("source component: ") {
        ("data-htmlswap-component", value)
    } else if let Some(value) = line.strip_prefix("source component source: ") {
        ("data-htmlswap-component-source", value)
    } else if let Some(value) = line.strip_prefix("source slot: ") {
        ("data-htmlswap-slot", value)
    } else if let Some(value) = line.strip_prefix("source child strategy: ") {
        ("data-htmlswap-children", value)
    } else if let Some(value) = line.strip_prefix("unmapped tooltip: ") {
        ("title", value)
    } else if let Some(prop) = line.strip_prefix("source prop ") {
        let (name, value) = prop.split_once('=')?;
        return Some((
            CompactString::from(format!("data-htmlswap-prop-{}", name.trim())),
            CompactString::from(unquote_debug_string(value.trim())),
        ));
    } else {
        let attribute = line.strip_prefix("unmapped attribute: ")?;
        let (name, value) = parse_unmapped_attribute(attribute)?;
        return Some((name.into(), value.into()));
    };

    Some((name.into(), CompactString::from(value.trim())))
}

fn source_hint_generated_text_input_attribute(
    line: &str,
) -> Option<(CompactString, CompactString)> {
    if !line.contains("htmlswap_input_value") && !line.contains("htmlswap_input_placeholder") {
        return None;
    }

    let Stmt::Local(local) = syn::parse_str::<Stmt>(line).ok()? else {
        return None;
    };
    let syn::Pat::Ident(ident) = &local.pat else {
        return None;
    };
    let name = match ident.ident.to_string().as_str() {
        "htmlswap_input_value" => "value",
        "htmlswap_input_placeholder" => "placeholder",
        _ => return None,
    };
    let value = text_from_expression(&local.init.as_ref()?.expr)?;
    if name == "placeholder" && value.is_empty() {
        return None;
    }

    Some((name.into(), value.into()))
}

fn source_hint_tag(line: &str) -> Option<CompactString> {
    let line = line.trim_start_matches("//").trim();
    line.strip_prefix("source tag: ")
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(CompactString::from)
}

fn source_hint_raw_html(line: &str) -> Option<CompactString> {
    let line = line.trim_start_matches("//").trim();
    line.strip_prefix("htmlswap raw HTML omitted: ")
        .map(str::trim)
        .filter(|html| !html.is_empty())
        .map(CompactString::from)
}

fn source_hint_raw_child_html(line: &str) -> Option<CompactString> {
    let line = line.trim_start_matches("//").trim();
    line.strip_prefix("htmlswap raw child HTML omitted: ")
        .map(str::trim)
        .filter(|html| !html.is_empty())
        .map(CompactString::from)
}

fn source_hint_style(line: &str) -> Option<StyleDeclaration> {
    let line = line.trim_start_matches("//").trim();
    let value = line
        .strip_prefix("unmapped CSS: ")
        .or_else(|| line.strip_prefix("unmapped root CSS: "))
        .or_else(|| line.strip_prefix("preserved CSS: "))
        .or_else(|| line.strip_prefix("preserved root CSS: "))
        .or_else(|| line.strip_prefix("planner CSS: "))?;
    parse_css_declarations(value).into_iter().next()
}

fn source_hint_style_variant(line: &str) -> Option<RenderStyleVariant> {
    let line = line.trim_start_matches("//").trim();
    let rest = line.strip_prefix("conditional CSS ")?;
    let (header, declarations) = split_preserved_css_block(rest)?;
    let (conditions, selector) = parse_preserved_style_header(header);
    let declarations = parse_css_declarations(declarations);
    (!declarations.is_empty()).then(|| RenderStyleVariant {
        conditions,
        selector: selector.into(),
        declarations,
        span: None,
    })
}

fn source_hint_pseudo_element(line: &str) -> Option<RenderPseudoElement> {
    let line = line.trim_start_matches("//").trim();
    let rest = line.strip_prefix("pseudo-element CSS ")?;
    if let Some(pseudo_element) = source_hint_pseudo_element_metadata(rest) {
        return Some(pseudo_element);
    }
    source_hint_pseudo_element_legacy(rest)
}

fn source_hint_pseudo_element_metadata(rest: &str) -> Option<RenderPseudoElement> {
    let (header, declarations) = split_preserved_css_block(rest)?;
    let header = header.trim();
    let header = header.strip_prefix("selector=")?;
    let (selector, rest) = header.split_once("; kind=")?;
    let (kind, conditions) = rest.split_once("; conditions=")?;
    let selector = unquote_debug_string(selector);
    let kind = unquote_debug_string(kind);
    let conditions = parse_pseudo_condition_metadata(&unquote_debug_string(conditions));
    let (styles, children) = pseudo_styles_and_children(declarations);

    Some(RenderPseudoElement {
        kind: kind.into(),
        selector: selector.into(),
        conditions,
        styles,
        children,
        span: None,
    })
}

fn source_hint_pseudo_element_legacy(rest: &str) -> Option<RenderPseudoElement> {
    let (header, declarations) = split_preserved_css_block(rest)?;
    let (conditions, selector) = parse_preserved_style_header(header);
    let (selector, kind) = selector.rsplit_once("::")?;
    let (styles, children) = pseudo_styles_and_children(declarations);

    Some(RenderPseudoElement {
        kind: kind.into(),
        selector: selector.into(),
        conditions,
        styles,
        children,
        span: None,
    })
}

fn pseudo_styles_and_children(declarations: &str) -> (Vec<StyleDeclaration>, Vec<RenderNode>) {
    let mut styles = parse_css_declarations(declarations);
    let mut children = Vec::new();
    styles.retain(|style| {
        if style.property == StyleProperty::Content
            && let Some(content) = css_content_text(style.value.as_str())
        {
            children.push(text_node(content));
            return false;
        }
        true
    });
    (styles, children)
}

fn parse_pseudo_condition_metadata(value: &str) -> Vec<RenderStyleCondition> {
    value
        .split("; ")
        .filter_map(|part| {
            let (kind, value) = part.split_once('=')?;
            let value = CompactString::from(unquote_debug_string(value));
            match kind {
                "pseudo-class" => Some(RenderStyleCondition::PseudoClass(value)),
                "pseudo-element" => Some(RenderStyleCondition::PseudoElement(value)),
                "media" => Some(RenderStyleCondition::Media(value)),
                "supports" => Some(RenderStyleCondition::Supports(value)),
                "container" => Some(RenderStyleCondition::Container(value)),
                _ => None,
            }
        })
        .collect()
}

fn split_preserved_css_block(value: &str) -> Option<(&str, &str)> {
    let start = value.find('{')?;
    let end = value.rfind('}')?;
    if end <= start {
        return None;
    }
    Some((value[..start].trim(), value[start + 1..end].trim()))
}

fn parse_preserved_style_header(header: &str) -> (Vec<RenderStyleCondition>, &str) {
    let mut conditions = Vec::new();
    let mut rest = header.trim();
    loop {
        let Some((condition, next)) = take_preserved_style_condition(rest) else {
            break;
        };
        conditions.push(condition);
        rest = next.trim_start();
        if let Some(next) = rest.strip_prefix("and ") {
            rest = next.trim_start();
        }
    }

    (conditions, rest)
}

fn take_preserved_style_condition(value: &str) -> Option<(RenderStyleCondition, &str)> {
    if let Some(value) = value.strip_prefix("::") {
        let (name, rest) = take_css_identifier(value)?;
        return Some((RenderStyleCondition::PseudoElement(name.into()), rest));
    }
    if let Some(value) = value.strip_prefix(':') {
        let (name, rest) = take_css_identifier(value)?;
        return Some((RenderStyleCondition::PseudoClass(name.into()), rest));
    }
    for (prefix, build) in [
        (
            "@media ",
            RenderStyleCondition::Media as fn(CompactString) -> RenderStyleCondition,
        ),
        ("@supports ", RenderStyleCondition::Supports),
        ("@container ", RenderStyleCondition::Container),
    ] {
        if let Some(value) = value.strip_prefix(prefix)
            && let Some((condition, rest)) = value.split_once(" and ")
        {
            return Some((build(condition.trim().into()), rest));
        }
    }
    None
}

fn take_css_identifier(value: &str) -> Option<(&str, &str)> {
    let end = value
        .char_indices()
        .find_map(|(index, ch)| {
            (!matches!(ch, '-' | '_' | 'a'..='z' | 'A'..='Z' | '0'..='9')).then_some(index)
        })
        .unwrap_or(value.len());
    (end > 0).then(|| (&value[..end], &value[end..]))
}

fn parse_css_declarations(value: &str) -> Vec<StyleDeclaration> {
    parse_style_attribute(value, None).value
}

fn css_content_text(value: &str) -> Option<String> {
    let value = value.trim();
    let quote = value.chars().next()?;
    if !matches!(quote, '"' | '\'') || !value.ends_with(quote) {
        return None;
    }
    let inner = value.get(quote.len_utf8()..value.len() - quote.len_utf8())?;
    Some(
        inner
            .replace("\\\"", "\"")
            .replace("\\'", "'")
            .replace("\\\\", "\\"),
    )
}

fn source_hint_dynamic_style(line: &str) -> Option<(Option<CompactString>, CompactString)> {
    let line = line.trim_start_matches("//").trim();
    let (state, value) = line.strip_prefix("dynamic ")?.split_once(": ")?;
    let state = if state == "style" {
        None
    } else {
        Some(CompactString::from(state.strip_suffix(" style")?.trim()))
    };
    let value = value.split(" [").next().unwrap_or(value).trim();
    let value = unquote_debug_string(value);
    (!value.is_empty()).then(|| (state, CompactString::from(value)))
}

fn source_hint_action(line: &str) -> Option<ActionBinding> {
    let line = line.trim_start_matches("//").trim();
    let rest = line.strip_prefix("template action: ")?;
    let (event, value) = rest.split_once('=')?;
    let event = event.trim().to_ascii_lowercase();
    let start = value.find("{{")? + 2;
    let end = value[start..].find("}}")? + start;
    let handler = rust_binding_to_template_path(value[start..end].trim());
    Some(action_binding(event.as_str(), handler.into()))
}

fn source_hint_classes(line: &str) -> Option<Vec<CompactString>> {
    let line = line.trim_start_matches("//").trim();
    let classes = line.strip_prefix("html classes: ")?;
    Some(
        classes
            .split_whitespace()
            .filter(|class| !class.is_empty())
            .map(CompactString::from)
            .collect(),
    )
}

fn parse_unmapped_attribute(attribute: &str) -> Option<(String, String)> {
    if let Some((name, value)) = attribute.split_once('=') {
        return Some((name.trim().to_owned(), unquote_debug_string(value.trim())));
    }

    let name = attribute.trim();
    (!name.is_empty()).then(|| (name.to_owned(), String::new()))
}

fn unquote_debug_string(value: &str) -> String {
    let value = value.trim();
    let Some(value) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    else {
        return value.to_owned();
    };
    value.replace("\\\"", "\"").replace("\\\\", "\\")
}

fn statement_expression(statement: &Stmt) -> Option<&Expr> {
    match statement {
        Stmt::Expr(expression, _) => Some(expression),
        Stmt::Local(local) => local.init.as_ref().map(|init| &*init.expr),
        _ => None,
    }
}

fn find_method_call_argument<'a>(expression: &'a Expr, method_name: &str) -> Option<&'a Expr> {
    match final_expression(expression) {
        Expr::MethodCall(method) => {
            if method.method == method_name {
                return method.args.first();
            }
            find_method_call_argument(&method.receiver, method_name).or_else(|| {
                method
                    .args
                    .iter()
                    .find_map(|arg| find_method_call_argument(arg, method_name))
            })
        }
        Expr::Call(call) => call
            .args
            .iter()
            .find_map(|arg| find_method_call_argument(arg, method_name)),
        Expr::Closure(closure) => find_method_call_argument(&closure.body, method_name),
        Expr::Block(block) => block.block.stmts.iter().find_map(|statement| {
            statement_expression(statement)
                .and_then(|expr| find_method_call_argument(expr, method_name))
        }),
        Expr::If(if_expr) => find_method_call_argument_in_block(&if_expr.then_branch, method_name)
            .or_else(|| {
                if_expr
                    .else_branch
                    .as_ref()
                    .and_then(|(_, expr)| find_method_call_argument(expr, method_name))
            }),
        _ => None,
    }
}

fn find_method_call_argument_in_block<'a>(
    block: &'a syn::Block,
    method_name: &str,
) -> Option<&'a Expr> {
    block.stmts.iter().find_map(|statement| {
        statement_expression(statement)
            .and_then(|expr| find_method_call_argument(expr, method_name))
    })
}

fn local_value_reference(expression: &Expr) -> Option<String> {
    match final_expression(expression) {
        Expr::Path(path) => path.path.get_ident().map(ToString::to_string),
        Expr::Reference(reference) => local_value_reference(&reference.expr),
        Expr::MethodCall(method)
            if matches!(
                method.method.to_string().as_str(),
                "clone" | "into" | "to_owned" | "to_string"
            ) =>
        {
            local_value_reference(&method.receiver)
        }
        _ => None,
    }
}

fn text_input_metadata_from_initializer(expression: &Expr) -> Option<TextInputMetadata> {
    let expression = state_initializer_expression(expression);
    let (base, methods) = flatten_method_chain(expression);
    let Expr::Call(call) = final_expression(base) else {
        return None;
    };

    match call_path(call)?.as_str() {
        "HtmlswapGpuiTextInput::new" => {
            let mut metadata = TextInputMetadata {
                value: call.args.first().and_then(string_literal).map(Into::into),
                placeholder: call
                    .args
                    .iter()
                    .nth(1)
                    .and_then(string_literal)
                    .map(Into::into),
                ..TextInputMetadata::default()
            };
            apply_input_state_methods(&mut metadata, &methods);
            Some(metadata)
        }
        "gpui_component::input::InputState::new" => {
            let mut metadata = TextInputMetadata::default();
            apply_input_state_methods(&mut metadata, &methods);
            Some(metadata)
        }
        _ => None,
    }
}

fn state_initializer_expression(expression: &Expr) -> &Expr {
    match final_expression(expression) {
        Expr::MethodCall(method) if method.method == "new" => method
            .args
            .first()
            .and_then(|argument| match final_expression(argument) {
                Expr::Closure(closure) => Some(final_expression(&closure.body)),
                _ => None,
            })
            .unwrap_or(expression),
        other => other,
    }
}

fn apply_input_state_methods(metadata: &mut TextInputMetadata, methods: &[&ExprMethodCall]) {
    for method in methods {
        match method.method.to_string().as_str() {
            "placeholder" => {
                if let Some(value) = method.args.first().and_then(string_literal) {
                    metadata.placeholder = Some(value.into());
                }
            }
            "default_value" => {
                if let Some(value) = method.args.first().and_then(string_literal) {
                    metadata.value = Some(value.into());
                }
            }
            "multi_line" => {
                metadata.multiline = method.args.first().and_then(bool_literal).unwrap_or(true);
            }
            "rows" => {
                metadata.rows = method.args.first().and_then(u16_literal);
            }
            "masked" => {
                metadata.password = method.args.first().and_then(bool_literal).unwrap_or(true);
            }
            _ => {}
        }
    }
}

fn bool_literal(expression: &Expr) -> Option<bool> {
    let Expr::Lit(lit) = final_expression(expression) else {
        return None;
    };
    let Lit::Bool(value) = &lit.lit else {
        return None;
    };
    Some(value.value)
}

fn u16_literal(expression: &Expr) -> Option<u16> {
    let Expr::Lit(lit) = final_expression(expression) else {
        return None;
    };
    let Lit::Int(value) = &lit.lit else {
        return None;
    };
    value.base10_parse::<u16>().ok()
}

fn member_name(member: &Member) -> Option<String> {
    match member {
        Member::Named(name) => Some(name.to_string()),
        Member::Unnamed(_) => None,
    }
}

fn inferred_id_from_state_field(field: &str) -> Option<CompactString> {
    if looks_like_generated_state_field(field) {
        return None;
    }
    Some(field.to_kebab_case().into())
}

fn looks_like_generated_state_field(field: &str) -> bool {
    let Some((prefix, suffix)) = field.rsplit_once('_') else {
        return false;
    };
    matches!(
        prefix,
        "input" | "text_input" | "select" | "checkbox" | "radio" | "state"
    ) && suffix.chars().all(|character| character.is_ascii_digit())
}

fn has_attribute(element: &RenderElement, name: &str) -> bool {
    element
        .attributes
        .iter()
        .any(|attribute| attribute.name == name)
}

fn has_attribute_value(element: &RenderElement, name: &str, value: &str) -> bool {
    element
        .attributes
        .iter()
        .any(|attribute| attribute.name == name && attribute.value.eq_ignore_ascii_case(value))
}

fn has_click_like_action(element: &RenderElement) -> bool {
    element.actions.iter().any(|action| {
        matches!(
            action.event.as_str(),
            "click" | "doubleclick" | "mouseup" | "mousedown"
        )
    })
}

fn is_generated_component_input_fill_method(element: &RenderElement, method_name: &str) -> bool {
    element.role == UiRole::TextInput
        && element.source_tag == "input"
        && matches!(method_name, "w_full" | "h_full")
}

fn has_only_button_phrasing_children(element: &RenderElement) -> bool {
    element.children.iter().all(button_child_is_phrasing)
}

fn button_child_is_phrasing(node: &RenderNode) -> bool {
    match node {
        RenderNode::Text(_) | RenderNode::Raw(_) => true,
        RenderNode::Element(element) => {
            if has_click_like_action(element) {
                return false;
            }

            matches!(element.source_tag.as_str(), "span" | "img")
                && element.children.iter().all(button_child_is_phrasing)
        }
    }
}

fn control_flow_element(
    tag: &str,
    condition_attribute: &str,
    condition: impl Into<CompactString>,
    children: Vec<RenderNode>,
) -> RenderElement {
    let mut element = element(tag, UiRole::Container);
    element
        .attributes
        .push(attribute(condition_attribute, condition));
    element.children = children;
    element
}

fn format_template_binding(expression: &str) -> String {
    let expression = expression.trim();
    if expression.starts_with("{{") {
        expression.to_owned()
    } else {
        format!("{{{{ {expression} }}}}")
    }
}

fn normalize_control_flow_expression(expression: &str) -> String {
    let expression = expression
        .trim()
        .strip_prefix("{{")
        .and_then(|value| value.strip_suffix("}}"))
        .map(str::trim)
        .unwrap_or_else(|| expression.trim());
    expression
        .trim_start_matches("self.")
        .trim_start_matches("this.")
        .replace("()", "")
        .replace(['_', '.'], "")
        .replace(char::is_whitespace, "")
        .to_ascii_lowercase()
}

fn normalize_control_flow_binding(binding: &str) -> String {
    binding
        .trim()
        .trim_start_matches("r#")
        .replace('_', "")
        .to_ascii_lowercase()
}

fn parse_control_flow_placeholder(value: &str) -> Option<(CompactString, CompactString)> {
    let (name, value) = value.split_once('=')?;
    Some((
        name.trim().into(),
        CompactString::from(unquote_debug_string(value.trim())),
    ))
}

fn pattern_ident(pattern: &syn::Pat) -> Option<String> {
    match pattern {
        syn::Pat::Ident(ident) => Some(ident.ident.to_string()),
        _ => None,
    }
}

fn loop_iter_call_from_map_receiver(receiver: &Expr) -> Option<&ExprMethodCall> {
    let receiver = final_expression(receiver);
    let Expr::MethodCall(call) = receiver else {
        return None;
    };
    if call.method == "iter" || call.method == "into_iter" {
        return Some(call);
    }
    if call.method == "enumerate" {
        let Expr::MethodCall(iter_call) = final_expression(&call.receiver) else {
            return None;
        };
        if iter_call.method == "iter" || iter_call.method == "into_iter" {
            return Some(iter_call);
        }
    }
    None
}

fn loop_item_pattern_ident(pattern: &syn::Pat) -> Option<String> {
    if let Some(ident) = pattern_ident(pattern) {
        return Some(ident);
    }
    let syn::Pat::Tuple(tuple) = pattern else {
        return None;
    };
    let mut idents = tuple.elems.iter().filter_map(pattern_ident);
    match (idents.next(), idents.next(), idents.next()) {
        (Some(index), Some(item), None) if index.starts_with("htmlswap_index_") => Some(item),
        _ => None,
    }
}

fn is_path_ident(expression: &Expr, ident: &str) -> bool {
    let Expr::Path(path) = final_expression(expression) else {
        return false;
    };
    path.path.is_ident(ident)
}

fn expression_summary(expression: &Expr) -> String {
    match final_expression(expression) {
        Expr::Path(path) => path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
        Expr::Field(field) => compact_expr_name(&Expr::Field(field.clone())),
        Expr::MethodCall(method) => format!("method call `{}`", method.method),
        Expr::Call(call) => call_path(call).unwrap_or_else(|| "call".to_owned()),
        Expr::Macro(mac) => mac
            .mac
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
        Expr::If(_) => "if expression".to_owned(),
        Expr::Block(_) => "block expression".to_owned(),
        Expr::Array(_) => "array expression".to_owned(),
        other => format!("{:?}", std::mem::discriminant(other)),
    }
}

fn find_this_invocation(expression: &Expr) -> Option<CompactString> {
    match final_expression(expression) {
        Expr::MethodCall(method) => {
            if let Expr::Path(receiver) = final_expression(&method.receiver)
                && receiver.path.is_ident("this")
            {
                let name = method.method.to_string();
                if !matches!(name.as_str(), "update" | "read" | "write" | "notify") {
                    return Some(name.into());
                }
            }
            if is_recoverable_receiver_action(&method.method.to_string()) {
                let receiver = compact_expr_name(&method.receiver);
                if !matches!(
                    receiver.as_str(),
                    "self" | "this" | "value" | "input" | "cx" | "_cx"
                ) {
                    return Some(format!("{receiver}.{}", method.method).into());
                }
            }
            find_this_invocation(&method.receiver)
                .or_else(|| method.args.iter().find_map(find_this_invocation))
        }
        Expr::Call(call) => free_function_invocation(call)
            .or_else(|| call.args.iter().find_map(find_this_invocation)),
        Expr::Closure(closure) => find_this_invocation(&closure.body),
        Expr::Block(block) => block.block.stmts.iter().find_map(|stmt| match stmt {
            Stmt::Expr(expr, _) => find_this_invocation(expr),
            _ => None,
        }),
        Expr::If(if_expr) => find_this_invocation_in_block(&if_expr.then_branch).or_else(|| {
            if_expr
                .else_branch
                .as_ref()
                .and_then(|(_, expr)| find_this_invocation(expr))
        }),
        _ => None,
    }
}

fn is_recoverable_receiver_action(name: &str) -> bool {
    !matches!(
        name,
        "clone"
            | "into"
            | "read"
            | "write"
            | "update"
            | "notify"
            | "listener"
            | "value"
            | "to_string"
            | "to_owned"
            | "as_ref"
            | "borrow"
            | "borrow_mut"
    )
}

fn free_function_invocation(call: &ExprCall) -> Option<CompactString> {
    let Expr::Path(path) = final_expression(&call.func) else {
        return None;
    };
    let name = path.path.segments.last()?.ident.to_string();
    (!matches!(name.as_str(), "format" | "Some" | "Ok" | "Err")).then(|| name.into())
}

fn find_this_invocation_in_block(block: &syn::Block) -> Option<CompactString> {
    block.stmts.iter().find_map(|stmt| match stmt {
        Stmt::Expr(expr, _) => find_this_invocation(expr),
        _ => None,
    })
}

fn expression_has_double_click_guard(expression: &Expr) -> bool {
    match final_expression(expression) {
        Expr::Binary(binary) => {
            click_count_compared_to_two(&binary.left, &binary.right)
                || click_count_compared_to_two(&binary.right, &binary.left)
        }
        Expr::Closure(closure) => expression_has_double_click_guard(&closure.body),
        Expr::Block(block) => block
            .block
            .stmts
            .iter()
            .any(statement_has_double_click_guard),
        Expr::If(if_expr) => {
            expression_has_double_click_guard(&if_expr.cond)
                || if_expr
                    .then_branch
                    .stmts
                    .iter()
                    .any(statement_has_double_click_guard)
                || if_expr
                    .else_branch
                    .as_ref()
                    .is_some_and(|(_, expr)| expression_has_double_click_guard(expr))
        }
        Expr::MethodCall(method) => {
            expression_has_double_click_guard(&method.receiver)
                || method.args.iter().any(expression_has_double_click_guard)
        }
        Expr::Call(call) => call.args.iter().any(expression_has_double_click_guard),
        Expr::Paren(paren) => expression_has_double_click_guard(&paren.expr),
        Expr::Group(group) => expression_has_double_click_guard(&group.expr),
        _ => false,
    }
}

fn statement_has_double_click_guard(statement: &Stmt) -> bool {
    match statement {
        Stmt::Expr(expr, _) => expression_has_double_click_guard(expr),
        Stmt::Local(local) => local
            .init
            .as_ref()
            .is_some_and(|init| expression_has_double_click_guard(&init.expr)),
        _ => false,
    }
}

fn click_count_compared_to_two(left: &Expr, right: &Expr) -> bool {
    is_click_count_call(left) && is_integer_literal(right, 2)
}

fn is_click_count_call(expression: &Expr) -> bool {
    matches!(final_expression(expression), Expr::MethodCall(method) if method.method == "click_count")
}

fn is_integer_literal(expression: &Expr, expected: u64) -> bool {
    let Expr::Lit(lit) = final_expression(expression) else {
        return false;
    };
    let Lit::Int(value) = &lit.lit else {
        return false;
    };
    value
        .base10_parse::<u64>()
        .is_ok_and(|value| value == expected)
}

fn styles_from_method(name: &str, method: &ExprMethodCall) -> Option<Vec<StyleDeclaration>> {
    let first_arg = method.args.first();
    let declaration = match name {
        "block" => ("display", "block".to_owned()),
        "flex" => ("display", "flex".to_owned()),
        "grid" => ("display", "grid".to_owned()),
        "hidden" => ("display", "none".to_owned()),
        "flex_col" => ("flex-direction", "column".to_owned()),
        "flex_col_reverse" => ("flex-direction", "column-reverse".to_owned()),
        "flex_row" => ("flex-direction", "row".to_owned()),
        "flex_row_reverse" => ("flex-direction", "row-reverse".to_owned()),
        "flex_nowrap" => ("flex-wrap", "nowrap".to_owned()),
        "flex_wrap" => ("flex-wrap", "wrap".to_owned()),
        "flex_wrap_reverse" => ("flex-wrap", "wrap-reverse".to_owned()),
        "flex_none" => ("flex", "none".to_owned()),
        "flex_auto" => ("flex", "1 1 auto".to_owned()),
        "items_center" => ("align-items", "center".to_owned()),
        "items_start" => ("align-items", "flex-start".to_owned()),
        "items_end" => ("align-items", "flex-end".to_owned()),
        "items_baseline" => ("align-items", "baseline".to_owned()),
        "justify_center" => ("justify-content", "center".to_owned()),
        "justify_between" => ("justify-content", "space-between".to_owned()),
        "justify_around" => ("justify-content", "space-around".to_owned()),
        "justify_start" => ("justify-content", "flex-start".to_owned()),
        "justify_end" => ("justify-content", "flex-end".to_owned()),
        "content_normal" => ("align-content", "normal".to_owned()),
        "content_start" => ("align-content", "start".to_owned()),
        "content_end" => ("align-content", "end".to_owned()),
        "content_center" => ("align-content", "center".to_owned()),
        "content_between" => ("align-content", "space-between".to_owned()),
        "content_around" => ("align-content", "space-around".to_owned()),
        "content_evenly" => ("align-content", "space-evenly".to_owned()),
        "content_stretch" => ("align-content", "stretch".to_owned()),
        "absolute" => ("position", "absolute".to_owned()),
        "relative" => ("position", "relative".to_owned()),
        "overflow_hidden" => ("overflow", "hidden".to_owned()),
        "overflow_x_hidden" => ("overflow-x", "hidden".to_owned()),
        "overflow_y_hidden" => ("overflow-y", "hidden".to_owned()),
        "overflow_scroll" => ("overflow", "scroll".to_owned()),
        "overflow_x_scroll" => ("overflow-x", "scroll".to_owned()),
        "overflow_y_scroll" => ("overflow-y", "scroll".to_owned()),
        "w_full" => ("width", "100%".to_owned()),
        "h_full" => ("height", "100%".to_owned()),
        "size_full" => {
            return Some(vec![
                style_declaration("width", "100%"),
                style_declaration("height", "100%"),
            ]);
        }
        "text_right" => ("text-align", "right".to_owned()),
        "text_center" => ("text-align", "center".to_owned()),
        "text_left" => ("text-align", "left".to_owned()),
        "text_ellipsis" => ("text-overflow", "ellipsis".to_owned()),
        "whitespace_normal" => ("white-space", "normal".to_owned()),
        "whitespace_nowrap" => ("white-space", "nowrap".to_owned()),
        "underline" => ("text-decoration", "underline".to_owned()),
        "line_through" => ("text-decoration", "line-through".to_owned()),
        "text_decoration_none" => ("text-decoration", "none".to_owned()),
        "rounded_full" => ("border-radius", "9999px".to_owned()),
        "bg" => {
            let value = css_value(first_arg?)?;
            let property = if value.starts_with("linear-gradient(") {
                "background"
            } else {
                "background-color"
            };
            (property, value)
        }
        "text_color" => ("color", css_value(first_arg?)?),
        "text_size" => ("font-size", css_value(first_arg?)?),
        "line_height" => ("line-height", css_value(first_arg?)?),
        "font_family" => ("font-family", string_literal(first_arg?)?),
        "font_weight" => ("font-weight", font_weight_value(first_arg?)?),
        "italic" => ("font-style", "italic".to_owned()),
        "not_italic" => ("font-style", "normal".to_owned()),
        "gap" => ("gap", css_value(first_arg?)?),
        "w" => ("width", css_value(first_arg?)?),
        "h" => ("height", css_value(first_arg?)?),
        "min_w" => ("min-width", css_value(first_arg?)?),
        "min_h" => ("min-height", css_value(first_arg?)?),
        "max_w" => ("max-width", css_value(first_arg?)?),
        "max_h" => ("max-height", css_value(first_arg?)?),
        "top" => ("top", css_value(first_arg?)?),
        "right" => ("right", css_value(first_arg?)?),
        "bottom" => ("bottom", css_value(first_arg?)?),
        "left" => ("left", css_value(first_arg?)?),
        "inset" => ("inset", css_value(first_arg?)?),
        "flex_1" => ("flex", "1 1 0".to_owned()),
        "flex_initial" => ("flex", "0 1 auto".to_owned()),
        "flex_grow" => ("flex-grow", "1".to_owned()),
        "flex_shrink" => ("flex-shrink", "1".to_owned()),
        "flex_shrink_0" => ("flex-shrink", "0".to_owned()),
        "flex_basis" => ("flex-basis", css_value(first_arg?)?),
        "grid_cols" => (
            "grid-template-columns",
            format!("repeat({}, 1fr)", number_literal(first_arg?)?),
        ),
        "p" => ("padding", css_value(first_arg?)?),
        "px" => {
            let value = css_value(first_arg?)?;
            return Some(vec![
                style_declaration("padding-left", value.clone()),
                style_declaration("padding-right", value),
            ]);
        }
        "py" => {
            let value = css_value(first_arg?)?;
            return Some(vec![
                style_declaration("padding-top", value.clone()),
                style_declaration("padding-bottom", value),
            ]);
        }
        "pt" => ("padding-top", css_value(first_arg?)?),
        "pr" => ("padding-right", css_value(first_arg?)?),
        "pb" => ("padding-bottom", css_value(first_arg?)?),
        "pl" => ("padding-left", css_value(first_arg?)?),
        "m" => ("margin", css_value(first_arg?)?),
        "mx" => {
            let value = css_value(first_arg?)?;
            return Some(vec![
                style_declaration("margin-left", value.clone()),
                style_declaration("margin-right", value),
            ]);
        }
        "my" => {
            let value = css_value(first_arg?)?;
            return Some(vec![
                style_declaration("margin-top", value.clone()),
                style_declaration("margin-bottom", value),
            ]);
        }
        "mt" => ("margin-top", css_value(first_arg?)?),
        "mr" => ("margin-right", css_value(first_arg?)?),
        "mb" => ("margin-bottom", css_value(first_arg?)?),
        "ml" => ("margin-left", css_value(first_arg?)?),
        "border" => ("border-width", css_value(first_arg?)?),
        "border_t" => ("border-top-width", css_value(first_arg?)?),
        "border_r" => ("border-right-width", css_value(first_arg?)?),
        "border_b" => ("border-bottom-width", css_value(first_arg?)?),
        "border_l" => ("border-left-width", css_value(first_arg?)?),
        "border_dashed" => {
            return Some(vec![
                style_declaration("border-top-style", "dashed"),
                style_declaration("border-right-style", "dashed"),
                style_declaration("border-bottom-style", "dashed"),
                style_declaration("border-left-style", "dashed"),
            ]);
        }
        "border_color" => ("border-color", css_value(first_arg?)?),
        "rounded" => ("border-radius", css_value(first_arg?)?),
        "opacity" => ("opacity", css_value(first_arg?)?),
        "cursor_default" => ("cursor", "default".to_owned()),
        "cursor_pointer" => ("cursor", "pointer".to_owned()),
        "cursor_text" => ("cursor", "text".to_owned()),
        "cursor_move" => ("cursor", "move".to_owned()),
        "cursor_not_allowed" => ("cursor", "not-allowed".to_owned()),
        "cursor_context_menu" => ("cursor", "context-menu".to_owned()),
        "cursor_crosshair" => ("cursor", "crosshair".to_owned()),
        "cursor_vertical_text" => ("cursor", "vertical-text".to_owned()),
        "cursor_alias" => ("cursor", "alias".to_owned()),
        "cursor_copy" => ("cursor", "copy".to_owned()),
        "cursor_no_drop" => ("cursor", "no-drop".to_owned()),
        "cursor_grab" => ("cursor", "grab".to_owned()),
        "cursor_grabbing" => ("cursor", "grabbing".to_owned()),
        "cursor_ew_resize" => ("cursor", "ew-resize".to_owned()),
        "cursor_ns_resize" => ("cursor", "ns-resize".to_owned()),
        "cursor_nesw_resize" => ("cursor", "nesw-resize".to_owned()),
        "cursor_nwse_resize" => ("cursor", "nwse-resize".to_owned()),
        "cursor_col_resize" => ("cursor", "col-resize".to_owned()),
        "cursor_row_resize" => ("cursor", "row-resize".to_owned()),
        "cursor_n_resize" => ("cursor", "n-resize".to_owned()),
        "cursor_e_resize" => ("cursor", "e-resize".to_owned()),
        "cursor_s_resize" => ("cursor", "s-resize".to_owned()),
        "cursor_w_resize" => ("cursor", "w-resize".to_owned()),
        "shadow" => ("box-shadow", shadow_value(first_arg?)?),
        "shadow_none" => ("box-shadow", "none".to_owned()),
        "map" => ("aspect-ratio", aspect_ratio_from_map_method(method)?),
        _ => return None,
    };

    Some(vec![style_declaration(declaration.0, declaration.1)])
}

fn style_declaration(property: &str, value: impl Into<CompactString>) -> StyleDeclaration {
    StyleDeclaration::new(StyleProperty::from(property), value.into(), false, None)
}

fn style_variant_from_method_argument(method: &ExprMethodCall) -> Option<String> {
    let closure = method
        .args
        .first()
        .and_then(|argument| match final_expression(argument) {
            Expr::Closure(closure) => Some(closure),
            _ => None,
        })?;
    let (base, methods) = flatten_method_chain(&closure.body);
    if !is_path_ident(base, "this") {
        return None;
    }
    let styles = methods
        .iter()
        .filter_map(|method| styles_from_method(&method.method.to_string(), method))
        .flatten()
        .collect::<Vec<_>>();
    (!styles.is_empty()).then(|| format_style_declarations(&styles))
}

fn format_style_declarations(styles: &[StyleDeclaration]) -> String {
    styles
        .iter()
        .map(|style| {
            if style.important {
                format!("{}: {} !important", style.property, style.value.as_str())
            } else {
                format!("{}: {}", style.property, style.value.as_str())
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn css_value(expression: &Expr) -> Option<String> {
    match final_expression(expression) {
        Expr::Lit(lit) => match &lit.lit {
            Lit::Int(value) => Some(value.base10_digits().to_owned()),
            Lit::Float(value) => value
                .base10_parse::<f64>()
                .ok()
                .map(|value| trim_float(&value.to_string())),
            Lit::Str(value) => Some(value.value()),
            _ => None,
        },
        Expr::Path(path) => Some(
            path.path
                .segments
                .last()
                .map(|segment| segment.ident.to_string())
                .unwrap_or_else(|| "value".to_owned()),
        ),
        Expr::Call(call) => {
            let path = call_path(call)?;
            match path.as_str() {
                "gpui::px" | "px" => {
                    let value = call.args.first().and_then(number_literal)?;
                    Some(format!("{value}px"))
                }
                "gpui::relative" | "relative" => {
                    let value = call.args.first().and_then(number_literal)?;
                    Some(format!(
                        "{}%",
                        trim_float(&(value.parse::<f64>().ok()? * 100.0).to_string())
                    ))
                }
                "gpui::auto" | "auto" => Some("auto".to_owned()),
                "gpui::rgb" | "rgb" => call
                    .args
                    .first()
                    .and_then(hex_literal)
                    .map(|hex| format!("#{hex:06X}")),
                "gpui::rgba" | "rgba" => call
                    .args
                    .first()
                    .and_then(hex_literal)
                    .map(|hex| format!("#{hex:08X}")),
                "gpui::linear_gradient" | "linear_gradient" => linear_gradient_value(call),
                _ => None,
            }
        }
        _ => None,
    }
}

fn linear_gradient_value(call: &ExprCall) -> Option<String> {
    if call.args.len() < 3 {
        return None;
    }
    let angle = call.args.first().and_then(number_literal)?;
    let from = linear_color_stop_value(call.args.iter().nth(1)?)?;
    let to = linear_color_stop_value(call.args.iter().nth(2)?)?;
    let mut stops = vec![from, to];
    let angle = trim_float(&angle);
    if angle == "180" {
        Some(format!("linear-gradient({})", stops.join(", ")))
    } else {
        stops.insert(0, format!("{angle}deg"));
        Some(format!("linear-gradient({})", stops.join(", ")))
    }
}

fn linear_color_stop_value(expression: &Expr) -> Option<String> {
    let Expr::Call(call) = final_expression(expression) else {
        return css_value(expression);
    };
    match call_path(call)?.as_str() {
        "gpui::linear_color_stop" | "linear_color_stop" => call.args.first().and_then(css_value),
        _ => css_value(expression),
    }
}

fn shadow_value(expression: &Expr) -> Option<String> {
    let shadows = match final_expression(expression) {
        Expr::Array(array) => array.elems.iter().filter_map(box_shadow_value).collect(),
        Expr::Macro(macro_expr) if path_ends_with(&macro_expr.mac.path, "vec") => {
            let parser = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated;
            parser
                .parse2(macro_expr.mac.tokens.clone())
                .ok()?
                .iter()
                .filter_map(box_shadow_value)
                .collect()
        }
        expression => vec![box_shadow_value(expression)?],
    };
    (!shadows.is_empty()).then(|| shadows.join(", "))
}

fn box_shadow_value(expression: &Expr) -> Option<String> {
    let Expr::Struct(item) = final_expression(expression) else {
        return None;
    };

    let mut color = None;
    let mut offset = None;
    let mut blur_radius = None;
    let mut spread_radius = None;
    for field in &item.fields {
        let Member::Named(name) = &field.member else {
            continue;
        };
        match name.to_string().as_str() {
            "color" => color = css_value(&field.expr),
            "offset" => offset = point_value(&field.expr),
            "blur_radius" => blur_radius = css_value(&field.expr),
            "spread_radius" => spread_radius = css_value(&field.expr),
            _ => {}
        }
    }

    let (offset_x, offset_y) = offset?;
    let color = color.unwrap_or_else(|| "#000000FF".to_owned());
    let mut parts = vec![
        normalize_zero_length(offset_x),
        normalize_zero_length(offset_y),
    ];
    if let Some(blur_radius) = blur_radius {
        parts.push(normalize_zero_length(blur_radius));
    }
    if let Some(spread_radius) = spread_radius
        && !is_zero_css_length(&spread_radius)
    {
        parts.push(normalize_zero_length(spread_radius));
    }
    parts.push(color);
    Some(parts.join(" "))
}

fn point_value(expression: &Expr) -> Option<(String, String)> {
    let Expr::Call(call) = final_expression(expression) else {
        return None;
    };
    match call_path(call)?.as_str() {
        "gpui::point" | "point" => {
            let x = call.args.first().and_then(css_value)?;
            let y = call.args.iter().nth(1).and_then(css_value)?;
            Some((x, y))
        }
        _ => None,
    }
}

fn normalize_zero_length(value: String) -> String {
    if is_zero_css_length(&value) {
        "0".to_owned()
    } else {
        value
    }
}

fn is_zero_css_length(value: &str) -> bool {
    matches!(value.trim(), "0" | "0px" | "0%" | "0.0px")
}

fn aspect_ratio_from_map_method(method: &ExprMethodCall) -> Option<String> {
    let closure = method
        .args
        .first()
        .and_then(|argument| match final_expression(argument) {
            Expr::Closure(closure) => Some(closure),
            _ => None,
        })?;
    find_aspect_ratio_assignment(&closure.body)
}

fn find_aspect_ratio_assignment(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Paren(paren) => return find_aspect_ratio_assignment(&paren.expr),
        Expr::Group(group) => return find_aspect_ratio_assignment(&group.expr),
        Expr::Block(block) => {
            return block
                .block
                .stmts
                .iter()
                .find_map(|statement| match statement {
                    Stmt::Expr(expr, _) => find_aspect_ratio_assignment(expr),
                    Stmt::Local(local) => local
                        .init
                        .as_ref()
                        .and_then(|init| find_aspect_ratio_assignment(&init.expr)),
                    _ => None,
                });
        }
        _ => {}
    }

    match final_expression(expression) {
        Expr::Assign(assign) => {
            if compact_expr_name(&assign.left).ends_with(".style().aspect_ratio") {
                return some_number_argument(&assign.right);
            }
            None
        }
        _ => None,
    }
}

fn some_number_argument(expression: &Expr) -> Option<String> {
    let Expr::Call(call) = final_expression(expression) else {
        return None;
    };
    if call_path(call)?.as_str() != "Some" {
        return None;
    }
    call.args.first().and_then(number_literal)
}

fn number_literal(expression: &Expr) -> Option<String> {
    match final_expression(expression) {
        Expr::Lit(lit) => match &lit.lit {
            Lit::Float(value) => value
                .base10_parse::<f64>()
                .ok()
                .map(|value| trim_float(&value.to_string())),
            Lit::Int(value) => Some(value.base10_digits().to_owned()),
            _ => None,
        },
        Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => {
            number_literal(&unary.expr).map(|value| format!("-{value}"))
        }
        _ => None,
    }
}

fn hex_literal(expression: &Expr) -> Option<u32> {
    let Expr::Lit(lit) = final_expression(expression) else {
        return None;
    };
    let Lit::Int(value) = &lit.lit else {
        return None;
    };
    value.base10_parse::<u32>().ok()
}

fn font_weight_value(expression: &Expr) -> Option<String> {
    let Expr::Path(path) = final_expression(expression) else {
        return None;
    };
    path.path
        .segments
        .last()
        .map(|segment| match segment.ident.to_string().as_str() {
            "BOLD" => "700".to_owned(),
            "NORMAL" => "400".to_owned(),
            other => other.to_ascii_lowercase(),
        })
}

fn trim_float(value: &str) -> String {
    if !value.contains('.') {
        return value.to_owned();
    }
    let trimmed = value.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        "0".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn apply_component_size_method(element: &mut RenderElement, method: &ExprMethodCall) {
    if let Some(size) = method.args.first().and_then(component_size_attribute) {
        push_or_replace_attribute(element, "data-htmlswap-size", size);
    }
}

fn component_size_attribute(expression: &Expr) -> Option<&'static str> {
    let Expr::Path(path) = final_expression(expression) else {
        return None;
    };
    component_size_value(&path.path.segments.last()?.ident.to_string())
}

fn push_id_arg(element: &mut RenderElement, call: &ExprCall) {
    if let Some(id) = call.args.first().and_then(string_literal) {
        element.attributes.push(attribute("id", id));
    }
}

fn push_or_replace_attribute(
    element: &mut RenderElement,
    name: impl Into<CompactString>,
    value: impl Into<CompactString>,
) {
    let name = name.into();
    let value = value.into();
    if let Some(attribute) = element.attributes.iter_mut().find(|attr| attr.name == name) {
        attribute.value = value;
        return;
    }
    element.attributes.push(RenderAttribute {
        name,
        value,
        template: None,
        span: None,
    });
}

fn apply_source_hint(element: &mut RenderElement, hint: SourceHint) {
    if let Some(source_order) = hint.source_order {
        push_or_replace_attribute(
            element,
            INTERNAL_SOURCE_ORDER_ATTRIBUTE,
            source_order.to_string(),
        );
    }
    if let Some(tag) = hint.source_tag {
        element.source_tag = tag;
    }
    for class in hint.classes {
        if !element.classes.contains(&class) {
            element.classes.push(class);
        }
    }
    for (name, value) in hint.attributes {
        push_or_replace_attribute(element, name, value);
    }
    for style in hint.styles {
        push_or_replace_style(&mut element.styles, style);
    }
    element.style_variants.extend(hint.style_variants);
    element.pseudo_elements.extend(hint.pseudo_elements);
    for (state, value) in hint.dynamic_styles {
        element.dynamic_styles.push(RenderDynamicStyleBinding {
            state,
            expression: TemplateString::new(value, Vec::new(), None),
            span: None,
        });
    }
    for action in hint.actions {
        if !element.actions.iter().any(|existing| {
            existing.event == action.event && existing.expression == action.expression
        }) {
            element.actions.push(action);
        }
    }
    for html in hint.raw_child_html {
        element.children.push(RenderNode::Raw(RenderRaw {
            html: html.to_string(),
            span: None,
        }));
    }
    reconcile_preserved_grid_fallbacks(element);
    reconcile_preserved_transform_fallbacks(element);
    reconcile_preserved_position_calc_fallbacks(element);
    reconcile_preserved_inset_shadow_fallbacks(element);
    reconcile_preserved_dynamic_style_fallbacks(element);
}

fn collapsed_wrapped_text_input(wrapper: &RenderElement) -> Option<RenderElement> {
    let [RenderNode::Element(child)] = wrapper.children.as_slice() else {
        return None;
    };

    let mut input = if child.role == UiRole::TextInput || child.source_tag == "input" {
        (**child).clone()
    } else {
        collapsed_wrapped_text_input(child)?
    };
    merge_source_wrapper_into_text_input(wrapper, &mut input);
    Some(input)
}

fn merge_source_wrapper_into_text_input(wrapper: &RenderElement, input: &mut RenderElement) {
    input.source_tag = "input".into();
    input.role = UiRole::TextInput;

    for class in &wrapper.classes {
        if !input.classes.iter().any(|existing| existing == class) {
            input.classes.push(class.clone());
        }
    }

    for attribute in &wrapper.attributes {
        if attribute.name == "id" && is_generated_runtime_id(attribute.value.as_str()) {
            continue;
        }
        push_or_replace_attribute(input, attribute.name.clone(), attribute.value.clone());
    }

    for style in &wrapper.styles {
        push_or_replace_style(&mut input.styles, style.clone());
    }

    input
        .style_variants
        .extend(wrapper.style_variants.iter().cloned());
    input
        .dynamic_styles
        .extend(wrapper.dynamic_styles.iter().cloned());
    input
        .pseudo_elements
        .extend(wrapper.pseudo_elements.iter().cloned());

    for action in &wrapper.actions {
        if !input.actions.iter().any(|existing| {
            existing.event == action.event && existing.expression == action.expression
        }) {
            input.actions.push(action.clone());
        }
    }

    if input.state.is_none() {
        input.state = wrapper.state.clone();
    }
    if input.form_control.is_none() {
        input.form_control = wrapper.form_control.clone();
    }
    if input.accessibility.is_none() {
        input.accessibility = wrapper.accessibility.clone();
    }
    if input.control_flow.is_none() {
        input.control_flow = wrapper.control_flow.clone();
    }
    if input.source_intent.is_none() {
        input.source_intent = wrapper.source_intent.clone();
    }
    if input.semantics.is_none() {
        input.semantics = wrapper.semantics.clone();
    }
    if input.region.is_none() {
        input.region = wrapper.region.clone();
    }
}

fn is_generated_runtime_id(value: &str) -> bool {
    value.starts_with("htmlswap_interactive_") || value.starts_with("htmlswap_debug_")
}

fn collapse_source_material_symbol_wrapper(element: &mut RenderElement) {
    if !has_class(element, "ms") {
        return;
    }

    let [RenderNode::Element(child)] = element.children.as_slice() else {
        return;
    };
    if !has_class(child, "ms")
        || child.role != UiRole::Inline
        || !child.attributes.is_empty()
        || !child.styles.is_empty()
        || !child.style_variants.is_empty()
        || !child.dynamic_styles.is_empty()
        || !child.actions.is_empty()
        || child.state.is_some()
        || child.form_control.is_some()
        || child.accessibility.is_some()
        || child.control_flow.is_some()
        || child.source_intent.is_some()
        || child.semantics.is_some()
        || child.region.is_some()
    {
        return;
    }

    element.children = child.children.clone();
}

fn has_class(element: &RenderElement, class: &str) -> bool {
    element.classes.iter().any(|existing| existing == class)
}

fn mark_generated_dc_body_wrapper(nodes: &mut [RenderNode]) {
    let [RenderNode::Element(root)] = nodes else {
        return;
    };
    if !is_div_like_source(root)
        || root.role != UiRole::Container
        || root.children.len() != 1
        || !root.actions.is_empty()
        || root.control_flow.is_some()
        || root.state.is_some()
        || root.form_control.is_some()
    {
        return;
    }

    let RenderNode::Element(child) = &root.children[0] else {
        return;
    };
    if child.source_tag == "x-dc" {
        root.source_tag = "body".into();
    }
}

fn unwrap_transparent_loop_body(nodes: Vec<RenderNode>) -> Vec<RenderNode> {
    let [RenderNode::Element(wrapper)] = nodes.as_slice() else {
        return nodes;
    };
    if !is_transparent_generated_loop_body(wrapper) {
        return nodes;
    }
    wrapper.children.clone()
}

fn unwrap_transparent_generated_nodes(nodes: Vec<RenderNode>) -> Vec<RenderNode> {
    let [RenderNode::Element(wrapper)] = nodes.as_slice() else {
        return nodes;
    };
    if !is_transparent_generated_container(wrapper) {
        return nodes;
    }
    wrapper.children.clone()
}

fn is_transparent_generated_loop_body(element: &RenderElement) -> bool {
    (element.children.len() > 1 || has_single_control_flow_child(element))
        && is_transparent_generated_container(element)
}

fn is_transparent_generated_container(element: &RenderElement) -> bool {
    element.source_tag.is_empty()
        && element.role == UiRole::Container
        && element.attributes.is_empty()
        && element.classes.is_empty()
        && element
            .styles
            .iter()
            .all(is_ignorable_generated_wrapper_style)
        && element.style_variants.is_empty()
        && element.dynamic_styles.is_empty()
        && element.actions.is_empty()
        && element.state.is_none()
        && element.form_control.is_none()
        && element.accessibility.is_none()
        && element.control_flow.is_none()
        && element.source_intent.is_none()
        && element.semantics.is_none()
        && element.region.is_none()
}

fn has_single_control_flow_child(element: &RenderElement) -> bool {
    matches!(
        element.children.as_slice(),
        [RenderNode::Element(child)] if child.control_flow.is_some() || is_control_flow_source_tag(child.source_tag.as_str())
    )
}

fn is_control_flow_source_tag(tag: &str) -> bool {
    matches!(
        tag,
        "sc-for" | "sc-if" | "sc-else-if" | "sc-else" | "sc-switch" | "sc-case"
    )
}

fn is_ignorable_generated_wrapper_style(style: &StyleDeclaration) -> bool {
    matches!(
        style.property,
        StyleProperty::Margin | StyleProperty::Padding
    ) && matches!(style.value.as_str(), "0" | "0px" | "0.0px")
}

fn is_div_like_source(element: &RenderElement) -> bool {
    element.source_tag.is_empty() || element.source_tag == "div"
}

fn push_or_replace_style(styles: &mut Vec<StyleDeclaration>, style: StyleDeclaration) {
    styles.retain(|existing| {
        !style_properties_conflict(style.property.clone(), existing.property.clone())
    });
    if let Some(existing) = styles
        .iter_mut()
        .find(|existing| existing.property == style.property)
    {
        *existing = style;
    } else {
        styles.push(style);
    }
}

fn reconcile_preserved_grid_fallbacks(element: &mut RenderElement) {
    let has_grid_template = element
        .styles
        .iter()
        .any(|style| style.property == StyleProperty::GridTemplateColumns);
    if !has_grid_template {
        return;
    }

    let has_grid_display = element.styles.iter().any(|style| {
        style.property == StyleProperty::Display
            && style.value.as_str().eq_ignore_ascii_case("grid")
    });
    element.styles.retain(|style| {
        !matches!(
            style.property,
            StyleProperty::Display if style.value.as_str().eq_ignore_ascii_case("flex") && has_grid_display
        ) && !matches!(
            style.property,
            StyleProperty::FlexWrap if style.value.as_str().eq_ignore_ascii_case("wrap")
        )
    });
    if !has_grid_display {
        element.styles.push(style_declaration("display", "grid"));
    }
}

fn reconcile_preserved_dynamic_style_fallbacks(element: &mut RenderElement) {
    if element.dynamic_styles.is_empty() {
        return;
    }

    let mut base_properties = HashSet::new();
    let mut state_properties = BTreeMap::<CompactString, HashSet<StyleProperty>>::new();
    for dynamic_style in &element.dynamic_styles {
        let properties = dynamic_style_properties(dynamic_style.expression.raw.as_str());
        if properties.is_empty() {
            continue;
        }
        if let Some(state) = &dynamic_style.state {
            state_properties
                .entry(state.clone())
                .or_default()
                .extend(properties);
        } else {
            base_properties.extend(properties);
        }
    }

    if !base_properties.is_empty() {
        element
            .styles
            .retain(|style| !dynamic_properties_cover(&base_properties, &style.property));
    }

    if !state_properties.is_empty() {
        for variant in &mut element.style_variants {
            let Some(state) = variant.conditions.iter().find_map(|condition| {
                if let RenderStyleCondition::PseudoClass(state) = condition {
                    Some(state)
                } else {
                    None
                }
            }) else {
                continue;
            };
            let Some(properties) = state_properties.get(state) else {
                continue;
            };
            variant
                .declarations
                .retain(|style| !dynamic_properties_cover(properties, &style.property));
        }
        element
            .style_variants
            .retain(|variant| !variant.declarations.is_empty());
    }
}

fn dynamic_style_properties(value: &str) -> HashSet<StyleProperty> {
    value
        .split(';')
        .filter_map(|declaration| {
            let (property, _) = declaration.trim().split_once(':')?;
            Some(StyleProperty::from(property.trim()))
        })
        .collect()
}

fn dynamic_properties_cover(
    dynamic_properties: &HashSet<StyleProperty>,
    property: &StyleProperty,
) -> bool {
    dynamic_properties
        .iter()
        .any(|dynamic_property| dynamic_property_covers(dynamic_property, property))
}

fn dynamic_property_covers(dynamic_property: &StyleProperty, property: &StyleProperty) -> bool {
    if dynamic_property == property
        || style_properties_conflict(dynamic_property.clone(), property.clone())
    {
        return true;
    }

    match dynamic_property {
        StyleProperty::Padding => matches!(
            property,
            StyleProperty::PaddingTop
                | StyleProperty::PaddingRight
                | StyleProperty::PaddingBottom
                | StyleProperty::PaddingLeft
        ),
        StyleProperty::Margin => matches!(
            property,
            StyleProperty::MarginTop
                | StyleProperty::MarginRight
                | StyleProperty::MarginBottom
                | StyleProperty::MarginLeft
        ),
        StyleProperty::Inset => matches!(
            property,
            StyleProperty::Top | StyleProperty::Right | StyleProperty::Bottom | StyleProperty::Left
        ),
        StyleProperty::Border => matches!(
            property,
            StyleProperty::BorderTop
                | StyleProperty::BorderRight
                | StyleProperty::BorderBottom
                | StyleProperty::BorderLeft
                | StyleProperty::BorderWidth
                | StyleProperty::BorderStyle
                | StyleProperty::BorderColor
        ),
        StyleProperty::BorderWidth => matches!(
            property,
            StyleProperty::BorderTop
                | StyleProperty::BorderRight
                | StyleProperty::BorderBottom
                | StyleProperty::BorderLeft
        ),
        StyleProperty::Background => matches!(
            property,
            StyleProperty::BackgroundColor
                | StyleProperty::BackgroundImage
                | StyleProperty::BackgroundSize
                | StyleProperty::BackgroundClip
        ),
        StyleProperty::Flex => matches!(
            property,
            StyleProperty::Flex
                | StyleProperty::FlexGrow
                | StyleProperty::FlexShrink
                | StyleProperty::FlexBasis
        ),
        _ => false,
    }
}

fn reconcile_preserved_transform_fallbacks(element: &mut RenderElement) {
    let Some(transform) = element
        .styles
        .iter()
        .rev()
        .find(|style| style.property == StyleProperty::Transform)
        .map(|style| style.value.to_string())
    else {
        return;
    };
    let Some((x, y)) = transform_translate_offsets(element, &transform) else {
        return;
    };

    let horizontal =
        if has_style(element, StyleProperty::Right) && !has_style(element, StyleProperty::Left) {
            StyleProperty::MarginRight
        } else {
            StyleProperty::MarginLeft
        };
    let vertical =
        if has_style(element, StyleProperty::Bottom) && !has_style(element, StyleProperty::Top) {
            StyleProperty::MarginBottom
        } else {
            StyleProperty::MarginTop
        };

    element.styles.retain(|style| {
        !matches_transform_fallback_margin(style, &horizontal, x)
            && !matches_transform_fallback_margin(style, &vertical, y)
    });
}

fn matches_transform_fallback_margin(
    style: &StyleDeclaration,
    property: &StyleProperty,
    offset: Option<f32>,
) -> bool {
    style.property == *property
        && offset.is_some_and(|offset| {
            css_px_value(style.value.as_str()).is_some_and(|px| float_nearly_equal(px, offset))
        })
}

fn reconcile_preserved_position_calc_fallbacks(element: &mut RenderElement) {
    let fallbacks = [
        (StyleProperty::Left, StyleProperty::MarginLeft),
        (StyleProperty::Right, StyleProperty::MarginRight),
        (StyleProperty::Top, StyleProperty::MarginTop),
        (StyleProperty::Bottom, StyleProperty::MarginBottom),
    ]
    .into_iter()
    .filter_map(|(position, margin)| {
        let style = element
            .styles
            .iter()
            .rev()
            .find(|style| style.property == position)?;
        Some((margin, calc_position_margin_offset(style.value.as_str())?))
    })
    .collect::<Vec<_>>();

    if fallbacks.is_empty() {
        return;
    }

    element.styles.retain(|style| {
        !fallbacks.iter().any(|(property, offset)| {
            style.property == *property
                && css_px_value(style.value.as_str())
                    .is_some_and(|px| float_nearly_equal(px, *offset))
        })
    });
}

fn calc_position_margin_offset(value: &str) -> Option<f32> {
    let value = value.trim().to_ascii_lowercase();
    let inner = strip_css_function(&value, "calc")?;
    let inner = strip_wrapping_parentheses(inner.trim());
    let (left, operator, right) = split_top_level_binary_operator(inner, &['+', '-'])?;
    let left = strip_wrapping_parentheses(left.trim());
    let right = strip_wrapping_parentheses(right.trim());

    if css_number_suffix(left, "%").is_some()
        && let Some(px) = css_px_value(right)
    {
        return Some(if operator == '-' { -px } else { px });
    }

    if let Some(px) = css_px_value(left)
        && css_number_suffix(right, "%").is_some()
    {
        return Some(px);
    }

    None
}

fn split_top_level_binary_operator<'a>(
    value: &'a str,
    operators: &[char],
) -> Option<(&'a str, char, &'a str)> {
    let mut depth = 0usize;
    for (index, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ch if depth == 0 && operators.contains(&ch) && index > 0 => {
                let left = value[..index].trim();
                let right = value[index + ch.len_utf8()..].trim();
                if !left.is_empty() && !right.is_empty() {
                    return Some((left, ch, right));
                }
            }
            _ => {}
        }
    }
    None
}

fn reconcile_preserved_inset_shadow_fallbacks(element: &mut RenderElement) {
    let fallbacks = element
        .styles
        .iter()
        .filter(|style| style.property == StyleProperty::BoxShadow)
        .filter_map(|style| inset_shadow_border_fallback(style.value.as_str()))
        .collect::<Vec<_>>();

    if fallbacks.is_empty() {
        return;
    }

    element.styles.retain(|style| {
        !fallbacks.iter().any(|fallback| {
            (style.property.as_str() == fallback.width_property.as_str()
                && css_px_value(style.value.as_str())
                    .is_some_and(|px| float_nearly_equal(px, fallback.width)))
                || (style.property == StyleProperty::BorderColor
                    && css_colors_equivalent(style.value.as_str(), fallback.color.as_str()))
        })
    });
}

#[derive(Debug)]
struct InsetShadowBorderFallback {
    width_property: CompactString,
    width: f32,
    color: CompactString,
}

fn inset_shadow_border_fallback(value: &str) -> Option<InsetShadowBorderFallback> {
    let shadows = split_top_level_commas(value);
    let [shadow] = shadows.as_slice() else {
        return None;
    };

    let mut inset = false;
    let mut lengths = Vec::new();
    let mut color = None;
    for part in split_top_level_whitespace(shadow) {
        if part.eq_ignore_ascii_case("inset") {
            inset = true;
            continue;
        }
        if let Some(length) = css_px_value(part) {
            lengths.push(length);
            continue;
        }
        if color.is_none() && css_color_token(part).is_some() {
            color = css_color_token(part);
        }
    }

    if !inset || lengths.len() < 2 {
        return None;
    }

    let offset_x = lengths[0];
    let offset_y = lengths[1];
    let blur = lengths.get(2).copied().unwrap_or(0.0);
    let spread = lengths.get(3).copied().unwrap_or(0.0);
    if !float_nearly_equal(blur, 0.0) || !float_nearly_equal(spread, 0.0) {
        return None;
    }

    let (width_property, width) = if float_nearly_equal(offset_x, 0.0) && offset_y < 0.0 {
        ("border-bottom-width", -offset_y)
    } else if float_nearly_equal(offset_x, 0.0) && offset_y > 0.0 {
        ("border-top-width", offset_y)
    } else if float_nearly_equal(offset_y, 0.0) && offset_x < 0.0 {
        ("border-right-width", -offset_x)
    } else if float_nearly_equal(offset_y, 0.0) && offset_x > 0.0 {
        ("border-left-width", offset_x)
    } else {
        return None;
    };

    Some(InsetShadowBorderFallback {
        width_property: width_property.into(),
        width,
        color: color.unwrap_or_else(|| "#000000".into()),
    })
}

fn css_color_token(value: &str) -> Option<CompactString> {
    let value = value.trim();
    if value.starts_with('#') || value.starts_with("rgb(") || value.starts_with("rgba(") {
        return Some(value.to_ascii_lowercase().into());
    }
    None
}

fn css_colors_equivalent(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn transform_translate_offsets(
    element: &RenderElement,
    transform: &str,
) -> Option<(Option<f32>, Option<f32>)> {
    let transform = transform.trim().to_ascii_lowercase();
    let width = static_px_dimension(element, StyleProperty::Width);
    let height = static_px_dimension(element, StyleProperty::Height);

    if let Some(inner) = strip_css_function(&transform, "translatex") {
        return Some((Some(translate_component_px(inner, width)?), None));
    }
    if let Some(inner) = strip_css_function(&transform, "translatey") {
        return Some((None, Some(translate_component_px(inner, height)?)));
    }
    let inner = strip_css_function(&transform, "translate")?;
    let parts = split_translate_components(inner);
    let x = translate_component_px(parts.first().copied()?, width)?;
    let y = parts
        .get(1)
        .map_or(Some(0.0), |part| translate_component_px(part, height))?;
    Some((
        (!float_nearly_equal(x, 0.0)).then_some(x),
        (!float_nearly_equal(y, 0.0)).then_some(y),
    ))
}

fn translate_component_px(value: &str, basis_px: Option<f32>) -> Option<f32> {
    let value = strip_wrapping_parentheses(value.trim()).to_ascii_lowercase();
    if value == "0" {
        return Some(0.0);
    }
    if let Some(px) = numeric_suffix(&value, "px") {
        return Some(px);
    }
    if let Some(percent) = numeric_suffix(&value, "%") {
        return basis_px.map(|basis| basis * percent / 100.0);
    }
    None
}

fn split_translate_components(value: &str) -> Vec<&str> {
    let comma_parts = split_top_level_commas(value);
    if comma_parts.len() > 1 {
        return comma_parts;
    }
    split_top_level_whitespace(value)
}

fn static_px_dimension(element: &RenderElement, property: StyleProperty) -> Option<f32> {
    element
        .styles
        .iter()
        .rev()
        .find(|style| style.property == property)
        .and_then(|style| css_px_value(style.value.as_str()))
}

fn has_style(element: &RenderElement, property: StyleProperty) -> bool {
    element
        .styles
        .iter()
        .any(|style| style.property == property)
}

fn css_px_value(value: &str) -> Option<f32> {
    let value = value.trim().to_ascii_lowercase();
    if value == "0" {
        return Some(0.0);
    }
    numeric_suffix(&value, "px")
}

fn css_number_suffix(value: &str, suffix: &str) -> Option<f32> {
    let value = value.trim().to_ascii_lowercase();
    numeric_suffix(&value, suffix)
}

fn numeric_suffix(value: &str, suffix: &str) -> Option<f32> {
    value
        .strip_suffix(suffix)
        .map(str::trim)
        .and_then(|value| value.parse::<f32>().ok())
}

fn strip_css_function<'a>(value: &'a str, function: &str) -> Option<&'a str> {
    let rest = value.trim().strip_prefix(function)?;
    let rest = rest.trim_start();
    rest.strip_prefix('(')?.strip_suffix(')')
}

fn split_top_level_commas(value: &str) -> Vec<&str> {
    split_top_level(value, ',')
}

fn split_top_level_whitespace(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    for (index, ch) in value.char_indices() {
        match ch {
            '(' => {
                depth += 1;
                start.get_or_insert(index);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                start.get_or_insert(index);
            }
            ch if ch.is_whitespace() && depth == 0 => {
                if let Some(part_start) = start.take() {
                    parts.push(value[part_start..index].trim());
                }
            }
            _ => {
                start.get_or_insert(index);
            }
        }
    }
    if let Some(part_start) = start {
        parts.push(value[part_start..].trim());
    }
    parts.into_iter().filter(|part| !part.is_empty()).collect()
}

fn split_top_level(value: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    for (index, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ch if ch == separator && depth == 0 => {
                parts.push(value[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(value[start..].trim());
    parts.into_iter().filter(|part| !part.is_empty()).collect()
}

fn strip_wrapping_parentheses(mut value: &str) -> &str {
    while let Some(inner) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    {
        value = inner.trim();
    }
    value
}

fn float_nearly_equal(left: f32, right: f32) -> bool {
    (left - right).abs() < 0.01
}

fn reconcile_generated_flex_fallbacks(element: &mut RenderElement) {
    let has_flex_none = element.styles.iter().any(|style| {
        style.property == StyleProperty::Flex && style.value.as_str().eq_ignore_ascii_case("none")
    });
    if !has_flex_none {
        return;
    }

    element.styles.retain(|style| {
        !(style.property == StyleProperty::FlexShrink && style.value.as_str() == "0")
    });
}

fn style_properties_conflict(incoming: StyleProperty, existing: StyleProperty) -> bool {
    match incoming {
        StyleProperty::Background => matches!(
            existing,
            StyleProperty::Background
                | StyleProperty::BackgroundColor
                | StyleProperty::BackgroundImage
        ),
        StyleProperty::BackgroundImage => {
            matches!(
                existing,
                StyleProperty::Background | StyleProperty::BackgroundImage
            )
        }
        StyleProperty::BackgroundColor => {
            matches!(
                existing,
                StyleProperty::Background | StyleProperty::BackgroundColor
            )
        }
        _ => false,
    }
}

fn element(tag: &str, role: UiRole) -> RenderElement {
    RenderElement {
        role,
        source_tag: tag.into(),
        attributes: Vec::new(),
        classes: Vec::new(),
        source_inline_styles: Vec::new(),
        styles: Vec::new(),
        stylesheet_rules: Vec::new(),
        stylesheet_declarations: Vec::new(),
        style_variants: Vec::new(),
        dynamic_styles: Vec::new(),
        pseudo_elements: Vec::new(),
        actions: Vec::new(),
        state: None,
        form_control: None,
        accessibility: None,
        control_flow: None,
        source_intent: None,
        semantics: None,
        region: None,
        children: Vec::new(),
        span: None,
    }
}

fn generated_container_element() -> RenderElement {
    let mut element = element("div", UiRole::Container);
    element.source_tag.clear();
    element
}

fn attribute(name: impl Into<CompactString>, value: impl Into<CompactString>) -> RenderAttribute {
    RenderAttribute {
        name: name.into(),
        value: value.into(),
        template: None,
        span: None,
    }
}

fn text_node(value: impl Into<String>) -> RenderNode {
    RenderNode::Text(RenderText {
        value: value.into(),
        template: None,
        span: None,
    })
}

fn action_binding(event: &str, action: CompactString) -> ActionBinding {
    let action: CompactString = rust_binding_to_template_path(action.as_str()).into();
    ActionBinding {
        event: event.into(),
        expression: action.to_string(),
        template: None,
        action: Some(action.clone()),
        resolved: true,
        handler: RenderActionHandler {
            invocations: vec![RenderActionInvocation {
                action,
                arguments: vec![RenderActionArgument::Event],
                resolved: true,
                span: None,
                action_span: None,
            }],
            effects: Vec::new(),
        },
        payload: ActionPayload::None,
        span: None,
        action_span: None,
    }
}

trait MemberName {
    fn to_token_stream_string(&self) -> String;
}

impl MemberName for syn::Member {
    fn to_token_stream_string(&self) -> String {
        match self {
            syn::Member::Named(name) => name.to_string(),
            syn::Member::Unnamed(index) => index.index.to_string(),
        }
    }
}
