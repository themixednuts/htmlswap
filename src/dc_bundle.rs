use std::collections::BTreeMap;

use compact_str::CompactString;
use heck::ToKebabCase;
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BinaryOperator, BindingPattern as OxcBindingPattern, ClassElement, Expression, FunctionBody,
    LogicalOperator, ObjectProperty, ObjectPropertyKind, PropertyKey, Statement, UnaryOperator,
};
use oxc_parser::{ParseOptions, Parser};
use oxc_span::SourceType;

use crate::bundle::BundlePlan;
use crate::compiler::CompiledFragment;
use crate::expr::{BindingPattern, Expr, TemplateSegment, TemplateString};
use crate::plan::{
    ActionBinding, ComponentId, RenderActionHandler, RenderActionHandlerEffect,
    RenderActionInvocation, RenderAnnotation, RenderAttribute, RenderChoiceOption,
    RenderControlFlow, RenderDynamicStyleBinding, RenderElement, RenderFormControl, RenderNode,
    RenderPlan, RenderPseudoElement, RenderScriptReference, RenderSemanticExtra, RenderSourceLogic,
    RenderSourceProp, RenderStateBinding, RenderStateKind, RenderStatePlan, RenderStyleCondition,
    RenderStyleVariant, RenderThemeScope, RenderThemeToken, RenderThemeTokenValue, UiRole,
};
use crate::source::{SourceId, SourceMap, Span};
use crate::style::{StyleDeclaration, StyleProperty, StyleToken, StyleValue};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DcComponentFragment {
    pub component: ComponentId,
    pub fragment: CompiledFragment,
}

impl DcComponentFragment {
    #[must_use]
    pub fn new(component: impl Into<ComponentId>, fragment: CompiledFragment) -> Self {
        Self {
            component: component.into(),
            fragment,
        }
    }
}

/// Inline sibling DC component templates into a root fragment.
///
/// This preserves the original per-file source locations by merging source maps
/// and remapping spans before component nodes are inserted.
#[must_use]
pub fn inline_dc_component_imports(
    root: CompiledFragment,
    components: impl IntoIterator<Item = DcComponentFragment>,
) -> CompiledFragment {
    let mut merger = SourceMerger::new(root.sources);
    let mut root_plan = root.plan;
    let mut component_plans = BTreeMap::new();

    for component in components {
        let mut fragment = component.fragment;
        let remap = merger.import_sources(&fragment.sources);
        remap_plan(&mut fragment.plan, &remap);
        let bindings = component_bindings_from_plan(&fragment.plan);
        component_plans.insert(
            CompactString::from(component.component.canonical_str()),
            DcComponentDefinition {
                plan: fragment.plan,
                bindings,
            },
        );
    }

    let component_metadata = component_plans
        .values()
        .map(|definition| definition.plan.clone())
        .collect::<Vec<_>>();
    let mut expander = DcImportExpander {
        components: &component_plans,
    };
    root_plan.nodes = expander.expand_nodes(std::mem::take(&mut root_plan.nodes), 0);
    for plan in &component_metadata {
        root_plan.annotations.extend(plan.annotations.clone());
        root_plan.head.extend(plan.head.clone());
        root_plan.scripts.extend(plan.scripts.clone());
        root_plan.source_logic.extend(plan.source_logic.clone());
        root_plan.theme.tokens.extend(plan.theme.tokens.clone());
    }
    root_plan.state = RenderStatePlan::from_nodes(&root_plan.nodes);
    let sources = merger.finish();
    let bundle = BundlePlan::from_render_plan(&root_plan, &sources);

    CompiledFragment {
        plan: root_plan,
        sources,
        bundle,
    }
}

pub(crate) fn apply_root_dc_style_bindings(plan: &mut RenderPlan) {
    let bindings = component_bindings_from_plan(plan);
    if bindings.styles.is_empty() {
        return;
    }

    rewrite_nodes_for_root_style_bindings(&mut plan.nodes, &bindings);
    plan.state = RenderStatePlan::from_nodes(&plan.nodes);
}

