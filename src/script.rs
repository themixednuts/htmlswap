use std::collections::HashMap;
use std::path::Path;

use compact_str::CompactString;
use heck::ToKebabCase;
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, ArrowFunctionExpression, BindingPattern, ChainElement, CommentKind, Expression,
    Function, FunctionBody, ImportDeclaration, ImportDeclarationSpecifier, ImportExpression,
    ImportOrExportKind, MethodDefinition, ObjectExpression, ObjectProperty, ObjectPropertyKind,
    PropertyKey, Statement, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::{ParseOptions, Parser};
use oxc_span::{SourceType, Span as OxcSpan};
use oxc_syntax::scope::ScopeFlags;

use crate::diagnostics::{Compilation, Diagnostic, Diagnostics};
use crate::ir::{HtmlDocument, HtmlElement, HtmlNode};
use crate::plan::{
    RenderActionArgument, RenderActionHandler, RenderActionHandlerEffect, RenderActionInvocation,
    RenderAnnotation, RenderAnnotationKind,
};
use crate::source::{SourceId, Span};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ScriptModule {
    pub(crate) actions: Vec<ActionDefinition>,
    pub(crate) annotations: Vec<RenderAnnotation>,
    pub(crate) imports: Vec<ScriptImport>,
    pub(crate) style_objects: Vec<ScriptStyleObject>,
}

impl ScriptModule {
    #[must_use]
    pub(crate) fn new(
        actions: Vec<ActionDefinition>,
        annotations: Vec<RenderAnnotation>,
        imports: Vec<ScriptImport>,
        style_objects: Vec<ScriptStyleObject>,
    ) -> Self {
        Self {
            actions,
            annotations,
            imports,
            style_objects,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActionDefinition {
    pub(crate) name: CompactString,
    pub(crate) span: Option<Span>,
    pub(crate) handler: RenderActionHandler,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptImport {
    pub(crate) specifier: CompactString,
    pub(crate) span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptStyleObject {
    pub(crate) name: CompactString,
    pub(crate) path: Vec<CompactString>,
    pub(crate) declarations: Vec<ScriptStyleDeclaration>,
    pub(crate) span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptStyleDeclaration {
    pub(crate) name: CompactString,
    pub(crate) value: CompactString,
    pub(crate) span: Option<Span>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ScriptStyleIndex {
    objects_by_name: HashMap<CompactString, Option<Vec<ScriptStyleDeclaration>>>,
    objects_by_path: HashMap<CompactString, Option<Vec<ScriptStyleDeclaration>>>,
}

impl ScriptStyleIndex {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn extend_module(&mut self, module: &mut ScriptModule) {
        for style_object in module.style_objects.drain(..) {
            merge_style_object_exact(
                &mut self.objects_by_name,
                style_object.name.clone(),
                style_object.declarations.clone(),
            );

            if !style_object.path.is_empty() {
                merge_style_object_common(
                    &mut self.objects_by_path,
                    style_path_key(style_object.path.iter().map(AsRef::as_ref)),
                    style_object.declarations,
                );
            }
        }
    }

    #[must_use]
    pub(crate) fn unique_declarations(&self, name: &str) -> Option<&[ScriptStyleDeclaration]> {
        self.objects_by_name
            .get(name)
            .and_then(Option::as_deref)
            .filter(|declarations| !declarations.is_empty())
    }

    #[must_use]
    pub(crate) fn unique_declarations_for_path<'a, I>(
        &self,
        path: I,
    ) -> Option<&[ScriptStyleDeclaration]>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let key = style_path_key(path);
        self.objects_by_path
            .get(key.as_str())
            .and_then(Option::as_deref)
            .filter(|declarations| !declarations.is_empty())
    }

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.objects_by_name.is_empty() && self.objects_by_path.is_empty()
    }
}

fn merge_style_object_exact(
    objects: &mut HashMap<CompactString, Option<Vec<ScriptStyleDeclaration>>>,
    key: CompactString,
    declarations: Vec<ScriptStyleDeclaration>,
) {
    match objects.entry(key) {
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(Some(declarations));
        }
        std::collections::hash_map::Entry::Occupied(mut entry) => {
            if entry.get().as_ref() != Some(&declarations) {
                entry.insert(None);
            }
        }
    }
}

fn merge_style_object_common(
    objects: &mut HashMap<CompactString, Option<Vec<ScriptStyleDeclaration>>>,
    key: CompactString,
    declarations: Vec<ScriptStyleDeclaration>,
) {
    match objects.entry(key) {
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(Some(declarations));
        }
        std::collections::hash_map::Entry::Occupied(mut entry) => {
            if let Some(existing) = entry.get_mut() {
                existing.retain(|declaration| {
                    declarations.iter().any(|next| {
                        next.name == declaration.name && next.value == declaration.value
                    })
                });
            }
        }
    }
}

fn style_path_key<'a>(path: impl IntoIterator<Item = &'a str>) -> CompactString {
    CompactString::from(path.into_iter().collect::<Vec<_>>().join("."))
}

#[must_use]
pub(crate) fn parse_script_with_source(
    source: &str,
    source_name: Option<&str>,
    source_id: SourceId,
) -> Compilation<ScriptModule> {
    parse_script_with_span_base(source, source_name, Some(SpanBase::new(source_id, 0)))
}

#[must_use]
pub(crate) fn parse_inline_script(script: &InlineScript) -> Compilation<ScriptModule> {
    parse_script_with_span_base(&script.source, None, script.span.map(SpanBase::from))
}

#[must_use]
pub(crate) fn parse_inline_script_with_offset(
    source: &str,
    source_id: SourceId,
    offset: usize,
) -> Compilation<ScriptModule> {
    parse_script_with_span_base(source, None, Some(SpanBase::new(source_id, offset)))
}

#[derive(Debug, Clone, Copy)]
struct SpanBase {
    source: SourceId,
    offset: usize,
}

impl SpanBase {
    fn new(source: SourceId, offset: usize) -> Self {
        Self { source, offset }
    }
}

impl From<Span> for SpanBase {
    fn from(span: Span) -> Self {
        Self::new(span.source, span.start)
    }
}

fn parse_script_with_span_base(
    source: &str,
    source_name: Option<&str>,
    span_base: Option<SpanBase>,
) -> Compilation<ScriptModule> {
    let allocator = Allocator::default();
    let source_type = source_type_for(source_name);
    let parser = Parser::new(&allocator, source, source_type).with_options(ParseOptions {
        parse_regular_expression: true,
        allow_return_outside_function: source_name.is_none(),
        ..ParseOptions::default()
    });
    let parsed = parser.parse();

    let mut diagnostics = parsed
        .diagnostics
        .into_iter()
        .map(|diagnostic| {
            let span = span_base.and_then(|base| {
                diagnostic.labels.first().map(|label| {
                    let start = label.offset() as usize;
                    Span::new(
                        base.source,
                        base.offset + start,
                        base.offset + start + label.len() as usize,
                    )
                })
            });

            Diagnostic::warning(diagnostic.to_string(), span)
        })
        .collect::<Diagnostics>();

    if parsed.panicked {
        diagnostics.push(Diagnostic::error("JavaScript parser stopped early", None));
    }

    let mut collector = ModuleCollector::new(span_base);
    collector.visit_program(&parsed.program);
    let annotations = parsed
        .program
        .comments
        .iter()
        .map(|comment| {
            let content_span = comment.content_span();
            let value = source
                .get(content_span.start as usize..content_span.end as usize)
                .unwrap_or_default()
                .to_owned();
            let kind = match comment.kind {
                CommentKind::Line => RenderAnnotationKind::JavaScriptLineComment,
                CommentKind::SingleLineBlock | CommentKind::MultiLineBlock => {
                    RenderAnnotationKind::JavaScriptBlockComment
                }
            };
            RenderAnnotation::new(
                kind,
                value,
                span_base.map(|base| span_from_oxc(comment.span, base)),
            )
        })
        .collect();

    Compilation::new(
        ScriptModule::new(
            collector.actions,
            annotations,
            collector.imports,
            collector.style_objects,
        ),
        diagnostics,
    )
}

fn source_type_for(source_name: Option<&str>) -> SourceType {
    source_name
        .and_then(|name| SourceType::from_path(Path::new(name)).ok())
        .map(|source_type| source_type.with_typescript(true))
        .unwrap_or_else(SourceType::ts)
}

struct ModuleCollector {
    actions: Vec<ActionDefinition>,
    imports: Vec<ScriptImport>,
    style_objects: Vec<ScriptStyleObject>,
    style_aliases: HashMap<CompactString, Vec<ScriptStyleDeclaration>>,
    style_factories: HashMap<CompactString, Vec<ScriptStyleDeclaration>>,
    span_base: Option<SpanBase>,
}

impl ModuleCollector {
    fn new(span_base: Option<SpanBase>) -> Self {
        Self {
            actions: Vec::new(),
            imports: Vec::new(),
            style_objects: Vec::new(),
            style_aliases: HashMap::new(),
            style_factories: HashMap::new(),
            span_base,
        }
    }

