//! xubamp binary entry point.
//!
//! Composes a skin's main window and shows it in a native Wayland window, and (when built with
//! the `audio` feature) plays a media-file argument through PipeWire alongside it. Arguments
//! are classified by extension: a `.wsz`/`.zip` path is a skin, an audio file is a track.
//! The skin is otherwise resolved in order: `$XUBAMP_SKIN`, the saved skin, a local `skins/` test
//! skin if one is checked out, then the built-in default (original, clean-room;
//! `xubamp_skin::default_skin`).

use std::path::{Path, PathBuf};

use xubamp_skin::{default_skin, Skin};

/// The playlist-to-engine player. Only with the audio feature (it owns the PipeWire-backed engine).
#[cfg(feature = "audio")]
mod player;
mod portal_actions;

/// A local skin used only during development. It lives under `skins/`, which is gitignored
/// (third-party art, never committed or shipped), so a released binary never finds it and
/// falls through to the built-in default. This is the "use the XMMS skin for now" hook.
const DEV_SKIN: &str = "skins/XMMS_standard_skin.wsz";

fn has_ext(name: &str, ext: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// The marquee title for a media path: its file name without the extension. This is Winamp's
/// fallback when there are no tags to read (tag-based titles come with the playlist). A path
/// with no file name yields an empty title, which draws no marquee. A leading-dot name (e.g.
/// `.mp3`) is extension-less to Rust, so the whole name is the stem.
fn track_title(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Split CLI arguments into an optional skin path and the media playlist, by extension. The first
/// skin wins; every media file is kept, in order, so `xubamp a.mp3 b.mp3` (or a shell glob) builds a
/// playlist. Anything unrecognized is ignored. Pure and iterator-based, so it is unit-testable
/// without a real argv.
fn classify<I: IntoIterator<Item = String>>(args: I) -> (Option<String>, Vec<String>) {
    let mut skin = None;
    let mut media = Vec::new();
    for arg in args {
        if has_ext(&arg, "wsz") || has_ext(&arg, "zip") {
            skin.get_or_insert(arg);
        } else if xubamp_library::is_audio_path(Path::new(&arg)) {
            media.push(arg);
        }
    }
    (skin, media)
}

fn load_skin(path: &Path) -> Result<Skin, String> {
    portal_actions::load_skin_archive(path)
}

/// Select an archive in priority order. A missing saved archive is deliberately skipped: stale
/// persisted state must not prevent the application from starting. Explicit CLI/environment paths
/// remain authoritative and report their load error instead of silently changing skins.
fn preferred_skin_path(
    cli: Option<&str>,
    environment: Option<std::ffi::OsString>,
    saved: Option<&Path>,
    saved_exists: bool,
    dev_exists: bool,
) -> Option<PathBuf> {
    cli.map(PathBuf::from)
        .or_else(|| environment.map(PathBuf::from))
        .or_else(|| saved.filter(|_| saved_exists).map(Path::to_path_buf))
        .or_else(|| dev_exists.then(|| PathBuf::from(DEV_SKIN)))
}

/// Resolve which skin to show, in priority order: CLI path, `$XUBAMP_SKIN`, saved skin, a local dev
/// skin if checked out, else the built-in default.
fn resolve_skin(cli: Option<&str>, saved: Option<&Path>) -> Skin {
    let saved_exists = saved.is_some_and(Path::exists);
    if let Some(path) = saved.filter(|_| !saved_exists) {
        eprintln!(
            "xubamp: saved skin {} no longer exists; falling back",
            path.display()
        );
    }
    let dev_exists = Path::new(DEV_SKIN).exists();
    let environment = std::env::var_os("XUBAMP_SKIN");
    let selected = preferred_skin_path(cli, environment.clone(), saved, saved_exists, dev_exists);
    if let Some(path) = selected {
        if path == Path::new(DEV_SKIN) {
            eprintln!("xubamp: using local dev skin {DEV_SKIN}");
        }
        match load_skin(&path) {
            Ok(skin) => return skin,
            Err(error) if cli.is_some() || environment.is_some() => {
                eprintln!("xubamp: {error}");
                std::process::exit(1);
            }
            Err(error) => {
                eprintln!("xubamp: {error}; falling back to the built-in default skin");
            }
        }
    }
    eprintln!("xubamp: no skin given, using the built-in default skin");
    default_skin()
}

/// Load the settings file selected by the XDG base-directory rules. A malformed field only emits a
/// warning and falls back independently; an unreadable file falls back to all defaults.
fn load_settings(path: Option<&Path>) -> xubamp_config::Settings {
    let Some(path) = path else {
        eprintln!("xubamp: HOME and XDG_CONFIG_HOME are unset; settings will not be persisted");
        return xubamp_config::Settings::default();
    };
    match xubamp_config::load(path) {
        Ok(report) => {
            for warning in report.warnings {
                let key = if warning.key.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", warning.key)
                };
                eprintln!(
                    "xubamp: settings warning at line {}{key}: {}",
                    warning.line, warning.message
                );
            }
            report.settings
        }
        Err(error) => {
            eprintln!(
                "xubamp: cannot load settings from {}: {error}; using defaults",
                path.display()
            );
            xubamp_config::Settings::default()
        }
    }
}

/// Synchronize the two mutable playback modes and atomically save them after a user toggle. Other
/// settings stay untouched. `None` is valid on systems without a resolvable config directory.
#[cfg(any(feature = "audio", test))]
fn persist_playback_modes(
    path: Option<&Path>,
    settings: &mut xubamp_config::Settings,
    shuffle: bool,
    repeat: bool,
) -> std::io::Result<()> {
    settings.playback.shuffle = shuffle;
    settings.playback.repeat = repeat;
    match path {
        Some(path) => xubamp_config::save(path, settings),
        None => Ok(()),
    }
}

/// Save a validated live skin selection, or clear it when the authored base skin is selected.
/// Updating memory even without an XDG config path keeps the current session coherent.
fn persist_skin_path(
    path: Option<&Path>,
    settings: &mut xubamp_config::Settings,
    skin_path: Option<PathBuf>,
) -> std::io::Result<()> {
    settings.skin_path = skin_path;
    match path {
        Some(path) => xubamp_config::save(path, settings),
        None => Ok(()),
    }
}

fn ui_time_display(display: xubamp_config::TimeDisplay) -> xubamp_render::hit::TimeDisplay {
    match display {
        xubamp_config::TimeDisplay::Elapsed => xubamp_render::hit::TimeDisplay::Elapsed,
        xubamp_config::TimeDisplay::Remaining => xubamp_render::hit::TimeDisplay::Remaining,
    }
}

fn config_time_display(display: xubamp_render::hit::TimeDisplay) -> xubamp_config::TimeDisplay {
    match display {
        xubamp_render::hit::TimeDisplay::Elapsed => xubamp_config::TimeDisplay::Elapsed,
        xubamp_render::hit::TimeDisplay::Remaining => xubamp_config::TimeDisplay::Remaining,
    }
}

fn vis_analyzer_style(style: xubamp_config::AnalyzerStyle) -> xubamp_render::vis::AnalyzerStyle {
    match style {
        xubamp_config::AnalyzerStyle::Normal => xubamp_render::vis::AnalyzerStyle::Normal,
        xubamp_config::AnalyzerStyle::Fire => xubamp_render::vis::AnalyzerStyle::Fire,
        xubamp_config::AnalyzerStyle::Line => xubamp_render::vis::AnalyzerStyle::Line,
    }
}

fn config_analyzer_style(style: xubamp_render::vis::AnalyzerStyle) -> xubamp_config::AnalyzerStyle {
    match style {
        xubamp_render::vis::AnalyzerStyle::Normal => xubamp_config::AnalyzerStyle::Normal,
        xubamp_render::vis::AnalyzerStyle::Fire => xubamp_config::AnalyzerStyle::Fire,
        xubamp_render::vis::AnalyzerStyle::Line => xubamp_config::AnalyzerStyle::Line,
    }
}

fn vis_band_width(width: xubamp_config::BandWidth) -> xubamp_render::vis::BandWidth {
    match width {
        xubamp_config::BandWidth::Thick => xubamp_render::vis::BandWidth::Thick,
        xubamp_config::BandWidth::Thin => xubamp_render::vis::BandWidth::Thin,
    }
}

fn config_band_width(width: xubamp_render::vis::BandWidth) -> xubamp_config::BandWidth {
    match width {
        xubamp_render::vis::BandWidth::Thick => xubamp_config::BandWidth::Thick,
        xubamp_render::vis::BandWidth::Thin => xubamp_config::BandWidth::Thin,
    }
}

fn vis_osc_style(style: xubamp_config::OscilloscopeStyle) -> xubamp_render::vis::OscStyle {
    match style {
        xubamp_config::OscilloscopeStyle::Dots => xubamp_render::vis::OscStyle::Dots,
        xubamp_config::OscilloscopeStyle::Lines => xubamp_render::vis::OscStyle::Lines,
        xubamp_config::OscilloscopeStyle::Solid => xubamp_render::vis::OscStyle::Solid,
    }
}

fn config_osc_style(style: xubamp_render::vis::OscStyle) -> xubamp_config::OscilloscopeStyle {
    match style {
        xubamp_render::vis::OscStyle::Dots => xubamp_config::OscilloscopeStyle::Dots,
        xubamp_render::vis::OscStyle::Lines => xubamp_config::OscilloscopeStyle::Lines,
        xubamp_render::vis::OscStyle::Solid => xubamp_config::OscilloscopeStyle::Solid,
    }
}

/// Seed the Preferences window's model from the persisted settings so it opens showing the user's
/// real values rather than defaults.
fn preferences_model_from(
    settings: &xubamp_config::Settings,
) -> xubamp_render::preferences::PreferencesModel {
    use xubamp_render::preferences as pref;
    pref::PreferencesModel {
        shuffle_morph_rate: settings.playback.shuffle_morph_rate,
        visualization_mode: match settings.visualization.mode {
            xubamp_config::VisualizationMode::Spectrum => pref::VisualizationMode::Spectrum,
            xubamp_config::VisualizationMode::Oscilloscope => pref::VisualizationMode::Oscilloscope,
            xubamp_config::VisualizationMode::Off => pref::VisualizationMode::Off,
        },
        visualization_show_peaks: settings.visualization.show_peaks,
        display_time: match settings.display.time {
            xubamp_config::TimeDisplay::Elapsed => pref::TimeDisplay::Elapsed,
            xubamp_config::TimeDisplay::Remaining => pref::TimeDisplay::Remaining,
        },
        display_double_size: settings.display.double_size,
        display_scroll_title: settings.display.scroll_title,
        display_clutterbar: settings.display.show_clutterbar,
        display_playlist_numbers: settings.display.playlist_numbers,
        display_snap_px: settings.display.snap_px,
        visualization_analyzer_style: vis_analyzer_style(settings.visualization.analyzer_style),
        visualization_band_width: vis_band_width(settings.visualization.band_width),
        visualization_osc_style: vis_osc_style(settings.visualization.oscilloscope_style),
        // Older files stored ten falloff speeds; the classic scale has five.
        visualization_bar_falloff: settings.visualization.bar_falloff.min(5),
        visualization_peak_falloff: settings.visualization.peak_falloff.min(5),
        visualization_refresh_rate: settings.visualization.refresh_rate,
        read_titles_on_load: settings.playback.read_titles_on_load,
        sort_on_load: settings.playback.sort_on_load,
        manual_advance: settings.playback.manual_advance,
        convert_underscores: settings.display.convert_underscores,
        convert_percent20: settings.display.convert_percent20,
        library_roots: settings.library.roots.clone(),
        library_recurse: settings.library.recurse,
        skin_path: settings.skin_path.clone(),
    }
}

/// Fold one committed Preferences command into the settings. Returns whether it changed a persisted
/// value (so the caller knows to write the file). `ChooseLibraryDirectory` opens a picker elsewhere
/// and is not itself a settings mutation.
fn apply_preference_to_settings(
    command: &xubamp_render::preferences::Command,
    settings: &mut xubamp_config::Settings,
) -> bool {
    use xubamp_render::preferences::Command;
    match command {
        Command::SetShuffleMorphRate(rate) => settings.playback.shuffle_morph_rate = *rate,
        Command::SetVisualizationMode(mode) => {
            settings.visualization.mode = match mode {
                xubamp_render::preferences::VisualizationMode::Spectrum => {
                    xubamp_config::VisualizationMode::Spectrum
                }
                xubamp_render::preferences::VisualizationMode::Oscilloscope => {
                    xubamp_config::VisualizationMode::Oscilloscope
                }
                xubamp_render::preferences::VisualizationMode::Off => {
                    xubamp_config::VisualizationMode::Off
                }
            }
        }
        Command::SetVisualizationShowPeaks(show) => settings.visualization.show_peaks = *show,
        Command::SetDisplayTime(time) => {
            settings.display.time = match time {
                xubamp_render::preferences::TimeDisplay::Elapsed => {
                    xubamp_config::TimeDisplay::Elapsed
                }
                xubamp_render::preferences::TimeDisplay::Remaining => {
                    xubamp_config::TimeDisplay::Remaining
                }
            }
        }
        Command::SetDisplayDoubleSize(on) => settings.display.double_size = *on,
        Command::SetDisplayScrollTitle(on) => settings.display.scroll_title = *on,
        Command::SetDisplayClutterbar(on) => settings.display.show_clutterbar = *on,
        Command::SetDisplayPlaylistNumbers(on) => settings.display.playlist_numbers = *on,
        Command::SetSnapPx(px) => settings.display.snap_px = *px,
        Command::SetAnalyzerStyle(style) => {
            settings.visualization.analyzer_style = config_analyzer_style(*style)
        }
        Command::SetBandWidth(width) => {
            settings.visualization.band_width = config_band_width(*width)
        }
        Command::SetOscilloscopeStyle(style) => {
            settings.visualization.oscilloscope_style = config_osc_style(*style)
        }
        Command::SetBarFalloff(speed) => settings.visualization.bar_falloff = *speed,
        Command::SetPeakFalloff(speed) => settings.visualization.peak_falloff = *speed,
        Command::SetRefreshRate(speed) => settings.visualization.refresh_rate = *speed,
        Command::SetReadTitlesOnLoad(on_load) => {
            settings.playback.read_titles_on_load = *on_load
        }
        Command::SetSortOnLoad(on) => settings.playback.sort_on_load = *on,
        Command::SetManualAdvance(on) => settings.playback.manual_advance = *on,
        Command::SetConvertUnderscores(on) => settings.display.convert_underscores = *on,
        Command::SetConvertPercent20(on) => settings.display.convert_percent20 = *on,
        Command::SetLibraryRoots(roots) => settings.library.roots = roots.clone(),
        Command::SetLibraryRecurse(recurse) => settings.library.recurse = *recurse,
        Command::SetSkinPath(path) => settings.skin_path = path.clone(),
        Command::ChooseLibraryDirectory | Command::ChooseSkinFile => return false,
    }
    true
}

/// Copy the final, user-visible window and equalizer state out of the Wayland event loop before the
/// settings file is saved. Runtime-only hover/drag state never crosses this boundary.
fn apply_ui_session(settings: &mut xubamp_config::Settings, session: xubamp_wl::SessionState) {
    let panes = session.panes;
    settings.windows.main.open = true;
    settings.windows.main.shaded = panes.main_shaded;
    settings.windows.equalizer.open = panes.equalizer_open;
    settings.windows.equalizer.shaded = session.equalizer_shaded;
    settings.windows.equalizer.x = panes.equalizer_position.0;
    settings.windows.equalizer.y = panes.equalizer_position.1;
    settings.windows.playlist.open = panes.playlist_open;
    settings.windows.playlist.shaded = panes.playlist_shaded;
    settings.windows.playlist.x = panes.playlist_position.0;
    settings.windows.playlist.y = panes.playlist_position.1;
    settings.windows.playlist.width = panes.playlist_size.0;
    settings.windows.playlist.height = panes.playlist_size.1;
    settings.equalizer.enabled = session.equalizer_enabled;
    settings.equalizer.preamp_db = session.equalizer_preamp_db;
    settings.equalizer.bands_db = session.equalizer_bands_db;
    settings.display.time = config_time_display(session.time_display);
    settings.display.scroll_title = session.scroll_title;
    settings.display.double_size = session.double_size;
    settings.visualization.mode = match session.visualization_mode {
        xubamp_render::vis::VisMode::Bars => xubamp_config::VisualizationMode::Spectrum,
        xubamp_render::vis::VisMode::Oscilloscope => {
            xubamp_config::VisualizationMode::Oscilloscope
        }
        xubamp_render::vis::VisMode::Off => xubamp_config::VisualizationMode::Off,
    };
    settings.visualization.show_peaks = session.visualization_show_peaks;
    settings.visualization.analyzer_style = config_analyzer_style(session.analyzer_style);
    settings.visualization.band_width = config_band_width(session.band_width);
    settings.visualization.oscilloscope_style = config_osc_style(session.oscilloscope_style);
    settings.visualization.bar_falloff = session.bar_falloff;
    settings.visualization.peak_falloff = session.peak_falloff;
    settings.visualization.refresh_rate = session.refresh_rate;
}

fn classic_equalizer_presets() -> Vec<xubamp_render::equalizer::Preset> {
    xubamp_dsp::presets::builtins()
        .into_iter()
        .map(|preset| {
            let settings = preset.settings(true);
            xubamp_render::equalizer::Preset {
                name: preset.name,
                preamp_db: settings.preamp_db,
                bands_db: settings.bands_db,
            }
        })
        .collect()
}

fn dispatch_portal_request(launcher: &portal_actions::Launcher, request: xubamp_wl::MenuRequest) {
    match launcher.launch(request.clone()) {
        portal_actions::LaunchResult::Started => {}
        portal_actions::LaunchResult::Busy => {
            eprintln!("xubamp: a file chooser is already open")
        }
        portal_actions::LaunchResult::Unsupported => {
            eprintln!("xubamp: menu action pending integration: {request:?}")
        }
    }
}

/// Carry out a playlist-editor mutation on the player. Save and Load need a native file dialog, so
/// they go through the portal launcher (whose completions are applied on the event-loop tick); the
/// rest mutate the player directly and the playlist pane resyncs on the next tick.
#[cfg(feature = "audio")]
fn apply_playlist_request(
    player: &std::rc::Rc<std::cell::RefCell<player::Player>>,
    launcher: &portal_actions::Launcher,
    op: xubamp_wl::PlaylistRequest,
) {
    use xubamp_wl::PlaylistRequest as P;
    if matches!(op, P::Save | P::Load) {
        dispatch_portal_request(launcher, xubamp_wl::MenuRequest::Playlist(op));
        return;
    }
    let mut player = player.borrow_mut();
    match op {
        P::RemoveSelected(indices) => player.remove_indices(&indices),
        P::Crop(indices) => player.crop_indices(&indices),
        P::RemoveAll => player.clear_playlist(),
        P::RemoveDead => player.remove_dead_tracks(),
        P::Reverse => player.reverse_playlist(),
        P::Randomize => player.randomize_playlist(),
        P::Sort(sort) => {
            let mut entries = player.playlist_entries();
            entries.sort_by(|a, b| {
                playlist_sort_key(sort, &a.1).cmp(&playlist_sort_key(sort, &b.1))
            });
            let order: Vec<_> = entries.into_iter().map(|(id, _)| id).collect();
            player.reorder_playlist(&order);
        }
        P::Save | P::Load => {}
    }
}

/// The comparable key a playlist Sort uses: the derived display title, the file name, or the whole
/// path, each lowercased so the sort is case-insensitive like Winamp's.
#[cfg(feature = "audio")]
fn playlist_sort_key(sort: xubamp_wl::PlaylistSort, path: &Path) -> String {
    use xubamp_wl::PlaylistSort as S;
    match sort {
        S::Title => track_title(&path.to_string_lossy()).to_lowercase(),
        S::Filename => path
            .file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
            .unwrap_or_default(),
        S::Path => path.to_string_lossy().to_lowercase(),
    }
}

/// Read and parse a `.m3u`/`.m3u8`/`.pls` file into its entry paths (relative entries resolved
/// against the playlist's own directory). Non-audio entries are dropped later by the player.
#[cfg(feature = "audio")]
fn load_playlist_paths(path: &Path) -> Result<Vec<PathBuf>, String> {
    use xubamp_audio::playlist_file;
    let format = playlist_file::format_from_path(path)
        .ok_or_else(|| format!("{} is not a playlist file", path.display()))?;
    let bytes =
        std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let entries = playlist_file::parse(&text, format, path.parent());
    Ok(entries.into_iter().map(|entry| entry.path).collect())
}

/// Serialize the given entries to `dest`, choosing PLS or extended M3U by the destination's
/// extension. Paths under the destination's directory are written relative for a portable file.
#[cfg(feature = "audio")]
fn write_playlist(
    dest: &Path,
    items: &[xubamp_audio::playlist_file::PlaylistEntry],
) -> Result<(), String> {
    use xubamp_audio::playlist_file::{self, PlaylistFormat};
    let base = dest.parent();
    let text = match playlist_file::format_from_path(dest) {
        Some(PlaylistFormat::Pls) => playlist_file::write_pls(items, base),
        _ => playlist_file::write_m3u(items, base),
    };
    std::fs::write(dest, text).map_err(|error| format!("cannot write {}: {error}", dest.display()))
}

/// The session playlist sidecar: the same directory as the settings file. Written on exit and
/// read back on an argument-less start, so closing the player never loses the playlist.
#[cfg(feature = "audio")]
fn session_playlist_path(settings_path: &Path) -> PathBuf {
    settings_path.with_file_name("session.m3u8")
}

/// The player's Options-page behaviours as persisted.
#[cfg(feature = "audio")]
fn player_options_from(settings: &xubamp_config::Settings) -> player::PlayerOptions {
    player::PlayerOptions {
        read_titles_on_load: settings.playback.read_titles_on_load,
        sort_on_load: settings.playback.sort_on_load,
        manual_advance: settings.playback.manual_advance,
        playlist_numbers: settings.display.playlist_numbers,
        convert_underscores: settings.display.convert_underscores,
        convert_percent20: settings.display.convert_percent20,
    }
}

/// Assemble everything the file-info box shows for a track: filesystem size, header-level stream
/// facts, and the tag form prefilled from the ID3v1 tail (falling back to the embedded ID3v2 or
/// Vorbis artist/title for display). Editable only for MP3, whose ID3v1 tail we can write.
#[cfg(feature = "audio")]
fn file_info_data(
    player: &player::Player,
    query: xubamp_wl::FileInfoQuery,
) -> Option<xubamp_render::fileinfo::FileInfoData> {
    use xubamp_render::fileinfo::{FileInfoData, TrackFacts};
    let path = match query {
        xubamp_wl::FileInfoQuery::Current => player.current_path()?,
        xubamp_wl::FileInfoQuery::Row(index) => player.track_path(index)?,
    };
    let size_bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    let stream = xubamp_audio::decode::probe_stream_info(&path).unwrap_or_default();
    let bitrate_kbps = stream.duration_secs.and_then(|secs| {
        (secs > 0).then(|| (size_bytes.saturating_mul(8) / 1000 / u64::from(secs)) as u32)
    });
    let v1 = xubamp_audio::id3v1::read(&path).ok().flatten().unwrap_or_default();
    let embedded = xubamp_audio::decode::probe_tags(&path).unwrap_or_default();
    let or_embedded = |v1_field: String, embedded_field: Option<String>| {
        if v1_field.is_empty() {
            embedded_field.unwrap_or_default()
        } else {
            v1_field
        }
    };
    let fields = [
        or_embedded(v1.title.clone(), embedded.title),
        or_embedded(v1.artist.clone(), embedded.artist),
        v1.album.clone(),
        v1.year.clone(),
        v1.comment.clone(),
        v1.genre_name(),
        v1.track.map(|t| t.to_string()).unwrap_or_default(),
    ];
    let editable = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("mp3"));
    Some(FileInfoData {
        facts: TrackFacts {
            path: path.display().to_string(),
            size_bytes,
            duration_secs: stream.duration_secs,
            bitrate_kbps,
            sample_rate_hz: stream.sample_rate,
            channels: stream.channels,
            codec: stream.codec,
        },
        path,
        fields,
        editable,
    })
}

