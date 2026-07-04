use std::path::Path;

use arcstr::ArcStr;
use compact_str::CompactString;
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, ArrayExpression, ArrayExpressionElement, ArrowFunctionExpression, AssignmentTarget,
    BindingPattern as OxcBindingPattern, CallExpression, Declaration, ExportDefaultDeclarationKind,
    Expression, Function, FunctionBody, JSXAttribute, JSXAttributeItem, JSXAttributeName,
    JSXAttributeValue, JSXChild, JSXElement, JSXElementName, JSXExpression, JSXExpressionContainer,
    JSXFragment, JSXMemberExpression, JSXMemberExpressionObject, ModuleExportName,
    ObjectExpression, ObjectPropertyKind, Program, PropertyKey, Statement,
};
use oxc_parser::{ParseOptions, Parser};
use oxc_span::{GetSpan, SourceType, Span as OxcSpan};
use oxc_syntax::operator::AssignmentOperator;

use crate::diagnostics::{Compilation, Diagnostic, Diagnostics};
use crate::expr::{
    BindingPattern, Expr, ExprLiteral, ObjectEntry, TemplateSegment, TemplateString,
};
use crate::plan::{
    RenderAttribute, RenderControlFlow, RenderControlFlowHost, RenderControlFlowKind,
    RenderDynamicStyleBinding, RenderElement, RenderLoopLocal, RenderNode, RenderPlan,
    RenderSourceLogic, RenderText, UiRole,
};
use crate::source::{SourceId, Span};
use crate::style::StyleDeclaration;

pub(crate) fn lower_jsx_module(
    source: &str,
    source_name: Option<&str>,
    source_id: SourceId,
) -> Compilation<RenderPlan> {
    let allocator = Allocator::default();
    let parser =
        Parser::new(&allocator, source, source_type_for(source_name)).with_options(ParseOptions {
            parse_regular_expression: true,
            ..ParseOptions::default()
        });
    let parsed = parser.parse();
    let mut diagnostics = parsed
        .diagnostics
        .into_iter()
        .map(|diagnostic| {
            let span = diagnostic.labels.first().map(|label| {
                let start = label.offset() as usize;
                Span::new(source_id, start, start + label.len() as usize)
            });
            Diagnostic::warning(diagnostic.to_string(), span)
        })
        .collect::<Diagnostics>();
    if parsed.panicked {
        diagnostics.push(Diagnostic::error("JSX parser stopped early", None));
    }

    let mut lowerer = JsxLowerer {
        source,
        source_id,
        diagnostics,
    };
    let plan = lowerer.lower_program(&parsed.program);
    Compilation::new(plan, lowerer.diagnostics)
}

fn source_type_for(source_name: Option<&str>) -> SourceType {
    source_name
        .and_then(|name| SourceType::from_path(Path::new(name)).ok())
        .unwrap_or_else(SourceType::jsx)
}

struct JsxLowerer<'a> {
    source: &'a str,
    source_id: SourceId,
    diagnostics: Diagnostics,
}

struct JsxComponent<'a> {
    name: CompactString,
    function: &'a Function<'a>,
    module_end: usize,
}

impl<'a> JsxLowerer<'a> {
    fn lower_program(&mut self, program: &'a Program<'a>) -> RenderPlan {
        let Some(component) = self.find_component(program) else {
            self.diagnostics.push(Diagnostic::error(
                "JSX source must expose a function component through window.Name, export, or export default in Phase 0",
                None,
            ));
            return RenderPlan::new(Vec::new());
        };
        let Some(body) = component.function.body.as_deref() else {
            self.diagnostics.push(Diagnostic::error(
                format!("JSX component `{}` has no body", component.name),
                Some(self.span(component.function.span)),
            ));
            return RenderPlan::new(Vec::new());
        };
        let Some(return_expression) = returned_expression(body) else {
            self.diagnostics.push(Diagnostic::error(
                format!("JSX component `{}` must return JSX", component.name),
                Some(self.span(component.function.span)),
            ));
            return RenderPlan::new(Vec::new());
        };

        let mut plan = RenderPlan::new(self.lower_return_expression(return_expression));
        let prelude = self.source_logic_prelude(body, component.module_end);
        if !prelude.trim().is_empty() {
            plan.source_logic.push(RenderSourceLogic {
                dialect: "jsx".into(),
                script_type: Some("module".into()),
                body: ArcStr::from(prelude),
                data_props: None,
                span: Some(self.span(component.function.span)),
            });
        }
        plan
    }

    fn find_component(&self, program: &'a Program<'a>) -> Option<JsxComponent<'a>> {
        if let Some(name) = self.find_window_component_name(program)
            && let Some(function) = self.find_function_declaration(program, name.as_str())
        {
            return Some(JsxComponent {
                name,
                function,
                module_end: function.span.start as usize,
            });
        }

