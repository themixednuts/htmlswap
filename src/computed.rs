//! Computed values: the context-dependent part of CSS that a renderer resolves
//! while walking the element tree.
//!
//! Compilation keeps declarations as authored, because some values depend on
//! where they are used: `var()` on the custom properties in scope,
//! `light-dark()` and system colors on the element's used color scheme, and
//! `currentColor` on its `color`. [`ComputedScope`] carries that inherited
//! state from parent to child, and [`ComputedScope::resolve`] turns a
//! declaration into one that only needs ordinary parsing.
//!
//! Colors are parsed by lightningcss, which supports every CSS Color 4 and 5
//! syntax and maps out-of-gamut colors into sRGB as the specification
//! describes.

use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use compact_str::CompactString;
use lightningcss::traits::Parse;
use lightningcss::values::color::{CssColor, RGBA};

use crate::style::{StyleDeclaration, StyleProperty};

/// A color scheme, as used by `color-scheme`, `light-dark()`, system colors
/// and `prefers-color-scheme`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ColorScheme {
    #[default]
    Light,
    Dark,
}

/// An sRGB color with 8-bit channels.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const BLACK: Self = Self::opaque(0x000000);
    pub const WHITE: Self = Self::opaque(0xffffff);

    #[must_use]
    pub const fn opaque(rgb: u32) -> Self {
        Self {
            r: (rgb >> 16) as u8,
            g: (rgb >> 8) as u8,
            b: rgb as u8,
            a: 255,
        }
    }

    /// `0xRRGGBBAA`.
    #[must_use]
    pub const fn to_u32(self) -> u32 {
        ((self.r as u32) << 24) | ((self.g as u32) << 16) | ((self.b as u32) << 8) | self.a as u32
    }

    fn to_css(self) -> String {
        format!("#{:08x}", self.to_u32())
    }
}

/// What context-dependent colors resolve against.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ColorContext {
    pub scheme: ColorScheme,
    pub current_color: Rgba,
}

impl Default for ColorContext {
    fn default() -> Self {
        Self {
            scheme: ColorScheme::Light,
            current_color: Rgba::BLACK,
        }
    }
}

/// Parse any CSS color in a context: every CSS Color 4/5 syntax, including
/// `color-mix()`, `light-dark()`, `currentColor` and system colors.
#[must_use]
pub fn parse_color(value: &str, context: &ColorContext) -> Option<Rgba> {
    let value = resolve_contextual(value.trim(), context, true);
    let color = CssColor::parse_string(&value).ok()?;
    css_color(&color, context)
}

fn css_color(color: &CssColor, context: &ColorContext) -> Option<Rgba> {
    match color {
        CssColor::CurrentColor => Some(context.current_color),
        CssColor::LightDark(light, dark) => css_color(
            if context.scheme == ColorScheme::Dark {
                dark
            } else {
                light
            },
            context,
        ),
        CssColor::System(_) => None,
        color => {
            let rgba = RGBA::try_from(color).ok()?;
            Some(Rgba {
                r: rgba.red,
                g: rgba.green,
                b: rgba.blue,
                a: rgba.alpha,
            })
        }
    }
}

/// A system color (CSS Color 4 §6.2), including the deprecated ones, which
/// map to their modern equivalents.
#[must_use]
pub fn system_color(name: &str, scheme: ColorScheme) -> Option<Rgba> {
    let name = name.to_ascii_lowercase();
    let canonical = match name.as_str() {
        "activeborder" | "inactiveborder" | "threeddarkshadow" | "threedhighlight"
        | "threedlightshadow" | "threedshadow" | "windowframe" => "buttonborder",
        "buttonhighlight" | "buttonshadow" | "threedface" => "buttonface",
        "activecaption" | "appworkspace" | "background" | "inactivecaption" | "infobackground"
        | "menu" | "scrollbar" | "window" => "canvas",
        "captiontext" | "inactivecaptiontext" | "infotext" | "menutext" | "windowtext" => {
            "canvastext"
        }
        name => name,
    };
    let dark = scheme == ColorScheme::Dark;
    let rgb = match canonical {
        "canvas" => {
            if dark {
                0x121212
            } else {
                0xffffff
            }
        }
        "canvastext" | "fieldtext" | "buttontext" => {
            if dark {
                0xffffff
            } else {
                0x000000
            }
        }
        "linktext" => {
            if dark {
                0x9e9eff
            } else {
                0x0000ee
            }
        }
        "visitedtext" => {
            if dark {
                0xd0adf0
            } else {
                0x551a8b
            }
        }
        "activetext" => {
            if dark {
                0xff9e9e
            } else {
                0xff0000
            }
        }
        "buttonface" => {
            if dark {
                0x6b6b6b
            } else {
                0xefefef
            }
        }
        "buttonborder" => {
            if dark {
                0x6b6b6b
            } else {
                0x767676
            }
        }
        "field" => {
            if dark {
                0x3b3b3b
            } else {
                0xffffff
            }
        }
        "highlight" => {
            if dark {
                0x264f78
            } else {
                0xb5d5ff
            }
        }
        "highlighttext" => {
            if dark {
                0xffffff
            } else {
                0x000000
            }
        }
        "selecteditem" | "accentcolor" => {
            if dark {
                0x3b8eea
            } else {
                0x0075ff
            }
        }
        "selecteditemtext" | "accentcolortext" => 0xffffff,
        "mark" => 0xffff00,
        "marktext" => 0x000000,
        "graytext" => {
            if dark {
                0x8e8e8e
            } else {
                0x808080
            }
        }
        _ => return None,
    };
    Some(Rgba::opaque(rgb))
}

