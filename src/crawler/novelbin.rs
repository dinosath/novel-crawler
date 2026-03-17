//! Crawler for https://novelbin.com  (served via the CF-free mirror novelbin.me)
//!
//! NovelBin is a Node.js/Express application.  The main pages are accessible
//! via `novelbin.me` (no Cloudflare).  Chapter content is loaded via a
//! server-side AJAX endpoint that requires a valid session cookie + CSRF token.
//!
//! Flow
//!  1. GET  `novelbin.me/b/{slug}`                     → series info + CSRF token
//!  2. GET  `novelbin.me/ajax/chapter-archive?novelId={slug}` → full chapter list
//!  3. POST `novelbin.me/ajax/show-full-content`
//!          body: novel_id=<slug>&chapter_id=<ch-slug>&_csrf=<token>
//!          → JSON { success: true, content: "<html>" }
//!
//! URL patterns accepted
//!   https://novelbin.com/b/{slug}
//!   https://novelbin.me/b/{slug}
//!   https://novelbin.me/novel-book/{slug}
//!   … the slug is extracted in all cases.

use std::sync::Arc;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};
use tokio::sync::Mutex;

use crate::crawler::{absolute_url, clean_chapter_html, percent_encode, select_first_attr, select_first_text, Crawler};
use crate::models::{Chapter, Novel, SearchResult};

const BASE: &str = "https://novelbin.me";

/// Cached CSRF token so `read_chapter` doesn't have to re-fetch the series
/// page on every chapter download.
pub struct NovelBinCrawler {
    csrf: Arc<Mutex<Option<String>>>,
}

impl NovelBinCrawler {
    pub fn new() -> Self {
        NovelBinCrawler {
            csrf: Arc::new(Mutex::new(None)),
        }
    }

    /// Extract the novel slug from any supported NovelBin URL variant.
    ///
    /// ```text
    /// novelbin.com/b/{slug}              → slug
    /// novelbin.me/b/{slug}               → slug
    /// novelbin.me/novel-book/{slug}      → slug
    /// novelbin.me/novel-book/{slug}/...  → slug
    /// ```
    fn slug_from_url(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let path = parsed.path().trim_start_matches('/');
        // Split into at most 3 segments: prefix / slug / rest
        let mut parts = path.splitn(3, '/');
        match parts.next() {
            Some("b") | Some("novel-book") => {
                parts.next().filter(|s| !s.is_empty()).map(|s| s.to_string())
            }
            _ => None,
        }
    }

    /// Extract the CSRF token embedded in a NovelBin HTML page.
    ///
    /// The page includes:  `const csrf = "TOKEN_VALUE";`
    /// Returns `None` if the token is absent or empty (e.g. on 404 pages).
    fn extract_csrf(html_body: &str) -> Option<String> {
        let needle = "const csrf = \"";
        let start = html_body.find(needle)? + needle.len();
        let end = html_body[start..].find('"')? + start;
        let token = &html_body[start..end];
        if token.is_empty() { None } else { Some(token.to_string()) }
    }

    /// Fetch a CSRF token by loading a known-good series page.
    ///
    /// Some novels have a `/b/{slug}` page that returns HTTP 404 (the slug is
    /// only listed under `/novel-book/`), so their page never embeds a valid
    /// `const csrf = "…"` snippet.  The CSRF token is session-scoped on
    /// novelbin.me, so a token obtained from *any* 200-OK series page in the
    /// same reqwest session is accepted for every `show-full-content` POST.
    async fn bootstrap_csrf(client: &Client) -> String {
        // "catastrophic-necromancer" reliably returns HTTP 200 on novelbin.me.
        const BOOTSTRAP_URL: &str = "https://novelbin.me/b/catastrophic-necromancer";
        match client.get(BOOTSTRAP_URL).send().await {
            Ok(resp) => match resp.text().await {
                Ok(body) => Self::extract_csrf(&body).unwrap_or_default(),
                Err(_) => String::new(),
            },
            Err(_) => String::new(),
        }
    }

