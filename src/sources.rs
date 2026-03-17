/// Static metadata for every known source.
#[derive(Debug)]
pub struct SourceInfo {
    pub name: &'static str,
    pub base_url: &'static str,
    pub language: &'static str,
    pub supports_search: bool,
    /// Whether this source contains machine–translated content
    pub is_machine_translated: bool,
}

impl SourceInfo {
    const fn new(
        name: &'static str,
        base_url: &'static str,
        language: &'static str,
        supports_search: bool,
        is_machine_translated: bool,
    ) -> Self {
        SourceInfo {
            name,
            base_url,
            language,
            supports_search,
            is_machine_translated,
        }
    }
}

/// The full catalogue of supported sources, mirroring the Python project's list.
pub static SOURCES: &[SourceInfo] = &[
    // ── English ────────────────────────────────────────────────────────────
    SourceInfo::new("BrightNovels",      "https://brightnovels.com",           "en", true,  false),
    SourceInfo::new("Royal Road",        "https://www.royalroad.com",          "en", true,  false),
    SourceInfo::new("ScribbleHub",       "https://www.scribblehub.com",        "en", true,  false),
    SourceInfo::new("WebNovel",          "https://www.webnovel.com",           "en", true,  false),
    SourceInfo::new("NovelFull",         "https://novelfull.com",              "en", true,  false),
    SourceInfo::new("NovelBin",          "https://novelbin.com",               "en", true,  false),
    SourceInfo::new("NovelHulk",         "https://novelhulk.com",              "en", true,  false),
    SourceInfo::new("ReadNovelFull",     "https://readnovelfull.com",          "en", true,  false),
    SourceInfo::new("BoxNovel",          "https://boxnovel.com",               "en", true,  false),
    SourceInfo::new("LightNovelPub",     "https://www.lightnovelworld.com",     "en", true,  false),
    SourceInfo::new("WuxiaWorld",        "https://www.wuxiaworld.com",         "en", true,  false),
    SourceInfo::new("VolarNovels",       "https://www.volarenovels.com",       "en", true,  false),
    SourceInfo::new("AllNovel",          "https://allnovel.org",               "en", true,  false),
    SourceInfo::new("FreeWebNovel",      "https://freewebnovel.com",           "en", true,  false),
    SourceInfo::new("LibRead",           "https://libread.com",                "en", true,  false),
    SourceInfo::new("NovelNext",         "https://novelnext.com",              "en", true,  false),
    SourceInfo::new("NovelGate",         "https://novelgate.net",              "en", true,  false),
    SourceInfo::new("FanFiction",        "https://www.fanfiction.net",         "en", true,  false),
    SourceInfo::new("FictionPress",      "https://www.fictionpress.com",       "en", true,  false),
    SourceInfo::new("LnMTL",            "https://lnmtl.com",                  "en", true,  true ),
    SourceInfo::new("Ranobes",           "https://ranobes.net",                "en", true,  false),
    SourceInfo::new("MTLNation",         "https://mtlnation.com",              "en", true,  true ),
    SourceInfo::new("ReadWebNovels",     "https://readwebnovels.net",          "en", true,  false),
    SourceInfo::new("WuxiaNovels",       "https://wuxiaworld.online",          "en", false, false),
    SourceInfo::new("NobleMTL",         "https://noblemtl.com",               "en", true,  false),
    SourceInfo::new("FansTrans",         "https://fanstranslations.com",       "en", true,  false),
    SourceInfo::new("DaoTranslate",      "https://daotranslate.us",            "en", true,  true ),
    SourceInfo::new("Webnovel (mobile)", "https://m.webnovel.com",             "en", true,  false),
    SourceInfo::new("WuxiaBox",          "https://www.wuxiabox.com",           "en", true,  true ),
    SourceInfo::new("ReadMTL",          "https://readmtl.com",                "en", true,  true ),
    SourceInfo::new("DaoNovel",          "https://daonovel.com",               "en", true,  false),
    SourceInfo::new("BestLightNovel",    "https://bestlightnovel.com",         "en", true,  false),
    SourceInfo::new("NovelOnlineFree",   "https://novelonlinefree.com",        "en", true,  false),
    SourceInfo::new("WuxiaMTL",         "https://www.wuxiamtl.com",           "en", true,  true ),
    SourceInfo::new("WanderingInn",      "https://wanderinginn.com",           "en", false, false),
    SourceInfo::new("MCTranslation",     "https://www.machine-translation.org","en", true,  true ),
    // ── Chinese ────────────────────────────────────────────────────────────
    SourceInfo::new("69Shu",            "https://www.69shu.com",              "zh", true,  false),
    SourceInfo::new("27K",              "https://www.27k.net",                "zh", true,  false),
    SourceInfo::new("PiaoTian",         "https://www.piaotian.com",           "zh", true,  false),
    SourceInfo::new("BanXia",           "https://www.banxia.cc",              "zh", true,  false),
    // ── Vietnamese ─────────────────────────────────────────────────────────
    SourceInfo::new("DocLN",            "https://docln.net",                  "vi", true,  true ),
    SourceInfo::new("TruyenFull",       "https://truyenfull.vn",              "vi", true,  true ),
    // ── Japanese ───────────────────────────────────────────────────────────
    SourceInfo::new("Syosetu",          "https://ncode.syosetu.com",          "ja", true,  false),
    // ── Korean ─────────────────────────────────────────────────────────────
    // ── Indonesian ─────────────────────────────────────────────────────────
    SourceInfo::new("NovelKu",          "https://novelku.id",                 "id", false, false),
    SourceInfo::new("MeioNovel",        "https://meionovel.id",               "id", true,  false),
    // ── Arabic ─────────────────────────────────────────────────────────────
    SourceInfo::new("ArNovel",          "https://arnovel.me",                 "ar", true,  false),
    // ── French ─────────────────────────────────────────────────────────────
    SourceInfo::new("LightNovelFR",     "https://lightnovelfr.com",           "fr", true,  false),
    // ── Portuguese ─────────────────────────────────────────────────────────
    SourceInfo::new("CentralNovel",     "https://centralnovel.com",           "pt", true,  false),
    // ── Russian ────────────────────────────────────────────────────────────
    SourceInfo::new("RanobeLib",        "https://ranobelib.me",               "ru", false, false),
];

/// Find a source by exact base URL (lowercased host comparison).
pub fn find_source_for_url(url: &str) -> Option<&'static SourceInfo> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_lowercase();
    SOURCES.iter().find(|s| {
        if let Ok(su) = url::Url::parse(s.base_url) {
            su.host_str()
                .map(|h| h.to_lowercase() == host || host.ends_with(&format!(".{}", h.to_lowercase())))
                .unwrap_or(false)
        } else {
            false
        }
    })
}

/// Filter sources by a regex pattern (matches name or base_url).
pub fn filter_sources(pattern: &str) -> Vec<&'static SourceInfo> {
    match regex::Regex::new(&format!("(?i){}", pattern)) {
        Ok(re) => SOURCES
            .iter()
            .filter(|s| re.is_match(s.name) || re.is_match(s.base_url))
            .collect(),
        Err(_) => Vec::new(),
    }
}