/// The used color scheme of an element whose computed `color-scheme` is
/// `value`: the preferred scheme if the element supports it, else the first
/// it supports. `normal` (or nothing recognized) means light only.
#[must_use]
pub fn used_color_scheme(value: &str, preferred: ColorScheme) -> ColorScheme {
    let mut light = false;
    let mut dark = false;
    for keyword in value.split_whitespace() {
        if keyword.eq_ignore_ascii_case("light") {
            light = true;
        } else if keyword.eq_ignore_ascii_case("dark") {
            dark = true;
        }
    }
    match (light, dark) {
        (true, true) => preferred,
        (false, true) => {
            // `dark` alone still allows the user agent to force light only with
            // `only`; without it, a dark-only element is dark.
            ColorScheme::Dark
        }
        _ => ColorScheme::Light,
    }
}

/// Inherited custom properties, by name, with `var()` already substituted.
/// Cloning shares the map.
#[derive(Clone, Debug, Default)]
pub struct CustomProperties {
    values: Arc<HashMap<CompactString, CompactString>>,
    fingerprint: u64,
}

impl CustomProperties {
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(CompactString::as_str)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// A hash of the contents, for caches keyed by the inherited state.
    #[must_use]
    pub const fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    /// The custom properties of an element: inherited ones, overridden by its
    /// own declarations (in cascade order, `!important` last). References are
    /// resolved now, as computed values; a property in a reference cycle, or
    /// one whose `var()` cannot be resolved, is invalid and so not set.
    #[must_use]
    pub fn cascade<'a>(
        &self,
        declarations: impl IntoIterator<Item = &'a StyleDeclaration>,
    ) -> Self {
        let mut own: Vec<(&str, &str)> = Vec::new();
        let mut important: Vec<(&str, &str)> = Vec::new();
        for declaration in declarations {
            if let StyleProperty::Custom(name) = &declaration.property {
                let entry = (name.as_str(), declaration.value.as_str());
                if declaration.important {
                    important.push(entry);
                } else {
                    own.push(entry);
                }
            }
        }
        if own.is_empty() && important.is_empty() {
            return self.clone();
        }
        let mut raw: HashMap<&str, &str> = HashMap::with_capacity(own.len() + important.len());
        for (name, value) in own.into_iter().chain(important) {
            raw.insert(name, value);
        }
        let mut values = (*self.values).clone();
        let mut state: HashMap<&str, Option<Option<String>>> = HashMap::new();
        let names = raw.keys().copied().collect::<Vec<_>>();
        for name in names {
            match resolve_custom(name, &raw, &self.values, &mut state) {
                Some(value) => {
                    values.insert(name.into(), value.into());
                }
                None => {
                    values.remove(name);
                }
            }
        }
        let fingerprint = fingerprint(&values);
        Self {
            values: Arc::new(values),
            fingerprint,
        }
    }
}

fn fingerprint(values: &HashMap<CompactString, CompactString>) -> u64 {
    // Order-independent: combine per-entry hashes.
    values
        .iter()
        .map(|entry| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            entry.hash(&mut hasher);
            hasher.finish()
        })
        .fold(values.len() as u64, u64::wrapping_add)
}

/// Resolve one of an element's own custom properties. `state` marks names
/// being resolved (`None`) to detect cycles, and memoizes results.
fn resolve_custom<'a>(
    name: &'a str,
    raw: &HashMap<&'a str, &'a str>,
    inherited: &HashMap<CompactString, CompactString>,
    state: &mut HashMap<&'a str, Option<Option<String>>>,
) -> Option<String> {
    match state.get(name) {
        Some(Some(done)) => return done.clone(),
        // A cycle: every property in it is invalid.
        Some(None) => return None,
        None => {}
    }
    let value = raw.get(name)?.trim();
    let keyword = value.to_ascii_lowercase();
    let resolved = match keyword.as_str() {
        "initial" => None,
        "inherit" | "unset" | "revert" | "revert-layer" => {
            inherited.get(name).map(ToString::to_string)
        }
        _ => {
            state.insert(name, None);
            let mut lookup = |reference: &str| -> Option<String> {
                if let Some((key, _)) = raw.get_key_value(reference) {
                    resolve_custom(key, raw, inherited, state)
                } else {
                    inherited.get(reference).map(ToString::to_string)
                }
            };
            substitute(value, &mut lookup).map(Cow::into_owned)
        }
    };
    state.insert(name, Some(resolved.clone()));
    resolved
}