    fn span_from_oxc(&self, span: OxcSpan) -> Option<Span> {
        self.span_base.map(|base| span_from_oxc(span, base))
    }

    fn push_action(&mut self, name: &str, span: OxcSpan, handler: RenderActionHandler) {
        self.actions.push(ActionDefinition {
            name: CompactString::from(name),
            span: self.span_from_oxc(span),
            handler,
        });
    }

    fn push_import(&mut self, specifier: &str, span: OxcSpan) {
        self.imports.push(ScriptImport {
            specifier: CompactString::from(specifier),
            span: self.span_from_oxc(span),
        });
    }

    fn push_style_object_declarations(
        &mut self,
        name: &str,
        path_prefix: &[CompactString],
        declarations: Vec<ScriptStyleDeclaration>,
        span: OxcSpan,
    ) {
        if !looks_like_style_object_name(name) {
            return;
        }

        if declarations.is_empty() {
            return;
        }

        let path = if path_prefix.is_empty() {
            Vec::new()
        } else {
            path_prefix
                .iter()
                .cloned()
                .chain(std::iter::once(CompactString::from(name)))
                .collect()
        };

        self.style_objects.push(ScriptStyleObject {
            name: CompactString::from(name),
            path,
            declarations,
            span: self.span_from_oxc(span),
        });
    }

    fn remember_style_alias(&mut self, name: &str, declarations: Vec<ScriptStyleDeclaration>) {
        if !declarations.is_empty() {
            self.style_aliases
                .insert(CompactString::from(name), declarations);
        }
    }

    fn remember_style_factory(&mut self, name: &str, declarations: Vec<ScriptStyleDeclaration>) {
        if !declarations.is_empty() {
            self.style_factories
                .insert(CompactString::from(name), declarations);
        }
    }

    fn style_declarations_from_object(
        &self,
        object: &ObjectExpression<'_>,
    ) -> Vec<ScriptStyleDeclaration> {
        style_declarations_from_object(object, self.span_base, &self.style_aliases)
    }