/// Write the file-info box's edited fields as the track's ID3v1 tail.
#[cfg(feature = "audio")]
fn write_file_info(request: &xubamp_render::fileinfo::SaveRequest) -> Result<(), String> {
    use xubamp_audio::id3v1::{self, Id3v1};
    let fields = &request.fields;
    let tag = Id3v1 {
        title: fields[0].clone(),
        artist: fields[1].clone(),
        album: fields[2].clone(),
        year: fields[3].clone(),
        comment: fields[4].clone(),
        genre: Id3v1::genre_from_name(&fields[5]),
        track: fields[6].trim().parse::<u8>().ok().filter(|&t| t != 0),
    };
    id3v1::write(&request.path, &tag).map_err(|error| format!("Cannot write the tag: {error}"))
}

/// Eject always invokes the replace-and-play chooser. Play does so only for an empty playlist;
/// Pause stays inert there instead of becoming another chooser shortcut.
fn transport_opens_media(t: xubamp_render::hit::Transport, playlist_empty: bool) -> bool {
    matches!(t, xubamp_render::hit::Transport::Eject)
        || (playlist_empty && matches!(t, xubamp_render::hit::Transport::Play))
}

/// A primitive engine operation. Transport commands (Play/Pause/Stop) map to a short sequence of
/// these; keeping the mapping pure (independent of the live engine) lets the play/pause/stop policy
/// be unit-tested on the host without PipeWire. Only compiled with `audio` (where it is used) or
/// under `test` (where it is exercised).
#[cfg(any(feature = "audio", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EngineOp {
    /// Rewind the decoder to frame 0.
    SeekToStart,
    /// Activate (`true`) or deactivate (`false`) the output stream.
    SetActive(bool),
}

