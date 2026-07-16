//! Pure state, interaction, accessibility metadata, and software rendering for the supported
//! preferences surface.
//!
//! The window is intentionally OS-neutral rather than skinned. Its command vocabulary mirrors the
//! persisted settings fields without depending on the config crate, leaving the platform layer to
//! apply commands and open file or directory choosers. Unsupported product areas have no section,
//! control, or command variant.

use std::collections::HashSet;
use std::path::PathBuf;

use xubamp_skin::font;

use crate::vis::{AnalyzerStyle, BandWidth, OscStyle};
use crate::adwaita::{self, Palette, UiFont};

use crate::Framebuffer;

/// Minimum preferences window size. Larger sizes give the library list more room.
pub const PREFERENCES_W: i32 = 500;
pub const PREFERENCES_H: i32 = 560;
/// Title-bar band height, exported for platform drag handling.
pub const PREFERENCES_TITLE_H: i32 = 24;

const PAD: i32 = 12;
const SIDEBAR_W: i32 = 122;
const SECTION_H: i32 = 28;
const CONTENT_GAP: i32 = 12;
const BOTTOM_H: i32 = 42;
const CONTROL_H: i32 = 22;
const CHECK_SIZE: i32 = 12;
const BUTTON_W: i32 = 116;
const ROOT_ROW_H: i32 = 20;
const SLIDER_THUMB_W: i32 = 11;
const SLIDER_THUMB_H: i32 = 20;

/// Values matching the classic persisted shuffle morph range.
pub const SHUFFLE_MORPH_RATE_MIN: u8 = 0;
pub const SHUFFLE_MORPH_RATE_MAX: u8 = 50;
pub const DEFAULT_SHUFFLE_MORPH_RATE: u8 = SHUFFLE_MORPH_RATE_MAX;

const BG: [u8; 3] = [226, 226, 222];
const FACE: [u8; 3] = [214, 214, 210];
const LIGHT: [u8; 3] = [255, 255, 255];
const SHADOW: [u8; 3] = [118, 118, 114];
const DARK: [u8; 3] = [42, 42, 40];
const TEXT: [u8; 3] = [20, 20, 18];
const DIM: [u8; 3] = [96, 96, 92];
const SELECTED: [u8; 3] = [58, 91, 146];
const SELECTED_TEXT: [u8; 3] = [255, 255, 255];
const FIELD: [u8; 3] = [250, 250, 248];

/// Supported preference pages, in their deterministic sidebar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Section {
    #[default]
    Shuffle,
    Options,
    Visualization,
    Display,
    AudioLibrary,
    Skins,
}

impl Section {
    pub const ALL: [Self; 6] = [
        Self::Shuffle,
        Self::Options,
        Self::Visualization,
        Self::Display,
        Self::AudioLibrary,
        Self::Skins,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Shuffle => "Shuffle",
            Self::Options => "Options",
            Self::Visualization => "Visualization",
            Self::Display => "Display",
            Self::AudioLibrary => "Audio Library",
            Self::Skins => "Skins",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|&item| item == self).unwrap_or(0)
    }

    fn offset(self, delta: i32) -> Self {
        let len = Self::ALL.len() as i32;
        let index = (self.index() as i32 + delta).rem_euclid(len) as usize;
        Self::ALL[index]
    }
}

/// Values matching `xubamp_config::TimeDisplay`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeDisplay {
    #[default]
    Elapsed,
    Remaining,
}

/// Values matching `xubamp_config::VisualizationMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisualizationMode {
    #[default]
    Spectrum,
    Oscilloscope,
    Off,
}

/// Caller-owned values displayed and edited by the preferences window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreferencesModel {
    /// Shuffle is enabled from the main player. This independent value controls how much the
    /// generated order changes between repeated cycles.
    pub shuffle_morph_rate: u8,
    pub visualization_mode: VisualizationMode,
    pub visualization_show_peaks: bool,
    pub visualization_analyzer_style: AnalyzerStyle,
    pub visualization_band_width: BandWidth,
    pub visualization_osc_style: OscStyle,
    /// Falloff and refresh speeds, 1..=10 like the visualization menu.
    pub visualization_bar_falloff: u8,
    pub visualization_peak_falloff: u8,
    pub visualization_refresh_rate: u8,
    pub display_time: TimeDisplay,
    pub display_double_size: bool,
    pub display_scroll_title: bool,
    pub display_clutterbar: bool,
    pub display_playlist_numbers: bool,
    /// Edge-snap threshold for pane drags, 0..=30 px (0 disables).
    pub display_snap_px: u8,
    /// Classic "Read titles on Load / Play".
    pub read_titles_on_load: bool,
    pub sort_on_load: bool,
    pub manual_advance: bool,
    pub convert_underscores: bool,
    pub convert_percent20: bool,
    /// Audio scan roots. The UI deliberately has no non-audio library model.
    pub library_roots: Vec<PathBuf>,
    pub library_recurse: bool,
    /// `None` is the authored base skin.
    pub skin_path: Option<PathBuf>,
}

impl Default for PreferencesModel {
    fn default() -> Self {
        Self {
            shuffle_morph_rate: DEFAULT_SHUFFLE_MORPH_RATE,
            visualization_mode: VisualizationMode::Spectrum,
            visualization_show_peaks: true,
            visualization_analyzer_style: AnalyzerStyle::Normal,
            visualization_band_width: BandWidth::Thick,
            visualization_osc_style: OscStyle::Lines,
            visualization_bar_falloff: 7,
            visualization_peak_falloff: 6,
            visualization_refresh_rate: 8,
            display_time: TimeDisplay::Elapsed,
            display_double_size: false,
            display_scroll_title: true,
            display_clutterbar: true,
            display_playlist_numbers: true,
            display_snap_px: 15,
            read_titles_on_load: true,
            sort_on_load: false,
            manual_advance: false,
            convert_underscores: false,
            convert_percent20: false,
            library_roots: Vec::new(),
            library_recurse: true,
            skin_path: None,
        }
    }
}

/// Stable identity for every interactive element. The platform layer can use this for accessible
/// names and roles without reverse-engineering pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlId {
    Section(Section),
    ShuffleMorphRate,
    VisualizationSpectrum,
    VisualizationOscilloscope,
    VisualizationOff,
    VisualizationShowPeaks,
    VisualizationRefreshRate,
    VisualizationAnalyzerNormal,
    VisualizationAnalyzerFire,
    VisualizationAnalyzerLine,
    VisualizationBandThick,
    VisualizationBandThin,
    VisualizationBarFalloff,
    VisualizationPeakFalloff,
    VisualizationOscDots,
    VisualizationOscLines,
    VisualizationOscSolid,
    DisplayElapsed,
    DisplayRemaining,
    DisplayDoubleSize,
    DisplayScrollTitle,
    DisplayClutterbar,
    DisplayPlaylistNumbers,
    DisplaySnapPx,
    OptionsReadOnLoad,
    OptionsReadOnPlay,
    OptionsSortOnLoad,
    OptionsManualAdvance,
    OptionsConvertUnderscores,
    OptionsConvertPercent20,
    LibraryRoot(usize),
    LibraryAdd,
    LibraryRemove,
    LibraryRecurse,
    SkinLoad,
    SkinBase,
    Close,
}

/// Accessible role for one rendered control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlRole {
    Tab,
    CheckBox,
    RadioButton,
    Slider,
    ListItem,
    Button,
}

/// Accessible value metadata for a range control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeInfo {
    pub value: u8,
    pub minimum: u8,
    pub maximum: u8,
    pub step: u8,
}

/// Integer rectangle using inclusive top/left and exclusive bottom/right edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}

/// Deterministic accessibility and hit-test metadata for one visible control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlInfo {
    pub id: ControlId,
    pub role: ControlRole,
    pub label: String,
    pub rect: Rect,
    pub enabled: bool,
    pub selected: bool,
    /// Present only for check boxes and radio buttons.
    pub checked: Option<bool>,
    /// Present only for sliders.
    pub range: Option<RangeInfo>,
    pub focused: bool,
}

/// Commands emitted at the persistence boundary. `Choose*` commands request an OS chooser; after
/// a chooser succeeds, call [`PreferencesState::add_library_root`] or
/// [`PreferencesState::set_skin_path`] to obtain the corresponding persisted-field command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    SetShuffleMorphRate(u8),
    SetVisualizationMode(VisualizationMode),
    SetVisualizationShowPeaks(bool),
    SetAnalyzerStyle(AnalyzerStyle),
    SetBandWidth(BandWidth),
    SetOscilloscopeStyle(OscStyle),
    SetBarFalloff(u8),
    SetPeakFalloff(u8),
    SetRefreshRate(u8),
    SetDisplayTime(TimeDisplay),
    SetDisplayDoubleSize(bool),
    SetDisplayScrollTitle(bool),
    SetDisplayClutterbar(bool),
    SetDisplayPlaylistNumbers(bool),
    SetSnapPx(u8),
    SetReadTitlesOnLoad(bool),
    SetSortOnLoad(bool),
    SetManualAdvance(bool),
    SetConvertUnderscores(bool),
    SetConvertPercent20(bool),
    ChooseLibraryDirectory,
    SetLibraryRoots(Vec<PathBuf>),
    SetLibraryRecurse(bool),
    ChooseSkinFile,
    SetSkinPath(Option<PathBuf>),
}

/// Result of an input or chooser-completion event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Unchanged,
    Redraw,
    Command(Command),
    Close,
}

/// Keyboard vocabulary required by the platform adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Escape,
    Tab,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Space,
    Enter,
}

/// Complete preferences interaction state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreferencesState {
    pub model: PreferencesModel,
    section: Section,
    focus: ControlId,
    pressed: Option<ControlId>,
    library_selected: Option<usize>,
    library_scroll: usize,
}

impl Default for PreferencesState {
    fn default() -> Self {
        Self::new(PreferencesModel::default())
    }
}

impl PreferencesState {
    pub fn new(mut model: PreferencesModel) -> Self {
        model.shuffle_morph_rate = model
            .shuffle_morph_rate
            .clamp(SHUFFLE_MORPH_RATE_MIN, SHUFFLE_MORPH_RATE_MAX);
        // A hand-built caller model can contain duplicate roots. Persisting duplicate scan work is
        // never useful, so canonicalize while preserving the first-seen order.
        let mut seen = HashSet::new();
        model
            .library_roots
            .retain(|path| !path.as_os_str().is_empty() && seen.insert(path.clone()));
        Self {
            model,
            section: Section::Shuffle,
            focus: ControlId::Section(Section::Shuffle),
            pressed: None,
            library_selected: None,
            library_scroll: 0,
        }
    }

    pub fn section(&self) -> Section {
        self.section
    }

    pub fn focused_control(&self) -> ControlId {
        self.focus
    }

    pub fn library_selection(&self) -> Option<usize> {
        self.library_selected
    }

    /// Select a page directly, for example when the main menu opens a focused settings page.
    pub fn set_section(&mut self, section: Section) {
        self.section = section;
        self.focus = ControlId::Section(section);
        self.pressed = None;
        self.normalize_library(PREFERENCES_H);
    }

