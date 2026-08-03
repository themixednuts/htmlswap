use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use arcstr::ArcStr;
use compact_str::CompactString;
use heck::ToUpperCamelCase;
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, ArrayExpression, ArrayExpressionElement, ArrowFunctionExpression, AssignmentTarget,
    BindingPattern as OxcBindingPattern, CallExpression, Declaration, ExportDefaultDeclarationKind,
    Expression, Function, FunctionBody, JSXAttribute, JSXAttributeItem, JSXAttributeName,
    JSXAttributeValue, JSXChild, JSXElement, JSXElementName, JSXExpression, JSXExpressionContainer,
    JSXFragment, JSXMemberExpression, JSXMemberExpressionObject, ModuleExportName,
    ObjectExpression, ObjectPropertyKind, Program, PropertyKey, Statement, VariableDeclarationKind,
    VariableDeclarator,
};
use oxc_parser::{ParseOptions, Parser};
use oxc_span::{GetSpan, SourceType, Span as OxcSpan};
use oxc_syntax::operator::AssignmentOperator;

use crate::adapter::{Adapter, AdapterArtifact, AdapterContext, ArtifactBuilder, GeneratedFile};
use crate::adapters::svelte::{SvelteAdapter, SvelteAdapterOptions};
use crate::bundle::{BundleConfig, BundlePlan};
use crate::compiler::CompiledFragment;
use crate::diagnostics::{Compilation, Diagnostic, Diagnostics};
use crate::expr::{
    BindingPattern, Expr, ExprLiteral, ObjectEntry, TemplateSegment, TemplateString,
};
use crate::plan::{
    ComponentId, RenderAttribute, RenderControlFlow, RenderControlFlowHost, RenderControlFlowKind,
    RenderDynamicStyleBinding, RenderElement, RenderLoopLocal, RenderNode, RenderPlan,
    RenderSourceCallback, RenderSourceComponentLogic, RenderSourceDerived, RenderSourceEffect,
    RenderSourceIntent, RenderSourceLocal, RenderSourceLogic, RenderSourceLogicItem,
    RenderSourceMount, RenderSourceProp, RenderSourceRef, RenderSourceSnippet, RenderSourceState,
    RenderText, UiRole,
};
use crate::source::{SourceId, SourceKind, SourceMap, Span};
use crate::style::StyleDeclaration;

pub(crate) fn lower_jsx_module(
    source: &str,
    source_name: Option<&str>,
    source_id: SourceId,
) -> Compilation<RenderPlan> {
    lower_jsx_module_with_component(source, source_name, source_id, None, true)
}

fn lower_jsx_project_component_module(
    source: &str,
    source_name: Option<&str>,
    source_id: SourceId,
    component_name: &str,
) -> Compilation<RenderPlan> {
    lower_jsx_module_with_component(source, source_name, source_id, Some(component_name), false)
}

fn lower_jsx_module_with_component(
    source: &str,
    source_name: Option<&str>,
    source_id: SourceId,
    component_name: Option<&str>,
    inline_published_components: bool,
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
        component_names: BTreeSet::new(),
        function_components: BTreeMap::new(),
        published_component_names: BTreeSet::new(),
        setters: BTreeMap::new(),
        reactive_names: BTreeSet::new(),
        refs: BTreeMap::new(),
        snippets: BTreeSet::new(),
        source_aliases: BTreeMap::new(),
        inlined_component_items: Vec::new(),
        inlining_components: BTreeSet::new(),
        inline_published_components,
    };
    let plan = lowerer.lower_program(&parsed.program, component_name);
    Compilation::new(plan, lowerer.diagnostics)
}

fn source_type_for(source_name: Option<&str>) -> SourceType {
    source_name
        .and_then(|name| SourceType::from_path(Path::new(name)).ok())
        .unwrap_or_else(SourceType::jsx)
}

#[derive(Debug, Clone)]
pub(crate) struct JsxProjectSource {
    pub name: String,
    pub source: ArcStr,
}

impl JsxProjectSource {
    pub(crate) fn new(name: impl Into<String>, source: impl Into<ArcStr>) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
        }
    }
}

#[derive(Debug, Clone)]
struct JsxProjectRegistry {
    symbols: BTreeMap<CompactString, JsxProjectSymbol>,
    modules: BTreeMap<usize, JsxProjectModule>,
    diagnostics: Diagnostics,
}

#[derive(Debug, Clone)]
struct JsxProjectModule {
    path: String,
    body: String,
}

#[derive(Debug, Clone)]
struct JsxProjectSymbol {
    name: CompactString,
    source_index: usize,
    output_name: CompactString,
    kind: JsxProjectSymbolKind,
}

#[derive(Debug, Clone)]
enum JsxProjectSymbolKind {
    Component(JsxProjectComponentEmitter),
    Value,
    Alias(CompactString),
}

#[derive(Debug, Clone)]
enum JsxProjectComponentEmitter {
    Function { local_name: CompactString },
    Synthetic { source: ArcStr },
}

#[derive(Debug, Clone)]
struct JsxProjectLocal {
    kind: JsxProjectLocalKind,
}

#[derive(Debug, Clone)]
enum JsxProjectLocalKind {
    Component(JsxProjectComponentEmitter),
    Value,
}

#[derive(Debug, Clone)]
struct JsxProjectPublish {
    name: CompactString,
    target: CompactString,
}

#[derive(Debug, Clone)]
struct ResolvedComponentSymbol<'a> {
    output_name: &'a str,
}

pub(crate) fn compile_svelte_project(
    sources: Vec<JsxProjectSource>,
    options: SvelteAdapterOptions,
) -> Compilation<AdapterArtifact> {
    let registry = scan_jsx_project(&sources);
    let mut diagnostics = registry.diagnostics.clone();
    let mut adapter_context = AdapterContext::new();
    let mut artifact = ArtifactBuilder::new();

    for module in registry.modules.values() {
        artifact.add_file(
            GeneratedFile::new(
                module.path.clone(),
                rewrite_project_ts_value_references(&module.path, &module.body, &registry),
            ),
            &mut adapter_context,
        );
    }

    for symbol in registry.component_symbols() {
        let Some((source_name, source_text, requested_component)) =
            project_component_source(&sources, symbol)
        else {
            diagnostics.push(Diagnostic::error(
                format!("JSX project component `{}` has no source", symbol.name),
                None,
            ));
            continue;
        };
        let mut source_map = SourceMap::new();
        let source_id = source_map.add_file(
            SourceKind::JavaScript,
            Some(source_name),
            source_text.clone(),
        );
        let mut lowered = lower_jsx_project_component_module(
            source_text.as_str(),
            source_map.file(source_id).and_then(|file| file.name()),
            source_id,
            requested_component.as_str(),
        );
        diagnostics.extend(lowered.diagnostics);
        resolve_project_components(&mut lowered.value, &registry);

        let fragment = CompiledFragment {
            bundle: BundlePlan::from_render_plan_with_config(
                &lowered.value,
                &source_map,
                &BundleConfig::default(),
            ),
            plan: lowered.value,
            sources: source_map,
        };
        let output = SvelteAdapter::new(SvelteAdapterOptions {
            component_name: symbol.output_name.clone(),
            ..options.clone()
        })
        .adapt(&fragment, &mut adapter_context);
        match output {
            Ok(output) => {
                for mut file in output.artifact.files {
                    if file.path.ends_with(".svelte") {
                        file.contents = rewrite_project_value_references(&file.contents, &registry);
                    }
                    artifact.add_file(file, &mut adapter_context);
                }
                for dependency in output.artifact.dependencies {
                    artifact.add_dependency(dependency, &mut adapter_context);
                }
            }
            Err(error) => diagnostics.push(Diagnostic::error(
                format!(
                    "Svelte adapter failed for JSX project component `{}`: {error}",
                    symbol.name
                ),
                None,
            )),
        }
    }

    diagnostics.extend(adapter_context.into_diagnostics());
    Compilation::new(artifact.finish(), diagnostics)
}

impl JsxProjectRegistry {
    fn new() -> Self {
        Self {
            symbols: BTreeMap::new(),
            modules: BTreeMap::new(),
            diagnostics: Diagnostics::new(),
        }
    }

    fn component_symbols(&self) -> Vec<&JsxProjectSymbol> {
        self.symbols
            .values()
            .filter(|symbol| matches!(symbol.kind, JsxProjectSymbolKind::Component(_)))
            .collect()
    }

    fn value_symbols(&self) -> Vec<&JsxProjectSymbol> {
        self.symbols
            .values()
            .filter(|symbol| matches!(symbol.kind, JsxProjectSymbolKind::Value))
            .collect()
    }

    fn insert_symbol(&mut self, symbol: JsxProjectSymbol) {
        if let Some(existing) = self.symbols.get(symbol.name.as_str()) {
            if existing.source_index != symbol.source_index
                || existing.output_name != symbol.output_name
                || std::mem::discriminant(&existing.kind) != std::mem::discriminant(&symbol.kind)
            {
                self.diagnostics.push(Diagnostic::warning(
                    format!(
                        "JSX project symbol `{}` is published more than once; keeping first definition",
                        symbol.name
                    ),
                    None,
                ));
            }
            return;
        }
        self.symbols.insert(symbol.name.clone(), symbol);
    }

    fn resolve_component(&self, name: &str) -> Option<ResolvedComponentSymbol<'_>> {
        let mut current = name;
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(current.to_owned()) {
                return None;
            }
            let symbol = self.symbols.get(current)?;
            match &symbol.kind {
                JsxProjectSymbolKind::Component(_) => {
                    return Some(ResolvedComponentSymbol {
                        output_name: symbol.output_name.as_str(),
                    });
                }
                JsxProjectSymbolKind::Alias(target) => {
                    current = target.as_str();
                }
                JsxProjectSymbolKind::Value => return None,
            }
        }
    }

    fn module_for_value(&self, name: &str) -> Option<&str> {
        let symbol = self.symbols.get(name)?;
        if !matches!(symbol.kind, JsxProjectSymbolKind::Value) {
            return None;
        }
        self.modules
            .get(&symbol.source_index)
            .map(|module| module.path.as_str())
    }
}

fn scan_jsx_project(sources: &[JsxProjectSource]) -> JsxProjectRegistry {
    let mut registry = JsxProjectRegistry::new();
    for (source_index, source) in sources.iter().enumerate() {
        scan_jsx_project_source(source_index, source, &mut registry);
    }
    registry
}

fn scan_jsx_project_source(
    source_index: usize,
    source: &JsxProjectSource,
    registry: &mut JsxProjectRegistry,
) {
    let allocator = Allocator::default();
    let parser = Parser::new(
        &allocator,
        source.source.as_str(),
        source_type_for(Some(&source.name)),
    )
    .with_options(ParseOptions {
        parse_regular_expression: true,
        ..ParseOptions::default()
    });
    let parsed = parser.parse();
    for diagnostic in parsed.diagnostics {
        registry
            .diagnostics
            .push(Diagnostic::warning(diagnostic.to_string(), None));
    }
    if parsed.panicked {
        registry.diagnostics.push(Diagnostic::error(
            format!("JSX parser stopped early while scanning `{}`", source.name),
            None,
        ));
        return;
    }

    let mut locals = BTreeMap::new();
    collect_project_locals(source, source_index, &parsed.program, &mut locals, registry);
    for (name, local) in &locals {
        if let JsxProjectLocalKind::Component(emitter) = &local.kind {
            registry.insert_symbol(JsxProjectSymbol {
                name: name.clone(),
                source_index,
                output_name: project_component_name(name),
                kind: JsxProjectSymbolKind::Component(emitter.clone()),
            });
        }
    }

    for publish in collect_project_publishes(source.source.as_str(), &parsed.program) {
        if let Some(local) = locals.get(publish.target.as_str()) {
            match &local.kind {
                JsxProjectLocalKind::Component(emitter) => {
                    if publish.name == publish.target {
                        registry.insert_symbol(JsxProjectSymbol {
                            name: publish.name,
                            source_index,
                            output_name: project_component_name(&publish.target),
                            kind: JsxProjectSymbolKind::Component(emitter.clone()),
                        });
                    } else {
                        registry.insert_symbol(JsxProjectSymbol {
                            output_name: project_component_name(&publish.name),
                            name: publish.name,
                            source_index,
                            kind: JsxProjectSymbolKind::Alias(publish.target),
                        });
                    }
                }
                JsxProjectLocalKind::Value => {
                    registry.insert_symbol(JsxProjectSymbol {
                        output_name: publish.name.clone(),
                        name: publish.name,
                        source_index,
                        kind: JsxProjectSymbolKind::Value,
                    });
                }
            }
        } else if publish.name != publish.target {
            registry.insert_symbol(JsxProjectSymbol {
                output_name: project_component_name(&publish.name),
                name: publish.name,
                source_index,
                kind: JsxProjectSymbolKind::Alias(publish.target),
            });
        }
    }

    collect_project_icon_extensions(source, source_index, registry);

    if let Some(body) = emit_project_ts_module(source, &parsed.program, &locals) {
        registry.modules.insert(
            source_index,
            JsxProjectModule {
                path: project_module_path(&source.name),
                body,
            },
        );
    }
}

fn collect_project_locals(
    source: &JsxProjectSource,
    source_index: usize,
    program: &Program<'_>,
    locals: &mut BTreeMap<CompactString, JsxProjectLocal>,
    registry: &mut JsxProjectRegistry,
) {
    for statement in &program.body {
        match statement {
            Statement::FunctionDeclaration(function) => {
                let Some(id) = &function.id else {
                    continue;
                };
                let name = CompactString::from(id.name.as_str());
                let kind = if function_body_contains_jsx(&function.body, source.source.as_str()) {
                    JsxProjectLocalKind::Component(JsxProjectComponentEmitter::Function {
                        local_name: name.clone(),
                    })
                } else {
                    JsxProjectLocalKind::Value
                };
                locals.insert(name, JsxProjectLocal { kind });
            }
            Statement::VariableDeclaration(declaration) => {
                for declarator in &declaration.declarations {
                    let Some(name) = binding_identifier_name(&declarator.id) else {
                        continue;
                    };
                    let mut local_kind = JsxProjectLocalKind::Value;
                    if let Some(init) = &declarator.init {
                        if let Some(emitter) =
                            project_component_emitter_for_expression(source, name.as_str(), init)
                        {
                            local_kind = JsxProjectLocalKind::Component(emitter);
                        }
                        if let Expression::ObjectExpression(object) = init {
                            collect_project_object_components(
                                source,
                                source_index,
                                name.as_str(),
                                object,
                                locals,
                                registry,
                            );
                        }
                    }
                    locals.insert(name, JsxProjectLocal { kind: local_kind });
                }
            }
            _ => {}
        }
    }
}

