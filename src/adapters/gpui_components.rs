use crate::adapter::{
    Adapter, AdapterContext, AdapterError, HtmlArtifact, Importer, LayerClaim, RouteConfig,
    RouteTarget, TargetArtifact, TargetDependency,
};
use crate::adapters::gpui::{
    AccessibilityEmission, ActionEmission, ChildEmission, GPUI_LAYER_ID, GpuiAdapterOptions,
    GpuiElementContext, GpuiElementSpec, GpuiOutput, GpuiTargetLayer, IdAttributeEmission,
    StateActionEmission, StyleEmission, ThemeEmission, TitleAttributeEmission, adapt_with_layers,
    attribute_span, attribute_value, component_text_input_host_expression, element_id,
    gpui_base_layer, has_boolean_attribute, is_material_symbol_element, rust_string,
    text_only_children,
};
use crate::adapters::gpui_reverse::{GpuiImportMode, import_gpui_target};
use crate::adapters::gpui_semantics::{
    BUTTON_LOADING, SemanticAxis, component_size_variant, semantic_method,
};
use crate::compiler::CompiledFragment;
use crate::plan::{
    RenderDensity, RenderElement, RenderSize, RenderStateKind, RenderTone, RenderVariant, UiRole,
};

pub const GPUI_COMPONENT_CRATE_VERSION: &str = "0.5.1";
pub const GPUI_COMPONENTS_LAYER_ID: &str = "gpui-components";
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuiComponentsAdapterOptions {
    pub gpui: GpuiAdapterOptions,
    pub routes: RouteConfig,
}

impl Default for GpuiComponentsAdapterOptions {
    fn default() -> Self {
        Self::components()
    }
}