struct DcImportExpander<'a> {
    components: &'a BTreeMap<CompactString, DcComponentDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DcComponentDefinition {
    plan: RenderPlan,
    bindings: DcComponentBindings,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DcComponentBindings {
    values: BTreeMap<CompactString, TemplateString>,
    styles: BTreeMap<CompactString, Vec<DcComponentStyleBinding>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DcComponentStyleBinding {
    property: CompactString,
    value: TemplateString,
    span: Option<Span>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DcImportBindings {
    values: BTreeMap<CompactString, TemplateString>,
    styles: BTreeMap<CompactString, Vec<DcComponentStyleBinding>>,
}

impl DcImportBindings {
    fn is_empty(&self) -> bool {
        self.values.is_empty() && self.styles.is_empty()
    }
}

impl DcImportExpander<'_> {
    fn expand_nodes(&mut self, nodes: Vec<RenderNode>, depth: usize) -> Vec<RenderNode> {
        nodes
            .into_iter()
            .flat_map(|node| self.expand_node(node, depth))
            .collect()
    }

    fn expand_node(&mut self, node: RenderNode, depth: usize) -> Vec<RenderNode> {
        match node {
            RenderNode::Element(element) => self.expand_element(*element, depth),
            node => vec![node],
        }
    }

    fn expand_element(&mut self, mut element: RenderElement, depth: usize) -> Vec<RenderNode> {
        element.children = self.expand_nodes(element.children, depth);
        for pseudo in &mut element.pseudo_elements {
            pseudo.children = self.expand_nodes(std::mem::take(&mut pseudo.children), depth);
        }

        if element.source_tag != "dc-import" {
            return vec![RenderNode::Element(Box::new(element))];
        }

        let Some(component) = element
            .source_intent
            .as_ref()
            .and_then(|intent| intent.component.as_ref())
        else {
            return vec![RenderNode::Element(Box::new(element))];
        };

        let Some(definition) = self.components.get(component.canonical_str()) else {
            return vec![RenderNode::Element(Box::new(element))];
        };

        if depth > self.components.len().saturating_add(1) {
            return vec![RenderNode::Element(Box::new(element))];
        }

        let mut imported = self.expand_nodes(definition.plan.nodes.clone(), depth + 1);
        let import_bindings = instantiate_component_bindings(&element, definition);
        if !import_bindings.is_empty() {
            rewrite_nodes_for_component_import(&mut imported, &import_bindings);
        }
        if import_needs_host_wrapper(&element) {
            element.source_tag = "div".into();
            element.role = UiRole::Container;
            element.children = imported;
            vec![RenderNode::Element(Box::new(element))]
        } else {
            imported
        }
    }
}

fn import_needs_host_wrapper(element: &RenderElement) -> bool {
    !element.styles.is_empty()
        || !element.style_variants.is_empty()
        || !element.dynamic_styles.is_empty()
        || !element.actions.is_empty()
        || element.state.is_some()
        || element.form_control.is_some()
        || element.accessibility.is_some()
        || element.control_flow.is_some()
        || element.region.is_some()
}

fn component_bindings_from_plan(plan: &RenderPlan) -> DcComponentBindings {
    let mut bindings = DcComponentBindings::default();
    for logic in &plan.source_logic {
        if logic.dialect != "dc" {
            continue;
        }
        bindings.extend(component_bindings_from_logic(logic.body.as_str()));
    }
    bindings
}

impl DcComponentBindings {
    fn extend(&mut self, bindings: Self) {
        self.values.extend(bindings.values);
        self.styles.extend(bindings.styles);
    }
}

fn component_bindings_from_logic(source: &str) -> DcComponentBindings {
    let allocator = Allocator::default();
    let parser = Parser::new(&allocator, source, SourceType::ts()).with_options(ParseOptions {
        parse_regular_expression: true,
        ..ParseOptions::default()
    });
    let parsed = parser.parse();
    if parsed.panicked {
        return DcComponentBindings::default();
    }

    for statement in &parsed.program.body {
        let Statement::ClassDeclaration(class) = statement else {
            continue;
        };
        for element in &class.body.body {
            let ClassElement::MethodDefinition(method) = element else {
                continue;
            };
            if property_key_name(&method.key) == Some("renderVals")
                && let Some(body) = &method.value.body
            {
                return render_value_bindings_from_body(body);
            }
        }
    }

    DcComponentBindings::default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DcJsLocal {
    PropsRoot,
    PropPath(Vec<CompactString>),
    Value(TemplateString),
}

fn render_value_bindings_from_body(body: &FunctionBody<'_>) -> DcComponentBindings {
    let mut locals = BTreeMap::new();
    let mut bindings = DcComponentBindings::default();
    for statement in &body.statements {
        match statement {
            Statement::VariableDeclaration(declaration) => {
                for declarator in &declaration.declarations {
                    let Some(name) = oxc_binding_identifier_name(&declarator.id) else {
                        continue;
                    };
                    let Some(init) = &declarator.init else {
                        continue;
                    };
                    if let Some(alias) = dc_js_local_alias(init, &locals) {
                        locals.insert(CompactString::from(name), alias);
                    } else if let Some(value) = dc_js_expression_template(init, &locals) {
                        locals.insert(CompactString::from(name), DcJsLocal::Value(value));
                    }
                }
            }
            Statement::ReturnStatement(statement) => {
                let Some(Expression::ObjectExpression(object)) = &statement.argument else {
                    continue;
                };
                for property in object.properties.iter().filter_map(object_property) {
                    if property.computed || property.method {
                        continue;
                    }
                    let Some(name) = property_key_name(&property.key) else {
                        continue;
                    };
                    if let Expression::ObjectExpression(object) = &property.value
                        && looks_like_dc_style_object_name(name)
                    {
                        let styles = dc_style_bindings_from_object(object, &locals);
                        if !styles.is_empty() {
                            bindings.styles.insert(CompactString::from(name), styles);
                        }
                        continue;
                    }
                    let Some(value) = dc_js_expression_template(&property.value, &locals) else {
                        continue;
                    };
                    bindings.values.insert(CompactString::from(name), value);
                }
            }
            _ => {}
        }
    }
    bindings
}

fn dc_style_bindings_from_object(
    object: &oxc_ast::ast::ObjectExpression<'_>,
    locals: &BTreeMap<CompactString, DcJsLocal>,
) -> Vec<DcComponentStyleBinding> {
    object
        .properties
        .iter()
        .filter_map(object_property)
        .filter_map(|property| {
            if property.computed || property.method {
                return None;
            }
            let name = property_key_name(&property.key)?;
            let value = dc_js_style_expression_template(&property.value, locals)?;
            Some(DcComponentStyleBinding {
                property: CompactString::from(css_property_name(name)),
                value,
                span: None,
            })
        })
        .collect()
}

fn looks_like_dc_style_object_name(name: &str) -> bool {
    name == "style"
        || name.ends_with("Style")
        || name.ends_with("Styles")
        || name.ends_with("style")
        || name.ends_with("styles")
}

fn css_property_name(name: &str) -> String {
    if let Some(rest) = name.strip_prefix("Webkit") {
        return format!("-webkit-{}", rest.to_kebab_case());
    }
    if let Some(rest) = name.strip_prefix("Moz") {
        return format!("-moz-{}", rest.to_kebab_case());
    }
    if name.starts_with("ms")
        && name
            .chars()
            .nth(2)
            .is_some_and(|ch| ch.is_ascii_uppercase())
    {
        return format!("-ms-{}", name[2..].to_kebab_case());
    }
    name.to_kebab_case()
}

fn dc_js_local_alias(
    expression: &Expression<'_>,
    locals: &BTreeMap<CompactString, DcJsLocal>,
) -> Option<DcJsLocal> {
    let expression = strip_dc_js_fallback(expression);
    let path = dc_js_member_path(expression)?;
    if path.as_slice() == ["this", "props"] || path.as_slice() == ["this", "state"] {
        return Some(DcJsLocal::PropsRoot);
    }
    if path.len() >= 3 && path[0] == "this" && (path[1] == "props" || path[1] == "state") {
        return Some(DcJsLocal::PropPath(path[2..].to_vec()));
    }
    if path.len() == 1 {
        return locals.get(&path[0]).and_then(|local| match local {
            DcJsLocal::PropsRoot => Some(DcJsLocal::PropsRoot),
            DcJsLocal::PropPath(path) => Some(DcJsLocal::PropPath(path.clone())),
            DcJsLocal::Value(_) => None,
        });
    }
    None
}

fn strip_dc_js_fallback<'a>(expression: &'a Expression<'a>) -> &'a Expression<'a> {
    match expression {
        Expression::LogicalExpression(logical)
            if matches!(
                logical.operator,
                LogicalOperator::Or | LogicalOperator::Coalesce
            ) =>
        {
            strip_dc_js_fallback(&logical.left)
        }
        Expression::ParenthesizedExpression(expression) => {
            strip_dc_js_fallback(&expression.expression)
        }
        _ => expression,
    }
}

fn dc_js_expression_template(
    expression: &Expression<'_>,
    locals: &BTreeMap<CompactString, DcJsLocal>,
) -> Option<TemplateString> {
    match expression {
        Expression::Identifier(identifier) => match locals.get(identifier.name.as_str())? {
            DcJsLocal::Value(value) => Some(value.clone()),
            DcJsLocal::PropPath(path) => Some(template_from_expr(Expr::path(path.clone()))),
            DcJsLocal::PropsRoot => None,
        },
        Expression::StaticMemberExpression(_)
        | Expression::ComputedMemberExpression(_)
        | Expression::PrivateFieldExpression(_) => {
            let path = dc_js_member_path(expression)?;
            dc_js_path_template(&path, locals)
        }
        Expression::LogicalExpression(logical)
            if matches!(
                logical.operator,
                LogicalOperator::Or | LogicalOperator::Coalesce
            ) =>
        {
            dc_js_expression_template(&logical.left, locals)
                .or_else(|| dc_js_expression_template(&logical.right, locals))
        }
        Expression::ConditionalExpression(expression) => {
            dc_js_expression_template(&expression.consequent, locals)
                .or_else(|| dc_js_expression_template(&expression.alternate, locals))
        }
        Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::LogicalNot => {
            match &unary.argument {
                Expression::UnaryExpression(inner)
                    if inner.operator == UnaryOperator::LogicalNot =>
                {
                    dc_js_expression_template(&inner.argument, locals)
                }
                _ => None,
            }
        }
        Expression::BinaryExpression(binary) if binary.operator == BinaryOperator::Addition => {
            let left = dc_js_expression_template(&binary.left, locals)?;
            let right = dc_js_expression_template(&binary.right, locals)?;
            Some(concat_templates(&left, &right))
        }
        Expression::ParenthesizedExpression(expression) => {
            dc_js_expression_template(&expression.expression, locals)
        }
        Expression::StringLiteral(literal) => {
            Some(TemplateString::new(literal.value.as_str(), vec![], None))
        }
        Expression::NumericLiteral(literal) => Some(TemplateString::new(
            literal
                .raw
                .as_ref()
                .map_or_else(|| format_dc_number(literal.value), ToString::to_string),
            vec![],
            None,
        )),
        Expression::BooleanLiteral(literal) => {
            Some(TemplateString::new(literal.value.to_string(), vec![], None))
        }
        Expression::NullLiteral(_) => Some(TemplateString::new("null", vec![], None)),
        _ => None,
    }
}

fn dc_js_style_expression_template(
    expression: &Expression<'_>,
    locals: &BTreeMap<CompactString, DcJsLocal>,
) -> Option<TemplateString> {
    match expression {
        Expression::ConditionalExpression(_) => None,
        Expression::LogicalExpression(logical)
            if matches!(
                logical.operator,
                LogicalOperator::Or | LogicalOperator::Coalesce
            ) =>
        {
            dc_js_style_expression_template(&logical.left, locals)
                .or_else(|| dc_js_style_expression_template(&logical.right, locals))
        }
        Expression::ParenthesizedExpression(expression) => {
            dc_js_style_expression_template(&expression.expression, locals)
        }
        _ => dc_js_expression_template(expression, locals),
    }
}

fn dc_js_path_template(
    path: &[CompactString],
    locals: &BTreeMap<CompactString, DcJsLocal>,
) -> Option<TemplateString> {
    if path.len() >= 3 && path[0] == "this" && (path[1] == "props" || path[1] == "state") {
        return Some(template_from_expr(Expr::path(path[2..].to_vec())));
    }

    let (first, rest) = path.split_first()?;
    match locals.get(first)? {
        DcJsLocal::Value(value) if rest.is_empty() => Some(value.clone()),
        DcJsLocal::PropPath(base) => {
            let mut path = base.clone();
            path.extend(rest.iter().cloned());
            Some(template_from_expr(Expr::path(path)))
        }
        DcJsLocal::PropsRoot => Some(template_from_expr(Expr::path(rest.to_vec()))),
        DcJsLocal::Value(_) => None,
    }
}

fn dc_js_member_path(expression: &Expression<'_>) -> Option<Vec<CompactString>> {
    match expression {
        Expression::Identifier(identifier) => {
            Some(vec![CompactString::from(identifier.name.as_str())])
        }
        Expression::ThisExpression(_) => Some(vec![CompactString::from("this")]),
        Expression::StaticMemberExpression(member) => {
            let mut path = dc_js_member_path(&member.object)?;
            path.push(CompactString::from(member.property.name.as_str()));
            Some(path)
        }
        Expression::ParenthesizedExpression(expression) => {
            dc_js_member_path(&expression.expression)
        }
        _ => None,
    }
}

fn template_from_expr(expr: Expr) -> TemplateString {
    let raw = format!("{{{{ {expr} }}}}");
    TemplateString::new(raw, vec![TemplateSegment::Expression(expr)], None)
}

fn concat_templates(left: &TemplateString, right: &TemplateString) -> TemplateString {
    let mut segments = Vec::new();
    if left.segments.is_empty() {
        if !left.raw.is_empty() {
            segments.push(TemplateSegment::Literal(left.raw.clone()));
        }
    } else {
        segments.extend(left.segments.clone());
    }
    if right.segments.is_empty() {
        if !right.raw.is_empty() {
            segments.push(TemplateSegment::Literal(right.raw.clone()));
        }
    } else {
        segments.extend(right.segments.clone());
    }
    TemplateString::new(template_raw_from_segments(&segments), segments, None)
}

fn format_dc_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

fn instantiate_component_bindings(
    import: &RenderElement,
    definition: &DcComponentDefinition,
) -> DcImportBindings {
    let Some(intent) = import.source_intent.as_deref() else {
        return DcImportBindings::default();
    };

    let prop_bindings = intent
        .props
        .iter()
        .map(|prop| {
            (
                prop.name.clone(),
                prop.template
                    .clone()
                    .unwrap_or_else(|| TemplateString::new(prop.value.clone(), vec![], prop.span)),
            )
        })
        .collect::<BTreeMap<_, _>>();

    let mut bindings = DcImportBindings::default();
    bindings.values.extend(prop_bindings.clone());
    for (name, value) in &definition.bindings.values {
        bindings
            .values
            .insert(name.clone(), rewrite_template(value, &prop_bindings));
    }
    for (name, declarations) in &definition.bindings.styles {
        let declarations = declarations
            .iter()
            .map(|declaration| DcComponentStyleBinding {
                property: declaration.property.clone(),
                value: rewrite_template(&declaration.value, &prop_bindings),
                span: declaration.span,
            })
            .collect::<Vec<_>>();
        bindings.styles.insert(name.clone(), declarations);
    }
    bindings
}

fn rewrite_nodes_for_component_import(nodes: &mut [RenderNode], bindings: &DcImportBindings) {
    for node in nodes {
        rewrite_node_for_component_import(node, bindings);
    }
}

fn rewrite_nodes_for_root_style_bindings(nodes: &mut [RenderNode], bindings: &DcComponentBindings) {
    for node in nodes {
        rewrite_node_for_root_style_bindings(node, bindings);
    }
}

fn rewrite_node_for_root_style_bindings(node: &mut RenderNode, bindings: &DcComponentBindings) {
    match node {
        RenderNode::Element(element) => rewrite_element_for_root_style_bindings(element, bindings),
        RenderNode::Text(_) | RenderNode::Raw(_) => {}
    }
}

fn rewrite_element_for_root_style_bindings(
    element: &mut RenderElement,
    bindings: &DcComponentBindings,
) {
    let mut dynamic_styles = Vec::with_capacity(element.dynamic_styles.len());
    for style in std::mem::take(&mut element.dynamic_styles) {
        if let Some(style_name) = style_binding_name(&style.expression)
            && let Some(declarations) = bindings.styles.get(style_name)
        {
            apply_component_style_bindings(
                element,
                &mut dynamic_styles,
                style.state.clone(),
                declarations,
            );
            continue;
        }
        dynamic_styles.push(style);
    }
    element.dynamic_styles = dynamic_styles;

    for child in &mut element.children {
        rewrite_node_for_root_style_bindings(child, bindings);
    }
    for pseudo in &mut element.pseudo_elements {
        rewrite_nodes_for_root_style_bindings(&mut pseudo.children, bindings);
    }
}

fn rewrite_node_for_component_import(node: &mut RenderNode, bindings: &DcImportBindings) {
    match node {
        RenderNode::Element(element) => rewrite_element_for_component_import(element, bindings),
        RenderNode::Text(text) => {
            if let Some(template) = &mut text.template {
                let rewritten = rewrite_template(template, &bindings.values);
                if let Some(value) = static_template_value(&rewritten) {
                    text.value = value.to_string();
                    text.template = None;
                } else {
                    *template = rewritten;
                }
            }
        }
        RenderNode::Raw(_) => {}
    }
}

fn rewrite_element_for_component_import(element: &mut RenderElement, bindings: &DcImportBindings) {
    for attribute in &mut element.attributes {
        if let Some(template) = &mut attribute.template {
            let rewritten = rewrite_template(template, &bindings.values);
            attribute.value = rewritten.raw.clone();
            if static_template_value(&rewritten).is_some() {
                attribute.template = None;
            } else {
                *template = rewritten;
            }
        }
    }
    let mut dynamic_styles = Vec::with_capacity(element.dynamic_styles.len());
    for mut style in std::mem::take(&mut element.dynamic_styles) {
        if let Some(style_name) = style_binding_name(&style.expression)
            && let Some(declarations) = bindings.styles.get(style_name)
        {
            apply_component_style_bindings(
                element,
                &mut dynamic_styles,
                style.state.clone(),
                declarations,
            );
            continue;
        }
        style.expression = rewrite_template(&style.expression, &bindings.values);
        if static_template_value(&style.expression).is_some_and(|value| value.trim().is_empty()) {
            continue;
        }
        dynamic_styles.push(style);
    }
    element.dynamic_styles = dynamic_styles;
    for action in &mut element.actions {
        if let Some(template) = &mut action.template {
            *template = rewrite_template(template, &bindings.values);
            action.expression = template.raw.to_string();
        }
        rewrite_action_handler(action, bindings);
    }
    if let Some(control_flow) = &mut element.control_flow
        && let Some(expression) = &mut control_flow.expression
        && let Some(rewritten) = rewrite_expr(expression, &bindings.values)
        && let Some(rewritten_expr) = rewritten.single_expression()
    {
        *expression = rewritten_expr.clone();
    }
    if let Some(state) = &mut element.state
        && let RenderStateKind::TextInput(input) = &mut state.kind
    {
        if let Some(template) = &mut input.initial_template {
            let rewritten = rewrite_template(template, &bindings.values);
            input.initial_value = Some(rewritten.raw.clone());
            if static_template_value(&rewritten).is_some() {
                input.initial_template = None;
            } else {
                *template = rewritten;
            }
        }
        if let Some(template) = &mut input.placeholder_template {
            let rewritten = rewrite_template(template, &bindings.values);
            input.placeholder = Some(rewritten.raw.clone());
            if static_template_value(&rewritten).is_some() {
                input.placeholder_template = None;
            } else {
                *template = rewritten;
            }
        }
    }
    for pseudo in &mut element.pseudo_elements {
        rewrite_nodes_for_component_import(&mut pseudo.children, bindings);
    }
    rewrite_nodes_for_component_import(&mut element.children, bindings);
}

fn rewrite_action_handler(action: &mut ActionBinding, bindings: &DcImportBindings) {
    for invocation in &mut action.handler.invocations {
        let action_expr = Expr::path([invocation.action.clone()]);
        if let Some(rewritten) = rewrite_expr(&action_expr, &bindings.values)
            && let Some(Expr::Path(path)) = rewritten.single_expression()
        {
            invocation.action = CompactString::from(
                path.iter()
                    .map(AsRef::as_ref)
                    .collect::<Vec<&str>>()
                    .join("."),
            );
        }
    }
}

fn style_binding_name(template: &TemplateString) -> Option<&str> {
    let Expr::Path(path) = template.single_expression()? else {
        return None;
    };
    (path.len() == 1).then(|| path[0].as_str())
}

fn apply_component_style_bindings(
    element: &mut RenderElement,
    dynamic_styles: &mut Vec<RenderDynamicStyleBinding>,
    state: Option<CompactString>,
    declarations: &[DcComponentStyleBinding],
) {
    let mut variant_declarations = Vec::new();
    for declaration in declarations {
        if let Some(value) = static_template_value(&declaration.value) {
            if value.trim().is_empty() {
                continue;
            }
            let style = StyleDeclaration::new(
                StyleProperty::from(declaration.property.as_str()),
                value,
                false,
                declaration.span,
            );
            if state.is_some() {
                variant_declarations.push(style);
            } else {
                merge_component_style(&mut element.styles, style);
            }
        } else {
            dynamic_styles.push(RenderDynamicStyleBinding {
                state: state.clone(),
                expression: dynamic_style_declaration_template(
                    declaration.property.as_str(),
                    &declaration.value,
                    declaration.span,
                ),
                span: declaration.span.or(declaration.value.span),
            });
        }
    }

    if let Some(state) = state
        && !variant_declarations.is_empty()
    {
        element.style_variants.push(RenderStyleVariant {
            conditions: vec![RenderStyleCondition::PseudoClass(state.clone())],
            selector: CompactString::from(format!("[style-{state}]")),
            declarations: variant_declarations,
            span: None,
        });
    }
}

fn merge_component_style(styles: &mut Vec<StyleDeclaration>, style: StyleDeclaration) {
    if let Some(existing) = styles
        .iter_mut()
        .find(|existing| existing.property == style.property)
    {
        *existing = style;
    } else {
        styles.push(style);
    }
}

fn dynamic_style_declaration_template(
    property: &str,
    value: &TemplateString,
    span: Option<Span>,
) -> TemplateString {
    let mut segments = Vec::with_capacity(value.segments.len() + 1);
    segments.push(TemplateSegment::Literal(CompactString::from(format!(
        "{property}: "
    ))));
    segments.extend(value.segments.clone());
    TemplateString::new(template_raw_from_segments(&segments), segments, span)
}

fn rewrite_template(
    template: &TemplateString,
    bindings: &BTreeMap<CompactString, TemplateString>,
) -> TemplateString {
    if let Some(expression) = template.single_expression()
        && let Some(rewritten) = rewrite_expr(expression, bindings)
    {
        return rewritten;
    }

    let mut changed = false;
    let mut segments = Vec::new();
    for segment in &template.segments {
        match segment {
            TemplateSegment::Literal(value) => {
                segments.push(TemplateSegment::Literal(value.clone()))
            }
            TemplateSegment::Expression(expression) => {
                if let Some(rewritten) = rewrite_expr(expression, bindings) {
                    changed = true;
                    if rewritten.segments.is_empty() {
                        segments.push(TemplateSegment::Literal(rewritten.raw));
                    } else {
                        segments.extend(rewritten.segments);
                    }
                } else {
                    segments.push(TemplateSegment::Expression(expression.clone()));
                }
            }
        }
    }

    if changed {
        TemplateString::new(
            template_raw_from_segments(&segments),
            segments,
            template.span,
        )
    } else {
        template.clone()
    }
}

fn rewrite_expr(
    expression: &Expr,
    bindings: &BTreeMap<CompactString, TemplateString>,
) -> Option<TemplateString> {
    let Expr::Path(path) = expression else {
        return None;
    };
    let (first, rest) = path.split_first()?;
    let binding = bindings.get(first)?;
    if rest.is_empty() {
        return Some(binding.clone());
    }
    let binding_expr = binding.single_expression()?;
    let Expr::Path(binding_path) = binding_expr else {
        return None;
    };
    let mut path = binding_path.clone();
    path.extend(rest.iter().cloned());
    Some(template_from_expr(Expr::path(path)))
}

fn static_template_value(template: &TemplateString) -> Option<CompactString> {
    template
        .segments
        .iter()
        .all(|segment| matches!(segment, TemplateSegment::Literal(_)))
        .then(|| template.raw.clone())
}

fn template_raw_from_segments(segments: &[TemplateSegment]) -> CompactString {
    let mut raw = String::new();
    for segment in segments {
        match segment {
            TemplateSegment::Literal(value) => raw.push_str(value),
            TemplateSegment::Expression(expression) => {
                raw.push_str("{{ ");
                raw.push_str(&expression.to_string());
                raw.push_str(" }}");
            }
        }
    }
    raw.into()
}

fn object_property<'a>(property: &'a ObjectPropertyKind<'a>) -> Option<&'a ObjectProperty<'a>> {
    match property {
        ObjectPropertyKind::ObjectProperty(property) => Some(property),
        ObjectPropertyKind::SpreadProperty(_) => None,
    }
}

