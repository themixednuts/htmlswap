use std::collections::BTreeMap;

use crate::plan::{RenderElement, RenderNode, RenderPlan};
use crate::source::{SourceId, SourceMap, Span};

const DEBUG_ID_ATTRIBUTE: &str = "data-htmlswap-debug-id";
const SOURCE_SPAN_ATTRIBUTE: &str = "data-htmlswap-source-span";
const SOURCE_KEY_ATTRIBUTE: &str = "data-htmlswap-source-key";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutDebugOptions {
    pub inject_script: bool,
}

impl Default for LayoutDebugOptions {
    fn default() -> Self {
        Self {
            inject_script: true,
        }
    }
}

#[must_use]
pub fn layout_debug_id_for_element(element: &RenderElement) -> Option<String> {
    source_element_id(element)
        .map(str::to_owned)
        .or_else(|| element.span.map(layout_debug_id_for_span))
}

#[must_use]
pub fn layout_debug_id_for_span(span: Span) -> String {
    format!(
        "htmlswap_debug_s{}_{}_{}",
        span.source.index(),
        span.start,
        span.end
    )
}

#[must_use]
pub fn layout_debug_id_for_element_in_sources(
    element: &RenderElement,
    sources: Option<&SourceMap>,
) -> Option<String> {
    source_element_id(element).map(str::to_owned).or_else(|| {
        element
            .span
            .map(|span| layout_debug_id_for_span_in_sources(span, sources))
    })
}

#[must_use]
pub fn layout_debug_id_for_span_in_sources(span: Span, sources: Option<&SourceMap>) -> String {
    format!(
        "htmlswap_debug_{}",
        layout_source_key_for_span_in_sources(span, sources)
    )
}

#[must_use]
pub fn layout_source_key_for_span(span: Span) -> String {
    format!("s{}:{}:{}", span.source.index(), span.start, span.end)
}

#[must_use]
pub fn layout_source_key_for_span_in_sources(span: Span, sources: Option<&SourceMap>) -> String {
    let scope = layout_source_scope(span.source, sources);
    if source_has_name(span.source, sources) {
        return format!("{scope}_{}_{}", span.start, span.end);
    }

    format!(
        "{scope}_s{}_{}_{}",
        span.source.index(),
        span.start,
        span.end
    )
}

#[must_use]
pub fn layout_source_scope(source: SourceId, sources: Option<&SourceMap>) -> String {
    let Some(name) = sources
        .and_then(|sources| sources.file(source))
        .and_then(|file| file.name())
    else {
        return "source".to_owned();
    };

    let file_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut sanitized = String::new();
    let mut previous_separator = false;
    for ch in file_name.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch.to_ascii_lowercase());
            previous_separator = false;
        } else if !previous_separator {
            sanitized.push('_');
            previous_separator = true;
        }
    }
    let sanitized = sanitized.trim_matches('_');
    let sanitized = if sanitized.is_empty() {
        "source"
    } else {
        sanitized
    };
    format!("{sanitized}_{:016x}", fnv1a64(name.as_bytes()))
}

