use serde::{Deserialize, Serialize};

/// Represents a light novel with metadata and chapters.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Novel {
    pub title: String,
    pub url: String,
    pub author: Option<String>,
    pub cover_url: Option<String>,
    pub synopsis: Option<String>,
    pub language: String,
    pub volumes: Vec<Volume>,
    pub chapters: Vec<Chapter>,
}

impl Novel {
    pub fn new(url: impl Into<String>) -> Self {
        Novel {
            url: url.into(),
            language: "en".to_string(),
            ..Default::default()
        }
    }

    pub fn chapters_in_volume(&self, volume_id: usize) -> Vec<&Chapter> {
        self.chapters
            .iter()
            .filter(|c| c.volume_id == Some(volume_id))
            .collect()
    }

    pub fn chapter_range_title(&self) -> String {
        let n = self.chapters.len();
        if n == 0 {
            return String::new();
        }
        if n == 1 {
            return format!("c{:04}", self.chapters[0].id);
        }
        format!(
            "c{:04}-c{:04}",
            self.chapters.first().unwrap().id,
            self.chapters.last().unwrap().id
        )
    }
}

/// A volume groups chapters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Volume {
    pub id: usize,
    pub title: String,
}

impl Volume {
    pub fn new(id: usize, title: impl Into<String>) -> Self {
        Volume {
            id,
            title: title.into(),
        }
    }
}

/// A single chapter (initially without content).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Chapter {
    /// Sequential 1-based index in this download
    pub id: usize,
    pub title: String,
    pub url: String,
    pub volume_id: Option<usize>,
    /// Raw HTML body; None until downloaded
    pub body: Option<String>,
}

impl Chapter {
    pub fn new(id: usize, title: impl Into<String>, url: impl Into<String>) -> Self {
        Chapter {
            id,
            title: title.into(),
            url: url.into(),
            ..Default::default()
        }
    }

    pub fn plain_text(&self) -> String {
        match &self.body {
            None => String::new(),
            Some(html) => html_to_text(html),
        }
    }
}

/// Result entry from a source search.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub author: Option<String>,
    pub cover_url: Option<String>,
    pub synopsis: Option<String>,
    pub source_name: String,
}

/// Supported output formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
    Epub,
    Txt,
}

impl OutputFormat {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "json" => Some(OutputFormat::Json),
            "epub" => Some(OutputFormat::Epub),
            "txt" | "text" => Some(OutputFormat::Txt),
            _ => None,
        }
    }

    pub fn extension(&self) -> &str {
        match self {
            OutputFormat::Json => "json",
            OutputFormat::Epub => "epub",
            OutputFormat::Txt => "txt",
        }
    }

    pub fn all_formats() -> Vec<OutputFormat> {
        vec![OutputFormat::Json, OutputFormat::Epub, OutputFormat::Txt]
    }
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.extension())
    }
}

/// Strip HTML tags to plain text.
pub fn html_to_text(html: &str) -> String {
    use scraper::{Html, Selector};
    let frag = Html::parse_fragment(html);
    let body_sel = Selector::parse("body").unwrap();
    let text: String = if let Some(body) = frag.select(&body_sel).next() {
        body.text().collect::<Vec<_>>().join("")
    } else {
        frag.root_element().text().collect::<Vec<_>>().join("")
    };
    // Collapse whitespace
    let mut result = String::new();
    let mut prev_newline = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !prev_newline {
                result.push('\n');
            }
            prev_newline = true;
        } else {
            result.push_str(trimmed);
            result.push('\n');
            prev_newline = false;
        }
    }
    result
}