fn property_key_name<'a>(key: &'a PropertyKey<'a>) -> Option<&'a str> {
    match key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.as_str()),
        PropertyKey::StringLiteral(literal) => Some(literal.value.as_str()),
        _ => None,
    }
}

fn oxc_binding_identifier_name<'a>(binding: &'a OxcBindingPattern<'a>) -> Option<&'a str> {
    match binding {
        OxcBindingPattern::BindingIdentifier(identifier) => Some(identifier.name.as_str()),
        _ => None,
    }
}

#[derive(Debug)]
struct SourceMerger {
    sources: SourceMap,
}

impl SourceMerger {
    fn new(sources: SourceMap) -> Self {
        Self { sources }
    }

    fn import_sources(&mut self, sources: &SourceMap) -> SourceRemap {
        let mut remap = SourceRemap::default();
        for file in sources.files() {
            let target = self.sources.add_file(
                file.kind(),
                file.name().map(str::to_owned),
                file.buffer().clone(),
            );
            remap.sources.insert(file.id(), target);
        }
        remap
    }

    fn finish(self) -> SourceMap {
        self.sources
    }
}

#[derive(Debug, Default)]
struct SourceRemap {
    sources: BTreeMap<SourceId, SourceId>,
}

impl SourceRemap {
    fn span(&self, span: Span) -> Span {
        self.sources
            .get(&span.source)
            .copied()
            .map_or(span, |source| span.with_source(source))
    }

