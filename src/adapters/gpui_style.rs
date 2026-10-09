//! Printing a typed GPUI style plan as builder calls.
//!
//! [`crate::computed::gpui::plan`] decides what each computed style becomes
//! in GPUI; this module only spells those decisions as Rust. Lengths with
//! runtime parts read the window's rem size and viewport from locals the
//! render function declares when [`StylePrinter`] reports it needs them.

use crate::computed::gpui::{
    Absolute, Definite, Features, GpuiAlign, GpuiCursor, GpuiDisplay, GpuiDistribute, GpuiLength,
    GpuiOverflow, GpuiPosition, GpuiStyle, GpuiTextAlign, GpuiTracks, Radius,
};
use crate::computed::{FlexDirection, FlexWrap, FontFamily, FontStyle, Rgba, Sides};

/// The local holding the window's rem size.
pub(crate) const REM_SIZE_LOCAL: &str = "htmlswap_rem_size";
/// The local holding the window's viewport size.
pub(crate) const VIEWPORT_LOCAL: &str = "htmlswap_viewport";

/// Prints plans, recording which runtime values the printed code reads.
#[derive(Debug, Default)]
pub(crate) struct StylePrinter {
    /// The target's GPUI features, which decide what can be printed.
    pub(crate) features: Features,
    pub(crate) uses_rem_size: bool,
    pub(crate) uses_viewport: bool,
}

pub(crate) fn format_number(value: f32) -> String {
    let mut formatted = format!("{value:.3}");
    while formatted.contains('.') && formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.push('0');
    }
    if formatted == "-0.0" {
        formatted = "0.0".to_owned();
    }
    formatted
}

pub(crate) fn color(color: Rgba) -> String {
    if color.a == 255 {
        format!("gpui::rgb(0x{:02X}{:02X}{:02X})", color.r, color.g, color.b)
    } else {
        format!(
            "gpui::rgba(0x{:02X}{:02X}{:02X}{:02X})",
            color.r, color.g, color.b, color.a
        )
    }
}

impl StylePrinter {
    /// The `prelude` lines declaring the runtime values printed code reads.
    pub(crate) fn prelude(&self, window: &str) -> Vec<String> {
        let mut lines = Vec::new();
        if self.uses_rem_size {
            lines.push(format!("let {REM_SIZE_LOCAL} = {window}.rem_size();"));
        }
        if self.uses_viewport {
            lines.push(format!("let {VIEWPORT_LOCAL} = {window}.viewport_size();"));
        }
        lines
    }

    /// An expression of type `gpui::Pixels`.
    pub(crate) fn pixels(&mut self, value: Absolute) -> String {
        let length = value.length();
        if let Some(px) = length.as_px() {
            return format!("gpui::px({})", format_number(px));
        }
        let mut terms = Vec::new();
        if length.px != 0.0 {
            terms.push(format!("gpui::px({})", format_number(length.px)));
        }
        let mut scaled = |base: String, factor: f32| {
            if factor == 0.0 {
                return;
            }
            if (factor - 1.0).abs() < f32::EPSILON {
                terms.push(base);
            } else {
                terms.push(format!("{base} * {}_f32", format_number(factor)));
            }
        };
        if length.rem != 0.0 {
            self.uses_rem_size = true;
        }
        scaled(REM_SIZE_LOCAL.to_owned(), length.rem);
        let viewport = length.uses_viewport();
        let width = format!("{VIEWPORT_LOCAL}.width");
        let height = format!("{VIEWPORT_LOCAL}.height");
        scaled(width.clone(), length.vw);
        scaled(height.clone(), length.vh);
        scaled(
            format!("(if {width} < {height} {{ {width} }} else {{ {height} }})"),
            length.vmin,
        );
        scaled(
            format!("(if {width} > {height} {{ {width} }} else {{ {height} }})"),
            length.vmax,
        );
        if viewport {
            self.uses_viewport = true;
        }
        terms.join(" + ")
    }

    /// An expression convertible into `gpui::AbsoluteLength`.
    pub(crate) fn absolute(&mut self, value: Absolute) -> String {
        if value.as_px().is_some() {
            return self.pixels(value);
        }
        if let Some(rems) = value.as_rems() {
            return format!("gpui::rems({})", format_number(rems));
        }
        self.pixels(value)
    }

    /// An expression convertible into `gpui::DefiniteLength`.
    pub(crate) fn definite(&mut self, value: Definite) -> String {
        match value {
            Definite::Absolute(value) => self.absolute(value),
            Definite::Fraction(fraction) => format!("gpui::relative({})", format_number(fraction)),
        }
    }