    /// Clamp a caller-provided list selection. `None` clears selection.
    pub fn set_library_selection(&mut self, selection: Option<usize>, height: i32) {
        self.library_selected =
            selection.map(|index| index.min(self.model.library_roots.len().saturating_sub(1)));
        if self.model.library_roots.is_empty() {
            self.library_selected = None;
        }
        self.normalize_library(height.max(PREFERENCES_H));
    }

    /// Accept a directory returned by the platform chooser. Empty and duplicate paths are ignored.
    pub fn add_library_root(&mut self, path: PathBuf, height: i32) -> Outcome {
        if path.as_os_str().is_empty() || self.model.library_roots.contains(&path) {
            return Outcome::Unchanged;
        }
        self.model.library_roots.push(path);
        self.library_selected = Some(self.model.library_roots.len() - 1);
        self.focus = ControlId::LibraryRoot(self.model.library_roots.len() - 1);
        self.normalize_library(height.max(PREFERENCES_H));
        Outcome::Command(Command::SetLibraryRoots(self.model.library_roots.clone()))
    }

    /// Accept a skin file returned by the platform chooser. Empty paths are ignored.
    pub fn set_skin_path(&mut self, path: PathBuf) -> Outcome {
        if path.as_os_str().is_empty() || self.model.skin_path.as_ref() == Some(&path) {
            return Outcome::Unchanged;
        }
        self.model.skin_path = Some(path.clone());
        Outcome::Command(Command::SetSkinPath(Some(path)))
    }

    /// Visible controls in deterministic reading and hit-test order.
    pub fn controls(&self, width: i32, height: i32) -> Vec<ControlInfo> {
        let (width, height) = clamped_size(width, height);
        let mut controls = Vec::new();
        for (index, section) in Section::ALL.into_iter().enumerate() {
            controls.push(ControlInfo {
                id: ControlId::Section(section),
                role: ControlRole::Tab,
                label: section.label().into(),
                rect: Rect {
                    x: PAD,
                    y: PREFERENCES_TITLE_H + PAD + index as i32 * SECTION_H,
                    width: SIDEBAR_W,
                    height: SECTION_H,
                },
                enabled: true,
                selected: section == self.section,
                checked: None,
                range: None,
                focused: self.focus == ControlId::Section(section),
            });
        }

        let content = content_rect(width, height);
        let x = content.x + 18;
        let control_width = (content.width - 36).max(1);
        let check = |id, label: &str, y, checked, role| ControlInfo {
            id,
            role,
            label: label.into(),
            rect: Rect {
                x,
                y,
                width: control_width,
                height: CONTROL_H,
            },
            enabled: true,
            selected: false,
            checked: Some(checked),
            range: None,
            focused: self.focus == id,
        };
        let slider = |id, label: &str, y, height, value| ControlInfo {
            id,
            role: ControlRole::Slider,
            label: label.into(),
            rect: Rect {
                x,
                y,
                width: control_width,
                height,
            },
            enabled: true,
            selected: false,
            checked: None,
            range: slider_bounds(id).map(|(minimum, maximum)| RangeInfo {
                value,
                minimum,
                maximum,
                step: 1,
            }),
            focused: self.focus == id,
        };

        match self.section {
            Section::Shuffle => {
                controls.push(ControlInfo {
                    id: ControlId::ShuffleMorphRate,
                    role: ControlRole::Slider,
                    label: "Shuffle morph rate".into(),
                    rect: Rect {
                        x,
                        y: content.y + 52,
                        width: control_width,
                        height: 52,
                    },
                    enabled: true,
                    selected: false,
                    checked: None,
                    range: Some(RangeInfo {
                        value: self.model.shuffle_morph_rate,
                        minimum: SHUFFLE_MORPH_RATE_MIN,
                        maximum: SHUFFLE_MORPH_RATE_MAX,
                        step: 1,
                    }),
                    focused: self.focus == ControlId::ShuffleMorphRate,
                });
            }
            Section::Visualization => {
                controls.push(check(
                    ControlId::VisualizationSpectrum,
                    "Spectrum analyzer",
                    content.y + 50,
                    self.model.visualization_mode == VisualizationMode::Spectrum,
                    ControlRole::RadioButton,
                ));
                controls.push(check(
                    ControlId::VisualizationOscilloscope,
                    "Oscilloscope",
                    content.y + 76,
                    self.model.visualization_mode == VisualizationMode::Oscilloscope,
                    ControlRole::RadioButton,
                ));
                controls.push(check(
                    ControlId::VisualizationOff,
                    "Off",
                    content.y + 102,
                    self.model.visualization_mode == VisualizationMode::Off,
                    ControlRole::RadioButton,
                ));
                let y0 = content.y + 126;
                controls.push(slider(
                    ControlId::VisualizationRefreshRate,
                    "Refresh rate",
                    y0,
                    36,
                    self.model.visualization_refresh_rate,
                ));
                controls.push(check(
                    ControlId::VisualizationAnalyzerNormal,
                    "Normal analyzer",
                    y0 + 42,
                    self.model.visualization_analyzer_style == AnalyzerStyle::Normal,
                    ControlRole::RadioButton,
                ));
                controls.push(check(
                    ControlId::VisualizationAnalyzerFire,
                    "Fire analyzer",
                    y0 + 66,
                    self.model.visualization_analyzer_style == AnalyzerStyle::Fire,
                    ControlRole::RadioButton,
                ));
                controls.push(check(
                    ControlId::VisualizationAnalyzerLine,
                    "Line analyzer",
                    y0 + 90,
                    self.model.visualization_analyzer_style == AnalyzerStyle::Line,
                    ControlRole::RadioButton,
                ));
                controls.push(check(
                    ControlId::VisualizationShowPeaks,
                    "Show spectrum peaks",
                    y0 + 114,
                    self.model.visualization_show_peaks,
                    ControlRole::CheckBox,
                ));
                controls.push(check(
                    ControlId::VisualizationBandThick,
                    "Thick bands",
                    y0 + 138,
                    self.model.visualization_band_width == BandWidth::Thick,
                    ControlRole::RadioButton,
                ));
                controls.push(check(
                    ControlId::VisualizationBandThin,
                    "Thin bands",
                    y0 + 162,
                    self.model.visualization_band_width == BandWidth::Thin,
                    ControlRole::RadioButton,
                ));
                controls.push(slider(
                    ControlId::VisualizationBarFalloff,
                    "Analyzer falloff",
                    y0 + 192,
                    36,
                    self.model.visualization_bar_falloff,
                ));
                controls.push(slider(
                    ControlId::VisualizationPeakFalloff,
                    "Peaks falloff",
                    y0 + 234,
                    36,
                    self.model.visualization_peak_falloff,
                ));
                controls.push(check(
                    ControlId::VisualizationOscDots,
                    "Dot scope",
                    y0 + 280,
                    self.model.visualization_osc_style == OscStyle::Dots,
                    ControlRole::RadioButton,
                ));
                controls.push(check(
                    ControlId::VisualizationOscLines,
                    "Line scope",
                    y0 + 304,
                    self.model.visualization_osc_style == OscStyle::Lines,
                    ControlRole::RadioButton,
                ));
                controls.push(check(
                    ControlId::VisualizationOscSolid,
                    "Solid scope",
                    y0 + 328,
                    self.model.visualization_osc_style == OscStyle::Solid,
                    ControlRole::RadioButton,
                ));
            }
            Section::Options => {
                controls.push(check(
                    ControlId::OptionsReadOnLoad,
                    "Read titles on load",
                    content.y + 50,
                    self.model.read_titles_on_load,
                    ControlRole::RadioButton,
                ));
                controls.push(check(
                    ControlId::OptionsReadOnPlay,
                    "Read titles on play",
                    content.y + 74,
                    !self.model.read_titles_on_load,
                    ControlRole::RadioButton,
                ));
                controls.push(check(
                    ControlId::OptionsSortOnLoad,
                    "Sort files on load",
                    content.y + 118,
                    self.model.sort_on_load,
                    ControlRole::CheckBox,
                ));
                controls.push(check(
                    ControlId::OptionsManualAdvance,
                    "Manual playlist advance (no automatic)",
                    content.y + 142,
                    self.model.manual_advance,
                    ControlRole::CheckBox,
                ));
                controls.push(check(
                    ControlId::OptionsConvertUnderscores,
                    "Convert underscores to spaces in titles",
                    content.y + 166,
                    self.model.convert_underscores,
                    ControlRole::CheckBox,
                ));
                controls.push(check(
                    ControlId::OptionsConvertPercent20,
                    "Convert %20 to spaces in titles",
                    content.y + 190,
                    self.model.convert_percent20,
                    ControlRole::CheckBox,
                ));
            }
            Section::Display => {
                controls.push(check(
                    ControlId::DisplayElapsed,
                    "Elapsed time",
                    content.y + 50,
                    self.model.display_time == TimeDisplay::Elapsed,
                    ControlRole::RadioButton,
                ));
                controls.push(check(
                    ControlId::DisplayRemaining,
                    "Remaining time",
                    content.y + 76,
                    self.model.display_time == TimeDisplay::Remaining,
                    ControlRole::RadioButton,
                ));
                controls.push(check(
                    ControlId::DisplayDoubleSize,
                    "Double size",
                    content.y + 120,
                    self.model.display_double_size,
                    ControlRole::CheckBox,
                ));
                controls.push(check(
                    ControlId::DisplayScrollTitle,
                    "Scroll track title",
                    content.y + 146,
                    self.model.display_scroll_title,
                    ControlRole::CheckBox,
                ));
                controls.push(check(
                    ControlId::DisplayClutterbar,
                    "Always show clutterbar",
                    content.y + 172,
                    self.model.display_clutterbar,
                    ControlRole::CheckBox,
                ));
                controls.push(check(
                    ControlId::DisplayPlaylistNumbers,
                    "Show numbers in playlist",
                    content.y + 198,
                    self.model.display_playlist_numbers,
                    ControlRole::CheckBox,
                ));
                controls.push(slider(
                    ControlId::DisplaySnapPx,
                    "Snap windows at (pixels)",
                    content.y + 230,
                    36,
                    self.model.display_snap_px,
                ));
            }
            Section::AudioLibrary => {
                let list = library_list_rect(content);
                let visible = visible_root_rows(list);
                let max_scroll = self.model.library_roots.len().saturating_sub(visible);
                let scroll = self.library_scroll.min(max_scroll);
                for (screen_row, index) in (scroll..self.model.library_roots.len())
                    .take(visible)
                    .enumerate()
                {
                    let path = self.model.library_roots[index]
                        .to_string_lossy()
                        .into_owned();
                    let id = ControlId::LibraryRoot(index);
                    controls.push(ControlInfo {
                        id,
                        role: ControlRole::ListItem,
                        label: path,
                        rect: Rect {
                            x: list.x + 2,
                            y: list.y + 2 + screen_row as i32 * ROOT_ROW_H,
                            width: list.width - 4,
                            height: ROOT_ROW_H,
                        },
                        enabled: true,
                        selected: self.library_selected == Some(index),
                        checked: None,
                        range: None,
                        focused: self.focus == id,
                    });
                }
                let button_y = list.y + list.height + 9;
                controls.push(button_info(
                    ControlId::LibraryAdd,
                    "Add Directory...",
                    Rect {
                        x,
                        y: button_y,
                        width: BUTTON_W,
                        height: CONTROL_H,
                    },
                    true,
                    self.focus,
                ));
                controls.push(button_info(
                    ControlId::LibraryRemove,
                    "Remove",
                    Rect {
                        x: x + BUTTON_W + 10,
                        y: button_y,
                        width: 82,
                        height: CONTROL_H,
                    },
                    self.library_selected.is_some(),
                    self.focus,
                ));
                controls.push(check(
                    ControlId::LibraryRecurse,
                    "Include subdirectories",
                    button_y + 32,
                    self.model.library_recurse,
                    ControlRole::CheckBox,
                ));
            }
            Section::Skins => {
                controls.push(button_info(
                    ControlId::SkinLoad,
                    "Load Skin...",
                    Rect {
                        x,
                        y: content.y + 92,
                        width: BUTTON_W,
                        height: CONTROL_H,
                    },
                    true,
                    self.focus,
                ));
                controls.push(button_info(
                    ControlId::SkinBase,
                    "Use Base Skin",
                    Rect {
                        x,
                        y: content.y + 124,
                        width: BUTTON_W,
                        height: CONTROL_H,
                    },
                    true,
                    self.focus,
                ));
            }
        }

        controls.push(button_info(
            ControlId::Close,
            "Close",
            close_rect(width, height),
            true,
            self.focus,
        ));
        controls
    }