/// Substitute every `var()` in `value`. Fails, making the declaration
/// invalid at computed-value time, when a reference has neither a value nor a
/// fallback.
fn substitute<'v>(
    value: &'v str,
    lookup: &mut dyn FnMut(&str) -> Option<String>,
) -> Option<Cow<'v, str>> {
    let Some(start) = find_function(value, "var") else {
        return Some(Cow::Borrowed(value));
    };
    let mut output = String::with_capacity(value.len());
    let mut rest = value;
    let mut next = Some(start);
    while let Some(start) = next {
        output.push_str(&rest[..start]);
        let open = start + "var(".len();
        let close = matching_paren(rest, open)?;
        let arguments = &rest[open..close];
        let (name, fallback) = match split_top_level_once(arguments) {
            Some((name, fallback)) => (name.trim(), Some(fallback)),
            None => (arguments.trim(), None),
        };
        if !name.starts_with("--") {
            return None;
        }
        match lookup(name) {
            Some(found) => output.push_str(&found),
            None => {
                let fallback = fallback?;
                output.push_str(&substitute(fallback.trim(), lookup)?);
            }
        }
        rest = &rest[close + 1..];
        next = find_function(rest, "var");
    }
    output.push_str(rest);
    Some(Cow::Owned(output))
}

/// Replace `light-dark()`, `currentColor` and (where `colors` allows)
/// system-color keywords with concrete colors.
fn resolve_contextual<'v>(value: &'v str, context: &ColorContext, colors: bool) -> Cow<'v, str> {
    if !needs_context(value, colors) {
        return Cow::Borrowed(value);
    }
    let mut output = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'"' || byte == b'\'' {
            let end = value[index + 1..]
                .find(byte as char)
                .map_or(value.len(), |offset| index + 2 + offset);
            output.push_str(&value[index..end]);
            index = end;
            continue;
        }
        let starts_ident = byte.is_ascii_alphabetic()
            && (index == 0 || !is_ident_byte(bytes[index - 1]) && bytes[index - 1] != b'#');
        if !starts_ident {
            let character = value[index..].chars().next().unwrap_or_default();
            output.push(character);
            index += character.len_utf8().max(1);
            continue;
        }
        let end = index
            + value[index..]
                .bytes()
                .take_while(|byte| is_ident_byte(*byte))
                .count();
        let ident = &value[index..end];
        if bytes.get(end) == Some(&b'(') && ident.eq_ignore_ascii_case("light-dark") {
            let Some(close) = matching_paren(value, end + 1) else {
                output.push_str(&value[index..]);
                break;
            };
            let arguments = &value[end + 1..close];
            if let Some((light, dark)) = split_top_level_once(arguments) {
                let chosen = if context.scheme == ColorScheme::Dark {
                    dark
                } else {
                    light
                };
                output.push_str(&resolve_contextual(chosen.trim(), context, true));
            } else {
                output.push_str(&value[index..=close]);
            }
            index = close + 1;
            continue;
        }
        if ident.eq_ignore_ascii_case("currentcolor") {
            output.push_str(&context.current_color.to_css());
        } else if colors
            && bytes.get(end) != Some(&b'(')
            && let Some(color) = system_color(ident, context.scheme)
        {
            output.push_str(&color.to_css());
        } else {
            output.push_str(ident);
        }
        index = end;
    }
    Cow::Owned(output)
}

const fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

fn needs_context(value: &str, colors: bool) -> bool {
    contains_ignore_case(value, "light-dark(")
        || contains_ignore_case(value, "currentcolor")
        || (colors
            && value
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .any(|word| {
                    word.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                        && system_color(word, ColorScheme::Light).is_some()
                }))
}

fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

/// Byte offset of `name(` in `value`, as a function token.
fn find_function(value: &str, name: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    let mut from = 0;
    while let Some(offset) = value[from..].find(name) {
        let start = from + offset;
        let end = start + name.len();
        let boundary = start == 0 || !is_ident_byte(bytes[start - 1]);
        if boundary && bytes.get(end) == Some(&b'(') {
            return Some(start);
        }
        from = end;
    }
    None
}

/// Index of the `)` closing the parenthesis opened just before `open`.
fn matching_paren(value: &str, open: usize) -> Option<usize> {
    let mut depth = 1_usize;
    for (offset, byte) in value.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split at the first comma outside parentheses.
fn split_top_level_once(value: &str) -> Option<(&str, &str)> {
    let mut depth = 0_usize;
    for (index, byte) in value.bytes().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => return Some((&value[..index], &value[index + 1..])),
            _ => {}
        }
    }
    None
}

