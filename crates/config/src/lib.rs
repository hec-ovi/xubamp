//! Versioned settings and session persistence.
//!
//! The format is deliberately small and dependency-free: UTF-8 `key=value` records, repeated keys
//! for library roots, and percent-encoded raw Unix path bytes. Unknown keys are ignored so newer
//! settings remain readable by older builds. Invalid known values fall back independently instead
//! of discarding the whole file.

use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const FORMAT_VERSION: u32 = 1;
pub const BAND_FREQUENCIES: [u32; 10] = [
    60, 170, 310, 600, 1_000, 3_000, 6_000, 12_000, 14_000, 16_000,
];

/// There is intentionally no plugin discovery or loading surface. Preferences can display this as
/// unavailable, but a config file cannot turn it on.
pub const PLUGINS_SUPPORTED: bool = false;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TimeDisplay {
    #[default]
    Elapsed,
    Remaining,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VisualizationMode {
    #[default]
    Spectrum,
    Oscilloscope,
    Off,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaybackSettings {
    pub shuffle: bool,
    pub repeat: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplaySettings {
    pub time: TimeDisplay,
    pub double_size: bool,
    pub scroll_title: bool,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            time: TimeDisplay::Elapsed,
            double_size: false,
            scroll_title: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualizationSettings {
    pub mode: VisualizationMode,
    pub show_peaks: bool,
}

impl Default for VisualizationSettings {
    fn default() -> Self {
        Self {
            mode: VisualizationMode::Spectrum,
            show_peaks: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibrarySettings {
    /// Audio-only scan roots. There are no video or CD-ripping settings in the model.
    pub roots: Vec<PathBuf>,
    pub recurse: bool,
}

impl Default for LibrarySettings {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            recurse: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EqualizerSettings {
    pub enabled: bool,
    /// Preamp and band values in dB, each clamped to the classic -12..=12 range.
    pub preamp_db: f32,
    pub bands_db: [f32; 10],
}

impl Default for EqualizerSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            preamp_db: 0.0,
            bands_db: [0.0; 10],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneSettings {
    pub open: bool,
    pub shaded: bool,
    /// Relative position inside the main/EQ/playlist pane cluster.
    pub x: i32,
    pub y: i32,
    /// Expanded size. Main and EQ remain fixed; playlist may persist a larger size.
    pub width: u32,
    pub height: u32,
}

impl PaneSettings {
    const fn new(open: bool, x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            open,
            shaded: false,
            x,
            y,
            width,
            height,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowSettings {
    pub main: PaneSettings,
    pub equalizer: PaneSettings,
    pub playlist: PaneSettings,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            main: PaneSettings::new(true, 0, 0, 275, 116),
            equalizer: PaneSettings::new(false, 0, 116, 275, 116),
            playlist: PaneSettings::new(false, 275, 0, 275, 116),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub playback: PlaybackSettings,
    pub display: DisplaySettings,
    pub visualization: VisualizationSettings,
    pub library: LibrarySettings,
    pub equalizer: EqualizerSettings,
    pub windows: WindowSettings,
    /// `None` selects the authored base skin.
    pub skin_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub line: usize,
    pub key: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseReport {
    pub settings: Settings,
    pub warnings: Vec<Warning>,
}

impl Settings {
    /// Parse a settings file. Unknown keys are forward-compatible and ignored. A malformed known
    /// value produces a warning and leaves just that field at its default.
    pub fn parse(text: &str) -> ParseReport {
        let mut settings = Self::default();
        let mut warnings = Vec::new();
        let mut saw_library_root = false;

        for (idx, raw) in text.lines().enumerate() {
            let line = idx + 1;
            let raw = raw.trim();
            if raw.is_empty() || raw.starts_with('#') {
                continue;
            }
            let Some((key, value)) = raw.split_once('=') else {
                warnings.push(Warning {
                    line,
                    key: String::new(),
                    message: "expected key=value".into(),
                });
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            let bad = |message: &str, warnings: &mut Vec<Warning>| {
                warnings.push(Warning {
                    line,
                    key: key.into(),
                    message: message.into(),
                });
            };

            match key {
                "version" => match value.parse::<u32>() {
                    Ok(v) if v <= FORMAT_VERSION => {}
                    Ok(v) => bad(
                        &format!("unsupported future format version {v}"),
                        &mut warnings,
                    ),
                    Err(_) => bad("expected an unsigned integer", &mut warnings),
                },
                "playback.shuffle" => set_bool(
                    value,
                    &mut settings.playback.shuffle,
                    line,
                    key,
                    &mut warnings,
                ),
                "playback.repeat" => set_bool(
                    value,
                    &mut settings.playback.repeat,
                    line,
                    key,
                    &mut warnings,
                ),
                "display.time" => match value {
                    "elapsed" => settings.display.time = TimeDisplay::Elapsed,
                    "remaining" => settings.display.time = TimeDisplay::Remaining,
                    _ => bad("expected elapsed or remaining", &mut warnings),
                },
                "display.double_size" => set_bool(
                    value,
                    &mut settings.display.double_size,
                    line,
                    key,
                    &mut warnings,
                ),
                "display.scroll_title" => set_bool(
                    value,
                    &mut settings.display.scroll_title,
                    line,
                    key,
                    &mut warnings,
                ),
                "visualization.mode" => match value {
                    "spectrum" => settings.visualization.mode = VisualizationMode::Spectrum,
                    "oscilloscope" => settings.visualization.mode = VisualizationMode::Oscilloscope,
                    "off" => settings.visualization.mode = VisualizationMode::Off,
                    _ => bad("expected spectrum, oscilloscope, or off", &mut warnings),
                },
                "visualization.show_peaks" => set_bool(
                    value,
                    &mut settings.visualization.show_peaks,
                    line,
                    key,
                    &mut warnings,
                ),
                "library.recurse" => set_bool(
                    value,
                    &mut settings.library.recurse,
                    line,
                    key,
                    &mut warnings,
                ),
                "library.root" => match decode_path(value) {
                    Ok(path) => {
                        if !saw_library_root {
                            settings.library.roots.clear();
                            saw_library_root = true;
                        }
                        if !settings.library.roots.contains(&path) {
                            settings.library.roots.push(path);
                        }
                    }
                    Err(e) => bad(e, &mut warnings),
                },
                "skin.path" => match value {
                    "base" => settings.skin_path = None,
                    _ => match decode_path(value) {
                        Ok(path) => settings.skin_path = Some(path),
                        Err(e) => bad(e, &mut warnings),
                    },
                },
                "equalizer.enabled" => set_bool(
                    value,
                    &mut settings.equalizer.enabled,
                    line,
                    key,
                    &mut warnings,
                ),
                "equalizer.preamp_db" => set_db(
                    value,
                    &mut settings.equalizer.preamp_db,
                    line,
                    key,
                    &mut warnings,
                ),
                _ if key.starts_with("equalizer.band.") => {
                    let frequency = key
                        .trim_start_matches("equalizer.band.")
                        .parse::<u32>()
                        .ok();
                    if let Some(i) = frequency.and_then(|f| {
                        BAND_FREQUENCIES
                            .iter()
                            .position(|&candidate| candidate == f)
                    }) {
                        set_db(
                            value,
                            &mut settings.equalizer.bands_db[i],
                            line,
                            key,
                            &mut warnings,
                        );
                    } else {
                        bad("unknown equalizer band", &mut warnings);
                    }
                }
                _ if key.starts_with("window.") => {
                    if !set_pane_field(key, value, &mut settings.windows, line, &mut warnings) {
                        // Unknown window fields are ignored for forward compatibility.
                    }
                }
                // Plugins are permanently unsupported. Treat an attempted enable as a warning so
                // hand-edited config cannot silently claim otherwise.
                "plugins.enabled" if value == "true" => {
                    bad("plugin loading is not supported", &mut warnings)
                }
                "plugins.enabled" => {}
                _ => {}
            }
        }

        ParseReport { settings, warnings }
    }

    /// Canonical stable representation used for atomic persistence.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        line(&mut out, "version", FORMAT_VERSION);
        line(&mut out, "playback.shuffle", self.playback.shuffle);
        line(&mut out, "playback.repeat", self.playback.repeat);
        line(
            &mut out,
            "display.time",
            match self.display.time {
                TimeDisplay::Elapsed => "elapsed",
                TimeDisplay::Remaining => "remaining",
            },
        );
        line(&mut out, "display.double_size", self.display.double_size);
        line(&mut out, "display.scroll_title", self.display.scroll_title);
        line(
            &mut out,
            "visualization.mode",
            match self.visualization.mode {
                VisualizationMode::Spectrum => "spectrum",
                VisualizationMode::Oscilloscope => "oscilloscope",
                VisualizationMode::Off => "off",
            },
        );
        line(
            &mut out,
            "visualization.show_peaks",
            self.visualization.show_peaks,
        );
        line(&mut out, "library.recurse", self.library.recurse);
        for root in &self.library.roots {
            line(&mut out, "library.root", encode_path(root));
        }
        line(
            &mut out,
            "skin.path",
            self.skin_path
                .as_ref()
                .map_or_else(|| "base".into(), |p| encode_path(p)),
        );
        line(&mut out, "equalizer.enabled", self.equalizer.enabled);
        line(
            &mut out,
            "equalizer.preamp_db",
            format_db(self.equalizer.preamp_db),
        );
        for (frequency, value) in BAND_FREQUENCIES.iter().zip(self.equalizer.bands_db) {
            line(
                &mut out,
                &format!("equalizer.band.{frequency}"),
                format_db(value),
            );
        }
        write_pane(&mut out, "main", &self.windows.main);
        write_pane(&mut out, "equalizer", &self.windows.equalizer);
        write_pane(&mut out, "playlist", &self.windows.playlist);
        line(&mut out, "plugins.enabled", false);
        out
    }
}

fn set_bool(value: &str, dst: &mut bool, line: usize, key: &str, warnings: &mut Vec<Warning>) {
    match value {
        "true" => *dst = true,
        "false" => *dst = false,
        _ => warnings.push(Warning {
            line,
            key: key.into(),
            message: "expected true or false".into(),
        }),
    }
}

fn set_db(value: &str, dst: &mut f32, line: usize, key: &str, warnings: &mut Vec<Warning>) {
    match value.parse::<f32>() {
        Ok(v) if v.is_finite() && (-12.0..=12.0).contains(&v) => *dst = v,
        _ => warnings.push(Warning {
            line,
            key: key.into(),
            message: "expected a finite value from -12 to 12 dB".into(),
        }),
    }
}

fn set_pane_field(
    key: &str,
    value: &str,
    windows: &mut WindowSettings,
    line: usize,
    warnings: &mut Vec<Warning>,
) -> bool {
    let mut parts = key.split('.');
    if parts.next() != Some("window") {
        return false;
    }
    let pane = match parts.next() {
        Some("main") => &mut windows.main,
        Some("equalizer") => &mut windows.equalizer,
        Some("playlist") => &mut windows.playlist,
        _ => return false,
    };
    let Some(field) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    let warn = |message: &str, warnings: &mut Vec<Warning>| {
        warnings.push(Warning {
            line,
            key: key.into(),
            message: message.into(),
        });
    };
    match field {
        "open" => set_bool(value, &mut pane.open, line, key, warnings),
        "shaded" => set_bool(value, &mut pane.shaded, line, key, warnings),
        "x" => match value.parse() {
            Ok(v) => pane.x = v,
            Err(_) => warn("expected a signed integer", warnings),
        },
        "y" => match value.parse() {
            Ok(v) => pane.y = v,
            Err(_) => warn("expected a signed integer", warnings),
        },
        "width" => match value.parse::<u32>() {
            Ok(v) if v >= 14 => pane.width = v,
            _ => warn("expected an integer of at least 14", warnings),
        },
        "height" => match value.parse::<u32>() {
            Ok(v) if v >= 14 => pane.height = v,
            _ => warn("expected an integer of at least 14", warnings),
        },
        _ => return false,
    }
    true
}

fn write_pane(out: &mut String, name: &str, pane: &PaneSettings) {
    line(out, &format!("window.{name}.open"), pane.open);
    line(out, &format!("window.{name}.shaded"), pane.shaded);
    line(out, &format!("window.{name}.x"), pane.x);
    line(out, &format!("window.{name}.y"), pane.y);
    line(out, &format!("window.{name}.width"), pane.width);
    line(out, &format!("window.{name}.height"), pane.height);
}

fn line(out: &mut String, key: &str, value: impl fmt::Display) {
    use fmt::Write as _;
    let _ = writeln!(out, "{key}={value}");
}

fn format_db(value: f32) -> String {
    let value = if value == -0.0 { 0.0 } else { value };
    format!("{value:.3}")
}

fn encode_path(path: &Path) -> String {
    let mut out = String::new();
    use fmt::Write as _;
    for &byte in path.as_os_str().as_bytes() {
        if matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'_' | b'-' | b'.') {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

fn decode_path(value: &str) -> Result<PathBuf, &'static str> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            decoded.push(bytes[i]);
            i += 1;
            continue;
        }
        if i + 2 >= bytes.len() {
            return Err("truncated percent escape in path");
        }
        let hi = hex(bytes[i + 1]).ok_or("invalid percent escape in path")?;
        let lo = hex(bytes[i + 2]).ok_or("invalid percent escape in path")?;
        decoded.push((hi << 4) | lo);
        i += 3;
    }
    Ok(PathBuf::from(OsString::from_vec(decoded)))
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Resolve `$XDG_CONFIG_HOME/xubamp/settings.conf`, falling back to
/// `$HOME/.config/xubamp/settings.conf` as required by the XDG base-directory convention.
pub fn default_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("XDG_CONFIG_HOME").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path).join("xubamp/settings.conf"));
    }
    env::var_os("HOME")
        .filter(|p| !p.is_empty())
        .map(|home| PathBuf::from(home).join(".config/xubamp/settings.conf"))
}

/// Load settings from an injected path. A missing file is a normal first run and returns defaults.
pub fn load(path: &Path) -> io::Result<ParseReport> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Settings::parse(&text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(ParseReport {
            settings: Settings::default(),
            warnings: Vec::new(),
        }),
        Err(e) => Err(e),
    }
}

/// Atomically replace `path` with `settings`. The temporary file lives in the same directory so
/// rename cannot cross filesystems; both file and directory metadata are synced before returning.
pub fn save(path: &Path, settings: &Settings) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temp = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), stamp));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(settings.to_text().as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;

    fn temp_file(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir()
            .join(format!("xubamp-config-{}-{nonce}", std::process::id()))
            .join(name)
    }

    #[test]
    fn defaults_match_the_supported_product_surface() {
        let s = Settings::default();
        assert!(!s.playback.shuffle && !s.playback.repeat);
        assert_eq!(s.display.time, TimeDisplay::Elapsed);
        assert_eq!(s.visualization.mode, VisualizationMode::Spectrum);
        assert!(s.library.roots.is_empty() && s.library.recurse);
        assert!(s.equalizer.enabled);
        assert_eq!(s.equalizer.bands_db, [0.0; 10]);
        assert!(
            s.skin_path.is_none(),
            "the authored base skin is the default"
        );
        assert!(s.windows.main.open);
        assert!(!s.windows.equalizer.open && !s.windows.playlist.open);
    }

    #[test]
    fn canonical_text_round_trips_every_supported_setting_and_raw_path_bytes() {
        let odd_path = PathBuf::from(OsString::from_vec(b" /music/nonutf8-\xFF=100%\n ".to_vec()));
        let mut s = Settings::default();
        s.playback.shuffle = true;
        s.playback.repeat = true;
        s.display.time = TimeDisplay::Remaining;
        s.display.double_size = true;
        s.display.scroll_title = false;
        s.visualization.mode = VisualizationMode::Oscilloscope;
        s.visualization.show_peaks = false;
        s.library.roots = vec![PathBuf::from("/music/A B"), odd_path.clone()];
        s.library.recurse = false;
        s.skin_path = Some(odd_path);
        s.equalizer.enabled = false;
        s.equalizer.preamp_db = 3.5;
        s.equalizer.bands_db = [-12.0, -9.5, -3.0, 0.0, 1.25, 2.0, 4.0, 6.0, 9.0, 12.0];
        s.windows.equalizer.open = true;
        s.windows.equalizer.shaded = true;
        s.windows.playlist.x = -275;
        s.windows.playlist.width = 400;

        let text = s.to_text();
        assert!(text.contains("plugins.enabled=false"));
        assert!(!text.contains("video") && !text.contains("ripping") && !text.contains("setup"));
        let report = Settings::parse(&text);
        assert!(
            report.warnings.is_empty(),
            "canonical output parses cleanly: {:?}",
            report.warnings
        );
        assert_eq!(report.settings, s);
    }

    #[test]
    fn corrupt_known_values_fall_back_independently_and_report_lines() {
        let report = Settings::parse(
            "version=1\nplayback.shuffle=yes\nplayback.repeat=true\nequalizer.preamp_db=nan\n\
             equalizer.band.60=99\nvisualization.mode=milkdrop\nplugins.enabled=true\nfuture.option=kept\n",
        );
        assert!(
            !report.settings.playback.shuffle,
            "bad shuffle kept its default"
        );
        assert!(
            report.settings.playback.repeat,
            "valid neighbor still applied"
        );
        assert_eq!(report.settings.equalizer.preamp_db, 0.0);
        assert_eq!(report.settings.equalizer.bands_db[0], 0.0);
        assert_eq!(
            report.settings.visualization.mode,
            VisualizationMode::Spectrum
        );
        assert_eq!(report.warnings.len(), 5);
        assert!(report.warnings.iter().any(|w| w.key == "plugins.enabled"));
    }

    #[test]
    fn save_and_load_exercise_the_real_atomic_file_entry_points() {
        let path = temp_file("nested/settings.conf");
        let mut expected = Settings::default();
        expected.playback.shuffle = true;
        expected.library.roots.push(PathBuf::from("/srv/music"));
        expected.equalizer.bands_db[4] = 4.25;

        save(&path, &expected).expect("atomic save");
        let report = load(&path).expect("load saved settings");
        assert!(report.warnings.is_empty());
        assert_eq!(report.settings, expected);
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "atomic save left no temporary files");
        fs::remove_dir_all(path.ancestors().nth(2).unwrap()).unwrap();
    }

    #[test]
    fn missing_file_is_a_clean_first_run() {
        let path = temp_file("missing/settings.conf");
        let report = load(&path).expect("missing settings are not an error");
        assert_eq!(report.settings, Settings::default());
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn repeated_library_roots_keep_order_and_deduplicate() {
        let report = Settings::parse(
            "library.root=/b\nlibrary.root=/a\nlibrary.root=/b\nlibrary.recurse=false\n",
        );
        assert_eq!(
            report.settings.library.roots,
            [PathBuf::from("/b"), PathBuf::from("/a")]
        );
        assert!(!report.settings.library.recurse);
    }
}