    pub fn pointer_press(&mut self, x: i32, y: i32, width: i32, height: i32) -> Outcome {
        self.normalize_library(height.max(PREFERENCES_H));
        let Some(control) = self.hit_test(x, y, width, height) else {
            self.pressed = None;
            return Outcome::Unchanged;
        };
        if !control.enabled {
            self.pressed = None;
            return Outcome::Unchanged;
        }
        self.focus = control.id;
        self.pressed = Some(control.id);
        if let Some(bounds) = slider_bounds(control.id) {
            let value = slider_value_at_x(control.rect, x, bounds);
            self.set_slider_value(control.id, value)
        } else {
            Outcome::Redraw
        }
    }

    /// Update a pressed slider while the pointer is dragged. The value clamps at either end even
    /// when the pointer moves beyond the visible track.
    pub fn pointer_motion(&mut self, x: i32, width: i32, height: i32) -> Outcome {
        let Some((pressed, bounds)) = self
            .pressed
            .and_then(|id| slider_bounds(id).map(|bounds| (id, bounds)))
        else {
            return Outcome::Unchanged;
        };
        let Some(control) = self
            .controls(width, height)
            .into_iter()
            .find(|control| control.id == pressed)
        else {
            return Outcome::Unchanged;
        };
        self.set_slider_value(pressed, slider_value_at_x(control.rect, x, bounds))
    }

    pub fn pointer_release(&mut self, x: i32, y: i32, width: i32, height: i32) -> Outcome {
        let pressed = self.pressed.take();
        if let Some((id, bounds)) = pressed.and_then(|id| slider_bounds(id).map(|b| (id, b))) {
            let Some(control) = self
                .controls(width, height)
                .into_iter()
                .find(|control| control.id == id)
            else {
                return Outcome::Redraw;
            };
            return self.set_slider_value(id, slider_value_at_x(control.rect, x, bounds));
        }
        let released = self
            .hit_test(x, y, width, height)
            .filter(|control| control.enabled)
            .map(|control| control.id);
        match (pressed, released) {
            (Some(expected), Some(actual)) if expected == actual => {
                self.activate(actual, height.max(PREFERENCES_H))
            }
            (Some(_), _) => Outcome::Redraw,
            _ => Outcome::Unchanged,
        }
    }

    pub fn pointer_leave(&mut self) -> Outcome {
        if self.pressed.take().is_some() {
            Outcome::Redraw
        } else {
            Outcome::Unchanged
        }
    }

    pub fn key(&mut self, key: Key, width: i32, height: i32) -> Outcome {
        let height = height.max(PREFERENCES_H);
        self.normalize_library(height);
        match key {
            Key::Escape => Outcome::Close,
            Key::Tab => self.move_tab(1, width, height),
            Key::BackTab => self.move_tab(-1, width, height),
            Key::Space | Key::Enter => self.activate(self.focus, height),
            Key::Left | Key::Down if slider_bounds(self.focus).is_some() => {
                self.adjust_slider(self.focus, -1)
            }
            Key::Right | Key::Up if slider_bounds(self.focus).is_some() => {
                self.adjust_slider(self.focus, 1)
            }
            Key::Home if slider_bounds(self.focus).is_some() => {
                let (minimum, _) = slider_bounds(self.focus).unwrap();
                self.set_slider_value(self.focus, minimum)
            }
            Key::End if slider_bounds(self.focus).is_some() => {
                let (_, maximum) = slider_bounds(self.focus).unwrap();
                self.set_slider_value(self.focus, maximum)
            }
            Key::Up | Key::Down if matches!(self.focus, ControlId::Section(_)) => {
                let delta = if key == Key::Up { -1 } else { 1 };
                let next = self.section.offset(delta);
                self.set_section(next);
                Outcome::Redraw
            }
            Key::Home | Key::End if matches!(self.focus, ControlId::Section(_)) => {
                let next = if key == Key::Home {
                    Section::ALL[0]
                } else {
                    Section::ALL[Section::ALL.len() - 1]
                };
                self.set_section(next);
                Outcome::Redraw
            }
            Key::Up | Key::Down if matches!(self.focus, ControlId::LibraryRoot(_)) => {
                self.move_library_selection(if key == Key::Up { -1 } else { 1 }, height)
            }
            Key::Home | Key::End if matches!(self.focus, ControlId::LibraryRoot(_)) => {
                if self.model.library_roots.is_empty() {
                    return Outcome::Unchanged;
                }
                let index = if key == Key::Home {
                    0
                } else {
                    self.model.library_roots.len() - 1
                };
                self.library_selected = Some(index);
                self.focus = ControlId::LibraryRoot(index);
                self.normalize_library(height);
                Outcome::Redraw
            }
            Key::Left | Key::Right => self.move_radio(key == Key::Right),
            Key::Up => self.move_tab(-1, width, height),
            Key::Down => self.move_tab(1, width, height),
            Key::Home => self.focus_edge(false, width, height),
            Key::End => self.focus_edge(true, width, height),
        }
    }

    fn hit_test(&self, x: i32, y: i32, width: i32, height: i32) -> Option<ControlInfo> {
        self.controls(width, height)
            .into_iter()
            .rev()
            .find(|control| control.rect.contains(x, y))
    }

    fn activate(&mut self, id: ControlId, height: i32) -> Outcome {
        match id {
            ControlId::Section(section) => {
                self.set_section(section);
                Outcome::Redraw
            }
            ControlId::ShuffleMorphRate => Outcome::Redraw,
            ControlId::VisualizationSpectrum => {
                self.set_visualization_mode(VisualizationMode::Spectrum)
            }
            ControlId::VisualizationOscilloscope => {
                self.set_visualization_mode(VisualizationMode::Oscilloscope)
            }
            ControlId::VisualizationOff => self.set_visualization_mode(VisualizationMode::Off),
            ControlId::VisualizationShowPeaks => {
                self.model.visualization_show_peaks = !self.model.visualization_show_peaks;
                Outcome::Command(Command::SetVisualizationShowPeaks(
                    self.model.visualization_show_peaks,
                ))
            }
            ControlId::VisualizationAnalyzerNormal => self.set_analyzer_style(AnalyzerStyle::Normal),
            ControlId::VisualizationAnalyzerFire => self.set_analyzer_style(AnalyzerStyle::Fire),
            ControlId::VisualizationAnalyzerLine => self.set_analyzer_style(AnalyzerStyle::Line),
            ControlId::VisualizationBandThick => self.set_band_width(BandWidth::Thick),
            ControlId::VisualizationBandThin => self.set_band_width(BandWidth::Thin),
            ControlId::VisualizationOscDots => self.set_osc_style(OscStyle::Dots),
            ControlId::VisualizationOscLines => self.set_osc_style(OscStyle::Lines),
            ControlId::VisualizationOscSolid => self.set_osc_style(OscStyle::Solid),
            ControlId::VisualizationRefreshRate
            | ControlId::VisualizationBarFalloff
            | ControlId::VisualizationPeakFalloff
            | ControlId::DisplaySnapPx => Outcome::Redraw,
            ControlId::DisplayClutterbar => {
                self.model.display_clutterbar = !self.model.display_clutterbar;
                Outcome::Command(Command::SetDisplayClutterbar(self.model.display_clutterbar))
            }
            ControlId::DisplayPlaylistNumbers => {
                self.model.display_playlist_numbers = !self.model.display_playlist_numbers;
                Outcome::Command(Command::SetDisplayPlaylistNumbers(
                    self.model.display_playlist_numbers,
                ))
            }
            ControlId::OptionsReadOnLoad => self.set_read_titles_on_load(true),
            ControlId::OptionsReadOnPlay => self.set_read_titles_on_load(false),
            ControlId::OptionsSortOnLoad => {
                self.model.sort_on_load = !self.model.sort_on_load;
                Outcome::Command(Command::SetSortOnLoad(self.model.sort_on_load))
            }
            ControlId::OptionsManualAdvance => {
                self.model.manual_advance = !self.model.manual_advance;
                Outcome::Command(Command::SetManualAdvance(self.model.manual_advance))
            }
            ControlId::OptionsConvertUnderscores => {
                self.model.convert_underscores = !self.model.convert_underscores;
                Outcome::Command(Command::SetConvertUnderscores(
                    self.model.convert_underscores,
                ))
            }
            ControlId::OptionsConvertPercent20 => {
                self.model.convert_percent20 = !self.model.convert_percent20;
                Outcome::Command(Command::SetConvertPercent20(self.model.convert_percent20))
            }
            ControlId::DisplayElapsed => self.set_display_time(TimeDisplay::Elapsed),
            ControlId::DisplayRemaining => self.set_display_time(TimeDisplay::Remaining),
            ControlId::DisplayDoubleSize => {
                self.model.display_double_size = !self.model.display_double_size;
                Outcome::Command(Command::SetDisplayDoubleSize(
                    self.model.display_double_size,
                ))
            }
            ControlId::DisplayScrollTitle => {
                self.model.display_scroll_title = !self.model.display_scroll_title;
                Outcome::Command(Command::SetDisplayScrollTitle(
                    self.model.display_scroll_title,
                ))
            }
            ControlId::LibraryRoot(index) => {
                if index >= self.model.library_roots.len() {
                    return Outcome::Unchanged;
                }
                self.library_selected = Some(index);
                self.focus = ControlId::LibraryRoot(index);
                self.normalize_library(height);
                Outcome::Redraw
            }
            ControlId::LibraryAdd => Outcome::Command(Command::ChooseLibraryDirectory),
            ControlId::LibraryRemove => self.remove_selected_library_root(height),
            ControlId::LibraryRecurse => {
                self.model.library_recurse = !self.model.library_recurse;
                Outcome::Command(Command::SetLibraryRecurse(self.model.library_recurse))
            }
            ControlId::SkinLoad => Outcome::Command(Command::ChooseSkinFile),
            ControlId::SkinBase => {
                if self.model.skin_path.take().is_some() {
                    Outcome::Command(Command::SetSkinPath(None))
                } else {
                    Outcome::Redraw
                }
            }
            ControlId::Close => Outcome::Close,
        }
    }