    fn opt_span(&self, span: &mut Option<Span>) {
        if let Some(value) = span {
            *value = self.span(*value);
        }
    }

    fn opt_source(&self, source: &mut Option<SourceId>) {
        if let Some(value) = source
            && let Some(mapped) = self.sources.get(value).copied()
        {
            *value = mapped;
        }
    }
}

fn remap_plan(plan: &mut RenderPlan, remap: &SourceRemap) {
    for style in &mut plan.root.styles {
        remap_style(style, remap);
    }
    for variant in &mut plan.root.style_variants {
        remap_style_variant(variant, remap);
    }
    remap.opt_span(&mut plan.root.span);
    for head in &mut plan.head {
        remap.opt_span(&mut head.span);
    }
    for annotation in &mut plan.annotations {
        remap_annotation(annotation, remap);
    }
    for script in &mut plan.scripts {
        remap_script(script, remap);
    }
    for logic in &mut plan.source_logic {
        remap_source_logic(logic, remap);
    }
    for token in &mut plan.theme.tokens {
        remap_theme_token(token, remap);
    }
    for node in &mut plan.nodes {
        remap_node(node, remap);
    }
    plan.state = RenderStatePlan::from_nodes(&plan.nodes);
}

fn remap_node(node: &mut RenderNode, remap: &SourceRemap) {
    match node {
        RenderNode::Element(element) => remap_element(element, remap),
        RenderNode::Text(text) => {
            remap_opt_template(&mut text.template, remap);
            remap.opt_span(&mut text.span);
        }
        RenderNode::Raw(raw) => remap.opt_span(&mut raw.span),
    }
}