    /// An expression convertible into `gpui::Length`.
    pub(crate) fn length(&mut self, value: GpuiLength) -> String {
        match value {
            GpuiLength::Auto => "gpui::auto()".to_owned(),
            GpuiLength::Definite(value) => self.definite(value),
        }
    }

    fn edges<T: Copy + PartialEq>(
        &mut self,
        methods: &mut Vec<String>,
        sides: Sides<Option<T>>,
        all: &str,
        each: [&str; 4],
        print: impl Fn(&mut Self, T) -> String,
    ) {
        let values = [sides.top, sides.right, sides.bottom, sides.left];
        if let [Some(first), ..] = values
            && values.iter().all(|value| *value == Some(first))
        {
            let value = print(self, first);
            methods.push(format!(".{all}({value})"));
            return;
        }
        for (value, method) in values.into_iter().zip(each) {
            if let Some(value) = value {
                let value = print(self, value);
                methods.push(format!(".{method}({value})"));
            }
        }
    }

    fn size(&mut self, methods: &mut Vec<String>, value: Option<GpuiLength>, method: &str) {
        let Some(value) = value else { return };
        if value == GpuiLength::Definite(Definite::Fraction(1.0)) && matches!(method, "w" | "h") {
            methods.push(format!(".{method}_full()"));
        } else {
            let value = self.length(value);
            methods.push(format!(".{method}({value})"));
        }
    }

