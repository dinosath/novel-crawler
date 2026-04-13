mod cli;
mod crawler;
mod models;
mod output;
mod sources;

use anyhow::{bail, Context, Result};
use clap::Parser;
use colored::Colorize;
use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cli::{Cli, Commands, CrawlArgs, SearchArgs, SourcesCmd, UpdateArgs};
use crawler::{build_client, create_crawler, search_crawlers};
use models::{Chapter, Novel};
use output::{build_filename, write_outputs};
use sources::{filter_sources, SOURCES};

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    // Set log level from -l / -ll / -lll
    let _log_level = match cli.verbose {
        0 => "error",
        1 => "warn",
        2 => "info",
        _ => "debug",
    };

    match cli.command {
        None => run_interactive().await,
        Some(Commands::Version) => {
            println!("lncrawl {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(Commands::Sources(sub)) => run_sources(sub),
        Some(Commands::Crawl(args)) => run_crawl(args).await,
        Some(Commands::Search(args)) => run_search(args).await,
        Some(Commands::Update(args)) => run_update(args).await,
    }
}

// ── Interactive mode (no subcommand given) ────────────────────────────────────

async fn run_interactive() -> Result<()> {
    print_banner();

    println!("{}", "Enter a novel URL to crawl, or a search query:".cyan());

    let input: String = dialoguer::Input::new()
        .with_prompt("URL / query")
        .interact_text()?;
    let input = input.trim().to_string();

    if input.starts_with("http://") || input.starts_with("https://") {
        let args = CrawlArgs {
            url: Some(input),
            ..Default::default()
        };
        run_crawl(args).await
    } else {
        let args = SearchArgs {
            query: input,
            sources: None,
            limit: 10,
        };
        run_search(args).await
    }
}

fn print_banner() {
    println!(
        "{}",
        concat!(
            "  _                                       _ \n",
            " | |  _ __   ___ _ __ __ ___      _| |\n",
            " | | | '_ \\ / __| '__/ _` \\ \\ /\\ / / |\n",
            " | |_| | | | (__| | | (_| |\\ V  V /| |\n",
            " |___|_| |_|\\___|_|  \\__,_| \\_/\\_/ |_|\n",
            "\n  lightnovel-crawler-rs  (Rust Edition)\n",
        )
        .cyan()
    );
}

// ── sources list ──────────────────────────────────────────────────────────────

fn run_sources(cmd: SourcesCmd) -> Result<()> {
    match cmd {
        SourcesCmd::List { filter } => {
            let list: Vec<_> = match &filter {
                Some(pat) => filter_sources(pat),
                None => SOURCES.iter().collect(),
            };

            println!(
                "{:30} {:5} {:8} {}",
                "Name".bold(),
                "Lang".bold(),
                "Search".bold(),
                "URL".bold()
            );
            println!("{}", "-".repeat(80));
            for s in &list {
                let search_flag = if s.supports_search { "yes" } else { "no" };
                let mt = if s.is_machine_translated { " 🤖" } else { "" };
                println!(
                    "{:30} {:5} {:8} {}{}",
                    s.name, s.language, search_flag, s.base_url, mt
                );
            }
            println!("\n{} sources listed.", list.len());
        }
    }
    Ok(())
}

// ── crawl ─────────────────────────────────────────────────────────────────────

async fn run_crawl(mut args: CrawlArgs) -> Result<()> {
    // 1. Resolve URL
    let url = match args.url.take() {
        Some(u) => u,
        None => {
            if args.suppress {
                bail!("No URL provided and --suppress is active");
            }
            dialoguer::Input::<String>::new()
                .with_prompt("Novel URL")
                .interact_text()?
                .trim()
                .to_string()
        }
    };

    // 2. Find crawler
    let crawler = create_crawler(&url).with_context(|| {
        let host = url::Url::parse(&url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()))
            .unwrap_or_else(|| url.clone());
        format!(
            "No crawler found for '{}'. Run `lncrawl sources list` to see supported sites.",
            host
        )
    })?;

    let client = Arc::new(build_client()?);

    // 3. Fetch novel metadata + chapter list
    println!(
        "{} {} …",
        "Fetching novel info from".cyan(),
        crawler.name().yellow()
    );
    let mut novel = crawler
        .read_novel_info(&client, &url)
        .await
        .context("Failed to read novel info")?;

    print_novel_info(&novel);

    if novel.chapters.is_empty() {
        bail!("No chapters found. The URL may be incorrect or unsupported.");
    }

    // 4. Resolve output dir early so we can check what's already downloaded.
    let out_dir = resolve_output_dir(&novel, &args)?;
    let already_downloaded = if out_dir.exists() {
        load_downloaded_urls(&out_dir)
    } else {
        std::collections::HashSet::new()
    };
    // Count only chapters that are in the current (free) chapter list.
    let current_downloaded_count = novel
        .chapters
        .iter()
        .filter(|ch| already_downloaded.contains(&ch.url))
        .count();

    // Print download status below novel info.
    if current_downloaded_count > 0 {
        println!(
            "  {} {} downloaded, {} remaining",
            "📥".to_string(),
            format!("{current_downloaded_count}").green(),
            format!("{}", novel.chapters.len() - current_downloaded_count).yellow(),
        );
        println!();
    }

    // 5. Determine which chapters to download
    let mut indices = if args.all
        || args.first.is_some()
        || args.last.is_some()
        || args.range.is_some()
        || args.suppress
    {
        // CLI-specified: honour the range but skip already downloaded
        let all = args.select_indices(novel.chapters.len());
        all.into_iter()
            .filter(|&i| !already_downloaded.contains(&novel.chapters[i].url))
            .collect::<Vec<_>>()
    } else {
        prompt_chapter_selection(&novel, &already_downloaded)?
    };

    if indices.is_empty() {
        println!("{}", "No new chapters to download.".green());
        return Ok(());
    }
    println!("{} {} chapters to download.", "→".green(), indices.len());

    // 6. Handle existing output directory.
    if out_dir.exists() && args.force {
        std::fs::remove_dir_all(&out_dir)?;
    } else if out_dir.exists() && args.ignore {
        println!("{} Output already exists, skipping.", "⚠".yellow());
        return Ok(());
    }
    std::fs::create_dir_all(&out_dir)?;

    // 7. Download chapters with bounded concurrency
    let pb = ProgressBar::new(indices.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}",
            )
            .unwrap()
            .progress_chars("##-"),
    );

    let max_workers = args.workers.max(1).min(20);
    let novel_arc = Arc::new(tokio::sync::Mutex::new(novel.clone()));
    let crawler_arc = Arc::new(crawler);
    let abort = Arc::new(AtomicBool::new(false));

    futures::stream::iter(indices.clone())
        .map(|idx| {
            let client = Arc::clone(&client);
            let pb = pb.clone();
            let novel_arc = Arc::clone(&novel_arc);
            let crawler = Arc::clone(&crawler_arc);
            let abort = Arc::clone(&abort);
            async move {
                if abort.load(Ordering::Relaxed) {
                    pb.inc(1);
                    return;
                }
                let chapter = {
                    let n = novel_arc.lock().await;
                    n.chapters[idx].clone()
                };
                let mut chapter = chapter;
                pb.set_message(format!("Ch.{:04}", chapter.id));
                match crawler.read_chapter(&client, &mut chapter).await {
                    Ok(()) => {
                        let mut n = novel_arc.lock().await;
                        n.chapters[idx] = chapter;
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("did not return JSON") {
                            abort.store(true, Ordering::Relaxed);
                            pb.println(format!(
                                "{} Non-JSON response from server — stopping chapter downloads.",
                                "error:".red().bold()
                            ));
                        } else {
                            pb.println(format!(
                                "{} chapter {}: {}",
                                "warn:".yellow(),
                                idx + 1,
                                e
                            ));
                        }
                    }
                }
                pb.inc(1);
            }
        })
        .buffer_unordered(max_workers)
        .collect::<Vec<_>>()
        .await;

    pb.finish_with_message("download complete");

    // 8. Gather selected chapters (only those with content)
    let novel = Arc::try_unwrap(novel_arc)
        .expect("no other references")
        .into_inner();
    let selected_chapters: Vec<&Chapter> = indices
        .iter()
        .map(|&i| &novel.chapters[i])
        .filter(|c| c.body.is_some())
        .collect();

    if selected_chapters.is_empty() {
        println!("{}", "No chapters were downloaded successfully.".yellow());
        return Ok(());
    }

    // Merge with already-downloaded chapters from existing JSON if resuming.
    let merged_novel;
    let write_novel;
    let all_chapters: Vec<&Chapter>;
    let existing = load_existing_novel(&out_dir);
    if let Some(mut prev) = existing {
        // Overwrite any chapter that was just downloaded.
        let new_ids: std::collections::HashSet<usize> =
            selected_chapters.iter().map(|c| c.id).collect();
        prev.chapters.retain(|c| !new_ids.contains(&c.id));
        for c in &selected_chapters {
            prev.chapters.push((*c).clone());
        }
        prev.chapters.sort_by_key(|c| c.id);
        merged_novel = prev;
        all_chapters = merged_novel.chapters.iter().filter(|c| c.body.is_some()).collect();
        write_novel = &merged_novel;
    } else {
        all_chapters = selected_chapters;
        write_novel = &novel;
    }

    // 9. Write output files
    let formats = if !args.format.is_empty() || args.suppress {
        args.resolved_formats()
    } else if let Some(detected) = detect_existing_formats(&out_dir) {
        detected
    } else {
        prompt_format_selection()?
    };
    let stem = args.filename.clone().unwrap_or_else(|| build_filename(write_novel));
    // Rename any old files that have the chapter-range suffix.
    output::rename_old_chapter_files(&out_dir, &stem);
    let created = write_outputs(write_novel, &all_chapters, &out_dir, &formats, &stem).await?;

    println!("\n{}", "Files created:".green().bold());
    for path in &created {
        println!("  {}", path.display());
    }
    Ok(())
}

