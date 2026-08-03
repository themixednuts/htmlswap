use htmlswap::{
    Adapter, AdapterContext, CompileAssets, Compiler, Frontend, SVELTE_PACKAGE_VERSION, Severity,
    SvelteAdapter, SvelteAdapterOptions,
};

fn adapt_svelte(
    source: &str,
    frontend: Frontend,
    assets: CompileAssets,
) -> (String, AdapterContext) {
    let compiled = Compiler::new()
        .with_frontend(frontend)
        .compile_fragment(source, &assets);
    assert!(
        compiled.diagnostics.is_empty(),
        "{:?}",
        compiled.diagnostics
    );

    let mut cx = AdapterContext::new();
    let options = SvelteAdapterOptions {
        component_name: "Demo".into(),
        emit_source_comments: false,
        ..SvelteAdapterOptions::default()
    };
    let output = SvelteAdapter::new(options)
        .adapt(&compiled.value, &mut cx)
        .expect("Svelte adapter should emit code");
    (output.code().to_owned(), cx)
}

fn emit_svelte(source: &str, frontend: Frontend) -> String {
    let (code, cx) = adapt_svelte(source, frontend, CompileAssets::new());
    assert!(cx.diagnostics().is_empty(), "{:?}", cx.diagnostics());
    code
}

fn emit_svelte_with_assets(source: &str, frontend: Frontend, assets: CompileAssets) -> String {
    let (code, cx) = adapt_svelte(source, frontend, assets);
    assert!(cx.diagnostics().is_empty(), "{:?}", cx.diagnostics());
    code
}

fn output_line_containing<'a>(code: &'a str, needle: &str) -> &'a str {
    code.lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("expected output to contain `{needle}`:\n{code}"))
}

fn style_rule_count(code: &str, selector: &str) -> usize {
    code.lines()
        .filter(|line| line.trim_start().starts_with(selector))
        .count()
}

#[test]
fn svelte_adapter_emits_svelte5_event_attributes() {
    let code = emit_svelte(
        r#"<button class="primary" onclick="{{ save }}">Save</button>"#,
        Frontend::html(),
    );

    assert!(
        code.contains(&format!("svelte@^{SVELTE_PACKAGE_VERSION}")),
        "{code}"
    );
    assert!(
        code.contains("let { save }: { save?: any } = $props();"),
        "{code}"
    );
    assert!(
        code.contains("<button class=\"primary\" onclick={save}>"),
        "{code}"
    );
    assert!(!code.contains("on:click"), "{code}");
    assert!(!code.contains("$effect"), "{code}");
}

#[test]
fn svelte_adapter_emits_target_owned_state_with_runes() {
    let code = emit_svelte(
        r#"<input data-htmlswap-state="name" value="Ada">"#,
        Frontend::html(),
    );

    assert!(code.contains("let name = $state(\"Ada\");"), "{code}");
    assert!(code.contains("<input bind:value={name} />"), "{code}");
    assert!(!code.contains("on:input"), "{code}");
}

#[test]
fn svelte_adapter_emits_snippets_and_attachments() {
    let code = emit_svelte(
        r#"<slot></slot><button data-htmlswap-attach="tooltip(label)">Hover</button>"#,
        Frontend::html(),
    );

    assert!(
        code.contains("import type { Snippet } from 'svelte';"),
        "{code}"
    );
    assert!(code.contains("children?: Snippet"), "{code}");
    assert!(code.contains("{@render children?.()}"), "{code}");
    assert!(code.contains("let { children, label, tooltip }"), "{code}");
    assert!(code.contains("<button {@attach tooltip(label)}>"), "{code}");
    assert!(!code.contains("use:tooltip"), "{code}");
}

#[test]
fn svelte_adapter_emits_dc_control_flow_and_bindings() {
    let code = emit_svelte(
        r#"<x-dc><sc-for list="{{ items }}" as="item"><button onClick="{{ item.onClick }}">{{ item.label }}</button></sc-for><sc-if value="{{ visible }}"><span>Shown</span></sc-if></x-dc>"#,
        Frontend::dc(),
    );

    assert!(code.contains("let { items = [], visible }"), "{code}");
    assert!(code.contains("{#each items as item, $index}"), "{code}");
    assert!(code.contains("<button onclick={item.onClick}>"), "{code}");
    assert!(code.contains("{item.label}"), "{code}");
    assert!(code.contains("{#if visible}"), "{code}");
    assert!(!code.contains("{{ item"), "{code}");
}

