use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::mem;
use std::rc::{Rc, Weak};

use html5ever::interface::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{
    Attribute, ExpandedName, ParseOpts, QualName, local_name, ns,
    parse_document as parse_html_document, parse_fragment as parse_html_fragment,
};

use crate::diagnostics::{Compilation, Diagnostic, Diagnostics};
use crate::ir::{
    HtmlAttribute, HtmlComment, HtmlDocument, HtmlElement, HtmlName, HtmlNode, HtmlText,
};
use crate::source::{SourceId, Span};

type Handle = Rc<SinkNode>;
type WeakHandle = Weak<SinkNode>;

#[must_use]
pub fn parse_fragment(source: &str) -> Compilation<HtmlDocument> {
    parse_fragment_with_source(source, SourceId::primary())
}

#[must_use]
pub(crate) fn parse_fragment_with_source(
    source: &str,
    source_id: SourceId,
) -> Compilation<HtmlDocument> {
    let context = QualName::new(None, ns!(html), local_name!("body"));

    parse_html_fragment(
        HtmlSink::new(source, source_id),
        ParseOpts::default(),
        context,
        Vec::new(),
        false,
    )
    .one(source)
    .map(strip_fragment_wrappers)
    .map(|mut document| {
        attach_source_spans(&mut document, source, source_id);
        document
    })
}

#[must_use]
pub fn parse_document(source: &str) -> Compilation<HtmlDocument> {
    parse_document_with_source(source, SourceId::primary())
}

#[must_use]
pub(crate) fn parse_document_with_source(
    source: &str,
    source_id: SourceId,
) -> Compilation<HtmlDocument> {
    parse_html_document(HtmlSink::new(source, source_id), ParseOpts::default())
        .one(source)
        .map(|mut document| {
            attach_source_spans(&mut document, source, source_id);
            document
        })
}

struct SinkNode {
    parent: Cell<Option<WeakHandle>>,
    children: RefCell<Vec<Handle>>,
    data: SinkNodeData,
}

impl SinkNode {
    fn new(data: SinkNodeData) -> Handle {
        Rc::new(Self {
            parent: Cell::new(None),
            children: RefCell::new(Vec::new()),
            data,
        })
    }

    fn parent(&self) -> Option<WeakHandle> {
        let parent = self.parent.take();
        self.parent.set(parent.clone());
        parent
    }
}

enum SinkNodeData {
    Document,
    Doctype,
    Text {
        contents: RefCell<String>,
    },
    Comment {
        contents: String,
    },
    Element {
        name: QualName,
        attrs: RefCell<Vec<Attribute>>,
        template_contents: RefCell<Option<Handle>>,
        mathml_annotation_xml_integration_point: bool,
    },
    ProcessingInstruction,
}

struct HtmlParseError {
    message: Cow<'static, str>,
    line_number: u64,
}

struct HtmlSink<'source> {
    document: Handle,
    errors: RefCell<Vec<HtmlParseError>>,
    quirks_mode: Cell<QuirksMode>,
    current_line: Cell<u64>,
    source: &'source str,
    source_id: SourceId,
}

impl<'source> HtmlSink<'source> {
    fn new(source: &'source str, source_id: SourceId) -> Self {
        Self {
            document: SinkNode::new(SinkNodeData::Document),
            errors: RefCell::new(Vec::new()),
            quirks_mode: Cell::new(QuirksMode::NoQuirks),
            current_line: Cell::new(1),
            source,
            source_id,
        }
    }
}

fn is_non_actionable_parse_recovery(message: &str) -> bool {
    message == "Found special tag while closing generic tag"
}