fn prompt_chapter_selection(
    novel: &Novel,
    already_downloaded: &std::collections::HashSet<String>,
) -> Result<Vec<usize>> {
    let total = novel.chapters.len();
    let missing: Vec<usize> = (0..total)
        .filter(|&i| !already_downloaded.contains(&novel.chapters[i].url))
        .collect();
    let choices = vec![
        format!(
            "Download missing {} chapters ({} already have)",
            missing.len(),
            already_downloaded.len()
        ),
        format!("All {} chapters (re-download existing)", total),
        "First N chapters".to_string(),
        "Last N chapters".to_string(),
        "Index range (FROM  TO)".to_string(),
    ];
    let selection = dialoguer::Select::new()
        .with_prompt("Which chapters to download?")
        .items(&choices)
        .default(0)
        .interact()?;

    match selection {
        0 => Ok(missing),
        1 => Ok((0..total).collect()),
        2 => {
            let n: usize = dialoguer::Input::new()
                .with_prompt("How many chapters from the start?")
                .default(10usize)
                .interact_text()?;
            Ok((0..n.min(total))
                .filter(|&i| !already_downloaded.contains(&novel.chapters[i].url))
                .collect())
        }
        3 => {
            let n: usize = dialoguer::Input::new()
                .with_prompt("How many chapters from the end?")
                .default(10usize)
                .interact_text()?;
            let start = total.saturating_sub(n);
            Ok((start..total)
                .filter(|&i| !already_downloaded.contains(&novel.chapters[i].url))
                .collect())
        }
        4 => {
            let from: usize = dialoguer::Input::new()
                .with_prompt("From chapter index (1-based)")
                .default(1usize)
                .interact_text()?;
            let to: usize = dialoguer::Input::new()
                .with_prompt("To chapter index (1-based, inclusive)")
                .default(total)
                .interact_text()?;
            Ok((from.saturating_sub(1)..to.min(total))
                .filter(|&i| !already_downloaded.contains(&novel.chapters[i].url))
                .collect())
        }
        _ => Ok(missing),
    }
}