fn remap_element(element: &mut RenderElement, remap: &SourceRemap) {
    for attribute in &mut element.attributes {
        remap_attribute(attribute, remap);
    }
    for style in &mut element.styles {
        remap_style(style, remap);
    }
    for variant in &mut element.style_variants {
        remap_style_variant(variant, remap);
    }
    for style in &mut element.dynamic_styles {
        remap_dynamic_style(style, remap);
    }
    for pseudo in &mut element.pseudo_elements {
        remap_pseudo(pseudo, remap);
    }
    for action in &mut element.actions {
        remap_action(action, remap);
    }
    if let Some(state) = &mut element.state {
        remap_state(state, remap);
    }
    if let Some(control) = &mut element.form_control {
        remap_form_control(control, remap);
    }
    if let Some(accessibility) = &mut element.accessibility {
        for attribute in &mut accessibility.aria {
            remap_attribute(attribute, remap);
        }
    }
    if let Some(control_flow) = &mut element.control_flow {
        remap_control_flow(control_flow, remap);
    }
    if let Some(source_intent) = &mut element.source_intent {
        for prop in &mut source_intent.props {
            remap_source_prop(prop, remap);
        }
    }
    if let Some(semantics) = &mut element.semantics {
        for extra in &mut semantics.extras {
            remap_semantic_extra(extra, remap);
        }
    }
    for child in &mut element.children {
        remap_node(child, remap);
    }
    remap.opt_span(&mut element.span);
}