impl TreeSink for HtmlSink<'_> {
    type Handle = Handle;
    type Output = Compilation<HtmlDocument>;
    type ElemName<'a>
        = ExpandedName<'a>
    where
        Self: 'a;

    fn finish(self) -> Self::Output {
        let diagnostics = self
            .errors
            .borrow()
            .iter()
            .filter(|error| !is_non_actionable_parse_recovery(&error.message))
            .map(|error| {
                Diagnostic::warning(
                    format!("HTML parse error: {}", error.message),
                    line_span(self.source, self.source_id, error.line_number),
                )
            })
            .collect::<Diagnostics>();

        Compilation::new(
            HtmlDocument::new(convert_children(&self.document)),
            diagnostics,
        )
    }

    fn parse_error(&self, msg: Cow<'static, str>) {
        self.errors.borrow_mut().push(HtmlParseError {
            message: msg,
            line_number: self.current_line.get(),
        });
    }

    fn get_document(&self) -> Self::Handle {
        self.document.clone()
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> Self::ElemName<'a> {
        match &target.data {
            SinkNodeData::Element { name, .. } => name.expanded(),
            _ => panic!("elem_name called on a non-element node"),
        }
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        flags: ElementFlags,
    ) -> Self::Handle {
        SinkNode::new(SinkNodeData::Element {
            name,
            attrs: RefCell::new(attrs),
            template_contents: RefCell::new(if flags.template {
                Some(SinkNode::new(SinkNodeData::Document))
            } else {
                None
            }),
            mathml_annotation_xml_integration_point: flags.mathml_annotation_xml_integration_point,
        })
    }

    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        SinkNode::new(SinkNodeData::Comment {
            contents: text.to_string(),
        })
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Self::Handle {
        let _ = (target, data);
        SinkNode::new(SinkNodeData::ProcessingInstruction)
    }

    fn append(&self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        if let NodeOrText::AppendText(text) = &child
            && let Some(previous) = parent.children.borrow().last()
            && append_to_existing_text(previous, text)
        {
            return;
        }

        append_handle(
            parent,
            match child {
                NodeOrText::AppendText(text) => SinkNode::new(SinkNodeData::Text {
                    contents: RefCell::new(text.to_string()),
                }),
                NodeOrText::AppendNode(node) => node,
            },
        );
    }

    fn append_based_on_parent_node(
        &self,
        element: &Self::Handle,
        prev_element: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        if element.parent().is_some() {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        let _ = (name, public_id, system_id);
        append_handle(&self.document, SinkNode::new(SinkNodeData::Doctype));
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        match &target.data {
            SinkNodeData::Element {
                template_contents, ..
            } => template_contents
                .borrow()
                .as_ref()
                .expect("template contents requested from a non-template element")
                .clone(),
            _ => panic!("template contents requested from a non-element node"),
        }
    }

    fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
        Rc::ptr_eq(x, y)
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.quirks_mode.set(mode);
    }

    fn set_current_line(&self, line_number: u64) {
        self.current_line.set(line_number);
    }

    fn append_before_sibling(&self, sibling: &Self::Handle, child: NodeOrText<Self::Handle>) {
        let (parent, index) =
            parent_and_index(sibling).expect("append_before_sibling called without a parent");

        let child = match (child, index) {
            (NodeOrText::AppendText(text), 0) => SinkNode::new(SinkNodeData::Text {
                contents: RefCell::new(text.to_string()),
            }),
            (NodeOrText::AppendText(text), index) => {
                let children = parent.children.borrow();
                let previous = &children[index - 1];
                if append_to_existing_text(previous, &text) {
                    return;
                }
                SinkNode::new(SinkNodeData::Text {
                    contents: RefCell::new(text.to_string()),
                })
            }
            (NodeOrText::AppendNode(node), _) => node,
        };

        remove_from_parent(&child);
        child.parent.set(Some(Rc::downgrade(&parent)));
        parent.children.borrow_mut().insert(index, child);
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<Attribute>) {
        let SinkNodeData::Element {
            attrs: existing, ..
        } = &target.data
        else {
            panic!("add_attrs_if_missing called on a non-element node");
        };

        let mut existing = existing.borrow_mut();
        let existing_names = existing
            .iter()
            .map(|attribute| attribute.name.clone())
            .collect::<HashSet<_>>();
        existing.extend(
            attrs
                .into_iter()
                .filter(|attribute| !existing_names.contains(&attribute.name)),
        );
    }

    fn remove_from_parent(&self, target: &Self::Handle) {
        remove_from_parent(target);
    }

    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        let mut children = node.children.borrow_mut();
        for child in children.iter() {
            let previous_parent = child.parent.replace(Some(Rc::downgrade(new_parent)));
            assert!(
                previous_parent
                    .and_then(|parent| parent.upgrade())
                    .is_some_and(|parent| Rc::ptr_eq(&parent, node))
            );
        }
        new_parent
            .children
            .borrow_mut()
            .extend(mem::take(&mut *children));
    }

    fn is_mathml_annotation_xml_integration_point(&self, target: &Self::Handle) -> bool {
        match target.data {
            SinkNodeData::Element {
                mathml_annotation_xml_integration_point,
                ..
            } => mathml_annotation_xml_integration_point,
            _ => panic!("integration-point check called on a non-element node"),
        }
    }
}