    /// The builder calls that apply `style`, in a stable order.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn methods(&mut self, style: &GpuiStyle) -> Vec<String> {
        let mut methods = Vec::new();
        if let Some(display) = style.display {
            methods.push(
                match display {
                    GpuiDisplay::Hidden => ".hidden()",
                    GpuiDisplay::Block => ".block()",
                    GpuiDisplay::Flex => ".flex()",
                    GpuiDisplay::Grid => ".grid()",
                }
                .to_owned(),
            );
        }
        if let Some(position) = style.position {
            methods.push(
                match position {
                    GpuiPosition::Relative => ".relative()",
                    GpuiPosition::Absolute => ".absolute()",
                }
                .to_owned(),
            );
        }
        self.edges(
            &mut methods,
            style.inset,
            "inset",
            ["top", "right", "bottom", "left"],
            Self::length,
        );
        self.size(&mut methods, style.width, "w");
        self.size(&mut methods, style.height, "h");
        self.size(&mut methods, style.min_width, "min_w");
        self.size(&mut methods, style.min_height, "min_h");
        self.size(&mut methods, style.max_width, "max_w");
        self.size(&mut methods, style.max_height, "max_h");
        if let Some(ratio) = style.aspect_ratio {
            methods.push(format!(
                ".map(|mut this| {{ this.style().aspect_ratio = Some({}); this }})",
                format_number(ratio)
            ));
        }
        self.edges(
            &mut methods,
            style.margin,
            "m",
            ["mt", "mr", "mb", "ml"],
            Self::length,
        );
        self.edges(
            &mut methods,
            style.padding,
            "p",
            ["pt", "pr", "pb", "pl"],
            Self::definite,
        );
        self.flex(&mut methods, style);
        self.alignment(&mut methods, style);
        match (style.column_gap, style.row_gap) {
            (Some(column), Some(row)) if column == row => {
                let value = self.definite(column);
                methods.push(format!(".gap({value})"));
            }
            (column, row) => {
                if let Some(column) = column {
                    let value = self.definite(column);
                    methods.push(format!(".gap_x({value})"));
                }
                if let Some(row) = row {
                    let value = self.definite(row);
                    methods.push(format!(".gap_y({value})"));
                }
            }
        }
        for (tracks, method) in [
            (&style.grid_template_columns, "grid_cols"),
            (&style.grid_template_rows, "grid_rows"),
        ] {
            if let Some(GpuiTracks::Count(count)) = tracks {
                methods.push(format!(".{method}({count})"));
            }
        }
        self.overflow(&mut methods, style);
        if let Some(visible) = style.visible {
            methods.push(
                if visible {
                    ".visible()"
                } else {
                    ".invisible()"
                }
                .to_owned(),
            );
        }
        if let Some(opacity) = style.opacity {
            methods.push(format!(".opacity({})", format_number(opacity)));
        }
        if let Some(cursor) = style.cursor {
            methods.push(format!(".{}()", cursor_method(cursor)));
        }
        if let Some(background) = style.background {
            methods.push(format!(".bg({})", color(background)));
        }
        self.edges(
            &mut methods,
            style.border_widths,
            "border",
            ["border_t", "border_r", "border_b", "border_l"],
            Self::absolute,
        );
        if style.border_dashed {
            methods.push(".border_dashed()".to_owned());
        }
        if let Some(border) = style.border_color {
            methods.push(format!(".border_color({})", color(border)));
        }
        self.radii(&mut methods, style);
        if let Some(shadows) = &style.box_shadow {
            if shadows.is_empty() {
                methods.push(".shadow_none()".to_owned());
            } else {
                let shadows = shadows
                    .iter()
                    .map(|shadow| {
                        let inset = if self.features.inset_shadows {
                            format!(", inset: {}", shadow.inset)
                        } else {
                            String::new()
                        };
                        format!(
                            "gpui::BoxShadow {{ color: {}.into(), offset: gpui::point({}, {}), blur_radius: {}, spread_radius: {}{inset} }}",
                            color(shadow.color),
                            self.pixels(shadow.x),
                            self.pixels(shadow.y),
                            self.pixels(shadow.blur),
                            self.pixels(shadow.spread),
                        )
                    })
                    .collect::<Vec<_>>();
                methods.push(format!(".shadow(std::vec![{}])", shadows.join(", ")));
            }
        }
        self.text(&mut methods, style);
        methods
    }

    fn flex(&mut self, methods: &mut Vec<String>, style: &GpuiStyle) {
        if let Some(direction) = style.flex_direction {
            methods.push(
                match direction {
                    FlexDirection::Row => ".flex_row()",
                    FlexDirection::RowReverse => ".flex_row_reverse()",
                    FlexDirection::Column => ".flex_col()",
                    FlexDirection::ColumnReverse => ".flex_col_reverse()",
                }
                .to_owned(),
            );
        }
        if let Some(wrap) = style.flex_wrap {
            methods.push(
                match wrap {
                    FlexWrap::NoWrap => ".flex_nowrap()",
                    FlexWrap::Wrap => ".flex_wrap()",
                    FlexWrap::WrapReverse => ".flex_wrap_reverse()",
                }
                .to_owned(),
            );
        }
        let zero_basis = Some(GpuiLength::Definite(Definite::Fraction(0.0)));
        let auto_basis = Some(GpuiLength::Auto);
        let mut basis = style.flex_basis;
        match (style.flex_grow, style.flex_shrink) {
            (Some(grow), Some(shrink)) if grow == 1.0 && shrink == 1.0 && basis == zero_basis => {
                methods.push(".flex_1()".to_owned());
                basis = None;
            }
            (Some(grow), Some(shrink)) if grow == 1.0 && shrink == 1.0 && basis == auto_basis => {
                methods.push(".flex_auto()".to_owned());
                basis = None;
            }
            (Some(grow), Some(shrink)) if grow == 0.0 && shrink == 1.0 && basis == auto_basis => {
                methods.push(".flex_initial()".to_owned());
                basis = None;
            }
            (Some(grow), Some(shrink)) if grow == 0.0 && shrink == 0.0 => {
                methods.push(".flex_none()".to_owned());
            }
            (grow, shrink) => {
                if let Some(grow) = grow {
                    methods.push(if grow == 1.0 {
                        ".flex_grow_1()".to_owned()
                    } else {
                        format!(
                            ".map(|mut this| {{ this.style().flex_grow = Some({}); this }})",
                            format_number(grow)
                        )
                    });
                }
                if let Some(shrink) = shrink {
                    methods.push(match shrink {
                        0.0 => ".flex_shrink_0()".to_owned(),
                        1.0 => ".flex_shrink_1()".to_owned(),
                        _ => format!(
                            ".map(|mut this| {{ this.style().flex_shrink = Some({}); this }})",
                            format_number(shrink)
                        ),
                    });
                }
            }
        }
        if let Some(basis) = basis {
            let value = self.length(basis);
            methods.push(format!(".flex_basis({value})"));
        }
    }

    fn alignment(&mut self, methods: &mut Vec<String>, style: &GpuiStyle) {
        // Stretch is how GPUI lays out items that set no alignment.
        if let Some(align) = style.align_items {
            methods.extend(
                match align {
                    GpuiAlign::Start | GpuiAlign::FlexStart => Some(".items_start()"),
                    GpuiAlign::End | GpuiAlign::FlexEnd => Some(".items_end()"),
                    GpuiAlign::Center => Some(".items_center()"),
                    GpuiAlign::Baseline => Some(".items_baseline()"),
                    GpuiAlign::Stretch => None,
                }
                .map(str::to_owned),
            );
        }
        if let Some(align) = style.align_self
            && align != GpuiAlign::Stretch
        {
            methods.push(format!(
                ".map(|mut this| {{ this.style().align_self = Some(gpui::AlignSelf::{}); this }})",
                align_name(align)
            ));
        }
        if let Some(content) = style.align_content {
            methods.push(
                match content {
                    GpuiDistribute::Start | GpuiDistribute::FlexStart => ".content_start()",
                    GpuiDistribute::End | GpuiDistribute::FlexEnd => ".content_end()",
                    GpuiDistribute::Center => ".content_center()",
                    GpuiDistribute::Stretch => ".content_stretch()",
                    GpuiDistribute::SpaceBetween => ".content_between()",
                    GpuiDistribute::SpaceAround => ".content_around()",
                    GpuiDistribute::SpaceEvenly => ".content_evenly()",
                }
                .to_owned(),
            );
        }
        if let Some(justify) = style.justify_content {
            methods.extend(
                match justify {
                    GpuiDistribute::Start | GpuiDistribute::FlexStart => Some(".justify_start()"),
                    GpuiDistribute::End | GpuiDistribute::FlexEnd => Some(".justify_end()"),
                    GpuiDistribute::Center => Some(".justify_center()"),
                    GpuiDistribute::SpaceBetween => Some(".justify_between()"),
                    GpuiDistribute::SpaceAround => Some(".justify_around()"),
                    GpuiDistribute::SpaceEvenly => Some(".justify_evenly()"),
                    GpuiDistribute::Stretch => None,
                }
                .map(str::to_owned),
            );
        }
    }

    fn overflow(&mut self, methods: &mut Vec<String>, style: &GpuiStyle) {
        let method = |overflow: GpuiOverflow, axis: &str| match overflow {
            GpuiOverflow::Visible => None,
            GpuiOverflow::Hidden | GpuiOverflow::Clip => Some(format!(".overflow{axis}_hidden()")),
            GpuiOverflow::Scroll => Some(format!(".overflow{axis}_scroll()")),
        };
        match (style.overflow_x, style.overflow_y) {
            (Some(x), Some(y)) if x == y => methods.extend(method(x, "")),
            (x, y) => {
                methods.extend(x.and_then(|x| method(x, "_x")));
                methods.extend(y.and_then(|y| method(y, "_y")));
            }
        }
    }

    fn radii(&mut self, methods: &mut Vec<String>, style: &GpuiStyle) {
        let corners = [
            style.corner_radii.top_left,
            style.corner_radii.top_right,
            style.corner_radii.bottom_right,
            style.corner_radii.bottom_left,
        ];
        let print = |printer: &mut Self, radius: Radius| match radius {
            Radius::Length(length) => printer.absolute(length),
            // GPUI clamps a radius to half the shorter side.
            Radius::Full => "gpui::px(9999.0)".to_owned(),
        };
        if let [Some(first), ..] = corners
            && corners.iter().all(|corner| *corner == Some(first))
        {
            if first == Radius::Full {
                methods.push(".rounded_full()".to_owned());
            } else {
                let value = print(self, first);
                methods.push(format!(".rounded({value})"));
            }
            return;
        }
        for (corner, method) in
            corners
                .into_iter()
                .zip(["rounded_tl", "rounded_tr", "rounded_br", "rounded_bl"])
        {
            if let Some(corner) = corner {
                let value = print(self, corner);
                methods.push(format!(".{method}({value})"));
            }
        }
    }

    fn text(&mut self, methods: &mut Vec<String>, style: &GpuiStyle) {
        if let Some(text) = style.text_color {
            methods.push(format!(".text_color({})", color(text)));
        }
        if let Some(families) = &style.font_family
            && let Some(first) = families.first()
        {
            let family = match first {
                FontFamily::Named(name) => name.as_str(),
                _ => ".SystemUIFont",
            };
            methods.push(format!(".font_family({family:?})"));
        }
        if let Some(size) = style.font_size {
            let value = self.absolute(size);
            methods.push(format!(".text_size({value})"));
        }
        if let Some(weight) = style.font_weight {
            methods.push(format!(".font_weight({})", font_weight(weight)));
        }
        if let Some(font_style) = style.font_style {
            methods.push(
                match font_style {
                    FontStyle::Normal => ".not_italic()",
                    FontStyle::Italic | FontStyle::Oblique => ".italic()",
                }
                .to_owned(),
            );
        }
        if let Some(height) = style.line_height {
            let value = self.definite(height);
            methods.push(format!(".line_height({value})"));
        }
        if let Some(align) = style.text_align {
            methods.push(
                match align {
                    GpuiTextAlign::Left => ".text_left()",
                    GpuiTextAlign::Center => ".text_center()",
                    GpuiTextAlign::Right => ".text_right()",
                }
                .to_owned(),
            );
        }
        if let Some(no_wrap) = style.no_wrap {
            methods.push(
                if no_wrap {
                    ".whitespace_nowrap()"
                } else {
                    ".whitespace_normal()"
                }
                .to_owned(),
            );
        }
        if style.text_ellipsis {
            methods.push(".text_ellipsis()".to_owned());
        }
        if let Some(Some(lines)) = style.line_clamp {
            methods.push(format!(".line_clamp({lines})"));
        }
        if let Some(decorations) = style.decorations {
            match (decorations.underline, decorations.strikethrough) {
                (None, None) => methods.push(".text_decoration_none()".to_owned()),
                (underline, strikethrough) => {
                    if let Some(underline) = underline {
                        methods.push(".underline()".to_owned());
                        if underline.wavy {
                            methods.push(".text_decoration_wavy()".to_owned());
                        }
                        if let Some(decoration) = underline.color {
                            methods.push(format!(".text_decoration_color({})", color(decoration)));
                        }
                        let method = match underline.thickness.as_px() {
                            Some(0.0) => Some(".text_decoration_0()"),
                            Some(2.0) => Some(".text_decoration_2()"),
                            Some(4.0) => Some(".text_decoration_4()"),
                            Some(8.0) => Some(".text_decoration_8()"),
                            _ => None,
                        };
                        methods.extend(method.map(str::to_owned));
                    }
                    if strikethrough.is_some() {
                        methods.push(".line_through()".to_owned());
                    }
                }
            }
        }
    }
}