fn remap_attribute(attribute: &mut RenderAttribute, remap: &SourceRemap) {
    remap_opt_template(&mut attribute.template, remap);
    remap.opt_span(&mut attribute.span);
}

fn remap_opt_template(template: &mut Option<TemplateString>, remap: &SourceRemap) {
    let Some(template) = template else {
        return;
    };
    remap_template(template, remap);
}

fn remap_template(template: &mut TemplateString, remap: &SourceRemap) {
    remap.opt_span(&mut template.span);
    for segment in &mut template.segments {
        if let TemplateSegment::Expression(_) = segment {}
    }
}

fn remap_style(style: &mut StyleDeclaration, remap: &SourceRemap) {
    remap_style_value(&mut style.value, remap);
    remap.opt_span(&mut style.span);
}

fn remap_style_value(value: &mut StyleValue, remap: &SourceRemap) {
    if let StyleValue::Token(token) = value {
        remap_style_token(token, remap);
    }
}

fn remap_style_token(token: &mut StyleToken, remap: &SourceRemap) {
    remap.opt_span(&mut token.span);
    if let Some(fallback) = &mut token.fallback {
        remap_style_value(fallback, remap);
    }
}

fn remap_style_variant(variant: &mut RenderStyleVariant, remap: &SourceRemap) {
    for style in &mut variant.declarations {
        remap_style(style, remap);
    }
    remap.opt_span(&mut variant.span);
}