fn append_handle(parent: &Handle, child: Handle) {
    let previous_parent = child.parent.replace(Some(Rc::downgrade(parent)));
    assert!(previous_parent.is_none());
    parent.children.borrow_mut().push(child);
}

fn append_to_existing_text(previous: &Handle, text: &str) -> bool {
    match &previous.data {
        SinkNodeData::Text { contents } => {
            contents.borrow_mut().push_str(text);
            true
        }
        _ => false,
    }
}

fn parent_and_index(target: &Handle) -> Option<(Handle, usize)> {
    let parent = target.parent()?;
    let parent = parent.upgrade().expect("dangling parent pointer");
    let index = parent
        .children
        .borrow()
        .iter()
        .position(|child| Rc::ptr_eq(child, target))
        .expect("node parent did not contain node");

    Some((parent, index))
}

fn remove_from_parent(target: &Handle) {
    if let Some((parent, index)) = parent_and_index(target) {
        parent.children.borrow_mut().remove(index);
        target.parent.set(None);
    }
}

fn convert_children(node: &Handle) -> Vec<HtmlNode> {
    node.children
        .borrow()
        .iter()
        .filter_map(convert_node)
        .collect()
}

fn convert_node(node: &Handle) -> Option<HtmlNode> {
    match &node.data {
        SinkNodeData::Document => None,
        SinkNodeData::Doctype | SinkNodeData::ProcessingInstruction => None,
        SinkNodeData::Text { contents } => Some(HtmlNode::Text(HtmlText {
            value: contents.borrow().clone(),
            span: None,
        })),
        SinkNodeData::Comment { contents } => Some(HtmlNode::Comment(HtmlComment {
            value: contents.clone(),
            span: None,
        })),
        SinkNodeData::Element { name, attrs, .. } => Some(HtmlNode::Element(HtmlElement {
            name: html_name(name),
            attributes: attrs
                .borrow()
                .iter()
                .map(|attribute| HtmlAttribute {
                    name: html_name(&attribute.name),
                    value: attribute.value.to_string().into(),
                    span: None,
                })
                .collect(),
            children: convert_children(node),
            span: None,
        })),
    }
}

fn html_name(name: &QualName) -> HtmlName {
    let namespace = name.ns.to_string();

    HtmlName::new(
        name.prefix.as_ref().map(ToString::to_string),
        (!namespace.is_empty()).then_some(namespace),
        name.local.to_string(),
    )
}

fn strip_fragment_wrappers(document: HtmlDocument) -> HtmlDocument {
    let mut nodes = document.nodes;

    loop {
        if nodes.len() != 1 {
            return HtmlDocument::new(nodes);
        }

        let node = nodes.remove(0);
        match node {
            HtmlNode::Element(element) if matches!(element.name.local(), "html" | "body") => {
                nodes = element.children;
            }
            node => return HtmlDocument::new(vec![node]),
        }
    }
}

fn attach_source_spans(document: &mut HtmlDocument, source: &str, source_id: SourceId) {
    let mut cursor = 0;
    attach_node_spans(&mut document.nodes, source, source_id, &mut cursor);
}

fn line_span(source: &str, source_id: SourceId, line_number: u64) -> Option<Span> {
    let target_line = usize::try_from(line_number.saturating_sub(1)).ok()?;
    let mut current_line = 0;
    let mut line_start = 0;

    for (index, byte) in source.bytes().enumerate() {
        if current_line == target_line {
            let line_end = source[index..]
                .find('\n')
                .map_or(source.len(), |offset| index + offset);
            return Some(Span::new(source_id, line_start, line_end));
        }

        if byte == b'\n' {
            current_line += 1;
            line_start = index + 1;
        }
    }

    (current_line == target_line).then_some(Span::new(source_id, line_start, source.len()))
}

