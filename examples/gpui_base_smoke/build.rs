use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

use htmlswap::{
    Adapter, AdapterContext, CompileAssets, Compiler, Frontend, GpuiAdapter, GpuiAdapterOptions,
    RustFormatOptions, ThemeEmission,
};

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=src/view.dc.html");

    let source = fs::read_to_string("src/view.dc.html")?;
    let compiled = Compiler::new()
        .with_frontend(Frontend::dc())
        .compile_fragment_named(Some("src/view.dc.html".to_owned()), source, &CompileAssets::new());

    if !compiled.diagnostics.is_empty() {
        return Err(format!("htmlswap compile diagnostics: {:?}", compiled.diagnostics).into());
    }

    let mut adapter_context = AdapterContext::new();
    let output = GpuiAdapter::new(GpuiAdapterOptions {
        component_name: "BaseSmokeView".into(),
        include_dependency_header: false,
        include_imports: true,
        emit_source_comments: false,
        theme: ThemeEmission::Literal,
        format: RustFormatOptions::cargo_fmt_tabs(),
        ..GpuiAdapterOptions::default()
    })
    .adapt(&compiled.value, &mut adapter_context)?;

    if !adapter_context.diagnostics().is_empty() {
        return Err(format!(
            "htmlswap adapter diagnostics: {:?}",
            adapter_context.diagnostics()
        )
        .into());
    }

    // The view exercises these emission paths; compiling them is the point.
    for expected in [
        ".group_hover(",
        ".group_active(",
        ".in_focus(",
        "htmlswap_rem_size",
        "htmlswap_viewport",
        "gpui::BoxShadow",
    ] {
        if !output.code().contains(expected) {
            return Err(format!("generated view lacks `{expected}`").into());
        }
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    fs::write(out_dir.join("base_smoke_view.rs"), output.code())?;

    Ok(())
}