    fn style_declarations_from_expression(
        &self,
        expression: &Expression<'_>,
    ) -> Vec<ScriptStyleDeclaration> {
        match expression {
            Expression::ObjectExpression(object) => self.style_declarations_from_object(object),
            Expression::Identifier(identifier) => self
                .style_aliases
                .get(identifier.name.as_str())
                .cloned()
                .unwrap_or_default(),
            Expression::CallExpression(call) => callee_style_factory_name(&call.callee)
                .and_then(|name| self.style_factories.get(name.as_str()))
                .cloned()
                .unwrap_or_default(),
            Expression::ConditionalExpression(expression) => common_declarations(
                self.style_declarations_from_expression(&expression.consequent),
                self.style_declarations_from_expression(&expression.alternate),
            ),
            Expression::ParenthesizedExpression(expression) => {
                self.style_declarations_from_expression(&expression.expression)
            }
            _ => Vec::new(),
        }
    }

    fn style_declarations_from_function_body(
        &self,
        body: &FunctionBody<'_>,
    ) -> Vec<ScriptStyleDeclaration> {
        let mut declarations = None;
        for statement in &body.statements {
            let Some(next) = self.style_return_declarations_from_statement(statement) else {
                continue;
            };
            declarations = Some(match declarations {
                Some(existing) => common_declarations(existing, next),
                None => next,
            });
        }
        declarations.unwrap_or_default()
    }

    fn style_return_declarations_from_statement(
        &self,
        statement: &Statement<'_>,
    ) -> Option<Vec<ScriptStyleDeclaration>> {
        match statement {
            Statement::ReturnStatement(statement) => statement
                .argument
                .as_ref()
                .map(|argument| self.style_declarations_from_expression(argument)),
            Statement::ExpressionStatement(statement) => {
                Some(self.style_declarations_from_expression(&statement.expression))
            }
            Statement::BlockStatement(block) => {
                let mut declarations = None;
                for statement in &block.body {
                    let Some(next) = self.style_return_declarations_from_statement(statement)
                    else {
                        continue;
                    };
                    declarations = Some(match declarations {
                        Some(existing) => common_declarations(existing, next),
                        None => next,
                    });
                }
                declarations
            }
            Statement::IfStatement(statement) => {
                let consequent =
                    self.style_return_declarations_from_statement(&statement.consequent);
                let alternate = statement
                    .alternate
                    .as_ref()
                    .and_then(|alternate| self.style_return_declarations_from_statement(alternate));
                Some(match (consequent, alternate) {
                    (Some(consequent), Some(alternate)) => {
                        common_declarations(consequent, alternate)
                    }
                    (Some(declarations), None) | (None, Some(declarations)) => declarations,
                    (None, None) => Vec::new(),
                })
            }
            _ => None,
        }
    }

    fn collect_style_objects_from_expression(
        &mut self,
        expression: &Expression<'_>,
        path_prefix: &[CompactString],
    ) {
        match expression {
            Expression::ObjectExpression(object) => {
                self.collect_style_objects_from_object(object, path_prefix);
            }
            Expression::ArrayExpression(array) => {
                for element in &array.elements {
                    if let Some(expression) = array_expression_element(element) {
                        self.collect_style_objects_from_expression(expression, path_prefix);
                    }
                }
            }
            Expression::CallExpression(call) => {
                for argument in &call.arguments {
                    self.collect_style_objects_from_argument(argument, path_prefix);
                }
            }
            Expression::ArrowFunctionExpression(function) => {
                self.collect_style_objects_from_function_body(&function.body, path_prefix);
            }
            Expression::FunctionExpression(function) => {
                if let Some(body) = &function.body {
                    self.collect_style_objects_from_function_body(body, path_prefix);
                }
            }
            Expression::ConditionalExpression(expression) => {
                self.collect_style_objects_from_expression(&expression.consequent, path_prefix);
                self.collect_style_objects_from_expression(&expression.alternate, path_prefix);
            }
            Expression::ParenthesizedExpression(expression) => {
                self.collect_style_objects_from_expression(&expression.expression, path_prefix);
            }
            _ => {}
        }
    }

    fn collect_style_objects_from_argument(
        &mut self,
        argument: &Argument<'_>,
        path_prefix: &[CompactString],
    ) {
        match argument {
            Argument::ArrowFunctionExpression(function) => {
                self.collect_style_objects_from_function_body(&function.body, path_prefix);
            }
            Argument::FunctionExpression(function) => {
                if let Some(body) = &function.body {
                    self.collect_style_objects_from_function_body(body, path_prefix);
                }
            }
            Argument::ObjectExpression(object) => {
                self.collect_style_objects_from_object(object, path_prefix);
            }
            Argument::ArrayExpression(array) => {
                for element in &array.elements {
                    if let Some(expression) = array_expression_element(element) {
                        self.collect_style_objects_from_expression(expression, path_prefix);
                    }
                }
            }
            Argument::ParenthesizedExpression(expression) => {
                self.collect_style_objects_from_expression(&expression.expression, path_prefix);
            }
            _ => {}
        }
    }

    fn collect_style_objects_from_function_body(
        &mut self,
        body: &FunctionBody<'_>,
        path_prefix: &[CompactString],
    ) {
        for statement in &body.statements {
            match statement {
                Statement::ReturnStatement(statement) => {
                    if let Some(argument) = &statement.argument {
                        self.collect_style_objects_from_expression(argument, path_prefix);
                    }
                }
                Statement::ExpressionStatement(statement) => {
                    self.collect_style_objects_from_expression(&statement.expression, path_prefix);
                }
                _ => {}
            }
        }
    }