/// Whether a property takes colors, so that bare keywords in it may name
/// system colors.
fn color_valued(property: &StyleProperty) -> bool {
    match property {
        StyleProperty::Color
        | StyleProperty::Background
        | StyleProperty::BackgroundColor
        | StyleProperty::BorderColor
        | StyleProperty::Border
        | StyleProperty::BorderTop
        | StyleProperty::BorderRight
        | StyleProperty::BorderBottom
        | StyleProperty::BorderLeft
        | StyleProperty::Outline
        | StyleProperty::BoxShadow
        | StyleProperty::TextDecoration => true,
        property => {
            let name = property.as_str();
            name.ends_with("-color")
                || matches!(name, "fill" | "stroke" | "text-shadow" | "column-rule")
        }
    }
}

/// The inherited computed state an element's declarations resolve against.
#[derive(Clone, Debug, Default)]
pub struct ComputedScope {
    custom: CustomProperties,
    /// The computed `color-scheme` value, inherited.
    color_scheme: Option<CompactString>,
    preferred: ColorScheme,
    scheme: ColorScheme,
    color: Option<Rgba>,
}

impl ComputedScope {
    /// The scope above the document root, for a user preferring `preferred`.
    #[must_use]
    pub fn root(preferred: ColorScheme) -> Self {
        Self {
            preferred,
            ..Self::default()
        }
    }

    /// The element's used color scheme.
    #[must_use]
    pub const fn scheme(&self) -> ColorScheme {
        self.scheme
    }

    /// Whether `color-scheme` was set on this element or an ancestor.
    #[must_use]
    pub const fn declares_color_scheme(&self) -> bool {
        self.color_scheme.is_some()
    }

    /// The computed `color`, or `CanvasText` of the used scheme where no
    /// element sets one.
    #[must_use]
    pub fn color(&self) -> Rgba {
        self.color
            .unwrap_or_else(|| system_color("canvastext", self.scheme).unwrap_or(Rgba::BLACK))
    }

    #[must_use]
    pub const fn custom_properties(&self) -> &CustomProperties {
        &self.custom
    }

    #[must_use]
    pub fn color_context(&self) -> ColorContext {
        ColorContext {
            scheme: self.scheme,
            current_color: self.color(),
        }
    }

    /// A hash of everything declarations resolve against, for caches.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.custom.fingerprint.hash(&mut hasher);
        self.scheme.hash(&mut hasher);
        self.color.hash(&mut hasher);
        hasher.finish()
    }

    /// The scope of a child element, given its declarations in cascade
    /// order.
    #[must_use]
    pub fn child<'a, I>(&self, declarations: I) -> Self
    where
        I: IntoIterator<Item = &'a StyleDeclaration>,
        I::IntoIter: Clone,
    {
        let declarations = declarations.into_iter();
        let custom = self.custom.cascade(declarations.clone());
        let mut scope = Self {
            custom,
            color_scheme: self.color_scheme.clone(),
            preferred: self.preferred,
            scheme: self.scheme,
            color: self.color,
        };
        let last = |name: &str| {
            declarations
                .clone()
                .filter(|declaration| declaration.property.as_str() == name)
                .max_by_key(|declaration| declaration.important)
        };
        if let Some(declaration) = last("color-scheme")
            && let Some(value) = scope.substitute(declaration.value.as_str())
        {
            let value = value.trim();
            if !matches!(value.to_ascii_lowercase().as_str(), "inherit" | "unset") {
                scope.color_scheme = Some(value.into());
            }
        }
        scope.scheme = scope
            .color_scheme
            .as_deref()
            .map_or(ColorScheme::Light, |value| {
                used_color_scheme(value, scope.preferred)
            });
        if let Some(declaration) = last("color") {
            // `currentColor` in `color` is the inherited color.
            let inherited = ColorContext {
                scheme: scope.scheme,
                current_color: self.color(),
            };
            if let Some(color) = scope
                .substitute(declaration.value.as_str())
                .and_then(|value| parse_color(&value, &inherited))
            {
                scope.color = Some(color);
            }
        }
        scope
    }

    fn substitute<'v>(&self, value: &'v str) -> Option<Cow<'v, str>> {
        substitute(value, &mut |name| {
            self.custom.get(name).map(ToOwned::to_owned)
        })
    }

    /// The declaration with `var()`, `light-dark()`, `currentColor` and
    /// system colors resolved, borrowing it when nothing needed resolving.
    /// `None` means it is invalid at computed-value time: the property should
    /// take its inherited or initial value instead.
    #[must_use]
    pub fn resolve<'d>(
        &self,
        declaration: &'d StyleDeclaration,
    ) -> Option<Cow<'d, StyleDeclaration>> {
        let raw = declaration.value.as_str();
        let substituted = self.substitute(raw)?;
        let colors = color_valued(&declaration.property);
        let resolved = match substituted {
            Cow::Borrowed(value) => resolve_contextual(value, &self.color_context(), colors),
            Cow::Owned(value) => {
                Cow::Owned(resolve_contextual(&value, &self.color_context(), colors).into_owned())
            }
        };
        Some(match resolved {
            Cow::Borrowed(_) => Cow::Borrowed(declaration),
            Cow::Owned(value) => Cow::Owned(StyleDeclaration::new(
                declaration.property.clone(),
                value,
                declaration.important,
                declaration.span,
            )),
        })
    }
}