        self.find_exported_component(program)
    }

    fn find_window_component_name(&self, program: &Program<'a>) -> Option<CompactString> {
        program.body.iter().find_map(|statement| {
            let Statement::ExpressionStatement(statement) = statement else {
                return None;
            };
            let Expression::AssignmentExpression(assignment) = &statement.expression else {
                return None;
            };
            if assignment.operator != AssignmentOperator::Assign {
                return None;
            }
            if !assignment_target_is_window_member(&assignment.left) {
                return None;
            }
            let Expression::Identifier(identifier) = &assignment.right else {
                return None;
            };
            Some(CompactString::from(identifier.name.as_str()))
        })
    }

    fn find_function_declaration(
        &self,
        program: &'a Program<'a>,
        name: &str,
    ) -> Option<&'a Function<'a>> {
        program.body.iter().find_map(|statement| {
            let Statement::FunctionDeclaration(function) = statement else {
                return None;
            };
            function
                .id
                .as_ref()
                .filter(|id| id.name.as_str() == name)
                .map(|_| function.as_ref())
        })
    }

    fn find_exported_component(&self, program: &'a Program<'a>) -> Option<JsxComponent<'a>> {
        for statement in &program.body {
            let statement_start = statement.span().start as usize;
            match statement {
                Statement::ExportNamedDeclaration(exported) => {
                    if let Some(Declaration::FunctionDeclaration(function)) = &exported.declaration
                    {
                        let name = function
                            .id
                            .as_ref()
                            .map(|id| CompactString::from(id.name.as_str()))?;
                        return Some(JsxComponent {
                            name,
                            function: function.as_ref(),
                            module_end: statement_start,
                        });
                    }
                    for specifier in &exported.specifiers {
                        let Some(name) = module_export_name(&specifier.local) else {
                            continue;
                        };
                        if let Some(function) =
                            self.find_function_declaration(program, name.as_str())
                        {
                            return Some(JsxComponent {
                                name,
                                function,
                                module_end: (function.span.start as usize).min(statement_start),
                            });
                        }
                    }
                }
                Statement::ExportDefaultDeclaration(exported) => match &exported.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                        let name = function
                            .id
                            .as_ref()
                            .map(|id| CompactString::from(id.name.as_str()))
                            .unwrap_or_else(|| CompactString::from("default"));
                        return Some(JsxComponent {
                            name,
                            function: function.as_ref(),
                            module_end: statement_start,
                        });
                    }
                    ExportDefaultDeclarationKind::Identifier(identifier) => {
                        let name = identifier.name.as_str();
                        if let Some(function) = self.find_function_declaration(program, name) {
                            return Some(JsxComponent {
                                name: CompactString::from(name),
                                function,
                                module_end: (function.span.start as usize).min(statement_start),
                            });
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        None
    }

    fn source_logic_prelude(&self, body: &FunctionBody<'a>, module_end: usize) -> String {
        let module = self.source.get(0..module_end).unwrap_or_default().trim();
        let setup_start = body
            .statements
            .first()
            .map(|statement| statement.span().start as usize)
            .unwrap_or(body.span.start as usize + 1);
        let setup_end = body
            .statements
            .iter()
            .find_map(|statement| {
                matches!(statement, Statement::ReturnStatement(_))
                    .then(|| statement.span().start as usize)
            })
            .unwrap_or((body.span.end as usize).saturating_sub(1));
        let setup = self
            .source
            .get(setup_start..setup_end)
            .unwrap_or_default()
            .trim();
        match (module.is_empty(), setup.is_empty()) {
            (true, true) => String::new(),
            (false, true) => module.to_owned(),
            (true, false) => setup.to_owned(),
            (false, false) => format!("{module}\n\n{setup}"),
        }
    }

    fn lower_return_expression(&mut self, expression: &Expression<'a>) -> Vec<RenderNode> {
        match expression {
            Expression::ParenthesizedExpression(expression) => {
                self.lower_return_expression(&expression.expression)
            }
            Expression::JSXElement(element) => vec![self.lower_jsx_element(element).0],
            Expression::JSXFragment(fragment) => self.lower_jsx_fragment(fragment),
            _ => {
                self.diagnostics.push(Diagnostic::error(
                    "Phase 0 JSX components must return a JSX element or fragment",
                    Some(self.span(expression.span())),
                ));
                Vec::new()
            }
        }
    }

    fn lower_jsx_fragment(&mut self, fragment: &JSXFragment<'a>) -> Vec<RenderNode> {
        self.diagnostics.push(Diagnostic::error(
            "JSX fragments are outside Phase 0 scope",
            Some(self.span(fragment.span)),
        ));
        self.lower_children(&fragment.children)
    }

    fn lower_jsx_element(&mut self, element: &JSXElement<'a>) -> (RenderNode, Option<Expr>) {
        let tag = self.jsx_element_name(&element.opening_element.name);
        if is_jsx_component_tag(&tag) {
            return (self.lower_component_placeholder(&tag, element), None);
        }
        if tag == "style" {
            return (self.lower_style_element(element), None);
        }

        let mut render = self.empty_element(&tag, element.span);
        let mut key = None;
        for attribute in &element.opening_element.attributes {
            match attribute {
                JSXAttributeItem::Attribute(attribute) => {
                    if self.lower_attribute(attribute, &mut render, &mut key) {
                        continue;
                    }
                }
                JSXAttributeItem::SpreadAttribute(spread) => {
                    self.diagnostics.push(Diagnostic::error(
                        "JSX spread attributes are outside Phase 1 scope",
                        Some(self.span(spread.span)),
                    ));
                }
            }
        }
        render.children = self.lower_children(&element.children);
        (RenderNode::Element(Box::new(render)), key)
    }

    fn lower_component_placeholder(&mut self, tag: &str, element: &JSXElement<'a>) -> RenderNode {
        self.diagnostics.push(Diagnostic::warning(
            format!("JSX component tag `{tag}` is outside Phase 1 scope; emitted placeholder"),
            Some(self.span(element.opening_element.span)),
        ));

        let mut placeholder = self.empty_element("div", element.span);
        placeholder
            .classes
            .push("htmlswap-jsx-component-placeholder".into());
        placeholder.attributes.push(RenderAttribute {
            name: "data-htmlswap-jsx-component-placeholder".into(),
            value: tag.into(),
            template: None,
            span: Some(self.span(element.opening_element.span)),
        });
        placeholder.children.push(RenderNode::Text(RenderText {
            value: format!("[unsupported JSX component: {tag}]"),
            template: None,
            span: Some(self.span(element.opening_element.span)),
        }));
        RenderNode::Element(Box::new(placeholder))
    }

    fn lower_style_element(&mut self, element: &JSXElement<'a>) -> RenderNode {
        let mut render = self.empty_element("style", element.span);
        for attribute in &element.opening_element.attributes {
            match attribute {
                JSXAttributeItem::Attribute(attribute) => {
                    self.diagnostics.push(Diagnostic::warning(
                        format!(
                            "JSX <style> attribute `{}` is outside Phase 1 scope",
                            self.jsx_attribute_name(&attribute.name)
                        ),
                        Some(self.span(attribute.span)),
                    ));
                }
                JSXAttributeItem::SpreadAttribute(spread) => {
                    self.diagnostics.push(Diagnostic::error(
                        "JSX <style> spread attributes are outside Phase 1 scope",
                        Some(self.span(spread.span)),
                    ));
                }
            }
        }
        if let Some(css) = self.style_element_text(element) {
            render.children.push(RenderNode::Text(RenderText {
                value: css,
                template: None,
                span: Some(self.span(element.span)),
            }));
        }
        RenderNode::Element(Box::new(render))
    }

    fn style_element_text(&mut self, element: &JSXElement<'a>) -> Option<String> {
        let mut css = String::new();
        for child in &element.children {
            match child {
                JSXChild::Text(text) => css.push_str(text.value.as_str()),
                JSXChild::ExpressionContainer(container) => match &container.expression {
                    JSXExpression::TemplateLiteral(template) => {
                        if template.expressions.is_empty() {
                            for quasi in &template.quasis {
                                css.push_str(quasi.value.raw.as_str());
                            }
                        } else {
                            self.diagnostics.push(Diagnostic::error(
                                "dynamic JSX <style> template expressions are outside Phase 1 scope",
                                Some(self.span(container.span)),
                            ));
                        }
                    }
                    JSXExpression::StringLiteral(value) => css.push_str(value.value.as_str()),
                    JSXExpression::EmptyExpression(_) => {}
                    _ => self.diagnostics.push(Diagnostic::error(
                        "JSX <style> children must be static text or a static template literal in Phase 1",
                        Some(self.span(container.span)),
                    )),
                },
                JSXChild::Element(child) => self.diagnostics.push(Diagnostic::error(
                    "JSX elements inside <style> are outside Phase 1 scope",
                    Some(self.span(child.span)),
                )),
                JSXChild::Fragment(fragment) => self.diagnostics.push(Diagnostic::error(
                    "JSX fragments inside <style> are outside Phase 1 scope",
                    Some(self.span(fragment.span)),
                )),
                JSXChild::Spread(spread) => self.diagnostics.push(Diagnostic::error(
                    "JSX spread children inside <style> are outside Phase 1 scope",
                    Some(self.span(spread.span)),
                )),
            }
        }
        let css = css.trim();
        (!css.is_empty()).then(|| css.to_owned())
    }

    fn lower_attribute(
        &mut self,
        attribute: &JSXAttribute<'a>,
        element: &mut RenderElement,
        key: &mut Option<Expr>,
    ) -> bool {
        let raw_name = self.jsx_attribute_name(&attribute.name);
        if raw_name == "key" {
            *key = attribute
                .value
                .as_ref()
                .and_then(|value| self.attribute_expr(value));
            return true;
        }
        let name = react_attribute_name(&raw_name);
        if name == "class" {
            if let Some(value) = &attribute.value {
                match value {
                    JSXAttributeValue::StringLiteral(value) => {
                        element.classes.extend(
                            value
                                .value
                                .as_str()
                                .split_whitespace()
                                .map(CompactString::from),
                        );
                    }
                    JSXAttributeValue::ExpressionContainer(container) => {
                        let expr = self.lower_jsx_expression(&container.expression);
                        element.attributes.push(RenderAttribute {
                            name: "class".into(),
                            value: CompactString::new(""),
                            template: Some(TemplateString::new(
                                self.source_for_span(container.span),
                                vec![TemplateSegment::Expression(expr)],
                                Some(self.span(container.span)),
                            )),
                            span: Some(self.span(attribute.span)),
                        });
                    }
                    _ => self.unsupported_attribute_value(value),
                }
            }
            return true;
        }
        if name == "style" {
            if let Some(value) = &attribute.value {
                self.lower_style_attribute(value, element, attribute.span);
            }
            return true;
        }
        let span = Some(self.span(attribute.span));
        match &attribute.value {
            None => element.attributes.push(RenderAttribute {
                name,
                value: CompactString::new(""),
                template: None,
                span,
            }),
            Some(JSXAttributeValue::StringLiteral(value)) => {
                element.attributes.push(RenderAttribute {
                    name,
                    value: CompactString::from(value.value.as_str()),
                    template: None,
                    span,
                })
            }
            Some(JSXAttributeValue::ExpressionContainer(container)) => {
                let expr = self.lower_jsx_expression(&container.expression);
                element.attributes.push(RenderAttribute {
                    name,
                    value: CompactString::new(""),
                    template: Some(TemplateString::new(
                        self.source_for_span(container.span),
                        vec![TemplateSegment::Expression(expr)],
                        Some(self.span(container.span)),
                    )),
                    span,
                });
            }
            Some(value) => self.unsupported_attribute_value(value),
        }
        true
    }

    fn lower_style_attribute(
        &mut self,
        value: &JSXAttributeValue<'a>,
        element: &mut RenderElement,
        span: OxcSpan,
    ) {
        let JSXAttributeValue::ExpressionContainer(container) = value else {
            self.unsupported_attribute_value(value);
            return;
        };
        match &container.expression {
            JSXExpression::ObjectExpression(object) => {
                self.lower_style_object(object, element, Some(self.span(span)));
            }
            _ => {
                let expr = self.lower_jsx_expression(&container.expression);
                element.dynamic_styles.push(RenderDynamicStyleBinding {
                    state: None,
                    expression: TemplateString::new(
                        self.source_for_span(container.span),
                        vec![TemplateSegment::Expression(expr)],
                        Some(self.span(container.span)),
                    ),
                    span: Some(self.span(span)),
                });
            }
        }
    }

    fn lower_style_object(
        &mut self,
        object: &ObjectExpression<'a>,
        element: &mut RenderElement,
        span: Option<Span>,
    ) {
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                self.diagnostics.push(Diagnostic::error(
                    "style object spreads are outside Phase 0 scope",
                    Some(self.span(property.span())),
                ));
                continue;
            };
            let Some(key) = self.property_key(&property.key) else {
                self.diagnostics.push(Diagnostic::error(
                    "computed style object keys are outside Phase 0 scope",
                    Some(self.span(property.span)),
                ));
                continue;
            };
            let css_name = css_property_name(&key);
            if let Some(value) = self.static_style_value(&css_name, &property.value) {
                element
                    .styles
                    .push(StyleDeclaration::new(css_name.as_str(), value, false, span));
            } else {
                let expr = self.lower_expression(&property.value);
                let suffix = dynamic_style_unit_suffix(&css_name, &property.value);
                element.dynamic_styles.push(RenderDynamicStyleBinding {
                    state: None,
                    expression: TemplateString::new(
                        self.source_for_span(property.value.span()),
                        vec![
                            TemplateSegment::Literal(format!("{css_name}: ").into()),
                            TemplateSegment::Expression(expr),
                            TemplateSegment::Literal(suffix.into()),
                        ],
                        Some(self.span(property.value.span())),
                    ),
                    span: Some(self.span(property.span)),
                });
            }
        }
    }

    fn static_style_value(&self, property: &str, expression: &Expression<'a>) -> Option<String> {
        match expression {
            Expression::StringLiteral(value) => Some(value.value.to_string()),
            Expression::NumericLiteral(value) => {
                let raw = value
                    .raw
                    .as_ref()
                    .map_or_else(|| value.value.to_string(), ToString::to_string);
                if property.starts_with("--") || react_style_is_unitless(property) {
                    Some(raw)
                } else {
                    Some(format!("{raw}px"))
                }
            }
            _ => None,
        }
    }

    fn lower_children(&mut self, children: &[JSXChild<'a>]) -> Vec<RenderNode> {
        let mut nodes = Vec::new();
        let mut text_segments = Vec::new();
        for child in children {
            match child {
                JSXChild::Text(text) => {
                    if let Some(value) = normalize_jsx_text(text.value.as_str()) {
                        text_segments.push(TemplateSegment::Literal(value.into()));
                    }
                }
                JSXChild::ExpressionContainer(container) => {
                    if let Some(node) = self.lower_expression_child(container) {
                        flush_text(
                            &mut nodes,
                            &mut text_segments,
                            Some(self.span(container.span)),
                        );
                        nodes.push(node);
                    } else if !matches!(container.expression, JSXExpression::EmptyExpression(_)) {
                        text_segments.push(TemplateSegment::Expression(
                            self.lower_jsx_expression(&container.expression),
                        ));
                    }
                }
                JSXChild::Element(element) => {
                    flush_text(
                        &mut nodes,
                        &mut text_segments,
                        Some(self.span(element.span)),
                    );
                    nodes.push(self.lower_jsx_element(element).0);
                }
                JSXChild::Fragment(fragment) => {
                    flush_text(
                        &mut nodes,
                        &mut text_segments,
                        Some(self.span(fragment.span)),
                    );
                    nodes.extend(self.lower_jsx_fragment(fragment));
                }
                JSXChild::Spread(spread) => {
                    self.diagnostics.push(Diagnostic::error(
                        "JSX spread children are outside Phase 0 scope",
                        Some(self.span(spread.span)),
                    ));
                }
            }
        }
        flush_text(&mut nodes, &mut text_segments, None);
        nodes
    }

    fn lower_expression_child(
        &mut self,
        container: &JSXExpressionContainer<'a>,
    ) -> Option<RenderNode> {
        match &container.expression {
            JSXExpression::CallExpression(call) => self.lower_map_call(call),
            JSXExpression::JSXElement(element) => Some(self.lower_jsx_element(element).0),
            JSXExpression::ParenthesizedExpression(expression) => match &expression.expression {
                Expression::JSXElement(element) => Some(self.lower_jsx_element(element).0),
                _ => None,
            },
            _ => None,
        }
    }

    fn lower_map_call(&mut self, call: &CallExpression<'a>) -> Option<RenderNode> {
        let Expression::StaticMemberExpression(member) = &call.callee else {
            return None;
        };
        if member.property.name.as_str() != "map" {
            return None;
        }
        let Some(argument) = call.arguments.first() else {
            self.diagnostics.push(Diagnostic::error(
                "JSX .map(...) must have a callback in Phase 0",
                Some(self.span(call.span)),
            ));
            return None;
        };
        let Some(callback) = argument_arrow_function(argument) else {
            self.diagnostics.push(Diagnostic::error(
                "JSX .map(...) callback must be an arrow function in Phase 0",
                Some(self.span(argument.span())),
            ));
            return None;
        };
        let Some(binding) = callback
            .params
            .items
            .first()
            .and_then(|param| self.callback_binding(&param.pattern))
        else {
            self.diagnostics.push(Diagnostic::error(
                "JSX .map(...) callback must bind an item parameter in Phase 0",
                Some(self.span(callback.params.span)),
            ));
            return None;
        };
        let index_binding = callback
            .params
            .items
            .get(1)
            .and_then(|param| self.callback_binding(&param.pattern));
        let list_expr = self.lower_expression(&member.object);
        let locals = self.callback_locals(callback);
        let Some(return_expression) = returned_expression(&callback.body) else {
            self.diagnostics.push(Diagnostic::error(
                "JSX .map(...) callback must return JSX in Phase 0",
                Some(self.span(callback.body.span)),
            ));
            return None;
        };
        let (children, key) = self.lower_map_return(return_expression);
        let mut wrapper = self.empty_element("jsx-each", call.span);
        wrapper.control_flow = Some(Box::new(RenderControlFlow {
            kind: RenderControlFlowKind::For,
            host: RenderControlFlowHost::Wrapper,
            expression: Some(list_expr),
            binding: Some(binding),
            index_binding,
            key,
            locals,
            placeholder: None,
            span: Some(self.span(call.span)),
        }));
        wrapper.children = children;
        Some(RenderNode::Element(Box::new(wrapper)))
    }

    fn lower_map_return(&mut self, expression: &Expression<'a>) -> (Vec<RenderNode>, Option<Expr>) {
        match expression {
            Expression::ParenthesizedExpression(expression) => {
                self.lower_map_return(&expression.expression)
            }
            Expression::JSXElement(element) => {
                let (node, key) = self.lower_jsx_element(element);
                (vec![node], key)
            }
            Expression::JSXFragment(fragment) => (self.lower_jsx_fragment(fragment), None),
            _ => {
                self.diagnostics.push(Diagnostic::error(
                    "JSX .map(...) callback must return JSX in Phase 0",
                    Some(self.span(expression.span())),
                ));
                (Vec::new(), None)
            }
        }
    }

    fn callback_locals(&mut self, callback: &ArrowFunctionExpression<'a>) -> Vec<RenderLoopLocal> {
        let mut locals = Vec::new();
        for statement in &callback.body.statements {
            let Statement::VariableDeclaration(declaration) = statement else {
                if matches!(statement, Statement::ReturnStatement(_)) {
                    break;
                }
                continue;
            };
            for declarator in &declaration.declarations {
                let Some(name) = binding_identifier_name(&declarator.id) else {
                    self.diagnostics.push(Diagnostic::error(
                        "destructured .map(...) callback locals are outside Phase 0 scope",
                        Some(self.span(declarator.span)),
                    ));
                    continue;
                };
                let Some(init) = &declarator.init else {
                    continue;
                };
                locals.push(RenderLoopLocal {
                    name,
                    value: Expr::Opaque(self.source_for_span(init.span()).into()),
                    span: Some(self.span(declarator.span)),
                });
            }
        }
        locals
    }

    fn callback_binding(&self, pattern: &OxcBindingPattern<'a>) -> Option<BindingPattern> {
        match pattern {
            OxcBindingPattern::BindingIdentifier(identifier) => Some(BindingPattern::new(
                identifier.name.as_str(),
                Some(self.span(pattern.span())),
            )),
            OxcBindingPattern::ArrayPattern(_) | OxcBindingPattern::ObjectPattern(_) => {
                let source = self.source_for_span(pattern.span()).trim();
                (!source.is_empty())
                    .then(|| BindingPattern::new(source, Some(self.span(pattern.span()))))
            }
            OxcBindingPattern::AssignmentPattern(pattern) => self.callback_binding(&pattern.left),
        }
    }

    fn attribute_expr(&mut self, value: &JSXAttributeValue<'a>) -> Option<Expr> {
        match value {
            JSXAttributeValue::ExpressionContainer(container) => {
                Some(self.lower_jsx_expression(&container.expression))
            }
            JSXAttributeValue::StringLiteral(value) => Some(Expr::Literal(ExprLiteral::String(
                value.value.as_str().into(),
            ))),
            other => {
                self.unsupported_attribute_value(other);
                None
            }
        }
    }

    fn lower_jsx_expression(&mut self, expression: &JSXExpression<'a>) -> Expr {
        match expression {
            JSXExpression::Identifier(identifier) => Expr::path([identifier.name.as_str()]),
            JSXExpression::StaticMemberExpression(member) => Expr::Member {
                object: Box::new(self.lower_expression(&member.object)),
                property: member.property.name.as_str().into(),
            },
            JSXExpression::ComputedMemberExpression(member) => Expr::Index {
                object: Box::new(self.lower_expression(&member.object)),
                index: Box::new(self.lower_expression(&member.expression)),
            },
            JSXExpression::CallExpression(call) => self.lower_call_expression(call),
            JSXExpression::BinaryExpression(binary) => Expr::Binary {
                left: Box::new(self.lower_expression(&binary.left)),
                operator: binary.operator.as_str().into(),
                right: Box::new(self.lower_expression(&binary.right)),
            },
            JSXExpression::LogicalExpression(logical) => Expr::Logical {
                left: Box::new(self.lower_expression(&logical.left)),
                operator: logical.operator.as_str().into(),
                right: Box::new(self.lower_expression(&logical.right)),
            },
            JSXExpression::ConditionalExpression(conditional) => Expr::Conditional {
                test: Box::new(self.lower_expression(&conditional.test)),
                consequent: Box::new(self.lower_expression(&conditional.consequent)),
                alternate: Box::new(self.lower_expression(&conditional.alternate)),
            },
            JSXExpression::TemplateLiteral(template) => self.lower_template_literal(template),
            JSXExpression::StringLiteral(value) => {
                Expr::Literal(ExprLiteral::String(value.value.as_str().into()))
            }
            JSXExpression::NumericLiteral(value) => Expr::Literal(ExprLiteral::Number(
                value
                    .raw
                    .as_ref()
                    .map_or_else(|| value.value.to_string(), ToString::to_string)
                    .into(),
            )),
            JSXExpression::BooleanLiteral(value) => Expr::Literal(ExprLiteral::Bool(value.value)),
            JSXExpression::NullLiteral(_) => Expr::Literal(ExprLiteral::Null),
            JSXExpression::ArrayExpression(array) => self.lower_array_expression(array),
            JSXExpression::ObjectExpression(object) => self.lower_object_expression(object),
            JSXExpression::ParenthesizedExpression(expression) => {
                self.lower_expression(&expression.expression)
            }
            JSXExpression::EmptyExpression(_) => Expr::Opaque(CompactString::new("")),
            _ => Expr::Opaque(self.source_for_span(expression.span()).into()),
        }
    }

    fn lower_expression(&mut self, expression: &Expression<'a>) -> Expr {
        match expression {
            Expression::Identifier(identifier) => Expr::path([identifier.name.as_str()]),
            Expression::StaticMemberExpression(member) => Expr::Member {
                object: Box::new(self.lower_expression(&member.object)),
                property: member.property.name.as_str().into(),
            },
            Expression::ComputedMemberExpression(member) => Expr::Index {
                object: Box::new(self.lower_expression(&member.object)),
                index: Box::new(self.lower_expression(&member.expression)),
            },
            Expression::CallExpression(call) => self.lower_call_expression(call),
            Expression::BinaryExpression(binary) => Expr::Binary {
                left: Box::new(self.lower_expression(&binary.left)),
                operator: binary.operator.as_str().into(),
                right: Box::new(self.lower_expression(&binary.right)),
            },
            Expression::LogicalExpression(logical) => Expr::Logical {
                left: Box::new(self.lower_expression(&logical.left)),
                operator: logical.operator.as_str().into(),
                right: Box::new(self.lower_expression(&logical.right)),
            },
            Expression::ConditionalExpression(conditional) => Expr::Conditional {
                test: Box::new(self.lower_expression(&conditional.test)),
                consequent: Box::new(self.lower_expression(&conditional.consequent)),
                alternate: Box::new(self.lower_expression(&conditional.alternate)),
            },
            Expression::TemplateLiteral(template) => self.lower_template_literal(template),
            Expression::StringLiteral(value) => {
                Expr::Literal(ExprLiteral::String(value.value.as_str().into()))
            }
            Expression::NumericLiteral(value) => Expr::Literal(ExprLiteral::Number(
                value
                    .raw
                    .as_ref()
                    .map_or_else(|| value.value.to_string(), ToString::to_string)
                    .into(),
            )),
            Expression::BooleanLiteral(value) => Expr::Literal(ExprLiteral::Bool(value.value)),
            Expression::NullLiteral(_) => Expr::Literal(ExprLiteral::Null),
            Expression::ArrayExpression(array) => self.lower_array_expression(array),
            Expression::ObjectExpression(object) => self.lower_object_expression(object),
            Expression::ParenthesizedExpression(expression) => {
                self.lower_expression(&expression.expression)
            }
            _ => Expr::Opaque(self.source_for_span(expression.span()).into()),
        }
    }

    fn lower_call_expression(&mut self, call: &CallExpression<'a>) -> Expr {
        Expr::Call {
            callee: Box::new(self.lower_expression(&call.callee)),
            arguments: call
                .arguments
                .iter()
                .map(|argument| Expr::Opaque(self.source_for_span(argument.span()).into()))
                .collect(),
        }
    }

    fn lower_array_expression(&mut self, array: &ArrayExpression<'a>) -> Expr {
        Expr::Array(
            array
                .elements
                .iter()
                .filter_map(|element| {
                    if matches!(element, ArrayExpressionElement::Elision(_)) {
                        None
                    } else {
                        Some(Expr::Opaque(self.source_for_span(element.span()).into()))
                    }
                })
                .collect(),
        )
    }

    fn lower_object_expression(&mut self, object: &ObjectExpression<'a>) -> Expr {
        let entries = object
            .properties
            .iter()
            .filter_map(|property| {
                let ObjectPropertyKind::ObjectProperty(property) = property else {
                    return None;
                };
                Some(ObjectEntry::new(
                    self.property_key(&property.key)?,
                    self.lower_expression(&property.value),
                ))
            })
            .collect();
        Expr::Object(entries)
    }

    fn lower_template_literal(&mut self, template: &oxc_ast::ast::TemplateLiteral<'a>) -> Expr {
        let mut segments = Vec::new();
        for (index, quasi) in template.quasis.iter().enumerate() {
            if !quasi.value.raw.is_empty() {
                segments.push(TemplateSegment::Literal(quasi.value.raw.as_str().into()));
            }
            if let Some(expression) = template.expressions.get(index) {
                segments.push(TemplateSegment::Expression(
                    self.lower_expression(expression),
                ));
            }
        }
        Expr::TemplateLiteral { segments }
    }

    fn jsx_element_name(&mut self, name: &JSXElementName<'a>) -> CompactString {
        match name {
            JSXElementName::Identifier(identifier) => identifier.name.as_str().into(),
            JSXElementName::IdentifierReference(identifier) => identifier.name.as_str().into(),
            JSXElementName::MemberExpression(member) => self.jsx_member_expression_name(member),
            JSXElementName::NamespacedName(name) => {
                format!("{}:{}", name.namespace.name, name.name.name).into()
            }
            JSXElementName::ThisExpression(_) => {
                self.diagnostics.push(Diagnostic::error(
                    "`this` JSX tags are outside Phase 0 scope",
                    Some(self.span(name.span())),
                ));
                "this".into()
            }
        }
    }

    fn jsx_member_expression_name(&self, member: &JSXMemberExpression<'a>) -> CompactString {
        let object = match &member.object {
            JSXMemberExpressionObject::IdentifierReference(identifier) => {
                identifier.name.to_string()
            }
            JSXMemberExpressionObject::MemberExpression(member) => {
                self.jsx_member_expression_name(member).to_string()
            }
            JSXMemberExpressionObject::ThisExpression(_) => "this".to_owned(),
        };
        format!("{}.{}", object, member.property.name).into()
    }

    fn jsx_attribute_name(&self, name: &JSXAttributeName<'a>) -> CompactString {
        match name {
            JSXAttributeName::Identifier(identifier) => identifier.name.as_str().into(),
            JSXAttributeName::NamespacedName(name) => {
                format!("{}:{}", name.namespace.name, name.name.name).into()
            }
        }
    }

    fn property_key(&self, key: &PropertyKey<'a>) -> Option<CompactString> {
        match key {
            PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.as_str().into()),
            PropertyKey::StringLiteral(value) => Some(value.value.as_str().into()),
            _ => None,
        }
    }

    fn unsupported_attribute_value(&mut self, value: &JSXAttributeValue<'a>) {
        self.diagnostics.push(Diagnostic::error(
            "JSX attribute value is outside Phase 0 scope",
            Some(self.span(value.span())),
        ));
    }

    fn empty_element(&self, tag: &str, span: OxcSpan) -> RenderElement {
        RenderElement {
            role: role_for_tag(tag),
            source_tag: tag.into(),
            attributes: Vec::new(),
            classes: Vec::new(),
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
            span: Some(self.span(span)),
        }
    }

    fn span(&self, span: OxcSpan) -> Span {
        Span::new(self.source_id, span.start as usize, span.end as usize)
    }

    fn source_for_span(&self, span: OxcSpan) -> &str {
        self.source
            .get(span.start as usize..span.end as usize)
            .unwrap_or_default()
    }
}

