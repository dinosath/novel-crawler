//! Crawler for https://novelfull.com  and  https://novelfull.net

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};

use crate::crawler::{absolute_url, clean_chapter_html, select_first_attr, select_first_text, Crawler};
use crate::models::{Chapter, Novel, SearchResult};

pub struct NovelFullCrawler;

impl NovelFullCrawler {
    pub fn new() -> Self {
        NovelFullCrawler
    }

    /// Return the host prefix (e.g. "https://novelfull.com") extracted from a URL.
    fn base_from_url(url: &str) -> String {
        if let Ok(parsed) = url::Url::parse(url) {
            let scheme = parsed.scheme();
            let host = parsed.host_str().unwrap_or("novelfull.com");
            format!("{}://{}", scheme, host)
        } else {
            "https://novelfull.com".to_string()
        }
    }

    /// Collect all chapter URLs across paginated chapter lists.
    async fn fetch_all_chapters(
        &self,
        client: &Client,
        base: &str,
        first_page_html: &str,
    ) -> Result<Vec<(String, String)>> {
        let mut pairs: Vec<(String, String)> = Vec::new();
        let mut current_html = first_page_html.to_string();
        let mut page: u32 = 1;

        loop {
            // Parse HTML and extract all data in a non-async block so that
            // `Html` (which is !Send) is dropped before the next `.await`.
            let (batch, next_url_opt) = {
                let doc = Html::parse_document(&current_html);
                let li_sel = Selector::parse("ul.list-chapter li a").expect("valid");
                let next_sel = Selector::parse("li.next a[href]").expect("valid");

                let mut batch: Vec<(String, String)> = Vec::new();
                for a in doc.select(&li_sel) {
                    let href = a.value().attr("href").unwrap_or_default();
                    if href.is_empty() { continue; }
                    let title = a
                        .value()
                        .attr("title")
                        .map(|s| s.trim().to_string())
                        .or_else(|| {
                            let t = a.text().collect::<String>().trim().to_string();
                            if t.is_empty() { None } else { Some(t) }
                        })
                        .unwrap_or_else(|| format!("Chapter {}", pairs.len() + batch.len() + 1));
                    let url = absolute_url(base, href);
                    batch.push((title, url));
                }

                let next_url = doc
                    .select(&next_sel)
                    .next()
                    .and_then(|el| el.value().attr("href"))
                    .filter(|h| !h.is_empty() && *h != "#")
                    .map(|h| absolute_url(base, h));

                (batch, next_url)
                // `doc` is dropped here — before any `.await`
            };

            pairs.extend(batch);

            if let Some(next_url) = next_url_opt {
                page += 1;
                if page > 500 { break; }
                let resp = client.get(&next_url).send().await?;
                current_html = resp.text().await?;
            } else {
                break;
            }
        }
        Ok(pairs)
    }
}

#[async_trait]
impl Crawler for NovelFullCrawler {
    fn name(&self) -> &str {
        "NovelFull"
    }

    fn base_urls(&self) -> &[&'static str] {
        &["https://novelfull.com", "https://novelfull.net"]
    }

    fn supports_search(&self) -> bool {
        true
    }

    async fn read_novel_info(&self, client: &Client, url: &str) -> Result<Novel> {
        let base = Self::base_from_url(url);

        let resp = client
            .get(url)
            .send()
            .await
            .context("Failed to GET novel page")?;
        let body = resp.text().await?;

        // Extract all metadata in a scoped block so `Html` (which is !Send)
        // is dropped before the next `.await`.
        let (title, author, cover_url, synopsis) = {
            let doc = Html::parse_document(&body);

            let title = select_first_text(&doc, "h3.title")
                .or_else(|| select_first_text(&doc, ".book h3"))
                .or_else(|| select_first_text(&doc, "h1"))
                .unwrap_or_else(|| "Unknown Title".to_string());

            let author = {
                let sel = Selector::parse(".info-meta li").unwrap();
                doc.select(&sel)
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
                .map(|src| absolute_url(&base, &src));

            let synopsis = select_first_text(&doc, ".desc-text");

            (title, author, cover_url, synopsis)
            // `doc` is dropped here — before any `.await`
        };

        let mut novel = Novel::new(url);
        novel.title = title;
        novel.author = author;
        novel.cover_url = cover_url;
        novel.synopsis = synopsis;

        // Chapter list (may be paginated)
        let chapter_pairs = self.fetch_all_chapters(&client, &base, &body).await?;
        novel.chapters = chapter_pairs
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
            .context("Failed to GET chapter")?;
        let body = resp.text().await?;
        let doc = Html::parse_document(&body);

        // Primary selector; fall back to broader container
        let content_sel = Selector::parse("#chapter-content").expect("valid");
        let fallback_sel = Selector::parse(".chapter-body").expect("valid");

        let inner = doc
            .select(&content_sel)
            .next()
            .or_else(|| doc.select(&fallback_sel).next())
            .map(|el| el.inner_html());

        chapter.body = inner.map(|h| clean_chapter_html(&h, &chapter.url));
        Ok(())
    }

    async fn search(&self, client: &Client, query: &str) -> Result<Vec<SearchResult>> {
        let url = format!(
            "https://novelfull.com/search?keyword={}",
            query.replace(' ', "+")
        );
        let resp = client.get(&url).send().await?;
        let body = resp.text().await?;
        let doc = Html::parse_document(&body);

        let item_sel = Selector::parse(".col-truyen-main .row[itemtype*='Book']").expect("valid");
        let title_sel = Selector::parse("h3.truyen-title a").expect("valid");
        let author_sel = Selector::parse(".author").expect("valid");
        let cover_sel = Selector::parse("img").expect("valid");

        let mut results = Vec::new();
        for item in doc.select(&item_sel) {
            let frag_html = item.inner_html();
            let frag = Html::parse_fragment(&frag_html);

            let title_el = frag.select(&title_sel).next();
            let title = title_el
                .map(|el| el.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            let url = title_el
                .and_then(|el| el.value().attr("href"))
                .map(|h| absolute_url("https://novelfull.com", h))
                .unwrap_or_default();

            results.push(SearchResult {
                title,
                url,
                author: frag
                    .select(&author_sel)
                    .next()
                    .map(|el| el.text().collect::<String>().trim().to_string()),
                cover_url: frag
                    .select(&cover_sel)
                    .next()
                    .and_then(|el| el.value().attr("src"))
                    .map(|s| absolute_url("https://novelfull.com", s)),
                synopsis: None,
                source_name: self.name().to_string(),
            });
        }
        Ok(results)
    }
}
