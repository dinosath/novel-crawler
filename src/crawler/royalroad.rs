//! Crawler for https://www.royalroad.com

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};

use crate::crawler::{absolute_url, clean_chapter_html, percent_encode, select_first_attr, select_first_text, Crawler};
use crate::models::{Chapter, Novel, SearchResult, Volume};

pub struct RoyalRoadCrawler;

impl RoyalRoadCrawler {
    pub fn new() -> Self {
        RoyalRoadCrawler
    }
}

#[async_trait]
impl Crawler for RoyalRoadCrawler {
    fn name(&self) -> &str {
        "Royal Road"
    }

    fn base_urls(&self) -> &[&'static str] {
        &["https://www.royalroad.com"]
    }

    fn supports_search(&self) -> bool {
        true
    }

    async fn read_novel_info(&self, client: &Client, url: &str) -> Result<Novel> {
        let resp = client
            .get(url)
            .send()
            .await
            .context("Failed to GET novel page")?;
        let body = resp.text().await?;
        let doc = Html::parse_document(&body);

        let mut novel = Novel::new(url);

        // Title
        novel.title = select_first_text(&doc, "h1.font-white")
            .or_else(|| select_first_text(&doc, ".fic-title h1"))
            .or_else(|| select_first_text(&doc, "h1"))
            .unwrap_or_else(|| "Unknown Title".to_string());

        // Author
        novel.author = select_first_text(&doc, "span[property='name']")
            .or_else(|| select_first_text(&doc, ".author-name-badge"));

        // Cover
        novel.cover_url = select_first_attr(&doc, ".thumbnail", "src")
            .or_else(|| select_first_attr(&doc, ".fic-header img", "src"));

        // Synopsis
        novel.synopsis = select_first_text(&doc, ".description .hidden-content")
            .or_else(|| select_first_text(&doc, ".description"));

        // Chapter list  ─ <tr class="chapter-row"> in <table id="chapters">
        let row_sel = Selector::parse("tr.chapter-row").expect("valid selector");
        let a_sel = Selector::parse("td a").expect("valid selector");

        let mut chapters: Vec<Chapter> = Vec::new();
        for row in doc.select(&row_sel) {
            if let Some(a) = row.select(&a_sel).next() {
                let href = a.value().attr("href").unwrap_or_default();
                let title = a.text().collect::<String>().trim().to_string();
                let chapter_url = absolute_url("https://www.royalroad.com", href);
                let id = chapters.len() + 1;
                chapters.push(Chapter::new(id, title, chapter_url));
            }
        }
        novel.chapters = chapters;

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

        let content_sel = Selector::parse(".chapter-content").expect("valid selector");
        if let Some(el) = doc.select(&content_sel).next() {
            chapter.body = Some(clean_chapter_html(&el.inner_html(), &chapter.url));
        }
        Ok(())
    }

    async fn search(&self, client: &Client, query: &str) -> Result<Vec<SearchResult>> {
        let url = format!(
            "https://www.royalroad.com/fictions/search?title={}",
            percent_encode(query)
        );
        let resp = client.get(&url).send().await?;
        let body = resp.text().await?;
        let doc = Html::parse_document(&body);

        let item_sel = Selector::parse(".fiction-list-item").expect("valid selector");
        let title_sel = Selector::parse("h2.fiction-title a").expect("valid selector");
        let author_sel = Selector::parse(".author").expect("valid selector");
        let cover_sel = Selector::parse("img").expect("valid selector");
        let desc_sel = Selector::parse(".fiction-description p").expect("valid selector");

        let mut results = Vec::new();
        for item in doc.select(&item_sel) {
            let item_html = Html::parse_fragment(&item.html());

            let title_el = item_html.select(&title_sel).next();
            let title = title_el
                .map(|el| el.text().collect::<String>().trim().to_string())
                .unwrap_or_default();
            let url = title_el
                .and_then(|el| el.value().attr("href"))
                .map(|h| absolute_url("https://www.royalroad.com", h))
                .unwrap_or_default();

            if title.is_empty() || url.is_empty() {
                continue;
            }

            results.push(SearchResult {
                title,
                url,
                author: item_html
                    .select(&author_sel)
                    .next()
                    .map(|el| el.text().collect::<String>().trim().to_string()),
                cover_url: item_html
                    .select(&cover_sel)
                    .next()
                    .and_then(|el| el.value().attr("src"))
                    .map(|s| s.to_string()),
                synopsis: item_html
                    .select(&desc_sel)
                    .next()
                    .map(|el| el.text().collect::<String>().trim().to_string()),
                source_name: self.name().to_string(),
            });
        }
        Ok(results)
    }
}