fn returned_expression<'b, 'a>(body: &'b FunctionBody<'a>) -> Option<&'b Expression<'a>> {
    body.statements
        .iter()
        .find_map(|statement| match statement {
            Statement::ReturnStatement(statement) => statement.argument.as_ref(),
            Statement::ExpressionStatement(statement) => Some(&statement.expression),
            _ => None,
        })
}

fn assignment_target_is_window_member(target: &AssignmentTarget<'_>) -> bool {
    let AssignmentTarget::StaticMemberExpression(member) = target else {
        return false;
    };
    matches!(&member.object, Expression::Identifier(identifier) if identifier.name.as_str() == "window")
}

fn module_export_name(name: &ModuleExportName<'_>) -> Option<CompactString> {
    match name {
        ModuleExportName::IdentifierName(identifier) => Some(identifier.name.as_str().into()),
        ModuleExportName::IdentifierReference(identifier) => Some(identifier.name.as_str().into()),
        ModuleExportName::StringLiteral(literal) => Some(literal.value.as_str().into()),
    }
}

fn argument_arrow_function<'b, 'a>(
    argument: &'b Argument<'a>,
) -> Option<&'b ArrowFunctionExpression<'a>> {
    match argument {
        Argument::ArrowFunctionExpression(callback) => Some(callback),
        _ => None,
    }
}