    /// Pretty-print a URL slug as a title.
    ///
    /// `"the-regressor-can-make-them-all"` → `"The Regressor Can Make Them All"`
    fn title_from_slug(slug: &str) -> String {
        slug.split('-')
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    None => String::new(),
                    Some(c) => c.to_uppercase().to_string() + chars.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Parse the series info page in a scoped block so `Html` is dropped
    /// before any `.await`.  Returns `(title, author, cover_url, synopsis)`.
    fn parse_series_page(body: &str, base_url: &str) -> (String, Option<String>, Option<String>, Option<String>) {
        let doc = Html::parse_document(body);

        let title = select_first_text(&doc, "h3.title")
            .or_else(|| select_first_text(&doc, ".book h3"))
            .or_else(|| select_first_text(&doc, "h1"))
            .unwrap_or_else(|| "Unknown Title".to_string());

        // Author is inside a <li> whose first child is <h3>Author:</h3>
        let author = {
            let li_sel = Selector::parse(".info-meta li").unwrap();
            doc.select(&li_sel)
                .find(|el| {
                    let text = el.text().collect::<String>().to_lowercase();
                    text.contains("author")
                })
                .and_then(|el| {
                    let a_sel = Selector::parse("a").unwrap();
                    el.select(&a_sel)
                        .next()
                        .map(|a| a.text().collect::<String>().trim().to_string())
                })
        };

        let cover_url = select_first_attr(&doc, ".book img", "src")
            .map(|src| absolute_url(base_url, &src));

        // Synopsis is in the first <div class="desc"> element
        let synopsis = select_first_text(&doc, "div.desc");

        (title, author, cover_url, synopsis)
        // `doc` dropped here
    }

    /// Parse the chapter-archive HTML fragment: returns a vec of `(slug, title)`.
    ///
    /// The `/ajax/chapter-archive` endpoint returns an HTML fragment (not a
    /// full page) so we wrap it in a synthetic `<html>` before parsing.
    ///
    /// Chapter slugs look like `chapter-1` or `chapter-42-some-title`.
    fn parse_chapter_archive(html_fragment: &str, novel_slug: &str) -> Vec<(String, String)> {
        let full_html = format!("<html><body>{}</body></html>", html_fragment);
        let doc = Html::parse_document(&full_html);
        let a_sel = Selector::parse("ul.list-chapter li a").expect("valid");

        let novel_book_prefix = format!("{}/novel-book/{}/", BASE, novel_slug);

        let mut pairs: Vec<(String, String)> = Vec::new();
        for a in doc.select(&a_sel) {
            let href = a.value().attr("href").unwrap_or_default();
            // href is the full URL, e.g. https://novelbin.me/novel-book/slug/chapter-5
            let ch_slug = href
                .strip_prefix(&novel_book_prefix)
                .unwrap_or_default()
                .trim_matches('/')
                .to_string();
            if ch_slug.is_empty() {
                continue;
            }
            let title = a
                .value()
                .attr("title")
                .map(|s| s.trim().to_string())
                .or_else(|| {
                    let t = a.text().collect::<String>().trim().to_string();
                    if t.is_empty() { None } else { Some(t) }
                })
                .unwrap_or_else(|| ch_slug.replace('-', " "));
            pairs.push((ch_slug, title));
        }
        pairs
        // `doc` is dropped here
    }
}

#[async_trait]
impl Crawler for NovelBinCrawler {
    fn name(&self) -> &str {
        "NovelBin"
    }

    fn base_urls(&self) -> &[&'static str] {
        &["https://novelbin.com", "https://novelbin.me"]
    }

    fn supports_search(&self) -> bool {
        true
    }

    async fn read_novel_info(&self, client: &Client, url: &str) -> Result<Novel> {
        let slug = Self::slug_from_url(url)
            .context("Failed to extract novel slug from NovelBin URL")?;

        // ── Step 1: fetch the series page ────────────────────────────────────
        let series_url = format!("{}/b/{}", BASE, slug);
        let resp = client
            .get(&series_url)
            .send()
            .await
            .context("Failed to GET NovelBin series page")?;
        let body = resp.text().await?;

        // ── Step 2: extract CSRF ──────────────────────────────────────────────
        // The `const csrf = "…"` snippet is only present when the series page
        // returns HTTP 200.  For novels whose `/b/{slug}` page 404s, the token
        // is empty.  In that case we bootstrap from a known-good series page:
        // the CSRF token is session-scoped so a token from any 200-OK page in
        // the same reqwest Client session is valid for all show-full-content POSTs.
        let csrf = match Self::extract_csrf(&body) {
            Some(t) => t,
            None => Self::bootstrap_csrf(client).await,
        };

        // Cache so read_chapter doesn't have to re-fetch.
        *self.csrf.lock().await = Some(csrf.clone());

        // ── Step 3: parse metadata ───────────────────────────────────────────
        let (title, author, cover_url, synopsis) = Self::parse_series_page(&body, BASE);
        // When the series page 404s the title will be generic; fall back to slug.
        let title = if title == "Unknown Title" || title.to_lowercase().contains("not found") {
            Self::title_from_slug(&slug)
        } else {
            title
        };

        let mut novel = Novel::new(url);
        novel.title = title;
        novel.author = author;
        novel.cover_url = cover_url;
        novel.synopsis = synopsis;

        // ── Step 4: fetch the full chapter list from the AJAX endpoint ────────
        let archive_url = format!("{}/ajax/chapter-archive?novelId={}", BASE, slug);
        let archive_body = client
            .get(&archive_url)
            .header("Referer", &series_url)
            .send()
            .await
            .context("Failed to GET NovelBin chapter archive")?
            .text()
            .await?;

        let pairs = Self::parse_chapter_archive(&archive_body, &slug);
        if pairs.is_empty() {
            bail!("NovelBin: no chapters found for '{}'", slug);
        }

        // ── Step 5: build Chapter objects ──────────────────────────────────────
        novel.chapters = pairs
            .into_iter()
            .enumerate()
            .map(|(i, (ch_slug, ch_title))| {
                let ch_url = format!("{}/novel-book/{}/{}", BASE, slug, ch_slug);
                Chapter::new(i + 1, ch_title, ch_url)
            })
            .collect();

        Ok(novel)
    }

