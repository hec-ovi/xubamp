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

use crate::Framebuffer;

/// Minimum preferences window size. Larger sizes give the library list more room.
pub const PREFERENCES_W: i32 = 500;
pub const PREFERENCES_H: i32 = 350;
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
    Visualization,
    Display,
    AudioLibrary,
    Skins,
}

impl Section {
    pub const ALL: [Self; 5] = [
        Self::Shuffle,
        Self::Visualization,
        Self::Display,
        Self::AudioLibrary,
        Self::Skins,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Shuffle => "Shuffle",
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
    pub display_time: TimeDisplay,
    pub display_double_size: bool,
    pub display_scroll_title: bool,
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
            display_time: TimeDisplay::Elapsed,
            display_double_size: false,
            display_scroll_title: true,
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
    DisplayElapsed,
    DisplayRemaining,
    DisplayDoubleSize,
    DisplayScrollTitle,
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
    SetDisplayTime(TimeDisplay),
    SetDisplayDoubleSize(bool),
    SetDisplayScrollTitle(bool),
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
                controls.push(check(
                    ControlId::VisualizationShowPeaks,
                    "Show spectrum peaks",
                    content.y + 146,
                    self.model.visualization_show_peaks,
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
        if control.id == ControlId::ShuffleMorphRate {
            self.set_shuffle_morph_rate(slider_value_at_x(control.rect, x))
        } else {
            Outcome::Redraw
        }
    }

    /// Update a pressed slider while the pointer is dragged. The value clamps at either end even
    /// when the pointer moves beyond the visible track.
    pub fn pointer_motion(&mut self, x: i32, width: i32, height: i32) -> Outcome {
        if self.pressed != Some(ControlId::ShuffleMorphRate) {
            return Outcome::Unchanged;
        }
        let Some(control) = self
            .controls(width, height)
            .into_iter()
            .find(|control| control.id == ControlId::ShuffleMorphRate)
        else {
            return Outcome::Unchanged;
        };
        self.set_shuffle_morph_rate(slider_value_at_x(control.rect, x))
    }

    pub fn pointer_release(&mut self, x: i32, y: i32, width: i32, height: i32) -> Outcome {
        let pressed = self.pressed.take();
        if pressed == Some(ControlId::ShuffleMorphRate) {
            let Some(control) = self
                .controls(width, height)
                .into_iter()
                .find(|control| control.id == ControlId::ShuffleMorphRate)
            else {
                return Outcome::Redraw;
            };
            return self.set_shuffle_morph_rate(slider_value_at_x(control.rect, x));
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
            Key::Left | Key::Down if self.focus == ControlId::ShuffleMorphRate => {
                self.adjust_shuffle_morph_rate(-1)
            }
            Key::Right | Key::Up if self.focus == ControlId::ShuffleMorphRate => {
                self.adjust_shuffle_morph_rate(1)
            }
            Key::Home if self.focus == ControlId::ShuffleMorphRate => {
                self.set_shuffle_morph_rate(SHUFFLE_MORPH_RATE_MIN)
            }
            Key::End if self.focus == ControlId::ShuffleMorphRate => {
                self.set_shuffle_morph_rate(SHUFFLE_MORPH_RATE_MAX)
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

    fn set_shuffle_morph_rate(&mut self, rate: u8) -> Outcome {
        let rate = rate.clamp(SHUFFLE_MORPH_RATE_MIN, SHUFFLE_MORPH_RATE_MAX);
        if self.model.shuffle_morph_rate == rate {
            Outcome::Redraw
        } else {
            self.model.shuffle_morph_rate = rate;
            Outcome::Command(Command::SetShuffleMorphRate(rate))
        }
    }

    fn adjust_shuffle_morph_rate(&mut self, delta: i8) -> Outcome {
        let rate = i16::from(self.model.shuffle_morph_rate) + i16::from(delta);
        self.set_shuffle_morph_rate(rate.clamp(
            i16::from(SHUFFLE_MORPH_RATE_MIN),
            i16::from(SHUFFLE_MORPH_RATE_MAX),
        ) as u8)
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
pub fn compose(state: &PreferencesState, width: i32, height: i32) -> Framebuffer {
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

fn slider_value_at_x(rect: Rect, x: i32) -> u8 {
    let start = rect.x + SLIDER_THUMB_W / 2;
    let span = (rect.width - SLIDER_THUMB_W).max(1);
    let position = (x - start).clamp(0, span);
    let value_span = i32::from(SHUFFLE_MORPH_RATE_MAX - SHUFFLE_MORPH_RATE_MIN);
    let rounded = (position * value_span + span / 2) / span;
    SHUFFLE_MORPH_RATE_MIN + rounded as u8
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
    fn keyboard_navigation_changes_sections_and_activates_controls() {
        let mut state = PreferencesState::default();
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
        let first = compose(&state, 1, 1);
        let second = compose(&state, PREFERENCES_W, PREFERENCES_H);
        assert_eq!(
            (first.width, first.height),
            (PREFERENCES_W as u32, PREFERENCES_H as u32)
        );
        assert_eq!(first.rgba, second.rgba);
        assert!(first.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
    }
}