/// Playback state relevant to the classic transport buttons. `Paused` and `Stopped` are both
/// inactive at the audio backend, but Play must resume the former in place and restart the latter.
#[cfg(any(feature = "audio", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransportState {
    Playing,
    Paused,
    Stopped,
    Finished,
}

/// The engine operations a transport button maps to. Play restarts unless playback is paused, in
/// which case it resumes in place. Pause toggles while playing/paused and acts like Play from a
/// stopped or naturally-finished state. Stop halts and rewinds. This mirrors Webamp's media entry
/// point: `play()` seeks to zero for every state except Paused, while the Pause action dispatches
/// Play whenever it is not currently playing.
#[cfg(any(feature = "audio", test))]
fn transport_ops(t: xubamp_render::hit::Transport, state: TransportState) -> Vec<EngineOp> {
    use xubamp_render::hit::Transport;
    use EngineOp::{SeekToStart, SetActive};
    match t {
        Transport::Play if state == TransportState::Paused => vec![SetActive(true)],
        Transport::Play => vec![SeekToStart, SetActive(true)],
        Transport::Pause if state == TransportState::Playing => vec![SetActive(false)],
        Transport::Pause if state == TransportState::Paused => vec![SetActive(true)],
        Transport::Pause => vec![SeekToStart, SetActive(true)],
        Transport::Stop => vec![SetActive(false), SeekToStart],
        Transport::Prev | Transport::Next | Transport::Eject => Vec::new(),
    }
}