/// `text-transform`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum TextTransform {
    #[default]
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

impl TextTransform {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "none" => Self::None,
            "uppercase" => Self::Uppercase,
            "lowercase" => Self::Lowercase,
            "capitalize" => Self::Capitalize,
            _ => return None,
        })
    }

    /// Apply to text, borrowing it when unchanged.
    #[must_use]
    pub fn apply<'t>(self, text: &'t str) -> Cow<'t, str> {
        match self {
            Self::None => Cow::Borrowed(text),
            Self::Uppercase => Cow::Owned(text.to_uppercase()),
            Self::Lowercase => Cow::Owned(text.to_lowercase()),
            Self::Capitalize => {
                let mut output = String::with_capacity(text.len());
                let mut at_word_start = true;
                for character in text.chars() {
                    if at_word_start && character.is_alphanumeric() {
                        output.extend(character.to_uppercase());
                        at_word_start = false;
                    } else {
                        output.push(character);
                        if character.is_whitespace() || matches!(character, '-' | '/') {
                            at_word_start = true;
                        } else if character.is_alphanumeric() {
                            at_word_start = false;
                        }
                    }
                }
                Cow::Owned(output)
            }
        }
    }
}

/// Line kinds of `text-decoration-line`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct DecorationLines {
    pub underline: bool,
    pub overline: bool,
    pub line_through: bool,
}

/// `text-decoration-style`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DecorationStyle {
    #[default]
    Solid,
    Double,
    Dotted,
    Dashed,
    Wavy,
}

/// The parts of `text-decoration` or its longhands that a value sets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextDecoration {
    pub lines: Option<DecorationLines>,
    pub style: Option<DecorationStyle>,
    /// A color, still to be parsed.
    pub color: Option<CompactString>,
    /// A thickness: `auto`, `from-font`, or a length.
    pub thickness: Option<CompactString>,
}

impl TextDecoration {
    /// Parse the `text-decoration` shorthand.
    #[must_use]
    pub fn parse_shorthand(value: &str) -> Option<Self> {
        let mut decoration = Self {
            lines: Some(DecorationLines::default()),
            style: Some(DecorationStyle::Solid),
            ..Self::default()
        };
        let mut color = Vec::new();
        for word in split_whitespace_top_level(value) {
            let lower = word.to_ascii_lowercase();
            if let Some(lines) = decoration.lines.as_mut()
                && apply_line_keyword(lines, &lower)
            {
                continue;
            }
            if let Some(style) = decoration_style(&lower) {
                decoration.style = Some(style);
            } else if lower == "auto"
                || lower == "from-font"
                || lower.starts_with(|c: char| c.is_ascii_digit() || c == '.')
            {
                decoration.thickness = Some(word.into());
            } else {
                color.push(word);
            }
        }
        if !color.is_empty() {
            decoration.color = Some(color.join(" ").into());
        }
        Some(decoration)
    }

    /// Parse `text-decoration-line`.
    #[must_use]
    pub fn parse_lines(value: &str) -> Option<DecorationLines> {
        let mut lines = DecorationLines::default();
        for word in value.split_whitespace() {
            if !apply_line_keyword(&mut lines, &word.to_ascii_lowercase()) {
                return None;
            }
        }
        Some(lines)
    }

    /// Parse `text-decoration-style`.
    #[must_use]
    pub fn parse_style(value: &str) -> Option<DecorationStyle> {
        decoration_style(&value.trim().to_ascii_lowercase())
    }
}

fn apply_line_keyword(lines: &mut DecorationLines, keyword: &str) -> bool {
    match keyword {
        "none" => *lines = DecorationLines::default(),
        "underline" => lines.underline = true,
        "overline" => lines.overline = true,
        "line-through" => lines.line_through = true,
        // Not rendered, as in browsers.
        "blink" => {}
        _ => return false,
    }
    true
}

fn decoration_style(keyword: &str) -> Option<DecorationStyle> {
    Some(match keyword {
        "solid" => DecorationStyle::Solid,
        "double" => DecorationStyle::Double,
        "dotted" => DecorationStyle::Dotted,
        "dashed" => DecorationStyle::Dashed,
        "wavy" => DecorationStyle::Wavy,
        _ => return None,
    })
}

fn split_whitespace_top_level(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0_usize;
    let mut start = None;
    for (index, character) in value.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if c.is_whitespace() && depth == 0 => {
                if let Some(from) = start.take() {
                    parts.push(&value[from..index]);
                }
                continue;
            }
            _ => {}
        }
        start.get_or_insert(index);
    }
    if let Some(from) = start {
        parts.push(&value[from..]);
    }
    parts
}

/// OpenType features selected by the inherited `font-variant-*`,
/// `font-kerning` and `font-feature-settings` properties.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct FontFeatures {
    /// Features from `font-variant-*` and `font-kerning`, by property.
    variants: [Vec<(CompactString, u32)>; 6],
    /// `font-feature-settings`, applied last.
    settings: Vec<(CompactString, u32)>,
}

