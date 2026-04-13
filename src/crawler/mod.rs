pub mod brightnovels;
pub mod novelishuniverse;
pub mod novelfull;
pub mod novelbin;
pub mod royalroad;
pub mod scribblehub;
pub mod webnovel;

use anyhow::Result;
use async_trait::async_trait;
use reqwest::{Client, ClientBuilder};

use crate::models::{Chapter, Novel, SearchResult};

// ── Crawler trait ────────────────────────────────────────────────────────────

/// Every source must implement this trait.
///
/// Mirrors the Python `Crawler` base class:
/// - `read_novel_info` → fetches title/author/cover/synopsis and the full chapter list
/// - `read_chapter`    → downloads and parses a single chapter's HTML body
/// - `search`          → optional, returns a list of matching novels
#[async_trait]
pub trait Crawler: Send + Sync {
    fn name(&self) -> &str;
    fn base_urls(&self) -> &[&'static str];

    fn language(&self) -> &str {
        "en"
    }

    fn supports_search(&self) -> bool {
        false
    }

    /// Fetch novel metadata **and** the chapter list.  Sets `novel.chapters`
    /// (and optionally `novel.volumes`) but does NOT download chapter bodies.
    async fn read_novel_info(&self, client: &Client, url: &str) -> Result<Novel>;

    /// Download and parse the body of a single chapter.
    /// Sets `chapter.body` to cleaned HTML.
    async fn read_chapter(&self, client: &Client, chapter: &mut Chapter) -> Result<()>;

    /// Search this source for novels matching `query`.
    async fn search(&self, _client: &Client, _query: &str) -> Result<Vec<SearchResult>> {
        anyhow::bail!("{} does not support search", self.name())
    }
}

// ── Shared HTTP client ───────────────────────────────────────────────────────

/// Build a shared `reqwest::Client` with sensible defaults.
pub fn build_client() -> Result<Client> {
    let client = ClientBuilder::new()
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
             AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/120.0.0.0 Safari/537.36",
        )
        .cookie_store(true)
        .gzip(true)
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    Ok(client)
}

// ── Registry / factory ───────────────────────────────────────────────────────

/// Return the appropriate crawler for `url`, or `None` if unsupported.
pub fn create_crawler(url: &str) -> Option<Box<dyn Crawler>> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_lowercase();
    let host = host.trim_start_matches("www.");

    match host {
        "brightnovels.com" => Some(Box::new(brightnovels::BrightNovelsCrawler::new())),
        "royalroad.com" => Some(Box::new(royalroad::RoyalRoadCrawler::new())),
        "novelfull.com" | "novelfull.net" => Some(Box::new(novelfull::NovelFullCrawler::new())),
        "scribblehub.com" => Some(Box::new(scribblehub::ScribbleHubCrawler::new())),
        "webnovel.com" | "m.webnovel.com" => Some(Box::new(webnovel::WebnovelCrawler::new())),
        "novelbin.com" | "novelbin.me" => Some(Box::new(novelbin::NovelBinCrawler::new())),
        "novelishuniverse.com" => Some(Box::new(novelishuniverse::NovelishUniverseCrawler::new())),
        _ => None,
    }
}

/// Convenience: collect all registered crawler instances for search.
pub fn all_crawlers() -> Vec<Box<dyn Crawler>> {
    vec![
        Box::new(brightnovels::BrightNovelsCrawler::new()),
        Box::new(royalroad::RoyalRoadCrawler::new()),
        Box::new(novelfull::NovelFullCrawler::new()),
        Box::new(scribblehub::ScribbleHubCrawler::new()),
        Box::new(webnovel::WebnovelCrawler::new()),
        Box::new(novelbin::NovelBinCrawler::new()),
        Box::new(novelishuniverse::NovelishUniverseCrawler::new()),
    ]
}

/// Convenience: crawlers that support search.
pub fn search_crawlers() -> Vec<Box<dyn Crawler>> {
    all_crawlers()
        .into_iter()
        .filter(|c| c.supports_search())
        .collect()
}

// ── Shared HTML helpers ──────────────────────────────────────────────────────

/// Strip unwanted tags/attributes from chapter HTML to produce clean XHTML.
pub fn clean_chapter_html(html: &str, base_url: &str) -> String {
    use scraper::{Html, Selector};

    let fragment = Html::parse_fragment(html);

    // Remove script, style, ad containers
    let bad_selectors = ["script", "style", ".ads", "#ads", "[class*='ad-']"];
    let mut cleaned = html.to_string();

    for sel_str in &bad_selectors {
        if let Ok(sel) = Selector::parse(sel_str) {
            for el in fragment.select(&sel) {
                let outer = el.html();
                cleaned = cleaned.replace(&outer, "");
            }
        }
    }

    // Resolve relative image URLs
    if let Ok(base) = url::Url::parse(base_url) {
        let re = regex::Regex::new(r#"src="(/[^"]*)"#).unwrap();
        cleaned = re
            .replace_all(&cleaned, |caps: &regex::Captures| {
                let path = &caps[1];
                base.join(path)
                    .map(|u| format!(r#"src="{}"#, u))
                    .unwrap_or_else(|_| caps[0].to_string())
            })
            .to_string();
    }

    cleaned
}

/// Select the **first** element matching `selector` and return its trimmed inner HTML.
pub fn select_first_html(
    html: &scraper::Html,
    selector: &str,
) -> Option<String> {
    let sel = scraper::Selector::parse(selector).ok()?;
    html.select(&sel).next().map(|el| el.inner_html().trim().to_string())
}

/// Select the **first** element matching `selector` and return its trimmed text.
pub fn select_first_text(
    html: &scraper::Html,
    selector: &str,
) -> Option<String> {
    let sel = scraper::Selector::parse(selector).ok()?;
    html.select(&sel).next().map(|el| {
        el.text().collect::<String>().trim().to_string()
    })
}

/// Select the `attr` attribute of the first element matching `selector`.
pub fn select_first_attr<'a>(
    html: &'a scraper::Html,
    selector: &str,
    attr: &str,
) -> Option<String> {
    let sel = scraper::Selector::parse(selector).ok()?;
    html.select(&sel)
        .next()
        .and_then(|el| el.value().attr(attr))
        .map(|v| v.trim().to_string())
}

// ── Shared URL helpers ───────────────────────────────────────────────────────

/// Resolve `href` to an absolute URL.  If `href` already starts with `http`
/// it is returned unchanged; otherwise it is appended to `base`.
pub fn absolute_url(base: &str, href: &str) -> String {
    if href.starts_with("http") {
        href.to_string()
    } else {
        format!("{}{}", base, href)
    }
}

/// Percent-encode a string for safe embedding in a query parameter.
/// Spaces are encoded as `+`; all other non-unreserved bytes are hex-escaped.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
        }
    }
    out
}
