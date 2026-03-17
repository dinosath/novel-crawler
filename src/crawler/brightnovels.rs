//! Crawler for https://brightnovels.com
//!
//! BrightNovels is a Laravel + Inertia.js application.  All page data is
//! embedded as JSON in the `data-page` attribute of the `<div id="app">`
//! element.  A dedicated JSON API at `/series/{slug}/chapters` returns the
//! complete chapter list.
//!
//! URL patterns
//!   Series page : https://brightnovels.com/series/{slug}
//!   Chapter page: https://brightnovels.com/series/{slug}/{chapterSlug}
//!   Chapters API: https://brightnovels.com/series/{slug}/chapters  (JSON)

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};

use crate::crawler::{clean_chapter_html, percent_encode, Crawler};
use crate::models::{Chapter, Novel, SearchResult};

pub struct BrightNovelsCrawler;

impl BrightNovelsCrawler {
    pub fn new() -> Self {
        BrightNovelsCrawler
    }

    /// Extract the series slug from a BrightNovels URL.
    ///
    /// ```
    /// "https://brightnovels.com/series/some-slug"          → "some-slug"
    /// "https://brightnovels.com/series/some-slug/chapter1" → "some-slug"
    /// ```
    fn slug_from_url(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let mut parts = parsed.path().trim_start_matches('/').splitn(3, '/');
        if parts.next() == Some("series") {
            parts.next().filter(|s| !s.is_empty()).map(|s| s.to_string())
        } else {
            None
        }
    }

    /// Parse the Inertia.js `data-page` JSON from an HTML response body.
    ///
    /// This is a **synchronous** helper.  The `Html` object is created and
    /// dropped entirely within this function, so callers can safely call it
    /// before an `.await` point without running into `!Send` issues.
    fn parse_inertia_data(html_body: &str) -> Option<serde_json::Value> {
        let doc = Html::parse_document(html_body);
        let sel = Selector::parse("[data-page]").ok()?;
        let data_page = doc.select(&sel).next()?.value().attr("data-page")?;
        serde_json::from_str(data_page).ok()
        // `doc` is dropped here
    }

    /// Decode HTML entities that BrightNovels stores inside JSON string fields.
    ///
    /// The `description` and `content` fields contain HTML that has been
    /// HTML-entity–encoded a second time (e.g. `&lt;p&gt;` instead of `<p>`).
    fn decode_entities(s: &str) -> String {
        s.replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#039;", "'")
            .replace("&apos;", "'")
            .replace("&nbsp;", "\u{00A0}")
    }
}

#[async_trait]
impl Crawler for BrightNovelsCrawler {
    fn name(&self) -> &str {
        "BrightNovels"
    }

    fn base_urls(&self) -> &[&'static str] {
        &["https://brightnovels.com"]
    }

    async fn read_novel_info(&self, client: &Client, url: &str) -> Result<Novel> {
        let slug = Self::slug_from_url(url)
            .context("Failed to extract series slug from BrightNovels URL")?;

        // 1. Fetch the series HTML page.
        let body = client
            .get(url)
            .send()
            .await
            .context("Failed to GET BrightNovels series page")?
            .text()
            .await?;

        // 2. Extract metadata from the Inertia JSON.
        //    `parse_inertia_data` is sync and drops `Html` before returning,
        //    so we are safe to `.await` afterwards.
        let page_data = Self::parse_inertia_data(&body)
            .context("Failed to parse Inertia page data from BrightNovels series page")?;

        let series = &page_data["props"]["series"];

        let title = series["title"]
            .as_str()
            .unwrap_or("Unknown Title")
            .to_string();

        let author = series["user"]["name"].as_str().map(|s| s.to_string());

        // `description` is HTML with all tags double-entity-encoded.
        let synopsis = series["description"]
            .as_str()
            .map(|s| Self::decode_entities(s));

        let cover_url = series["cover"]["path"]
            .as_str()
            .map(|p| format!("https://brightnovels.com/storage/{}", p));

        let mut novel = Novel::new(url);
        novel.title = title;
        novel.author = author;
        novel.synopsis = synopsis;
        novel.cover_url = cover_url;

        // 3. Fetch the full chapter list from the JSON API.
        //    The default endpoint returns all chapters sorted *descending*; we
        //    reverse to get ascending (chapter 1 first).
        let chapters_api =
            format!("https://brightnovels.com/series/{}/chapters", slug);

        let chapters_resp: serde_json::Value = client
            .get(&chapters_api)
            .header("Accept", "application/json")
            .send()
            .await
            .context("Failed to GET BrightNovels chapters API")?
            .json()
            .await?;

        let empty_vec: Vec<serde_json::Value> = Vec::new();
        let all_chapters = chapters_resp["chapters"]
            .as_array()
            .unwrap_or(&empty_vec);

        let free_chapters: Vec<&serde_json::Value> = all_chapters
            .iter()
            .rev() // API is desc → reverse to asc
            .filter(|ch| ch["is_premium"].as_bool() != Some(true))
            .collect();

        novel.chapters = free_chapters
            .iter()
            .enumerate()
            .map(|(i, ch)| {
                let ch_slug = ch["slug"].as_str().unwrap_or_default();
                let ch_name = ch["name"]
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("Chapter {}", i + 1));
                let ch_url = format!(
                    "https://brightnovels.com/series/{}/{}",
                    slug, ch_slug
                );
                Chapter::new(i + 1, ch_name, ch_url)
            })
            .collect();