const NUMERIC: usize = 0;
const LIGATURES: usize = 1;
const CAPS: usize = 2;
const POSITION: usize = 3;
const EAST_ASIAN: usize = 4;
const KERNING: usize = 5;

impl FontFeatures {
    /// Whether any feature is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.variants.iter().all(Vec::is_empty) && self.settings.is_empty()
    }

    /// The features of an element, inheriting those its declarations leave
    /// unset. Declarations are in cascade order.
    #[must_use]
    pub fn cascade<'a>(
        &self,
        declarations: impl IntoIterator<Item = &'a StyleDeclaration>,
    ) -> Self {
        let mut features = self.clone();
        for declaration in declarations {
            let value = declaration.value.as_str().trim();
            let lower = value.to_ascii_lowercase();
            match declaration.property.as_str() {
                "font-variant-numeric" => {
                    if let Some(list) = variant_features(&lower, NUMERIC) {
                        features.variants[NUMERIC] = list;
                    }
                }
                "font-variant-ligatures" => {
                    if let Some(list) = variant_features(&lower, LIGATURES) {
                        features.variants[LIGATURES] = list;
                    }
                }
                "font-variant-caps" => {
                    if let Some(list) = variant_features(&lower, CAPS) {
                        features.variants[CAPS] = list;
                    }
                }
                "font-variant-position" => {
                    if let Some(list) = variant_features(&lower, POSITION) {
                        features.variants[POSITION] = list;
                    }
                }
                "font-variant-east-asian" => {
                    if let Some(list) = variant_features(&lower, EAST_ASIAN) {
                        features.variants[EAST_ASIAN] = list;
                    }
                }
                "font-kerning" => match lower.as_str() {
                    "none" => features.variants[KERNING] = vec![("kern".into(), 0)],
                    "normal" | "auto" => features.variants[KERNING].clear(),
                    _ => {}
                },
                "font-variant" => {
                    // The shorthand resets every longhand it covers.
                    if lower == "normal" || lower == "none" {
                        for index in [NUMERIC, LIGATURES, CAPS, POSITION, EAST_ASIAN] {
                            features.variants[index].clear();
                        }
                        if lower == "none" {
                            features.variants[LIGATURES] =
                                variant_features("none", LIGATURES).unwrap_or_default();
                        }
                        continue;
                    }
                    let mut lists: [Vec<(CompactString, u32)>; 5] = Default::default();
                    let mut valid = true;
                    for word in lower.split_whitespace() {
                        let target = [NUMERIC, LIGATURES, CAPS, POSITION, EAST_ASIAN]
                            .into_iter()
                            .find(|index| variant_features(word, *index).is_some());
                        match target {
                            Some(index) => lists[index]
                                .extend(variant_features(word, index).unwrap_or_default()),
                            None => valid = false,
                        }
                    }
                    if valid {
                        for (index, list) in lists.into_iter().enumerate() {
                            features.variants[index] = list;
                        }
                    }
                }
                "font-feature-settings" => {
                    if let Some(settings) = feature_settings(value) {
                        features.settings = settings;
                    }
                }
                _ => {}
            }
        }
        features
    }

    /// Every selected feature as `(tag, value)`, later entries overriding
    /// earlier ones for the same tag.
    #[must_use]
    pub fn features(&self) -> Vec<(CompactString, u32)> {
        let mut merged: Vec<(CompactString, u32)> = Vec::new();
        for (tag, value) in self.variants.iter().flatten().chain(&self.settings) {
            match merged.iter_mut().find(|(existing, _)| existing == tag) {
                Some(entry) => entry.1 = *value,
                None => merged.push((tag.clone(), *value)),
            }
        }
        merged
    }
}