impl GpuiComponentsAdapterOptions {
    #[must_use]
    pub fn components() -> Self {
        let layer = gpui_components_layer();
        Self {
            gpui: GpuiAdapterOptions {
                theme: ThemeEmission::PreferTheme,
                ..GpuiAdapterOptions::default()
            },
            routes: RouteConfig::new()
                .with_base(GPUI_LAYER_ID)
                .with_layer(layer.id(), layer.claim()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct GpuiComponentsAdapter {
    options: GpuiComponentsAdapterOptions,
}

impl GpuiComponentsAdapter {
    #[must_use]
    pub fn new(options: GpuiComponentsAdapterOptions) -> Self {
        Self { options }
    }

    #[must_use]
    pub fn options(&self) -> &GpuiComponentsAdapterOptions {
        &self.options
    }
}

impl Adapter for GpuiComponentsAdapter {
    type Output = GpuiOutput;

    fn adapt(
        &self,
        fragment: &CompiledFragment,
        cx: &mut AdapterContext,
    ) -> Result<Self::Output, AdapterError> {
        let layers = [gpui_base_layer(), gpui_components_layer()];
        adapt_with_layers(
            &self.options.gpui,
            &self.options.routes,
            &layers,
            fragment,
            cx,
        )
    }
}

impl Importer for GpuiComponentsAdapter {
    type Source = TargetArtifact;
    type Output = HtmlArtifact;

    fn import(
        &self,
        source: &Self::Source,
        cx: &mut AdapterContext,
    ) -> Result<Self::Output, AdapterError> {
        import_gpui_target(source, cx, GpuiImportMode::GpuiComponents)
    }
}

struct GpuiComponentsLayer;

impl GpuiTargetLayer for GpuiComponentsLayer {
    fn id(&self) -> &'static str {
        GPUI_COMPONENTS_LAYER_ID
    }

    fn claim(&self) -> LayerClaim {
        LayerClaim::Targets(vec![
            RouteTarget::component("titlebar"),
            RouteTarget::component("tabs"),
            RouteTarget::component("tabbar"),
            RouteTarget::component("tab"),
            RouteTarget::Role(UiRole::Button),
            RouteTarget::Role(UiRole::TextInput),
            RouteTarget::Role(UiRole::Link),
            RouteTarget::Role(UiRole::Label),
            RouteTarget::Role(UiRole::ListItem),
            RouteTarget::Role(UiRole::Form),
            RouteTarget::Role(UiRole::Fieldset),
            RouteTarget::Role(UiRole::Select),
            RouteTarget::class("material-icons"),
            RouteTarget::class("material-symbols-outlined"),
            RouteTarget::class("ms"),
            RouteTarget::tag("input"),
        ])
    }

    fn dependencies(&self) -> Vec<TargetDependency> {
        vec![TargetDependency::crates_io(
            "gpui-component",
            GPUI_COMPONENT_CRATE_VERSION,
        )]
    }

    fn write_imports(&self, output: &mut String) {
        output.push_str("use gpui_component::button::ButtonVariants as _;\n");
        output.push_str("use gpui_component::Disableable as _;\n");
        output.push_str("use gpui_component::Selectable as _;\n");
        output.push_str("use gpui_component::Sizable as _;\n");
    }

    fn element_spec(
        &self,
        element: &RenderElement,
        context: &mut GpuiElementContext<'_>,
    ) -> Option<GpuiElementSpec> {
        if is_material_symbol_element(element) {
            return self.material_symbol_spec(element);
        }

        if component_is(element, "titlebar") {
            return self.title_bar_spec(element);
        }
        if component_is_any(element, &["tabs", "tabbar", "tab-list"]) {
            return self.tab_bar_spec(element, context);
        }
        if component_is(element, "tab") {
            return self.tab_spec(element);
        }

        match element.role {
            UiRole::Button => self.button_spec(element, context),
            UiRole::Link => self.link_spec(element, context),
            UiRole::Label => self.label_spec(element),
            UiRole::ListItem => self.list_item_spec(element, context),
            UiRole::Form => self.form_spec(element),
            UiRole::Fieldset => self.fieldset_spec(element),
            UiRole::Select => self.select_spec(element, context),
            UiRole::TextInput => self.input_component_spec(element, context),
            _ => None,
        }
    }
}

impl GpuiComponentsLayer {
    fn material_symbol_spec(&self, _element: &RenderElement) -> Option<GpuiElementSpec> {
        Some(
            GpuiElementSpec::new("gpui::div()")
                .with_children(ChildEmission::MaterialIcon)
                .with_actions(ActionEmission::Events)
                .with_accessibility(AccessibilityEmission::Comment),
        )
    }

    fn title_bar_spec(&self, _element: &RenderElement) -> Option<GpuiElementSpec> {
        Some(
            GpuiElementSpec::new("gpui_component::TitleBar::new()")
                .with_id_attribute(IdAttributeEmission::Consumed)
                .with_title_attribute(TitleAttributeEmission::Warn)
                .with_children(ChildEmission::TitleBarChildren)
                .with_style_variants(StyleEmission::Comment)
                .with_actions(ActionEmission::Comment)
                .with_accessibility(AccessibilityEmission::Comment),
        )
    }

    fn tab_bar_spec(
        &self,
        element: &RenderElement,
        context: &mut GpuiElementContext<'_>,
    ) -> Option<GpuiElementSpec> {
        let id = context.element_id_or_generated_id(element, "tabs");
        let mut expression = format!("gpui_component::tab::TabBar::new({})", rust_string(&id));
        push_tab_variant_methods(&mut expression, element, context);
        push_component_size_method(&mut expression, element, context, "TabBar");

        Some(
            GpuiElementSpec::new(expression)
                .with_id_attribute(IdAttributeEmission::Consumed)
                .with_title_attribute(TitleAttributeEmission::Warn)
                .with_actions(ActionEmission::Comment)
                .with_accessibility(AccessibilityEmission::Comment),
        )
    }

    fn tab_spec(&self, element: &RenderElement) -> Option<GpuiElementSpec> {
        let mut expression = "gpui_component::tab::Tab::new()".to_owned();
        push_component_method_if(&mut expression, tab_selected(element), ".selected(true)");
        push_component_method_if(
            &mut expression,
            has_boolean_attribute(element, "disabled")
                || attribute_value(element, "aria-disabled")
                    .is_some_and(|value| value.eq_ignore_ascii_case("true")),
            ".disabled(true)",
        );

        Some(
            GpuiElementSpec::new(expression)
                .with_id_attribute(IdAttributeEmission::Warn)
                .with_title_attribute(TitleAttributeEmission::Warn)
                .with_accessibility(AccessibilityEmission::Comment)
                .with_consumed_attributes(["selected", "data-htmlswap-selected", "disabled"]),
        )
    }

    fn button_spec(
        &self,
        element: &RenderElement,
        context: &mut GpuiElementContext<'_>,
    ) -> Option<GpuiElementSpec> {
        let id = context.element_id_or_generated_id(element, "button");
        let mut expression = format!("gpui_component::button::Button::new({})", rust_string(&id));
        push_button_semantic_methods(&mut expression, element, context);
        push_component_method_if(
            &mut expression,
            has_boolean_attribute(element, "disabled"),
            ".disabled(true)",
        );
        push_component_method_if(&mut expression, button_selected(element), ".selected(true)");
        push_component_method_if(
            &mut expression,
            button_loading(element),
            &format!(".{}(true)", BUTTON_LOADING.method),
        );

        Some(
            GpuiElementSpec::new(expression)
                .with_id_attribute(IdAttributeEmission::Consumed)
                .with_title_attribute(TitleAttributeEmission::ComponentTooltip)
                .with_children(ChildEmission::TextAsLabel)
                .with_accessibility(AccessibilityEmission::Methods)
                .with_consumed_attributes([
                    "disabled",
                    "selected",
                    "data-htmlswap-selected",
                    BUTTON_LOADING.attribute,
                ]),
        )
    }

    fn link_spec(
        &self,
        element: &RenderElement,
        context: &mut GpuiElementContext<'_>,
    ) -> Option<GpuiElementSpec> {
        let id = context.element_id_or_generated_id(element, "link");
        let mut expression = format!("gpui_component::link::Link::new({})", rust_string(&id));

        if let Some(href) = attribute_value(element, "href") {
            push_component_method(&mut expression, &format!(".href({})", rust_string(href)));
        }
        push_component_method_if(
            &mut expression,
            has_boolean_attribute(element, "disabled"),
            ".disabled(true)",
        );

        Some(
            GpuiElementSpec::new(expression)
                .with_id_attribute(IdAttributeEmission::Consumed)
                .with_consumed_attributes(["href", "disabled"]),
        )
    }

    fn label_spec(&self, element: &RenderElement) -> Option<GpuiElementSpec> {
        let text = text_only_children(element)?;

        Some(
            GpuiElementSpec::new(format!(
                "gpui_component::label::Label::new({})",
                rust_string(&text)
            ))
            .with_id_attribute(IdAttributeEmission::Warn)
            .with_actions(ActionEmission::Comment)
            .with_children(ChildEmission::Consumed),
        )
    }

    fn list_item_spec(
        &self,
        element: &RenderElement,
        context: &mut GpuiElementContext<'_>,
    ) -> Option<GpuiElementSpec> {
        let id = context.element_id_or_generated_id(element, "list_item");
        let mut expression = format!("gpui_component::list::ListItem::new({})", rust_string(&id));

        push_component_method_if(
            &mut expression,
            has_boolean_attribute(element, "disabled"),
            ".disabled(true)",
        );
        push_component_method_if(
            &mut expression,
            has_boolean_attribute(element, "selected"),
            ".selected(true)",
        );

        Some(
            GpuiElementSpec::new(expression)
                .with_id_attribute(IdAttributeEmission::Consumed)
                .with_consumed_attributes(["disabled", "selected"]),
        )
    }

    fn input_component_spec(
        &self,
        element: &RenderElement,
        context: &mut GpuiElementContext<'_>,
    ) -> Option<GpuiElementSpec> {
        if element.source_tag == "textarea" {
            return self.text_input_spec(element, context);
        }

        match input_type(element).as_deref() {
            Some("checkbox") => self.checkbox_spec(element, context),
            Some("radio") => self.radio_spec(element, context),
            Some("button" | "submit" | "reset") => self.input_button_spec(element, context),
            _ => self.text_input_spec(element, context),
        }
    }

    fn form_spec(&self, element: &RenderElement) -> Option<GpuiElementSpec> {
        let mut expression = match attribute_value(element, "data-htmlswap-form-layout")
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("horizontal") | Some("row") => {
                "gpui_component::form::Form::horizontal()".to_owned()
            }
            _ => "gpui_component::form::Form::vertical()".to_owned(),
        };

        if let Some(columns) = attribute_value(element, "data-htmlswap-form-columns")
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|columns| *columns > 0)
        {
            push_component_method(&mut expression, &format!(".columns({columns})"));
        }

        let mut spec = GpuiElementSpec::new(expression)
            .with_id_attribute(IdAttributeEmission::Warn)
            .with_children(ChildEmission::FormFields)
            .with_actions(ActionEmission::Events)
            .with_state_actions(StateActionEmission::Methods)
            .with_consumed_attribute("data-htmlswap-form-layout");

        if attribute_value(element, "data-htmlswap-form-columns").is_some() {
            spec = spec.with_consumed_attribute("data-htmlswap-form-columns");
        }

        Some(spec)
    }

    fn fieldset_spec(&self, element: &RenderElement) -> Option<GpuiElementSpec> {
        let mut expression = "gpui_component::group_box::GroupBox::new()".to_owned();

        if let Some(id) = element_id(element) {
            push_component_method(&mut expression, &format!(".id({})", rust_string(id)));
        }
        if let Some(legend) = legend_text(element) {
            push_component_method(
                &mut expression,
                &format!(".title(gpui::div().child({}))", rust_string(&legend)),
            );
        }

        Some(
            GpuiElementSpec::new(expression)
                .with_id_attribute(IdAttributeEmission::Consumed)
                .with_children(ChildEmission::FieldsetChildren)
                .with_actions(ActionEmission::Comment),
        )
    }

    fn select_spec(
        &self,
        element: &RenderElement,
        context: &mut GpuiElementContext<'_>,
    ) -> Option<GpuiElementSpec> {
        let state = element.state.as_ref()?;
        let RenderStateKind::Choice(choice) = &state.kind else {
            return None;
        };
        let state_field = context.state_field(state)?;
        let mut expression = format!("gpui_component::select::Select::new(&{state_field})");

        push_component_method_if(&mut expression, choice.disabled, ".disabled(true)");
        if let Some(placeholder) = attribute_value(element, "placeholder")
            .or_else(|| attribute_value(element, "data-htmlswap-placeholder"))
        {
            push_component_method(
                &mut expression,
                &format!(".placeholder({})", rust_string(placeholder)),
            );
        }

        Some(
            GpuiElementSpec::new(expression)
                .with_id_attribute(IdAttributeEmission::Consumed)
                .with_children(ChildEmission::Consumed)
                .with_actions(ActionEmission::Events)
                .with_state_actions(StateActionEmission::Methods)
                .with_accessibility(AccessibilityEmission::Methods)
                .with_consumed_attributes([
                    "name",
                    "disabled",
                    "required",
                    "aria-label",
                    "placeholder",
                    "data-htmlswap-placeholder",
                    "value",
                ]),
        )
    }

    fn text_input_spec(
        &self,
        element: &RenderElement,
        context: &mut GpuiElementContext<'_>,
    ) -> Option<GpuiElementSpec> {
        let state = element.state.as_ref()?;
        let RenderStateKind::TextInput(input) = &state.kind else {
            return None;
        };
        let state_field = context.state_field(state)?;
        let mut input_expression = format!("gpui_component::input::Input::new(&{state_field})");

        push_component_method_if(&mut input_expression, input.disabled, ".disabled(true)");
        push_component_method_if(&mut input_expression, input.password, ".mask_toggle()");

        Some(
            GpuiElementSpec::new(component_text_input_host_expression(&input_expression))
                .with_id_attribute(IdAttributeEmission::Method)
                .with_title_attribute(TitleAttributeEmission::GpuiTooltipWrapper)
                .with_children(ChildEmission::Consumed)
                .with_style_variants(StyleEmission::Comment)
                .with_actions(ActionEmission::Events)
                .with_state_actions(StateActionEmission::Methods)
                .with_accessibility(AccessibilityEmission::Methods)
                .with_unsupported_action_events(["input"])
                .with_consumed_attributes([
                    "type",
                    "value",
                    "placeholder",
                    "disabled",
                    "required",
                    "name",
                    "aria-label",
                    "rows",
                ]),
        )
    }

    fn checkbox_spec(
        &self,
        element: &RenderElement,
        context: &mut GpuiElementContext<'_>,
    ) -> Option<GpuiElementSpec> {
        let id = context.element_id_or_generated_id(element, "checkbox");
        let mut expression = format!(
            "gpui_component::checkbox::Checkbox::new({})",
            rust_string(&id)
        );

        if let Some(state) = &element.state
            && let RenderStateKind::Toggle(_) = &state.kind
            && let Some(state_field) = context.state_field(state)
        {
            push_component_method(&mut expression, &format!(".checked({state_field})"));
        } else {
            push_component_method_if(
                &mut expression,
                has_boolean_attribute(element, "checked"),
                ".checked(true)",
            );
        }
        push_component_method_if(
            &mut expression,
            has_boolean_attribute(element, "disabled"),
            ".disabled(true)",
        );
        if let Some(label) = form_label(element).or_else(|| attribute_value(element, "aria-label"))
        {
            push_component_method(&mut expression, &format!(".label({})", rust_string(label)));
        }

        Some(
            GpuiElementSpec::new(expression)
                .with_id_attribute(IdAttributeEmission::Consumed)
                .with_children(ChildEmission::Consumed)
                .with_actions(ActionEmission::Events)
                .with_state_actions(StateActionEmission::Methods)
                .with_accessibility(AccessibilityEmission::Methods)
                .with_consumed_attributes([
                    "type",
                    "checked",
                    "disabled",
                    "aria-label",
                    "name",
                    "value",
                ]),
        )
    }

    fn radio_spec(
        &self,
        element: &RenderElement,
        context: &mut GpuiElementContext<'_>,
    ) -> Option<GpuiElementSpec> {
        let id = context.element_id_or_generated_id(element, "radio");
        let mut expression = format!("gpui_component::radio::Radio::new({})", rust_string(&id));

        if let Some(state) = &element.state
            && let RenderStateKind::Toggle(_) = &state.kind
            && let Some(state_field) = context.state_field(state)
        {
            push_component_method(&mut expression, &format!(".checked({state_field})"));
        } else {
            push_component_method_if(
                &mut expression,
                has_boolean_attribute(element, "checked"),
                ".checked(true)",
            );
        }
        push_component_method_if(
            &mut expression,
            has_boolean_attribute(element, "disabled"),
            ".disabled(true)",
        );
        if let Some(label) = form_label(element).or_else(|| attribute_value(element, "aria-label"))
        {
            push_component_method(&mut expression, &format!(".label({})", rust_string(label)));
        }

        Some(
            GpuiElementSpec::new(expression)
                .with_id_attribute(IdAttributeEmission::Consumed)
                .with_children(ChildEmission::Consumed)
                .with_actions(ActionEmission::Events)
                .with_state_actions(StateActionEmission::Methods)
                .with_accessibility(AccessibilityEmission::Methods)
                .with_consumed_attributes([
                    "type",
                    "checked",
                    "disabled",
                    "aria-label",
                    "name",
                    "value",
                ]),
        )
    }

    fn input_button_spec(
        &self,
        element: &RenderElement,
        context: &mut GpuiElementContext<'_>,
    ) -> Option<GpuiElementSpec> {
        let id = context.element_id_or_generated_id(element, "button");
        let input_type = input_type(element).unwrap_or_else(|| "button".to_owned());
        let label = attribute_value(element, "value")
            .map(str::to_owned)
            .unwrap_or_else(|| default_input_button_label(&input_type).to_owned());
        let mut expression = format!("gpui_component::button::Button::new({})", rust_string(&id));
        push_button_semantic_methods(&mut expression, element, context);
        push_component_method(&mut expression, &format!(".label({})", rust_string(&label)));
        push_component_method_if(
            &mut expression,
            has_boolean_attribute(element, "disabled"),
            ".disabled(true)",
        );

        Some(
            GpuiElementSpec::new(expression)
                .with_id_attribute(IdAttributeEmission::Consumed)
                .with_title_attribute(TitleAttributeEmission::ComponentTooltip)
                .with_children(ChildEmission::Consumed)
                .with_accessibility(AccessibilityEmission::Methods)
                .with_consumed_attributes(["type", "value", "disabled"]),
        )
    }
}

static GPUI_COMPONENTS_LAYER: GpuiComponentsLayer = GpuiComponentsLayer;

fn gpui_components_layer() -> &'static dyn GpuiTargetLayer {
    &GPUI_COMPONENTS_LAYER
}

