pub mod epub;
pub mod json;
pub mod txt;

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::models::{Novel, OutputFormat};

/// Write `novel` to `out_dir` in all requested `formats`.
///
/// Returns a list of paths that were created.
pub async fn write_outputs(
    novel: &Novel,
    chapters: &[&crate::models::Chapter],
    out_dir: &Path,
    formats: &[OutputFormat],
    filename_stem: &str,
) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(out_dir)?;
    let mut created = Vec::new();

    for format in formats {
        let path = out_dir.join(format!("{}.{}", filename_stem, format.extension()));
        match format {
            OutputFormat::Json => json::write(novel, chapters, &path)?,
            OutputFormat::Epub => epub::write(novel, chapters, &path)?,
            OutputFormat::Txt => txt::write(novel, chapters, &path)?,
        }
        created.push(path);
    }
    Ok(created)
}

/// Build a safe filesystem stem from the novel title (no chapter range).
pub fn build_filename(novel: &Novel) -> String {
    novel
        .title
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// Rename any files in `out_dir` whose stem starts with `stem` followed by `_c`
/// (old chapter-range naming) to the plain `stem.<ext>` name.
pub fn rename_old_chapter_files(out_dir: &Path, stem: &str) {
    let prefix = format!("{}_c", stem);
    if let Ok(entries) = std::fs::read_dir(out_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let file_stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if file_stem.starts_with(&prefix) {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    let new_path = out_dir.join(format!("{}.{}", stem, ext));
                    // Only rename if target doesn't already exist to avoid overwriting.
                    if !new_path.exists() {
                        let _ = std::fs::rename(&path, &new_path);
                    }
                }
            }
        }
    }
}
