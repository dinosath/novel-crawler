//! Crawler for https://www.scribblehub.com

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};

use crate::crawler::{clean_chapter_html, select_first_attr, select_first_text, Crawler};
use crate::models::{Chapter, Novel, SearchResult};

pub struct ScribbleHubCrawler;

impl ScribbleHubCrawler {
    pub fn new() -> Self {
        ScribbleHubCrawler
    }

    /// Extract the fiction numeric ID from a ScribbleHub URL.
    /// e.g. /series/12345/my-novel/ → 12345
    fn extract_post_id(url: &str) -> Option<u64> {
        let re = regex::Regex::new(r"/series/(\d+)/").ok()?;
        let caps = re.captures(url)?;
        caps[1].parse().ok()
    }

    /// Fetch paginated chapter list via the AJAX endpoint.
    async fn fetch_chapters(
        &self,
        client: &Client,
        post_id: u64,
    ) -> Result<Vec<(String, String)>> {
        let mut pairs: Vec<(String, String)> = Vec::new();
        let mut page: u32 = 1;

        loop {
            let resp = client
                .post("https://www.scribblehub.com/wp-admin/admin-ajax.php")
                .form(&[
                    ("action", "wi_getreleases_n"),
                    ("pagenum", &page.to_string()),
                    ("mypostid", &post_id.to_string()),
                ])
                .send()
                .await?;

            let body = resp.text().await?;
            let doc = Html::parse_fragment(&body);
            let a_sel = Selector::parse("li.toc_w a").expect("valid");

            let count_before = pairs.len();
            for a in doc.select(&a_sel) {
                let url = a.value().attr("href").unwrap_or_default().to_string();
                let title = a
                    .value()
                    .attr("title")
                    .map(|s| s.trim().to_string())
                    .or_else(|| {
                        let t = a.text().collect::<String>().trim().to_string();
                        if t.is_empty() { None } else { Some(t) }
                    })
                    .unwrap_or_else(|| format!("Chapter {}", pairs.len() + 1));
                if !url.is_empty() {
                    pairs.push((title, url));
                }
            }

            if pairs.len() == count_before {
                break; // no new items → last page
            }
            page += 1;
            if page > 1000 {
                break;
            }
        }

        // ScribbleHub returns chapters newest-first; reverse to reading order.
        pairs.reverse();
        Ok(pairs)
    }
}

#[async_trait]
impl Crawler for ScribbleHubCrawler {
    fn name(&self) -> &str {
        "ScribbleHub"
    }

    fn base_urls(&self) -> &[&'static str] {
        &["https://www.scribblehub.com"]
    }

    fn supports_search(&self) -> bool {
        true
    }

    async fn read_novel_info(&self, client: &Client, url: &str) -> Result<Novel> {
        let post_id = Self::extract_post_id(url)
            .context("Could not extract ScribbleHub fiction ID from URL")?;

        let resp = client
            .get(url)
            .send()
            .await
            .context("Failed to GET ScribbleHub novel page")?;
        let body = resp.text().await?;

        // Extract metadata in a block so `Html` is dropped before the next await.
        let (title, author, cover_url, synopsis) = {
            let doc = Html::parse_document(&body);
            let title = select_first_text(&doc, ".fic-title h1")
                .or_else(|| select_first_text(&doc, "h1"))
                .unwrap_or_else(|| "Unknown Title".to_string());
            let author = select_first_text(&doc, ".auth a");
            let cover_url = select_first_attr(&doc, ".fic-header img", "src");
            let synopsis = select_first_text(&doc, ".wi_fic_desc");
            (title, author, cover_url, synopsis)
            // `doc` dropped here
        };

        let mut novel = Novel::new(url);
        novel.title = title;
        novel.author = author;
        novel.cover_url = cover_url;
        novel.synopsis = synopsis;

        let chapter_pairs = self.fetch_chapters(client, post_id).await?;
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
            .context("Failed to GET ScribbleHub chapter")?;
        let body = resp.text().await?;
        let doc = Html::parse_document(&body);

        let sel = Selector::parse(".chp_raw").expect("valid");
        if let Some(el) = doc.select(&sel).next() {
            chapter.body = Some(clean_chapter_html(&el.inner_html(), &chapter.url));
        }
        Ok(())
    }

    async fn search(&self, client: &Client, query: &str) -> Result<Vec<SearchResult>> {
        let url = format!(
            "https://www.scribblehub.com/?s={}&post_type=fictionposts",
            query.replace(' ', "+")
        );
        let resp = client.get(&url).send().await?;
        let body = resp.text().await?;
        let doc = Html::parse_document(&body);

        let item_sel = Selector::parse(".search_main_box").expect("valid");
        let title_sel = Selector::parse(".search_title a").expect("valid");
        let author_sel = Selector::parse(".search_author").expect("valid");
        let cover_sel = Selector::parse("img.lazy").expect("valid");

        let mut results = Vec::new();
        for item in doc.select(&item_sel) {
            let frag = Html::parse_fragment(&item.inner_html());

            let tit_el = frag.select(&title_sel).next();
            let title = tit_el
                .map(|el| el.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            let url = tit_el
                .and_then(|el| el.value().attr("href"))
                .map(|s| s.to_string())
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
                    .and_then(|el| {
                        el.value().attr("data-src").or_else(|| el.value().attr("src"))
                    })
                    .map(|s| s.to_string()),
                synopsis: None,
                source_name: self.name().to_string(),
            });
        }
        Ok(results)
    }
}