fn collect_project_object_components(
    source: &JsxProjectSource,
    source_index: usize,
    object_name: &str,
    object: &ObjectExpression<'_>,
    locals: &mut BTreeMap<CompactString, JsxProjectLocal>,
    registry: &mut JsxProjectRegistry,
) {
    for property in &object.properties {
        let ObjectPropertyKind::ObjectProperty(property) = property else {
            continue;
        };
        let Some(key) = project_property_key(&property.key) else {
            continue;
        };
        let public_name = CompactString::from(format!("{object_name}.{key}"));
        let component_name = project_component_name(&public_name);
        let Some(emitter) = project_component_emitter_for_expression(
            source,
            component_name.as_str(),
            &property.value,
        ) else {
            continue;
        };
        locals.insert(
            public_name.clone(),
            JsxProjectLocal {
                kind: JsxProjectLocalKind::Component(emitter.clone()),
            },
        );
        registry.insert_symbol(JsxProjectSymbol {
            name: public_name,
            source_index,
            output_name: component_name,
            kind: JsxProjectSymbolKind::Component(emitter),
        });
    }
}

fn project_component_emitter_for_expression(
    source: &JsxProjectSource,
    component_name: &str,
    expression: &Expression<'_>,
) -> Option<JsxProjectComponentEmitter> {
    if !expression_contains_jsx(expression, source.source.as_str()) {
        return None;
    }
    let synthetic = match expression {
        Expression::ArrowFunctionExpression(function) => {
            synthetic_arrow_component_source(source.source.as_str(), component_name, function)?
        }
        Expression::FunctionExpression(function) => {
            synthetic_function_component_source(source.source.as_str(), component_name, function)?
        }
        _ => return None,
    };
    Some(JsxProjectComponentEmitter::Synthetic {
        source: ArcStr::from(synthetic),
    })
}

fn collect_project_publishes(source: &str, program: &Program<'_>) -> Vec<JsxProjectPublish> {
    let mut publishes = Vec::new();
    for statement in &program.body {
        collect_project_statement_publishes(source, statement, &mut publishes);
    }
    publishes
}

fn collect_project_statement_publishes(
    source: &str,
    statement: &Statement<'_>,
    publishes: &mut Vec<JsxProjectPublish>,
) {
    match statement {
        Statement::ExpressionStatement(statement) => {
            collect_project_expression_publishes(source, &statement.expression, publishes);
        }
        Statement::IfStatement(statement) => {
            collect_project_statement_publishes(source, &statement.consequent, publishes);
            if let Some(alternate) = &statement.alternate {
                collect_project_statement_publishes(source, alternate, publishes);
            }
        }
        Statement::BlockStatement(block) => {
            for statement in &block.body {
                collect_project_statement_publishes(source, statement, publishes);
            }
        }
        _ => {}
    }
}

fn collect_project_expression_publishes(
    source: &str,
    expression: &Expression<'_>,
    publishes: &mut Vec<JsxProjectPublish>,
) {
    match expression {
        Expression::AssignmentExpression(assignment) => {
            if assignment.operator != AssignmentOperator::Assign {
                return;
            }
            let Some(name) = project_window_assignment_name(&assignment.left) else {
                return;
            };
            let Some(target) = project_expression_reference_name(source, &assignment.right) else {
                return;
            };
            publishes.push(JsxProjectPublish { name, target });
        }
        Expression::CallExpression(call) if call_is_object_assign_window(call) => {
            let Some(Argument::ObjectExpression(object)) = call.arguments.get(1) else {
                return;
            };
            for property in &object.properties {
                let ObjectPropertyKind::ObjectProperty(property) = property else {
                    continue;
                };
                let Some(name) = project_property_key(&property.key) else {
                    continue;
                };
                let target = project_expression_reference_name(source, &property.value)
                    .unwrap_or(name.clone());
                publishes.push(JsxProjectPublish { name, target });
            }
        }
        _ => {}
    }
}

fn collect_project_icon_extensions(
    source: &JsxProjectSource,
    source_index: usize,
    registry: &mut JsxProjectRegistry,
) {
    for line in source.source.lines() {
        let Some(rest) = line.trim().strip_prefix("if (!I.") else {
            continue;
        };
        let Some((icon, after_icon)) = rest.split_once(')') else {
            continue;
        };
        let Some((_, after_mk)) = after_icon.split_once("mk(") else {
            continue;
        };
        let Some(path) = first_js_string_literal(after_mk) else {
            continue;
        };
        let public_name = CompactString::from(format!("Icon.{icon}"));
        let component_name = project_component_name(&public_name);
        let synthetic = format!(
            "function {component_name}(p = {{}}) {{
  return <svg width={{p.size || 12}} height={{p.size || 12}} viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" strokeWidth=\"2\" strokeLinecap=\"round\" strokeLinejoin=\"round\" {{...p}}>{path}</svg>;
}}
window.{component_name} = {component_name};
"
        );
        registry.insert_symbol(JsxProjectSymbol {
            name: public_name,
            source_index,
            output_name: component_name,
            kind: JsxProjectSymbolKind::Component(JsxProjectComponentEmitter::Synthetic {
                source: ArcStr::from(synthetic),
            }),
        });
    }
}

fn emit_project_ts_module(
    source: &JsxProjectSource,
    program: &Program<'_>,
    locals: &BTreeMap<CompactString, JsxProjectLocal>,
) -> Option<String> {
    let mut statements = Vec::new();
    for statement in &program.body {
        if project_statement_is_publish(statement)
            || statement_contains_jsx(statement, source.source.as_str())
            || project_statement_declares_component(statement, locals)
        {
            continue;
        }
        let text = source_for_oxc_span(source.source.as_str(), statement.span()).trim();
        if text.is_empty() {
            continue;
        }
        if matches!(
            statement,
            Statement::FunctionDeclaration(_) | Statement::VariableDeclaration(_)
        ) {
            statements.push(format!("export {text}"));
        }
    }
    (!statements.is_empty()).then(|| {
        let mut body = statements.join("\n\n");
        body.push('\n');
        body
    })
}

fn project_statement_declares_component(
    statement: &Statement<'_>,
    locals: &BTreeMap<CompactString, JsxProjectLocal>,
) -> bool {
    match statement {
        Statement::FunctionDeclaration(function) => function.id.as_ref().is_some_and(|id| {
            matches!(
                locals.get(id.name.as_str()).map(|local| &local.kind),
                Some(JsxProjectLocalKind::Component(_))
            )
        }),
        Statement::VariableDeclaration(declaration) => {
            declaration.declarations.iter().any(|declarator| {
                binding_identifier_name(&declarator.id).is_some_and(|name| {
                    matches!(
                        locals.get(name.as_str()).map(|local| &local.kind),
                        Some(JsxProjectLocalKind::Component(_))
                    )
                })
            })
        }
        _ => false,
    }
}

fn project_statement_is_publish(statement: &Statement<'_>) -> bool {
    match statement {
        Statement::ExpressionStatement(statement) => {
            project_expression_is_publish(&statement.expression)
        }
        Statement::IfStatement(statement) => {
            project_statement_is_publish(&statement.consequent)
                || statement
                    .alternate
                    .as_ref()
                    .is_some_and(|alternate| project_statement_is_publish(alternate))
        }
        _ => false,
    }
}

fn project_expression_is_publish(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::AssignmentExpression(assignment) => {
            assignment.operator == AssignmentOperator::Assign
                && project_window_assignment_name(&assignment.left).is_some()
        }
        Expression::CallExpression(call) => call_is_object_assign_window(call),
        _ => false,
    }
}

fn project_component_source(
    sources: &[JsxProjectSource],
    symbol: &JsxProjectSymbol,
) -> Option<(String, ArcStr, CompactString)> {
    match &symbol.kind {
        JsxProjectSymbolKind::Component(JsxProjectComponentEmitter::Function { local_name }) => {
            let source = sources.get(symbol.source_index)?;
            Some((
                source.name.clone(),
                source.source.clone(),
                local_name.clone(),
            ))
        }
        JsxProjectSymbolKind::Component(JsxProjectComponentEmitter::Synthetic { source }) => {
            Some((
                format!("{}.jsx", symbol.output_name),
                source.clone(),
                symbol.output_name.clone(),
            ))
        }
        JsxProjectSymbolKind::Value | JsxProjectSymbolKind::Alias(_) => None,
    }
}

fn resolve_project_components(plan: &mut RenderPlan, registry: &JsxProjectRegistry) {
    for node in &mut plan.nodes {
        resolve_project_component_node(node, registry);
    }
    for logic in &mut plan.source_logic {
        let Some(component) = &mut logic.component else {
            continue;
        };
        for item in &mut component.items {
            let RenderSourceLogicItem::Snippet(snippet) = item else {
                continue;
            };
            for node in &mut snippet.nodes {
                resolve_project_component_node(node, registry);
            }
        }
    }
}

fn resolve_project_component_node(node: &mut RenderNode, registry: &JsxProjectRegistry) {
    let RenderNode::Element(element) = node else {
        return;
    };
    if element.source_tag == "jsx-component"
        && let Some(intent) = element.source_intent.as_deref_mut()
        && let Some(component) = intent.component.as_ref()
        && let Some(resolved) = registry.resolve_component(component.as_str())
    {
        intent.component = Some(ComponentId::new(resolved.output_name));
        intent.component_source = Some(format!("./{}.svelte", resolved.output_name).into());
    }
    for child in &mut element.children {
        resolve_project_component_node(child, registry);
    }
}

fn rewrite_project_value_references(code: &str, registry: &JsxProjectRegistry) -> String {
    let (rewritten, imports) = rewrite_project_value_references_with_imports(code, registry, None);
    insert_svelte_project_imports(&rewritten, imports)
}

fn rewrite_project_ts_value_references(
    module_path: &str,
    code: &str,
    registry: &JsxProjectRegistry,
) -> String {
    let (rewritten, imports) =
        rewrite_project_value_references_with_imports(code, registry, Some(module_path));
    insert_ts_project_imports(&rewritten, imports)
}

fn rewrite_project_value_references_with_imports(
    code: &str,
    registry: &JsxProjectRegistry,
    current_module: Option<&str>,
) -> (String, BTreeMap<String, BTreeSet<String>>) {
    let mut rewritten = code.to_owned();
    let mut imports: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut values = registry.value_symbols();
    values.sort_by_key(|value| std::cmp::Reverse(value.name.len()));
    for symbol in values {
        let name = symbol.name.as_str();
        let needle = format!("window.{name}");
        if !rewritten.contains(&needle) {
            continue;
        }
        rewritten = rewritten.replace(&needle, name);
        if let Some(module) = registry.module_for_value(name) {
            insert_project_import(&mut imports, current_module, module, name);
        }
    }
    rewritten =
        rewrite_project_window_destructures(&rewritten, registry, current_module, &mut imports);
    (rewritten, imports)
}

fn rewrite_project_window_destructures(
    code: &str,
    registry: &JsxProjectRegistry,
    current_module: Option<&str>,
    imports: &mut BTreeMap<String, BTreeSet<String>>,
) -> String {
    let mut output = String::new();
    for line in code.lines() {
        let trimmed = line.trim();
        let destructured = trimmed
            .strip_prefix("const {")
            .and_then(|rest| rest.split_once("} = window"))
            .map(|(names, _)| names);
        let Some(names) = destructured else {
            output.push_str(line);
            output.push('\n');
            continue;
        };
        let names = names
            .split(',')
            .map(|name| name.trim())
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>();
        if names.is_empty()
            || names.iter().any(|name| {
                !registry.symbols.contains_key(*name)
                    || matches!(
                        registry.symbols.get(*name).map(|symbol| &symbol.kind),
                        Some(JsxProjectSymbolKind::Alias(_))
                    )
            })
        {
            output.push_str(line);
            output.push('\n');
            continue;
        }
        for name in names {
            if let Some(module) = registry.module_for_value(name) {
                insert_project_import(imports, current_module, module, name);
            }
        }
    }
    output
}

fn insert_project_import(
    imports: &mut BTreeMap<String, BTreeSet<String>>,
    current_module: Option<&str>,
    module: &str,
    name: &str,
) {
    if current_module == Some(module) {
        return;
    }
    imports
        .entry(format!("./{module}"))
        .or_default()
        .insert(name.to_owned());
}