fn input_type(element: &RenderElement) -> Option<String> {
    attribute_value(element, "type").map(|value| value.trim().to_ascii_lowercase())
}

fn form_label(element: &RenderElement) -> Option<&str> {
    element.form_control.as_ref()?.label.as_deref()
}

fn legend_text(element: &RenderElement) -> Option<String> {
    element.children.iter().find_map(|child| {
        let crate::plan::RenderNode::Element(child) = child else {
            return None;
        };
        (child.role == UiRole::Legend)
            .then(|| text_only_children(child))
            .flatten()
    })
}

fn default_input_button_label(input_type: &str) -> &'static str {
    match input_type {
        "submit" => "Submit",
        "reset" => "Reset",
        _ => "Button",
    }
}

fn component_is(element: &RenderElement, expected: &str) -> bool {
    element
        .source_intent
        .as_ref()
        .and_then(|hints| hints.component.as_ref())
        .is_some_and(|component| component.is(expected))
}

fn component_is_any(element: &RenderElement, expected: &[&str]) -> bool {
    element
        .source_intent
        .as_ref()
        .and_then(|hints| hints.component.as_ref())
        .is_some_and(|component| expected.iter().any(|expected| component.is(expected)))
}

fn push_component_method(expression: &mut String, method: &str) {
    expression.push('\n');
    expression.push_str("    ");
    expression.push_str(method);
}

