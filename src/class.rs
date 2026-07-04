use compact_str::CompactString;

use crate::ir::{HtmlDocument, HtmlElement, HtmlNode};
use crate::source::{SourceId, Span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassCandidate {
    pub name: CompactString,
    pub span: Option<Span>,
}

impl ClassCandidate {
    #[must_use]
    pub fn new(name: impl Into<CompactString>, span: Option<Span>) -> Self {
        Self {
            name: name.into(),
            span,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassIndex {
    candidates: Vec<ClassCandidate>,
}

impl ClassIndex {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn collect(document: &HtmlDocument, source: SourceId) -> Self {
        let mut index = Self::new();
        for node in &document.nodes {
            collect_node_classes(node, source, &mut index);
        }
        index
    }

    pub fn push(&mut self, candidate: ClassCandidate) {
        self.candidates.push(candidate);
    }

    #[must_use]
    pub fn candidates(&self) -> &[ClassCandidate] {
        &self.candidates
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.candidates
            .iter()
            .map(|candidate| candidate.name.as_str())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }
}

fn collect_node_classes(node: &HtmlNode, source: SourceId, index: &mut ClassIndex) {
    let HtmlNode::Element(element) = node else {
        return;
    };

    collect_element_classes(element, source, index);
    for child in &element.children {
        collect_node_classes(child, source, index);
    }
}

fn collect_element_classes(element: &HtmlElement, source: SourceId, index: &mut ClassIndex) {
    for attribute in &element.attributes {
        if !attribute.name.local().eq_ignore_ascii_case("class") {
            continue;
        }

        let span = attribute.span.filter(|span| span.source == source);
        for name in attribute.value.split_ascii_whitespace() {
            index.push(ClassCandidate::new(name, span));
        }
    }
}
