use std::fmt;

use crate::diagnostics::Compilation;
use crate::resource::ResourceResolver;
use crate::source::{SourceId, SourceMap};

pub trait ModuleGraphProvider: Send + Sync {
    fn graph(&self, input: ModuleGraphInput<'_>) -> Compilation<ModuleGraph>;
}

#[derive(Clone, Copy)]
pub struct ModuleGraphInput<'a> {
    pub entries: &'a [SourceId],
    pub sources: &'a SourceMap,
    pub resources: &'a dyn ResourceResolver,
    pub options: &'a ModuleGraphOptions,
}

impl fmt::Debug for ModuleGraphInput<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModuleGraphInput")
            .field("entries", &self.entries)
            .field("sources", &self.sources)
            .field("options", &self.options)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModuleGraphOptions {
    pub code_splitting: bool,
}

impl ModuleGraphOptions {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            code_splitting: false,
        }
    }

    #[must_use]
    pub const fn with_code_splitting(mut self, code_splitting: bool) -> Self {
        self.code_splitting = code_splitting;
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModuleGraph {
    pub modules: Vec<ModuleNode>,
    pub edges: Vec<ModuleEdge>,
    pub chunks: Vec<ModuleChunk>,
}

impl ModuleGraph {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty() && self.edges.is_empty() && self.chunks.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModuleNodeId(u32);

impl ModuleNodeId {
    #[must_use]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleNode {
    pub id: ModuleNodeId,
    pub source: SourceId,
    pub specifier: Option<String>,
}

impl ModuleNode {
    #[must_use]
    pub fn new(id: ModuleNodeId, source: SourceId, specifier: Option<String>) -> Self {
        Self {
            id,
            source,
            specifier,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleEdge {
    pub from: ModuleNodeId,
    pub to: ModuleNodeId,
    pub kind: ModuleEdgeKind,
}

impl ModuleEdge {
    #[must_use]
    pub const fn new(from: ModuleNodeId, to: ModuleNodeId, kind: ModuleEdgeKind) -> Self {
        Self { from, to, kind }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleEdgeKind {
    Import,
    DynamicImport,
    ReExport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleChunk {
    pub name: Option<String>,
    pub entry: Option<ModuleNodeId>,
    pub modules: Vec<ModuleNodeId>,
}

impl ModuleChunk {
    #[must_use]
    pub fn new(
        name: Option<String>,
        entry: Option<ModuleNodeId>,
        modules: Vec<ModuleNodeId>,
    ) -> Self {
        Self {
            name,
            entry,
            modules,
        }
    }
}