fn main() {
    let (skin_arg, media_args) = classify(std::env::args().skip(1));
    let settings_path = xubamp_config::default_path();
    let settings = load_settings(settings_path.as_deref());
    let skin = resolve_skin(skin_arg.as_deref(), settings.skin_path.as_deref());

    // The marquee shows the first track's file name (tag-based titles arrive later); it updates per
    // track as the playlist advances.
    let title = media_args
        .first()
        .map(|p| track_title(p))
        .unwrap_or_default();
    let equalizer_state = xubamp_render::equalizer::EqState {
        enabled: settings.equalizer.enabled,
        preamp_db: settings.equalizer.preamp_db,
        bands_db: settings.equalizer.bands_db,
        shade: settings.windows.equalizer.shaded,
        ..Default::default()
    };
    let pane_layout = xubamp_wl::PaneLayout {
        main_shaded: settings.windows.main.shaded,
        equalizer_open: settings.windows.equalizer.open,
        equalizer_position: (settings.windows.equalizer.x, settings.windows.equalizer.y),
        playlist_open: settings.windows.playlist.open,
        playlist_shaded: settings.windows.playlist.shaded,
        playlist_position: (settings.windows.playlist.x, settings.windows.playlist.y),
        playlist_size: (
            settings.windows.playlist.width,
            settings.windows.playlist.height,
        ),
    };
    let ui_options = xubamp_wl::UiOptions {
        time_display: ui_time_display(settings.display.time),
        scroll_title: settings.display.scroll_title,
        double_size: settings.display.double_size,
        show_clutterbar: settings.display.show_clutterbar,
        snap_px: settings.display.snap_px,
        visualization_mode: match settings.visualization.mode {
            xubamp_config::VisualizationMode::Spectrum => xubamp_render::vis::VisMode::Bars,
            xubamp_config::VisualizationMode::Oscilloscope => {
                xubamp_render::vis::VisMode::Oscilloscope
            }
            xubamp_config::VisualizationMode::Off => xubamp_render::vis::VisMode::Off,
        },
        visualization_show_peaks: settings.visualization.show_peaks,
        analyzer_style: vis_analyzer_style(settings.visualization.analyzer_style),
        band_width: vis_band_width(settings.visualization.band_width),
        oscilloscope_style: vis_osc_style(settings.visualization.oscilloscope_style),
        bar_falloff: settings.visualization.bar_falloff,
        peak_falloff: settings.visualization.peak_falloff,
        refresh_rate: settings.visualization.refresh_rate,
        // Native (non-skin) menus and dialogs follow the desktop's light/dark preference, read once
        // at startup from the settings portal. Unreachable portal falls back to light.
        dark: matches!(
            xubamp_portal::read_color_scheme_blocking(),
            xubamp_portal::ColorScheme::Dark
        ),
    };
    let equalizer_presets = classic_equalizer_presets();

    // Debug affordance / seed for the later headless render-diff harness: dump the raw RGBA the
    // window would display, then exit without opening a window. `XUBAMP_TITLE` overrides the
    // marquee text, `XUBAMP_VOLUME` (0-100) / `XUBAMP_BALANCE` (-100..100) the slider positions,
    // and `XUBAMP_POSITION` (0.0-1.0) the seek-bar thumb, so those strips can be diffed without a
    // real media file or live input.
    if let Ok(path) = std::env::var("XUBAMP_DUMP_RGBA") {
        let mut state = xubamp_render::hit::UiState {
            title: std::env::var("XUBAMP_TITLE").unwrap_or_else(|_| title.clone()),
            time_display: ui_options.time_display,
            ..Default::default()
        };
        if let Some(v) = std::env::var("XUBAMP_VOLUME")
            .ok()
            .and_then(|s| s.parse().ok())
        {
            state.volume = v;
        }
        if let Some(b) = std::env::var("XUBAMP_BALANCE")
            .ok()
            .and_then(|s| s.parse().ok())
        {
            state.balance = b;
        }
        if let Some(p) = std::env::var("XUBAMP_POSITION")
            .ok()
            .and_then(|s| s.parse().ok())
        {
            state.position = Some(p);
        }
        let fb = xubamp_render::compose_main_window(&skin, &state);
        std::fs::write(&path, &fb.rgba).expect("write rgba dump");
        println!("dumped {}x{} rgba to {path}", fb.width, fb.height);
        return;
    }

    // Play the media playlist (with the audio feature). The Player owns the current AudioEngine and
    // switches tracks on prev/next and auto-advance; it lives on this (the UI) thread and is shared
    // with the window's callbacks through Rc<RefCell>, which the calloop event loop borrows one at a
    // time (all on this thread, so no locking).
    #[cfg(feature = "audio")]
    {
        use std::cell::RefCell;
        use std::rc::Rc;

        let (portal_launcher, mut portal_receiver) = portal_actions::bridge();
        // CLI tracks start a fresh playlist and autoplay. With no arguments, the previous
        // session's playlist comes back from its sidecar file, selected but stopped, so opening
        // the player never blasts audio uninvited.
        let session_playlist = settings_path.as_deref().map(session_playlist_path);
        let restoring = media_args.is_empty();
        let tracks: Vec<std::path::PathBuf> = if restoring {
            session_playlist
                .as_deref()
                .filter(|path| path.exists())
                .and_then(|path| load_playlist_paths(path).ok())
                .unwrap_or_default()
        } else {
            media_args.iter().map(std::path::PathBuf::from).collect()
        };
        let equalizer = xubamp_audio::EqSettings {
            enabled: settings.equalizer.enabled,
            preamp_db: settings.equalizer.preamp_db,
            bands_db: settings.equalizer.bands_db,
        };
        let session_track = settings.playback.session_track as usize;
        let player = Rc::new(RefCell::new(player::Player::with_settings_and_options(
            tracks,
            settings.playback.shuffle,
            settings.playback.repeat,
            settings.playback.shuffle_morph_rate,
            equalizer,
            player_options_from(&settings),
        )));
        let settings = Rc::new(RefCell::new(settings));
        if restoring {
            player.borrow_mut().restore_selection(session_track);
        } else {
            player.borrow_mut().start(); // begin the first track
        }

        let on_command = {
            let player = Rc::clone(&player);
            let settings = Rc::clone(&settings);
            let settings_path = settings_path.clone();
            let portal_launcher = portal_launcher.clone();
            move |command: xubamp_render::hit::Command| {
                use xubamp_render::hit::Command;
                let mut player = player.borrow_mut();
                match command {
                    Command::Transport(t) => {
                        let opens_media = transport_opens_media(t, player.is_empty());
                        if opens_media {
                            drop(player);
                            dispatch_portal_request(
                                &portal_launcher,
                                xubamp_wl::MenuRequest::OpenMedia,
                            );
                        } else {
                            player.transport(t);
                        }
                    }
                    Command::Volume(v) => player.set_volume(v),
                    Command::Balance(b) => player.set_balance(b),
                    Command::Seek(fraction) => player.seek_fraction(fraction),
                    Command::Restart => player.restart(),
                    Command::ToggleMode(mode) => {
                        player.toggle_mode(mode);
                        let result = persist_playback_modes(
                            settings_path.as_deref(),
                            &mut settings.borrow_mut(),
                            player.shuffle(),
                            player.repeat(),
                        );
                        if let Err(error) = result {
                            eprintln!("xubamp: cannot save playback settings: {error}");
                        }
                    }
                    Command::PlayIndex(i) => player.play_index(i),
                    Command::SkipTracks(delta) => player.skip_tracks(delta),
                }
            }
        };
        let on_equalizer = {
            let player = Rc::clone(&player);
            let settings = Rc::clone(&settings);
            move |command: xubamp_render::equalizer::Command| {
                use xubamp_render::equalizer::Command;
                let mut player = player.borrow_mut();
                match command {
                    Command::Volume(volume) => player.set_volume(volume),
                    Command::Balance(balance) => player.set_balance(balance),
                    Command::Enabled(enabled) => {
                        let mut equalizer = player.equalizer_settings();
                        equalizer.enabled = enabled;
                        player.set_equalizer_settings(equalizer);
                        settings.borrow_mut().equalizer.enabled = enabled;
                    }
                    Command::Preamp(preamp_db) => {
                        let mut equalizer = player.equalizer_settings();
                        equalizer.preamp_db = preamp_db;
                        player.set_equalizer_settings(equalizer);
                        settings.borrow_mut().equalizer.preamp_db = preamp_db;
                    }
                    Command::Band { index, db } => {
                        let mut equalizer = player.equalizer_settings();
                        if let Some(band) = equalizer.bands_db.get_mut(index) {
                            *band = db;
                            player.set_equalizer_settings(equalizer);
                            settings.borrow_mut().equalizer.bands_db[index] = db;
                        }
                    }
                    Command::Preset {
                        preamp_db,
                        bands_db,
                    } => {
                        let equalizer = xubamp_audio::EqSettings {
                            enabled: player.equalizer_settings().enabled,
                            preamp_db,
                            bands_db,
                        };
                        player.set_equalizer_settings(equalizer);
                        let mut settings = settings.borrow_mut();
                        settings.equalizer.preamp_db = preamp_db;
                        settings.equalizer.bands_db = bands_db;
                    }
                }
            }
        };
        let on_menu = {
            let portal_launcher = portal_launcher.clone();
            let settings = Rc::clone(&settings);
            let settings_path = settings_path.clone();
            let player = Rc::clone(&player);
            move |request: xubamp_wl::MenuRequest| match request {
                xubamp_wl::MenuRequest::Action(
                    xubamp_render::menu::ClassicMenuAction::UseBaseSkin,
                ) => {
                    if let Err(error) = persist_skin_path(
                        settings_path.as_deref(),
                        &mut settings.borrow_mut(),
                        None,
                    ) {
                        eprintln!("xubamp: cannot save base skin selection: {error}");
                    }
                }
                xubamp_wl::MenuRequest::Playlist(op) => {
                    apply_playlist_request(&player, &portal_launcher, op);
                }
                other => dispatch_portal_request(&portal_launcher, other),
            }
        };
        let playback_source = {
            let player = Rc::clone(&player);
            move || {
                let mut player = player.borrow_mut();
                player.poll(); // auto-advance to the next track when the current one ends
                player.playback()
            }
        };
        let sample_source = {
            let player = Rc::clone(&player);
            move |out: &mut [f32]| player.borrow().read_scope(out)
        };
        let playlist_source = {
            let player = Rc::clone(&player);
            move || player.borrow().playlist_view()
        };
        let file_info_source = {
            let player = Rc::clone(&player);
            move |query| file_info_data(&player.borrow(), query)
        };
        let on_file_info_save = {
            let player = Rc::clone(&player);
            move |request: &xubamp_render::fileinfo::SaveRequest| {
                write_file_info(request)?;
                // The file changed under the caches: re-probe so rows and marquee update.
                player.borrow_mut().refresh_metadata(&request.path);
                Ok(())
            }
        };
        let external_source = {
            let player = Rc::clone(&player);
            let settings = Rc::clone(&settings);
            let settings_path = settings_path.clone();
            move || {
                let (completions, pending) = portal_receiver.poll();
                let mut events = Vec::new();
                for completion in completions {
                    match completion {
                        portal_actions::Completion::AddPaths { paths, warnings } => {
                            for warning in warnings {
                                eprintln!("xubamp: directory scan warning: {warning}");
                            }
                            let count = player.borrow_mut().append_paths(paths).len();
                            eprintln!("xubamp: added {count} audio file(s) to the playlist");
                        }
                        portal_actions::Completion::OpenPaths(paths) => {
                            let mut player = player.borrow_mut();
                            let count = player.replace_paths(paths);
                            if count > 0 {
                                player.start();
                                eprintln!("xubamp: opened {count} audio file(s)");
                            }
                        }
                        portal_actions::Completion::EqualizerPreset(preset) => {
                            events.push(xubamp_wl::ExternalEvent::EqualizerPreset(preset))
                        }
                        portal_actions::Completion::Saved(path) => {
                            eprintln!("xubamp: saved equalizer preset to {}", path.display());
                        }
                        portal_actions::Completion::PlaylistToLoad(file) => {
                            match load_playlist_paths(&file) {
                                Ok(paths) => {
                                    let mut player = player.borrow_mut();
                                    let count = player.replace_paths(paths);
                                    if count > 0 {
                                        player.start();
                                        eprintln!(
                                            "xubamp: loaded {count} track(s) from {}",
                                            file.display()
                                        );
                                    } else {
                                        eprintln!(
                                            "xubamp: {} has no playable tracks",
                                            file.display()
                                        );
                                    }
                                }
                                Err(error) => eprintln!("xubamp: {error}"),
                            }
                        }
                        portal_actions::Completion::PlaylistToSave(dest) => {
                            let items: Vec<_> = player
                                .borrow()
                                .playlist_entries()
                                .into_iter()
                                .map(|(_, path)| xubamp_audio::playlist_file::PlaylistEntry {
                                    title: Some(track_title(&path.to_string_lossy())),
                                    duration_secs: None,
                                    path,
                                })
                                .collect();
                            match write_playlist(&dest, &items) {
                                Ok(()) => eprintln!(
                                    "xubamp: saved {} track(s) to {}",
                                    items.len(),
                                    dest.display()
                                ),
                                Err(error) => eprintln!("xubamp: {error}"),
                            }
                        }
                        portal_actions::Completion::SkinLoaded { path, skin } => {
                            if let Err(error) = persist_skin_path(
                                settings_path.as_deref(),
                                &mut settings.borrow_mut(),
                                Some(path.clone()),
                            ) {
                                eprintln!("xubamp: cannot save skin selection: {error}");
                            }
                            eprintln!("xubamp: loaded skin {}", path.display());
                            events.push(xubamp_wl::ExternalEvent::SkinLoaded(skin));
                        }
                        portal_actions::Completion::LibraryRoot(root) => {
                            let mut settings = settings.borrow_mut();
                            if !settings.library.roots.contains(&root) {
                                settings.library.roots.push(root.clone());
                                if let Some(path) = settings_path.as_deref() {
                                    if let Err(error) = xubamp_config::save(path, &settings) {
                                        eprintln!("xubamp: cannot save library root: {error}");
                                    }
                                }
                            }
                            events.push(xubamp_wl::ExternalEvent::LibraryRootChosen(root));
                        }
                        portal_actions::Completion::Error(error) => {
                            eprintln!("xubamp: {error}");
                        }
                    }
                }
                xubamp_wl::ExternalPoll { events, pending }
            }
        };
        // Committed Preferences values reach the live player and the settings file here. The window
        // opens seeded from the persisted settings so it shows the user's real values.
        let preferences_model = preferences_model_from(&settings.borrow());
        let on_preferences = {
            let player = Rc::clone(&player);
            let settings = Rc::clone(&settings);
            let settings_path = settings_path.clone();
            move |command: xubamp_render::preferences::Command| {
                if let xubamp_render::preferences::Command::SetShuffleMorphRate(rate) = command {
                    player.borrow_mut().set_shuffle_morph_rate(rate);
                }
                let mut settings = settings.borrow_mut();
                if apply_preference_to_settings(&command, &mut settings) {
                    // The Options-page behaviours act inside the player; refresh them from the
                    // just-updated settings so the change is live, not only persisted.
                    let options = player_options_from(&settings);
                    if player.borrow().options() != options {
                        player.borrow_mut().set_options(options);
                    }
                    if let Some(path) = settings_path.as_deref() {
                        if let Err(error) = xubamp_config::save(path, &settings) {
                            eprintln!("xubamp: cannot save preference: {error}");
                        }
                    }
                }
            }
        };

        let result = xubamp_wl::run(
            skin,
            title,
            equalizer_state,
            pane_layout,
            xubamp_wl::Runtime::new(
                on_command,
                on_equalizer,
                on_menu,
                equalizer_presets,
                playback_source,
                sample_source,
                playlist_source,
            )
            .with_ui_options(ui_options)
            .with_preferences(preferences_model, on_preferences)
            .with_file_info(file_info_source, on_file_info_save)
            .with_external_source(external_source),
        );
        if let Ok(session) = result.as_ref() {
            apply_ui_session(&mut settings.borrow_mut(), *session);
        }
        // The playlist survives close/reopen: write it (and the current row) beside the settings.
        settings.borrow_mut().playback.session_track =
            player.borrow().current_index().unwrap_or(0) as u32;
        if let Some(session_file) = session_playlist.as_deref() {
            let items: Vec<_> = player
                .borrow()
                .playlist_entries()
                .into_iter()
                .map(|(_, path)| xubamp_audio::playlist_file::PlaylistEntry {
                    title: Some(track_title(&path.to_string_lossy())),
                    duration_secs: None,
                    path,
                })
                .collect();
            if let Err(error) = write_playlist(session_file, &items) {
                eprintln!("xubamp: cannot save the session playlist: {error}");
            }
        }
        // Equalizer sliders update live audio continuously, but the final state and pane layout
        // reach disk once when the event loop exits rather than once per pointer-motion event.
        if let Some(path) = settings_path.as_deref() {
            if let Err(error) = xubamp_config::save(path, &settings.borrow()) {
                eprintln!("xubamp: cannot save settings on exit: {error}");
            }
        }
        if let Err(e) = result {
            eprintln!("xubamp: {e}");
            std::process::exit(1);
        }
    }

    // Without the audio feature: no playback, commands are logged, the clock stays blank.
    #[cfg(not(feature = "audio"))]
    {
        use std::cell::RefCell;
        use std::rc::Rc;

        if !media_args.is_empty() {
            eprintln!("xubamp: built without audio; rebuild with `--features audio` to play files");
        }
        let settings = Rc::new(RefCell::new(settings));
        let (portal_launcher, mut portal_receiver) = portal_actions::bridge();
        let on_command = {
            let portal_launcher = portal_launcher.clone();
            move |command: xubamp_render::hit::Command| {
                use xubamp_render::hit::Command;
                if matches!(command, Command::Transport(t) if transport_opens_media(t, true)) {
                    dispatch_portal_request(&portal_launcher, xubamp_wl::MenuRequest::OpenMedia);
                } else {
                    eprintln!("xubamp: command {command:?}");
                }
            }
        };
        let on_equalizer = |command: xubamp_render::equalizer::Command| {
            eprintln!("xubamp: equalizer command {command:?}")
        };
        let on_menu = {
            let settings = Rc::clone(&settings);
            let settings_path = settings_path.clone();
            let portal_launcher = portal_launcher.clone();
            move |request: xubamp_wl::MenuRequest| {
                if matches!(
                    request,
                    xubamp_wl::MenuRequest::Action(
                        xubamp_render::menu::ClassicMenuAction::UseBaseSkin
                    )
                ) {
                    if let Err(error) = persist_skin_path(
                        settings_path.as_deref(),
                        &mut settings.borrow_mut(),
                        None,
                    ) {
                        eprintln!("xubamp: cannot save base skin selection: {error}");
                    }
                } else {
                    dispatch_portal_request(&portal_launcher, request);
                }
            }
        };
        let playback_source = xubamp_render::hit::Playback::default;
        let sample_source = |out: &mut [f32]| out.iter_mut().for_each(|s| *s = 0.0);
        let playlist_source = || (Vec::new(), None);
        let external_source = {
            let settings = Rc::clone(&settings);
            let settings_path = settings_path.clone();
            move || {
                let (completions, pending) = portal_receiver.poll();
                let mut events = Vec::new();
                for completion in completions {
                    match completion {
                        portal_actions::Completion::EqualizerPreset(preset) => {
                            events.push(xubamp_wl::ExternalEvent::EqualizerPreset(preset));
                        }
                        portal_actions::Completion::AddPaths { paths, warnings } => {
                            for warning in warnings {
                                eprintln!("xubamp: directory scan warning: {warning}");
                            }
                            eprintln!(
                                "xubamp: built without audio; {} selected file(s) were not added",
                                paths.len()
                            );
                        }
                        portal_actions::Completion::OpenPaths(paths) => {
                            eprintln!(
                                "xubamp: built without audio; {} opened file(s) were not played",
                                paths.len()
                            );
                        }
                        portal_actions::Completion::Saved(path) => {
                            eprintln!("xubamp: saved equalizer preset to {}", path.display());
                        }
                        portal_actions::Completion::PlaylistToLoad(_)
                        | portal_actions::Completion::PlaylistToSave(_) => {
                            eprintln!(
                                "xubamp: built without audio; playlist save/load needs `--features audio`"
                            );
                        }
                        portal_actions::Completion::SkinLoaded { path, skin } => {
                            if let Err(error) = persist_skin_path(
                                settings_path.as_deref(),
                                &mut settings.borrow_mut(),
                                Some(path.clone()),
                            ) {
                                eprintln!("xubamp: cannot save skin selection: {error}");
                            }
                            eprintln!("xubamp: loaded skin {}", path.display());
                            events.push(xubamp_wl::ExternalEvent::SkinLoaded(skin));
                        }
                        portal_actions::Completion::LibraryRoot(root) => {
                            let mut settings = settings.borrow_mut();
                            if !settings.library.roots.contains(&root) {
                                settings.library.roots.push(root.clone());
                                if let Some(path) = settings_path.as_deref() {
                                    if let Err(error) = xubamp_config::save(path, &settings) {
                                        eprintln!("xubamp: cannot save library root: {error}");
                                    }
                                }
                            }
                            events.push(xubamp_wl::ExternalEvent::LibraryRootChosen(root));
                        }
                        portal_actions::Completion::Error(error) => eprintln!("xubamp: {error}"),
                    }
                }
                xubamp_wl::ExternalPoll { events, pending }
            }
        };
        // No player in this build, so committed preferences only persist to the settings file.
        let preferences_model = preferences_model_from(&settings.borrow());
        let on_preferences = {
            let settings = Rc::clone(&settings);
            let settings_path = settings_path.clone();
            move |command: xubamp_render::preferences::Command| {
                let mut settings = settings.borrow_mut();
                if apply_preference_to_settings(&command, &mut settings) {
                    if let Some(path) = settings_path.as_deref() {
                        if let Err(error) = xubamp_config::save(path, &settings) {
                            eprintln!("xubamp: cannot save preference: {error}");
                        }
                    }
                }
            }
        };
        match xubamp_wl::run(
            skin,
            title,
            equalizer_state,
            pane_layout,
            xubamp_wl::Runtime::new(
                on_command,
                on_equalizer,
                on_menu,
                equalizer_presets,
                playback_source,
                sample_source,
                playlist_source,
            )
            .with_ui_options(ui_options)
            .with_preferences(preferences_model, on_preferences)
            .with_external_source(external_source),
        ) {
            Ok(session) => {
                apply_ui_session(&mut settings.borrow_mut(), session);
                if let Some(path) = settings_path.as_deref() {
                    if let Err(error) = xubamp_config::save(path, &settings.borrow()) {
                        eprintln!("xubamp: cannot save settings on exit: {error}");
                    }
                }
            }
            Err(error) => {
                eprintln!("xubamp: {error}");
                std::process::exit(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_preference_to_settings, apply_ui_session, classic_equalizer_presets, classify,
        config_time_display, persist_playback_modes, persist_skin_path, preferences_model_from,
        preferred_skin_path, track_title, transport_opens_media, transport_ops, ui_time_display,
        EngineOp, TransportState, DEV_SKIN,
    };
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};
    use xubamp_render::hit::Transport;

    #[test]
    fn only_eject_and_empty_play_open_the_media_picker() {
        assert!(transport_opens_media(Transport::Eject, false));
        assert!(transport_opens_media(Transport::Eject, true));
        assert!(transport_opens_media(Transport::Play, true));
        assert!(!transport_opens_media(Transport::Play, false));
        assert!(!transport_opens_media(Transport::Pause, true));
        assert!(!transport_opens_media(Transport::Stop, true));
        assert!(!transport_opens_media(Transport::Next, true));
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn temp_settings_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("xubamp-main-test-{}-{nonce}", std::process::id()))
            .join("settings.conf")
    }

    #[test]
    fn skin_priority_is_cli_then_environment_then_saved_then_development() {
        let saved = Path::new("/saved/theme.wsz");
        let env = Some(OsString::from("/environment/theme.wsz"));
        assert_eq!(
            preferred_skin_path(Some("/cli/theme.wsz"), env.clone(), Some(saved), true, true),
            Some(PathBuf::from("/cli/theme.wsz"))
        );
        assert_eq!(
            preferred_skin_path(None, env, Some(saved), true, true),
            Some(PathBuf::from("/environment/theme.wsz"))
        );
        assert_eq!(
            preferred_skin_path(None, None, Some(saved), true, true),
            Some(saved.to_path_buf())
        );
        assert_eq!(
            preferred_skin_path(None, None, Some(saved), false, true),
            Some(PathBuf::from(DEV_SKIN)),
            "a deleted saved skin falls through to the development/base choices"
        );
        assert_eq!(preferred_skin_path(None, None, None, false, false), None);
    }

    #[test]
    fn classic_equalizer_menu_uses_the_canonical_dsp_presets() {
        let presets = classic_equalizer_presets();
        assert_eq!(presets.len(), 17);
        assert_eq!(presets.first().unwrap().name, "Classical");
        assert_eq!(presets.last().unwrap().name, "Techno");
        assert!(presets.iter().all(|preset| {
            preset.preamp_db.is_finite()
                && preset.bands_db.iter().all(|db| (-12.0..=12.0).contains(db))
        }));
    }

    #[test]
    fn mode_persistence_changes_only_modes_and_round_trips_atomically() {
        let path = temp_settings_path();
        let mut settings = xubamp_config::Settings::default();
        settings.playback.shuffle_morph_rate = 17;
        settings.equalizer.preamp_db = 3.5;
        settings.skin_path = Some(PathBuf::from("/skins/kept.wsz"));

        persist_playback_modes(Some(&path), &mut settings, true, true).unwrap();

        let loaded = xubamp_config::load(&path).unwrap().settings;
        assert!(loaded.playback.shuffle);
        assert!(loaded.playback.repeat);
        assert_eq!(loaded.playback.shuffle_morph_rate, 17);
        assert_eq!(loaded.equalizer.preamp_db, 3.5);
        assert_eq!(loaded.skin_path, Some(PathBuf::from("/skins/kept.wsz")));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn mode_persistence_without_a_config_directory_still_updates_memory() {
        let mut settings = xubamp_config::Settings::default();
        persist_playback_modes(None, &mut settings, true, false).unwrap();
        assert!(settings.playback.shuffle);
        assert!(!settings.playback.repeat);
    }

    #[test]
    fn preferences_seed_from_and_fold_back_into_settings() {
        use xubamp_render::preferences::{Command, TimeDisplay, VisualizationMode};

        // The model opens seeded from the persisted settings, not defaults.
        let mut settings = xubamp_config::Settings::default();
        settings.playback.shuffle_morph_rate = 20;
        settings.display.scroll_title = false;
        settings.display.double_size = true;
        settings.visualization.mode = xubamp_config::VisualizationMode::Oscilloscope;
        settings.library.roots = vec![PathBuf::from("/music")];
        let model = preferences_model_from(&settings);
        assert_eq!(model.shuffle_morph_rate, 20);
        assert!(!model.display_scroll_title);
        assert!(model.display_double_size);
        assert_eq!(model.visualization_mode, VisualizationMode::Oscilloscope);
        assert_eq!(model.library_roots, vec![PathBuf::from("/music")]);

        // Committed commands fold back into the settings, and report whether a value changed.
        let mut s = xubamp_config::Settings::default();
        s.library.recurse = true;
        assert!(apply_preference_to_settings(
            &Command::SetShuffleMorphRate(33),
            &mut s
        ));
        assert_eq!(s.playback.shuffle_morph_rate, 33);
        assert!(apply_preference_to_settings(
            &Command::SetDisplayTime(TimeDisplay::Remaining),
            &mut s
        ));
        assert_eq!(s.display.time, xubamp_config::TimeDisplay::Remaining);
        assert!(apply_preference_to_settings(
            &Command::SetLibraryRecurse(false),
            &mut s
        ));
        assert!(!s.library.recurse);
        // The picker-opening commands are not themselves settings mutations.
        assert!(!apply_preference_to_settings(
            &Command::ChooseLibraryDirectory,
            &mut s
        ));
    }

    #[test]
    fn skin_persistence_switches_between_validated_archive_and_base() {
        let path = temp_settings_path();
        let mut settings = xubamp_config::Settings::default();
        let archive = PathBuf::from("/skins/selected.wsz");

        persist_skin_path(Some(&path), &mut settings, Some(archive.clone())).unwrap();
        assert_eq!(
            xubamp_config::load(&path).unwrap().settings.skin_path,
            Some(archive)
        );

        persist_skin_path(Some(&path), &mut settings, None).unwrap();
        assert_eq!(xubamp_config::load(&path).unwrap().settings.skin_path, None);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn final_ui_session_updates_panes_and_equalizer_without_losing_other_settings() {
        let mut settings = xubamp_config::Settings::default();
        settings.playback.shuffle = true;
        settings.skin_path = Some(PathBuf::from("/skins/kept.wsz"));
        let session = xubamp_wl::SessionState {
            panes: xubamp_wl::PaneLayout {
                main_shaded: true,
                equalizer_open: true,
                equalizer_position: (-12, 14),
                playlist_open: true,
                playlist_shaded: true,
                playlist_position: (263, 14),
                playlist_size: (550, 232),
            },
            equalizer_enabled: false,
            equalizer_shaded: true,
            equalizer_preamp_db: 3.5,
            equalizer_bands_db: [-12.0, -9.0, -6.0, -3.0, 0.0, 3.0, 6.0, 9.0, 12.0, 1.5],
            time_display: xubamp_render::hit::TimeDisplay::Remaining,
            scroll_title: false,
            double_size: true,
            visualization_mode: xubamp_render::vis::VisMode::Oscilloscope,
            visualization_show_peaks: false,
            analyzer_style: xubamp_render::vis::AnalyzerStyle::Fire,
            band_width: xubamp_render::vis::BandWidth::Thin,
            oscilloscope_style: xubamp_render::vis::OscStyle::Solid,
            bar_falloff: 3,
            peak_falloff: 9,
            refresh_rate: 4,
        };

        apply_ui_session(&mut settings, session);

        assert!(settings.windows.main.shaded);
        assert!(settings.windows.equalizer.open);
        assert!(settings.windows.equalizer.shaded);
        assert_eq!(
            (settings.windows.equalizer.x, settings.windows.equalizer.y),
            (-12, 14)
        );
        assert!(settings.windows.playlist.open);
        assert!(settings.windows.playlist.shaded);
        assert_eq!(
            (
                settings.windows.playlist.x,
                settings.windows.playlist.y,
                settings.windows.playlist.width,
                settings.windows.playlist.height,
            ),
            (263, 14, 550, 232)
        );
        assert!(!settings.equalizer.enabled);
        assert_eq!(settings.equalizer.preamp_db, 3.5);
        assert_eq!(settings.equalizer.bands_db, session.equalizer_bands_db);
        assert_eq!(settings.display.time, xubamp_config::TimeDisplay::Remaining);
        assert!(!settings.display.scroll_title);
        assert!(settings.display.double_size);
        assert_eq!(
            settings.visualization.mode,
            xubamp_config::VisualizationMode::Oscilloscope
        );
        assert!(!settings.visualization.show_peaks);
        assert!(
            settings.playback.shuffle,
            "unrelated playback state is kept"
        );
        assert_eq!(settings.skin_path, Some(PathBuf::from("/skins/kept.wsz")));
    }

    #[test]
    fn config_and_render_time_modes_round_trip_without_loss() {
        for config in [
            xubamp_config::TimeDisplay::Elapsed,
            xubamp_config::TimeDisplay::Remaining,
        ] {
            assert_eq!(config_time_display(ui_time_display(config)), config);
        }
    }

    #[test]
    fn play_resumes_only_when_paused_and_otherwise_restarts() {
        // Already playing: restart from the beginning, like the X hotkey.
        assert_eq!(
            transport_ops(Transport::Play, TransportState::Playing),
            vec![EngineOp::SeekToStart, EngineOp::SetActive(true)],
            "playing: Play restarts",
        );
        // Paused: resume in place.
        assert_eq!(
            transport_ops(Transport::Play, TransportState::Paused),
            vec![EngineOp::SetActive(true)],
            "paused: Play resumes in place",
        );
        for state in [TransportState::Stopped, TransportState::Finished] {
            assert_eq!(
                transport_ops(Transport::Play, state),
                vec![EngineOp::SeekToStart, EngineOp::SetActive(true)],
                "{state:?}: Play starts from the top",
            );
        }
    }

    #[test]
    fn pause_toggles_from_the_live_playing_state() {
        assert_eq!(
            transport_ops(Transport::Pause, TransportState::Playing),
            vec![EngineOp::SetActive(false)],
            "playing -> pause",
        );
        assert_eq!(
            transport_ops(Transport::Pause, TransportState::Paused),
            vec![EngineOp::SetActive(true)],
            "paused -> resume",
        );
        for state in [TransportState::Stopped, TransportState::Finished] {
            assert_eq!(
                transport_ops(Transport::Pause, state),
                vec![EngineOp::SeekToStart, EngineOp::SetActive(true)],
                "{state:?}: Pause acts like Play",
            );
        }
    }

    #[test]
    fn stop_halts_and_rewinds() {
        // Order matters: deactivate first, then rewind, so the clock shows 00:00 stopped.
        assert_eq!(
            transport_ops(Transport::Stop, TransportState::Playing),
            vec![EngineOp::SetActive(false), EngineOp::SeekToStart],
        );
    }

    #[test]
    fn skip_commands_map_to_nothing_until_a_playlist() {
        for t in [Transport::Prev, Transport::Next, Transport::Eject] {
            assert!(
                transport_ops(t, TransportState::Playing).is_empty(),
                "{t:?} maps to nothing yet"
            );
        }
    }

    #[test]
    fn track_title_is_the_file_stem() {
        assert_eq!(
            track_title("/music/Aphex Twin - Xtal.mp3"),
            "Aphex Twin - Xtal"
        );
        assert_eq!(track_title("song.flac"), "song");
        assert_eq!(track_title("no_extension"), "no_extension");
        assert_eq!(track_title(""), "");
        // A leading-dot name is extension-less to Rust, so the whole name is the stem.
        assert_eq!(track_title(".mp3"), ".mp3");
    }

    #[test]
    fn classifies_skin_and_media_by_extension() {
        let (skin, media) = classify(s(&["My Skin.wsz", "song.mp3"]));
        assert_eq!(skin.as_deref(), Some("My Skin.wsz"));
        assert_eq!(media, ["song.mp3"]);
    }

    #[test]
    fn order_independent_and_case_insensitive() {
        let (skin, media) = classify(s(&["track.MP3", "Base.WSZ"]));
        assert_eq!(skin.as_deref(), Some("Base.WSZ"));
        assert_eq!(media, ["track.MP3"]);
    }

    #[test]
    fn first_skin_wins_all_media_kept_in_order_unknown_ignored() {
        let (skin, media) = classify(s(&["notes.txt", "a.mp3", "b.wav", "one.wsz", "two.wsz"]));
        assert_eq!(skin.as_deref(), Some("one.wsz"), "the first skin wins");
        assert_eq!(
            media,
            ["a.mp3", "b.wav"],
            "every media file is kept as a playlist, in order"
        );
    }

    #[test]
    fn no_recognized_args_yields_none() {
        let (skin, media) = classify(s(&[
            "readme.md",
            "cover.png",
            "movie.mp4",
            "playlist.m3u",
        ]));
        assert!(skin.is_none());
        assert!(media.is_empty());
    }
}
