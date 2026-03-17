//! Plain-text output.

use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::models::{Chapter, Novel};

pub fn write(novel: &Novel, chapters: &[&Chapter], path: &Path) -> Result<()> {
    let mut file = std::fs::File::create(path)?;

    // Header
    writeln!(file, "{}", novel.title)?;
    if let Some(author) = &novel.author {
        writeln!(file, "Author: {}", author)?;
    }
    writeln!(file, "Source: {}", novel.url)?;
    if let Some(syn) = &novel.synopsis {
        writeln!(file, "\n{}\n", syn)?;
    }
    writeln!(file, "{}", "=".repeat(72))?;

    for chapter in chapters {
        writeln!(file)?;
        writeln!(file, "{}", chapter.title)?;
        writeln!(file, "{}", "-".repeat(chapter.title.len().min(72)))?;
        writeln!(file)?;
        write!(file, "{}", chapter.plain_text())?;
        writeln!(file)?;
        writeln!(file, "{}", "=".repeat(72))?;
    }

    Ok(())
}