fn push_component_method_if(expression: &mut String, condition: bool, method: &str) {
    if condition {
        push_component_method(expression, method);
    }
}

fn push_button_semantic_methods(
    expression: &mut String,
    element: &RenderElement,
    context: &mut GpuiElementContext<'_>,
) {
    let Some(semantics) = &element.semantics else {
        return;
    };

    if let Some(tone) = &semantics.tone {
        if let Some(method) = semantic_method(SemanticAxis::Tone, tone.as_str()) {
            push_component_method(expression, &format!(".{method}()"));
        } else if !matches!(tone, RenderTone::Neutral) {
            warn_unsupported_button_semantic(
                context,
                element,
                "tone",
                tone.as_str(),
                "no gpui-component Button variant exists for this tone",
            );
        }
    }

    if let Some(variant) = &semantics.variant {
        if let Some(method) = semantic_method(SemanticAxis::Variant, variant.as_str()) {
            push_component_method(expression, &format!(".{method}()"));
        } else {
            match variant {
                RenderVariant::Solid => {}
                RenderVariant::Soft => warn_unsupported_button_semantic(
                    context,
                    element,
                    "variant",
                    variant.as_str(),
                    "no gpui-component Button method exists for soft buttons",
                ),
                _ => warn_unsupported_button_semantic(
                    context,
                    element,
                    "variant",
                    variant.as_str(),
                    "no gpui-component Button method exists for this variant",
                ),
            }
        }
    }

    push_component_size_method(expression, element, context, "Button");

    if let Some(density) = &semantics.density {
        if let Some(method) = semantic_method(SemanticAxis::Density, density.as_str()) {
            push_component_method(expression, &format!(".{method}()"));
        } else {
            match density {
                RenderDensity::Comfortable => {}
                RenderDensity::Spacious => warn_unsupported_button_semantic(
                    context,
                    element,
                    "density",
                    density.as_str(),
                    "no gpui-component Button method exists for spacious density",
                ),
                _ => warn_unsupported_button_semantic(
                    context,
                    element,
                    "density",
                    density.as_str(),
                    "no gpui-component Button method exists for this density",
                ),
            }
        }
    }
}

