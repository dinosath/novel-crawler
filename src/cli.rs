use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// lncrawl — Lightnovel Crawler (Rust edition)
///
/// Download light novels from 400+ online sources and export them as
/// EPUB, JSON, or plain text.
#[derive(Parser, Debug)]
#[command(
    name = "lncrawl",
    version,
    about = "Download light novels from online sources",
    long_about = None,
    propagate_version = true,
)]
pub struct Cli {
    /// Verbosity: -l = warn, -ll = info, -lll = debug
    #[arg(short = 'l', long = "verbose", action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Path to a config file
    #[arg(short = 'c', long, value_name = "FILE", global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Show the current version
    Version,

    /// List or manage supported sources
    #[command(subcommand)]
    Sources(SourcesCmd),

    /// Crawl and download a novel from its URL
    Crawl(CrawlArgs),

    /// Search for novels across supported sources
    Search(SearchArgs),
}

// ── Sources subcommand ───────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum SourcesCmd {
    /// List all supported sources (optionally filtered by a regex)
    List {
        /// Filter pattern (regex, case-insensitive) applied to name and URL
        filter: Option<String>,
    },
}

// ── Crawl arguments ──────────────────────────────────────────────────────────

#[derive(Args, Debug, Default)]
pub struct CrawlArgs {
    /// Novel page URL (prompted interactively if omitted)
    pub url: Option<String>,

    /// Download ALL available chapters
    #[arg(long)]
    pub all: bool,

    /// Download first N chapters
    #[arg(long, value_name = "N", conflicts_with_all = ["last", "range", "chapters"])]
    pub first: Option<usize>,

    /// Download last N chapters
    #[arg(long, value_name = "N", conflicts_with_all = ["first", "range", "chapters"])]
    pub last: Option<usize>,

    /// Download chapters in the index range FROM..=TO (1-based, inclusive)
    #[arg(long, value_names = ["FROM", "TO"], num_args = 2,
          conflicts_with_all = ["first", "last", "chapters"])]
    pub range: Option<Vec<usize>>,

    /// Download specific chapter URLs
    #[arg(long, value_name = "URL", num_args = 1..)]
    pub chapters: Option<Vec<String>>,

    /// Download specific volume numbers
    #[arg(long, value_name = "N", num_args = 1..)]
    pub volumes: Option<Vec<usize>>,

    /// Output formats (comma-separated: json, epub, txt). Default: all
    #[arg(short = 'f', long, value_name = "FORMAT", value_delimiter = ',')]
    pub format: Vec<String>,

    /// Output directory (default: ./Lightnovels/<novel-title>/)
    #[arg(short = 'o', long, value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// Override the output file stem (default: inferred from title + chapter range)
    #[arg(long, value_name = "NAME")]
    pub filename: Option<String>,

    /// Only use this filename stem (skip appending the chapter range)
    #[arg(long)]
    pub filename_only: bool,

    /// Force-replace an existing output directory
    #[arg(long)]
    pub force: bool,

    /// Skip downloading if the output directory already exists
    #[arg(short = 'i', long)]
    pub ignore: bool,

    /// Suppress all interactive prompts and use defaults
    #[arg(long)]
    pub suppress: bool,

    /// Pack all chapters into a single output file
    #[arg(long)]
    pub single: bool,

    /// Produce one output file per volume
    #[arg(long, conflicts_with = "single")]
    pub multi: bool,

    /// Skip downloading chapter cover images
    #[arg(long)]
    pub ignore_images: bool,

    /// Resume a previously interrupted download
    #[arg(long, value_name = "NAME_OR_URL")]
    pub resume: Option<String>,

    /// Maximum concurrent chapter downloads (default: 5)
    #[arg(long, default_value = "5", value_name = "N")]
    pub workers: usize,
}

impl CrawlArgs {
    /// Determine which chapter indices (0-based) to download, given the full list.
    pub fn select_indices(&self, total: usize) -> Vec<usize> {
        if self.all || (self.first.is_none()
            && self.last.is_none()
            && self.range.is_none()
            && self.volumes.is_none()
            && self.chapters.is_none())
        {
            return (0..total).collect();
        }
        if let Some(n) = self.first {
            return (0..n.min(total)).collect();
        }
        if let Some(n) = self.last {
            let start = total.saturating_sub(n);
            return (start..total).collect();
        }
        if let Some(r) = &self.range {
            let from = r[0].saturating_sub(1);
            let to = r[1].min(total);
            return (from..to).collect();
        }
        (0..total).collect()
    }

    /// Resolve the output formats, defaulting to all three when none given.
    pub fn resolved_formats(&self) -> Vec<crate::models::OutputFormat> {
        if self.format.is_empty() {
            return crate::models::OutputFormat::all_formats();
        }
        self.format
            .iter()
            .filter_map(|s| crate::models::OutputFormat::from_str(s))
            .collect()
    }
}

// ── Search arguments ─────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Search query string
    pub query: String,

    /// Filter sources by regex (applied to name and URL)
    #[arg(short = 'x', long, value_name = "REGEX")]
    pub sources: Option<String>,

    /// Maximum results per source (default: 10)
    #[arg(short = 'l', long, default_value = "10", value_name = "N")]
    pub limit: usize,
}