    fn set_visualization_mode(&mut self, mode: VisualizationMode) -> Outcome {
        if self.model.visualization_mode == mode {
            Outcome::Redraw
        } else {
            self.model.visualization_mode = mode;
            Outcome::Command(Command::SetVisualizationMode(mode))
        }
    }

    fn set_analyzer_style(&mut self, style: AnalyzerStyle) -> Outcome {
        if self.model.visualization_analyzer_style == style {
            Outcome::Redraw
        } else {
            self.model.visualization_analyzer_style = style;
            Outcome::Command(Command::SetAnalyzerStyle(style))
        }
    }

    fn set_band_width(&mut self, width: BandWidth) -> Outcome {
        if self.model.visualization_band_width == width {
            Outcome::Redraw
        } else {
            self.model.visualization_band_width = width;
            Outcome::Command(Command::SetBandWidth(width))
        }
    }

    fn set_osc_style(&mut self, style: OscStyle) -> Outcome {
        if self.model.visualization_osc_style == style {
            Outcome::Redraw
        } else {
            self.model.visualization_osc_style = style;
            Outcome::Command(Command::SetOscilloscopeStyle(style))
        }
    }

    fn set_read_titles_on_load(&mut self, on_load: bool) -> Outcome {
        if self.model.read_titles_on_load == on_load {
            Outcome::Redraw
        } else {
            self.model.read_titles_on_load = on_load;
            Outcome::Command(Command::SetReadTitlesOnLoad(on_load))
        }
    }

    /// The current value of a range control.
    fn slider_value(&self, id: ControlId) -> u8 {
        match id {
            ControlId::ShuffleMorphRate => self.model.shuffle_morph_rate,
            ControlId::VisualizationRefreshRate => self.model.visualization_refresh_rate,
            ControlId::VisualizationBarFalloff => self.model.visualization_bar_falloff,
            ControlId::VisualizationPeakFalloff => self.model.visualization_peak_falloff,
            ControlId::DisplaySnapPx => self.model.display_snap_px,
            _ => 0,
        }
    }

    /// Set a range control's value, emitting its command on a real change.
    fn set_slider_value(&mut self, id: ControlId, value: u8) -> Outcome {
        let Some((minimum, maximum)) = slider_bounds(id) else {
            return Outcome::Unchanged;
        };
        let value = value.clamp(minimum, maximum);
        if self.slider_value(id) == value {
            return Outcome::Redraw;
        }
        match id {
            ControlId::ShuffleMorphRate => {
                self.model.shuffle_morph_rate = value;
                Outcome::Command(Command::SetShuffleMorphRate(value))
            }
            ControlId::VisualizationRefreshRate => {
                self.model.visualization_refresh_rate = value;
                Outcome::Command(Command::SetRefreshRate(value))
            }
            ControlId::VisualizationBarFalloff => {
                self.model.visualization_bar_falloff = value;
                Outcome::Command(Command::SetBarFalloff(value))
            }
            ControlId::VisualizationPeakFalloff => {
                self.model.visualization_peak_falloff = value;
                Outcome::Command(Command::SetPeakFalloff(value))
            }
            ControlId::DisplaySnapPx => {
                self.model.display_snap_px = value;
                Outcome::Command(Command::SetSnapPx(value))
            }
            _ => Outcome::Unchanged,
        }
    }

    fn adjust_slider(&mut self, id: ControlId, delta: i8) -> Outcome {
        let value = (i16::from(self.slider_value(id)) + i16::from(delta)).max(0) as u8;
        self.set_slider_value(id, value)
    }


    fn set_display_time(&mut self, time: TimeDisplay) -> Outcome {
        if self.model.display_time == time {
            Outcome::Redraw
        } else {
            self.model.display_time = time;
            Outcome::Command(Command::SetDisplayTime(time))
        }
    }

    fn remove_selected_library_root(&mut self, height: i32) -> Outcome {
        let Some(index) = self.library_selected else {
            return Outcome::Unchanged;
        };
        if index >= self.model.library_roots.len() {
            self.library_selected = None;
            return Outcome::Unchanged;
        }
        self.model.library_roots.remove(index);
        if self.model.library_roots.is_empty() {
            self.library_selected = None;
            self.focus = ControlId::LibraryAdd;
        } else {
            let selected = index.min(self.model.library_roots.len() - 1);
            self.library_selected = Some(selected);
            self.focus = ControlId::LibraryRoot(selected);
        }
        self.normalize_library(height);
        Outcome::Command(Command::SetLibraryRoots(self.model.library_roots.clone()))
    }

    fn move_library_selection(&mut self, delta: i32, height: i32) -> Outcome {
        if self.model.library_roots.is_empty() {
            return Outcome::Unchanged;
        }
        let current = match self.focus {
            ControlId::LibraryRoot(index) => index,
            _ => self.library_selected.unwrap_or(0),
        };
        let last = self.model.library_roots.len() as i32 - 1;
        let next = (current as i32 + delta).clamp(0, last) as usize;
        self.library_selected = Some(next);
        self.focus = ControlId::LibraryRoot(next);
        self.normalize_library(height);
        Outcome::Redraw
    }

    fn move_radio(&mut self, forward: bool) -> Outcome {
        let delta = if forward { 1_i32 } else { -1_i32 };
        match self.focus {
            ControlId::VisualizationSpectrum
            | ControlId::VisualizationOscilloscope
            | ControlId::VisualizationOff => {
                let modes = [
                    VisualizationMode::Spectrum,
                    VisualizationMode::Oscilloscope,
                    VisualizationMode::Off,
                ];
                let current = modes
                    .iter()
                    .position(|&mode| mode == self.model.visualization_mode)
                    .unwrap_or(0) as i32;
                let index = (current + delta).rem_euclid(modes.len() as i32) as usize;
                let id = [
                    ControlId::VisualizationSpectrum,
                    ControlId::VisualizationOscilloscope,
                    ControlId::VisualizationOff,
                ][index];
                self.focus = id;
                self.set_visualization_mode(modes[index])
            }
            ControlId::DisplayElapsed | ControlId::DisplayRemaining => {
                let time = if self.model.display_time == TimeDisplay::Elapsed {
                    TimeDisplay::Remaining
                } else {
                    TimeDisplay::Elapsed
                };
                self.focus = if time == TimeDisplay::Elapsed {
                    ControlId::DisplayElapsed
                } else {
                    ControlId::DisplayRemaining
                };
                self.set_display_time(time)
            }
            ControlId::Section(section) if forward => {
                let next = section.offset(1);
                self.set_section(next);
                Outcome::Redraw
            }
            ControlId::Section(section) => {
                let next = section.offset(-1);
                self.set_section(next);
                Outcome::Redraw
            }
            _ => Outcome::Unchanged,
        }
    }

    fn move_tab(&mut self, delta: i32, width: i32, height: i32) -> Outcome {
        let order = self.focus_order(width, height);
        if order.is_empty() {
            return Outcome::Unchanged;
        }
        let current = order.iter().position(|&id| id == self.focus).unwrap_or(0) as i32;
        let next = (current + delta).rem_euclid(order.len() as i32) as usize;
        self.focus = order[next];
        if let ControlId::LibraryRoot(index) = self.focus {
            self.library_selected = Some(index);
            self.normalize_library(height);
        }
        Outcome::Redraw
    }

    fn focus_edge(&mut self, end: bool, width: i32, height: i32) -> Outcome {
        let order = self.focus_order(width, height);
        let Some(&id) = (if end { order.last() } else { order.first() }) else {
            return Outcome::Unchanged;
        };
        self.focus = id;
        Outcome::Redraw
    }

    fn focus_order(&self, width: i32, height: i32) -> Vec<ControlId> {
        let selected_tab = ControlId::Section(self.section);
        let mut order = vec![selected_tab];
        order.extend(
            self.controls(width, height)
                .into_iter()
                .filter(|control| {
                    control.enabled
                        && !matches!(control.id, ControlId::Section(_) | ControlId::Close)
                })
                .map(|control| control.id),
        );
        order.push(ControlId::Close);
        order
    }

    fn normalize_library(&mut self, height: i32) {
        if self.model.library_roots.is_empty() {
            self.library_selected = None;
            self.library_scroll = 0;
            if matches!(self.focus, ControlId::LibraryRoot(_)) {
                self.focus = ControlId::LibraryAdd;
            }
            return;
        }
        if let Some(selected) = self.library_selected {
            let selected = selected.min(self.model.library_roots.len() - 1);
            self.library_selected = Some(selected);
            if matches!(self.focus, ControlId::LibraryRoot(_)) {
                self.focus = ControlId::LibraryRoot(selected);
            }
            let content = content_rect(PREFERENCES_W, height);
            let visible = visible_root_rows(library_list_rect(content)).max(1);
            if selected < self.library_scroll {
                self.library_scroll = selected;
            } else if selected >= self.library_scroll + visible {
                self.library_scroll = selected + 1 - visible;
            }
        }
        let content = content_rect(PREFERENCES_W, height);
        let visible = visible_root_rows(library_list_rect(content));
        let max_scroll = self.model.library_roots.len().saturating_sub(visible);
        self.library_scroll = self.library_scroll.min(max_scroll);
    }
}

/// Compose an opaque preferences window. Requests below the supported minimum are clamped.
/// Rendering theme for the preferences window. `classic()` reproduces the original OS-neutral
/// bitmap chrome (used when no system UI font is available, e.g. headless tests); `adwaita(...)`
/// paints a native GNOME (libadwaita) look in the given palette with the system UI font. The control
/// GEOMETRY is identical either way, so hit-testing, which reads the same layout, is unaffected.
pub struct PrefsTheme<'a> {
    palette: Palette,
    font: Option<&'a UiFont>,
}

impl Default for PrefsTheme<'_> {
    fn default() -> Self {
        Self::classic()
    }
}

impl<'a> PrefsTheme<'a> {
    pub fn classic() -> Self {
        Self {
            palette: Palette::light(),
            font: None,
        }
    }

    pub fn adwaita(palette: Palette, font: &'a UiFont) -> Self {
        Self {
            palette,
            font: Some(font),
        }
    }
}

/// Compose the preferences window. With a system font it renders the native Adwaita look; otherwise
/// it falls back to the classic bitmap chrome.
pub fn compose(
    state: &PreferencesState,
    width: i32,
    height: i32,
    theme: &PrefsTheme,
) -> Framebuffer {
    match theme.font {
        Some(font) => compose_adwaita(state, width, height, &theme.palette, font),
        None => compose_classic(state, width, height),
    }
}