fn insert_svelte_project_imports(
    code: &str,
    imports: BTreeMap<String, BTreeSet<String>>,
) -> String {
    if imports.is_empty() {
        return code.to_owned();
    }
    let import_block = imports
        .into_iter()
        .map(|(module, names)| {
            format!(
                "\timport {{ {} }} from {};",
                names.into_iter().collect::<Vec<_>>().join(", "),
                js_string_literal(&module)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let insert_after = code
        .find("<script")
        .and_then(|script| code[script..].find('\n').map(|offset| script + offset));
    if let Some(position) = insert_after {
        let mut output = String::with_capacity(code.len() + import_block.len() + 1);
        output.push_str(&code[..=position]);
        output.push_str(&import_block);
        output.push('\n');
        output.push_str(&code[position + 1..]);
        output
    } else {
        format!("{import_block}\n{code}")
    }
}

fn insert_ts_project_imports(code: &str, imports: BTreeMap<String, BTreeSet<String>>) -> String {
    if imports.is_empty() {
        return code.to_owned();
    }
    let import_block = imports
        .into_iter()
        .map(|(module, names)| {
            format!(
                "import {{ {} }} from {};\n",
                names.into_iter().collect::<Vec<_>>().join(", "),
                js_string_literal(&module)
            )
        })
        .collect::<String>();
    format!("{import_block}\n{code}")
}

fn synthetic_arrow_component_source(
    source: &str,
    component_name: &str,
    function: &ArrowFunctionExpression<'_>,
) -> Option<String> {
    let params = function
        .params
        .items
        .iter()
        .map(|param| {
            source_for_oxc_span(source, param.pattern.span())
                .trim()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join(", ");
    let body = source_for_oxc_span(source, function.body.span()).trim();
    let body = if function.expression {
        format!("return ({body});")
    } else {
        trim_js_block(body).to_owned()
    };
    Some(format!(
        "function {component_name}({params}) {{\n{body}\n}}\nwindow.{component_name} = {component_name};\n"
    ))
}

fn synthetic_function_component_source(
    source: &str,
    component_name: &str,
    function: &Function<'_>,
) -> Option<String> {
    let params = function
        .params
        .items
        .iter()
        .map(|param| {
            source_for_oxc_span(source, param.pattern.span())
                .trim()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join(", ");
    let body = function
        .body
        .as_deref()
        .map(|body| source_for_oxc_span(source, body.span()).trim())
        .map(trim_js_block)?;
    Some(format!(
        "function {component_name}({params}) {{\n{body}\n}}\nwindow.{component_name} = {component_name};\n"
    ))
}

fn trim_js_block(body: &str) -> &str {
    body.trim()
        .strip_prefix('{')
        .and_then(|body| body.strip_suffix('}'))
        .map(str::trim)
        .unwrap_or(body)
}

fn project_window_assignment_name(target: &AssignmentTarget<'_>) -> Option<CompactString> {
    let AssignmentTarget::StaticMemberExpression(member) = target else {
        return None;
    };
    if !matches!(&member.object, Expression::Identifier(identifier) if identifier.name.as_str() == "window")
    {
        return None;
    }
    Some(member.property.name.as_str().into())
}

fn project_expression_reference_name(
    source: &str,
    expression: &Expression<'_>,
) -> Option<CompactString> {
    match expression {
        Expression::Identifier(identifier) => Some(identifier.name.as_str().into()),
        Expression::StaticMemberExpression(_) => {
            let raw = source_for_oxc_span(source, expression.span()).trim();
            raw.strip_prefix("window.")
                .filter(|name| is_dotted_identifier(name))
                .map(CompactString::from)
        }
        _ => None,
    }
}

fn project_property_key(key: &PropertyKey<'_>) -> Option<CompactString> {
    match key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.as_str().into()),
        PropertyKey::StringLiteral(value) => Some(value.value.as_str().into()),
        _ => None,
    }
}

fn source_for_oxc_span(source: &str, span: OxcSpan) -> &str {
    source
        .get(span.start as usize..span.end as usize)
        .unwrap_or_default()
}

fn project_component_name(name: &str) -> CompactString {
    if name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        let mut output = String::with_capacity(name.len());
        for (index, ch) in name.chars().enumerate() {
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
        return output.into();
    }
    name.to_upper_camel_case().into()
}

fn project_module_path(source_name: &str) -> String {
    let stem = source_name
        .rsplit(['/', '\\'])
        .next()
        .and_then(|name| {
            name.strip_suffix(".jsx")
                .or_else(|| name.strip_suffix(".tsx"))
        })
        .filter(|stem| !stem.is_empty())
        .unwrap_or("module");
    format!("{stem}.ts")
}

fn is_dotted_identifier(value: &str) -> bool {
    value
        .split('.')
        .all(|part| is_simple_identifier(part) || component_identifier_name(part))
}

fn first_js_string_literal(source: &str) -> Option<String> {
    let source = source.trim_start();
    let quote = source.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let mut escaped = false;
    let mut output = String::new();
    for ch in source[quote.len_utf8()..].chars() {
        if escaped {
            output.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            output.push(ch);
            continue;
        }
        if ch == quote {
            return Some(output);
        }
        output.push(ch);
    }
    None
}

fn js_string_literal(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('\'');
    for ch in value.chars() {
        match ch {
            '\\' => output.push_str("\\\\"),
            '\'' => output.push_str("\\'"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            _ => output.push(ch),
        }
    }
    output.push('\'');
    output
}

struct JsxLowerer<'a> {
    source: &'a str,
    source_id: SourceId,
    diagnostics: Diagnostics,
    component_names: BTreeSet<CompactString>,
    function_components: BTreeMap<CompactString, &'a Function<'a>>,
    published_component_names: BTreeSet<CompactString>,
    setters: BTreeMap<CompactString, CompactString>,
    reactive_names: BTreeSet<CompactString>,
    refs: BTreeMap<CompactString, bool>,
    snippets: BTreeSet<CompactString>,
    source_aliases: BTreeMap<CompactString, CompactString>,
    inlined_component_items: Vec<RenderSourceLogicItem>,
    inlining_components: BTreeSet<CompactString>,
    inline_published_components: bool,
}

struct JsxComponent<'a> {
    name: CompactString,
    function: &'a Function<'a>,
    module_end: usize,
}

struct JsxSourceLogicPrelude {
    body: String,
    component: Option<RenderSourceComponentLogic>,
}

struct ComponentPropInit {
    name: CompactString,
    default: Option<CompactString>,
}

impl<'a> JsxLowerer<'a> {
    fn lower_program(
        &mut self,
        program: &'a Program<'a>,
        requested_component: Option<&str>,
    ) -> RenderPlan {
        self.component_names = self.discover_component_names(program);
        self.function_components = self.discover_function_components(program);
        self.published_component_names = self.discover_published_component_names(program);
        self.inlined_component_items.clear();
        self.inlining_components.clear();
        let component = requested_component
            .and_then(|name| self.find_named_component(program, name))
            .or_else(|| self.find_component(program));
        let Some(component) = component else {
            let mut plan = RenderPlan::new(Vec::new());
            let prelude = self.module_source_logic_prelude(program, usize::MAX);
            if !prelude.trim().is_empty() || !self.component_names.is_empty() {
                if !prelude.trim().is_empty() {
                    plan.source_logic.push(RenderSourceLogic {
                        dialect: "jsx".into(),
                        script_type: Some("module".into()),
                        body: ArcStr::from(prelude),
                        data_props: None,
                        component: None,
                        span: None,
                    });
                }
                return plan;
            }
            self.diagnostics.push(Diagnostic::error(
                "JSX source must expose a function component through window.Name, export, export default, or Object.assign(window, ...) in Phase 2",
                None,
            ));
            return plan;
        };
        if let Some(requested_component) = requested_component
            && component.name != requested_component
        {
            self.diagnostics.push(Diagnostic::error(
                format!("JSX component `{requested_component}` was not found"),
                None,
            ));
            return RenderPlan::new(Vec::new());
        }
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

        let props = self.function_props_pattern(component.function);
        self.collect_component_bindings(body, props.as_deref());
        let mut prelude = self.source_logic_prelude(program, body, &component);
        let nodes = self.lower_component_return_nodes(body, return_expression);
        if !self.inlined_component_items.is_empty() {
            prelude
                .component
                .get_or_insert_with(|| RenderSourceComponentLogic { items: Vec::new() })
                .items
                .append(&mut self.inlined_component_items);
        }
        let mut plan = RenderPlan::new(nodes);
        let props = self.function_props_pattern(component.function);
        if !prelude.body.trim().is_empty() || props.is_some() || prelude.component.is_some() {
            plan.source_logic.push(RenderSourceLogic {
                dialect: "jsx".into(),
                script_type: Some("module".into()),
                body: ArcStr::from(prelude.body),
                data_props: props,
                component: prelude.component,
                span: Some(self.span(component.function.span)),
            });
        }
        plan
    }

    fn discover_component_names(&self, program: &'a Program<'a>) -> BTreeSet<CompactString> {
        let mut names = BTreeSet::new();
        for statement in &program.body {
            match statement {
                Statement::FunctionDeclaration(function) => {
                    if let Some(id) = &function.id
                        && component_identifier_name(id.name.as_str())
                        && function_body_contains_jsx(&function.body, self.source)
                    {
                        names.insert(id.name.as_str().into());
                    }
                }
                Statement::VariableDeclaration(declaration) => {
                    for declarator in &declaration.declarations {
                        let Some(name) = binding_identifier_name(&declarator.id) else {
                            continue;
                        };
                        if let Some(init) = &declarator.init {
                            if component_identifier_name(name.as_str())
                                && expression_contains_jsx(init, self.source)
                            {
                                names.insert(name);
                                continue;
                            }
                            if let Expression::ObjectExpression(object) = init {
                                self.discover_object_component_names(
                                    name.as_str(),
                                    object,
                                    &mut names,
                                );
                            }
                        }
                    }
                }
                Statement::ExpressionStatement(statement) => {
                    self.discover_window_object_assign_names(&statement.expression, &mut names);
                }
                _ => {}
            }
        }
        names
    }

    fn discover_function_components(
        &self,
        program: &'a Program<'a>,
    ) -> BTreeMap<CompactString, &'a Function<'a>> {
        let mut functions = BTreeMap::new();
        for statement in &program.body {
            let Statement::FunctionDeclaration(function) = statement else {
                continue;
            };
            let Some(id) = &function.id else {
                continue;
            };
            if component_identifier_name(id.name.as_str())
                && function_body_contains_jsx(&function.body, self.source)
            {
                functions.insert(id.name.as_str().into(), function.as_ref());
            }
        }
        functions
    }

    fn discover_published_component_names(
        &self,
        program: &'a Program<'a>,
    ) -> BTreeSet<CompactString> {
        let mut names = BTreeSet::new();
        for statement in &program.body {
            match statement {
                Statement::ExpressionStatement(statement) => {
                    self.discover_window_object_assign_names(&statement.expression, &mut names);
                    if let Some(name) = window_assignment_component_name(&statement.expression) {
                        names.insert(name);
                    }
                }
                Statement::ExportNamedDeclaration(export) => {
                    if let Some(declaration) = &export.declaration {
                        self.discover_declaration_component_names(declaration, &mut names);
                    }
                }
                Statement::ExportDefaultDeclaration(export) => {
                    if let ExportDefaultDeclarationKind::FunctionDeclaration(function) =
                        &export.declaration
                        && let Some(id) = &function.id
                        && component_identifier_name(id.name.as_str())
                    {
                        names.insert(id.name.as_str().into());
                    }
                }
                _ => {}
            }
        }
        names
    }

    fn discover_declaration_component_names(
        &self,
        declaration: &Declaration<'a>,
        names: &mut BTreeSet<CompactString>,
    ) {
        match declaration {
            Declaration::FunctionDeclaration(function) => {
                if let Some(id) = &function.id
                    && component_identifier_name(id.name.as_str())
                {
                    names.insert(id.name.as_str().into());
                }
            }
            Declaration::VariableDeclaration(declaration) => {
                for declarator in &declaration.declarations {
                    if let Some(name) = binding_identifier_name(&declarator.id)
                        && component_identifier_name(name.as_str())
                    {
                        names.insert(name);
                    }
                }
            }
            _ => {}
        }
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

        if let Some(name) = self.find_object_assign_component_name(program)
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

    fn find_named_component(
        &self,
        program: &'a Program<'a>,
        name: &str,
    ) -> Option<JsxComponent<'a>> {
        self.find_function_declaration(program, name)
            .map(|function| JsxComponent {
                name: name.into(),
                function,
                module_end: function.span.start as usize,
            })
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

    fn find_object_assign_component_name(&self, program: &Program<'a>) -> Option<CompactString> {
        let names = program.body.iter().find_map(|statement| {
            let Statement::ExpressionStatement(statement) = statement else {
                return None;
            };
            let Expression::CallExpression(call) = &statement.expression else {
                return None;
            };
            if !call_is_object_assign_window(call) {
                return None;
            }
            let Some(Argument::ObjectExpression(object)) = call.arguments.get(1) else {
                return None;
            };
            Some(
                object
                    .properties
                    .iter()
                    .filter_map(|property| {
                        let ObjectPropertyKind::ObjectProperty(property) = property else {
                            return None;
                        };
                        self.property_key(&property.key)
                    })
                    .filter(|name| {
                        self.find_function_declaration(program, name.as_str())
                            .is_some()
                    })
                    .collect::<Vec<_>>(),
            )
        })?;
        (names.len() > 1).then(|| names[names.len() - 1].clone())
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

    fn discover_object_component_names(
        &self,
        object_name: &str,
        object: &ObjectExpression<'a>,
        names: &mut BTreeSet<CompactString>,
    ) {
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                continue;
            };
            let Some(key) = self.property_key(&property.key) else {
                continue;
            };
            if expression_contains_jsx(&property.value, self.source) {
                names.insert(format!("{object_name}.{key}").into());
            }
        }
    }

    fn discover_window_object_assign_names(
        &self,
        expression: &Expression<'a>,
        names: &mut BTreeSet<CompactString>,
    ) {
        let Expression::CallExpression(call) = expression else {
            return;
        };
        if !call_is_object_assign_window(call) {
            return;
        }
        let Some(Argument::ObjectExpression(object)) = call.arguments.get(1) else {
            return;
        };
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                continue;
            };
            let Some(key) = self.property_key(&property.key) else {
                continue;
            };
            if component_identifier_name(key.as_str()) {
                names.insert(key);
            }
        }
    }

    fn source_logic_prelude(
        &mut self,
        program: &'a Program<'a>,
        body: &'a FunctionBody<'a>,
        component: &JsxComponent<'a>,
    ) -> JsxSourceLogicPrelude {
        let module = self.module_source_logic_prelude(program, component.module_end);
        let items = self.component_logic_items(body);
        JsxSourceLogicPrelude {
            body: module,
            component: (!items.is_empty()).then_some(RenderSourceComponentLogic { items }),
        }
    }

    fn module_source_logic_prelude(&self, program: &'a Program<'a>, module_end: usize) -> String {
        program
            .body
            .iter()
            .filter(|statement| (statement.span().start as usize) < module_end)
            .filter(|statement| !self.should_skip_module_statement(statement))
            .filter_map(|statement| {
                let source = self.source_for_span(statement.span()).trim();
                (!source.is_empty()).then_some(source)
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn should_skip_module_statement(&self, statement: &Statement<'a>) -> bool {
        match statement {
            Statement::FunctionDeclaration(function) => function
                .id
                .as_ref()
                .is_some_and(|id| self.component_names.contains(id.name.as_str()))
                || function_body_contains_jsx(&function.body, self.source),
            Statement::VariableDeclaration(declaration) => declaration.declarations.iter().any(
                |declarator| {
                    let componentish_binding = binding_identifier_name(&declarator.id)
                        .is_some_and(|name| component_identifier_name(name.as_str()));
                    declarator
                        .init
                        .as_ref()
                        .is_some_and(|init| expression_contains_jsx(init, self.source))
                        || (componentish_binding
                            && declarator.init.as_ref().is_some_and(|init| {
                                matches!(init, Expression::Identifier(identifier) if component_identifier_name(identifier.name.as_str()))
                            }))
                },
            ),
            Statement::ExpressionStatement(statement) => {
                expression_contains_jsx(&statement.expression, self.source)
                    || expression_is_window_publish(&statement.expression)
            }
            Statement::ExportNamedDeclaration(_) | Statement::ExportDefaultDeclaration(_) => true,
            _ => false,
        }
    }

    fn function_props_pattern(&self, function: &Function<'a>) -> Option<CompactString> {
        let param = function.params.items.first()?;
        let source = self.source_for_span(param.pattern.span()).trim();
        (!source.is_empty()).then(|| source.into())
    }

    fn collect_component_bindings(&mut self, body: &FunctionBody<'a>, props: Option<&str>) {
        self.setters.clear();
        self.reactive_names.clear();
        self.refs.clear();
        self.snippets.clear();
        self.source_aliases.clear();
        self.add_component_bindings(body, props);
    }

    fn add_component_bindings(&mut self, body: &FunctionBody<'a>, props: Option<&str>) {
        if let Some(props) = props {
            for local in binding_pattern_locals(props) {
                self.reactive_names.insert(local.into());
            }
        }

        for statement in component_setup_statements(body) {
            let Statement::VariableDeclaration(declaration) = statement else {
                continue;
            };
            for declarator in &declaration.declarations {
                if let Some((name, setter, _initial)) = self.use_state_declarator(declarator) {
                    self.setters.insert(setter, name.clone());
                    self.reactive_names.insert(name);
                } else if let Some((name, _body, _by)) = self.use_memo_declarator(declarator) {
                    self.reactive_names.insert(name);
                } else if let Some((name, _initial)) = self.use_ref_declarator(declarator) {
                    self.refs.insert(name, false);
                }
            }
        }
    }

    fn component_logic_items(&mut self, body: &'a FunctionBody<'a>) -> Vec<RenderSourceLogicItem> {
        let statements = component_setup_statements(body);
        let mut items = Vec::new();
        let mut index = 0usize;
        while index < statements.len() {
            if let Some((item, consumed)) = self.derived_block_item(&statements, index) {
                self.register_logic_item_name(&item);
                items.push(item);
                index += consumed;
                continue;
            }

            match statements[index] {
                Statement::VariableDeclaration(declaration) => {
                    for item in self.variable_logic_items(declaration) {
                        self.register_logic_item_name(&item);
                        items.push(item);
                    }
                }
                Statement::ExpressionStatement(statement) => {
                    if let Some(item) = self.expression_logic_item(&statement.expression) {
                        if let RenderSourceLogicItem::Derived(derived) = &item {
                            remove_state_item(&mut items, &derived.name);
                        }
                        self.register_logic_item_name(&item);
                        items.push(item);
                    } else if let Some((item, source, alias)) =
                        self.layout_side_derived_item(&statement.expression)
                    {
                        self.register_logic_item_name(&item);
                        self.source_aliases.insert(source, alias);
                        items.push(item);
                    } else if self.expression_depends_on_reactive(&statement.expression) {
                        let dependencies = source_identifier_roots(
                            self.source_for_span(statement.expression.span()),
                        )
                        .into_iter()
                        .filter(|root| self.reactive_names.contains(root.as_str()))
                        .map(CompactString::from)
                        .collect();
                        items.push(RenderSourceLogicItem::Effect(RenderSourceEffect {
                            dependencies,
                            body: self
                                .rewrite_react_source(self.source_for_span(statement.span).trim())
                                .into(),
                            span: Some(self.span(statement.span)),
                        }));
                    } else {
                        let body = self.rewrite_react_source(self.source_for_span(statement.span));
                        if !body.trim().is_empty() {
                            items.push(RenderSourceLogicItem::Local(RenderSourceLocal {
                                body: ArcStr::from(body),
                                span: Some(self.span(statement.span)),
                            }));
                        }
                    }
                }
                Statement::FunctionDeclaration(function) => {
                    let body = self.rewrite_react_source(self.source_for_span(function.span));
                    if !body.trim().is_empty() {
                        items.push(RenderSourceLogicItem::Local(RenderSourceLocal {
                            body: ArcStr::from(body),
                            span: Some(self.span(function.span)),
                        }));
                    }
                }
                statement => {
                    let body = self.rewrite_react_source(self.source_for_span(statement.span()));
                    if !body.trim().is_empty() {
                        items.push(RenderSourceLogicItem::Local(RenderSourceLocal {
                            body: ArcStr::from(body),
                            span: Some(self.span(statement.span())),
                        }));
                    }
                }
            }
            index += 1;
        }

        for (name, dom) in self.refs.clone() {
            if dom {
                for item in &mut items {
                    if let RenderSourceLogicItem::Ref(reference) = item
                        && reference.name == name
                    {
                        reference.dom = true;
                    }
                }
            }
        }

        items
    }

    fn register_logic_item_name(&mut self, item: &RenderSourceLogicItem) {
        match item {
            RenderSourceLogicItem::State(state) => {
                self.reactive_names.insert(state.name.clone());
            }
            RenderSourceLogicItem::Derived(derived) => {
                self.reactive_names.insert(derived.name.clone());
            }
            RenderSourceLogicItem::Ref(reference) => {
                self.refs
                    .entry(reference.name.clone())
                    .or_insert(reference.dom);
            }
            RenderSourceLogicItem::Callback(callback) => {
                self.reactive_names.insert(callback.name.clone());
            }
            RenderSourceLogicItem::Snippet(snippet) => {
                self.snippets.insert(snippet.name.clone());
            }
            RenderSourceLogicItem::Local(_)
            | RenderSourceLogicItem::Mount(_)
            | RenderSourceLogicItem::Effect(_) => {}
        }
    }

    fn variable_logic_items(
        &mut self,
        declaration: &'a oxc_ast::ast::VariableDeclaration<'a>,
    ) -> Vec<RenderSourceLogicItem> {
        let mut items = Vec::new();
        for declarator in &declaration.declarations {
            if let Some((name, setter, initial)) = self.use_state_declarator(declarator) {
                items.push(RenderSourceLogicItem::State(RenderSourceState {
                    name,
                    setter,
                    initial,
                    span: Some(self.span(declarator.span)),
                }));
                continue;
            }
            if let Some((name, body, by)) = self.use_memo_declarator(declarator) {
                items.push(RenderSourceLogicItem::Derived(RenderSourceDerived {
                    name,
                    body,
                    by,
                    span: Some(self.span(declarator.span)),
                }));
                continue;
            }
            if let Some((name, initial)) = self.use_ref_declarator(declarator) {
                items.push(RenderSourceLogicItem::Ref(RenderSourceRef {
                    name,
                    initial,
                    dom: false,
                    span: Some(self.span(declarator.span)),
                }));
                continue;
            }
            if let Some((name, body)) = self.use_callback_declarator(declarator) {
                items.push(RenderSourceLogicItem::Callback(RenderSourceCallback {
                    name,
                    body,
                    span: Some(self.span(declarator.span)),
                }));
                continue;
            }
            if let Some(snippet) = self.snippet_declarator(declarator) {
                items.push(RenderSourceLogicItem::Snippet(snippet));
                continue;
            }
            if let Some((name, body)) = self.arrow_or_function_declarator(declarator) {
                items.push(RenderSourceLogicItem::Callback(RenderSourceCallback {
                    name,
                    body,
                    span: Some(self.span(declarator.span)),
                }));
                continue;
            }
            items.push(self.local_or_derived_declarator(declaration.kind, declarator));
        }
        items
    }

    fn local_or_derived_declarator(
        &mut self,
        kind: VariableDeclarationKind,
        declarator: &VariableDeclarator<'a>,
    ) -> RenderSourceLogicItem {
        let name = self.binding_source(&declarator.id);
        let Some(init) = &declarator.init else {
            let keyword = variable_kind_keyword(kind);
            return RenderSourceLogicItem::Local(RenderSourceLocal {
                body: ArcStr::from(format!("{keyword} {name};")),
                span: Some(self.span(declarator.span)),
            });
        };
        let value = self.rewrite_react_source(self.source_for_span(init.span()).trim());
        if kind == VariableDeclarationKind::Const && self.expression_depends_on_reactive(init) {
            RenderSourceLogicItem::Derived(RenderSourceDerived {
                name,
                body: value.into(),
                by: false,
                span: Some(self.span(declarator.span)),
            })
        } else {
            let keyword = variable_kind_keyword(kind);
            RenderSourceLogicItem::Local(RenderSourceLocal {
                body: ArcStr::from(format!("{keyword} {name} = {value};")),
                span: Some(self.span(declarator.span)),
            })
        }
    }

    fn derived_block_item(
        &mut self,
        statements: &[&'a Statement<'a>],
        index: usize,
    ) -> Option<(RenderSourceLogicItem, usize)> {
        let Statement::VariableDeclaration(declaration) = statements.get(index)? else {
            return None;
        };
        if declaration.kind != VariableDeclarationKind::Let || declaration.declarations.len() != 1 {
            return None;
        }
        let declarator = declaration.declarations.first()?;
        let name = binding_identifier_name(&declarator.id)?;
        let init = declarator.init.as_ref()?;
        if !self.expression_depends_on_reactive(init) {
            return None;
        }
        let mut consumed = 1usize;
        let mut body = format!(
            "let {name} = {};",
            self.rewrite_react_source(self.source_for_span(init.span()).trim())
        );
        while let Some(statement) = statements.get(index + consumed) {
            if matches!(statement, Statement::ReturnStatement(_)) {
                break;
            }
            let source = self.source_for_span(statement.span());
            if !source_contains_assignment_to(source, name.as_str()) {
                break;
            }
            body.push('\n');
            body.push_str(&self.rewrite_react_source(source.trim()));
            consumed += 1;
        }
        if consumed == 1 {
            return None;
        }
        body.push('\n');
        body.push_str("return ");
        body.push_str(name.as_str());
        body.push(';');
        Some((
            RenderSourceLogicItem::Derived(RenderSourceDerived {
                name,
                body: body.into(),
                by: true,
                span: Some(self.span(declarator.span)),
            }),
            consumed,
        ))
    }

    fn expression_logic_item(
        &mut self,
        expression: &Expression<'a>,
    ) -> Option<RenderSourceLogicItem> {
        let Expression::CallExpression(call) = expression else {
            return None;
        };
        if !call_is_react_hook(call, "useEffect") && !call_is_react_hook(call, "useLayoutEffect") {
            return None;
        }
        self.effect_logic_item(call)
    }

    fn layout_side_derived_item(
        &mut self,
        expression: &Expression<'a>,
    ) -> Option<(RenderSourceLogicItem, CompactString, CompactString)> {
        let Expression::CallExpression(call) = expression else {
            return None;
        };
        if simple_callee_name(&call.callee)? != "layoutSide" {
            return None;
        }
        let groups = call.arguments.first()?;
        let groups_source = self.source_for_span(groups.span()).trim();
        let (root, property) = groups_source.split_once('.')?;
        if !self.reactive_names.contains(root) || !is_simple_identifier(property) {
            return None;
        }

        let alias = self.unique_source_alias(&format!("{property}Groups"));
        let callee = self.source_for_span(call.callee.span()).trim();
        let arguments = call
            .arguments
            .iter()
            .skip(1)
            .map(|argument| self.rewrite_react_source(self.source_for_span(argument.span()).trim()))
            .collect::<Vec<_>>();
        let call_arguments = if arguments.is_empty() {
            "groups".to_owned()
        } else {
            format!("groups, {}", arguments.join(", "))
        };
        let body = format!(
            "const groups = {groups_source}.map((group) => ({{
  ...group,
  items: group.items.map((item) => ({{ ...item }})),
}}));
{callee}({call_arguments});
return groups;"
        );
        Some((
            RenderSourceLogicItem::Derived(RenderSourceDerived {
                name: alias.clone(),
                body: body.into(),
                by: true,
                span: Some(self.span(call.span)),
            }),
            groups_source.into(),
            alias,
        ))
    }

    fn unique_source_alias(&self, base: &str) -> CompactString {
        if !self.reactive_names.contains(base)
            && !self
                .source_aliases
                .values()
                .any(|alias| alias.as_str() == base)
        {
            return base.into();
        }
        let mut index = 1usize;
        loop {
            let candidate = format!("{base}{index}");
            if !self.reactive_names.contains(candidate.as_str())
                && !self
                    .source_aliases
                    .values()
                    .any(|alias| alias.as_str() == candidate)
            {
                return candidate.into();
            }
            index += 1;
        }
    }

    fn use_state_declarator(
        &self,
        declarator: &VariableDeclarator<'a>,
    ) -> Option<(CompactString, CompactString, CompactString)> {
        let OxcBindingPattern::ArrayPattern(pattern) = &declarator.id else {
            return None;
        };
        let init = declarator.init.as_ref()?;
        let Expression::CallExpression(call) = init else {
            return None;
        };
        if !call_is_react_hook(call, "useState") {
            return None;
        }
        let state = pattern.elements.first()?.as_ref()?;
        let setter = pattern.elements.get(1)?.as_ref()?;
        let state = binding_identifier_name(state)?;
        let setter = binding_identifier_name(setter)?;
        let initial = call
            .arguments
            .first()
            .map(|argument| self.source_for_span(argument.span()).trim().to_owned())
            .unwrap_or_else(|| "undefined".to_owned());
        Some((state, setter, initial.into()))
    }

    fn use_memo_declarator(
        &self,
        declarator: &VariableDeclarator<'a>,
    ) -> Option<(CompactString, CompactString, bool)> {
        let name = binding_identifier_name(&declarator.id)?;
        let init = declarator.init.as_ref()?;
        let Expression::CallExpression(call) = init else {
            return None;
        };
        if !call_is_react_hook(call, "useMemo") {
            return None;
        }
        let callback = call.arguments.first().and_then(argument_arrow_function)?;
        Some((
            name,
            self.arrow_derived_body(callback),
            !callback.expression,
        ))
    }

    fn use_ref_declarator(
        &self,
        declarator: &VariableDeclarator<'a>,
    ) -> Option<(CompactString, CompactString)> {
        let name = binding_identifier_name(&declarator.id)?;
        let init = declarator.init.as_ref()?;
        let Expression::CallExpression(call) = init else {
            return None;
        };
        if !call_is_react_hook(call, "useRef") {
            return None;
        }
        let initial = call
            .arguments
            .first()
            .map(|argument| self.source_for_span(argument.span()).trim().to_owned())
            .unwrap_or_else(|| "undefined".to_owned());
        Some((name, initial.into()))
    }

    fn use_callback_declarator(
        &mut self,
        declarator: &VariableDeclarator<'a>,
    ) -> Option<(CompactString, CompactString)> {
        let name = binding_identifier_name(&declarator.id)?;
        let init = declarator.init.as_ref()?;
        let Expression::CallExpression(call) = init else {
            return None;
        };
        if !call_is_react_hook(call, "useCallback") {
            return None;
        }
        let callback = call.arguments.first()?;
        Some((
            name,
            self.rewrite_react_source(self.source_for_span(callback.span()).trim())
                .into(),
        ))
    }

    fn arrow_or_function_declarator(
        &mut self,
        declarator: &VariableDeclarator<'a>,
    ) -> Option<(CompactString, CompactString)> {
        let name = binding_identifier_name(&declarator.id)?;
        let init = declarator.init.as_ref()?;
        match init {
            Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => Some((
                name,
                self.rewrite_react_source(self.source_for_span(init.span()).trim())
                    .into(),
            )),
            _ => None,
        }
    }

    fn snippet_declarator(
        &mut self,
        declarator: &VariableDeclarator<'a>,
    ) -> Option<RenderSourceSnippet> {
        let name = binding_identifier_name(&declarator.id)?;
        let init = declarator.init.as_ref()?;
        let Expression::ArrowFunctionExpression(callback) = init else {
            return None;
        };
        let return_expression = returned_expression(&callback.body)?;
        if !expression_contains_jsx(return_expression, self.source) {
            return None;
        }
        let params = callback
            .params
            .items
            .iter()
            .map(|param| self.source_for_span(param.pattern.span()).trim().into())
            .collect();
        let nodes = self.lower_return_expression(return_expression);
        Some(RenderSourceSnippet {
            name,
            params,
            nodes,
            span: Some(self.span(declarator.span)),
        })
    }

    fn effect_logic_item(&mut self, call: &CallExpression<'a>) -> Option<RenderSourceLogicItem> {
        let callback = call.arguments.first().and_then(argument_arrow_function)?;
        let dependencies = call
            .arguments
            .get(1)
            .map(|argument| self.effect_dependencies(argument))
            .unwrap_or_default();
        if dependencies.is_empty() {
            return Some(RenderSourceLogicItem::Mount(RenderSourceMount {
                body: self
                    .rewrite_react_source(self.arrow_block_body(callback).trim())
                    .into(),
                span: Some(self.span(call.span)),
            }));
        }
        if let Some(derived) = self.derived_from_effect(call, callback, &dependencies) {
            return Some(RenderSourceLogicItem::Derived(derived));
        }
        Some(RenderSourceLogicItem::Effect(RenderSourceEffect {
            dependencies,
            body: self
                .rewrite_react_source(self.arrow_block_body(callback).trim())
                .into(),
            span: Some(self.span(call.span)),
        }))
    }

    fn derived_from_effect(
        &self,
        call: &CallExpression<'a>,
        callback: &ArrowFunctionExpression<'a>,
        dependencies: &[CompactString],
    ) -> Option<RenderSourceDerived> {
        let statements = callback.body.statements.as_slice();
        if statements.len() != 1 {
            return None;
        }
        let Statement::ExpressionStatement(statement) = &statements[0] else {
            return None;
        };
        let Expression::CallExpression(setter_call) = &statement.expression else {
            return None;
        };
        let setter = simple_callee_name(&setter_call.callee)?;
        let state = self.setters.get(setter)?;
        let argument = setter_call.arguments.first()?;
        let source = self.source_for_span(argument.span()).trim();
        let roots = source_identifier_roots(source);
        if !dependencies.iter().any(|dep| roots.contains(dep.as_str())) {
            return None;
        }
        Some(RenderSourceDerived {
            name: state.clone(),
            body: source.into(),
            by: false,
            span: Some(self.span(call.span)),
        })
    }

    fn effect_dependencies(&self, argument: &Argument<'a>) -> Vec<CompactString> {
        let Argument::ArrayExpression(array) = argument else {
            return Vec::new();
        };
        array
            .elements
            .iter()
            .filter_map(|element| match element {
                ArrayExpressionElement::Identifier(identifier) => {
                    Some(CompactString::from(identifier.name.as_str()))
                }
                ArrayExpressionElement::StaticMemberExpression(member) => {
                    Some(self.source_for_span(member.span()).trim().into())
                }
                _ => None,
            })
            .collect()
    }

    fn arrow_derived_body(&self, callback: &ArrowFunctionExpression<'a>) -> CompactString {
        if callback.expression {
            return returned_expression(&callback.body)
                .map(|expression| self.source_for_span(expression.span()).trim().into())
                .unwrap_or_default();
        }
        self.arrow_block_body(callback).trim().into()
    }

    fn arrow_block_body(&self, callback: &ArrowFunctionExpression<'a>) -> String {
        if callback.expression {
            return returned_expression(&callback.body)
                .map(|expression| {
                    format!("return {};", self.source_for_span(expression.span()).trim())
                })
                .unwrap_or_default();
        }
        let source = self.source_for_span(callback.body.span);
        source
            .trim()
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
            .unwrap_or(source)
            .trim()
            .to_owned()
    }

    fn expression_depends_on_reactive(&self, expression: &Expression<'a>) -> bool {
        let source = self.source_for_span(expression.span());
        source_identifier_roots(source)
            .into_iter()
            .any(|root| self.reactive_names.contains(root.as_str()))
    }

    fn binding_source(&self, pattern: &OxcBindingPattern<'a>) -> CompactString {
        binding_identifier_name(pattern)
            .unwrap_or_else(|| CompactString::from(self.source_for_span(pattern.span()).trim()))
    }

    fn rewrite_react_source(&self, source: &str) -> String {
        rewrite_source_aliases(
            &rewrite_setter_calls(source, &self.setters),
            &self.source_aliases,
        )
    }

    fn source_alias_for_span(&self, span: OxcSpan) -> Option<CompactString> {
        let source = self.source_for_span(span).trim();
        self.source_aliases.get(source).cloned()
    }

    fn setter_reference_expression(&self, name: &str) -> Option<Expr> {
        let state = self.setters.get(name)?;
        Some(Expr::Opaque(
            format!("(value) => {{ {state} = value; }}").into(),
        ))
    }

    fn lower_return_expression(&mut self, expression: &Expression<'a>) -> Vec<RenderNode> {
        match expression {
            Expression::ParenthesizedExpression(expression) => {
                self.lower_return_expression(&expression.expression)
            }
            Expression::JSXElement(element) => vec![self.lower_jsx_element(element).0],
            Expression::JSXFragment(fragment) => self.lower_jsx_fragment(fragment),
            Expression::ConditionalExpression(conditional) => self
                .lower_conditional_child(
                    &conditional.test,
                    &conditional.consequent,
                    &conditional.alternate,
                    conditional.span,
                )
                .unwrap_or_default(),
            Expression::LogicalExpression(logical) if logical.operator.as_str() == "&&" => self
                .lower_expression_child_expression(expression)
                .unwrap_or_default(),
            Expression::NullLiteral(_) => Vec::new(),
            _ => {
                self.diagnostics.push(Diagnostic::error(
                    "Phase 3 JSX components must return JSX, a JSX conditional, or null",
                    Some(self.span(expression.span())),
                ));
                Vec::new()
            }
        }
    }

    fn lower_component_return_nodes(
        &mut self,
        body: &'a FunctionBody<'a>,
        return_expression: &'a Expression<'a>,
    ) -> Vec<RenderNode> {
        let setup_statements = body
            .statements
            .iter()
            .take_while(|statement| !matches!(statement, Statement::ReturnStatement(_)))
            .collect::<Vec<_>>();
        let Some(first_guard) = setup_statements
            .iter()
            .position(|statement| self.component_return_guard(statement).is_some())
        else {
            return self.lower_return_expression(return_expression);
        };
        self.lower_component_return_sequence(&setup_statements[first_guard..], return_expression)
    }

    fn lower_component_return_sequence(
        &mut self,
        statements: &[&'a Statement<'a>],
        return_expression: &'a Expression<'a>,
    ) -> Vec<RenderNode> {
        let mut setup_locals = Vec::new();
        let mut index = 0usize;
        while let Some(statement) = statements.get(index) {
            let Some((test, expression, span, branch_locals)) =
                self.component_return_guard(statement)
            else {
                if let Some((mut new_locals, consumed)) =
                    self.control_flow_setup_locals(statements, index)
                {
                    setup_locals.append(&mut new_locals);
                    index += consumed;
                    continue;
                }
                self.diagnostics.push(Diagnostic::warning(
                    "JSX setup after an early return must be variable declarations to become branch-local Svelte markup",
                    Some(self.span(statement.span())),
                ));
                index += 1;
                continue;
            };
            let condition = self.lower_expression(test);
            let children = self.lower_return_expression(expression);
            let else_children =
                self.lower_component_return_sequence(&statements[index + 1..], return_expression);
            let nodes = if children.is_empty() {
                vec![self.control_flow_wrapper(
                    RenderControlFlowKind::If,
                    Some(self.negated_condition(test)),
                    span,
                    else_children,
                    Vec::new(),
                )]
            } else {
                let mut nodes = vec![self.control_flow_wrapper(
                    RenderControlFlowKind::If,
                    Some(condition),
                    span,
                    children,
                    branch_locals,
                )];
                if !else_children.is_empty() {
                    nodes.push(self.control_flow_wrapper(
                        RenderControlFlowKind::Else,
                        None,
                        return_expression.span(),
                        else_children,
                        Vec::new(),
                    ));
                }
                nodes
            };
            return self.with_control_flow_locals(setup_locals, span, nodes);
        }

        let nodes = self.lower_return_expression(return_expression);
        self.with_control_flow_locals(setup_locals, return_expression.span(), nodes)
    }

    fn with_control_flow_locals(
        &self,
        locals: Vec<RenderLoopLocal>,
        span: OxcSpan,
        nodes: Vec<RenderNode>,
    ) -> Vec<RenderNode> {
        if locals.is_empty() {
            nodes
        } else {
            vec![self.control_flow_wrapper(
                RenderControlFlowKind::If,
                Some(Expr::Literal(ExprLiteral::Bool(true))),
                span,
                nodes,
                locals,
            )]
        }
    }

    fn negated_condition(&self, expression: &Expression<'a>) -> Expr {
        Expr::Opaque(
            format!(
                "!({})",
                self.rewrite_react_source(self.source_for_span(expression.span()).trim())
            )
            .into(),
        )
    }

    fn control_flow_setup_locals(
        &mut self,
        statements: &[&'a Statement<'a>],
        index: usize,
    ) -> Option<(Vec<RenderLoopLocal>, usize)> {
        let statement = statements.get(index)?;
        let Statement::VariableDeclaration(declaration) = statement else {
            return None;
        };
        if let Some((local, consumed)) = self.control_flow_derived_block_local(statements, index) {
            return Some((vec![local], consumed));
        }
        let locals = self.control_flow_locals(declaration);
        (!locals.is_empty()).then_some((locals, 1))
    }

    fn control_flow_derived_block_local(
        &mut self,
        statements: &[&'a Statement<'a>],
        index: usize,
    ) -> Option<(RenderLoopLocal, usize)> {
        let Statement::VariableDeclaration(declaration) = statements.get(index)? else {
            return None;
        };
        if declaration.kind != VariableDeclarationKind::Let || declaration.declarations.len() != 1 {
            return None;
        }
        let declarator = declaration.declarations.first()?;
        let name = binding_identifier_name(&declarator.id)?;
        let init = declarator.init.as_ref()?;
        let mut consumed = 1usize;
        let mut body = format!(
            "(() => {{\nlet {name} = {};",
            self.rewrite_react_source(self.source_for_span(init.span()).trim())
        );
        while let Some(statement) = statements.get(index + consumed) {
            let source = self.source_for_span(statement.span());
            if !source_contains_assignment_to(source, name.as_str()) {
                break;
            }
            body.push('\n');
            body.push_str(&self.rewrite_react_source(source.trim()));
            consumed += 1;
        }
        if consumed == 1 {
            return None;
        }
        body.push_str("\nreturn ");
        body.push_str(name.as_str());
        body.push_str(";\n})()");
        Some((
            RenderLoopLocal {
                name,
                value: Expr::Opaque(body.into()),
                span: Some(self.span(declarator.span)),
            },
            consumed,
        ))
    }

    fn component_return_guard(
        &mut self,
        statement: &'a Statement<'a>,
    ) -> Option<(
        &'a Expression<'a>,
        &'a Expression<'a>,
        OxcSpan,
        Vec<RenderLoopLocal>,
    )> {
        let Statement::IfStatement(statement) = statement else {
            return None;
        };
        if statement.alternate.is_some() {
            return None;
        }
        let (expression, locals) = self.return_guard_expression(&statement.consequent)?;
        Some((&statement.test, expression, statement.span, locals))
    }

    fn return_guard_expression(
        &mut self,
        statement: &'a Statement<'a>,
    ) -> Option<(&'a Expression<'a>, Vec<RenderLoopLocal>)> {
        match statement {
            Statement::ReturnStatement(statement) => {
                let expression = statement.argument.as_ref()?;
                render_return_expression(expression).then_some((expression, Vec::new()))
            }
            Statement::BlockStatement(block) => {
                let mut locals = Vec::new();
                for statement in &block.body {
                    match statement {
                        Statement::VariableDeclaration(declaration) => {
                            locals.extend(self.control_flow_locals(declaration));
                        }
                        Statement::ReturnStatement(statement) => {
                            let expression = statement.argument.as_ref()?;
                            return render_return_expression(expression)
                                .then_some((expression, locals));
                        }
                        _ => return None,
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn control_flow_locals(
        &mut self,
        declaration: &'a oxc_ast::ast::VariableDeclaration<'a>,
    ) -> Vec<RenderLoopLocal> {
        declaration
            .declarations
            .iter()
            .filter_map(|declarator| {
                let init = declarator.init.as_ref()?;
                let name = binding_identifier_name(&declarator.id).unwrap_or_else(|| {
                    CompactString::from(self.source_for_span(declarator.id.span()).trim())
                });
                Some(RenderLoopLocal {
                    name,
                    value: Expr::Opaque(
                        self.rewrite_react_source(self.source_for_span(init.span()))
                            .into(),
                    ),
                    span: Some(self.span(declarator.span)),
                })
            })
            .collect()
    }

    fn lower_jsx_fragment(&mut self, fragment: &JSXFragment<'a>) -> Vec<RenderNode> {
        self.lower_children(&fragment.children)
    }

    fn lower_jsx_element(&mut self, element: &JSXElement<'a>) -> (RenderNode, Option<Expr>) {
        let tag = self.jsx_element_name(&element.opening_element.name);
        if tag == "React.Fragment" {
            let (children, key) = self.lower_react_fragment_element(element);
            let mut wrapper = self.empty_element("jsx-fragment", element.span);
            wrapper.children = children;
            return (RenderNode::Element(Box::new(wrapper)), key);
        }
        if self.snippets.contains(tag.as_str()) {
            return (self.lower_snippet_element(tag.as_str(), element), None);
        }
        if is_jsx_component_tag(&tag) {
            return self.lower_component_element(&tag, element);
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
                    self.push_spread_attribute(
                        &mut render.attributes,
                        &spread.argument,
                        spread.span,
                    );
                }
            }
        }
        render.children = self.lower_children(&element.children);
        (RenderNode::Element(Box::new(render)), key)
    }

    fn lower_react_fragment_element(
        &mut self,
        element: &JSXElement<'a>,
    ) -> (Vec<RenderNode>, Option<Expr>) {
        let mut key = None;
        for attribute in &element.opening_element.attributes {
            match attribute {
                JSXAttributeItem::Attribute(attribute) => {
                    if self.jsx_attribute_name(&attribute.name) == "key" {
                        key = attribute
                            .value
                            .as_ref()
                            .and_then(|value| self.attribute_expr(value));
                    }
                }
                JSXAttributeItem::SpreadAttribute(spread) => {
                    self.diagnostics.push(Diagnostic::warning(
                        "JSX React.Fragment spread attributes are ignored; only key is meaningful",
                        Some(self.span(spread.span)),
                    ));
                }
            }
        }
        (self.lower_children(&element.children), key)
    }

    fn lower_published_component_inline(
        &mut self,
        tag: &str,
        element: &JSXElement<'a>,
    ) -> Option<(RenderNode, Option<Expr>)> {
        if !self.inline_published_components {
            return None;
        }
        if tag.contains('.') || !self.published_component_names.contains(tag) {
            return None;
        }
        let name = CompactString::from(tag);
        if !self.inlining_components.insert(name.clone()) {
            return None;
        }

        let inlined = (|| {
            let function = self.find_inlinable_function(name.as_str())?;
            let body = function.body.as_deref()?;
            let return_expression = returned_expression(body)?;
            let props_pattern = self.function_props_pattern(function);
            let props = self.component_call_props(element)?;
            let prop_items = self.inlined_component_prop_items(props_pattern.as_deref(), &props);
            for item in prop_items {
                self.register_logic_item_name(&item);
                self.inlined_component_items.push(item);
            }
            self.add_component_bindings(body, props_pattern.as_deref());
            let logic_items = self.component_logic_items(body);
            for item in logic_items {
                self.inlined_component_items.push(item);
            }

            let mut nodes = self.lower_return_expression(return_expression);
            let node = if nodes.len() == 1 {
                nodes.remove(0)
            } else {
                let mut wrapper = self.empty_element("jsx-fragment", element.span);
                wrapper.children = nodes;
                RenderNode::Element(Box::new(wrapper))
            };
            Some((node, None))
        })();

        self.inlining_components.remove(&name);
        inlined
    }

    fn find_inlinable_function(&self, name: &str) -> Option<&'a Function<'a>> {
        self.function_components.get(name).copied()
    }

    fn component_call_props(
        &mut self,
        element: &JSXElement<'a>,
    ) -> Option<BTreeMap<CompactString, CompactString>> {
        let mut props = BTreeMap::new();
        for attribute in &element.opening_element.attributes {
            match attribute {
                JSXAttributeItem::Attribute(attribute) => {
                    let name = self.jsx_attribute_name(&attribute.name);
                    if name == "key" {
                        continue;
                    }
                    let value = match &attribute.value {
                        None => "true".to_owned(),
                        Some(JSXAttributeValue::StringLiteral(value)) => {
                            format!("{:?}", value.value.as_str())
                        }
                        Some(JSXAttributeValue::ExpressionContainer(container)) => self
                            .rewrite_react_source(
                                self.source_for_span(container.expression.span()).trim(),
                            ),
                        Some(other) => {
                            self.unsupported_attribute_value(other);
                            return None;
                        }
                    };
                    props.insert(name, value.into());
                }
                JSXAttributeItem::SpreadAttribute(spread) => {
                    self.diagnostics.push(Diagnostic::warning(
                        "JSX component spreads are not inlined; emitted placeholder",
                        Some(self.span(spread.span)),
                    ));
                    return None;
                }
            }
        }
        Some(props)
    }

    fn inlined_component_prop_items(
        &self,
        props_pattern: Option<&str>,
        call_props: &BTreeMap<CompactString, CompactString>,
    ) -> Vec<RenderSourceLogicItem> {
        let Some(props_pattern) = props_pattern else {
            return Vec::new();
        };
        component_props_from_pattern(props_pattern)
            .into_iter()
            .filter_map(|prop| {
                let value = call_props
                    .get(prop.name.as_str())
                    .cloned()
                    .or(prop.default)?;
                if self.reactive_names.contains(prop.name.as_str()) && value == prop.name {
                    return None;
                }
                let reactive = source_identifier_roots(value.as_str())
                    .into_iter()
                    .any(|root| self.reactive_names.contains(root.as_str()));
                if reactive {
                    Some(RenderSourceLogicItem::Derived(RenderSourceDerived {
                        name: prop.name,
                        body: value,
                        by: false,
                        span: None,
                    }))
                } else {
                    Some(RenderSourceLogicItem::Local(RenderSourceLocal {
                        body: ArcStr::from(format!("const {} = {};", prop.name, value)),
                        span: None,
                    }))
                }
            })
            .collect()
    }

    fn lower_component_element(
        &mut self,
        tag: &str,
        element: &JSXElement<'a>,
    ) -> (RenderNode, Option<Expr>) {
        if let Some(inlined) = self.lower_published_component_inline(tag, element) {
            return inlined;
        }
        let component = component_reference_name(tag);
        let mut render = self.empty_element("jsx-component", element.span);
        let mut key = None;
        let mut props = Vec::new();
        for attribute in &element.opening_element.attributes {
            match attribute {
                JSXAttributeItem::Attribute(attribute) => {
                    if self.lower_component_attribute(attribute, &mut props, &mut key) {
                        continue;
                    }
                }
                JSXAttributeItem::SpreadAttribute(spread) => {
                    self.push_spread_attribute(
                        &mut render.attributes,
                        &spread.argument,
                        spread.span,
                    );
                }
            }
        }
        render.source_intent = Some(Box::new(RenderSourceIntent {
            key: key
                .as_ref()
                .map(ToString::to_string)
                .map(CompactString::from),
            state_id: None,
            component: Some(ComponentId::new(component.as_str())),
            component_source: None,
            slot: None,
            child_strategy: None,
            props,
        }));
        render.children = self.lower_children(&element.children);
        (RenderNode::Element(Box::new(render)), key)
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
        if raw_name == "ref" {
            if let Some(JSXAttributeValue::ExpressionContainer(container)) = &attribute.value {
                let name = self
                    .source_for_span(container.expression.span())
                    .trim()
                    .to_owned();
                if is_simple_identifier(&name) {
                    self.refs.insert(CompactString::from(name.as_str()), true);
                    element.attributes.push(RenderAttribute {
                        name: "bind:this".into(),
                        value: CompactString::from(name.as_str()),
                        template: None,
                        span: Some(self.span(attribute.span)),
                    });
                } else {
                    self.diagnostics.push(Diagnostic::warning(
                        "JSX ref attributes must be simple identifiers for Svelte bind:this",
                        Some(self.span(attribute.span)),
                    ));
                }
            }
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

    fn lower_component_attribute(
        &mut self,
        attribute: &JSXAttribute<'a>,
        props: &mut Vec<RenderSourceProp>,
        key: &mut Option<Expr>,
    ) -> bool {
        let name = self.jsx_attribute_name(&attribute.name);
        if name == "key" {
            *key = attribute
                .value
                .as_ref()
                .and_then(|value| self.attribute_expr(value));
            return true;
        }
        let span = Some(self.span(attribute.span));
        match &attribute.value {
            None => props.push(RenderSourceProp {
                name,
                value: CompactString::new(""),
                template: None,
                span,
            }),
            Some(JSXAttributeValue::StringLiteral(value)) => props.push(RenderSourceProp {
                name,
                value: CompactString::from(value.value.as_str()),
                template: None,
                span,
            }),
            Some(JSXAttributeValue::ExpressionContainer(container)) => {
                let expr = self.lower_jsx_expression(&container.expression);
                props.push(RenderSourceProp {
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

    fn push_spread_attribute(
        &mut self,
        attributes: &mut Vec<RenderAttribute>,
        argument: &Expression<'a>,
        span: OxcSpan,
    ) {
        let expr = self.lower_expression(argument);
        attributes.push(RenderAttribute {
            name: "{...}".into(),
            value: CompactString::new(""),
            template: Some(TemplateString::new(
                self.source_for_span(argument.span()),
                vec![TemplateSegment::Expression(expr)],
                Some(self.span(argument.span())),
            )),
            span: Some(self.span(span)),
        });
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
                    if let Some(children) = self.lower_expression_child(container) {
                        flush_text(
                            &mut nodes,
                            &mut text_segments,
                            Some(self.span(container.span)),
                        );
                        nodes.extend(children);
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
    ) -> Option<Vec<RenderNode>> {
        self.lower_jsx_expression_child(&container.expression)
    }

    fn lower_jsx_expression_child(
        &mut self,
        expression: &JSXExpression<'a>,
    ) -> Option<Vec<RenderNode>> {
        match expression {
            JSXExpression::CallExpression(call) => self.lower_call_child(call),
            JSXExpression::JSXElement(element) => Some(vec![self.lower_jsx_element(element).0]),
            JSXExpression::JSXFragment(fragment) => Some(self.lower_jsx_fragment(fragment)),
            JSXExpression::ParenthesizedExpression(expression) => {
                self.lower_expression_child_expression(&expression.expression)
            }
            JSXExpression::LogicalExpression(logical) if logical.operator.as_str() == "&&" => {
                let children = self.lower_expression_child_expression(&logical.right)?;
                let condition = self.lower_expression(&logical.left);
                Some(vec![self.control_flow_wrapper(
                    RenderControlFlowKind::If,
                    Some(condition),
                    logical.span,
                    children,
                    Vec::new(),
                )])
            }
            JSXExpression::ConditionalExpression(conditional) => self.lower_conditional_child(
                &conditional.test,
                &conditional.consequent,
                &conditional.alternate,
                conditional.span,
            ),
            JSXExpression::NullLiteral(_) => Some(Vec::new()),
            _ => None,
        }
    }

    fn lower_expression_child_expression(
        &mut self,
        expression: &Expression<'a>,
    ) -> Option<Vec<RenderNode>> {
        match expression {
            Expression::CallExpression(call) => self.lower_call_child(call),
            Expression::JSXElement(element) => Some(vec![self.lower_jsx_element(element).0]),
            Expression::JSXFragment(fragment) => Some(self.lower_jsx_fragment(fragment)),
            Expression::ParenthesizedExpression(expression) => {
                self.lower_expression_child_expression(&expression.expression)
            }
            Expression::LogicalExpression(logical) if logical.operator.as_str() == "&&" => {
                let children = self.lower_expression_child_expression(&logical.right)?;
                let condition = self.lower_expression(&logical.left);
                Some(vec![self.control_flow_wrapper(
                    RenderControlFlowKind::If,
                    Some(condition),
                    logical.span,
                    children,
                    Vec::new(),
                )])
            }
            Expression::ConditionalExpression(conditional) => self.lower_conditional_child(
                &conditional.test,
                &conditional.consequent,
                &conditional.alternate,
                conditional.span,
            ),
            Expression::NullLiteral(_) => Some(Vec::new()),
            _ => None,
        }
    }

    fn lower_call_child(&mut self, call: &CallExpression<'a>) -> Option<Vec<RenderNode>> {
        if let Some(node) = self.lower_snippet_call(call) {
            return Some(vec![node]);
        }
        if let Some(node) = self.lower_flat_map_call(call) {
            return Some(vec![node]);
        }
        if let Some(node) = self.lower_map_call(call) {
            return Some(vec![node]);
        }
        self.lower_iife_call(call)
    }

    fn lower_snippet_call(&mut self, call: &CallExpression<'a>) -> Option<RenderNode> {
        let name = simple_callee_name(&call.callee)?;
        if !self.snippets.contains(name) {
            return None;
        }
        let mut render = self.empty_element("jsx-render", call.span);
        render.attributes.push(RenderAttribute {
            name: "data-htmlswap-render".into(),
            value: self
                .rewrite_react_source(self.source_for_span(call.span).trim())
                .into(),
            template: None,
            span: Some(self.span(call.span)),
        });
        Some(RenderNode::Element(Box::new(render)))
    }

    fn lower_snippet_element(&mut self, tag: &str, element: &JSXElement<'a>) -> RenderNode {
        let mut render = self.empty_element("jsx-render", element.span);
        let props = self.component_call_props(element).unwrap_or_default();
        let arguments = props
            .into_iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect::<Vec<_>>()
            .join(", ");
        render.attributes.push(RenderAttribute {
            name: "data-htmlswap-render".into(),
            value: format!("{tag}({{ {arguments} }})").into(),
            template: None,
            span: Some(self.span(element.span)),
        });
        render
            .children
            .extend(self.lower_children(&element.children));
        RenderNode::Element(Box::new(render))
    }

    fn lower_flat_map_call(&mut self, call: &CallExpression<'a>) -> Option<RenderNode> {
        let Expression::StaticMemberExpression(member) = &call.callee else {
            return None;
        };
        if member.property.name.as_str() != "flatMap" {
            return None;
        }
        let argument = call.arguments.first()?;
        let callback = argument_arrow_function(argument)?;
        let binding = callback
            .params
            .items
            .first()
            .and_then(|param| self.callback_binding(&param.pattern))?;
        let index_binding = callback
            .params
            .items
            .get(1)
            .and_then(|param| self.callback_binding(&param.pattern));
        let expression = returned_expression(&callback.body)?;
        let Expression::CallExpression(inner_call) = expression else {
            return None;
        };
        let child = self.lower_map_call(inner_call)?;
        let mut wrapper = self.empty_element("jsx-each", call.span);
        wrapper.control_flow = Some(Box::new(RenderControlFlow {
            kind: RenderControlFlowKind::For,
            host: RenderControlFlowHost::Wrapper,
            expression: Some(self.lower_expression(&member.object)),
            binding: Some(binding),
            index_binding,
            key: None,
            locals: self.callback_locals(callback),
            placeholder: None,
            span: Some(self.span(call.span)),
        }));
        wrapper.children = vec![child];
        Some(RenderNode::Element(Box::new(wrapper)))
    }

    fn lower_conditional_child(
        &mut self,
        test: &Expression<'a>,
        consequent: &Expression<'a>,
        alternate: &Expression<'a>,
        span: OxcSpan,
    ) -> Option<Vec<RenderNode>> {
        let consequent_nodes = self.lower_expression_child_expression(consequent)?;
        let alternate_nodes = self.lower_expression_child_expression(alternate)?;
        if consequent_nodes.is_empty() && alternate_nodes.is_empty() {
            return Some(Vec::new());
        }
        if consequent_nodes.is_empty() {
            return Some(vec![self.control_flow_wrapper(
                RenderControlFlowKind::If,
                Some(Expr::Opaque(
                    format!("!({})", self.source_for_span(test.span())).into(),
                )),
                span,
                alternate_nodes,
                Vec::new(),
            )]);
        }

        let condition = self.lower_expression(test);
        let mut nodes = vec![self.control_flow_wrapper(
            RenderControlFlowKind::If,
            Some(condition),
            span,
            consequent_nodes,
            Vec::new(),
        )];
        if !alternate_nodes.is_empty() {
            nodes.push(self.control_flow_wrapper(
                RenderControlFlowKind::Else,
                None,
                alternate.span(),
                alternate_nodes,
                Vec::new(),
            ));
        }
        Some(nodes)
    }

    fn lower_iife_call(&mut self, call: &CallExpression<'a>) -> Option<Vec<RenderNode>> {
        if !call.arguments.is_empty() {
            return None;
        }
        let callback = match &call.callee {
            Expression::ArrowFunctionExpression(callback) => callback,
            Expression::ParenthesizedExpression(expression) => {
                let Expression::ArrowFunctionExpression(callback) = &expression.expression else {
                    return None;
                };
                callback
            }
            _ => return None,
        };
        let locals = self.callback_locals(callback);
        let return_expression = returned_expression(&callback.body)?;
        let children = self.lower_return_expression(return_expression);
        if locals.is_empty() {
            return Some(children);
        }
        Some(vec![self.control_flow_wrapper(
            RenderControlFlowKind::If,
            Some(Expr::Literal(ExprLiteral::Bool(true))),
            call.span,
            children,
            locals,
        )])
    }

    fn control_flow_wrapper(
        &self,
        kind: RenderControlFlowKind,
        expression: Option<Expr>,
        span: OxcSpan,
        children: Vec<RenderNode>,
        locals: Vec<RenderLoopLocal>,
    ) -> RenderNode {
        let tag = match kind {
            RenderControlFlowKind::If => "jsx-if",
            RenderControlFlowKind::ElseIf => "jsx-else-if",
            RenderControlFlowKind::Else => "jsx-else",
            _ => "jsx-flow",
        };
        let mut wrapper = self.empty_element(tag, span);
        wrapper.control_flow = Some(Box::new(RenderControlFlow {
            kind,
            host: RenderControlFlowHost::Wrapper,
            expression,
            binding: None,
            index_binding: None,
            key: None,
            locals,
            placeholder: None,
            span: Some(self.span(span)),
        }));
        wrapper.children = children;
        RenderNode::Element(Box::new(wrapper))
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
        let Some((children, key)) = self.lower_map_callback_children(callback) else {
            self.diagnostics.push(Diagnostic::error(
                "JSX .map(...) callback must return JSX in Phase 0",
                Some(self.span(callback.body.span)),
            ));
            return None;
        };
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

    fn lower_map_callback_children(
        &mut self,
        callback: &ArrowFunctionExpression<'a>,
    ) -> Option<(Vec<RenderNode>, Option<Expr>)> {
        let mut branches = Vec::new();
        for statement in &callback.body.statements {
            match statement {
                Statement::IfStatement(statement) => {
                    if let Some(branch) =
                        self.lower_if_return_branch(statement, branches.is_empty())
                    {
                        branches.extend(branch);
                    }
                }
                Statement::ReturnStatement(statement) => {
                    let expression = statement.argument.as_ref()?;
                    if branches.is_empty() {
                        return Some(self.lower_map_return(expression));
                    }
                    let (children, key) = self.lower_map_return(expression);
                    if !children.is_empty() {
                        branches.push(self.control_flow_wrapper(
                            RenderControlFlowKind::Else,
                            None,
                            statement.span,
                            children,
                            Vec::new(),
                        ));
                    }
                    return Some((branches, key));
                }
                _ => {}
            }
        }
        let expression = returned_expression(&callback.body)?;
        Some(self.lower_map_return(expression))
    }

    fn lower_if_return_branch(
        &mut self,
        statement: &oxc_ast::ast::IfStatement<'a>,
        first: bool,
    ) -> Option<Vec<RenderNode>> {
        let expression = return_expression_from_statement(&statement.consequent)?;
        let children = self.lower_map_return(expression).0;
        if children.is_empty() {
            return None;
        }
        let condition = self.lower_expression(&statement.test);
        let mut nodes = vec![self.control_flow_wrapper(
            if first {
                RenderControlFlowKind::If
            } else {
                RenderControlFlowKind::ElseIf
            },
            Some(condition),
            statement.span,
            children,
            Vec::new(),
        )];
        if let Some(alternate) = &statement.alternate {
            match alternate {
                Statement::IfStatement(alternate) => {
                    if let Some(branch) = self.lower_if_return_branch(alternate, false) {
                        nodes.extend(branch);
                    }
                }
                _ => {
                    if let Some(expression) = return_expression_from_statement(alternate) {
                        let children = self.lower_map_return(expression).0;
                        if !children.is_empty() {
                            nodes.push(self.control_flow_wrapper(
                                RenderControlFlowKind::Else,
                                None,
                                alternate.span(),
                                children,
                                Vec::new(),
                            ));
                        }
                    }
                }
            }
        }
        Some(nodes)
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
            Expression::ConditionalExpression(conditional) => (
                self.lower_conditional_child(
                    &conditional.test,
                    &conditional.consequent,
                    &conditional.alternate,
                    conditional.span,
                )
                .unwrap_or_default(),
                None,
            ),
            Expression::LogicalExpression(logical) if logical.operator.as_str() == "&&" => (
                self.lower_expression_child_expression(expression)
                    .unwrap_or_default(),
                None,
            ),
            Expression::NullLiteral(_) => (Vec::new(), None),
            _ => {
                self.diagnostics.push(Diagnostic::error(
                    "JSX .map(...) callback must return JSX, a JSX conditional, or null in Phase 3",
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
                let name = binding_identifier_name(&declarator.id).unwrap_or_else(|| {
                    CompactString::from(self.source_for_span(declarator.id.span()).trim())
                });
                let Some(init) = &declarator.init else {
                    continue;
                };
                locals.push(RenderLoopLocal {
                    name,
                    value: Expr::Opaque(
                        self.rewrite_react_source(self.source_for_span(init.span()))
                            .into(),
                    ),
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
        if let Some(alias) = self.source_alias_for_span(expression.span()) {
            return Expr::path([alias.as_str()]);
        }
        match expression {
            JSXExpression::Identifier(identifier) => self
                .setter_reference_expression(identifier.name.as_str())
                .unwrap_or_else(|| Expr::path([identifier.name.as_str()])),
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
            _ => Expr::Opaque(
                self.rewrite_react_source(self.source_for_span(expression.span()))
                    .into(),
            ),
        }
    }

    fn lower_expression(&mut self, expression: &Expression<'a>) -> Expr {
        if let Some(alias) = self.source_alias_for_span(expression.span()) {
            return Expr::path([alias.as_str()]);
        }
        match expression {
            Expression::Identifier(identifier) => self
                .setter_reference_expression(identifier.name.as_str())
                .unwrap_or_else(|| Expr::path([identifier.name.as_str()])),
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
            _ => Expr::Opaque(
                self.rewrite_react_source(self.source_for_span(expression.span()))
                    .into(),
            ),
        }
    }

    fn lower_call_expression(&mut self, call: &CallExpression<'a>) -> Expr {
        if let Some(name) = simple_callee_name(&call.callee)
            && self.setters.contains_key(name)
        {
            return Expr::Opaque(
                self.rewrite_react_source(self.source_for_span(call.span))
                    .into(),
            );
        }
        Expr::Call {
            callee: Box::new(self.lower_expression(&call.callee)),
            arguments: call
                .arguments
                .iter()
                .map(|argument| {
                    Expr::Opaque(
                        self.rewrite_react_source(self.source_for_span(argument.span()))
                            .into(),
                    )
                })
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
                        Some(Expr::Opaque(
                            self.rewrite_react_source(self.source_for_span(element.span()))
                                .into(),
                        ))
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
            _ => None,
        })
        .or_else(|| {
            body.statements
                .iter()
                .find_map(|statement| match statement {
                    Statement::ExpressionStatement(statement) => Some(&statement.expression),
                    _ => None,
                })
        })
}

fn return_expression_from_statement<'b, 'a>(
    statement: &'b Statement<'a>,
) -> Option<&'b Expression<'a>> {
    match statement {
        Statement::ReturnStatement(statement) => statement.argument.as_ref(),
        Statement::BlockStatement(block) => {
            block.body.iter().find_map(return_expression_from_statement)
        }
        _ => None,
    }
}

fn statement_is_component_return_guard(statement: &Statement<'_>) -> bool {
    let Statement::IfStatement(statement) = statement else {
        return false;
    };
    statement.alternate.is_none() && render_return_expression_from_statement(&statement.consequent)
}

fn render_return_expression_from_statement(statement: &Statement<'_>) -> bool {
    match statement {
        Statement::ReturnStatement(statement) => statement
            .argument
            .as_ref()
            .is_some_and(render_return_expression),
        Statement::BlockStatement(block) => {
            let mut saw_return = false;
            for statement in &block.body {
                match statement {
                    Statement::VariableDeclaration(_) => {}
                    Statement::ReturnStatement(statement) => {
                        saw_return = statement
                            .argument
                            .as_ref()
                            .is_some_and(render_return_expression);
                        break;
                    }
                    _ => return false,
                }
            }
            saw_return
        }
        _ => false,
    }
}

fn render_return_expression(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::ParenthesizedExpression(expression) => {
            render_return_expression(&expression.expression)
        }
        Expression::JSXElement(_) | Expression::JSXFragment(_) => true,
        Expression::NullLiteral(_) => true,
        Expression::LogicalExpression(logical) if logical.operator.as_str() == "&&" => {
            render_return_expression(&logical.right)
        }
        Expression::ConditionalExpression(conditional) => {
            render_return_expression(&conditional.consequent)
                && render_return_expression(&conditional.alternate)
        }
        _ => false,
    }
}

fn assignment_target_is_window_member(target: &AssignmentTarget<'_>) -> bool {
    let AssignmentTarget::StaticMemberExpression(member) = target else {
        return false;
    };
    matches!(&member.object, Expression::Identifier(identifier) if identifier.name.as_str() == "window")
}

fn window_assignment_component_name(expression: &Expression<'_>) -> Option<CompactString> {
    let Expression::AssignmentExpression(assignment) = expression else {
        return None;
    };
    if assignment.operator != AssignmentOperator::Assign
        || !assignment_target_is_window_member(&assignment.left)
    {
        return None;
    }
    let Expression::Identifier(identifier) = &assignment.right else {
        return None;
    };
    component_identifier_name(identifier.name.as_str())
        .then(|| CompactString::from(identifier.name.as_str()))
}

fn module_export_name(name: &ModuleExportName<'_>) -> Option<CompactString> {
    match name {
        ModuleExportName::IdentifierName(identifier) => Some(identifier.name.as_str().into()),
        ModuleExportName::IdentifierReference(identifier) => Some(identifier.name.as_str().into()),
        ModuleExportName::StringLiteral(literal) => Some(literal.value.as_str().into()),
    }
}

fn component_identifier_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_uppercase() && chars.any(|ch| ch.is_ascii_lowercase()) && !name.contains('_')
}

fn component_reference_name(name: &str) -> CompactString {
    name.strip_prefix("window.").unwrap_or(name).into()
}

fn function_body_contains_jsx(
    body: &Option<oxc_allocator::Box<'_, FunctionBody<'_>>>,
    source: &str,
) -> bool {
    body.as_deref()
        .is_some_and(|body| function_body_statements_contain_jsx(body, source))
}

fn function_body_statements_contain_jsx(body: &FunctionBody<'_>, source: &str) -> bool {
    body.statements
        .iter()
        .any(|statement| statement_contains_jsx(statement, source))
}

fn statement_contains_jsx(statement: &Statement<'_>, source: &str) -> bool {
    match statement {
        Statement::ExpressionStatement(statement) => {
            expression_contains_jsx(&statement.expression, source)
        }
        Statement::ReturnStatement(statement) => statement
            .argument
            .as_ref()
            .is_some_and(|expression| expression_contains_jsx(expression, source)),
        Statement::VariableDeclaration(declaration) => {
            declaration.declarations.iter().any(|declarator| {
                declarator
                    .init
                    .as_ref()
                    .is_some_and(|expression| expression_contains_jsx(expression, source))
            })
        }
        Statement::IfStatement(statement) => {
            statement_contains_jsx(&statement.consequent, source)
                || statement
                    .alternate
                    .as_ref()
                    .is_some_and(|alternate| statement_contains_jsx(alternate, source))
        }
        Statement::BlockStatement(block) => block
            .body
            .iter()
            .any(|statement| statement_contains_jsx(statement, source)),
        _ => source
            .get(statement.span().start as usize..statement.span().end as usize)
            .is_some_and(|text| text.contains("</") || text.contains("<>")),
    }
}

fn expression_contains_jsx(expression: &Expression<'_>, source: &str) -> bool {
    match expression {
        Expression::JSXElement(_) | Expression::JSXFragment(_) => true,
        Expression::ParenthesizedExpression(expression) => {
            expression_contains_jsx(&expression.expression, source)
        }
        Expression::ArrowFunctionExpression(function) => {
            function_body_statements_contain_jsx(&function.body, source)
        }
        Expression::CallExpression(call) => {
            expression_contains_jsx(&call.callee, source)
                || call
                    .arguments
                    .iter()
                    .any(|argument| argument_contains_jsx(argument, source))
        }
        Expression::ObjectExpression(object) => object.properties.iter().any(|property| {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                return false;
            };
            expression_contains_jsx(&property.value, source)
        }),
        Expression::ArrayExpression(array) => array
            .elements
            .iter()
            .any(|element| array_element_contains_jsx(element, source)),
        Expression::ConditionalExpression(conditional) => {
            expression_contains_jsx(&conditional.test, source)
                || expression_contains_jsx(&conditional.consequent, source)
                || expression_contains_jsx(&conditional.alternate, source)
        }
        Expression::LogicalExpression(logical) => {
            expression_contains_jsx(&logical.left, source)
                || expression_contains_jsx(&logical.right, source)
        }
        Expression::BinaryExpression(binary) => {
            expression_contains_jsx(&binary.left, source)
                || expression_contains_jsx(&binary.right, source)
        }
        _ => false,
    }
}

fn argument_contains_jsx(argument: &Argument<'_>, source: &str) -> bool {
    match argument {
        Argument::JSXElement(_) | Argument::JSXFragment(_) => true,
        Argument::ArrowFunctionExpression(function) => {
            function_body_statements_contain_jsx(&function.body, source)
        }
        _ => false,
    }
}

fn array_element_contains_jsx(element: &ArrayExpressionElement<'_>, source: &str) -> bool {
    match element {
        ArrayExpressionElement::JSXElement(_) | ArrayExpressionElement::JSXFragment(_) => true,
        ArrayExpressionElement::SpreadElement(spread) => {
            expression_contains_jsx(&spread.argument, source)
        }
        _ => false,
    }
}

fn expression_is_window_publish(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::AssignmentExpression(assignment) => {
            assignment_target_is_window_member(&assignment.left)
        }
        Expression::CallExpression(call) => call_is_object_assign_window(call),
        _ => false,
    }
}

fn call_is_object_assign_window(call: &CallExpression<'_>) -> bool {
    let Expression::StaticMemberExpression(member) = &call.callee else {
        return false;
    };
    if !matches!(&member.object, Expression::Identifier(identifier) if identifier.name.as_str() == "Object")
        || member.property.name.as_str() != "assign"
    {
        return false;
    }
    matches!(
        call.arguments.first(),
        Some(Argument::Identifier(identifier)) if identifier.name.as_str() == "window"
    )
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

fn remove_state_item(items: &mut Vec<RenderSourceLogicItem>, name: &str) {
    items.retain(|item| {
        !matches!(
            item,
            RenderSourceLogicItem::State(RenderSourceState { name: state_name, .. })
                if state_name == name
        )
    });
}

fn component_setup_statements<'b, 'a>(body: &'b FunctionBody<'a>) -> Vec<&'b Statement<'a>> {
    body.statements
        .iter()
        .take_while(|statement| {
            !matches!(statement, Statement::ReturnStatement(_))
                && !statement_is_component_return_guard(statement)
        })
        .collect()
}

fn variable_kind_keyword(kind: VariableDeclarationKind) -> &'static str {
    match kind {
        VariableDeclarationKind::Var => "var",
        VariableDeclarationKind::Let => "let",
        VariableDeclarationKind::Const => "const",
        VariableDeclarationKind::Using => "using",
        VariableDeclarationKind::AwaitUsing => "await using",
    }
}

fn call_is_react_hook(call: &CallExpression<'_>, hook: &str) -> bool {
    let Expression::StaticMemberExpression(member) = &call.callee else {
        return false;
    };
    member.property.name.as_str() == hook
        && matches!(&member.object, Expression::Identifier(identifier) if identifier.name.as_str() == "React")
}

fn simple_callee_name<'b, 'a>(expression: &'b Expression<'a>) -> Option<&'b str> {
    match expression {
        Expression::Identifier(identifier) => Some(identifier.name.as_str()),
        _ => None,
    }
}

fn source_contains_assignment_to(source: &str, name: &str) -> bool {
    let mut offset = 0usize;
    while let Some(index) = find_identifier(source, name, offset) {
        let cursor = skip_ascii_whitespace(source, index + name.len());
        if source[cursor..].starts_with('=') && !source[cursor..].starts_with("==") {
            return true;
        }
        offset = index + name.len();
    }
    false
}

fn source_identifier_roots(source: &str) -> BTreeSet<String> {
    let mut roots = BTreeSet::new();
    let mut cursor = 0usize;
    while cursor < source.len() {
        if let Some((name, end)) = scan_identifier_at(source, cursor) {
            if !JS_RESERVED_ROOTS.contains(&name) {
                roots.insert(name.to_owned());
            }
            cursor = end;
            continue;
        }
        cursor += char_at(source, cursor).map_or(1, char::len_utf8);
    }
    roots
}

fn binding_pattern_locals(source: &str) -> Vec<String> {
    source_identifier_roots(source).into_iter().collect()
}

fn component_props_from_pattern(source: &str) -> Vec<ComponentPropInit> {
    let trimmed = source.trim();
    let inner = trimmed
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
        .unwrap_or(trimmed);
    split_top_level_commas(inner)
        .into_iter()
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() || part.starts_with("...") || part.contains(':') {
                return None;
            }
            let (name, default) = part
                .split_once('=')
                .map_or((part, None), |(name, default)| {
                    (name.trim(), Some(default.trim()))
                });
            is_simple_identifier(name).then(|| ComponentPropInit {
                name: name.into(),
                default: default.map(CompactString::from),
            })
        })
        .collect()
}

fn split_top_level_commas(source: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut cursor = 0usize;
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    while cursor < source.len() {
        let ch = char_at(source, cursor).expect("cursor is within source");
        let next = cursor + ch.len_utf8();
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
            ',' if depth == 0 => {
                parts.push(&source[start..cursor]);
                cursor = next;
                start = cursor;
            }
            _ => cursor = next,
        }
    }
    parts.push(&source[start..]);
    parts
}

fn rewrite_setter_calls(source: &str, setters: &BTreeMap<CompactString, CompactString>) -> String {
    if setters.is_empty() {
        return source.to_owned();
    }

    let mut output = String::with_capacity(source.len());
    let mut cursor = 0usize;
    while cursor < source.len() {
        let Some((name, name_end)) = scan_identifier_at(source, cursor) else {
            let ch = char_at(source, cursor).expect("cursor is within source");
            output.push(ch);
            cursor += ch.len_utf8();
            continue;
        };

        let Some(state) = setters.get(name) else {
            output.push_str(name);
            cursor = name_end;
            continue;
        };

        let paren = skip_ascii_whitespace(source, name_end);
        if !source[paren..].starts_with('(') {
            output.push_str(name);
            cursor = name_end;
            continue;
        }
        let Some(call_end) = scan_balanced(source, paren, '(', ')') else {
            output.push_str(name);
            cursor = name_end;
            continue;
        };
        let args = &source[paren + 1..call_end - 1];
        let first_arg = first_call_argument(args).trim();
        if first_arg.is_empty() {
            output.push_str(name);
            cursor = name_end;
            continue;
        }
        let first_arg = rewrite_setter_calls(first_arg, setters);
        if top_level_contains_arrow(&first_arg) {
            output.push_str(&format!("{state} = ({first_arg})({state})"));
        } else {
            output.push_str(&format!("{state} = {first_arg}"));
        }
        cursor = call_end;
    }
    output
}

fn rewrite_source_aliases(
    source: &str,
    aliases: &BTreeMap<CompactString, CompactString>,
) -> String {
    if aliases.is_empty() {
        return source.to_owned();
    }
    aliases
        .iter()
        .fold(source.to_owned(), |rewritten, (from, to)| {
            replace_source_alias(&rewritten, from.as_str(), to.as_str())
        })
}

fn replace_source_alias(source: &str, from: &str, to: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0usize;
    while let Some(relative) = source[cursor..].find(from) {
        let index = cursor + relative;
        let end = index + from.len();
        let before_ok = source[..index]
            .chars()
            .next_back()
            .is_none_or(|ch| !is_identifier_continue(ch) && ch != '.');
        let after_ok = source[end..]
            .chars()
            .next()
            .is_none_or(|ch| !is_identifier_continue(ch));
        if before_ok && after_ok {
            output.push_str(&source[cursor..index]);
            output.push_str(to);
            cursor = end;
        } else {
            output.push_str(&source[cursor..end]);
            cursor = end;
        }
    }
    output.push_str(&source[cursor..]);
    output
}

fn first_call_argument(args: &str) -> &str {
    let mut cursor = 0usize;
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    while cursor < args.len() {
        let ch = char_at(args, cursor).expect("cursor is within args");
        let next = cursor + ch.len_utf8();
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
            '\'' | '"' | '`' => quote = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => return &args[..cursor],
            _ => {}
        }
        cursor = next;
    }
    args
}

fn top_level_contains_arrow(value: &str) -> bool {
    let mut cursor = 0usize;
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    while cursor < value.len() {
        let ch = char_at(value, cursor).expect("cursor is within value");
        let next = cursor + ch.len_utf8();
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
            '\'' | '"' | '`' => quote = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '=' if depth == 0 && value[next..].starts_with('>') => return true,
            _ => {}
        }
        cursor = next;
    }
    false
}

fn scan_balanced(source: &str, start: usize, open: char, close: char) -> Option<usize> {
    let mut cursor = start;
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = false;
    while cursor < source.len() {
        let ch = char_at(source, cursor)?;
        let next = cursor + ch.len_utf8();
        if line_comment {
            line_comment = ch != '\n';
            cursor = next;
            continue;
        }
        if block_comment {
            if ch == '*' && char_at(source, next) == Some('/') {
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
            '/' if char_at(source, next) == Some('/') => {
                line_comment = true;
                cursor = next + 1;
            }
            '/' if char_at(source, next) == Some('*') => {
                block_comment = true;
                cursor = next + 1;
            }
            '\'' | '"' | '`' => {
                quote = Some(ch);
                cursor = next;
            }
            ch if ch == open => {
                depth += 1;
                cursor = next;
            }
            ch if ch == close => {
                depth = depth.saturating_sub(1);
                cursor = next;
                if depth == 0 {
                    return Some(cursor);
                }
            }
            _ => cursor = next,
        }
    }
    None
}

fn find_identifier(source: &str, name: &str, mut offset: usize) -> Option<usize> {
    while let Some(relative) = source[offset..].find(name) {
        let index = offset + relative;
        let end = index + name.len();
        let before_ok = source[..index]
            .chars()
            .next_back()
            .is_none_or(|ch| !is_identifier_continue(ch));
        let after_ok = source[end..]
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

fn scan_identifier_at(source: &str, start: usize) -> Option<(&str, usize)> {
    let mut chars = source[start..].char_indices();
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
    Some((&source[start..end], end))
}

fn skip_ascii_whitespace(source: &str, mut cursor: usize) -> usize {
    while let Some(byte) = source.as_bytes().get(cursor) {
        if !byte.is_ascii_whitespace() {
            break;
        }
        cursor += 1;
    }
    cursor
}

fn char_at(source: &str, cursor: usize) -> Option<char> {
    source.get(cursor..)?.chars().next()
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

const JS_RESERVED_ROOTS: &[&str] = &[
    "Array",
    "Boolean",
    "Date",
    "Error",
    "JSON",
    "Math",
    "Number",
    "Object",
    "Promise",
    "React",
    "String",
    "console",
    "document",
    "event",
    "false",
    "function",
    "if",
    "let",
    "null",
    "return",
    "this",
    "true",
    "undefined",
    "window",
];

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
        _ if react_event_attribute_name(name).is_some() => react_event_attribute_name(name)
            .expect("checked above")
            .into(),
        _ if name.starts_with("aria-") || name.starts_with("data-") => name.into(),
        _ if let Some(mapped) = react_attribute_alias(name) => mapped.into(),
        _ if name.chars().any(char::is_uppercase) => camel_to_kebab(name).into(),
        _ => name.into(),
    }
}

fn react_event_attribute_name(name: &str) -> Option<String> {
    let event = name.strip_prefix("on")?;
    let first = event.chars().next()?;
    if !first.is_ascii_uppercase() {
        return None;
    }
    let normalized = match event {
        "DoubleClick" => "dblclick".to_owned(),
        _ => event.to_ascii_lowercase(),
    };
    Some(format!("on{normalized}"))
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
        "jsx-each" | "jsx-if" | "jsx-else-if" | "jsx-else" | "jsx-flow" => UiRole::Unknown,
        _ => UiRole::Container,
    }
}