fn attach_node_spans(
    nodes: &mut [HtmlNode],
    source: &str,
    source_id: SourceId,
    cursor: &mut usize,
) {
    for node in nodes {
        attach_node_span(node, source, source_id, cursor);
    }
}

fn attach_node_span(node: &mut HtmlNode, source: &str, source_id: SourceId, cursor: &mut usize) {
    match node {
        HtmlNode::Element(element) => attach_element_span(element, source, source_id, cursor),
        HtmlNode::Text(text) => attach_text_span(text, source, source_id, cursor),
        HtmlNode::Comment(comment) => attach_comment_span(comment, source, source_id, cursor),
    }
}

fn attach_element_span(
    element: &mut HtmlElement,
    source: &str,
    source_id: SourceId,
    cursor: &mut usize,
) {
    let Some(start_tag) = find_start_tag(source, *cursor, element.name.local()) else {
        attach_node_spans(&mut element.children, source, source_id, cursor);
        return;
    };

    attach_attribute_spans(&mut element.attributes, source, source_id, &start_tag);
    *cursor = start_tag.end;

    if !start_tag.self_closing && !is_void_element(element.name.local()) {
        attach_node_spans(&mut element.children, source, source_id, cursor);

        if let Some(end_tag) = find_end_tag(source, *cursor, element.name.local()) {
            element.span = Some(Span::new(source_id, start_tag.start, end_tag.end));
            *cursor = end_tag.end;
            return;
        }

        element.span = Some(Span::new(
            source_id,
            start_tag.start,
            (*cursor).max(start_tag.end),
        ));
        return;
    }

    element.span = Some(Span::new(source_id, start_tag.start, start_tag.end));
}

fn attach_text_span(text: &mut HtmlText, source: &str, source_id: SourceId, cursor: &mut usize) {
    if text.value.is_empty() {
        text.span = Some(Span::new(source_id, *cursor, *cursor));
        return;
    }

    let Some(start) = source[*cursor..]
        .find(&text.value)
        .map(|offset| *cursor + offset)
    else {
        return;
    };
    let end = start + text.value.len();
    text.span = Some(Span::new(source_id, start, end));
    *cursor = end;
}

fn attach_comment_span(
    comment: &mut HtmlComment,
    source: &str,
    source_id: SourceId,
    cursor: &mut usize,
) {
    let needle = format!("<!--{}-->", comment.value);
    let Some(start) = source[*cursor..]
        .find(&needle)
        .map(|offset| *cursor + offset)
    else {
        return;
    };
    let end = start + needle.len();
    comment.span = Some(Span::new(source_id, start, end));
    *cursor = end;
}

#[derive(Debug, Clone, Copy)]
struct StartTag {
    start: usize,
    name_end: usize,
    end: usize,
    self_closing: bool,
}

#[derive(Debug, Clone, Copy)]
struct EndTag {
    end: usize,
}

fn find_start_tag(source: &str, cursor: usize, local_name: &str) -> Option<StartTag> {
    let mut search = cursor.min(source.len());

    while let Some(offset) = source[search..].find('<') {
        let start = search + offset;
        let name_start = start + 1;
        let first = source.as_bytes().get(name_start)?;

        if matches!(*first, b'/' | b'!' | b'?') {
            search = name_start + 1;
            continue;
        }

        let name_end = consume_name(source, name_start);
        if name_end == name_start {
            search = name_start;
            continue;
        }

        let raw_name = &source[name_start..name_end];
        if !raw_name.eq_ignore_ascii_case(local_name) {
            search = name_end;
            continue;
        }

        let end = find_tag_close(source, name_end)?;
        let self_closing = source[name_end..end.saturating_sub(1)]
            .trim_end()
            .ends_with('/');

        return Some(StartTag {
            start,
            name_end,
            end,
            self_closing,
        });
    }

    None
}

