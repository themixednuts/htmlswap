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
        if tag.chars().next().is_some_and(char::is_uppercase) {
            self.diagnostics.push(Diagnostic::error(
                format!("JSX component tag `{tag}` is outside Phase 0 scope"),
                Some(self.span(element.opening_element.span)),
            ));
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
                        "JSX spread attributes are outside Phase 0 scope",
                        Some(self.span(spread.span)),
                    ));
                }
            }
        }
        render.children = self.lower_children(&element.children);
        (RenderNode::Element(Box::new(render)), key)
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
                let suffix = if react_style_is_unitless(&css_name) {
                    ""
                } else {
                    "px"
                };
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
                if react_style_is_unitless(property) {
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
        let Some(binding) =
            callback_binding(callback.params.items.first().map(|param| &param.pattern))
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
            .and_then(|param| callback_binding(Some(&param.pattern)));
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

fn callback_binding(pattern: Option<&OxcBindingPattern<'_>>) -> Option<BindingPattern> {
    let pattern = pattern?;
    let name = binding_identifier_name(pattern)?;
    Some(BindingPattern::new(name, None))
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
        return (!value.trim().is_empty()).then(|| value.to_owned());
    }
    let normalized = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

fn react_attribute_name(name: &str) -> CompactString {
    match name {
        "className" => "class".into(),
        "htmlFor" => "for".into(),
        "defaultValue" => "value".into(),
        "defaultChecked" => "checked".into(),
        _ if name.starts_with("aria-") || name.starts_with("data-") => name.into(),
        _ if name.chars().any(char::is_uppercase) => camel_to_kebab(name).into(),
        _ => name.into(),
    }
}

fn css_property_name(name: &str) -> String {
    if name.starts_with("--") {
        name.to_owned()
    } else {
        camel_to_kebab(name)
    }
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
            | "border-image-outset"
            | "border-image-slice"
            | "border-image-width"
            | "box-flex"
            | "box-flex-group"
            | "box-ordinal-group"
            | "column-count"
            | "columns"
            | "flex"
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
            | "tab-size"
            | "widows"
            | "z-index"
            | "zoom"
    )
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
