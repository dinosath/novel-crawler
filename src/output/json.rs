//! JSON output — dumps the full novel struct as pretty-printed JSON.

use anyhow::Result;
use std::path::Path;

use crate::models::{Chapter, Novel};

pub fn write(novel: &Novel, chapters: &[&Chapter], path: &Path) -> Result<()> {
    // Build a combined struct that includes only the selected chapters
    let mut out = novel.clone();
    out.chapters = chapters.iter().map(|c| (*c).clone()).collect();

    let json = serde_json::to_string_pretty(&out)?;
    std::fs::write(path, json)?;
    Ok(())
}
