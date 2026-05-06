//! Project files: the user-authored description of an audiovisual piece.

mod schema;

pub use schema::{NodeSpec, ParamValue, Project, ToneMap};

use std::path::Path;

use anyhow::{Context, Result};

impl Project {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading project {}", path.display()))?;
        let mut project: Project =
            ron::from_str(&text).with_context(|| format!("parsing project {}", path.display()))?;
        project.source_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| Path::new(".").to_path_buf());
        Ok(project)
    }
}
