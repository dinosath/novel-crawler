//! Crawler for https://novelishuniverse.com
//!
//! NovelishUniverse is a WordPress-based light novel translation site using
//! the "lightnovel" theme.
//!
//! URL patterns
//!   Series page : https://novelishuniverse.com/{slug}/
//!   Chapter page: https://novelishuniverse.com/{chapter-slug}/

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};

use crate::crawler::{clean_chapter_html, select_first_text, Crawler};
use crate::models::{Chapter, Novel, SearchResult};

pub struct NovelishUniverseCrawler;

impl NovelishUniverseCrawler {
    pub fn new() -> Self {
        NovelishUniverseCrawler
    }
}

#[async_trait]
impl Crawler for NovelishUniverseCrawler {
    fn name(&self) -> &str {
        "NovelishUniverse"
    }

    fn base_urls(&self) -> &[&'static str] {
        &["https://novelishuniverse.com"]
    }

    async fn read_novel_info(&self, client: &Client, url: &str) -> Result<Novel> {
        let resp = client
            .get(url)
            .send()
            .await
            .context("Failed to GET NovelishUniverse series page")?;
        let body = resp.text().await?;

        // Parse metadata in a scoped block so `Html` (!Send) is dropped
        // before the next `.await`.
        let (title, author, cover_url, synopsis, chapters) = {
            let doc = Html::parse_document(&body);

            // Title
            let title = select_first_text(&doc, "h1.entry-title")
                .unwrap_or_else(|| "Unknown Title".to_string());

            // Author — find the `.serl` whose `.sername` is "Author"
            let author = {
                let serl_sel = Selector::parse(".sertoauth .serl").expect("valid");
                let sername_sel = Selector::parse(".sername").expect("valid");
                let serval_sel = Selector::parse(".serval").expect("valid");
                doc.select(&serl_sel)
                    .find(|el| {
                        el.select(&sername_sel)
                            .next()
                            .map(|n| n.text().collect::<String>().trim().to_lowercase().contains("author"))
                            .unwrap_or(false)
                    })
                    .and_then(|el| {
                        el.select(&serval_sel)
                            .next()
                            .map(|v| v.text().collect::<String>().trim().to_string())
                    })
            };

            // Cover image — uses data-src for lazy loading
            let cover_url = {
                let sel = Selector::parse(".sertothumb img").expect("valid");
                doc.select(&sel)
                    .next()
                    .and_then(|el| {
                        el.value()
                            .attr("data-src")
                            .or_else(|| el.value().attr("src"))
                    })
                    .filter(|s| !s.starts_with("data:"))
                    .map(|s| s.to_string())
            };

            // Synopsis
            let synopsis = {
                let sel = Selector::parse(".sersys.entry-content").expect("valid");
                doc.select(&sel)
                    .next()
                    .map(|el| el.text().collect::<String>().trim().to_string())
            };

            // Chapter list — listed descending, we reverse to ascending.
            // Chapters marked with premium indicators (🔐, 🔒, 🔏, ⭐) in
            // the title cannot be read without a paid account, so we skip them.
            let chapters = {
                let li_sel = Selector::parse(".eplister ul li a").expect("valid");
                let num_sel = Selector::parse(".epl-num").expect("valid");

                let mut ch_list: Vec<(String, String)> = Vec::new();
                for a in doc.select(&li_sel) {
                    let href = a.value().attr("href").unwrap_or_default();
                    if href.is_empty() {
                        continue;
                    }

                    let ch_title = a
                        .select(&num_sel)
                        .next()
                        .map(|el| el.text().collect::<String>().trim().to_string())
                        .unwrap_or_default();

                    // Skip premium/locked chapters
                    if ch_title.contains('\u{1F512}')   // 🔒
                        || ch_title.contains('\u{1F510}') // 🔐
                        || ch_title.contains('\u{1F50F}') // 🔏
                        || ch_title.contains('\u{2B50}')  // ⭐
                    {
                        continue;
                    }

                    ch_list.push((ch_title, href.to_string()));
                }
                // Reverse so chapter 1 is first
                ch_list.reverse();
                ch_list
            };

            (title, author, cover_url, synopsis, chapters)
            // `doc` is dropped here — before any `.await`
        };

        let mut novel = Novel::new(url);
        novel.title = title;
        novel.author = author;
        novel.cover_url = cover_url;
        novel.synopsis = synopsis;

        novel.chapters = chapters
            .into_iter()
            .enumerate()
            .map(|(i, (title, url))| Chapter::new(i + 1, title, url))
            .collect();

        Ok(novel)
    }

    async fn read_chapter(&self, client: &Client, chapter: &mut Chapter) -> Result<()> {
        let resp = client
            .get(&chapter.url)
            .send()
            .await
            .context("Failed to GET NovelishUniverse chapter page")?;
        let body = resp.text().await?;
        let doc = Html::parse_document(&body);

        let content_sel = Selector::parse(".epcontent.entry-content").expect("valid");
        let paywall_sel = Selector::parse(".mycred-sell-this-wrapper").expect("valid");

        let inner = doc
            .select(&content_sel)
            .next()
            .map(|el| el.inner_html());

        // If the content div contains a paywall wrapper, treat as no content.
        let is_paywall = doc.select(&paywall_sel).next().is_some();

        chapter.body = if is_paywall {
            None
        } else {
            inner.map(|h| clean_chapter_html(&h, &chapter.url))
        };
        Ok(())
    }

    fn supports_search(&self) -> bool {
        true
    }

    async fn search(&self, client: &Client, query: &str) -> Result<Vec<SearchResult>> {
        let url = format!(
            "https://novelishuniverse.com/?s={}",
            query.replace(' ', "+")
        );
        let resp = client.get(&url).send().await?;
        let body = resp.text().await?;
        let doc = Html::parse_document(&body);

        let item_sel = Selector::parse(".bs .bsx").expect("valid");
        let title_sel = Selector::parse("a").expect("valid");
        let img_sel = Selector::parse("img").expect("valid");

        let mut results = Vec::new();
        for item in doc.select(&item_sel) {
            let a = match item.select(&title_sel).next() {
                Some(a) => a,
                None => continue,
            };
            let href = a.value().attr("href").unwrap_or_default().to_string();
            let title = a
                .value()
                .attr("title")
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| a.text().collect::<String>().trim().to_string());
            if title.is_empty() || href.is_empty() {
                continue;
            }

            let cover_url = item
                .select(&img_sel)
                .next()
                .and_then(|el| {
                    el.value()
                        .attr("data-src")
                        .or_else(|| el.value().attr("src"))
                })
                .filter(|s| !s.starts_with("data:"))
                .map(|s| s.to_string());

            results.push(SearchResult {
                title,
                url: href,
                author: None,
                cover_url,
                synopsis: None,
                source_name: self.name().to_string(),
            });
        }
        Ok(results)
    }
}