/// Features for a `font-variant-*` value, or `None` when it is not a value
/// of that property.
fn variant_features(value: &str, property: usize) -> Option<Vec<(CompactString, u32)>> {
    if value == "normal" {
        return Some(Vec::new());
    }
    if property == LIGATURES && value == "none" {
        return Some(
            ["liga", "clig", "dlig", "hlig", "calt"]
                .into_iter()
                .map(|tag| (tag.into(), 0))
                .collect(),
        );
    }
    let mut features = Vec::new();
    for word in value.split_whitespace() {
        let tags: &[(&str, u32)] = match (property, word) {
            (NUMERIC, "lining-nums") => &[("lnum", 1)],
            (NUMERIC, "oldstyle-nums") => &[("onum", 1)],
            (NUMERIC, "proportional-nums") => &[("pnum", 1)],
            (NUMERIC, "tabular-nums") => &[("tnum", 1)],
            (NUMERIC, "diagonal-fractions") => &[("frac", 1)],
            (NUMERIC, "stacked-fractions") => &[("afrc", 1)],
            (NUMERIC, "ordinal") => &[("ordn", 1)],
            (NUMERIC, "slashed-zero") => &[("zero", 1)],
            (LIGATURES, "common-ligatures") => &[("liga", 1), ("clig", 1)],
            (LIGATURES, "no-common-ligatures") => &[("liga", 0), ("clig", 0)],
            (LIGATURES, "discretionary-ligatures") => &[("dlig", 1)],
            (LIGATURES, "no-discretionary-ligatures") => &[("dlig", 0)],
            (LIGATURES, "historical-ligatures") => &[("hlig", 1)],
            (LIGATURES, "no-historical-ligatures") => &[("hlig", 0)],
            (LIGATURES, "contextual") => &[("calt", 1)],
            (LIGATURES, "no-contextual") => &[("calt", 0)],
            (CAPS, "small-caps") => &[("smcp", 1)],
            (CAPS, "all-small-caps") => &[("c2sc", 1), ("smcp", 1)],
            (CAPS, "petite-caps") => &[("pcap", 1)],
            (CAPS, "all-petite-caps") => &[("c2pc", 1), ("pcap", 1)],
            (CAPS, "unicase") => &[("unic", 1)],
            (CAPS, "titling-caps") => &[("titl", 1)],
            (POSITION, "sub") => &[("subs", 1)],
            (POSITION, "super") => &[("sups", 1)],
            (EAST_ASIAN, "jis78") => &[("jp78", 1)],
            (EAST_ASIAN, "jis83") => &[("jp83", 1)],
            (EAST_ASIAN, "jis90") => &[("jp90", 1)],
            (EAST_ASIAN, "jis04") => &[("jp04", 1)],
            (EAST_ASIAN, "simplified") => &[("smpl", 1)],
            (EAST_ASIAN, "traditional") => &[("trad", 1)],
            (EAST_ASIAN, "full-width") => &[("fwid", 1)],
            (EAST_ASIAN, "proportional-width") => &[("pwid", 1)],
            (EAST_ASIAN, "ruby") => &[("ruby", 1)],
            _ => return None,
        };
        features.extend(tags.iter().map(|(tag, value)| ((*tag).into(), *value)));
    }
    Some(features)
}

