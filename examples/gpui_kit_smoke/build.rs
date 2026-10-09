//! Generates both smoke views for the GPUI Kit target: the base view with
//! the plain GPUI adapter and the component view with GPUI Components.

use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

use htmlswap::{
    Adapter, AdapterContext, CompileAssets, Compiler, Frontend, GpuiAdapter, GpuiAdapterOptions,
    GpuiComponentsAdapter, GpuiComponentsAdapterOptions, GpuiTarget, RustFormatOptions,
    ThemeEmission,
};

fn options(component_name: &str, theme: ThemeEmission) -> GpuiAdapterOptions {
    GpuiAdapterOptions {
        component_name: component_name.into(),
        target: GpuiTarget::Kit,
        include_dependency_header: false,
        include_imports: true,
        emit_source_comments: false,
        theme,
        format: RustFormatOptions::cargo_fmt_tabs(),
        ..GpuiAdapterOptions::default()
    }
}

fn generate(
    source_path: &str,
    output_name: &str,
    adapt: impl FnOnce(
        &htmlswap::CompiledFragment,
        &mut AdapterContext,
    ) -> Result<String, htmlswap::AdapterError>,
) -> Result<String, Box<dyn Error>> {
    println!("cargo:rerun-if-changed={source_path}");
    let source = fs::read_to_string(source_path)?;
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment_named(Some(source_path.to_owned()), source, &CompileAssets::new());
    if !compiled.diagnostics.is_empty() {
        return Err(format!("htmlswap compile diagnostics: {:?}", compiled.diagnostics).into());
    }
    let mut adapter_context = AdapterContext::new();
    let code = adapt(&compiled.value, &mut adapter_context)?;
    if !adapter_context.diagnostics().is_empty() {
        return Err(format!(
            "htmlswap adapter diagnostics: {:?}",
            adapter_context.diagnostics()
        )
        .into());
    }
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    fs::write(out_dir.join(output_name), &code)?;
    Ok(code)
}

fn main() -> Result<(), Box<dyn Error>> {
    let base = generate("src/base.dc.html", "base_view.rs", |fragment, cx| {
        GpuiAdapter::new(options("BaseView", ThemeEmission::Literal))
            .adapt(fragment, cx)
            .map(|output| output.code().to_owned())
    })?;
    generate("src/components.dc.html", "components_view.rs", |fragment, cx| {
        let mut component_options = GpuiComponentsAdapterOptions::components();
        component_options.gpui = options("ComponentsView", ThemeEmission::PreferTheme);
        GpuiComponentsAdapter::new(component_options)
            .adapt(fragment, cx)
            .map(|output| output.code().to_owned())
    })?;
    // The base view exercises these emission paths; compiling them is the
    // point.
    for expected in [
        ".group_hover(",
        ".group_active(",
        ".in_focus(",
        "htmlswap_rem_size",
        "htmlswap_viewport",
        "inset: false",
        "gpui_base::motion::transition(",
        "HtmlswapMotionColor(",
        ".on_hover(",
    ] {
        if !base.contains(expected) {
            return Err(format!("generated base view lacks `{expected}`").into());
        }
    }
    Ok(())
}