fn find_end_tag(source: &str, cursor: usize, local_name: &str) -> Option<EndTag> {
    let mut search = cursor.min(source.len());

    while let Some(offset) = source[search..].find("</") {
        let start = search + offset;
        let name_start = start + 2;
        let name_end = consume_name(source, name_start);

        if name_end == name_start {
            search = name_start;
            continue;
        }

        let raw_name = &source[name_start..name_end];
        if !raw_name.eq_ignore_ascii_case(local_name) {
            search = name_end;
            continue;
        }

        let end = find_tag_close(source, name_end)?;
        return Some(EndTag { end });
    }

    None
}

fn attach_attribute_spans(
    attributes: &mut [HtmlAttribute],
    source: &str,
    source_id: SourceId,
    start_tag: &StartTag,
) {
    let mut cursor = start_tag.name_end;
    for attribute in attributes {
        if let Some(span) = find_attribute_span(
            source,
            source_id,
            cursor,
            start_tag.end,
            attribute.name.local(),
        ) {
            attribute.span = Some(span);
            cursor = span.end;
        }
    }
}

fn find_attribute_span(
    source: &str,
    source_id: SourceId,
    mut cursor: usize,
    tag_end: usize,
    attribute_name: &str,
) -> Option<Span> {
    while cursor < tag_end {
        cursor = skip_ascii_whitespace(source, cursor, tag_end);

        if source
            .as_bytes()
            .get(cursor)
            .is_none_or(|byte| matches!(*byte, b'>' | b'/'))
        {
            return None;
        }

        let name_start = cursor;
        let name_end = consume_attribute_name(source, name_start, tag_end);
        if name_end == name_start {
            cursor += 1;
            continue;
        }

        let raw_name = &source[name_start..name_end];
        let local_name = raw_name
            .rsplit_once(':')
            .map_or(raw_name, |(_, local)| local);
        let mut value_end = skip_ascii_whitespace(source, name_end, tag_end);

        if source.as_bytes().get(value_end) == Some(&b'=') {
            value_end += 1;
            value_end = skip_ascii_whitespace(source, value_end, tag_end);
            value_end = consume_attribute_value(source, value_end, tag_end);
        } else {
            value_end = name_end;
        }

        if local_name.eq_ignore_ascii_case(attribute_name) {
            return Some(Span::new(source_id, name_start, value_end));
        }

        cursor = value_end;
    }

    None
}

fn consume_attribute_value(source: &str, cursor: usize, tag_end: usize) -> usize {
    let Some(quote) = source.as_bytes().get(cursor) else {
        return cursor;
    };

    if matches!(*quote, b'\'' | b'"') {
        let value_start = cursor + 1;
        return source[value_start..tag_end]
            .find(*quote as char)
            .map_or(tag_end, |offset| value_start + offset + 1);
    }

    let mut index = cursor;
    while index < tag_end {
        let byte = source.as_bytes()[index];
        if byte.is_ascii_whitespace() || matches!(byte, b'>' | b'/') {
            break;
        }
        index += 1;
    }
    index
}

fn skip_ascii_whitespace(source: &str, mut cursor: usize, end: usize) -> usize {
    while cursor < end && source.as_bytes()[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    cursor
}

fn consume_name(source: &str, start: usize) -> usize {
    let mut end = start;
    while end < source.len() && is_name_byte(source.as_bytes()[end]) {
        end += 1;
    }
    end
}

fn consume_attribute_name(source: &str, start: usize, tag_end: usize) -> usize {
    let mut end = start;
    while end < tag_end {
        let byte = source.as_bytes()[end];
        if byte.is_ascii_whitespace() || matches!(byte, b'=' | b'>' | b'/') {
            break;
        }
        end += 1;
    }
    end
}

fn find_tag_close(source: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    let mut index = start;

    while index < source.len() {
        let byte = source.as_bytes()[index];

        match quote {
            Some(quote_byte) if byte == quote_byte => quote = None,
            Some(_) => {}
            None if matches!(byte, b'\'' | b'"') => quote = Some(byte),
            None if byte == b'>' => return Some(index + 1),
            None => {}
        }

        index += 1;
    }

    None
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.')
}

fn is_void_element(local_name: &str) -> bool {
    matches!(
        local_name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}