/// `font-feature-settings`: `normal`, or `"tag" [on | off | <integer>]`, ….
fn feature_settings(value: &str) -> Option<Vec<(CompactString, u32)>> {
    if value.eq_ignore_ascii_case("normal") {
        return Some(Vec::new());
    }
    value
        .split(',')
        .map(|setting| {
            let setting = setting.trim();
            let quote = setting.chars().next().filter(|c| matches!(c, '"' | '\''))?;
            let rest = &setting[1..];
            let close = rest.find(quote)?;
            let tag = &rest[..close];
            if tag.len() != 4 || !tag.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
                return None;
            }
            let value = match rest[close + 1..].trim().to_ascii_lowercase().as_str() {
                "" | "on" => 1,
                "off" => 0,
                number => number.parse::<u32>().ok()?,
            };
            Some((CompactString::from(tag), value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        ColorContext, ColorScheme, ComputedScope, CustomProperties, DecorationStyle, FontFeatures,
        Rgba, TextDecoration, TextTransform, parse_color, used_color_scheme,
    };
    use crate::style::StyleDeclaration;

    fn declarations(pairs: &[(&str, &str)]) -> Vec<StyleDeclaration> {
        pairs
            .iter()
            .map(|(property, value)| StyleDeclaration::new(*property, *value, false, None))
            .collect()
    }

    fn hex(value: &str, context: &ColorContext) -> u32 {
        parse_color(value, context).map_or(0xdead_beef, Rgba::to_u32)
    }

    #[test]
    fn colors_cover_css_color_4_and_5() {
        let light = ColorContext::default();
        for value in [
            "#ff8000",
            "rgb(255 128 0)",
            "rgb(255, 128, 0)",
            "hsl(30.12 100% 50%)",
            "color(srgb 1 0.502 0)",
        ] {
            assert_eq!(hex(value, &light), 0xff80_00ff, "{value}");
        }
        assert_eq!(hex("RebeccaPurple", &light), 0x6633_99ff);
        assert_eq!(hex("oklch(62.8% 0.2577 29.23)", &light), 0xff00_00ff);
        assert_eq!(hex("color-mix(in srgb, red, blue)", &light), 0x8000_80ff);
        // Out of gamut: mapped by chroma reduction, not channel clipping.
        let p3 = hex("color(display-p3 1 0 0)", &light);
        assert_eq!(p3 >> 24, 0xff);
        assert!(
            (p3 >> 16) & 0xff > 0,
            "gamut mapping keeps some green: {p3:x}"
        );
    }

    #[test]
    fn context_colors_follow_scheme_and_current_color() {
        let light = ColorContext {
            scheme: ColorScheme::Light,
            current_color: Rgba::opaque(0x112233),
        };
        let dark = ColorContext {
            scheme: ColorScheme::Dark,
            ..light
        };
        assert_eq!(hex("light-dark(#fff, #000)", &light), 0xffff_ffff);
        assert_eq!(hex("light-dark(#fff, #000)", &dark), 0x0000_00ff);
        assert_eq!(hex("currentColor", &light), 0x1122_33ff);
        assert_eq!(hex("Canvas", &dark), 0x1212_12ff);
        assert_eq!(hex("canvastext", &light), 0x0000_00ff);
        assert_eq!(
            hex(
                "color-mix(in srgb, currentcolor, light-dark(white, black))",
                &dark
            ),
            0x0911_1aff
        );
    }

    #[test]
    fn color_scheme_picks_the_preferred_supported_scheme() {
        assert_eq!(
            used_color_scheme("light dark", ColorScheme::Dark),
            ColorScheme::Dark
        );
        assert_eq!(
            used_color_scheme("light dark", ColorScheme::Light),
            ColorScheme::Light
        );
        assert_eq!(
            used_color_scheme("light", ColorScheme::Dark),
            ColorScheme::Light
        );
        assert_eq!(
            used_color_scheme("only dark", ColorScheme::Light),
            ColorScheme::Dark
        );
        assert_eq!(
            used_color_scheme("normal", ColorScheme::Dark),
            ColorScheme::Light
        );
    }

    #[test]
    fn custom_properties_inherit_substitute_and_reject_cycles() {
        let root = CustomProperties::default().cascade(&declarations(&[
            ("--space", "4px"),
            ("--double", "calc(var(--space) * 2)"),
            ("--a", "var(--b)"),
            ("--b", "var(--a)"),
            ("--fallback", "var(--missing, 1px)"),
        ]));
        assert_eq!(root.get("--double"), Some("calc(4px * 2)"));
        assert_eq!(root.get("--a"), None, "a cycle is invalid");
        assert_eq!(root.get("--fallback"), Some("1px"));
        let child = root.cascade(&declarations(&[
            ("--space", "8px"),
            ("--own", "var(--space)"),
        ]));
        assert_eq!(child.get("--own"), Some("8px"));
        assert_eq!(
            child.get("--double"),
            Some("calc(4px * 2)"),
            "inherited values were computed in their own scope"
        );
        let unchanged = child.cascade(&declarations(&[("color", "red")]));
        assert_eq!(unchanged.fingerprint(), child.fingerprint());
    }

    #[test]
    fn scopes_resolve_declarations() {
        let root = ComputedScope::root(ColorScheme::Dark).child(&declarations(&[
            ("--accent", "light-dark(#000, #fff)"),
            ("color-scheme", "light dark"),
            ("color", "rgb(0 128 0)"),
        ]));
        assert_eq!(root.scheme(), ColorScheme::Dark);
        let border = StyleDeclaration::new("border", "1px solid currentColor", false, None);
        assert_eq!(
            root.resolve(&border).map(|d| d.value.as_str().to_owned()),
            Some("1px solid #008000ff".to_owned())
        );
        let background = StyleDeclaration::new("background-color", "var(--accent)", false, None);
        assert_eq!(
            root.resolve(&background)
                .map(|d| d.value.as_str().to_owned()),
            Some("#fff".to_owned())
        );
        let missing = StyleDeclaration::new("width", "var(--nope)", false, None);
        assert!(
            root.resolve(&missing).is_none(),
            "invalid at computed-value time"
        );
        let quoted = StyleDeclaration::new("content", "\"é→\" currentColor", false, None);
        assert_eq!(
            root.resolve(&quoted).map(|d| d.value.as_str().to_owned()),
            Some("\"é→\" #008000ff".to_owned())
        );
        let plain = StyleDeclaration::new("font-family", "Field Gothic", false, None);
        assert!(matches!(
            root.resolve(&plain),
            Some(std::borrow::Cow::Borrowed(_))
        ));
        let light = ComputedScope::root(ColorScheme::Dark);
        assert_eq!(
            light.scheme(),
            ColorScheme::Light,
            "no color-scheme is light only"
        );
    }

    #[test]
    fn text_values_parse() {
        let decoration =
            TextDecoration::parse_shorthand("underline wavy rgb(1 2 3) 2px").unwrap_or_default();
        assert!(decoration.lines.is_some_and(|lines| lines.underline));
        assert_eq!(decoration.style, Some(DecorationStyle::Wavy));
        assert_eq!(decoration.color.as_deref(), Some("rgb(1 2 3)"));
        assert_eq!(decoration.thickness.as_deref(), Some("2px"));
        assert_eq!(
            TextTransform::Capitalize.apply("hello big-world"),
            "Hello Big-World"
        );

        let parent = FontFeatures::default().cascade(&declarations(&[(
            "font-variant",
            "small-caps tabular-nums",
        )]));
        let child = parent.cascade(&declarations(&[
            ("font-variant-numeric", "oldstyle-nums"),
            ("font-feature-settings", "\"smcp\" off, \"ss01\""),
        ]));
        let features = child.features();
        assert!(features.contains(&("onum".into(), 1)));
        assert!(!features.iter().any(|(tag, _)| tag == "tnum"));
        assert!(
            features.contains(&("smcp".into(), 0)),
            "settings apply last"
        );
        assert!(features.contains(&("ss01".into(), 1)));
    }
}
