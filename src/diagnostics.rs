use std::fmt;

use crate::source::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Info => f.write_str("info"),
            Self::Warning => f.write_str("warning"),
            Self::Error => f.write_str("error"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Option<Span>,
}

impl Diagnostic {
    #[must_use]
    pub fn new(severity: Severity, message: impl Into<String>, span: Option<Span>) -> Self {
        Self {
            severity,
            message: message.into(),
            span,
        }
    }

    #[must_use]
    pub fn info(message: impl Into<String>, span: Option<Span>) -> Self {
        Self::new(Severity::Info, message, span)
    }

    #[must_use]
    pub fn warning(message: impl Into<String>, span: Option<Span>) -> Self {
        Self::new(Severity::Warning, message, span)
    }

    #[must_use]
    pub fn error(message: impl Into<String>, span: Option<Span>) -> Self {
        Self::new(Severity::Error, message, span)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.items.push(diagnostic);
    }

    pub fn extend(&mut self, diagnostics: Diagnostics) {
        self.items.extend(diagnostics);
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.items
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter()
    }

    #[must_use]
    pub fn as_slice(&self) -> &[Diagnostic] {
        &self.items
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.items
    }
}

impl FromIterator<Diagnostic> for Diagnostics {
    fn from_iter<T: IntoIterator<Item = Diagnostic>>(iter: T) -> Self {
        Self {
            items: iter.into_iter().collect(),
        }
    }
}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<'a> IntoIterator for &'a Diagnostics {
    type Item = &'a Diagnostic;
    type IntoIter = std::slice::Iter<'a, Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compilation<T> {
    pub value: T,
    pub diagnostics: Diagnostics,
}

impl<T> Compilation<T> {
    #[must_use]
    pub fn new(value: T, diagnostics: Diagnostics) -> Self {
        Self { value, diagnostics }
    }

    #[must_use]
    pub fn clean(value: T) -> Self {
        Self {
            value,
            diagnostics: Diagnostics::new(),
        }
    }

    #[must_use]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Compilation<U> {
        Compilation {
            value: f(self.value),
            diagnostics: self.diagnostics,
        }
    }
}
