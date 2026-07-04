use compact_str::CompactString;

use crate::source::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlDocument {
    pub nodes: Vec<HtmlNode>,
}

impl HtmlDocument {
    #[must_use]
    pub fn new(nodes: Vec<HtmlNode>) -> Self {
        Self { nodes }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HtmlNode {
    Element(HtmlElement),
    Text(HtmlText),
    Comment(HtmlComment),
}

impl HtmlNode {
    #[must_use]
    pub fn span(&self) -> Option<Span> {
        match self {
            Self::Element(element) => element.span,
            Self::Text(text) => text.span,
            Self::Comment(comment) => comment.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlElement {
    pub name: HtmlName,
    pub attributes: Vec<HtmlAttribute>,
    pub children: Vec<HtmlNode>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlAttribute {
    pub name: HtmlName,
    pub value: CompactString,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlText {
    pub value: String,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlComment {
    pub value: String,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HtmlName {
    pub prefix: Option<CompactString>,
    pub namespace: Option<CompactString>,
    pub local: CompactString,
}

impl HtmlName {
    #[must_use]
    pub fn new(
        prefix: Option<impl Into<CompactString>>,
        namespace: Option<impl Into<CompactString>>,
        local: impl Into<CompactString>,
    ) -> Self {
        Self {
            prefix: prefix.map(Into::into),
            namespace: namespace.map(Into::into),
            local: local.into(),
        }
    }

    #[must_use]
    pub fn local(&self) -> &str {
        &self.local
    }

    #[must_use]
    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }
}
