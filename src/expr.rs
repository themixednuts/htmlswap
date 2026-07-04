use std::fmt;

use compact_str::CompactString;

use crate::source::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateString {
    pub raw: CompactString,
    pub segments: Vec<TemplateSegment>,
    pub span: Option<Span>,
}

impl TemplateString {
    #[must_use]
    pub fn new(
        raw: impl Into<CompactString>,
        segments: Vec<TemplateSegment>,
        span: Option<Span>,
    ) -> Self {
        Self {
            raw: raw.into(),
            segments,
            span,
        }
    }

    pub fn expressions(&self) -> impl Iterator<Item = &Expr> {
        self.segments.iter().filter_map(|segment| match segment {
            TemplateSegment::Literal(_) => None,
            TemplateSegment::Expression(expression) => Some(expression),
        })
    }

    #[must_use]
    pub fn single_expression(&self) -> Option<&Expr> {
        match self.segments.as_slice() {
            [TemplateSegment::Expression(expression)] => Some(expression),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateSegment {
    Literal(CompactString),
    Expression(Expr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Path(Vec<CompactString>),
    Member {
        object: Box<Expr>,
        property: CompactString,
    },
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        arguments: Vec<Expr>,
    },
    Binary {
        left: Box<Expr>,
        operator: CompactString,
        right: Box<Expr>,
    },
    Logical {
        left: Box<Expr>,
        operator: CompactString,
        right: Box<Expr>,
    },
    Conditional {
        test: Box<Expr>,
        consequent: Box<Expr>,
        alternate: Box<Expr>,
    },
    TemplateLiteral {
        segments: Vec<TemplateSegment>,
    },
    Array(Vec<Expr>),
    Object(Vec<ObjectEntry>),
    Literal(ExprLiteral),
    Opaque(CompactString),
}

impl Expr {
    #[must_use]
    pub fn path(segments: impl IntoIterator<Item = impl Into<CompactString>>) -> Self {
        Self::Path(segments.into_iter().map(Into::into).collect())
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(segments) => write!(
                f,
                "{}",
                segments
                    .iter()
                    .map(AsRef::as_ref)
                    .collect::<Vec<&str>>()
                    .join(".")
            ),
            Self::Member { object, property } => write!(f, "{object}.{property}"),
            Self::Index { object, index } => write!(f, "{object}[{index}]"),
            Self::Call { callee, arguments } => write!(
                f,
                "{callee}({})",
                arguments
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Binary {
                left,
                operator,
                right,
            }
            | Self::Logical {
                left,
                operator,
                right,
            } => write!(f, "{left} {operator} {right}"),
            Self::Conditional {
                test,
                consequent,
                alternate,
            } => write!(f, "{test} ? {consequent} : {alternate}"),
            Self::TemplateLiteral { segments } => {
                f.write_str("`")?;
                for segment in segments {
                    match segment {
                        TemplateSegment::Literal(value) => f.write_str(value)?,
                        TemplateSegment::Expression(expression) => write!(f, "${{{expression}}}")?,
                    }
                }
                f.write_str("`")
            }
            Self::Array(items) => write!(
                f,
                "[{}]",
                items
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Object(entries) => write!(
                f,
                "{{ {} }}",
                entries
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Literal(value) => write!(f, "{value}"),
            Self::Opaque(value) => write!(f, "{value}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectEntry {
    pub key: CompactString,
    pub value: Expr,
}

impl ObjectEntry {
    #[must_use]
    pub fn new(key: impl Into<CompactString>, value: Expr) -> Self {
        Self {
            key: key.into(),
            value,
        }
    }
}

impl fmt::Display for ObjectEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.key, self.value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprLiteral {
    String(CompactString),
    Bool(bool),
    Number(CompactString),
    Null,
}

impl fmt::Display for ExprLiteral {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(value) => write!(f, "{value:?}"),
            Self::Bool(value) => write!(f, "{value}"),
            Self::Number(value) => write!(f, "{value}"),
            Self::Null => write!(f, "null"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingPattern {
    pub name: CompactString,
    pub span: Option<Span>,
}

impl BindingPattern {
    #[must_use]
    pub fn new(name: impl Into<CompactString>, span: Option<Span>) -> Self {
        Self {
            name: name.into(),
            span,
        }
    }
}
