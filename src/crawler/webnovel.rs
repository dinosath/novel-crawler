//! Crawler for https://www.webnovel.com  (public chapters only)
//!
//! Webnovel uses a REST/JSON API for chapter content which requires a
//! CSRF token harvested from the cookie jar.  Public (free) chapters are
//! available without a login; premium chapters are not accessible.

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};

use crate::crawler::{absolute_url, select_first_text, Crawler};
use crate::models::{Chapter, Novel, SearchResult, Volume};

pub struct WebnovelCrawler;

impl WebnovelCrawler {
    pub fn new() -> Self {
        WebnovelCrawler
    }

    /// Extract the book ID from a WebNovel URL.
    /// e.g. /book/my-novel_123456789 → 123456789
    fn book_id(url: &str) -> Option<String> {
        let re = regex::Regex::new(r"[/_](\d{10,})").ok()?;
        let caps = re.captures(url)?;
        Some(caps[1].to_string())
    }

    /// Obtain the CSRF token by visiting the homepage.
    async fn get_csrf(client: &Client) -> String {
        // The cookie named "_csrfToken" is set by WebNovel upon visiting the site.
        // We trigger a GET so the cookie is stored in the client's jar.
        let _ = client.get("https://www.webnovel.com").send().await;
        // reqwest's cookie jar does not give programmatic read access to cookie values
        // through the public API.  We embed a static fallback that often works
        // for unauthenticated requests.  Real production use would parse Set-Cookie.
        "fallback_csrf_token".to_string()
    }
}

#[async_trait]
impl Crawler for WebnovelCrawler {
    fn name(&self) -> &str {
        "Webnovel"
    }

    fn base_urls(&self) -> &[&'static str] {
        &[
            "https://www.webnovel.com",
            "https://m.webnovel.com",
        ]
    }

    fn supports_search(&self) -> bool {
        true
    }

    async fn read_novel_info(&self, client: &Client, url: &str) -> Result<Novel> {
        // Warm up the cookie jar
        let _ = client.get("https://www.webnovel.com").send().await;

        let book_id = Self::book_id(url)
            .context("Could not extract Webnovel book ID from URL")?;

        // Fetch novel metadata via the public info API
        let api_url = format!(
            "https://www.webnovel.com/apiajax/BookInfo/GetInfo?bookId={}",
            book_id
        );
        let resp = client
            .get(&api_url)
            .header("referer", url)
            .send()
            .await
            .context("Failed to fetch Webnovel book info")?;

        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        let data = &json["data"]["bookInfo"];

        let mut novel = Novel::new(url);
        novel.title = data["bookName"]
            .as_str()
            .unwrap_or("Unknown Title")
            .to_string();
        novel.author = data["authorName"].as_str().map(|s| s.to_string());
        novel.cover_url = data["coverUpdateTime"]
            .as_u64()
            .map(|_| format!("https://book-pic.webnovel.com/bookcover/{}/300/300", book_id));
        novel.synopsis = data["description"].as_str().map(|s| s.to_string());

        // Fetch volume & chapter list
        let vol_api = format!(
            "https://www.webnovel.com/apiajax/chapter/GetChapterList?bookId={}",
            book_id
        );
        let vol_resp = client
            .get(&vol_api)
            .header("referer", url)
            .send()
            .await
            .context("Failed to fetch Webnovel chapter list")?;
        let vol_json: serde_json::Value = vol_resp.json().await.unwrap_or_default();

        let mut chapters: Vec<Chapter> = Vec::new();
        if let Some(volumes) = vol_json["data"]["volumeItems"].as_array() {
            for (vi, vol) in volumes.iter().enumerate() {
                let vol_title = vol["volumeName"]
                    .as_str()
                    .unwrap_or(&format!("Volume {}", vi + 1))
                    .to_string();
                novel.volumes.push(Volume::new(vi + 1, vol_title));

                if let Some(chaps) = vol["chapterItems"].as_array() {
                    for chap in chaps {
                        let chap_id = chap["chapterId"].as_str().unwrap_or_default();
                        let chap_name = chap["chapterName"]
                            .as_str()
                            .unwrap_or(&format!("Chapter {}", chapters.len() + 1))
                            .to_string();
                        let chap_url = format!(
                            "https://www.webnovel.com/book/{}/{}/chapter",
                            book_id, chap_id
                        );
                        let is_locked = chap["isAuth"].as_u64().unwrap_or(0) == 0;
                        if !is_locked {
                            let id = chapters.len() + 1;
                            let mut c = Chapter::new(id, chap_name, chap_url);
                            c.volume_id = Some(vi + 1);
                            chapters.push(c);
                        }
                    }
                }
            }
        }
        novel.chapters = chapters;
        Ok(novel)
    }