const BODY_PX: f32 = 13.0;
const TITLE_PX: f32 = 14.0;

/// Draw system-font text placing `top_y` as the top of the glyph box (matching the classic bitmap
/// coordinates) by advancing to an approximate baseline.
fn atext(fb: &mut Framebuffer, font: &UiFont, x: i32, top_y: i32, s: &str, px: f32, color: [u8; 4]) {
    let baseline = top_y + (px * 0.78).round() as i32;
    font.draw_text(fb, x, baseline, s, px, color);
}

/// Draw system-font text truncated with an ellipsis to fit `max_w`.
#[allow(clippy::too_many_arguments)]
fn atext_clipped(
    fb: &mut Framebuffer,
    font: &UiFont,
    x: i32,
    top_y: i32,
    s: &str,
    max_w: i32,
    px: f32,
    color: [u8; 4],
) {
    if font.text_width(s, px) as i32 <= max_w {
        atext(fb, font, x, top_y, s, px, color);
        return;
    }
    let mut chars: Vec<char> = s.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "\u{2026}";
        if font.text_width(&candidate, px) as i32 <= max_w {
            atext(fb, font, x, top_y, &candidate, px, color);
            return;
        }
    }
}

/// A rounded native card (list or field container): view background plus a hairline border.
fn acard(fb: &mut Framebuffer, rect: Rect, p: &Palette) {
    adwaita::fill_rounded_rect(fb, rect.x, rect.y, rect.width, rect.height, 10, p.view_bg);
    adwaita::stroke_rounded_rect(fb, rect.x, rect.y, rect.width, rect.height, 10, 1, p.border);
}

fn compose_adwaita(
    state: &PreferencesState,
    width: i32,
    height: i32,
    p: &Palette,
    font: &UiFont,
) -> Framebuffer {
    let (width, height) = clamped_size(width, height);
    let mut fb = Framebuffer::new(width as u32, height as u32);
    adwaita::fill_rect(&mut fb, 0, 0, width, height, p.window_bg);

    // Headerbar: title with a hairline separator underneath.
    atext(&mut fb, font, PAD, 5, "Preferences", TITLE_PX, p.fg);
    adwaita::draw_separator(&mut fb, 0, PREFERENCES_TITLE_H - 1, width, p);

    let content = content_rect(width, height);
    atext(&mut fb, font, content.x + 2, content.y + 4, state.section.label(), TITLE_PX, p.fg);

    let controls = state.controls(width, height);
    for control in controls
        .iter()
        .filter(|control| matches!(control.id, ControlId::Section(_)))
    {
        draw_sidebar_adwaita(&mut fb, control, p, font, state.pressed == Some(control.id));
    }
    draw_page_static_adwaita(&mut fb, state, content, p, font);
    for control in controls
        .iter()
        .filter(|control| !matches!(control.id, ControlId::Section(_)))
    {
        draw_control_adwaita(&mut fb, control, p, font, state.pressed == Some(control.id));
    }
    fb
}

fn draw_sidebar_adwaita(
    fb: &mut Framebuffer,
    control: &ControlInfo,
    p: &Palette,
    font: &UiFont,
    pressed: bool,
) {
    let r = control.rect;
    if control.selected {
        adwaita::fill_rounded_rect(fb, r.x, r.y, r.width, r.height, 8, p.accent_bg);
    } else if pressed {
        adwaita::fill_rounded_rect(fb, r.x, r.y, r.width, r.height, 8, p.active);
    }
    let color = if control.selected { p.accent_fg } else { p.fg };
    atext(
        fb,
        font,
        r.x + 10,
        r.y + (r.height - BODY_PX as i32) / 2,
        &control.label,
        BODY_PX,
        color,
    );
    if control.focused && !control.selected {
        adwaita::draw_focus_ring(fb, r.x, r.y, r.width, r.height, 8, p);
    }
}

fn draw_page_static_adwaita(
    fb: &mut Framebuffer,
    state: &PreferencesState,
    content: Rect,
    p: &Palette,
    font: &UiFont,
) {
    let dim = p.dim_fg;
    match state.section {
        Section::Shuffle => {
            atext(fb, font, content.x + 18, content.y + 34, "Shuffle morph rate", BODY_PX, dim);
        }
        Section::Visualization => {
            atext(fb, font, content.x + 18, content.y + 32, "Visualization mode", BODY_PX, dim);
        }
        Section::Options => {
            atext(fb, font, content.x + 18, content.y + 32, "Read titles on", BODY_PX, dim);
            atext(fb, font, content.x + 18, content.y + 100, "Playlist behaviour", BODY_PX, dim);
        }
        Section::Display => {
            atext(fb, font, content.x + 18, content.y + 32, "Time display", BODY_PX, dim);
            atext(fb, font, content.x + 18, content.y + 104, "Window display", BODY_PX, dim);
        }
        Section::AudioLibrary => {
            atext(fb, font, content.x + 18, content.y + 32, "Folders scanned for audio files", BODY_PX, dim);
            let list = library_list_rect(content);
            acard(fb, list, p);
            if state.model.library_roots.is_empty() {
                atext(fb, font, list.x + 10, list.y + 8, "No folders added", BODY_PX, dim);
            }
        }
        Section::Skins => {
            atext(fb, font, content.x + 18, content.y + 34, "Current skin", BODY_PX, dim);
            let field = Rect {
                x: content.x + 18,
                y: content.y + 50,
                width: content.width - 36,
                height: 28,
            };
            acard(fb, field, p);
            let current = state.model.skin_path.as_ref().map_or_else(
                || "Base Skin".into(),
                |path| path.to_string_lossy().into_owned(),
            );
            atext_clipped(fb, font, field.x + 10, field.y + 9, &current, field.width - 20, BODY_PX, p.fg);
        }
    }
}

fn draw_control_adwaita(
    fb: &mut Framebuffer,
    control: &ControlInfo,
    p: &Palette,
    font: &UiFont,
    pressed: bool,
) {
    match control.role {
        ControlRole::CheckBox => draw_check_adwaita(fb, control, p, font, false),
        ControlRole::RadioButton => draw_check_adwaita(fb, control, p, font, true),
        ControlRole::Slider => draw_slider_adwaita(fb, control, p, font),
        ControlRole::ListItem => draw_listitem_adwaita(fb, control, p, font),
        ControlRole::Button => draw_button_adwaita(fb, control, p, font, pressed),
        ControlRole::Tab => {}
    }
}

fn draw_check_adwaita(fb: &mut Framebuffer, c: &ControlInfo, p: &Palette, font: &UiFont, radio: bool) {
    let box_rect = Rect {
        x: c.rect.x,
        y: c.rect.y + (c.rect.height - CHECK_SIZE) / 2,
        width: CHECK_SIZE,
        height: CHECK_SIZE,
    };
    let checked = c.checked == Some(true);
    let radius = if radio { CHECK_SIZE / 2 } else { 3 };
    if checked {
        adwaita::fill_rounded_rect(fb, box_rect.x, box_rect.y, CHECK_SIZE, CHECK_SIZE, radius, p.accent_bg);
        if radio {
            let d = 4;
            adwaita::fill_rounded_rect(fb, box_rect.x + (CHECK_SIZE - d) / 2, box_rect.y + (CHECK_SIZE - d) / 2, d, d, d / 2, p.accent_fg);
        } else {
            for (dx, dy) in [(2, 6), (3, 7), (4, 8), (6, 5), (7, 4), (8, 3)] {
                adwaita::fill_rect(fb, box_rect.x + dx, box_rect.y + dy, 2, 2, p.accent_fg);
            }
        }
    } else {
        adwaita::fill_rounded_rect(fb, box_rect.x, box_rect.y, CHECK_SIZE, CHECK_SIZE, radius, p.view_bg);
        adwaita::stroke_rounded_rect(fb, box_rect.x, box_rect.y, CHECK_SIZE, CHECK_SIZE, radius, 1, p.border);
    }
    let color = if c.enabled { p.fg } else { p.dim_fg };
    atext(fb, font, c.rect.x + CHECK_SIZE + 8, c.rect.y + (c.rect.height - BODY_PX as i32) / 2, &c.label, BODY_PX, color);
    if c.focused {
        let label_w = font.text_width(&c.label, BODY_PX) as i32;
        adwaita::draw_focus_ring(fb, c.rect.x - 3, c.rect.y, (CHECK_SIZE + 11 + label_w).min(c.rect.width), c.rect.height, 6, p);
    }
}

fn draw_slider_adwaita(fb: &mut Framebuffer, c: &ControlInfo, p: &Palette, font: &UiFont) {
    let Some(range) = c.range else {
        return;
    };
    let track = Rect {
        x: c.rect.x + SLIDER_THUMB_W / 2,
        y: c.rect.y + 8,
        width: (c.rect.width - SLIDER_THUMB_W + 1).max(1),
        height: 4,
    };
    adwaita::fill_rounded_rect(fb, track.x, track.y, track.width, track.height, 2, p.border);
    let span = (track.width - 1).max(1);
    let value_span = i32::from(range.maximum.saturating_sub(range.minimum)).max(1);
    let offset = i32::from(range.value.saturating_sub(range.minimum)) * span / value_span;
    adwaita::fill_rounded_rect(fb, track.x, track.y, offset.max(1), track.height, 2, p.accent_bg);
    let tsize = 14;
    let tx = track.x + offset - tsize / 2;
    let ty = track.y + track.height / 2 - tsize / 2;
    adwaita::fill_rounded_rect(fb, tx, ty, tsize, tsize, tsize / 2, p.accent_fg);
    adwaita::stroke_rounded_rect(fb, tx, ty, tsize, tsize, tsize / 2, 1, p.border);

    let labels_y = c.rect.y + 27;
    atext(fb, font, c.rect.x, labels_y, "Slow", BODY_PX, p.dim_fg);
    let fast_w = font.text_width("Fast", BODY_PX) as i32;
    atext(fb, font, c.rect.x + c.rect.width - fast_w, labels_y, "Fast", BODY_PX, p.dim_fg);
    let value = range.value.to_string();
    let vw = font.text_width(&value, BODY_PX) as i32;
    atext(fb, font, c.rect.x + (c.rect.width - vw) / 2, labels_y, &value, BODY_PX, p.fg);
    if c.focused {
        adwaita::draw_focus_ring(fb, tx - 2, ty - 2, tsize + 4, tsize + 4, (tsize + 4) / 2, p);
    }
}

fn draw_button_adwaita(fb: &mut Framebuffer, c: &ControlInfo, p: &Palette, font: &UiFont, pressed: bool) {
    let r = c.rect;
    let primary = matches!(c.id, ControlId::Close);
    let (bg, fg) = if primary { (p.accent_bg, p.accent_fg) } else { (p.view_bg, p.fg) };
    adwaita::fill_rounded_rect(fb, r.x, r.y, r.width, r.height, 7, bg);
    if !primary {
        adwaita::stroke_rounded_rect(fb, r.x, r.y, r.width, r.height, 7, 1, p.border);
    }
    if pressed {
        adwaita::fill_rounded_rect(fb, r.x, r.y, r.width, r.height, 7, p.active);
    }
    let color = if c.enabled { fg } else { p.dim_fg };
    let lw = font.text_width(&c.label, BODY_PX) as i32;
    atext(fb, font, r.x + (r.width - lw) / 2, r.y + (r.height - BODY_PX as i32) / 2, &c.label, BODY_PX, color);
    if c.focused {
        adwaita::draw_focus_ring(fb, r.x, r.y, r.width, r.height, 7, p);
    }
}