const fn align_name(align: GpuiAlign) -> &'static str {
    match align {
        GpuiAlign::Start => "Start",
        GpuiAlign::End => "End",
        GpuiAlign::FlexStart => "FlexStart",
        GpuiAlign::FlexEnd => "FlexEnd",
        GpuiAlign::Center => "Center",
        GpuiAlign::Baseline => "Baseline",
        GpuiAlign::Stretch => "Stretch",
    }
}

const fn cursor_method(cursor: GpuiCursor) -> &'static str {
    match cursor {
        GpuiCursor::Arrow => "cursor_default",
        GpuiCursor::PointingHand => "cursor_pointer",
        GpuiCursor::IBeam => "cursor_text",
        GpuiCursor::IBeamVertical => "cursor_vertical_text",
        GpuiCursor::Crosshair => "cursor_crosshair",
        GpuiCursor::OpenHand => "cursor_grab",
        GpuiCursor::ClosedHand => "cursor_grabbing",
        GpuiCursor::NotAllowed => "cursor_not_allowed",
        GpuiCursor::DragLink => "cursor_alias",
        GpuiCursor::DragCopy => "cursor_copy",
        GpuiCursor::ResizeLeftRight => "cursor_ew_resize",
        GpuiCursor::ResizeUpDown => "cursor_ns_resize",
        GpuiCursor::ResizeUpRightDownLeft => "cursor_nesw_resize",
        GpuiCursor::ResizeUpLeftDownRight => "cursor_nwse_resize",
        GpuiCursor::ResizeColumn => "cursor_col_resize",
        GpuiCursor::ResizeRow => "cursor_row_resize",
    }
}

fn font_weight(weight: f32) -> String {
    const NAMES: [&str; 9] = [
        "THIN",
        "EXTRA_LIGHT",
        "LIGHT",
        "NORMAL",
        "MEDIUM",
        "SEMIBOLD",
        "BOLD",
        "EXTRA_BOLD",
        "BLACK",
    ];
    let hundreds = weight / 100.0;
    if hundreds.fract() == 0.0 && (1.0..=9.0).contains(&hundreds) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        return format!("gpui::FontWeight::{}", NAMES[hundreds as usize - 1]);
    }
    format!("gpui::FontWeight({})", format_number(weight))
}