fn warn_unsupported_button_semantic(
    context: &mut GpuiElementContext<'_>,
    element: &RenderElement,
    axis: &str,
    value: &str,
    detail: &str,
) {
    context.warn(
        format!(
            "gpui-components Button source {axis} `{value}` is not directly supported; {detail}"
        ),
        attribute_span(element, &format!("data-htmlswap-{axis}")).or(element.span),
    );
}

fn push_component_size_method(
    expression: &mut String,
    element: &RenderElement,
    context: &mut GpuiElementContext<'_>,
    component: &str,
) {
    let Some(size) = element
        .semantics
        .as_ref()
        .and_then(|semantics| semantics.size.as_ref())
    else {
        return;
    };

    let variant = match size {
        RenderSize::Xl => {
            warn_unsupported_component_semantic(
                context,
                element,
                component,
                "size",
                size.as_str(),
                "clamped to gpui-component Size::Large",
            );
            component_size_variant(RenderSize::Lg.as_str())
        }
        RenderSize::Custom(value) => {
            warn_unsupported_component_semantic(
                context,
                element,
                component,
                "size",
                value,
                "no gpui-component Size exists for this token",
            );
            None
        }
        size => component_size_variant(size.as_str()),
    };
    if let Some(variant) = variant {
        push_component_method(
            expression,
            &format!(".with_size(gpui_component::Size::{variant})"),
        );
    }
}