fn binding_identifier_name(pattern: &OxcBindingPattern<'_>) -> Option<CompactString> {
    match pattern {
        OxcBindingPattern::BindingIdentifier(identifier) => Some(identifier.name.as_str().into()),
        _ => None,
    }
}

fn flush_text(
    nodes: &mut Vec<RenderNode>,
    segments: &mut Vec<TemplateSegment>,
    span: Option<Span>,
) {
    if segments.is_empty() {
        return;
    }
    let raw = segments
        .iter()
        .map(|segment| match segment {
            TemplateSegment::Literal(value) => value.to_string(),
            TemplateSegment::Expression(expression) => expression.to_string(),
        })
        .collect::<Vec<_>>()
        .join("");
    let template = segments
        .iter()
        .any(|segment| matches!(segment, TemplateSegment::Expression(_)))
        .then(|| TemplateString::new(raw.clone(), std::mem::take(segments), span));
    if template.is_none() {
        segments.clear();
    }
    nodes.push(RenderNode::Text(RenderText {
        value: raw,
        template,
        span,
    }));
}

fn normalize_jsx_text(value: &str) -> Option<String> {
    if !value.contains('\n') {
        return (!value.is_empty()).then(|| value.replace(['\t', '\r'], " "));
    }

    let lines = value.lines().collect::<Vec<_>>();
    let last = lines.len().saturating_sub(1);
    let mut normalized = String::new();
    for (index, line) in lines.iter().enumerate() {
        let mut line = line.replace(['\t', '\r'], " ");
        if index > 0 {
            line = line.trim_start().to_owned();
        }
        if index < last {
            line = line.trim_end().to_owned();
        }
        if line.is_empty() {
            continue;
        }
        normalized.push_str(&line);
        if index < last {
            normalized.push(' ');
        }
    }
    (!normalized.is_empty()).then_some(normalized)
}

