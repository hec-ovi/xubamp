//! Pure reading and writing of `.m3u`, `.m3u8`, and `.pls` playlist files. No filesystem access:
//! the caller reads the file into a `&str`, hands it here, and receives resolved entries; writing
//! goes the other way. Keeping I/O out means every format quirk (BOM, CRLF, `#EXTINF` metadata,
//! PLS index gaps) is unit-tested without touching disk. The `base_dir` argument is the playlist
//! file's own directory, used to turn relative entries into absolute paths and back again.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One resolved playlist entry. `path` is absolute when a relative entry was resolved against a
/// `base_dir`; absolute entries and URLs are kept verbatim. `title` and `duration_secs` are only
/// present when the file carried them (`#EXTINF` for M3U, `TitleN`/`LengthN` for PLS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistEntry {
    pub path: PathBuf,
    pub title: Option<String>,
    pub duration_secs: Option<u32>,
}

/// The two on-disk families this module understands. `.m3u` and `.m3u8` share a parser here (the
/// `8` only promises UTF-8, which is already the caller's concern once the text is a `&str`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistFormat {
    M3u,
    Pls,
}

/// Detect the format from a path's extension: `.m3u`/`.m3u8` are [`PlaylistFormat::M3u`], `.pls` is
/// [`PlaylistFormat::Pls`], anything else (or no extension) is `None`. Case-insensitive.
pub fn format_from_path(path: &Path) -> Option<PlaylistFormat> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "m3u" | "m3u8" => Some(PlaylistFormat::M3u),
        "pls" => Some(PlaylistFormat::Pls),
        _ => None,
    }
}

/// Parse playlist text of a known `format`. `base_dir` resolves relative entry paths (pass the
/// playlist file's directory, or `None` to keep entries exactly as written). A leading UTF-8 BOM is
/// stripped; unparseable, comment, and blank lines are skipped rather than treated as errors, so a
/// partially-corrupt file still yields whatever entries it does contain.
pub fn parse(text: &str, format: PlaylistFormat, base_dir: Option<&Path>) -> Vec<PlaylistEntry> {
    // A BOM only has meaning at the very start of the file; strip it before line splitting so the
    // first real line is not silently misread as a comment or a bogus path.
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    match format {
        PlaylistFormat::M3u => parse_m3u(text, base_dir),
        PlaylistFormat::Pls => parse_pls(text, base_dir),
    }
}

/// Serialize to extended M3U: a `#EXTM3U` header, then a `#EXTINF:<secs>,<title>` line paired with
/// each entry's path. Unknown duration is written as `-1` and a missing title as an empty string,
/// both of which parse back to `None`. When `base_dir` is given, entries beneath it are written
/// relative to it (so the playlist stays portable); everything else is written as stored.
pub fn write_m3u(entries: &[PlaylistEntry], base_dir: Option<&Path>) -> String {
    let mut out = String::from("#EXTM3U\n");
    for entry in entries {
        let secs = entry.duration_secs.map(i64::from).unwrap_or(-1);
        let title = entry.title.as_deref().unwrap_or("");
        out.push_str(&format!("#EXTINF:{secs},{title}\n"));
        out.push_str(&display_path(&entry.path, base_dir));
        out.push('\n');
    }
    out
}

/// Serialize to PLS v2: a `[playlist]` header, then `FileN`/`TitleN`/`LengthN` for each 1-based
/// entry, closed by `NumberOfEntries` and `Version=2`. Unknown duration is written as `-1` and a
/// missing title as an empty value. Relative-ization under `base_dir` matches [`write_m3u`].
pub fn write_pls(entries: &[PlaylistEntry], base_dir: Option<&Path>) -> String {
    let mut out = String::from("[playlist]\n");
    for (i, entry) in entries.iter().enumerate() {
        let n = i + 1;
        let file = display_path(&entry.path, base_dir);
        let title = entry.title.as_deref().unwrap_or("");
        let length = entry.duration_secs.map(i64::from).unwrap_or(-1);
        out.push_str(&format!("File{n}={file}\n"));
        out.push_str(&format!("Title{n}={title}\n"));
        out.push_str(&format!("Length{n}={length}\n"));
    }
    out.push_str(&format!("NumberOfEntries={}\n", entries.len()));
    out.push_str("Version=2\n");
    out
}