fn draw_listitem_adwaita(fb: &mut Framebuffer, c: &ControlInfo, p: &Palette, font: &UiFont) {
    let r = c.rect;
    if c.selected {
        adwaita::fill_rounded_rect(fb, r.x, r.y, r.width, r.height, 6, p.accent_bg);
    }
    let color = if c.selected { p.accent_fg } else { p.fg };
    atext_clipped(fb, font, r.x + 6, r.y + (r.height - BODY_PX as i32) / 2, &c.label, r.width - 12, BODY_PX, color);
    if c.focused && !c.selected {
        adwaita::draw_focus_ring(fb, r.x, r.y, r.width, r.height, 6, p);
    }
}

fn compose_classic(state: &PreferencesState, width: i32, height: i32) -> Framebuffer {
    let (width, height) = clamped_size(width, height);
    let mut fb = Framebuffer::new(width as u32, height as u32);
    fill(
        &mut fb,
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
        BG,
    );

    // Title bar.
    fill(
        &mut fb,
        Rect {
            x: 0,
            y: 0,
            width,
            height: PREFERENCES_TITLE_H,
        },
        DARK,
    );
    draw_text(&mut fb, PAD, 8, "XUBAMP PREFERENCES", SELECTED_TEXT);

    let content = content_rect(width, height);
    panel(&mut fb, content);
    draw_text(
        &mut fb,
        content.x + 14,
        content.y + 14,
        state.section.label(),
        TEXT,
    );

    let controls = state.controls(width, height);
    for control in controls
        .iter()
        .filter(|control| matches!(control.id, ControlId::Section(_)))
    {
        draw_sidebar_control(&mut fb, control, state.pressed == Some(control.id));
    }

    draw_page_static(&mut fb, state, content);
    for control in controls
        .iter()
        .filter(|control| !matches!(control.id, ControlId::Section(_)))
    {
        draw_control(&mut fb, control, state.pressed == Some(control.id));
    }
    fb
}

fn draw_page_static(fb: &mut Framebuffer, state: &PreferencesState, content: Rect) {
    match state.section {
        Section::Shuffle => {
            draw_text(
                fb,
                content.x + 18,
                content.y + 34,
                "Shuffle morph rate",
                DIM,
            );
        }
        Section::Visualization => {
            draw_text(
                fb,
                content.x + 18,
                content.y + 32,
                "Visualization mode",
                DIM,
            );
        }
        Section::Options => {
            draw_text(fb, content.x + 18, content.y + 32, "Read titles on", DIM);
            draw_text(
                fb,
                content.x + 18,
                content.y + 100,
                "Playlist behaviour",
                DIM,
            );
        }
        Section::Display => {
            draw_text(fb, content.x + 18, content.y + 32, "Time display", DIM);
            draw_text(fb, content.x + 18, content.y + 104, "Window display", DIM);
        }
        Section::AudioLibrary => {
            draw_text(
                fb,
                content.x + 18,
                content.y + 32,
                "Folders scanned for audio files",
                DIM,
            );
            let list = library_list_rect(content);
            panel_inset(fb, list);
            if state.model.library_roots.is_empty() {
                draw_text(fb, list.x + 8, list.y + 8, "No folders added", DIM);
            }
        }
        Section::Skins => {
            draw_text(fb, content.x + 18, content.y + 34, "Current skin", DIM);
            let field = Rect {
                x: content.x + 18,
                y: content.y + 50,
                width: content.width - 36,
                height: 28,
            };
            panel_inset(fb, field);
            let current = state.model.skin_path.as_ref().map_or_else(
                || "Base Skin".into(),
                |path| path.to_string_lossy().into_owned(),
            );
            draw_text_clipped(
                fb,
                field.x + 6,
                field.y + 10,
                &current,
                field.width - 12,
                TEXT,
            );
        }
    }
}

fn draw_sidebar_control(fb: &mut Framebuffer, control: &ControlInfo, pressed: bool) {
    let background = if control.selected { SELECTED } else { FACE };
    fill(fb, control.rect, background);
    if control.selected || pressed {
        outline(fb, control.rect, if pressed { DARK } else { SELECTED });
    }
    let color = if control.selected {
        SELECTED_TEXT
    } else {
        TEXT
    };
    draw_text(
        fb,
        control.rect.x + 8,
        control.rect.y + (control.rect.height - font::GLYPH_H as i32) / 2,
        &control.label,
        color,
    );
    if control.focused {
        focus_rect(fb, inset(control.rect, 3), color);
    }
}

fn draw_control(fb: &mut Framebuffer, control: &ControlInfo, pressed: bool) {
    match control.role {
        ControlRole::CheckBox => {
            draw_check(fb, control, false);
        }
        ControlRole::RadioButton => {
            draw_check(fb, control, true);
        }
        ControlRole::Slider => draw_slider(fb, control, pressed),
        ControlRole::ListItem => {
            if control.selected {
                fill(fb, control.rect, SELECTED);
            }
            draw_text_clipped(
                fb,
                control.rect.x + 4,
                control.rect.y + 7,
                &control.label,
                control.rect.width - 8,
                if control.selected {
                    SELECTED_TEXT
                } else {
                    TEXT
                },
            );
            if control.focused {
                focus_rect(
                    fb,
                    inset(control.rect, 2),
                    if control.selected {
                        SELECTED_TEXT
                    } else {
                        DARK
                    },
                );
            }
        }
        ControlRole::Button => draw_button(fb, control, pressed),
        ControlRole::Tab => {}
    }
}

fn draw_slider(fb: &mut Framebuffer, control: &ControlInfo, pressed: bool) {
    let Some(range) = control.range else {
        return;
    };
    let track = Rect {
        x: control.rect.x + SLIDER_THUMB_W / 2,
        y: control.rect.y + 7,
        width: (control.rect.width - SLIDER_THUMB_W + 1).max(1),
        height: 5,
    };
    panel_inset(fb, track);

    let span = (track.width - 1).max(1);
    let value_span = i32::from(range.maximum.saturating_sub(range.minimum)).max(1);
    let offset = i32::from(range.value.saturating_sub(range.minimum)) * span / value_span;
    let thumb = Rect {
        x: track.x + offset - SLIDER_THUMB_W / 2,
        y: control.rect.y,
        width: SLIDER_THUMB_W,
        height: SLIDER_THUMB_H,
    };
    fill(fb, thumb, FACE);
    let (top, bottom) = if pressed {
        (DARK, LIGHT)
    } else {
        (LIGHT, DARK)
    };
    line_h(fb, thumb.x, thumb.y, thumb.width, top);
    line_v(fb, thumb.x, thumb.y, thumb.height, top);
    line_h(fb, thumb.x, thumb.y + thumb.height - 1, thumb.width, bottom);
    line_v(fb, thumb.x + thumb.width - 1, thumb.y, thumb.height, bottom);

    let labels_y = control.rect.y + 27;
    draw_text(fb, control.rect.x, labels_y, "Slow", DIM);
    let fast_x = control.rect.x + control.rect.width - font::text_width("Fast") as i32;
    draw_text(fb, fast_x, labels_y, "Fast", DIM);
    let value = range.value.to_string();
    let value_x = control.rect.x + (control.rect.width - font::text_width(&value) as i32) / 2;
    draw_text(fb, value_x, labels_y, &value, TEXT);
    if control.focused {
        focus_rect(fb, inset(control.rect, 1), DARK);
    }
}

fn draw_check(fb: &mut Framebuffer, control: &ControlInfo, radio: bool) {
    let box_rect = Rect {
        x: control.rect.x,
        y: control.rect.y + (control.rect.height - CHECK_SIZE) / 2,
        width: CHECK_SIZE,
        height: CHECK_SIZE,
    };
    if radio {
        draw_radio(fb, box_rect, control.checked == Some(true));
    } else {
        panel_inset(fb, box_rect);
        if control.checked == Some(true) {
            for step in 0..4 {
                pixel(fb, box_rect.x + 2 + step, box_rect.y + 6 + step / 2, TEXT);
                pixel(fb, box_rect.x + 5 + step, box_rect.y + 7 - step, TEXT);
            }
        }
    }
    draw_text(
        fb,
        control.rect.x + CHECK_SIZE + 7,
        control.rect.y + (control.rect.height - font::GLYPH_H as i32) / 2,
        &control.label,
        if control.enabled { TEXT } else { DIM },
    );
    if control.focused {
        let label_w = font::text_width(&control.label) as i32;
        focus_rect(
            fb,
            Rect {
                x: control.rect.x + CHECK_SIZE + 4,
                y: control.rect.y + 3,
                width: (label_w + 6).min(control.rect.width - CHECK_SIZE - 4),
                height: control.rect.height - 6,
            },
            DARK,
        );
    }
}

fn draw_radio(fb: &mut Framebuffer, rect: Rect, checked: bool) {
    let cx = rect.x + rect.width / 2;
    let cy = rect.y + rect.height / 2;
    for (dx, dy) in [
        (0, -5),
        (-3, -4),
        (3, -4),
        (-5, -2),
        (5, -2),
        (-5, 2),
        (5, 2),
        (-3, 4),
        (3, 4),
        (0, 5),
    ] {
        pixel(fb, cx + dx, cy + dy, DARK);
    }
    if checked {
        fill(
            fb,
            Rect {
                x: cx - 2,
                y: cy - 2,
                width: 5,
                height: 5,
            },
            TEXT,
        );
    }
}

fn draw_button(fb: &mut Framebuffer, control: &ControlInfo, pressed: bool) {
    fill(fb, control.rect, FACE);
    let (top, bottom) = if pressed {
        (DARK, LIGHT)
    } else {
        (LIGHT, DARK)
    };
    line_h(fb, control.rect.x, control.rect.y, control.rect.width, top);
    line_v(fb, control.rect.x, control.rect.y, control.rect.height, top);
    line_h(
        fb,
        control.rect.x,
        control.rect.y + control.rect.height - 1,
        control.rect.width,
        bottom,
    );
    line_v(
        fb,
        control.rect.x + control.rect.width - 1,
        control.rect.y,
        control.rect.height,
        bottom,
    );
    let color = if control.enabled { TEXT } else { DIM };
    let text_x = control.rect.x
        + (control.rect.width - font::text_width(&control.label) as i32) / 2
        + i32::from(pressed);
    let text_y =
        control.rect.y + (control.rect.height - font::GLYPH_H as i32) / 2 + i32::from(pressed);
    draw_text(fb, text_x, text_y, &control.label, color);
    if control.focused {
        focus_rect(fb, inset(control.rect, 3), color);
    }
}