fn react_attribute_name(name: &str) -> CompactString {
    match name {
        "className" => "class".into(),
        "htmlFor" => "for".into(),
        "defaultValue" => "value".into(),
        "defaultChecked" => "checked".into(),
        _ if name.starts_with("aria-") || name.starts_with("data-") => name.into(),
        _ if let Some(mapped) = react_attribute_alias(name) => mapped.into(),
        _ if name.chars().any(char::is_uppercase) => camel_to_kebab(name).into(),
        _ => name.into(),
    }
}

fn react_attribute_alias(name: &str) -> Option<&'static str> {
    Some(match name {
        "acceptCharset" => "accept-charset",
        "accessKey" => "accesskey",
        "allowFullScreen" => "allowfullscreen",
        "autoCapitalize" => "autocapitalize",
        "autoComplete" => "autocomplete",
        "autoCorrect" => "autocorrect",
        "autoFocus" => "autofocus",
        "autoPlay" => "autoplay",
        "cellPadding" => "cellpadding",
        "cellSpacing" => "cellspacing",
        "charSet" => "charset",
        "classID" => "classid",
        "colSpan" => "colspan",
        "contentEditable" => "contenteditable",
        "contextMenu" => "contextmenu",
        "controlsList" => "controlslist",
        "crossOrigin" => "crossorigin",
        "dateTime" => "datetime",
        "encType" => "enctype",
        "enterKeyHint" => "enterkeyhint",
        "formAction" => "formaction",
        "formEncType" => "formenctype",
        "formMethod" => "formmethod",
        "formNoValidate" => "formnovalidate",
        "formTarget" => "formtarget",
        "frameBorder" => "frameborder",
        "hrefLang" => "hreflang",
        "httpEquiv" => "http-equiv",
        "inputMode" => "inputmode",
        "keyParams" => "keyparams",
        "keyType" => "keytype",
        "marginHeight" => "marginheight",
        "marginWidth" => "marginwidth",
        "maxLength" => "maxlength",
        "mediaGroup" => "mediagroup",
        "minLength" => "minlength",
        "noModule" => "nomodule",
        "noValidate" => "novalidate",
        "radioGroup" => "radiogroup",
        "readOnly" => "readonly",
        "referrerPolicy" => "referrerpolicy",
        "rowSpan" => "rowspan",
        "spellCheck" => "spellcheck",
        "srcDoc" => "srcdoc",
        "srcLang" => "srclang",
        "srcSet" => "srcset",
        "tabIndex" => "tabindex",
        "useMap" => "usemap",
        "accentHeight" => "accent-height",
        "alignmentBaseline" => "alignment-baseline",
        "arabicForm" => "arabic-form",
        "attributeName" => "attributeName",
        "attributeType" => "attributeType",
        "baseFrequency" => "baseFrequency",
        "baselineShift" => "baseline-shift",
        "baseProfile" => "baseProfile",
        "calcMode" => "calcMode",
        "capHeight" => "cap-height",
        "clipPath" => "clip-path",
        "clipPathUnits" => "clipPathUnits",
        "clipRule" => "clip-rule",
        "colorInterpolation" => "color-interpolation",
        "colorInterpolationFilters" => "color-interpolation-filters",
        "colorProfile" => "color-profile",
        "colorRendering" => "color-rendering",
        "contentScriptType" => "contentScriptType",
        "contentStyleType" => "contentStyleType",
        "diffuseConstant" => "diffuseConstant",
        "dominantBaseline" => "dominant-baseline",
        "edgeMode" => "edgeMode",
        "enableBackground" => "enable-background",
        "externalResourcesRequired" => "externalResourcesRequired",
        "fillOpacity" => "fill-opacity",
        "fillRule" => "fill-rule",
        "filterRes" => "filterRes",
        "filterUnits" => "filterUnits",
        "floodColor" => "flood-color",
        "floodOpacity" => "flood-opacity",
        "fontFamily" => "font-family",
        "fontSize" => "font-size",
        "fontSizeAdjust" => "font-size-adjust",
        "fontStretch" => "font-stretch",
        "fontStyle" => "font-style",
        "fontVariant" => "font-variant",
        "fontWeight" => "font-weight",
        "glyphName" => "glyph-name",
        "glyphOrientationHorizontal" => "glyph-orientation-horizontal",
        "glyphOrientationVertical" => "glyph-orientation-vertical",
        "glyphRef" => "glyphRef",
        "gradientTransform" => "gradientTransform",
        "gradientUnits" => "gradientUnits",
        "horizAdvX" => "horiz-adv-x",
        "horizOriginX" => "horiz-origin-x",
        "imageRendering" => "image-rendering",
        "kernelMatrix" => "kernelMatrix",
        "kernelUnitLength" => "kernelUnitLength",
        "keyPoints" => "keyPoints",
        "keySplines" => "keySplines",
        "keyTimes" => "keyTimes",
        "lengthAdjust" => "lengthAdjust",
        "letterSpacing" => "letter-spacing",
        "lightingColor" => "lighting-color",
        "limitingConeAngle" => "limitingConeAngle",
        "markerEnd" => "marker-end",
        "markerHeight" => "markerHeight",
        "markerMid" => "marker-mid",
        "markerStart" => "marker-start",
        "markerUnits" => "markerUnits",
        "markerWidth" => "markerWidth",
        "maskContentUnits" => "maskContentUnits",
        "maskUnits" => "maskUnits",
        "numOctaves" => "numOctaves",
        "overlinePosition" => "overline-position",
        "overlineThickness" => "overline-thickness",
        "paintOrder" => "paint-order",
        "panose1" => "panose-1",
        "pathLength" => "pathLength",
        "patternContentUnits" => "patternContentUnits",
        "patternTransform" => "patternTransform",
        "patternUnits" => "patternUnits",
        "pointerEvents" => "pointer-events",
        "pointsAtX" => "pointsAtX",
        "pointsAtY" => "pointsAtY",
        "pointsAtZ" => "pointsAtZ",
        "preserveAlpha" => "preserveAlpha",
        "preserveAspectRatio" => "preserveAspectRatio",
        "primitiveUnits" => "primitiveUnits",
        "refX" => "refX",
        "refY" => "refY",
        "renderingIntent" => "rendering-intent",
        "repeatCount" => "repeatCount",
        "repeatDur" => "repeatDur",
        "requiredExtensions" => "requiredExtensions",
        "requiredFeatures" => "requiredFeatures",
        "specularConstant" => "specularConstant",
        "specularExponent" => "specularExponent",
        "spreadMethod" => "spreadMethod",
        "startOffset" => "startOffset",
        "stdDeviation" => "stdDeviation",
        "stitchTiles" => "stitchTiles",
        "stopColor" => "stop-color",
        "stopOpacity" => "stop-opacity",
        "strikethroughPosition" => "strikethrough-position",
        "strikethroughThickness" => "strikethrough-thickness",
        "strokeDasharray" => "stroke-dasharray",
        "strokeDashoffset" => "stroke-dashoffset",
        "strokeLinecap" => "stroke-linecap",
        "strokeLinejoin" => "stroke-linejoin",
        "strokeMiterlimit" => "stroke-miterlimit",
        "strokeOpacity" => "stroke-opacity",
        "strokeWidth" => "stroke-width",
        "surfaceScale" => "surfaceScale",
        "systemLanguage" => "systemLanguage",
        "tableValues" => "tableValues",
        "targetX" => "targetX",
        "targetY" => "targetY",
        "textAnchor" => "text-anchor",
        "textDecoration" => "text-decoration",
        "textLength" => "textLength",
        "textRendering" => "text-rendering",
        "underlinePosition" => "underline-position",
        "underlineThickness" => "underline-thickness",
        "unicodeBidi" => "unicode-bidi",
        "unicodeRange" => "unicode-range",
        "unitsPerEm" => "units-per-em",
        "vAlphabetic" => "v-alphabetic",
        "vHanging" => "v-hanging",
        "vIdeographic" => "v-ideographic",
        "vMathematical" => "v-mathematical",
        "vectorEffect" => "vector-effect",
        "vertAdvY" => "vert-adv-y",
        "vertOriginX" => "vert-origin-x",
        "vertOriginY" => "vert-origin-y",
        "viewBox" => "viewBox",
        "viewTarget" => "viewTarget",
        "wordSpacing" => "word-spacing",
        "writingMode" => "writing-mode",
        "xChannelSelector" => "xChannelSelector",
        "xHeight" => "x-height",
        "xlinkActuate" => "xlink:actuate",
        "xlinkArcrole" => "xlink:arcrole",
        "xlinkHref" => "xlink:href",
        "xlinkRole" => "xlink:role",
        "xlinkShow" => "xlink:show",
        "xlinkTitle" => "xlink:title",
        "xlinkType" => "xlink:type",
        "xmlBase" => "xml:base",
        "xmlLang" => "xml:lang",
        "xmlSpace" => "xml:space",
        "xmlnsXlink" => "xmlns:xlink",
        "yChannelSelector" => "yChannelSelector",
        "zoomAndPan" => "zoomAndPan",
        _ => return None,
    })
}