    async fn read_chapter(&self, client: &Client, chapter: &mut Chapter) -> Result<()> {
        // Derive the novel slug and chapter slug from the stored URL.
        // URL form: https://novelbin.me/novel-book/{novel-slug}/{ch-slug}
        let parsed = url::Url::parse(&chapter.url)
            .context("Invalid chapter URL")?;
        let path = parsed.path().trim_start_matches('/');
        let mut parts = path.splitn(4, '/');
        let _prefix = parts.next(); // "novel-book"
        let novel_slug = parts.next().unwrap_or_default().to_string();
        let ch_slug = parts.next().unwrap_or_default().to_string();

        if novel_slug.is_empty() || ch_slug.is_empty() {
            bail!("NovelBin: cannot derive slug from chapter URL '{}'", chapter.url);
        }

        // ── Get CSRF token ────────────────────────────────────────────────────
        // Use the value cached by read_novel_info.  If for some reason the
        // cache is empty (shouldn't happen in normal use), bootstrap from a
        // known-good series page.
        let csrf = {
            let cached = self.csrf.lock().await.clone();
            if let Some(c) = cached.filter(|s| !s.is_empty()) {
                c
            } else {
                let series_url = format!("{}/b/{}", BASE, novel_slug);
                let html = client
                    .get(&series_url)
                    .send()
                    .await
                    .ok()
                    .map(|r| async move { r.text().await.unwrap_or_default() });
                let html = if let Some(f) = html { f.await } else { String::new() };
                let token = match Self::extract_csrf(&html) {
                    Some(t) => t,
                    None => Self::bootstrap_csrf(client).await,
                };
                *self.csrf.lock().await = Some(token.clone());
                token
            }
        };

        // ── POST to the show-full-content endpoint ───────────────────────────
        let series_url = format!("{}/b/{}", BASE, novel_slug);
        let resp = client
            .post(format!("{}/ajax/show-full-content", BASE))
            .header("Referer", &series_url)
            .header("X-Requested-With", "XMLHttpRequest")
            .form(&[
                ("novel_id", novel_slug.as_str()),
                ("chapter_id", ch_slug.as_str()),
                ("_csrf", csrf.as_str()),
            ])
            .send()
            .await
            .context("Failed to POST to NovelBin show-full-content")?;

        let json: serde_json::Value = resp.json().await
            .context("NovelBin show-full-content did not return JSON")?;

        if json["success"].as_bool() != Some(true) {
            bail!(
                "NovelBin: show-full-content returned failure for '{}': {:?}",
                ch_slug,
                json.get("message")
            );
        }

        let content = json["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        if !content.is_empty() {
            chapter.body = Some(clean_chapter_html(&content, &chapter.url));
        }

        Ok(())
    }

    async fn search(&self, client: &Client, query: &str) -> Result<Vec<SearchResult>> {
        let url = format!(
            "{}/search?keyword={}",
            BASE,
            percent_encode(query)
        );

        let body = client
            .get(&url)
            .send()
            .await
            .context("Failed to GET NovelBin search")?
            .text()
            .await?;

        // Parse HTML results — all in a scoped block so `Html` is dropped.
        let results = {
            let doc = Html::parse_document(&body);
            let item_sel = Selector::parse(".list-novel .row").expect("valid");
            let title_sel = Selector::parse("h3.novel-title a").expect("valid");
            let cover_sel = Selector::parse("img.cover").expect("valid");
            let author_sel = Selector::parse(".author").expect("valid");
            let synopsis_sel = Selector::parse(".novel-item .list-inline").expect("valid");

            let mut out: Vec<SearchResult> = Vec::new();
            for item in doc.select(&item_sel) {
                let title_el = item.select(&title_sel).next();
                let title = title_el
                    .map(|el| el.text().collect::<String>().trim().to_string())
                    .unwrap_or_default();
                let item_url = title_el
                    .and_then(|el| el.value().attr("href"))
                    .map(|h| absolute_url(BASE, h))
                    .unwrap_or_default();
                if title.is_empty() || item_url.is_empty() {
                    continue;
                }
                let cover_url = item
                    .select(&cover_sel)
                    .next()
                    .and_then(|el| el.value().attr("src").or_else(|| el.value().attr("data-src")))
                    .map(|s| s.to_string());
                let author = item
                    .select(&author_sel)
                    .next()
                    .map(|el| el.text().collect::<String>().trim().to_string());
                let synopsis = item
                    .select(&synopsis_sel)
                    .next()
                    .map(|el| el.text().collect::<String>().trim().to_string());
                out.push(SearchResult {
                    title,
                    url: item_url,
                    author,
                    cover_url,
                    synopsis,
                    source_name: self.name().to_string(),
                });
            }
            out
        }; // `doc` dropped here

        Ok(results)
    }
}