fn button_info(
    id: ControlId,
    label: &str,
    rect: Rect,
    enabled: bool,
    focus: ControlId,
) -> ControlInfo {
    ControlInfo {
        id,
        role: ControlRole::Button,
        label: label.into(),
        rect,
        enabled,
        selected: false,
        checked: None,
        range: None,
        focused: focus == id,
    }
}

fn clamped_size(width: i32, height: i32) -> (i32, i32) {
    (width.max(PREFERENCES_W), height.max(PREFERENCES_H))
}

fn content_rect(width: i32, height: i32) -> Rect {
    Rect {
        x: PAD + SIDEBAR_W + CONTENT_GAP,
        y: PREFERENCES_TITLE_H + PAD,
        width: width - (PAD + SIDEBAR_W + CONTENT_GAP) - PAD,
        height: height - (PREFERENCES_TITLE_H + PAD) - BOTTOM_H,
    }
}

fn close_rect(width: i32, height: i32) -> Rect {
    Rect {
        x: width - PAD - 82,
        y: height - BOTTOM_H + 10,
        width: 82,
        height: CONTROL_H,
    }
}

fn slider_value_at_x(rect: Rect, x: i32, (minimum, maximum): (u8, u8)) -> u8 {
    let start = rect.x + SLIDER_THUMB_W / 2;
    let span = (rect.width - SLIDER_THUMB_W).max(1);
    let position = (x - start).clamp(0, span);
    let value_span = i32::from(maximum - minimum);
    let rounded = (position * value_span + span / 2) / span;
    minimum + rounded as u8
}

/// The (minimum, maximum) of a range control, `None` for everything that is not a slider.
fn slider_bounds(id: ControlId) -> Option<(u8, u8)> {
    match id {
        ControlId::ShuffleMorphRate => Some((SHUFFLE_MORPH_RATE_MIN, SHUFFLE_MORPH_RATE_MAX)),
        ControlId::VisualizationRefreshRate
        | ControlId::VisualizationBarFalloff
        | ControlId::VisualizationPeakFalloff => Some((1, 10)),
        ControlId::DisplaySnapPx => Some((0, 30)),
        _ => None,
    }
}

fn library_list_rect(content: Rect) -> Rect {
    Rect {
        x: content.x + 18,
        y: content.y + 48,
        width: content.width - 36,
        height: (content.height - 136).max(ROOT_ROW_H + 4),
    }
}

fn visible_root_rows(rect: Rect) -> usize {
    ((rect.height - 4) / ROOT_ROW_H).max(0) as usize
}

fn inset(rect: Rect, amount: i32) -> Rect {
    Rect {
        x: rect.x + amount,
        y: rect.y + amount,
        width: (rect.width - amount * 2).max(0),
        height: (rect.height - amount * 2).max(0),
    }
}

fn panel(fb: &mut Framebuffer, rect: Rect) {
    fill(fb, rect, FACE);
    line_h(fb, rect.x, rect.y, rect.width, SHADOW);
    line_v(fb, rect.x, rect.y, rect.height, SHADOW);
    line_h(fb, rect.x, rect.y + rect.height - 1, rect.width, LIGHT);
    line_v(fb, rect.x + rect.width - 1, rect.y, rect.height, LIGHT);
}

fn panel_inset(fb: &mut Framebuffer, rect: Rect) {
    fill(fb, rect, FIELD);
    line_h(fb, rect.x, rect.y, rect.width, DARK);
    line_v(fb, rect.x, rect.y, rect.height, DARK);
    line_h(fb, rect.x, rect.y + rect.height - 1, rect.width, LIGHT);
    line_v(fb, rect.x + rect.width - 1, rect.y, rect.height, LIGHT);
}

fn outline(fb: &mut Framebuffer, rect: Rect, color: [u8; 3]) {
    line_h(fb, rect.x, rect.y, rect.width, color);
    line_h(fb, rect.x, rect.y + rect.height - 1, rect.width, color);
    line_v(fb, rect.x, rect.y, rect.height, color);
    line_v(fb, rect.x + rect.width - 1, rect.y, rect.height, color);
}

fn focus_rect(fb: &mut Framebuffer, rect: Rect, color: [u8; 3]) {
    for x in (rect.x..rect.x + rect.width.max(0)).step_by(2) {
        pixel(fb, x, rect.y, color);
        pixel(fb, x, rect.y + rect.height - 1, color);
    }
    for y in (rect.y..rect.y + rect.height.max(0)).step_by(2) {
        pixel(fb, rect.x, y, color);
        pixel(fb, rect.x + rect.width - 1, y, color);
    }
}

fn fill(fb: &mut Framebuffer, rect: Rect, color: [u8; 3]) {
    for y in rect.y.max(0)..(rect.y + rect.height).min(fb.height as i32) {
        for x in rect.x.max(0)..(rect.x + rect.width).min(fb.width as i32) {
            pixel(fb, x, y, color);
        }
    }
}

fn line_h(fb: &mut Framebuffer, x: i32, y: i32, width: i32, color: [u8; 3]) {
    for xx in x..x + width.max(0) {
        pixel(fb, xx, y, color);
    }
}

fn line_v(fb: &mut Framebuffer, x: i32, y: i32, height: i32, color: [u8; 3]) {
    for yy in y..y + height.max(0) {
        pixel(fb, x, yy, color);
    }
}

fn pixel(fb: &mut Framebuffer, x: i32, y: i32, color: [u8; 3]) {
    if x < 0 || y < 0 || x >= fb.width as i32 || y >= fb.height as i32 {
        return;
    }
    let offset = ((y as u32 * fb.width + x as u32) * 4) as usize;
    fb.rgba[offset..offset + 3].copy_from_slice(&color);
    fb.rgba[offset + 3] = 255;
}

fn draw_text(fb: &mut Framebuffer, x: i32, y: i32, text: &str, color: [u8; 3]) {
    font::draw_text(&mut fb.rgba, fb.width, fb.height, x, y, text, color);
}

