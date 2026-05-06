//! Project files: the user-authored description of an audiovisual piece.

mod schema;

pub use schema::{NodeSpec, ParamValue, Project};

use std::path::Path;

use anyhow::{Context, Result};

impl Project {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading project {}", path.display()))?;
        let project: Project = ron::from_str(&text)
            .with_context(|| format!("parsing project {}", path.display()))?;
        Ok(project)
    }
}
