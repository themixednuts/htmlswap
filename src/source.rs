use arcstr::{ArcStr, Substr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(u32);

impl SourceId {
    #[must_use]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    #[must_use]
    pub const fn primary() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Html,
    Css,
    JavaScript,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub source: SourceId,
    pub start: usize,
    pub end: usize,
}

impl Span {
    #[must_use]
    pub const fn new(source: SourceId, start: usize, end: usize) -> Self {
        Self { source, start, end }
    }

    #[must_use]
    pub const fn primary(start: usize, end: usize) -> Self {
        Self::new(SourceId::primary(), start, end)
    }

    #[must_use]
    pub fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.start >= self.end
    }

    #[must_use]
    pub const fn with_source(self, source: SourceId) -> Self {
        Self {
            source,
            start: self.start,
            end: self.end,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GeneratedSpan {
    pub start: usize,
    pub end: usize,
}

impl GeneratedSpan {
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    #[must_use]
    pub fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.start >= self.end
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMapping {
    pub generated: GeneratedSpan,
    pub original: Span,
    pub kind: SourceMappingKind,
}

impl SourceMapping {
    #[must_use]
    pub const fn new(generated: GeneratedSpan, original: Span, kind: SourceMappingKind) -> Self {
        Self {
            generated,
            original,
            kind,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceMappingKind {
    Element,
    Attribute,
    Text,
    Style,
    Action,
    Comment,
    State,
    Raw,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GeneratedSourceMap {
    mappings: Vec<SourceMapping>,
}

impl GeneratedSourceMap {
    #[must_use]
    pub fn new(mappings: Vec<SourceMapping>) -> Self {
        Self { mappings }
    }

    pub fn push(&mut self, mapping: SourceMapping) {
        self.mappings.push(mapping);
    }

    #[must_use]
    pub fn mappings(&self) -> &[SourceMapping] {
        &self.mappings
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mappings.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    id: SourceId,
    name: Option<String>,
    kind: SourceKind,
    text: ArcStr,
    line_starts: Vec<usize>,
}

impl SourceFile {
    #[must_use]
    pub fn new(
        id: SourceId,
        kind: SourceKind,
        name: impl Into<Option<String>>,
        text: impl Into<ArcStr>,
    ) -> Self {
        let text = text.into();
        let line_starts = line_starts(&text);
        Self {
            id,
            name: name.into(),
            kind,
            text,
            line_starts,
        }
    }

    #[must_use]
    pub const fn id(&self) -> SourceId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    #[must_use]
    pub const fn kind(&self) -> SourceKind {
        self.kind
    }

    #[must_use]
    pub fn text(&self) -> &str {
        self.text.as_str()
    }

    #[must_use]
    pub fn buffer(&self) -> &ArcStr {
        &self.text
    }

    #[must_use]
    pub fn line_column(&self, offset: usize) -> Option<LineColumn> {
        if offset > self.text.len() {
            return None;
        }

        let line_index = match self.line_starts.binary_search(&offset) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        };
        let line_start = *self.line_starts.get(line_index)?;
        Some(LineColumn {
            line: line_index + 1,
            column: offset.saturating_sub(line_start) + 1,
        })
    }

    #[must_use]
    pub fn slice(&self, span: Span) -> Option<&str> {
        if span.source != self.id || span.start > span.end || span.end > self.text.len() {
            return None;
        }

        self.text.get(span.start..span.end)
    }

    #[must_use]
    pub fn substr(&self, span: Span) -> Option<Substr> {
        if span.source != self.id
            || span.start > span.end
            || span.end > self.text.len()
            || span.end > u32::MAX as usize
        {
            return None;
        }

        Some(self.text.substr(span.start..span.end))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineColumn {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_file(
        &mut self,
        kind: SourceKind,
        name: impl Into<Option<String>>,
        text: impl Into<ArcStr>,
    ) -> SourceId {
        let id = SourceId::new(self.files.len() as u32);
        self.files.push(SourceFile::new(id, kind, name, text));
        id
    }

    #[must_use]
    pub fn file(&self, id: SourceId) -> Option<&SourceFile> {
        self.files.get(id.index()).filter(|file| file.id == id)
    }

    #[must_use]
    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }

    #[must_use]
    pub fn source_text(&self, span: Span) -> Option<&str> {
        self.file(span.source)?.slice(span)
    }

    #[must_use]
    pub fn source_substr(&self, span: Span) -> Option<Substr> {
        self.file(span.source)?.substr(span)
    }
}

fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(
        text.bytes()
            .enumerate()
            .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
    );
    starts
}
