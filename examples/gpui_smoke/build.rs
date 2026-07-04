use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

use htmlswap::{
    Adapter, AdapterContext, CompileAssets, Compiler, Frontend, GpuiAdapterOptions,
    GpuiComponentsAdapter, GpuiComponentsAdapterOptions, RustFormatOptions, ThemeEmission,
};

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=src/view.dc.html");

    let source = fs::read_to_string("src/view.dc.html")?;
    let compiler = Compiler::new().with_frontend(Frontend::dc());
    let compiled =
        compiler.compile_fragment_named(Some("src/view.dc.html".to_owned()), source, &CompileAssets::new());

    if !compiled.diagnostics.is_empty() {
        return Err(format!("htmlswap compile diagnostics: {:?}", compiled.diagnostics).into());
    }

    let mut adapter_context = AdapterContext::new();
    let mut options = GpuiComponentsAdapterOptions::components();
    options.gpui = GpuiAdapterOptions {
        component_name: "SmokeView".into(),
        include_dependency_header: false,
        include_imports: true,
        emit_source_comments: false,
        theme: ThemeEmission::PreferTheme,
        format: RustFormatOptions::cargo_fmt_tabs(),
        ..GpuiAdapterOptions::default()
    };

    let output = GpuiComponentsAdapter::new(options).adapt(&compiled.value, &mut adapter_context)?;
    if !adapter_context.diagnostics().is_empty() {
        return Err(format!(
            "htmlswap adapter diagnostics: {:?}",
            adapter_context.diagnostics()
        )
        .into());
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    fs::write(out_dir.join("smoke_view.rs"), output.code())?;

    Ok(())
}