fn push_tab_variant_methods(
    expression: &mut String,
    element: &RenderElement,
    context: &mut GpuiElementContext<'_>,
) {
    let Some(variant) = element
        .semantics
        .as_ref()
        .and_then(|semantics| semantics.variant.as_ref())
    else {
        return;
    };

    let value = variant.as_str();
    if let Some(method) = semantic_method(SemanticAxis::TabVariant, value) {
        push_component_method(expression, &format!(".{method}()"));
    } else if value != "solid" {
        warn_unsupported_component_semantic(
            context,
            element,
            "TabBar",
            "variant",
            value,
            "no gpui-component TabBar method exists for this variant",
        );
    }
}

fn warn_unsupported_component_semantic(
    context: &mut GpuiElementContext<'_>,
    element: &RenderElement,
    component: &str,
    axis: &str,
    value: &str,
    detail: &str,
) {
    context.warn(
        format!(
            "gpui-components {component} source {axis} `{value}` is not directly supported; {detail}"
        ),
        attribute_span(element, &format!("data-htmlswap-{axis}")).or(element.span),
    );
}

fn tab_selected(element: &RenderElement) -> bool {
    has_boolean_attribute(element, "selected")
        || has_boolean_attribute(element, "data-htmlswap-selected")
        || attribute_value(element, "aria-selected")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn button_selected(element: &RenderElement) -> bool {
    has_boolean_attribute(element, "selected")
        || has_boolean_attribute(element, "data-htmlswap-selected")
        || attribute_value(element, "aria-pressed")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn button_loading(element: &RenderElement) -> bool {
    has_boolean_attribute(element, BUTTON_LOADING.attribute)
        || attribute_value(element, "aria-busy")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}
