use include_dir::{Dir, include_dir};

static DC_MATERIAL_SYMBOLS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets/icons/dc");

#[must_use]
pub(crate) fn material_symbol_svg_files() -> Vec<(&'static str, &'static str)> {
    DC_MATERIAL_SYMBOLS
        .files()
        .filter_map(|file| {
            let name = file.path().file_name()?.to_str()?;
            let contents = file.contents_utf8()?;
            Some((name, contents))
        })
        .collect()
}