        Ok(novel)
    }

    async fn read_chapter(&self, client: &Client, chapter: &mut Chapter) -> Result<()> {
        let body = client
            .get(&chapter.url)
            .send()
            .await
            .context("Failed to GET BrightNovels chapter page")?
            .text()
            .await?;

        // `parse_inertia_data` is sync; Html is dropped before we return.
        let content_opt = Self::parse_inertia_data(&body).and_then(|v| {
            v["props"]["chapter"]["content"]
                .as_str()
                .map(|s| s.to_string())
        });

        if let Some(encoded) = content_opt {
            // Content is HTML with tags double-entity-encoded; decode first.
            let html = Self::decode_entities(&encoded);
            chapter.body = Some(clean_chapter_html(&html, &chapter.url));
        }

        Ok(())
    }

    async fn search(&self, client: &Client, query: &str) -> Result<Vec<SearchResult>> {
        // BrightNovels exposes a simple JSON search endpoint.
        let url = format!(
            "https://brightnovels.com/series?search={}",
            percent_encode(query)
        );

        let body = client
            .get(&url)
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .send()
            .await
            .context("Failed to GET BrightNovels search")?
            .text()
            .await?;

        // The route may return an Inertia page (HTML) or JSON depending on
        // the Accept header.  Try JSON first, fall back to Inertia HTML.
        let mut results = Vec::new();

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
            // JSON response: array or {"data": [...]}
            let items = if let Some(arr) = json.as_array() {
                arr.clone()
            } else if let Some(arr) = json["data"].as_array() {
                arr.clone()
            } else if let Some(arr) = json["series"].as_array() {
                arr.clone()
            } else {
                vec![]
            };

            for item in &items {
                let title = item["title"].as_str().unwrap_or_default().to_string();
                let slug = item["slug"].as_str().unwrap_or_default();
                if title.is_empty() || slug.is_empty() {
                    continue;
                }
                let item_url =
                    format!("https://brightnovels.com/series/{}", slug);
                let cover = item["cover"]["path"]
                    .as_str()
                    .map(|p| format!("https://brightnovels.com/storage/{}", p));
                results.push(SearchResult {
                    title,
                    url: item_url,
                    author: item["user"]["name"].as_str().map(|s| s.to_string()),
                    cover_url: cover,
                    synopsis: item["description"]
                        .as_str()
                        .map(|s| Self::decode_entities(s)),
                    source_name: self.name().to_string(),
                });
            }
        } else {
            // Inertia HTML response – parse with same helper
            if let Some(page_data) = Self::parse_inertia_data(&body) {
                let empty_arr: Vec<serde_json::Value> = Vec::new();
                let items = page_data["props"]["series"]
                    .as_array()
                    .or_else(|| page_data["props"]["series"]["data"].as_array())
                    .unwrap_or(&empty_arr)
                    .clone();

                for item in &items {
                    let title =
                        item["title"].as_str().unwrap_or_default().to_string();
                    let slug = item["slug"].as_str().unwrap_or_default();
                    if title.is_empty() || slug.is_empty() {
                        continue;
                    }
                    let item_url =
                        format!("https://brightnovels.com/series/{}", slug);
                    let cover = item["cover"]["path"]
                        .as_str()
                        .map(|p| format!("https://brightnovels.com/storage/{}", p));
                    results.push(SearchResult {
                        title,
                        url: item_url,
                        author: item["user"]["name"]
                            .as_str()
                            .map(|s| s.to_string()),
                        cover_url: cover,
                        synopsis: item["description"]
                            .as_str()
                            .map(|s| Self::decode_entities(s)),
                        source_name: self.name().to_string(),
                    });
                }
            }
        }

        Ok(results)
    }

    fn supports_search(&self) -> bool {
        true
    }
}