fn css_property_name(name: &str) -> String {
    if name.starts_with("--") {
        name.to_owned()
    } else if is_vendor_prefixed_style_name(name) {
        format!("-{}", camel_to_kebab(name))
    } else {
        camel_to_kebab(name)
    }
}

fn is_vendor_prefixed_style_name(name: &str) -> bool {
    (name.starts_with("ms") && name.chars().nth(2).is_some_and(|ch| ch.is_uppercase()))
        || ["Moz", "O", "Webkit"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

fn camel_to_kebab(name: &str) -> String {
    let mut output = String::with_capacity(name.len() + 4);
    for (index, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                output.push('-');
            }
            output.push(ch.to_ascii_lowercase());
        } else {
            output.push(ch);
        }
    }
    output
}

fn react_style_is_unitless(property: &str) -> bool {
    matches!(
        property,
        "animation-iteration-count"
            | "aspect-ratio"
            | "border-image-outset"
            | "border-image-slice"
            | "border-image-width"
            | "box-flex"
            | "box-flex-group"
            | "box-ordinal-group"
            | "column-count"
            | "columns"
            | "fill-opacity"
            | "flex"
            | "flex-basis"
            | "flex-grow"
            | "flex-positive"
            | "flex-shrink"
            | "flex-negative"
            | "flex-order"
            | "grid-area"
            | "grid-row"
            | "grid-row-end"
            | "grid-row-span"
            | "grid-row-start"
            | "grid-column"
            | "grid-column-end"
            | "grid-column-span"
            | "grid-column-start"
            | "font-weight"
            | "line-clamp"
            | "line-height"
            | "opacity"
            | "order"
            | "orphans"
            | "scale"
            | "stroke-opacity"
            | "tab-size"
            | "widows"
            | "z-index"
            | "zoom"
    )
}