/// If `out_dir` contains files of exactly one known format type (json/epub/txt),
/// return that format automatically so the user isn't prompted again.
fn detect_existing_formats(out_dir: &std::path::Path) -> Option<Vec<models::OutputFormat>> {
    use models::OutputFormat;
    if !out_dir.exists() {
        return None;
    }
    let mut found: std::collections::HashSet<OutputFormat> = std::collections::HashSet::new();
    if let Ok(entries) = std::fs::read_dir(out_dir) {
        for entry in entries.flatten() {
            match entry.path().extension().and_then(|e| e.to_str()) {
                Some("json") => { found.insert(OutputFormat::Json); }
                Some("epub") => { found.insert(OutputFormat::Epub); }
                Some("txt")  => { found.insert(OutputFormat::Txt); }
                _ => {}
            }
        }
    }
    if found.len() == 1 {
        Some(found.into_iter().collect())
    } else {
        None
    }
}

fn prompt_format_selection() -> Result<Vec<models::OutputFormat>> {
    use dialoguer::MultiSelect;
    use models::OutputFormat;
    let options = ["JSON (.json)", "EPUB (.epub)", "TXT (.txt)"];
    let formats = [OutputFormat::Json, OutputFormat::Epub, OutputFormat::Txt];
    let chosen = MultiSelect::new()
        .with_prompt("Output formats (space to toggle, enter to confirm)")
        .items(&options)
        .defaults(&[true, false, false]) // JSON ticked by default
        .interact()?;
    if chosen.is_empty() {
        // Nothing selected → default to JSON
        return Ok(vec![OutputFormat::Json]);
    }
    Ok(chosen.into_iter().map(|i| formats[i].clone()).collect())
}