fn remap_dynamic_style(style: &mut RenderDynamicStyleBinding, remap: &SourceRemap) {
    remap_template(&mut style.expression, remap);
    remap.opt_span(&mut style.span);
}

fn remap_pseudo(pseudo: &mut RenderPseudoElement, remap: &SourceRemap) {
    for style in &mut pseudo.styles {
        remap_style(style, remap);
    }
    for child in &mut pseudo.children {
        remap_node(child, remap);
    }
    remap.opt_span(&mut pseudo.span);
}

fn remap_action(action: &mut ActionBinding, remap: &SourceRemap) {
    remap_opt_template(&mut action.template, remap);
    remap_handler(&mut action.handler, remap);
    remap.opt_span(&mut action.span);
    remap.opt_span(&mut action.action_span);
}

fn remap_handler(handler: &mut RenderActionHandler, remap: &SourceRemap) {
    for invocation in &mut handler.invocations {
        remap_invocation(invocation, remap);
    }
    for effect in &mut handler.effects {
        match effect {
            RenderActionHandlerEffect::PreventDefault { span }
            | RenderActionHandlerEffect::StopPropagation { span } => remap.opt_span(span),
        }
    }
}

fn remap_invocation(invocation: &mut RenderActionInvocation, remap: &SourceRemap) {
    remap.opt_span(&mut invocation.span);
    remap.opt_span(&mut invocation.action_span);
}