fn source_has_name(source: SourceId, sources: Option<&SourceMap>) -> bool {
    sources
        .and_then(|sources| sources.file(source))
        .and_then(|file| file.name())
        .is_some()
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[must_use]
pub fn instrument_layout_snapshot_html(
    source: &str,
    plan: &RenderPlan,
    options: &LayoutDebugOptions,
) -> String {
    instrument_layout_snapshot_html_with_sources(source, plan, None, options)
}

#[must_use]
pub fn instrument_layout_snapshot_html_with_sources(
    source: &str,
    plan: &RenderPlan,
    sources: Option<&SourceMap>,
    options: &LayoutDebugOptions,
) -> String {
    instrument_layout_snapshot_html_for_source(source, plan, SourceId::primary(), sources, options)
}

#[must_use]
pub fn instrument_layout_snapshot_html_for_source(
    source: &str,
    plan: &RenderPlan,
    source_id: SourceId,
    sources: Option<&SourceMap>,
    options: &LayoutDebugOptions,
) -> String {
    let mut insertions = BTreeMap::new();
    collect_element_insertions(&plan.nodes, source, source_id, sources, &mut insertions);

    let mut output = source.to_owned();
    for (offset, insertion) in insertions.into_iter().rev() {
        output.insert_str(offset, &insertion);
    }

    if options.inject_script {
        inject_snapshot_script(&mut output);
    }

    output
}

fn collect_element_insertions(
    nodes: &[RenderNode],
    source: &str,
    source_id: SourceId,
    sources: Option<&SourceMap>,
    insertions: &mut BTreeMap<usize, String>,
) {
    for node in nodes {
        if let RenderNode::Element(element) = node {
            collect_element_insertion(element, source, source_id, sources, insertions);
        }
    }
}

fn collect_element_insertion(
    element: &RenderElement,
    source: &str,
    source_id: SourceId,
    sources: Option<&SourceMap>,
    insertions: &mut BTreeMap<usize, String>,
) {
    if let Some(span) = element.span
        && span.source == source_id
        && let Some(opening) = source.get(span.start..span.end)
        && !opening.contains(DEBUG_ID_ATTRIBUTE)
        && let Some(offset) = opening_tag_attribute_insert_offset(source, span)
    {
        let debug_id = layout_debug_id_for_element_in_sources(element, sources)
            .unwrap_or_else(|| layout_debug_id_for_span_in_sources(span, sources));
        let source_key = layout_source_key_for_span_in_sources(span, sources);
        insertions.entry(offset).or_insert_with(|| {
            format!(
                " {DEBUG_ID_ATTRIBUTE}=\"{}\" {SOURCE_KEY_ATTRIBUTE}=\"{}\" {SOURCE_SPAN_ATTRIBUTE}=\"{}:{}\"",
                escape_attr(&debug_id),
                escape_attr(&source_key),
                span.start,
                span.end
            )
        });
    }

    collect_element_insertions(&element.children, source, source_id, sources, insertions);
}

fn opening_tag_attribute_insert_offset(source: &str, span: Span) -> Option<usize> {
    let bytes = source.as_bytes();
    if span.start >= span.end || span.end > bytes.len() {
        return None;
    }

    let mut quote = None;
    for offset in span.start..span.end {
        let byte = bytes[offset];
        if let Some(quote_byte) = quote {
            if byte == quote_byte {
                quote = None;
            }
            continue;
        }

        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'>' => return Some(tag_insert_offset_before_close(bytes, span.start, offset)),
            _ => {}
        }
    }

    None
}

fn tag_insert_offset_before_close(bytes: &[u8], start: usize, close: usize) -> usize {
    let mut offset = close;
    while offset > start && bytes[offset - 1].is_ascii_whitespace() {
        offset -= 1;
    }
    if offset > start && bytes[offset - 1] == b'/' {
        offset - 1
    } else {
        close
    }
}

fn inject_snapshot_script(output: &mut String) {
    let script = r#"<script>
(function () {
  function snapshot() {
    return Array.from(document.querySelectorAll("[data-htmlswap-debug-id]")).map(function (node, index) {
      var rect = node.getBoundingClientRect();
      var parentRect = node.parentElement ? node.parentElement.getBoundingClientRect() : null;
      return {
        index: index,
        tag: node.tagName.toLowerCase(),
        id: node.id || null,
        debug_id: node.getAttribute("data-htmlswap-debug-id"),
        source_key: node.getAttribute("data-htmlswap-source-key"),
        source_span: node.getAttribute("data-htmlswap-source-span"),
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        relative_x: parentRect ? rect.x - parentRect.x : rect.x,
        relative_y: parentRect ? rect.y - parentRect.y : rect.y,
        z_index: getComputedStyle(node).zIndex,
        display: getComputedStyle(node).display,
        position: getComputedStyle(node).position
      };
    });
  }
  window.__HTMLSWAP_LAYOUT_SNAPSHOT__ = snapshot;
  window.addEventListener("load", function () {
    console.log("HTMLSWAP_LAYOUT_SNAPSHOT", snapshot());
  });
})();
</script>"#;

    if let Some(offset) = find_case_insensitive(output, "</body>") {
        output.insert_str(offset, script);
    } else {
        output.push_str(script);
    }
}

fn find_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

fn source_element_id(element: &RenderElement) -> Option<&str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name == "id")
        .map(|attribute| attribute.value.as_str())
        .filter(|value| !value.trim().is_empty())
}

fn escape_attr(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '"' => output.push_str("&quot;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(ch),
        }
    }
    output
}