fn print_novel_info(novel: &Novel) {
    println!("\n{}", "═".repeat(60));
    println!("  {}", novel.title.bold().white());
    if let Some(a) = &novel.author {
        println!("  {} {}", "by".dimmed(), a.yellow());
    }
    println!(
        "  {} chapters{}",
        novel.chapters.len(),
        if novel.volumes.is_empty() {
            String::new()
        } else {
            format!(" in {} volumes", novel.volumes.len())
        }
    );
    if let Some(syn) = &novel.synopsis {
        let preview: String = syn.chars().take(220).collect();
        let ellipsis = if syn.len() > 220 { "…" } else { "" };
        println!("\n  {}{}", preview.dimmed(), ellipsis.dimmed());
    }
    println!("{}\n", "═".repeat(60));
}

/// Return the set of chapter URLs that have already been downloaded (body present)
/// by reading any .json file found in `out_dir`.
fn load_downloaded_urls(out_dir: &std::path::Path) -> std::collections::HashSet<String> {
    load_existing_novel(out_dir)
        .map(|n| {
            n.chapters
                .into_iter()
                .filter(|c| c.body.is_some())
                .map(|c| c.url.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Try to load a previously saved Novel from a JSON file in `out_dir`.
fn load_existing_novel(out_dir: &std::path::Path) -> Option<Novel> {
    let entry = std::fs::read_dir(out_dir).ok()?.flatten().find(|e| {
        e.path()
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| x == "json")
            .unwrap_or(false)
    })?;
    let text = std::fs::read_to_string(entry.path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn resolve_output_dir(novel: &Novel, args: &CrawlArgs) -> Result<PathBuf> {    if let Some(p) = &args.output {
        return Ok(p.clone());
    }
    let safe: String = novel
        .title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe = safe.trim().replace(' ', "_");
    Ok(PathBuf::from("Lightnovels").join(safe))
}

// ── update ─────────────────────────────────────────────────────────────────────

async fn run_update(args: UpdateArgs) -> Result<()> {
    let base_dir = args.output.clone().unwrap_or_else(|| PathBuf::from("Lightnovels"));
    if !base_dir.exists() {
        bail!(
            "Directory '{}' does not exist. Nothing to update.",
            base_dir.display()
        );
    }

    // Discover novel directories that contain a .json file
    let mut novels_to_update: Vec<(PathBuf, Novel)> = Vec::new();
    for entry in std::fs::read_dir(&base_dir)?.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        if let Some(novel) = load_existing_novel(&dir) {
            if novel.url.is_empty() {
                continue;
            }
            // Apply optional filter
            if let Some(ref filter) = args.filter {
                let filter_lower = filter.to_lowercase();
                let dir_name = dir
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                if !dir_name.contains(&filter_lower)
                    && !novel.title.to_lowercase().contains(&filter_lower)
                {
                    continue;
                }
            }
            novels_to_update.push((dir, novel));
        }
    }

    if novels_to_update.is_empty() {
        println!("{}", "No novels found to update.".yellow());
        return Ok(());
    }

    println!(
        "{} Found {} novel(s) to check for updates.\n",
        "→".cyan(),
        novels_to_update.len()
    );

    let client = Arc::new(build_client()?);

    for (out_dir, existing_novel) in &novels_to_update {
        println!(
            "{} Checking: {}",
            "→".cyan(),
            existing_novel.title.bold().white()
        );

        // Find crawler for this novel's URL
        let crawler = match create_crawler(&existing_novel.url) {
            Some(c) => c,
            None => {
                eprintln!(
                    "  {} No crawler for {}",
                    "skip:".yellow(),
                    existing_novel.url,
                );
                continue;
            }
        };

        // Fetch fresh chapter list
        let fresh_novel = match crawler.read_novel_info(&client, &existing_novel.url).await {
            Ok(n) => n,
            Err(e) => {
                eprintln!(
                    "  {} Failed to fetch info: {}",
                    "warn:".yellow(),
                    e
                );
                continue;
            }
        };

        // Determine which chapters are new
        let existing_urls: std::collections::HashSet<String> = existing_novel
            .chapters
            .iter()
            .filter(|c| c.body.is_some())
            .map(|c| c.url.clone())
            .collect();

        let new_indices: Vec<usize> = fresh_novel
            .chapters
            .iter()
            .enumerate()
            .filter(|(_, c)| !existing_urls.contains(&c.url))
            .map(|(i, _)| i)
            .collect();

        if new_indices.is_empty() {
            println!(
                "  {} Up to date ({} chapters)",
                "✓".green(),
                existing_novel.chapters.len()
            );
            continue;
        }

        println!(
            "  {} {} new chapter(s) found (had {}, now {})",
            "↓".green(),
            new_indices.len(),
            existing_novel.chapters.len(),
            fresh_novel.chapters.len()
        );

        // Download new chapters
        let pb = ProgressBar::new(new_indices.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "  {spinner:.green} [{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}",
                )
                .unwrap()
                .progress_chars("##-"),
        );

        let max_workers = args.workers.max(1).min(20);
        let novel_arc: Arc<tokio::sync::Mutex<Novel>> = Arc::new(tokio::sync::Mutex::new(fresh_novel.clone()));
        let crawler_arc: Arc<Box<dyn crawler::Crawler>> = Arc::new(crawler);
        let abort: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));

        futures::stream::iter(new_indices.clone())
            .map(|idx| {
                let client = Arc::clone(&client);
                let pb = pb.clone();
                let novel_arc = Arc::clone(&novel_arc);
                let crawler = Arc::clone(&crawler_arc);
                let abort = Arc::clone(&abort);
                async move {
                    if abort.load(Ordering::Relaxed) {
                        pb.inc(1);
                        return;
                    }
                    let chapter = {
                        let n = novel_arc.lock().await;
                        n.chapters[idx].clone()
                    };
                    let mut chapter = chapter;
                    pb.set_message(format!("Ch.{:04}", chapter.id));
                    match crawler.read_chapter(&client, &mut chapter).await {
                        Ok(()) => {
                            let mut n = novel_arc.lock().await;
                            n.chapters[idx] = chapter;
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            if msg.contains("did not return JSON") {
                                abort.store(true, Ordering::Relaxed);
                                pb.println(format!(
                                    "  {} Non-JSON response — stopping.",
                                    "error:".red().bold()
                                ));
                            } else {
                                pb.println(format!(
                                    "  {} chapter {}: {}",
                                    "warn:".yellow(),
                                    idx + 1,
                                    e
                                ));
                            }
                        }
                    }
                    pb.inc(1);
                }
            })
            .buffer_unordered(max_workers)
            .collect::<Vec<_>>()
            .await;

        pb.finish_with_message("done");

        // Merge new chapters into existing novel
        let fresh_novel = Arc::try_unwrap(novel_arc)
            .expect("no other references")
            .into_inner();

        let new_chapters: Vec<&Chapter> = new_indices
            .iter()
            .map(|&i| &fresh_novel.chapters[i])
            .filter(|c| c.body.is_some())
            .collect();

        if new_chapters.is_empty() {
            println!("  {} No new chapters downloaded successfully.", "⚠".yellow());
            continue;
        }

        // Merge: existing + new, sorted by id
        let mut merged = existing_novel.clone();
        merged.title = fresh_novel.title.clone();
        merged.author = fresh_novel.author.clone();
        merged.cover_url = fresh_novel.cover_url.clone();
        merged.synopsis = fresh_novel.synopsis.clone();
        merged.volumes = fresh_novel.volumes.clone();

        let new_ids: std::collections::HashSet<usize> =
            new_chapters.iter().map(|c| c.id).collect();
        merged.chapters.retain(|c| !new_ids.contains(&c.id));
        for c in &new_chapters {
            merged.chapters.push((*c).clone());
        }
        merged.chapters.sort_by_key(|c| c.id);

        // Determine output formats
        let formats = if !args.format.is_empty() {
            args.format
                .iter()
                .filter_map(|s| models::OutputFormat::from_str(s))
                .collect()
        } else if let Some(detected) = detect_existing_formats(out_dir) {
            detected
        } else {
            vec![models::OutputFormat::Json]
        };

        let stem = build_filename(&merged);
        output::rename_old_chapter_files(out_dir, &stem);
        let all_chapters: Vec<&Chapter> =
            merged.chapters.iter().filter(|c| c.body.is_some()).collect();
        let created = write_outputs(&merged, &all_chapters, out_dir, &formats, &stem).await?;

        println!(
            "  {} {} new chapters merged → {} total. Files updated:",
            "✓".green(),
            new_chapters.len(),
            all_chapters.len(),
        );
        for path in &created {
            println!("    {}", path.display());
        }
        println!();
    }

    println!("{}", "Update complete.".green().bold());
    Ok(())
}

// ── search ────────────────────────────────────────────────────────────────────

async fn run_search(args: SearchArgs) -> Result<()> {
    let client = Arc::new(build_client()?);

    let crawlers: Vec<Box<dyn crawler::Crawler>> = match &args.sources {
        Some(pat) => {
            let matching: Vec<String> = filter_sources(pat)
                .iter()
                .map(|s| s.base_url.to_string())
                .collect();
            search_crawlers()
                .into_iter()
                .filter(|c| {
                    c.base_urls()
                        .iter()
                        .any(|u| matching.iter().any(|m| m.contains(u)))
                })
                .collect()
        }
        None => search_crawlers(),
    };

    if crawlers.is_empty() {
        bail!("No crawlers match the given source filter.");
    }

    println!(
        "{} Searching {} source(s) for '{}'…",
        "→".cyan(),
        crawlers.len(),
        args.query.yellow()
    );

    let mut all_results: Vec<models::SearchResult> = Vec::new();
    for c in crawlers {
        match c.search(&client, &args.query).await {
            Ok(mut res) => {
                res.truncate(args.limit);
                all_results.extend(res);
            }
            Err(e) => {
                eprintln!("{} {}: {}", "warn:".yellow(), c.name(), e);
            }
        }
    }

    if all_results.is_empty() {
        println!("{}", "No results found.".yellow());
        return Ok(());
    }

    println!("\n{}", "Search Results:".bold().green());
    println!("{}", "─".repeat(60));
    for (i, r) in all_results.iter().enumerate() {
        println!(
            "  {}. {}  {}  [{}]",
            (i + 1).to_string().cyan(),
            r.title.white().bold(),
            r.author
                .as_deref()
                .map(|a| format!("by {}", a))
                .unwrap_or_default()
                .dimmed(),
            r.source_name.yellow()
        );
        println!("     {}", r.url.dimmed());
    }
    println!();

    // Offer to download one
    let mut choices: Vec<String> = all_results
        .iter()
        .enumerate()
        .map(|(i, r)| format!("{}. {} [{}]", i + 1, r.title, r.source_name))
        .collect();
    choices.push("Exit without downloading".to_string());

    let sel = dialoguer::Select::new()
        .with_prompt("Download one of these?")
        .items(&choices)
        .default(choices.len() - 1)
        .interact()?;

    if sel < all_results.len() {
        run_crawl(CrawlArgs {
            url: Some(all_results[sel].url.clone()),
            ..Default::default()
        })
        .await
    } else {
        Ok(())
    }
}