fn dynamic_style_unit_suffix(property: &str, expression: &Expression<'_>) -> &'static str {
    if property.starts_with("--")
        || react_style_is_unitless(property)
        || !style_property_allows_px(property)
        || expression_is_stringish(expression)
    {
        return "";
    }
    if expression_can_be_numeric(expression) {
        "px"
    } else {
        ""
    }
}

fn style_property_allows_px(property: &str) -> bool {
    matches!(
        property,
        "block-size"
            | "border-block-end-width"
            | "border-block-start-width"
            | "border-bottom-left-radius"
            | "border-bottom-right-radius"
            | "border-bottom-width"
            | "border-end-end-radius"
            | "border-end-start-radius"
            | "border-inline-end-width"
            | "border-inline-start-width"
            | "border-left-width"
            | "border-radius"
            | "border-right-width"
            | "border-spacing"
            | "border-start-end-radius"
            | "border-start-start-radius"
            | "border-top-left-radius"
            | "border-top-right-radius"
            | "border-top-width"
            | "border-width"
            | "bottom"
            | "column-gap"
            | "column-rule-width"
            | "flex-basis"
            | "font-size"
            | "gap"
            | "height"
            | "inline-size"
            | "inset"
            | "inset-block"
            | "inset-block-end"
            | "inset-block-start"
            | "inset-inline"
            | "inset-inline-end"
            | "inset-inline-start"
            | "left"
            | "letter-spacing"
            | "margin"
            | "margin-block"
            | "margin-block-end"
            | "margin-block-start"
            | "margin-bottom"
            | "margin-inline"
            | "margin-inline-end"
            | "margin-inline-start"
            | "margin-left"
            | "margin-right"
            | "margin-top"
            | "max-block-size"
            | "max-height"
            | "max-inline-size"
            | "max-width"
            | "min-block-size"
            | "min-height"
            | "min-inline-size"
            | "min-width"
            | "outline-offset"
            | "outline-width"
            | "padding"
            | "padding-block"
            | "padding-block-end"
            | "padding-block-start"
            | "padding-bottom"
            | "padding-inline"
            | "padding-inline-end"
            | "padding-inline-start"
            | "padding-left"
            | "padding-right"
            | "padding-top"
            | "perspective"
            | "right"
            | "row-gap"
            | "scroll-margin"
            | "scroll-margin-block"
            | "scroll-margin-block-end"
            | "scroll-margin-block-start"
            | "scroll-margin-bottom"
            | "scroll-margin-inline"
            | "scroll-margin-inline-end"
            | "scroll-margin-inline-start"
            | "scroll-margin-left"
            | "scroll-margin-right"
            | "scroll-margin-top"
            | "scroll-padding"
            | "scroll-padding-block"
            | "scroll-padding-block-end"
            | "scroll-padding-block-start"
            | "scroll-padding-bottom"
            | "scroll-padding-inline"
            | "scroll-padding-inline-end"
            | "scroll-padding-inline-start"
            | "scroll-padding-left"
            | "scroll-padding-right"
            | "scroll-padding-top"
            | "text-indent"
            | "text-underline-offset"
            | "top"
            | "width"
            | "word-spacing"
    )
}

fn expression_is_stringish(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::StringLiteral(_) | Expression::TemplateLiteral(_) => true,
        Expression::ParenthesizedExpression(expression) => {
            expression_is_stringish(&expression.expression)
        }
        Expression::ConditionalExpression(conditional) => {
            expression_is_stringish(&conditional.consequent)
                || expression_is_stringish(&conditional.alternate)
        }
        Expression::LogicalExpression(logical) => {
            expression_is_stringish(&logical.left) || expression_is_stringish(&logical.right)
        }
        Expression::BinaryExpression(binary) if binary.operator.as_str() == "+" => {
            expression_is_stringish(&binary.left) || expression_is_stringish(&binary.right)
        }
        _ => false,
    }
}