fn remap_state(state: &mut RenderStateBinding, remap: &SourceRemap) {
    if let crate::plan::RenderStateKind::TextInput(input) = &mut state.kind {
        remap_opt_template(&mut input.initial_template, remap);
    }
    if let crate::plan::RenderStateKind::Choice(choice) = &mut state.kind {
        for option in &mut choice.options {
            remap_choice_option(option, remap);
        }
    }
    remap.opt_span(&mut state.span);
}

fn remap_choice_option(option: &mut RenderChoiceOption, remap: &SourceRemap) {
    remap.opt_span(&mut option.span);
}

fn remap_form_control(control: &mut RenderFormControl, remap: &SourceRemap) {
    for option in &mut control.options {
        remap_choice_option(option, remap);
    }
    for constraint in &mut control.validation.constraints {
        let _ = constraint;
    }
    remap.opt_span(&mut control.span);
}

fn remap_control_flow(control_flow: &mut RenderControlFlow, remap: &SourceRemap) {
    remap_binding(&mut control_flow.binding, remap);
    remap.opt_span(&mut control_flow.span);
}

fn remap_binding(binding: &mut Option<BindingPattern>, remap: &SourceRemap) {
    if let Some(binding) = binding {
        remap.opt_span(&mut binding.span);
    }
}

fn remap_source_prop(prop: &mut RenderSourceProp, remap: &SourceRemap) {
    remap_opt_template(&mut prop.template, remap);
    remap.opt_span(&mut prop.span);
}

fn remap_semantic_extra(extra: &mut RenderSemanticExtra, remap: &SourceRemap) {
    remap.opt_span(&mut extra.span);
}

fn remap_annotation(annotation: &mut RenderAnnotation, remap: &SourceRemap) {
    remap.opt_span(&mut annotation.span);
}

fn remap_script(script: &mut RenderScriptReference, remap: &SourceRemap) {
    remap.opt_span(&mut script.span);
    remap.opt_source(&mut script.resolved_source);
}

fn remap_source_logic(logic: &mut RenderSourceLogic, remap: &SourceRemap) {
    remap.opt_span(&mut logic.span);
}

fn remap_theme_token(token: &mut RenderThemeToken, remap: &SourceRemap) {
    for value in &mut token.values {
        remap_theme_token_value(value, remap);
    }
}

fn remap_theme_token_value(value: &mut RenderThemeTokenValue, remap: &SourceRemap) {
    remap_theme_scope(&mut value.scope, remap);
    remap_style_value(&mut value.value, remap);
    remap.opt_span(&mut value.span);
}

fn remap_theme_scope(scope: &mut RenderThemeScope, remap: &SourceRemap) {
    if let RenderThemeScope::InlineElement { element_span } = scope {
        remap.opt_span(element_span);
    }
}