#[test]
fn svelte_adapter_emits_dc_template_class_bindings() {
    let pure = emit_svelte(
        r#"<x-dc><button class="{{ key.className }}">Pure</button></x-dc>"#,
        Frontend::dc(),
    );
    assert!(pure.contains("class={key.className}"), "{pure}");
    assert_eq!(pure.matches(" class=").count(), 1, "{pure}");
    assert!(!pure.contains("{{"), "{pure}");

    let mixed = emit_svelte(
        r#"<x-dc><button class="keycap {{ key.className }}">Mixed</button></x-dc>"#,
        Frontend::dc(),
    );
    assert!(
        mixed.contains(r#"class={`keycap ${key.className}`}"#),
        "{mixed}"
    );
    assert_eq!(mixed.matches(" class=").count(), 1, "{mixed}");
    assert!(!mixed.contains("{{"), "{mixed}");
}

#[test]
fn svelte_adapter_keeps_dc_component_hints_native_unless_imported() {
    let code = emit_svelte(
        r#"<x-dc><div data-htmlswap-component="nav-rail"><div data-htmlswap-slot="footer"><span>Footer</span></div></div></x-dc>"#,
        Frontend::dc(),
    );

    assert!(!code.contains("<Navrail"), "{code}");
    assert!(!code.contains("{@render footer"), "{code}");
    assert!(
        code.contains(r#"<div data-htmlswap-component="nav-rail">"#),
        "{code}"
    );
    assert!(code.contains("Footer"), "{code}");
}

#[test]
fn svelte_adapter_dedupes_tabs_role_attributes() {
    let code = emit_svelte(
        r#"<x-dc><div class="seg" role="tablist" data-htmlswap-component="tabs"><sc-for list="{{ tabs }}" as="tab"><button role="tab" aria-selected="{{ tab.selected }}">{{ tab.label }}</button></sc-for></div></x-dc>"#,
        Frontend::dc(),
    );

    assert!(!code.contains(r#"role="tablist" role="tablist""#), "{code}");
    assert!(!code.contains(r#"role="tab" role="tab""#), "{code}");
    assert_eq!(code.matches(r#"role="tablist""#).count(), 1, "{code}");
    assert_eq!(code.matches(r#"role="tab""#).count(), 1, "{code}");
    assert!(code.contains(r#"data-htmlswap-component="tabs""#), "{code}");
}

#[test]
fn svelte_adapter_dedupes_switch_role_attributes() {
    let code = emit_svelte(
        r#"<x-dc><button class="switch" role="switch" aria-checked="{{ y }}" data-htmlswap-component="switch"><span></span></button></x-dc>"#,
        Frontend::dc(),
    );

    assert!(!code.contains(r#"role="switch" role="switch""#), "{code}");
    assert_eq!(code.matches(r#"role="switch""#).count(), 1, "{code}");
    assert!(
        code.contains(r#"data-htmlswap-component="switch""#),
        "{code}"
    );
}

#[test]
fn svelte_adapter_does_not_extract_nested_loop_aliases_as_props() {
    let code = emit_svelte(
        r#"
        <x-dc>
            <sc-for list="{{ browse.maps }}" as="map">
                <section>
                    <sc-for list="{{ map.previewRows }}" as="previewRow">
                        <div>
                            <sc-for list="{{ previewRow.groups }}" as="group">
                                <span>{{ group.label }}</span>
                            </sc-for>
                        </div>
                    </sc-for>
                </section>
            </sc-for>
        </x-dc>
        "#,
        Frontend::dc(),
    );

    let props_line = code
        .lines()
        .find(|line| line.contains("$props"))
        .expect("Svelte output should declare props");
    assert!(props_line.contains("browse"), "{code}");
    assert!(!props_line.contains("map"), "{code}");
    assert!(!props_line.contains("previewRow"), "{code}");
    assert!(!props_line.contains("group"), "{code}");
}

#[test]
fn svelte_adapter_diagnoses_browser_only_static_styles() {
    let (code, cx) = adapt_svelte(
        r#"<div class="overlay" style="position: fixed; z-index: 300"></div>"#,
        Frontend::html(),
        CompileAssets::new(),
    );

    assert!(code.contains("position: fixed"), "{code}");
    assert!(code.contains("z-index: 300"), "{code}");
    let diagnostics = cx.diagnostics();
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == Severity::Warning
                && diagnostic.message.contains("position: fixed")
                && diagnostic
                    .message
                    .contains("flexbox-only authoring contract")
        }),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == Severity::Warning
                && diagnostic.message.contains("z-index: 300")
                && diagnostic
                    .message
                    .contains("flexbox-only authoring contract")
        }),
        "{diagnostics:?}"
    );
}

#[test]
fn svelte_adapter_keeps_source_owned_inputs_controlled_by_source() {
    let code = emit_svelte(
        r#"<x-dc><input data-htmlswap-state-owner="source" value="{{ name }}" onInput="{{ onName }}"></x-dc>"#,
        Frontend::dc(),
    );

    assert!(code.contains("let { name, onName }"), "{code}");
    assert!(
        code.contains("<input value={name} oninput={onName} />"),
        "{code}"
    );
    assert!(!code.contains("$state(name)"), "{code}");
    assert!(!code.contains("input_0 = event.currentTarget"), "{code}");
    assert!(!code.contains("data-htmlswap-state-owner"), "{code}");
}

#[test]
fn svelte_adapter_adds_keyboard_activation_for_clickable_noninteractive_elements() {
    let code = emit_svelte(
        r#"<div class="card" onclick="{{ open }}"><span>Open</span></div>"#,
        Frontend::html(),
    );

    assert!(
        code.contains("<div class=\"card\" role=\"button\" tabindex=\"0\""),
        "{code}"
    );
    assert!(code.contains("onclick={open}"), "{code}");
    assert!(code.contains("onkeydown={(event) =>"), "{code}");
    assert!(
        code.contains("event.key === 'Enter' || event.key === ' '"),
        "{code}"
    );
    assert!(
        code.contains("const handler = open; handler?.(event);"),
        "{code}"
    );
}

#[test]
fn svelte_adapter_does_not_add_keyboard_activation_to_native_buttons() {
    let code = emit_svelte(
        r#"<button onclick="{{ save }}">Save</button>"#,
        Frontend::html(),
    );

    assert!(code.contains("<button onclick={save}>"), "{code}");
    assert!(!code.contains("onkeydown="), "{code}");
    assert!(!code.contains("role=\"button\""), "{code}");
}
#[test]
fn svelte_adapter_treats_dc_templated_input_values_as_source_owned_by_default() {
    let code = emit_svelte(
        r#"<x-dc><input value="{{ gemQuery }}" onInput="{{ onGemSearch }}" placeholder="Filter gems"></x-dc>"#,
        Frontend::dc(),
    );

    assert!(code.contains("let { gemQuery, onGemSearch }"), "{code}");
    assert!(
        code.contains(
            "<input value={gemQuery} placeholder=\"Filter gems\" oninput={onGemSearch} />"
        ),
        "{code}"
    );
    assert!(!code.contains("$state(gemQuery)"), "{code}");
    assert!(!code.contains("input_0 = event.currentTarget"), "{code}");
}
#[test]
fn svelte_adapter_calls_template_handlers_after_target_state_updates() {
    let code = emit_svelte(
        r#"<input data-htmlswap-state="name" value="Ada" onInput="{{ onName }}">"#,
        Frontend::html(),
    );

    assert!(code.contains("let name = $state(\"Ada\");"), "{code}");
    assert!(
        code.contains(
            "oninput={(event) => { name = event.currentTarget?.value ?? ''; onName?.(event); }}"
        ),
        "{code}"
    );
}

#[test]
fn svelte_adapter_emits_real_dc_imports_as_components() {
    let code = emit_svelte(
        r#"<x-dc><dc-import name="GemCard" label="Project {{ item.name }}"><span>Child</span></dc-import><dc-import name="segmented-option"></dc-import></x-dc>"#,
        Frontend::dc(),
    );

    assert!(
        code.contains("import GemCard from \"./GemCard.svelte\";"),
        "{code}"
    );
    assert!(
        code.contains("import SegmentedOption from \"./SegmentedOption.svelte\";"),
        "{code}"
    );
    assert!(
        code.contains("<GemCard label={`Project ${item.name}`}>"),
        "{code}"
    );
    assert!(code.contains("<SegmentedOption>"), "{code}");
    assert!(code.contains("Child"), "{code}");
}
#[test]
fn svelte_adapter_hoists_pseudo_state_styles_to_css() {
    let code = emit_svelte(
        r#"<button class="card" style="color:red" style-hover="background: blue">Hover</button>"#,
        Frontend::html(),
    );

    assert!(code.contains("<style>"), "{code}");
    assert!(code.contains(".hs_0:hover"), "{code}");
    assert!(code.contains("background: #00f;"), "{code}");
    assert!(code.contains("class=\"card hs_0\""), "{code}");
    assert!(code.contains("style=\"color: red\""), "{code}");
}

#[test]
fn svelte_adapter_does_not_duplicate_external_hover_pseudo_selectors() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        r#".seg [role="tab"]:hover { background: blue; }"#,
    );
    let code = emit_svelte_with_assets(
        r#"<x-dc><div class="seg" role="tablist"><button role="tab">One</button></div></x-dc>"#,
        Frontend::dc(),
        assets,
    );

    assert!(!code.contains(":hover:hover"), "{code}");
    assert_eq!(
        style_rule_count(&code, r#".seg [role="tab"]:hover"#),
        1,
        "{code}"
    );
    assert!(!code.contains(r#".hs_0.seg [role="tab"]:hover"#), "{code}");
}

#[test]
fn svelte_adapter_scopes_external_descendant_variant_by_original_selector() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        r#".seg [role="tab"]:hover { color: red; }"#,
    );
    let code = emit_svelte_with_assets(
        r#"<div class="seg"><button role="tab">T</button></div>"#,
        Frontend::html(),
        assets,
    );

    assert_eq!(
        style_rule_count(&code, r#".seg [role="tab"]:hover"#),
        1,
        "{code}"
    );
    assert!(!code.contains(r#".hs_0.seg [role="tab"]:hover"#), "{code}");
    let button = output_line_containing(&code, r#"role="tab""#);
    assert!(!button.contains("hs_"), "{code}");
}

#[test]
fn svelte_adapter_scopes_external_tag_led_pseudo_element_by_original_selector() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        r#"input[type="range"]::-webkit-slider-thumb { width: 10px; }"#,
    );
    let code = emit_svelte_with_assets(r#"<input type="range">"#, Frontend::html(), assets);

    assert_eq!(
        style_rule_count(&code, r#"input[type="range"]::-webkit-slider-thumb"#),
        1,
        "{code}"
    );
    assert!(
        !code.contains(r#".hs_0 input[type="range"]::-webkit-slider-thumb"#),
        "{code}"
    );
    let input = output_line_containing(&code, "<input");
    assert!(!input.contains("hs_"), "{code}");
}

#[test]
fn svelte_adapter_scopes_external_simple_class_variant_by_original_selector() {
    let assets = CompileAssets::new()
        .with_stylesheet(Some("app.css".to_owned()), ".keycap:hover { color: red; }");
    let code = emit_svelte_with_assets(r#"<div class="keycap">K</div>"#, Frontend::html(), assets);

    assert_eq!(style_rule_count(&code, ".keycap:hover"), 1, "{code}");
    let element = output_line_containing(&code, r#"class="keycap""#);
    assert!(!element.contains("hs_"), "{code}");
}

#[test]
fn svelte_adapter_dedupes_external_complex_variant_rules_across_elements() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        r#".seg [role="tab"]:hover { color: red; }"#,
    );
    let code = emit_svelte_with_assets(
        r#"<div class="seg"><button role="tab">One</button><button role="tab">Two</button></div>"#,
        Frontend::html(),
        assets,
    );

    assert_eq!(
        style_rule_count(&code, r#".seg [role="tab"]:hover"#),
        1,
        "{code}"
    );
    assert!(!code.contains(r#".hs_0.seg [role="tab"]:hover"#), "{code}");
    assert!(!code.contains(r#".hs_1.seg [role="tab"]:hover"#), "{code}");
}

#[test]
fn svelte_adapter_emits_external_base_rule_once_and_keeps_author_inline_style() {
    let assets =
        CompileAssets::new().with_stylesheet(Some("app.css".to_owned()), ".card { color: red; }");
    let code = emit_svelte_with_assets(
        r#"<div class="card" style="gap: 12px">Hi</div>"#,
        Frontend::html(),
        assets,
    );

    assert!(code.contains("<style>"), "{code}");
    assert!(code.contains(".card {"), "{code}");
    assert!(code.contains("color: red;"), "{code}");
    let element = output_line_containing(&code, r#"<div class="card""#);
    assert!(element.contains(r#"style="gap: 12px""#), "{code}");
    assert!(!element.contains("color: red"), "{code}");
}

#[test]
fn svelte_adapter_keeps_author_override_inline_for_normal_stylesheet_rule() {
    let assets =
        CompileAssets::new().with_stylesheet(Some("app.css".to_owned()), ".card { color: red; }");
    let code = emit_svelte_with_assets(
        r#"<div class="card" style="color: blue">Hi</div>"#,
        Frontend::html(),
        assets,
    );

    assert!(code.contains(".card {"), "{code}");
    assert!(code.contains("color: red;"), "{code}");
    let element = output_line_containing(&code, r#"<div class="card""#);
    assert!(
        element.contains(r#"style="color: #00f""#) || element.contains(r#"style="color: blue""#),
        "{code}"
    );
}

#[test]
fn svelte_adapter_preserves_browser_cascade_for_stylesheet_important_override() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        ".card { color: red !important; }",
    );
    let code = emit_svelte_with_assets(
        r#"<div class="card" style="color: blue">Hi</div>"#,
        Frontend::html(),
        assets,
    );

    assert!(code.contains(".card {"), "{code}");
    assert!(code.contains("color: red !important;"), "{code}");
    let element = output_line_containing(&code, r#"<div class="card""#);
    assert!(
        element.contains(r#"style="color: #00f""#) || element.contains(r#"style="color: blue""#),
        "{code}"
    );
}

#[test]
fn svelte_adapter_dedupes_external_base_rules_across_elements() {
    let assets =
        CompileAssets::new().with_stylesheet(Some("app.css".to_owned()), ".card { color: red; }");
    let code = emit_svelte_with_assets(
        r#"<div class="card">One</div><div class="card">Two</div>"#,
        Frontend::html(),
        assets,
    );

    assert_eq!(code.matches(".card {").count(), 1, "{code}");
    assert_eq!(code.matches(r#"class="card""#).count(), 2, "{code}");
}

#[test]
fn svelte_adapter_preserves_non_stylesheet_inline_declarations() {
    let assets =
        CompileAssets::new().with_stylesheet(Some("app.css".to_owned()), ".card { color: red; }");
    let code = emit_svelte_with_assets(
        r#"<div class="card" style="box-sizing: border-box">Hi</div>"#,
        Frontend::html(),
        assets,
    );

    assert!(code.contains(".card {"), "{code}");
    let element = output_line_containing(&code, r#"<div class="card""#);
    assert!(
        element.contains(r#"style="box-sizing: border-box""#),
        "{code}"
    );
    assert!(!element.contains("color: red"), "{code}");
}

#[test]
fn svelte_adapter_diagnoses_browser_only_styles_from_external_stylesheet() {
    let assets = CompileAssets::new().with_stylesheet(
        Some("app.css".to_owned()),
        ".overlay { position: fixed; z-index: 10; }",
    );
    let (code, cx) = adapt_svelte(
        r#"<div class="overlay">Modal</div>"#,
        Frontend::html(),
        assets,
    );

    assert!(code.contains(".overlay {"), "{code}");
    assert!(code.contains("position: fixed;"), "{code}");
    assert!(code.contains("z-index: 10;"), "{code}");
    let element = output_line_containing(&code, r#"<div class="overlay""#);
    assert!(!element.contains("position: fixed"), "{code}");
    assert!(!element.contains("z-index: 10"), "{code}");
    let diagnostics = cx.diagnostics();
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == Severity::Warning
                && diagnostic.message.contains("position: fixed")
                && diagnostic
                    .message
                    .contains("flexbox-only authoring contract")
        }),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == Severity::Warning
                && diagnostic.message.contains("z-index: 10")
                && diagnostic
                    .message
                    .contains("flexbox-only authoring contract")
        }),
        "{diagnostics:?}"
    );
}