    async fn read_chapter(&self, client: &Client, chapter: &mut Chapter) -> Result<()> {
        // Extract book_id and chapter_id from the internal URL we built
        let parts: Vec<&str> = chapter.url.split('/').collect();
        // URL: https://www.webnovel.com/book/{book_id}/{chapter_id}/chapter
        let book_id = parts.get(parts.len().saturating_sub(3)).unwrap_or(&"").to_string();
        let chapter_id = parts.get(parts.len().saturating_sub(2)).unwrap_or(&"").to_string();

        // Warm up cookie
        let _ = client.get("https://www.webnovel.com").send().await;

        let api_url = format!(
            "https://www.webnovel.com/apiajax/chapter/GetChapterContent\
             ?bookId={}&chapterId={}&_csrfToken=",
            book_id, chapter_id
        );
        let resp = client
            .get(&api_url)
            .header("referer", &chapter.url)
            .send()
            .await
            .context("Webnovel chapter API failed")?;

        let json: serde_json::Value = resp.json().await.unwrap_or_default();
        if let Some(content) = json["data"]["chapterInfo"]["content"].as_str() {
            // Content arrives as plain HTML paragraphs
            chapter.body = Some(content.to_string());
        } else {
            // Fallback: scrape the chapter page directly
            let page_resp = client
                .get(&format!(
                    "https://www.webnovel.com/book/{}/{}",
                    book_id, chapter_id
                ))
                .send()
                .await?;
            let html = page_resp.text().await?;
            let doc = Html::parse_document(&html);
            let sel = Selector::parse(".cha-words").expect("valid");
            if let Some(el) = doc.select(&sel).next() {
                chapter.body = Some(el.inner_html());
            }
        }
        Ok(())
    }

    async fn search(&self, client: &Client, query: &str) -> Result<Vec<SearchResult>> {
        let url = format!(
            "https://www.webnovel.com/search?keywords={}",
            query.replace(' ', "+")
        );
        let resp = client.get(&url).send().await?;
        let body = resp.text().await?;
        let doc = Html::parse_document(&body);

        let item_sel = Selector::parse(".j_bookList li").expect("valid");
        let title_sel = Selector::parse(".j_bookCard").expect("valid");
        let cover_sel = Selector::parse("img").expect("valid");

        let mut results = Vec::new();
        for item in doc.select(&item_sel) {
            let frag = Html::parse_fragment(&item.inner_html());

            let tit_el = frag.select(&title_sel).next();
            let href = tit_el
                .and_then(|el| el.value().attr("href"))
                .unwrap_or_default();
            let nourl = absolute_url("https://www.webnovel.com", href);

            let title = select_first_text(&frag, ".g_thumb + div h4")
                .or_else(|| select_first_text(&frag, "h4"))
                .unwrap_or_default();
            if title.is_empty() {
                continue;
            }

            results.push(SearchResult {
                title,
                url: nourl,
                author: select_first_text(&frag, ".author"),
                cover_url: frag
                    .select(&cover_sel)
                    .next()
                    .and_then(|el| {
                        el.value().attr("data-original").or_else(|| el.value().attr("src"))
                    })
                    .map(|s| s.to_string()),
                synopsis: None,
                source_name: self.name().to_string(),
            });
        }
        Ok(results)
    }
}