    fn remember_style_bindings_for_named_init(&mut self, name: &str, init: &Expression<'_>) {
        match init {
            Expression::FunctionExpression(function) => {
                if let Some(body) = &function.body {
                    let declarations = self.style_declarations_from_function_body(body);
                    self.remember_style_factory(name, declarations);
                }
            }
            Expression::ArrowFunctionExpression(function) => {
                let declarations = self.style_declarations_from_function_body(&function.body);
                self.remember_style_factory(name, declarations);
            }
            Expression::ObjectExpression(object) => {
                let declarations = self.style_declarations_from_object(object);
                self.remember_style_alias(name, declarations);
            }
            _ => {}
        }
    }

    fn collect_render_value_style_objects_from_function_body(&mut self, body: &FunctionBody<'_>) {
        for statement in &body.statements {
            match statement {
                Statement::VariableDeclaration(declaration) => {
                    for declarator in &declaration.declarations {
                        let Some(name) = binding_identifier_name(&declarator.id) else {
                            continue;
                        };
                        let Some(init) = &declarator.init else {
                            continue;
                        };
                        self.remember_style_bindings_for_named_init(name, init);
                        self.collect_style_objects_from_expression(
                            init,
                            &[CompactString::from(name)],
                        );
                    }
                }
                Statement::ReturnStatement(statement) => {
                    if let Some(argument) = &statement.argument {
                        self.collect_render_value_style_objects_from_expression(argument);
                    }
                }
                Statement::BlockStatement(block) => {
                    for statement in &block.body {
                        if let Statement::ReturnStatement(statement) = statement
                            && let Some(argument) = &statement.argument
                        {
                            self.collect_render_value_style_objects_from_expression(argument);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn collect_render_value_style_objects_from_expression(&mut self, expression: &Expression<'_>) {
        let Expression::ObjectExpression(object) = expression else {
            self.collect_style_objects_from_expression(expression, &[]);
            return;
        };

        for property in object.properties.iter().filter_map(object_property) {
            let Some(name) = property_key_name(&property.key) else {
                continue;
            };

            let declarations = self.style_declarations_from_expression(&property.value);
            if !declarations.is_empty() {
                self.push_style_object_declarations(name, &[], declarations, property.span);
            }

            self.collect_style_objects_from_expression(
                &property.value,
                &[CompactString::from(name)],
            );
        }
    }

    fn collect_style_objects_from_object(
        &mut self,
        object: &ObjectExpression<'_>,
        path_prefix: &[CompactString],
    ) {
        for property in object.properties.iter().filter_map(object_property) {
            let Some(name) = property_key_name(&property.key) else {
                continue;
            };

            let declarations = self.style_declarations_from_expression(&property.value);
            if !declarations.is_empty() {
                self.push_style_object_declarations(name, path_prefix, declarations, property.span);
            }

            let child_prefix = if looks_like_style_object_name(name) {
                path_prefix.to_vec()
            } else if path_prefix.is_empty() {
                Vec::new()
            } else {
                path_prefix
                    .iter()
                    .cloned()
                    .chain(std::iter::once(CompactString::from(name)))
                    .collect()
            };
            self.collect_style_objects_from_expression(&property.value, &child_prefix);
        }
    }
}

impl<'a> Visit<'a> for ModuleCollector {
    fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
        if let Some(id) = &function.id {
            let handler = handler_from_function(function, self.span_base);
            self.push_action(id.name.as_str(), function.span, handler);
            if let Some(body) = &function.body {
                let declarations = self.style_declarations_from_function_body(body);
                self.remember_style_factory(id.name.as_str(), declarations);
            }
        }

        walk::walk_function(self, function, flags);
    }

    fn visit_method_definition(&mut self, definition: &MethodDefinition<'a>) {
        if !definition.computed {
            if let Some(name) = property_key_name(&definition.key) {
                if let Some(body) = &definition.value.body {
                    let declarations = self.style_declarations_from_function_body(body);
                    self.remember_style_factory(name, declarations);
                    if name == "renderVals" {
                        self.collect_render_value_style_objects_from_function_body(body);
                    }
                }
            }
        }

        walk::walk_method_definition(self, definition);
    }

    fn visit_import_declaration(&mut self, declaration: &ImportDeclaration<'a>) {
        if declaration.import_kind == ImportOrExportKind::Type {
            walk::walk_import_declaration(self, declaration);
            return;
        }

        self.push_import(declaration.source.value.as_str(), declaration.source.span);

        if let Some(specifiers) = &declaration.specifiers {
            for specifier in specifiers {
                match specifier {
                    ImportDeclarationSpecifier::ImportSpecifier(specifier)
                        if specifier.import_kind != ImportOrExportKind::Type =>
                    {
                        self.push_action(
                            specifier.local.name.as_str(),
                            specifier.span,
                            RenderActionHandler::default(),
                        );
                    }
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                        self.push_action(
                            specifier.local.name.as_str(),
                            specifier.span,
                            RenderActionHandler::default(),
                        );
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => {
                        self.push_action(
                            specifier.local.name.as_str(),
                            specifier.span,
                            RenderActionHandler::default(),
                        );
                    }
                    ImportDeclarationSpecifier::ImportSpecifier(_) => {}
                }
            }
        }

        walk::walk_import_declaration(self, declaration);
    }

    fn visit_import_expression(&mut self, expression: &ImportExpression<'a>) {
        if let Expression::StringLiteral(literal) = &expression.source {
            self.push_import(literal.value.as_str(), literal.span);
        }

        walk::walk_import_expression(self, expression);
    }

    fn visit_variable_declarator(&mut self, declarator: &VariableDeclarator<'a>) {
        let Some(name) = binding_identifier_name(&declarator.id) else {
            walk::walk_variable_declarator(self, declarator);
            return;
        };
        let Some(init) = &declarator.init else {
            walk::walk_variable_declarator(self, declarator);
            return;
        };

        let handler = match init {
            Expression::FunctionExpression(function) => {
                self.remember_style_bindings_for_named_init(name, init);
                Some(handler_from_function(function, self.span_base))
            }
            Expression::ArrowFunctionExpression(function) => {
                self.remember_style_bindings_for_named_init(name, init);
                Some(handler_from_arrow_function(function, self.span_base))
            }
            Expression::ObjectExpression(_) => {
                self.remember_style_bindings_for_named_init(name, init);
                None
            }
            _ => None,
        };

        if let Some(handler) = handler {
            self.push_action(name, declarator.span, handler);
        }

        self.collect_style_objects_from_expression(init, &[CompactString::from(name)]);

        walk::walk_variable_declarator(self, declarator);
    }

    fn visit_object_expression(&mut self, expression: &ObjectExpression<'a>) {
        for property in expression.properties.iter().filter_map(object_property) {
            let Some(name) = property_key_name(&property.key) else {
                continue;
            };
            let declarations = self.style_declarations_from_expression(&property.value);
            if !declarations.is_empty() {
                self.push_style_object_declarations(name, &[], declarations, property.span);
            }
        }

        walk::walk_object_expression(self, expression);
    }
}

fn array_expression_element<'a>(
    element: &'a oxc_ast::ast::ArrayExpressionElement<'a>,
) -> Option<&'a Expression<'a>> {
    element.as_expression()
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

fn looks_like_style_object_name(name: &str) -> bool {
    name == "style"
        || name.ends_with("Style")
        || name.ends_with("Styles")
        || name.ends_with("style")
        || name.ends_with("styles")
}

fn common_declarations(
    mut left: Vec<ScriptStyleDeclaration>,
    right: Vec<ScriptStyleDeclaration>,
) -> Vec<ScriptStyleDeclaration> {
    left.retain(|declaration| {
        right
            .iter()
            .any(|next| next.name == declaration.name && next.value == declaration.value)
    });
    left
}

fn merge_script_declarations(
    target: &mut Vec<ScriptStyleDeclaration>,
    declarations: impl IntoIterator<Item = ScriptStyleDeclaration>,
) {
    for declaration in declarations {
        if let Some(existing) = target
            .iter_mut()
            .find(|existing| existing.name == declaration.name)
        {
            *existing = declaration;
        } else {
            target.push(declaration);
        }
    }
}

fn callee_style_factory_name(expression: &Expression<'_>) -> Option<CompactString> {
    match expression {
        Expression::Identifier(identifier) => Some(CompactString::from(identifier.name.as_str())),
        Expression::StaticMemberExpression(member) => {
            Some(CompactString::from(member.property.name.as_str()))
        }
        Expression::ParenthesizedExpression(expression) => {
            callee_style_factory_name(&expression.expression)
        }
        _ => None,
    }
}

fn style_declarations_from_object(
    object: &ObjectExpression<'_>,
    span_base: Option<SpanBase>,
    aliases: &HashMap<CompactString, Vec<ScriptStyleDeclaration>>,
) -> Vec<ScriptStyleDeclaration> {
    let mut declarations = Vec::new();
    for property in &object.properties {
        match property {
            ObjectPropertyKind::ObjectProperty(property) => {
                if property.computed || property.method {
                    continue;
                }

                let Some(name) = property_key_name(&property.key).map(css_property_name) else {
                    continue;
                };
                let Some(value) = static_css_value(&property.value) else {
                    continue;
                };
                merge_script_declarations(
                    &mut declarations,
                    [ScriptStyleDeclaration {
                        name,
                        value,
                        span: span_base.map(|base| span_from_oxc(property.span, base)),
                    }],
                );
            }
            ObjectPropertyKind::SpreadProperty(spread) => match &spread.argument {
                Expression::Identifier(identifier) => {
                    if let Some(alias) = aliases.get(identifier.name.as_str()) {
                        merge_script_declarations(&mut declarations, alias.iter().cloned());
                    }
                }
                Expression::ObjectExpression(object) => {
                    merge_script_declarations(
                        &mut declarations,
                        style_declarations_from_object(object, span_base, aliases),
                    );
                }
                Expression::ParenthesizedExpression(expression) => {
                    if let Expression::Identifier(identifier) = &expression.expression {
                        if let Some(alias) = aliases.get(identifier.name.as_str()) {
                            merge_script_declarations(&mut declarations, alias.iter().cloned());
                        }
                    }
                }
                _ => {}
            },
        }
    }
    declarations
}

fn css_property_name(name: &str) -> CompactString {
    if let Some(rest) = name.strip_prefix("Webkit") {
        return CompactString::from(format!("-webkit-{}", rest.to_kebab_case()));
    }
    if let Some(rest) = name.strip_prefix("Moz") {
        return CompactString::from(format!("-moz-{}", rest.to_kebab_case()));
    }
    if name.starts_with("ms")
        && name
            .chars()
            .nth(2)
            .is_some_and(|ch| ch.is_ascii_uppercase())
    {
        return CompactString::from(format!("-ms-{}", name[2..].to_kebab_case()));
    }

    CompactString::from(name.to_kebab_case())
}

fn static_css_value(expression: &Expression<'_>) -> Option<CompactString> {
    match expression {
        Expression::StringLiteral(literal) => Some(CompactString::from(literal.value.as_str())),
        Expression::NumericLiteral(literal) => {
            Some(CompactString::from(literal.raw.as_ref().map_or_else(
                || format_number(literal.value),
                ToString::to_string,
            )))
        }
        Expression::BooleanLiteral(literal) => Some(CompactString::from(literal.value.to_string())),
        Expression::ParenthesizedExpression(expression) => static_css_value(&expression.expression),
        _ => None,
    }
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

fn binding_identifier_name<'a>(binding: &'a BindingPattern<'a>) -> Option<&'a str> {
    match binding {
        BindingPattern::BindingIdentifier(identifier) => Some(identifier.name.as_str()),
        _ => None,
    }
}

fn span_from_oxc(span: OxcSpan, base: SpanBase) -> Span {
    Span::new(
        base.source,
        base.offset + span.start as usize,
        base.offset + span.end as usize,
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ActionIndex {
    actions: HashMap<CompactString, ActionDefinition>,
}

impl ActionIndex {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn extend_module(&mut self, module: ScriptModule) {
        for action in module.actions {
            match self.actions.entry(action.name.clone()) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(action);
                }
                std::collections::hash_map::Entry::Occupied(mut entry)
                    if entry.get().handler.is_empty() && !action.handler.is_empty() =>
                {
                    entry.insert(action);
                }
                std::collections::hash_map::Entry::Occupied(_) => {}
            }
        }
    }

    pub(crate) fn resolve_handler(&self, handler: &mut RenderActionHandler) {
        let mut referenced_effects = Vec::new();
        for invocation in &mut handler.invocations {
            let Some(action) = self.actions.get(invocation.action.as_str()) else {
                continue;
            };
            invocation.resolved = true;
            invocation.action_span = action.span;
            referenced_effects.extend(action.handler.effects.iter().cloned());
        }

        for effect in referenced_effects {
            if !handler.effects.contains(&effect) {
                handler.effects.push(effect);
            }
        }
    }

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

#[must_use]
pub(crate) fn analyze_event_handler(
    handler: &str,
    span: Option<Span>,
    actions: Option<&ActionIndex>,
) -> RenderActionHandler {
    let mut analyzed = handler_from_source(handler, span.map(SpanBase::from));
    if let Some(actions) = actions {
        actions.resolve_handler(&mut analyzed);
    }
    analyzed
}

#[must_use]
pub(crate) fn action_name_from_handler(handler: &str) -> Option<CompactString> {
    let handler = handler.trim().trim_end_matches(';').trim();
    if handler.is_empty() {
        return None;
    }
    let handler = handler.strip_prefix("return ").unwrap_or(handler).trim();

    let candidate = handler
        .strip_suffix("()")
        .or_else(|| handler.split_once('(').map(|(name, _)| name))
        .unwrap_or(handler)
        .trim();

    is_identifier(candidate).then(|| CompactString::from(candidate))
}

fn is_identifier(candidate: &str) -> bool {
    let mut chars = candidate.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first == '_' || first == '$' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch == '$' || ch.is_ascii_alphanumeric())
}

fn handler_from_source(source: &str, span_base: Option<SpanBase>) -> RenderActionHandler {
    let allocator = Allocator::default();
    let parser = Parser::new(&allocator, source, SourceType::ts()).with_options(ParseOptions {
        parse_regular_expression: true,
        allow_return_outside_function: true,
        ..ParseOptions::default()
    });
    let parsed = parser.parse();
    if parsed.panicked {
        return fallback_handler_from_source(source, span_base);
    }

    let mut handler = RenderActionHandler::default();
    let event_names = default_event_names();
    for statement in &parsed.program.body {
        analyze_statement(statement, &event_names, span_base, &mut handler);
    }

    if handler.is_empty() {
        fallback_handler_from_source(source, span_base)
    } else {
        handler
    }
}

fn fallback_handler_from_source(source: &str, span_base: Option<SpanBase>) -> RenderActionHandler {
    let Some(action) = action_name_from_handler(source) else {
        return RenderActionHandler::default();
    };

    RenderActionHandler {
        invocations: vec![RenderActionInvocation {
            action,
            arguments: Vec::new(),
            resolved: false,
            span: span_base
                .map(|base| Span::new(base.source, base.offset, base.offset + source.len())),
            action_span: None,
        }],
        effects: Vec::new(),
    }
}

fn handler_from_function(
    function: &Function<'_>,
    span_base: Option<SpanBase>,
) -> RenderActionHandler {
    let Some(body) = &function.body else {
        return RenderActionHandler::default();
    };
    handler_from_body(body, event_parameter_names(&function.params), span_base)
}

fn handler_from_arrow_function(
    function: &ArrowFunctionExpression<'_>,
    span_base: Option<SpanBase>,
) -> RenderActionHandler {
    handler_from_body(
        &function.body,
        event_parameter_names(&function.params),
        span_base,
    )
}

fn handler_from_body(
    body: &FunctionBody<'_>,
    event_names: Vec<CompactString>,
    span_base: Option<SpanBase>,
) -> RenderActionHandler {
    let mut handler = RenderActionHandler::default();
    for statement in &body.statements {
        analyze_statement(statement, &event_names, span_base, &mut handler);
    }
    handler
}

fn event_parameter_names(params: &oxc_ast::ast::FormalParameters<'_>) -> Vec<CompactString> {
    let mut names = default_event_names();
    if let Some(first) = params.items.first()
        && let Some(name) = binding_identifier_name(&first.pattern)
        && !names.iter().any(|existing| existing == name)
    {
        names.push(CompactString::from(name));
    }
    names
}

fn default_event_names() -> Vec<CompactString> {
    vec![CompactString::from("event"), CompactString::from("e")]
}

fn analyze_statement(
    statement: &Statement<'_>,
    event_names: &[CompactString],
    span_base: Option<SpanBase>,
    handler: &mut RenderActionHandler,
) {
    match statement {
        Statement::ExpressionStatement(statement) => {
            analyze_expression(&statement.expression, event_names, span_base, handler);
        }
        Statement::ReturnStatement(statement) => {
            if let Some(argument) = &statement.argument {
                analyze_expression(argument, event_names, span_base, handler);
            }
        }
        Statement::BlockStatement(statement) => {
            for statement in &statement.body {
                analyze_statement(statement, event_names, span_base, handler);
            }
        }
        Statement::IfStatement(statement) => {
            analyze_statement(&statement.consequent, event_names, span_base, handler);
            if let Some(alternate) = &statement.alternate {
                analyze_statement(alternate, event_names, span_base, handler);
            }
        }
        _ => {}
    }
}

fn analyze_expression(
    expression: &Expression<'_>,
    event_names: &[CompactString],
    span_base: Option<SpanBase>,
    handler: &mut RenderActionHandler,
) {
    match expression {
        Expression::CallExpression(call) => analyze_call(call, event_names, span_base, handler),
        Expression::ParenthesizedExpression(expression) => {
            analyze_expression(&expression.expression, event_names, span_base, handler);
        }
        Expression::SequenceExpression(expression) => {
            for expression in &expression.expressions {
                analyze_expression(expression, event_names, span_base, handler);
            }
        }
        Expression::AssignmentExpression(expression) => {
            analyze_expression(&expression.right, event_names, span_base, handler);
        }
        Expression::AwaitExpression(expression) => {
            analyze_expression(&expression.argument, event_names, span_base, handler);
        }
        Expression::ChainExpression(expression) => {
            analyze_chain_element(&expression.expression, event_names, span_base, handler);
        }
        _ => {}
    }
}

fn analyze_chain_element(
    expression: &ChainElement<'_>,
    event_names: &[CompactString],
    span_base: Option<SpanBase>,
    handler: &mut RenderActionHandler,
) {
    if let ChainElement::CallExpression(call) = expression {
        analyze_call(call, event_names, span_base, handler);
    }
}

fn analyze_call(
    call: &oxc_ast::ast::CallExpression<'_>,
    event_names: &[CompactString],
    span_base: Option<SpanBase>,
    handler: &mut RenderActionHandler,
) {
    if let Some(effect) = handler_effect_from_call(call, event_names, span_base) {
        handler.effects.push(effect);
        return;
    }

    let Some(action) = callee_identifier(&call.callee) else {
        return;
    };
    let arguments = call
        .arguments
        .iter()
        .map(|argument| action_argument(argument, event_names))
        .collect();
    handler.invocations.push(RenderActionInvocation {
        action,
        arguments,
        resolved: false,
        span: span_base.map(|base| span_from_oxc(call.span, base)),
        action_span: None,
    });
}

fn handler_effect_from_call(
    call: &oxc_ast::ast::CallExpression<'_>,
    event_names: &[CompactString],
    span_base: Option<SpanBase>,
) -> Option<RenderActionHandlerEffect> {
    let Expression::StaticMemberExpression(member) = &call.callee else {
        return None;
    };
    if !expression_is_event(&member.object, event_names) {
        return None;
    }

    match member.property.name.as_str() {
        "preventDefault" => Some(RenderActionHandlerEffect::PreventDefault {
            span: span_base.map(|base| span_from_oxc(call.span, base)),
        }),
        "stopPropagation" => Some(RenderActionHandlerEffect::StopPropagation {
            span: span_base.map(|base| span_from_oxc(call.span, base)),
        }),
        _ => None,
    }
}

fn callee_identifier(expression: &Expression<'_>) -> Option<CompactString> {
    match expression {
        Expression::Identifier(identifier) => Some(CompactString::from(identifier.name.as_str())),
        Expression::ParenthesizedExpression(expression) => {
            callee_identifier(&expression.expression)
        }
        _ => None,
    }
}

fn action_argument(argument: &Argument<'_>, event_names: &[CompactString]) -> RenderActionArgument {
    match argument {
        Argument::Identifier(identifier)
            if identifier_is_event(identifier.name.as_str(), event_names) =>
        {
            RenderActionArgument::Event
        }
        Argument::Identifier(identifier) => {
            RenderActionArgument::Unknown(CompactString::from(identifier.name.as_str()))
        }
        Argument::ThisExpression(_) => RenderActionArgument::Element,
        Argument::StringLiteral(literal) => {
            RenderActionArgument::Literal(CompactString::from(literal.value.as_str()))
        }
        Argument::NumericLiteral(literal) => {
            RenderActionArgument::Literal(CompactString::from(literal.value.to_string()))
        }
        Argument::BooleanLiteral(literal) => {
            RenderActionArgument::Literal(CompactString::from(literal.value.to_string()))
        }
        Argument::StaticMemberExpression(member) => {
            action_argument_from_member(&member.object, member.property.name.as_str(), event_names)
        }
        Argument::ParenthesizedExpression(expression) => {
            action_argument_from_expression(&expression.expression, event_names)
        }
        _ => RenderActionArgument::Unknown(CompactString::from("expression")),
    }
}

fn action_argument_from_expression(
    expression: &Expression<'_>,
    event_names: &[CompactString],
) -> RenderActionArgument {
    match expression {
        Expression::Identifier(identifier)
            if identifier_is_event(identifier.name.as_str(), event_names) =>
        {
            RenderActionArgument::Event
        }
        Expression::ThisExpression(_) => RenderActionArgument::Element,
        Expression::StringLiteral(literal) => {
            RenderActionArgument::Literal(CompactString::from(literal.value.as_str()))
        }
        Expression::NumericLiteral(literal) => {
            RenderActionArgument::Literal(CompactString::from(literal.value.to_string()))
        }
        Expression::BooleanLiteral(literal) => {
            RenderActionArgument::Literal(CompactString::from(literal.value.to_string()))
        }
        Expression::StaticMemberExpression(member) => {
            action_argument_from_member(&member.object, member.property.name.as_str(), event_names)
        }
        Expression::ParenthesizedExpression(expression) => {
            action_argument_from_expression(&expression.expression, event_names)
        }
        _ => RenderActionArgument::Unknown(CompactString::from("expression")),
    }
}

fn action_argument_from_member(
    object: &Expression<'_>,
    property: &str,
    event_names: &[CompactString],
) -> RenderActionArgument {
    if expression_is_event_target(object, event_names) {
        return match property {
            "value" => RenderActionArgument::ElementValue,
            "checked" => RenderActionArgument::ElementChecked,
            _ => RenderActionArgument::Unknown(CompactString::from(format!("target.{property}"))),
        };
    }

    if expression_is_event(object, event_names) && matches!(property, "target" | "currentTarget") {
        return RenderActionArgument::Element;
    }

    RenderActionArgument::Unknown(CompactString::from(property))
}

fn expression_is_event(expression: &Expression<'_>, event_names: &[CompactString]) -> bool {
    matches!(expression, Expression::Identifier(identifier) if identifier_is_event(identifier.name.as_str(), event_names))
}

fn expression_is_event_target(expression: &Expression<'_>, event_names: &[CompactString]) -> bool {
    let Expression::StaticMemberExpression(member) = expression else {
        return false;
    };
    expression_is_event(&member.object, event_names)
        && matches!(member.property.name.as_str(), "target" | "currentTarget")
}

fn identifier_is_event(identifier: &str, event_names: &[CompactString]) -> bool {
    event_names.iter().any(|name| name == identifier)
}

#[must_use]
pub(crate) fn inline_scripts(document: &HtmlDocument) -> Vec<InlineScript> {
    let mut scripts = Vec::new();
    for node in &document.nodes {
        collect_inline_scripts(node, &mut scripts);
    }
    scripts
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InlineScript {
    pub(crate) source: String,
    pub(crate) span: Option<Span>,
}

#[must_use]
pub(crate) fn external_scripts(document: &HtmlDocument) -> Vec<ExternalScript> {
    let mut scripts = Vec::new();
    for node in &document.nodes {
        collect_external_scripts(node, &mut scripts);
    }
    scripts
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExternalScript {
    pub(crate) src: String,
    pub(crate) span: Option<Span>,
}

fn collect_inline_scripts(node: &HtmlNode, scripts: &mut Vec<InlineScript>) {
    let HtmlNode::Element(element) = node else {
        return;
    };

    if element.name.local().eq_ignore_ascii_case("script") {
        let script = script_text(element);
        if !script.source.trim().is_empty() {
            scripts.push(script);
        }
    }

    for child in &element.children {
        collect_inline_scripts(child, scripts);
    }
}

fn collect_external_scripts(node: &HtmlNode, scripts: &mut Vec<ExternalScript>) {
    let HtmlNode::Element(element) = node else {
        return;
    };

    if element.name.local().eq_ignore_ascii_case("script")
        && script_type_is_javascript(element)
        && let Some((src, span)) = html_attribute_value(element, "src")
    {
        let src = src.trim();
        if !src.is_empty() {
            scripts.push(ExternalScript {
                src: src.to_owned(),
                span,
            });
        }
    }

    for child in &element.children {
        collect_external_scripts(child, scripts);
    }
}

fn script_type_is_javascript(element: &HtmlElement) -> bool {
    let Some((script_type, _)) = html_attribute_value(element, "type") else {
        return true;
    };

    matches!(
        script_type.trim().to_ascii_lowercase().as_str(),
        "" | "module"
            | "text/javascript"
            | "application/javascript"
            | "text/ecmascript"
            | "application/ecmascript"
    )
}

fn html_attribute_value<'a>(
    element: &'a HtmlElement,
    name: &str,
) -> Option<(&'a str, Option<Span>)> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local().eq_ignore_ascii_case(name))
        .map(|attribute| (attribute.value.as_str(), attribute.span))
}

fn script_text(element: &HtmlElement) -> InlineScript {
    let mut source = String::new();
    let mut span_source = None;
    let mut span_start = None;
    let mut span_end = None;

    for child in &element.children {
        let HtmlNode::Text(text) = child else {
            continue;
        };

        if let Some(span) = text.span {
            span_source.get_or_insert(span.source);
            if span_start.is_none() {
                span_start = Some(span.start);
            }

            if span_end.is_none_or(|end| end == span.start) {
                span_end = Some(span.end);
            } else {
                span_start = None;
                span_end = None;
            }
        } else {
            span_start = None;
            span_end = None;
        }

        source.push_str(&text.value);
    }

    InlineScript {
        source,
        span: span_start
            .zip(span_end)
            .and_then(|(start, end)| span_source.map(|source| Span::new(source, start, end))),
    }
}
