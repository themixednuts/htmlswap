use arcstr::ArcStr;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompileAssets {
    pub stylesheets: Vec<SourceAsset>,
    pub scripts: Vec<SourceAsset>,
}

impl CompileAssets {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_stylesheet(
        mut self,
        name: impl Into<Option<String>>,
        contents: impl Into<ArcStr>,
    ) -> Self {
        self.stylesheets.push(SourceAsset::new(name, contents));
        self
    }

    #[must_use]
    pub fn with_script(
        mut self,
        name: impl Into<Option<String>>,
        contents: impl Into<ArcStr>,
    ) -> Self {
        self.scripts.push(SourceAsset::new(name, contents));
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAsset {
    pub name: Option<String>,
    pub contents: ArcStr,
}

impl SourceAsset {
    #[must_use]
    pub fn new(name: impl Into<Option<String>>, contents: impl Into<ArcStr>) -> Self {
        Self {
            name: name.into(),
            contents: contents.into(),
        }
    }
}