fn parse_m3u(text: &str, base_dir: Option<&Path>) -> Vec<PlaylistEntry> {
    let mut entries = Vec::new();
    // Metadata from the most recent `#EXTINF`, held until the next path line consumes it. Comments
    // between the two do not clear it; a second `#EXTINF` overwrites it.
    let mut pending: Option<(Option<u32>, Option<String>)> = None;

    // `lines()` splits on `\n` and drops a trailing `\r`, covering both CRLF and LF; the extra
    // trim also absorbs a stray `\r` or surrounding whitespace.
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            pending = Some(parse_extinf(rest));
            continue;
        }
        if line.starts_with('#') {
            // `#EXTM3U` header and any other `#` line are comments; leave `pending` untouched.
            continue;
        }
        let (duration, title) = pending.take().unwrap_or((None, None));
        entries.push(PlaylistEntry {
            path: resolve_path(line, base_dir),
            title,
            duration_secs: duration,
        });
    }
    entries
}

/// Split the text after `#EXTINF:` into a duration and a title. The title is everything after the
/// FIRST comma, so a title that itself contains commas survives intact. A `-1` (or any negative or
/// unparseable) duration becomes `None`, and an empty title becomes `None`.
fn parse_extinf(rest: &str) -> (Option<u32>, Option<String>) {
    let (secs, title) = match rest.split_once(',') {
        Some((secs, title)) => (secs, Some(title)),
        None => (rest, None),
    };
    let duration = parse_duration(secs);
    let title = title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string);
    (duration, title)
}

fn parse_pls(text: &str, base_dir: Option<&Path>) -> Vec<PlaylistEntry> {
    // Keyed by the 1-based index N so ascending iteration yields display order regardless of the
    // order keys appear in the file. A BTreeMap also tolerates gaps for free.
    let mut files: BTreeMap<u32, String> = BTreeMap::new();
    let mut titles: BTreeMap<u32, String> = BTreeMap::new();
    let mut lengths: BTreeMap<u32, String> = BTreeMap::new();

    for raw in text.lines() {
        let line = raw.trim();
        // Skip blanks, the `[playlist]` section header, and INI comments.
        if line.is_empty() || line.starts_with('[') || line.starts_with(';') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().to_string();
        if let Some(n) = key.strip_prefix("file").and_then(parse_index) {
            files.insert(n, value);
        } else if let Some(n) = key.strip_prefix("title").and_then(parse_index) {
            titles.insert(n, value);
        } else if let Some(n) = key.strip_prefix("length").and_then(parse_index) {
            lengths.insert(n, value);
        }
        // `NumberOfEntries`, `Version`, and anything unrecognized are metadata we can rebuild, so
        // they are dropped: the entry count is derived from the `File` keys actually present.
    }

    files
        .into_iter()
        .map(|(n, file)| {
            let title = titles
                .get(&n)
                .map(|t| t.trim())
                .filter(|t| !t.is_empty())
                .map(str::to_string);
            let duration = lengths.get(&n).and_then(|len| parse_duration(len));
            PlaylistEntry {
                path: resolve_path(&file, base_dir),
                title,
                duration_secs: duration,
            }
        })
        .collect()
}

/// Parse the numeric suffix of a `FileN`/`TitleN`/`LengthN` key. A non-numeric remainder (e.g.
/// `filename=`) yields `None`, which lets the caller ignore the line.
fn parse_index(suffix: &str) -> Option<u32> {
    suffix.parse().ok()
}

/// Interpret a duration token in seconds. Both formats use `-1` for "unknown"; any negative or
/// unparseable value is likewise `None`. Fractional seconds (seen in some extended M3U writers) are
/// rounded to the nearest whole second.
fn parse_duration(token: &str) -> Option<u32> {
    match token.trim().parse::<f64>() {
        Ok(secs) if secs >= 0.0 => Some(secs.round() as u32),
        _ => None,
    }
}

/// Turn a raw entry string into a path. Relative entries resolve against `base_dir` when one is
/// given; absolute paths and URLs (anything containing `://`) are kept verbatim.
fn resolve_path(entry: &str, base_dir: Option<&Path>) -> PathBuf {
    if let Some(base) = base_dir {
        let path = Path::new(entry);
        if !entry.contains("://") && path.is_relative() {
            return base.join(path);
        }
    }
    PathBuf::from(entry)
}