fn draw_text_clipped(
    fb: &mut Framebuffer,
    x: i32,
    y: i32,
    text: &str,
    max_width: i32,
    color: [u8; 3],
) {
    let max_chars = (max_width / font::ADVANCE as i32).max(0) as usize;
    let clipped: String = text.chars().take(max_chars).collect();
    draw_text(fb, x, y, &clipped, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control(state: &PreferencesState, id: ControlId) -> ControlInfo {
        state
            .controls(PREFERENCES_W, PREFERENCES_H)
            .into_iter()
            .find(|control| control.id == id)
            .expect("control is visible")
    }

    fn click(state: &mut PreferencesState, id: ControlId) -> Outcome {
        let rect = control(state, id).rect;
        assert_eq!(
            state.pointer_press(rect.x + 1, rect.y + 1, PREFERENCES_W, PREFERENCES_H),
            Outcome::Redraw
        );
        state.pointer_release(rect.x + 1, rect.y + 1, PREFERENCES_W, PREFERENCES_H)
    }

    #[test]
    fn sections_and_controls_include_only_supported_settings() {
        assert_eq!(
            Section::ALL.map(Section::label),
            [
                "Shuffle",
                "Options",
                "Visualization",
                "Display",
                "Audio Library",
                "Skins"
            ]
        );
        let mut state = PreferencesState::default();
        let mut labels = Vec::new();
        for section in Section::ALL {
            state.set_section(section);
            labels.extend(
                state
                    .controls(PREFERENCES_W, PREFERENCES_H)
                    .into_iter()
                    .map(|control| control.label),
            );
        }
        let labels = labels.join("|").to_ascii_lowercase();
        for excluded in [
            "setup",
            "video",
            "cd ripping",
            "plugin",
            "webamp",
            "milkdrop",
        ] {
            assert!(!labels.contains(excluded), "unexpected control: {excluded}");
        }
    }

    #[test]
    fn shuffle_page_exposes_an_accessible_clamped_morph_slider() {
        let state = PreferencesState::new(PreferencesModel {
            shuffle_morph_rate: u8::MAX,
            ..PreferencesModel::default()
        });
        assert_eq!(state.model.shuffle_morph_rate, SHUFFLE_MORPH_RATE_MAX);
        let slider = control(&state, ControlId::ShuffleMorphRate);
        assert_eq!(slider.role, ControlRole::Slider);
        assert_eq!(slider.label, "Shuffle morph rate");
        assert_eq!(slider.checked, None);
        assert_eq!(
            slider.range,
            Some(RangeInfo {
                value: SHUFFLE_MORPH_RATE_MAX,
                minimum: SHUFFLE_MORPH_RATE_MIN,
                maximum: SHUFFLE_MORPH_RATE_MAX,
                step: 1,
            })
        );
    }

    #[test]
    fn pointer_click_and_drag_update_the_shuffle_morph_rate_continuously() {
        let mut state = PreferencesState::default();
        let rect = control(&state, ControlId::ShuffleMorphRate).rect;
        assert_eq!(
            state.pointer_press(rect.x, rect.y + 1, PREFERENCES_W, PREFERENCES_H),
            Outcome::Command(Command::SetShuffleMorphRate(SHUFFLE_MORPH_RATE_MIN))
        );
        assert_eq!(state.model.shuffle_morph_rate, SHUFFLE_MORPH_RATE_MIN);
        assert_eq!(
            state.pointer_motion(rect.x + rect.width + 100, PREFERENCES_W, PREFERENCES_H),
            Outcome::Command(Command::SetShuffleMorphRate(SHUFFLE_MORPH_RATE_MAX))
        );
        assert_eq!(state.model.shuffle_morph_rate, SHUFFLE_MORPH_RATE_MAX);
        assert_eq!(
            state.pointer_release(
                rect.x + rect.width + 100,
                rect.y + 1,
                PREFERENCES_W,
                PREFERENCES_H
            ),
            Outcome::Redraw
        );
        assert_eq!(
            state.pointer_motion(rect.x, PREFERENCES_W, PREFERENCES_H),
            Outcome::Unchanged,
            "release ends the drag"
        );
    }

    #[test]
    fn shuffle_morph_slider_supports_range_keyboard_conventions() {
        let mut state = PreferencesState::default();
        assert_eq!(
            state.key(Key::Tab, PREFERENCES_W, PREFERENCES_H),
            Outcome::Redraw
        );
        assert_eq!(state.focused_control(), ControlId::ShuffleMorphRate);
        assert_eq!(
            state.key(Key::Left, PREFERENCES_W, PREFERENCES_H),
            Outcome::Command(Command::SetShuffleMorphRate(49))
        );
        assert_eq!(
            state.key(Key::Home, PREFERENCES_W, PREFERENCES_H),
            Outcome::Command(Command::SetShuffleMorphRate(0))
        );
        assert_eq!(
            state.key(Key::Down, PREFERENCES_W, PREFERENCES_H),
            Outcome::Redraw,
            "decrementing at the minimum clamps"
        );
        assert_eq!(
            state.key(Key::End, PREFERENCES_W, PREFERENCES_H),
            Outcome::Command(Command::SetShuffleMorphRate(50))
        );
        assert_eq!(
            state.key(Key::Up, PREFERENCES_W, PREFERENCES_H),
            Outcome::Redraw,
            "incrementing at the maximum clamps"
        );
    }

    #[test]
    fn visualization_and_display_radios_emit_exact_field_values() {
        let mut state = PreferencesState::default();
        state.set_section(Section::Visualization);
        assert_eq!(
            click(&mut state, ControlId::VisualizationOscilloscope),
            Outcome::Command(Command::SetVisualizationMode(
                VisualizationMode::Oscilloscope
            ))
        );
        assert_eq!(
            click(&mut state, ControlId::VisualizationShowPeaks),
            Outcome::Command(Command::SetVisualizationShowPeaks(false))
        );

        state.set_section(Section::Display);
        assert_eq!(
            click(&mut state, ControlId::DisplayRemaining),
            Outcome::Command(Command::SetDisplayTime(TimeDisplay::Remaining))
        );
        assert_eq!(
            click(&mut state, ControlId::DisplayDoubleSize),
            Outcome::Command(Command::SetDisplayDoubleSize(true))
        );
        assert_eq!(
            click(&mut state, ControlId::DisplayScrollTitle),
            Outcome::Command(Command::SetDisplayScrollTitle(false))
        );
    }

    #[test]
    fn options_page_toggles_emit_their_commands() {
        let mut state = PreferencesState::default();
        state.set_section(Section::Options);
        assert_eq!(
            click(&mut state, ControlId::OptionsReadOnPlay),
            Outcome::Command(Command::SetReadTitlesOnLoad(false))
        );
        assert_eq!(
            click(&mut state, ControlId::OptionsReadOnLoad),
            Outcome::Command(Command::SetReadTitlesOnLoad(true))
        );
        assert_eq!(
            click(&mut state, ControlId::OptionsSortOnLoad),
            Outcome::Command(Command::SetSortOnLoad(true))
        );
        assert_eq!(
            click(&mut state, ControlId::OptionsManualAdvance),
            Outcome::Command(Command::SetManualAdvance(true))
        );
        assert_eq!(
            click(&mut state, ControlId::OptionsConvertUnderscores),
            Outcome::Command(Command::SetConvertUnderscores(true))
        );
        assert_eq!(
            click(&mut state, ControlId::OptionsConvertPercent20),
            Outcome::Command(Command::SetConvertPercent20(true))
        );
    }

    #[test]
    fn visualization_page_styles_and_speed_sliders_are_live() {
        let mut state = PreferencesState::default();
        state.set_section(Section::Visualization);
        assert_eq!(
            click(&mut state, ControlId::VisualizationAnalyzerFire),
            Outcome::Command(Command::SetAnalyzerStyle(AnalyzerStyle::Fire))
        );
        assert_eq!(
            click(&mut state, ControlId::VisualizationBandThin),
            Outcome::Command(Command::SetBandWidth(BandWidth::Thin))
        );
        assert_eq!(
            click(&mut state, ControlId::VisualizationOscSolid),
            Outcome::Command(Command::SetOscilloscopeStyle(OscStyle::Solid))
        );
        // The three speed sliders are accessible ranges with the 1..=10 bounds.
        for (id, value) in [
            (ControlId::VisualizationRefreshRate, 8),
            (ControlId::VisualizationBarFalloff, 7),
            (ControlId::VisualizationPeakFalloff, 6),
        ] {
            let info = control(&state, id);
            assert_eq!(info.role, ControlRole::Slider);
            assert_eq!(
                info.range,
                Some(RangeInfo {
                    value,
                    minimum: 1,
                    maximum: 10,
                    step: 1
                })
            );
        }
        // Keyboard range conventions work on the generalized sliders.
        state.focus = ControlId::VisualizationRefreshRate;
        assert_eq!(
            state.key(Key::Left, PREFERENCES_W, PREFERENCES_H),
            Outcome::Command(Command::SetRefreshRate(7))
        );
        assert_eq!(
            state.key(Key::End, PREFERENCES_W, PREFERENCES_H),
            Outcome::Command(Command::SetRefreshRate(10))
        );
    }

    #[test]
    fn display_page_gains_clutterbar_numbers_and_snap() {
        let mut state = PreferencesState::default();
        state.set_section(Section::Display);
        assert_eq!(
            click(&mut state, ControlId::DisplayClutterbar),
            Outcome::Command(Command::SetDisplayClutterbar(false))
        );
        assert_eq!(
            click(&mut state, ControlId::DisplayPlaylistNumbers),
            Outcome::Command(Command::SetDisplayPlaylistNumbers(false))
        );
        let snap = control(&state, ControlId::DisplaySnapPx);
        assert_eq!(
            snap.range,
            Some(RangeInfo {
                value: 15,
                minimum: 0,
                maximum: 30,
                step: 1
            })
        );
        state.focus = ControlId::DisplaySnapPx;
        assert_eq!(
            state.key(Key::Home, PREFERENCES_W, PREFERENCES_H),
            Outcome::Command(Command::SetSnapPx(0)),
            "snapping can be disabled entirely"
        );
    }

    #[test]
    fn keyboard_navigation_changes_sections_and_activates_controls() {
        let mut state = PreferencesState::default();
        assert_eq!(
            state.key(Key::Down, PREFERENCES_W, PREFERENCES_H),
            Outcome::Redraw
        );
        assert_eq!(state.section(), Section::Options);
        assert_eq!(
            state.key(Key::Down, PREFERENCES_W, PREFERENCES_H),
            Outcome::Redraw
        );
        assert_eq!(state.section(), Section::Visualization);
        assert_eq!(
            state.key(Key::Tab, PREFERENCES_W, PREFERENCES_H),
            Outcome::Redraw
        );
        assert_eq!(state.focused_control(), ControlId::VisualizationSpectrum);
        assert_eq!(
            state.key(Key::Right, PREFERENCES_W, PREFERENCES_H),
            Outcome::Command(Command::SetVisualizationMode(
                VisualizationMode::Oscilloscope
            ))
        );
        assert_eq!(
            state.focused_control(),
            ControlId::VisualizationOscilloscope
        );
        assert_eq!(
            state.key(Key::Escape, PREFERENCES_W, PREFERENCES_H),
            Outcome::Close
        );
    }

    #[test]
    fn library_chooser_results_are_deduplicated_and_removal_is_clamped() {
        let mut state = PreferencesState::new(PreferencesModel {
            library_roots: vec!["/music/a".into(), "/music/a".into(), PathBuf::new()],
            ..PreferencesModel::default()
        });
        assert_eq!(state.model.library_roots, [PathBuf::from("/music/a")]);
        state.set_section(Section::AudioLibrary);
        assert_eq!(
            click(&mut state, ControlId::LibraryAdd),
            Outcome::Command(Command::ChooseLibraryDirectory)
        );
        assert_eq!(
            state.add_library_root("/music/a".into(), PREFERENCES_H),
            Outcome::Unchanged,
            "duplicate chooser result is ignored"
        );
        assert_eq!(
            state.add_library_root("/music/b".into(), PREFERENCES_H),
            Outcome::Command(Command::SetLibraryRoots(vec![
                "/music/a".into(),
                "/music/b".into()
            ]))
        );
        state.set_library_selection(Some(usize::MAX), PREFERENCES_H);
        assert_eq!(
            state.library_selection(),
            Some(1),
            "invalid index clamps to last root"
        );
        assert_eq!(
            click(&mut state, ControlId::LibraryRemove),
            Outcome::Command(Command::SetLibraryRoots(vec!["/music/a".into()]))
        );
        assert_eq!(state.library_selection(), Some(0));
    }

    #[test]
    fn long_library_lists_scroll_selection_into_view() {
        let mut state = PreferencesState::new(PreferencesModel {
            library_roots: (0..30)
                .map(|index| format!("/music/{index}").into())
                .collect(),
            ..PreferencesModel::default()
        });
        state.set_section(Section::AudioLibrary);
        state.set_library_selection(Some(29), PREFERENCES_H);
        state.focus = ControlId::LibraryRoot(29);
        let visible = state.controls(PREFERENCES_W, PREFERENCES_H);
        assert!(visible
            .iter()
            .any(|control| control.id == ControlId::LibraryRoot(29)));
        assert_eq!(
            state.key(Key::Down, PREFERENCES_W, PREFERENCES_H),
            Outcome::Redraw
        );
        assert_eq!(
            state.library_selection(),
            Some(29),
            "selection clamps at list end"
        );
    }

    #[test]
    fn skin_actions_keep_base_skin_and_picker_separate() {
        let mut state = PreferencesState::new(PreferencesModel {
            skin_path: Some("/skins/classic.wsz".into()),
            ..PreferencesModel::default()
        });
        state.set_section(Section::Skins);
        assert_eq!(
            click(&mut state, ControlId::SkinLoad),
            Outcome::Command(Command::ChooseSkinFile)
        );
        assert_eq!(
            state.set_skin_path("/skins/other.wsz".into()),
            Outcome::Command(Command::SetSkinPath(Some("/skins/other.wsz".into())))
        );
        assert_eq!(
            click(&mut state, ControlId::SkinBase),
            Outcome::Command(Command::SetSkinPath(None))
        );
        assert_eq!(state.model.skin_path, None);
    }

    #[test]
    fn disabled_remove_does_not_arm_or_activate() {
        let mut state = PreferencesState::default();
        state.set_section(Section::AudioLibrary);
        let remove = control(&state, ControlId::LibraryRemove);
        assert!(!remove.enabled);
        assert_eq!(
            state.pointer_press(
                remove.rect.x + 1,
                remove.rect.y + 1,
                PREFERENCES_W,
                PREFERENCES_H
            ),
            Outcome::Unchanged
        );
        assert_eq!(
            state.pointer_release(
                remove.rect.x + 1,
                remove.rect.y + 1,
                PREFERENCES_W,
                PREFERENCES_H
            ),
            Outcome::Unchanged
        );
    }

    #[test]
    fn composition_is_opaque_deterministic_and_clamps_tiny_sizes() {
        let state = PreferencesState::default();
        let theme = PrefsTheme::classic();
        let first = compose(&state, 1, 1, &theme);
        let second = compose(&state, PREFERENCES_W, PREFERENCES_H, &theme);
        assert_eq!(
            (first.width, first.height),
            (PREFERENCES_W as u32, PREFERENCES_H as u32)
        );
        assert_eq!(first.rgba, second.rgba);
        assert!(first.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
    }

    #[test]
    fn adwaita_theme_is_opaque_and_differs_by_palette_when_a_font_is_present() {
        // Skip on hosts without a system UI font (headless), so this passes anywhere.
        let Some(font) = UiFont::load_system() else {
            return;
        };
        let state = PreferencesState::default();
        let light = compose(&state, PREFERENCES_W, PREFERENCES_H, &PrefsTheme::adwaita(Palette::light(), &font));
        let dark = compose(&state, PREFERENCES_W, PREFERENCES_H, &PrefsTheme::adwaita(Palette::dark(), &font));
        assert!(light.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255), "opaque");
        assert_ne!(light.rgba, dark.rgba, "light and dark render differently");
        // The Adwaita body background differs from the classic gray.
        assert_ne!(&light.rgba[..3], &BG[..], "adwaita body is not the classic gray");
    }
}