fn expression_can_be_numeric(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::NumericLiteral(_)
        | Expression::Identifier(_)
        | Expression::StaticMemberExpression(_)
        | Expression::ComputedMemberExpression(_)
        | Expression::CallExpression(_) => true,
        Expression::ParenthesizedExpression(expression) => {
            expression_can_be_numeric(&expression.expression)
        }
        Expression::ConditionalExpression(conditional) => {
            expression_can_be_numeric(&conditional.consequent)
                && expression_can_be_numeric(&conditional.alternate)
        }
        Expression::BinaryExpression(binary) => {
            !expression_is_stringish(expression)
                && matches!(binary.operator.as_str(), "+" | "-" | "*" | "/" | "%" | "**")
        }
        _ => false,
    }
}

fn is_jsx_component_tag(tag: &str) -> bool {
    tag.contains('.') || tag.chars().next().is_some_and(char::is_uppercase)
}

fn role_for_tag(tag: &str) -> UiRole {
    match tag {
        "span" => UiRole::Inline,
        "p" => UiRole::Paragraph,
        "button" => UiRole::Button,
        "input" | "textarea" => UiRole::TextInput,
        "select" => UiRole::Select,
        "option" => UiRole::Option,
        "a" => UiRole::Link,
        "img" => UiRole::Image,
        "h1" => UiRole::Heading(1),
        "h2" => UiRole::Heading(2),
        "h3" => UiRole::Heading(3),
        "h4" => UiRole::Heading(4),
        "h5" => UiRole::Heading(5),
        "h6" => UiRole::Heading(6),
        "ul" => UiRole::List { ordered: false },
        "ol" => UiRole::List { ordered: true },
        "li" => UiRole::ListItem,
        "form" => UiRole::Form,
        "fieldset" => UiRole::Fieldset,
        "legend" => UiRole::Legend,
        "label" => UiRole::Label,
        "jsx-each" => UiRole::Unknown,
        _ => UiRole::Container,
    }
}