/// Render a stored path for output. When it lives beneath `base_dir` it is written relative to that
/// directory (the inverse of [`resolve_path`], so a written playlist round-trips); otherwise it is
/// written in full. Lossy UTF-8 is acceptable here because playlist text is UTF-8.
fn display_path(path: &Path, base_dir: Option<&Path>) -> String {
    if let Some(base) = base_dir {
        if let Ok(relative) = path.strip_prefix(base) {
            return relative.to_string_lossy().into_owned();
        }
    }
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, title: Option<&str>, duration_secs: Option<u32>) -> PlaylistEntry {
        PlaylistEntry {
            path: PathBuf::from(path),
            title: title.map(String::from),
            duration_secs,
        }
    }

    #[test]
    fn plain_m3u_has_paths_but_no_metadata() {
        let entries = parse("a.mp3\nsub/b.mp3\n", PlaylistFormat::M3u, None);
        assert_eq!(
            entries,
            vec![entry("a.mp3", None, None), entry("sub/b.mp3", None, None)]
        );
    }

    #[test]
    fn extended_m3u_parses_title_and_duration_with_minus_one_as_unknown() {
        let text = "#EXTM3U\n#EXTINF:217,Cool Song\n/music/cool.mp3\n#EXTINF:-1,Live Stream\n/music/unk.mp3\n";
        let entries = parse(text, PlaylistFormat::M3u, None);
        assert_eq!(
            entries,
            vec![
                entry("/music/cool.mp3", Some("Cool Song"), Some(217)),
                entry("/music/unk.mp3", Some("Live Stream"), None),
            ]
        );
    }

    #[test]
    fn m3u_handles_bom_crlf_and_skips_comments_and_blanks() {
        let text = "\u{FEFF}#EXTM3U\r\n# a plain comment\r\n\r\n#EXTINF:12,Track\r\nx.mp3\r\n";
        let entries = parse(text, PlaylistFormat::M3u, None);
        assert_eq!(entries, vec![entry("x.mp3", Some("Track"), Some(12))]);
    }

    #[test]
    fn m3u_extinf_title_may_contain_commas() {
        // Title is everything after the FIRST comma, so internal commas are preserved.
        let text = "#EXTINF:100,Artist Name, Song Title\nsong.mp3\n";
        let entries = parse(text, PlaylistFormat::M3u, None);
        assert_eq!(
            entries,
            vec![entry("song.mp3", Some("Artist Name, Song Title"), Some(100))]
        );
    }

    #[test]
    fn m3u_extinf_without_a_comma_yields_no_title() {
        let entries = parse("#EXTINF:42\nsong.mp3\n", PlaylistFormat::M3u, None);
        assert_eq!(entries, vec![entry("song.mp3", None, Some(42))]);
    }

    #[test]
    fn m3u_resolves_relative_and_keeps_absolute_and_urls_verbatim() {
        let base = Path::new("/music");
        let text = "rel/song.mp3\n/abs/song.mp3\nhttp://host.example/stream.mp3\n";
        let entries = parse(text, PlaylistFormat::M3u, Some(base));
        assert_eq!(
            entries,
            vec![
                entry("/music/rel/song.mp3", None, None),
                entry("/abs/song.mp3", None, None),
                entry("http://host.example/stream.mp3", None, None),
            ]
        );
    }

    #[test]
    fn m3u_without_base_dir_keeps_relative_entries_as_written() {
        let entries = parse("rel/song.mp3\n", PlaylistFormat::M3u, None);
        assert_eq!(entries, vec![entry("rel/song.mp3", None, None)]);
    }

    #[test]
    fn pls_parses_file_title_and_length_fields() {
        let text = "[playlist]\nFile1=a.mp3\nTitle1=First\nLength1=100\nFile2=b.mp3\nTitle2=Second\nLength2=-1\nNumberOfEntries=2\nVersion=2\n";
        let entries = parse(text, PlaylistFormat::Pls, None);
        assert_eq!(
            entries,
            vec![
                entry("a.mp3", Some("First"), Some(100)),
                entry("b.mp3", Some("Second"), None),
            ]
        );
    }

    #[test]
    fn pls_sorts_indices_tolerates_gaps_and_missing_count() {
        // Out-of-order keys, a gap at index 2, and no NumberOfEntries at all.
        let text = "[playlist]\nFile3=third.mp3\nLength3=30\nFile1=first.mp3\nLength1=10\n";
        let entries = parse(text, PlaylistFormat::Pls, None);
        assert_eq!(
            entries,
            vec![
                entry("first.mp3", None, Some(10)),
                entry("third.mp3", None, Some(30)),
            ]
        );
    }

    #[test]
    fn pls_keys_are_case_insensitive_and_relative_paths_resolve() {
        let base = Path::new("/music");
        let text = "[Playlist]\nFILE1=rel.mp3\ntitle1=Song\nLENGTH1=9\n";
        let entries = parse(text, PlaylistFormat::Pls, Some(base));
        assert_eq!(
            entries,
            vec![entry("/music/rel.mp3", Some("Song"), Some(9))]
        );
    }

    #[test]
    fn pls_keeps_absolute_and_url_file_entries_verbatim() {
        let base = Path::new("/music");
        let text = "[playlist]\nFile1=/abs/x.mp3\nFile2=https://host.example/s.mp3\n";
        let entries = parse(text, PlaylistFormat::Pls, Some(base));
        assert_eq!(
            entries,
            vec![
                entry("/abs/x.mp3", None, None),
                entry("https://host.example/s.mp3", None, None),
            ]
        );
    }

    #[test]
    fn format_from_path_matches_known_extensions() {
        assert_eq!(
            format_from_path(Path::new("list.m3u")),
            Some(PlaylistFormat::M3u)
        );
        assert_eq!(
            format_from_path(Path::new("LIST.M3U")),
            Some(PlaylistFormat::M3u)
        );
        assert_eq!(
            format_from_path(Path::new("list.m3u8")),
            Some(PlaylistFormat::M3u)
        );
        assert_eq!(
            format_from_path(Path::new("list.pls")),
            Some(PlaylistFormat::Pls)
        );
        assert_eq!(
            format_from_path(Path::new("LIST.PLS")),
            Some(PlaylistFormat::Pls)
        );
        assert_eq!(format_from_path(Path::new("song.mp3")), None);
        assert_eq!(format_from_path(Path::new("noext")), None);
    }

    #[test]
    fn write_m3u_round_trips_through_parse() {
        let entries = vec![
            entry("/m/a.mp3", Some("A Song"), Some(100)),
            entry("/m/b.mp3", None, None),
            entry("/m/c.mp3", Some("C, with comma"), Some(0)),
        ];
        let text = write_m3u(&entries, None);
        assert_eq!(parse(&text, PlaylistFormat::M3u, None), entries);
    }

    #[test]
    fn write_m3u_emits_header_and_minus_one_for_unknown_duration() {
        let text = write_m3u(&[entry("/m/x.mp3", None, None)], None);
        assert!(text.starts_with("#EXTM3U\n"));
        assert!(text.contains("#EXTINF:-1,\n"));
        assert!(text.contains("/m/x.mp3\n"));
    }

    #[test]
    fn write_m3u_relativizes_under_base_dir_and_round_trips() {
        let base = Path::new("/music");
        let entries = vec![entry("/music/a.mp3", Some("A"), Some(10))];
        let text = write_m3u(&entries, Some(base));
        assert!(text.contains("\na.mp3\n"), "entry written relative: {text:?}");
        assert!(!text.contains("/music/a.mp3"));
        assert_eq!(parse(&text, PlaylistFormat::M3u, Some(base)), entries);
    }

    #[test]
    fn write_pls_round_trips_through_parse() {
        let entries = vec![
            entry("/m/a.mp3", Some("A Song"), Some(100)),
            entry("/m/b.mp3", None, None),
        ];
        let text = write_pls(&entries, None);
        assert_eq!(parse(&text, PlaylistFormat::Pls, None), entries);
    }

    #[test]
    fn write_pls_emits_metadata_and_numbered_fields() {
        let entries = vec![
            entry("/m/a.mp3", Some("A"), Some(5)),
            entry("/m/b.mp3", None, None),
        ];
        let text = write_pls(&entries, None);
        assert!(text.starts_with("[playlist]\n"));
        assert!(text.contains("File1=/m/a.mp3\n"));
        assert!(text.contains("Title1=A\n"));
        assert!(text.contains("Length1=5\n"));
        assert!(text.contains("File2=/m/b.mp3\n"));
        assert!(text.contains("Title2=\n"), "empty title round-trips to None");
        assert!(text.contains("Length2=-1\n"), "unknown length is -1");
        assert!(text.contains("NumberOfEntries=2\n"));
        assert!(text.contains("Version=2\n"));
    }

    #[test]
    fn write_pls_relativizes_under_base_dir_and_round_trips() {
        let base = Path::new("/music");
        let entries = vec![entry("/music/nested/a.mp3", Some("A"), Some(3))];
        let text = write_pls(&entries, Some(base));
        assert!(text.contains("File1=nested/a.mp3\n"), "{text:?}");
        assert_eq!(parse(&text, PlaylistFormat::Pls, Some(base)), entries);
    }

    #[test]
    fn parse_tolerates_trailing_extinf_and_junk_lines() {
        // A dangling `#EXTINF` with no following path, and a keyless PLS line, are both ignored.
        assert!(parse("#EXTINF:10,Orphan\n", PlaylistFormat::M3u, None).is_empty());
        assert!(parse("[playlist]\nnot a pair\n", PlaylistFormat::Pls, None).is_empty());
    }
}
