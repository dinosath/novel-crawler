//! EPUB 3 output.
//!
//! Generates a standards-compliant EPUB 3 file (which is a ZIP archive with
//! a specific directory layout).  No external epub crate is required — the
//! `zip` crate is used directly.

use anyhow::Result;
use std::io::Write;
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

use crate::models::{Chapter, Novel};

// ── EPUB skeleton templates ──────────────────────────────────────────────────

const CONTAINER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0"
           xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf"
              media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

const STYLESHEET: &str = r#"body {
  font-family: Georgia, "Times New Roman", serif;
  line-height: 1.6;
  margin: 2em;
  max-width: 40em;
}
h1, h2, h3 { margin-top: 1.5em; }
p { margin: 0.5em 0; text-indent: 1.5em; }
p:first-of-type { text-indent: 0; }
"#;

// ── Entry point ──────────────────────────────────────────────────────────────

pub fn write(novel: &Novel, chapters: &[&Chapter], path: &Path) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);

    // mimetype MUST be the first file and MUST be stored (no compression).
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zip.start_file("mimetype", stored)?;
    zip.write_all(b"application/epub+zip")?;

    // container.xml
    zip.start_file("META-INF/container.xml", deflated)?;
    zip.write_all(CONTAINER_XML.as_bytes())?;

    // Stylesheet
    zip.start_file("OEBPS/css/style.css", deflated)?;
    zip.write_all(STYLESHEET.as_bytes())?;

    // Chapter XHTML files
    for chapter in chapters {
        let filename = format!("OEBPS/chapters/ch{:04}.xhtml", chapter.id);
        zip.start_file(&filename, deflated)?;
        zip.write_all(build_chapter_xhtml(chapter).as_bytes())?;
    }

    // Navigation document (EPUB 3)
    zip.start_file("OEBPS/nav.xhtml", deflated)?;
    zip.write_all(build_nav_xhtml(novel, chapters).as_bytes())?;

    // NCX (EPUB 2 fallback)
    zip.start_file("OEBPS/toc.ncx", deflated)?;
    zip.write_all(build_ncx(novel, chapters).as_bytes())?;

    // Package document (content.opf)
    zip.start_file("OEBPS/content.opf", deflated)?;
    zip.write_all(build_opf(novel, chapters).as_bytes())?;

    zip.finish()?;
    Ok(())
}

// ── Template builders ────────────────────────────────────────────────────────

fn build_chapter_xhtml(chapter: &Chapter) -> String {
    let body_html = chapter
        .body
        .as_deref()
        .unwrap_or("<p>[Chapter content not available]</p>");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml">
<head>
  <meta charset="utf-8"/>
  <title>{title}</title>
  <link rel="stylesheet" type="text/css" href="../css/style.css"/>
</head>
<body>
  <h2>{title}</h2>
  <div class="chapter-content">
    {body}
  </div>
</body>
</html>"#,
        title = xml_escape(&chapter.title),
        body = body_html,
    )
}

fn build_nav_xhtml(novel: &Novel, chapters: &[&Chapter]) -> String {
    let items: String = chapters
        .iter()
        .map(|c| {
            format!(
                r#"      <li><a href="chapters/ch{id:04}.xhtml">{title}</a></li>"#,
                id = c.id,
                title = xml_escape(&c.title)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml"
      xmlns:epub="http://www.idpf.org/2007/ops">
<head>
  <meta charset="utf-8"/>
  <title>Table of Contents</title>
</head>
<body>
  <nav epub:type="toc" id="toc">
    <h1>Table of Contents</h1>
    <ol>
{items}
    </ol>
  </nav>
</body>
</html>"#,
        items = items
    )
}

fn build_ncx(novel: &Novel, chapters: &[&Chapter]) -> String {
    let nav_points: String = chapters
        .iter()
        .enumerate()
        .map(|(i, c)| {
            format!(
                r#"  <navPoint id="navpoint-{idx}" playOrder="{idx}">
    <navLabel><text>{title}</text></navLabel>
    <content src="chapters/ch{id:04}.xhtml"/>
  </navPoint>"#,
                idx = i + 1,
                id = c.id,
                title = xml_escape(&c.title),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head>
    <meta name="dtb:uid" content="urn:uuid:lncrawl-rs"/>
  </head>
  <docTitle><text>{title}</text></docTitle>
  <navMap>
{nav_points}
  </navMap>
</ncx>"#,
        title = xml_escape(&novel.title),
        nav_points = nav_points,
    )
}

fn build_opf(novel: &Novel, chapters: &[&Chapter]) -> String {
    let author = novel.author.as_deref().unwrap_or("Unknown Author");
    let lang = &novel.language;

    // manifest items for chapters
    let chapter_manifest: String = chapters
        .iter()
        .map(|c| {
            format!(
                r#"    <item id="ch{id:04}" href="chapters/ch{id:04}.xhtml" media-type="application/xhtml+xml"/>"#,
                id = c.id
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    // spine itemrefs
    let spine_items: String = chapters
        .iter()
        .map(|c| format!(r#"    <itemref idref="ch{:04}"/>"#, c.id))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf"
         version="3.0"
         unique-identifier="bookid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>{title}</dc:title>
    <dc:creator>{author}</dc:creator>
    <dc:language>{lang}</dc:language>
    <dc:identifier id="bookid">urn:uuid:lncrawl-rs-{slug}</dc:identifier>
    <meta property="dcterms:modified">{date}</meta>
  </metadata>
  <manifest>
    <item id="nav"  href="nav.xhtml"      media-type="application/xhtml+xml" properties="nav"/>
    <item id="ncx"  href="toc.ncx"        media-type="application/x-dtbncx+xml"/>
    <item id="css"  href="css/style.css"  media-type="text/css"/>
{chapter_manifest}
  </manifest>
  <spine toc="ncx">
{spine_items}
  </spine>
</package>"#,
        title = xml_escape(&novel.title),
        author = xml_escape(author),
        lang = lang,
        slug = slugify(&novel.title),
        date = chrono_now(),
        chapter_manifest = chapter_manifest,
        spine_items = spine_items,
    )
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c.to_lowercase().next().unwrap_or('_') } else { '_' })
        .collect()
}

fn chrono_now() -> String {
    // ISO 8601 UTC timestamp without pulling in chrono
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Very rough conversion to YYYY-MM-DDThh:mm:ssZ
    let days_total = secs / 86400;
    let year = 1970 + days_total / 365;
    format!("{}-01-01T00:00:00Z", year)
}
