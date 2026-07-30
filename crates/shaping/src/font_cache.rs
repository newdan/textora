//! Font database cache: skips expensive font file parsing on subsequent startups.
//!
//! Strategy:
//! 1. First run: FontSystem::new() parses all fonts → serialize FaceInfo list to disk.
//! 2. Subsequent runs: load serialized FaceInfo → push_face_info() (zero parse cost).
//! 3. Invalidation: font directory mtimes vs cache file mtime + cache format version.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use cosmic_text::FontSystem;
use cosmic_text::fontdb::{self, Database, FaceInfo, ID, Source, Style};
use serde::{Deserialize, Serialize};

/// Bump when cache format changes.
const CACHE_VERSION: u32 = 1;

/// Font directories whose mtime is checked for cache invalidation.
#[cfg(target_os = "macos")]
const FONT_DIRS: &[&str] = &["/System/Library/Fonts", "/Library/Fonts", "/Network/Library/Fonts"];

#[cfg(not(target_os = "macos"))]
const FONT_DIRS: &[&str] = &["/usr/share/fonts", "/usr/local/share/fonts"];

// ---------------------------------------------------------------------------
// Serialized cache format
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct CachedFace {
    source_path: PathBuf,
    index: u32,
    /// Family names only (language is set to English_UnitedStates on load).
    family_names: Vec<String>,
    post_script_name: String,
    style: u8,
    weight: u16,
    stretch: u16,
    monospaced: bool,
}

#[derive(Serialize, Deserialize)]
struct FontDbCache {
    version: u32,
    faces: Vec<CachedFace>,
}

// ---------------------------------------------------------------------------
// Style / Stretch ↔ numeric
// ---------------------------------------------------------------------------

fn style_to_u8(s: Style) -> u8 {
    match s {
        Style::Normal => 0,
        Style::Italic => 1,
        Style::Oblique => 2,
    }
}

fn style_from_u8(v: u8) -> Style {
    match v {
        1 => Style::Italic,
        2 => Style::Oblique,
        _ => Style::Normal,
    }
}

fn stretch_to_u16(s: fontdb::Stretch) -> u16 {
    s.to_number()
}

fn stretch_from_u16(v: u16) -> fontdb::Stretch {
    match v {
        1 => fontdb::Stretch::UltraCondensed,
        2 => fontdb::Stretch::ExtraCondensed,
        3 => fontdb::Stretch::Condensed,
        4 => fontdb::Stretch::SemiCondensed,
        6 => fontdb::Stretch::SemiExpanded,
        7 => fontdb::Stretch::Expanded,
        8 => fontdb::Stretch::ExtraExpanded,
        9 => fontdb::Stretch::UltraExpanded,
        _ => fontdb::Stretch::Normal,
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Build a `FontSystem`, preferring a cached font database when available.
///
/// On cache hit: recreates the Database from serialized FaceInfo via `push_face_info()`,
/// skipping all font file parsing.
///
/// On cache miss: calls `FontSystem::new()` (full system font scan) and saves
/// the resulting face metadata to disk for the next run.
pub fn new_font_system_with_cache(cache_path: &Path) -> FontSystem {
    if let Some(fs) = load_from_cache(cache_path) {
        return fs;
    }

    let fs = FontSystem::new();

    // Save face metadata for next startup
    if let Err(e) = save_to_cache(fs.db(), cache_path) {
        eprintln!("[startup] FontSystem cache save failed: {e}");
    }

    fs
}

/// Returns the default cache file path.
pub fn default_cache_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join("Library/Caches/edit+/fontdb.cache"))
}

// ---------------------------------------------------------------------------
// Internal: cache I/O
// ---------------------------------------------------------------------------

fn load_from_cache(cache_path: &Path) -> Option<FontSystem> {
    let data = fs::read(cache_path).ok()?;

    if !cache_is_fresh(cache_path) {
        let _ = fs::remove_file(cache_path);
        return None;
    }

    let cache: FontDbCache = bincode::deserialize(&data).ok()?;

    if cache.version != CACHE_VERSION {
        let _ = fs::remove_file(cache_path);
        return None;
    }

    let mut db = Database::new();
    set_default_families(&mut db);

    for cf in &cache.faces {
        if !cf.source_path.exists() {
            continue;
        }

        let families: Vec<(String, fontdb::Language)> = cf
            .family_names
            .iter()
            .map(|name| (name.clone(), fontdb::Language::English_UnitedStates))
            .collect();

        let info = FaceInfo {
            id: ID::dummy(),
            source: Source::File(cf.source_path.clone()),
            index: cf.index,
            families,
            post_script_name: cf.post_script_name.clone(),
            style: style_from_u8(cf.style),
            weight: fontdb::Weight(cf.weight),
            stretch: stretch_from_u16(cf.stretch),
            monospaced: cf.monospaced,
        };
        db.push_face_info(info);
    }

    let locale = sys_locale::get_locale().unwrap_or_else(|| "en-US".into());
    Some(FontSystem::new_with_locale_and_db(locale, db))
}

fn save_to_cache(db: &Database, cache_path: &Path) -> io::Result<()> {
    let faces: Vec<CachedFace> = db
        .faces()
        .filter_map(|face| {
            let source_path = match &face.source {
                Source::File(p) | Source::SharedFile(p, _) => p.clone(),
                Source::Binary(_) => return None,
            };
            let family_names: Vec<String> =
                face.families.iter().map(|(name, _lang)| name.clone()).collect();

            Some(CachedFace {
                source_path,
                index: face.index,
                family_names,
                post_script_name: face.post_script_name.clone(),
                style: style_to_u8(face.style),
                weight: face.weight.0,
                stretch: stretch_to_u16(face.stretch),
                monospaced: face.monospaced,
            })
        })
        .collect();

    let cache = FontDbCache { version: CACHE_VERSION, faces };

    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = bincode::serialize(&cache).map_err(io::Error::other)?;
    fs::write(cache_path, data)?;
    Ok(())
}

fn cache_is_fresh(cache_path: &Path) -> bool {
    let cache_mtime = match fs::metadata(cache_path).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return false,
    };

    let dirs = collect_font_dirs();
    for dir in &dirs {
        if let Ok(meta) = fs::metadata(dir)
            && let Ok(dir_mtime) = meta.modified()
            && dir_mtime > cache_mtime
        {
            return false;
        }
    }

    // Check ~/Library/Fonts
    if let Ok(home) = std::env::var("HOME") {
        let user_fonts = PathBuf::from(home).join("Library/Fonts");
        if let Ok(meta) = fs::metadata(&user_fonts)
            && let Ok(dir_mtime) = meta.modified()
            && dir_mtime > cache_mtime
        {
            return false;
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn set_default_families(db: &mut Database) {
    db.set_monospace_family("Fira Mono");
    db.set_sans_serif_family("Fira Sans");
    db.set_serif_family("DejaVu Serif");
}

fn collect_font_dirs() -> HashSet<PathBuf> {
    let mut dirs: HashSet<PathBuf> = FONT_DIRS.iter().map(|d| PathBuf::from(*d)).collect();

    #[cfg(target_os = "macos")]
    {
        if let Ok(entries) = fs::read_dir("/System/Library/AssetsV2") {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with("com_apple_MobileAsset_Font") {
                    dirs.insert(entry.path());
                }
            }
        }
    }

    dirs
}
