//! Pure data models, layout, interaction, and software rendering for classic popup menus.
//!
//! This is deliberately not a claim of native OS menu integration. It provides deterministic,
//! authored popup surfaces and compositor-independent input policy; the Wayland layer can place
//! those surfaces in popups later. Models are recursive, so every menu can expose nested flyouts.

use std::fmt;

use xubamp_skin::font;

use crate::adwaita::{self, Palette, UiFont};
use crate::Framebuffer;

/// Height of an ordinary menu row in pixels.
pub const ROW_HEIGHT: i32 = 20;
/// Height of a separator row in pixels.
pub const SEPARATOR_HEIGHT: i32 = 7;

const BORDER: i32 = 2;
const MIN_WIDTH: i32 = 152;
const LEFT_PAD: i32 = 8;
const MARK_WIDTH: i32 = 16;
const SHORTCUT_GAP: i32 = 18;
const ARROW_WIDTH: i32 = 14;
const RIGHT_PAD: i32 = 7;
const FLYOUT_OVERLAP: i32 = 2;

// Authored, neutral popup colors. These approximate classic desktop menus without depending on a
// toolkit theme or copying any operating-system assets.
const FACE: [u8; 3] = [226, 226, 222];
const LIGHT: [u8; 3] = [255, 255, 255];
const SHADOW: [u8; 3] = [118, 118, 114];
const DARK: [u8; 3] = [42, 42, 40];
const TEXT: [u8; 3] = [20, 20, 18];
const DISABLED_TEXT: [u8; 3] = [126, 126, 122];
const SELECTED: [u8; 3] = [58, 91, 146];
const SELECTED_PRESSED: [u8; 3] = [42, 69, 116];
const SELECTED_TEXT: [u8; 3] = [255, 255, 255];

/// A check/radio indicator shown in a menu item's leading lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ItemMark {
    /// No leading indicator.
    #[default]
    None,
    /// A checkbox, with its current value.
    Check(bool),
    /// A mutually-exclusive radio choice, with its current value.
    Radio(bool),
}

/// Presentation and activation state shared by actions and submenus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemState {
    /// Disabled items are visible but cannot receive keyboard focus or activate.
    pub enabled: bool,
    /// Optional checkbox/radio indicator.
    pub mark: ItemMark,
}

impl Default for ItemState {
    fn default() -> Self {
        Self {
            enabled: true,
            mark: ItemMark::None,
        }
    }
}

/// The behavior represented by one menu row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuItemKind<A> {
    /// Activate a caller-owned command.
    Action(A),
    /// Open a nested flyout.
    Submenu(Menu<A>),
    /// A visual grouping line. Separators never receive focus.
    Separator,
}

/// One menu row. Labels and shortcut hints are owned so models can be assembled from settings and
/// caller-supplied equalizer preset names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem<A> {
    pub label: String,
    pub shortcut: Option<String>,
    pub state: ItemState,
    pub kind: MenuItemKind<A>,
}

impl<A> MenuItem<A> {
    pub fn action(label: impl Into<String>, action: A) -> Self {
        Self {
            label: label.into(),
            shortcut: None,
            state: ItemState::default(),
            kind: MenuItemKind::Action(action),
        }
    }

    pub fn submenu(label: impl Into<String>, menu: Menu<A>) -> Self {
        Self {
            label: label.into(),
            shortcut: None,
            state: ItemState::default(),
            kind: MenuItemKind::Submenu(menu),
        }
    }

    pub fn separator() -> Self {
        Self {
            label: String::new(),
            shortcut: None,
            state: ItemState::default(),
            kind: MenuItemKind::Separator,
        }
    }

    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn with_mark(mut self, mark: ItemMark) -> Self {
        self.state.mark = mark;
        self
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.state.enabled = enabled;
        self
    }

    fn is_keyboard_focusable(&self) -> bool {
        self.state.enabled && !matches!(self.kind, MenuItemKind::Separator)
    }
}

/// A recursive popup menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu<A> {
    pub items: Vec<MenuItem<A>>,
}

impl<A> Menu<A> {
    pub fn new(items: Vec<MenuItem<A>>) -> Self {
        Self { items }
    }
}

impl<A> Default for Menu<A> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

/// Caller commands emitted by xubamp's classic menus. Entries that the product intentionally does
/// not support have no action variant and are not placed in any constructor below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassicMenuAction {
    OpenMedia,
    Play,
    ToggleMainWindow,
    ToggleEqualizer,
    TogglePlaylistEditor,
    LoadSkin,
    UseBaseSkin,
    ShowElapsedTime,
    ShowRemainingTime,
    ToggleDoubleSize,
    ToggleRepeat,
    ToggleShuffle,
    OpenPreferences,
    Previous,
    Pause,
    Stop,
    Next,
    BackFiveSeconds,
    ForwardFiveSeconds,
    BackTenTracks,
    ForwardTenTracks,
    Exit,
    PlaylistAddUrl,
    PlaylistAddDirectory,
    PlaylistAddFile,
    PlaylistRemoveSelected,
    PlaylistRemoveAll,
    PlaylistCrop,
    PlaylistRemoveDead,
    PlaylistSelectAll,
    PlaylistSelectNone,
    PlaylistSelectInvert,
    PlaylistSortTitle,
    PlaylistSortFilename,
    PlaylistSortPath,
    PlaylistReverse,
    PlaylistRandomize,
    PlaylistFileInfo,
    PlaylistNewList,
    PlaylistSaveList,
    PlaylistLoadList,
    EqualizerLoadPreset(usize),
    EqualizerLoadEqf,
    EqualizerSaveAs,
}

/// Which clock representation is checked in the Options submenu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeDisplay {
    #[default]
    Elapsed,
    Remaining,
}

/// Runtime values reflected by checks and radios in the main menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MainMenuState {
    pub main_window_open: bool,
    pub equalizer_open: bool,
    pub playlist_open: bool,
    pub time_display: TimeDisplay,
    pub double_size: bool,
    pub repeat: bool,
    pub shuffle: bool,
}

impl Default for MainMenuState {
    fn default() -> Self {
        Self {
            main_window_open: true,
            equalizer_open: false,
            playlist_open: false,
            time_display: TimeDisplay::Elapsed,
            double_size: false,
            repeat: false,
            shuffle: false,
        }
    }
}

/// The classic main popup. Settings actions open focused settings surfaces; the Library action is
/// explicitly audio-only at the command boundary.
pub fn main_menu(state: MainMenuState) -> Menu<ClassicMenuAction> {
    let skins = Menu::new(vec![
        MenuItem::action("Load Skin...", ClassicMenuAction::LoadSkin),
        MenuItem::separator(),
        MenuItem::action("<Base Skin>", ClassicMenuAction::UseBaseSkin),
    ]);
    let options = Menu::new(vec![
        MenuItem::action("Preferences...", ClassicMenuAction::OpenPreferences)
            .with_shortcut("Ctrl+P"),
        MenuItem::separator(),
        MenuItem::submenu("Skins", skins.clone()),
        MenuItem::separator(),
        MenuItem::action("Time elapsed", ClassicMenuAction::ShowElapsedTime)
            .with_shortcut("(Ctrl+T toggles)")
            .with_mark(ItemMark::Radio(state.time_display == TimeDisplay::Elapsed)),
        MenuItem::action("Time remaining", ClassicMenuAction::ShowRemainingTime)
            .with_shortcut("(Ctrl+T toggles)")
            .with_mark(ItemMark::Radio(
                state.time_display == TimeDisplay::Remaining,
            )),
        MenuItem::action("Double Size", ClassicMenuAction::ToggleDoubleSize)
            .with_shortcut("Ctrl+D")
            .with_mark(ItemMark::Check(state.double_size)),
        MenuItem::separator(),
        MenuItem::action("Repeat", ClassicMenuAction::ToggleRepeat)
            .with_shortcut("R")
            .with_mark(ItemMark::Check(state.repeat)),
        MenuItem::action("Shuffle", ClassicMenuAction::ToggleShuffle)
            .with_shortcut("S")
            .with_mark(ItemMark::Check(state.shuffle)),
    ]);
    let playback = Menu::new(vec![
        MenuItem::action("Previous", ClassicMenuAction::Previous).with_shortcut("Z"),
        MenuItem::action("Play", ClassicMenuAction::Play).with_shortcut("X"),
        MenuItem::action("Pause", ClassicMenuAction::Pause).with_shortcut("C"),
        MenuItem::action("Stop", ClassicMenuAction::Stop).with_shortcut("V"),
        MenuItem::action("Next", ClassicMenuAction::Next).with_shortcut("B"),
        MenuItem::separator(),
        MenuItem::action("Back 5 seconds", ClassicMenuAction::BackFiveSeconds)
            .with_shortcut("Left"),
        MenuItem::action("Fwd 5 seconds", ClassicMenuAction::ForwardFiveSeconds)
            .with_shortcut("Right"),
        MenuItem::action("10 tracks back", ClassicMenuAction::BackTenTracks)
            .with_shortcut("Num. 1"),
        MenuItem::action("10 tracks fwd", ClassicMenuAction::ForwardTenTracks)
            .with_shortcut("Num. 3"),
    ]);
    let play = Menu::new(vec![MenuItem::action(
        "File...",
        ClassicMenuAction::OpenMedia,
    )
    .with_shortcut("L")]);

    Menu::new(vec![
        MenuItem::submenu("Play", play),
        MenuItem::separator(),
        MenuItem::action("Main Window", ClassicMenuAction::ToggleMainWindow)
            .with_mark(ItemMark::Check(state.main_window_open)),
        MenuItem::action("Equalizer", ClassicMenuAction::ToggleEqualizer)
            .with_mark(ItemMark::Check(state.equalizer_open)),
        MenuItem::action("Playlist Editor", ClassicMenuAction::TogglePlaylistEditor)
            .with_mark(ItemMark::Check(state.playlist_open)),
        MenuItem::separator(),
        MenuItem::submenu("Skins", skins),
        MenuItem::separator(),
        MenuItem::submenu("Options", options),
        MenuItem::submenu("Playback", playback),
        MenuItem::separator(),
        MenuItem::action("Exit", ClassicMenuAction::Exit),
    ])
}

/// The playlist editor's Add flyout.
pub fn playlist_add_menu() -> Menu<ClassicMenuAction> {
    Menu::new(vec![
        MenuItem::action("URL...", ClassicMenuAction::PlaylistAddUrl),
        MenuItem::action("Directory...", ClassicMenuAction::PlaylistAddDirectory),
        MenuItem::action("File...", ClassicMenuAction::PlaylistAddFile),
    ])
}

/// The playlist editor's Remove flyout: a Remove Misc submenu (dead-file cleanup), Remove All, Crop
/// (keep only the selection), and Remove Selected.
pub fn playlist_rem_menu() -> Menu<ClassicMenuAction> {
    let misc = Menu::new(vec![MenuItem::action(
        "Remove all dead files",
        ClassicMenuAction::PlaylistRemoveDead,
    )]);
    Menu::new(vec![
        MenuItem::submenu("Remove Misc", misc),
        MenuItem::action("Remove All", ClassicMenuAction::PlaylistRemoveAll),
        MenuItem::action("Crop", ClassicMenuAction::PlaylistCrop),
        MenuItem::action("Remove Selected", ClassicMenuAction::PlaylistRemoveSelected),
    ])
}

/// The playlist editor's Selection flyout.
pub fn playlist_sel_menu() -> Menu<ClassicMenuAction> {
    Menu::new(vec![
        MenuItem::action("Invert Selection", ClassicMenuAction::PlaylistSelectInvert),
        MenuItem::action("Select None", ClassicMenuAction::PlaylistSelectNone),
        MenuItem::action("Select All", ClassicMenuAction::PlaylistSelectAll),
    ])
}

/// The playlist editor's Misc flyout: the Sort List submenu (the full classic set of sorts plus
/// reverse and randomize) and a File Info entry (disabled until per-track info exists).
pub fn playlist_misc_menu() -> Menu<ClassicMenuAction> {
    let sort = Menu::new(vec![
        MenuItem::action("Sort list by title", ClassicMenuAction::PlaylistSortTitle),
        MenuItem::action(
            "Sort list by filename",
            ClassicMenuAction::PlaylistSortFilename,
        ),
        MenuItem::action(
            "Sort list by path and filename",
            ClassicMenuAction::PlaylistSortPath,
        ),
        MenuItem::separator(),
        MenuItem::action("Reverse list", ClassicMenuAction::PlaylistReverse),
        MenuItem::action("Randomize list", ClassicMenuAction::PlaylistRandomize),
    ]);
    Menu::new(vec![
        MenuItem::submenu("Sort List", sort),
        MenuItem::action("File Info", ClassicMenuAction::PlaylistFileInfo).with_enabled(false),
    ])
}

/// The playlist editor's List flyout: clear, save, and load the whole playlist.
pub fn playlist_list_menu() -> Menu<ClassicMenuAction> {
    Menu::new(vec![
        MenuItem::action("New List", ClassicMenuAction::PlaylistNewList),
        MenuItem::action("Save List", ClassicMenuAction::PlaylistSaveList),
        MenuItem::action("Load List", ClassicMenuAction::PlaylistLoadList),
    ])
}

/// The classic equalizer exposes exactly this many built-in preset choices.
pub const CLASSIC_EQ_PRESET_COUNT: usize = 17;

/// Error returned when the caller does not supply the complete classic preset set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetCountError {
    pub expected: usize,
    pub actual: usize,
}

impl fmt::Display for PresetCountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "expected {} equalizer presets, got {}",
            self.expected, self.actual
        )
    }
}

impl std::error::Error for PresetCountError {}

/// Construct the equalizer Presets popup. Preset names remain caller-supplied so this pure render
/// crate does not duplicate or depend on the DSP crate's canonical table.
pub fn equalizer_presets_menu<I, S>(
    preset_names: I,
) -> Result<Menu<ClassicMenuAction>, PresetCountError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let names: Vec<String> = preset_names.into_iter().map(Into::into).collect();
    if names.len() != CLASSIC_EQ_PRESET_COUNT {
        return Err(PresetCountError {
            expected: CLASSIC_EQ_PRESET_COUNT,
            actual: names.len(),
        });
    }

    let mut load_items: Vec<MenuItem<ClassicMenuAction>> = names
        .into_iter()
        .enumerate()
        .map(|(index, name)| MenuItem::action(name, ClassicMenuAction::EqualizerLoadPreset(index)))
        .collect();
    load_items.push(MenuItem::separator());
    load_items.push(MenuItem::action(
        "From EQF...",
        ClassicMenuAction::EqualizerLoadEqf,
    ));

    Ok(Menu::new(vec![
        MenuItem::submenu("Load", Menu::new(load_items)),
        MenuItem::action("Save As...", ClassicMenuAction::EqualizerSaveAs),
    ]))
}

/// Axis-aligned rectangle in the combined menu surface's coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl MenuRect {
    fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}

/// Geometry for one popup pane. `path` identifies the submenu that owns it: the root pane has an
/// empty path; a first-level flyout's path contains its parent item index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneLayout {
    pub depth: usize,
    pub path: Vec<usize>,
    pub rect: MenuRect,
    pub rows: Vec<MenuRect>,
}

/// An item under a pointer, identified by popup depth and item index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuHit {
    pub depth: usize,
    pub index: usize,
}

/// Layout one pane at the requested origin.
fn pane_layout<A>(menu: &Menu<A>, path: Vec<usize>, x: i32, y: i32) -> PaneLayout {
    let content_width = menu.items.iter().fold(0, |widest, item| {
        let label = font::text_width(&item.label) as i32;
        let shortcut = item
            .shortcut
            .as_deref()
            .map_or(0, |text| SHORTCUT_GAP + font::text_width(text) as i32);
        widest.max(label + shortcut)
    });
    let width = (LEFT_PAD + MARK_WIDTH + content_width + ARROW_WIDTH + RIGHT_PAD + 2 * BORDER)
        .max(MIN_WIDTH);
    let mut cursor_y = y + BORDER;
    let rows = menu
        .items
        .iter()
        .map(|item| {
            let height = if matches!(item.kind, MenuItemKind::Separator) {
                SEPARATOR_HEIGHT
            } else {
                ROW_HEIGHT
            };
            let rect = MenuRect {
                x: x + BORDER,
                y: cursor_y,
                width: width - 2 * BORDER,
                height,
            };
            cursor_y += height;
            rect
        })
        .collect();
    PaneLayout {
        depth: path.len(),
        path,
        rect: MenuRect {
            x,
            y,
            width,
            height: cursor_y - y + BORDER,
        },
        rows,
    }
}

/// State for one open menu chain. A selection path contains one item index per focused depth.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuInteraction {
    open: bool,
    selection: Vec<usize>,
    pressed: Option<Vec<usize>>,
}

impl MenuInteraction {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn selected_path(&self) -> &[usize] {
        &self.selection
    }

    pub fn pressed_path(&self) -> Option<&[usize]> {
        self.pressed.as_deref()
    }

    /// Open at the first enabled, non-separator root item. Empty/all-disabled menus still open but
    /// have no selection.
    pub fn open<A>(&mut self, menu: &Menu<A>) {
        self.open = true;
        self.pressed = None;
        self.selection = first_focusable(menu).into_iter().collect();
    }

    pub fn close(&mut self) {
        self.open = false;
        self.selection.clear();
        self.pressed = None;
    }

    /// Update hover focus. Moving across a separator closes deeper flyouts; leaving all panes keeps
    /// the current chain open so a pointer can travel diagonally into an existing flyout.
    pub fn pointer_move<A>(&mut self, menu: &Menu<A>, x: i32, y: i32) -> MenuOutcome<A> {
        if !self.open {
            return MenuOutcome::Unchanged;
        }
        let Some((pane, hit)) = hit_with_pane(menu, self, x, y) else {
            return MenuOutcome::Unchanged;
        };
        let Some(owner) = menu_at_path(menu, &pane.path) else {
            return MenuOutcome::Unchanged;
        };
        let Some(item) = owner.items.get(hit.index) else {
            return MenuOutcome::Unchanged;
        };
        let mut next = pane.path.clone();
        if !matches!(item.kind, MenuItemKind::Separator) {
            next.push(hit.index);
        }
        if next == self.selection {
            MenuOutcome::Unchanged
        } else {
            self.selection = next;
            self.pressed = None;
            MenuOutcome::Redraw
        }
    }

    /// Begin a click on an enabled row. Pressing outside leaves activation empty; releasing there
    /// will dismiss without firing an action.
    pub fn pointer_press<A>(&mut self, menu: &Menu<A>, x: i32, y: i32) -> MenuOutcome<A> {
        if !self.open {
            return MenuOutcome::Unchanged;
        }
        let _ = self.pointer_move(menu, x, y);
        self.pressed = hit_with_pane(menu, self, x, y).and_then(|(pane, hit)| {
            let owner = menu_at_path(menu, &pane.path)?;
            let item = owner.items.get(hit.index)?;
            if item.is_keyboard_focusable() {
                let mut path = pane.path;
                path.push(hit.index);
                Some(path)
            } else {
                None
            }
        });
        MenuOutcome::Redraw
    }

    /// Complete a click. Only press/release on the same enabled action activates. Every mismatch,
    /// including release outside all panes, cancels and dismisses the chain.
    pub fn pointer_release<A: Clone>(&mut self, menu: &Menu<A>, x: i32, y: i32) -> MenuOutcome<A> {
        if !self.open {
            return MenuOutcome::Unchanged;
        }
        let released = hit_with_pane(menu, self, x, y).map(|(pane, hit)| {
            let mut path = pane.path;
            path.push(hit.index);
            path
        });
        let pressed = self.pressed.take();
        if pressed != released || released.is_none() {
            self.close();
            return MenuOutcome::Dismissed;
        }

        let path = released.expect("checked as some above");
        match item_at_path(menu, &path) {
            Some(MenuItem {
                state: ItemState { enabled: true, .. },
                kind: MenuItemKind::Action(action),
                ..
            }) => {
                let action = action.clone();
                self.close();
                MenuOutcome::Activated(action)
            }
            Some(MenuItem {
                state: ItemState { enabled: true, .. },
                kind: MenuItemKind::Submenu(_),
                ..
            }) => {
                self.selection = path;
                MenuOutcome::Redraw
            }
            _ => {
                self.close();
                MenuOutcome::Dismissed
            }
        }
    }

    /// Apply an arrow, activation, dismissal, or first-letter navigation key.
    pub fn key<A: Clone>(&mut self, menu: &Menu<A>, key: MenuKey) -> MenuOutcome<A> {
        if !self.open {
            return MenuOutcome::Unchanged;
        }
        match key {
            MenuKey::Escape => {
                self.close();
                MenuOutcome::Dismissed
            }
            MenuKey::Up => self.move_focus(menu, -1),
            MenuKey::Down => self.move_focus(menu, 1),
            MenuKey::Home => self.focus_edge(menu, false),
            MenuKey::End => self.focus_edge(menu, true),
            MenuKey::Left => {
                if self.selection.len() > 1 {
                    self.selection.pop();
                    self.pressed = None;
                    MenuOutcome::Redraw
                } else {
                    MenuOutcome::Unchanged
                }
            }
            MenuKey::Right => self.open_selected_submenu(menu),
            MenuKey::Enter => self.activate_selected(menu),
            MenuKey::Character(ch) => self.focus_initial(menu, ch),
        }
    }

    fn move_focus<A>(&mut self, root: &Menu<A>, delta: i32) -> MenuOutcome<A> {
        let (owner_path, current) = self.current_owner();
        let Some(owner) = menu_at_path(root, owner_path) else {
            return MenuOutcome::Unchanged;
        };
        let candidates: Vec<usize> = owner
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| item.is_keyboard_focusable().then_some(i))
            .collect();
        if candidates.is_empty() {
            return MenuOutcome::Unchanged;
        }
        let position = current.and_then(|index| candidates.iter().position(|&i| i == index));
        let next_position = match (position, delta.is_negative()) {
            (Some(0), true) | (None, true) => candidates.len() - 1,
            (Some(pos), true) => pos - 1,
            (Some(pos), false) => (pos + 1) % candidates.len(),
            (None, false) => 0,
        };
        let mut next = owner_path.to_vec();
        next.push(candidates[next_position]);
        self.selection = next;
        self.pressed = None;
        MenuOutcome::Redraw
    }

    fn focus_edge<A>(&mut self, root: &Menu<A>, end: bool) -> MenuOutcome<A> {
        let (owner_path, _) = self.current_owner();
        let Some(owner) = menu_at_path(root, owner_path) else {
            return MenuOutcome::Unchanged;
        };
        let candidate = if end {
            owner
                .items
                .iter()
                .enumerate()
                .rev()
                .find_map(|(i, item)| item.is_keyboard_focusable().then_some(i))
        } else {
            first_focusable(owner)
        };
        let Some(candidate) = candidate else {
            return MenuOutcome::Unchanged;
        };
        self.selection = owner_path
            .iter()
            .copied()
            .chain(std::iter::once(candidate))
            .collect();
        self.pressed = None;
        MenuOutcome::Redraw
    }

    fn open_selected_submenu<A>(&mut self, root: &Menu<A>) -> MenuOutcome<A> {
        let Some(MenuItem {
            state: ItemState { enabled: true, .. },
            kind: MenuItemKind::Submenu(child),
            ..
        }) = item_at_path(root, &self.selection)
        else {
            return MenuOutcome::Unchanged;
        };
        let Some(first) = first_focusable(child) else {
            return MenuOutcome::Unchanged;
        };
        self.selection.push(first);
        self.pressed = None;
        MenuOutcome::Redraw
    }

    fn activate_selected<A: Clone>(&mut self, root: &Menu<A>) -> MenuOutcome<A> {
        match item_at_path(root, &self.selection) {
            Some(MenuItem {
                state: ItemState { enabled: true, .. },
                kind: MenuItemKind::Action(action),
                ..
            }) => {
                let action = action.clone();
                self.close();
                MenuOutcome::Activated(action)
            }
            Some(MenuItem {
                state: ItemState { enabled: true, .. },
                kind: MenuItemKind::Submenu(_),
                ..
            }) => self.open_selected_submenu(root),
            _ => MenuOutcome::Unchanged,
        }
    }

    fn focus_initial<A>(&mut self, root: &Menu<A>, needle: char) -> MenuOutcome<A> {
        let (owner_path, current) = self.current_owner();
        let Some(owner) = menu_at_path(root, owner_path) else {
            return MenuOutcome::Unchanged;
        };
        let needle = needle.to_ascii_lowercase();
        let start = current
            .and_then(|i| owner.items.get(i).map(|_| i + 1))
            .unwrap_or(0);
        let found = (0..owner.items.len()).find_map(|offset| {
            let index = (start + offset) % owner.items.len();
            let item = &owner.items[index];
            let matches = item
                .label
                .chars()
                .next()
                .is_some_and(|ch| ch.to_ascii_lowercase() == needle);
            (item.is_keyboard_focusable() && matches).then_some(index)
        });
        let Some(found) = found else {
            return MenuOutcome::Unchanged;
        };
        self.selection = owner_path
            .iter()
            .copied()
            .chain(std::iter::once(found))
            .collect();
        self.pressed = None;
        MenuOutcome::Redraw
    }

    fn current_owner(&self) -> (&[usize], Option<usize>) {
        self.selection
            .split_last()
            .map_or((&[][..], None), |(last, owner)| (owner, Some(*last)))
    }
}

/// Keys understood by the pure popup interaction model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKey {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Enter,
    Escape,
    /// Move focus to the next enabled item beginning with this character.
    Character(char),
}

/// Observable result of one input transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuOutcome<A> {
    Unchanged,
    Redraw,
    Activated(A),
    Dismissed,
}

fn first_focusable<A>(menu: &Menu<A>) -> Option<usize> {
    menu.items.iter().position(MenuItem::is_keyboard_focusable)
}

fn menu_at_path<'a, A>(root: &'a Menu<A>, path: &[usize]) -> Option<&'a Menu<A>> {
    let mut menu = root;
    for &index in path {
        let item = menu.items.get(index)?;
        let MenuItemKind::Submenu(child) = &item.kind else {
            return None;
        };
        if !item.state.enabled {
            return None;
        }
        menu = child;
    }
    Some(menu)
}

fn item_at_path<'a, A>(root: &'a Menu<A>, path: &[usize]) -> Option<&'a MenuItem<A>> {
    let (&last, owner) = path.split_last()?;
    menu_at_path(root, owner)?.items.get(last)
}

/// Lay out all currently-open panes. Flyouts open to the right with a two-pixel overlap and align
/// their top edge to the selected parent row.
pub fn layout_stack<A>(menu: &Menu<A>, state: &MenuInteraction) -> Vec<PaneLayout> {
    if !state.open {
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut owner = menu;
    let mut path = Vec::new();
    let mut x = 0;
    let mut y = 0;
    loop {
        let pane = pane_layout(owner, path.clone(), x, y);
        let depth = path.len();
        let Some(&selected) = state.selection.get(depth) else {
            result.push(pane);
            break;
        };
        let Some(item) = owner.items.get(selected) else {
            result.push(pane);
            break;
        };
        let Some(row) = pane.rows.get(selected).copied() else {
            result.push(pane);
            break;
        };
        let MenuItemKind::Submenu(child) = &item.kind else {
            result.push(pane);
            break;
        };
        if !item.state.enabled {
            result.push(pane);
            break;
        }
        x = pane.rect.x + pane.rect.width - FLYOUT_OVERLAP;
        y = row.y;
        result.push(pane);
        path.push(selected);
        owner = child;
    }
    result
}

/// Return the deepest visible item under a surface-local pointer.
pub fn hit_test<A>(menu: &Menu<A>, state: &MenuInteraction, x: i32, y: i32) -> Option<MenuHit> {
    hit_with_pane(menu, state, x, y).map(|(_, hit)| hit)
}

fn hit_with_pane<A>(
    menu: &Menu<A>,
    state: &MenuInteraction,
    x: i32,
    y: i32,
) -> Option<(PaneLayout, MenuHit)> {
    layout_stack(menu, state)
        .into_iter()
        .rev()
        .find_map(|pane| {
            let index = pane.rows.iter().position(|row| row.contains(x, y))?;
            let depth = pane.depth;
            Some((pane, MenuHit { depth, index }))
        })
}

/// How a popup menu is painted. `classic()` is the original bitmap chrome (also used by tests and
/// any headless build with no system font); `adwaita(...)` paints a native GNOME popover with the
/// system UI font. Layout and hit-testing are identical either way, so only the pixels change and a
/// menu stays clickable in exactly the same places.
pub struct MenuTheme<'a> {
    palette: Palette,
    font: Option<&'a UiFont>,
    font_px: f32,
}

impl Default for MenuTheme<'_> {
    fn default() -> Self {
        Self::classic()
    }
}

impl<'a> MenuTheme<'a> {
    /// The classic bitmap-font menu chrome.
    pub fn classic() -> Self {
        Self {
            palette: Palette::light(),
            font: None,
            font_px: 0.0,
        }
    }

    /// A native Adwaita popover in `palette`, drawn with the system UI `font`.
    pub fn adwaita(palette: Palette, font: &'a UiFont) -> Self {
        Self {
            palette,
            font: Some(font),
            font_px: 12.5,
        }
    }
}

/// Render every open pane into one RGBA framebuffer. Pixels belonging to each popup are opaque;
/// unused corners of the combined bounding rectangle remain transparent for later popup splitting.
pub fn compose<A>(menu: &Menu<A>, state: &MenuInteraction, theme: &MenuTheme) -> Framebuffer {
    let panes = layout_stack(menu, state);
    let width = panes
        .iter()
        .map(|pane| pane.rect.x + pane.rect.width)
        .max()
        .unwrap_or(0)
        .max(0) as u32;
    let height = panes
        .iter()
        .map(|pane| pane.rect.y + pane.rect.height)
        .max()
        .unwrap_or(0)
        .max(0) as u32;
    let mut fb = Framebuffer::new(width, height);
    for pane in &panes {
        draw_pane(&mut fb, menu, state, pane, theme);
    }
    fb
}

fn draw_pane<A>(
    fb: &mut Framebuffer,
    root: &Menu<A>,
    state: &MenuInteraction,
    pane: &PaneLayout,
    theme: &MenuTheme,
) {
    let Some(menu) = menu_at_path(root, &pane.path) else {
        return;
    };
    if let Some(font) = theme.font {
        draw_pane_adwaita(fb, menu, state, pane, &theme.palette, font, theme.font_px);
        return;
    }
    fill(fb, pane.rect, FACE);
    line_h(fb, pane.rect.x, pane.rect.y, pane.rect.width, LIGHT);
    line_v(fb, pane.rect.x, pane.rect.y, pane.rect.height, LIGHT);
    line_h(
        fb,
        pane.rect.x,
        pane.rect.y + pane.rect.height - 1,
        pane.rect.width,
        DARK,
    );
    line_v(
        fb,
        pane.rect.x + pane.rect.width - 1,
        pane.rect.y,
        pane.rect.height,
        DARK,
    );
    line_h(
        fb,
        pane.rect.x + 1,
        pane.rect.y + pane.rect.height - 2,
        pane.rect.width - 2,
        SHADOW,
    );
    line_v(
        fb,
        pane.rect.x + pane.rect.width - 2,
        pane.rect.y + 1,
        pane.rect.height - 2,
        SHADOW,
    );

    let selected = state.selection.get(pane.depth).copied();
    for (index, (item, row)) in menu.items.iter().zip(&pane.rows).enumerate() {
        if matches!(item.kind, MenuItemKind::Separator) {
            let y = row.y + row.height / 2;
            line_h(fb, row.x + 4, y, row.width - 8, SHADOW);
            line_h(fb, row.x + 4, y + 1, row.width - 8, LIGHT);
            continue;
        }

        let is_selected = selected == Some(index);
        let mut full_path = pane.path.clone();
        full_path.push(index);
        let is_pressed = state.pressed.as_ref() == Some(&full_path);
        if is_selected {
            fill(
                fb,
                *row,
                if is_pressed {
                    SELECTED_PRESSED
                } else {
                    SELECTED
                },
            );
        }
        let color = if !item.state.enabled {
            DISABLED_TEXT
        } else if is_selected {
            SELECTED_TEXT
        } else {
            TEXT
        };
        let text_y = row.y + (row.height - font::GLYPH_H as i32) / 2;
        draw_mark(
            fb,
            row.x + LEFT_PAD,
            row.y + row.height / 2,
            item.state.mark,
            color,
        );
        draw_text(
            fb,
            row.x + LEFT_PAD + MARK_WIDTH,
            text_y,
            &item.label,
            color,
        );
        if let Some(shortcut) = &item.shortcut {
            let x = row.x + row.width - RIGHT_PAD - ARROW_WIDTH - font::text_width(shortcut) as i32;
            draw_text(fb, x, text_y, shortcut, color);
        }
        if matches!(item.kind, MenuItemKind::Submenu(_)) {
            draw_arrow(
                fb,
                row.x + row.width - RIGHT_PAD - 5,
                row.y + row.height / 2,
                color,
            );
        }
    }
}

/// Paint one popup pane as a native GNOME popover: a rounded, bordered background, an inset accent
/// pill on the focused row, system-font labels, dimmed right-aligned shortcuts, and a chevron for
/// submenus. Layout (row rects) is shared with the classic path, so hit-testing is unchanged.
fn draw_pane_adwaita<A>(
    fb: &mut Framebuffer,
    menu: &Menu<A>,
    state: &MenuInteraction,
    pane: &PaneLayout,
    p: &Palette,
    font: &UiFont,
    px: f32,
) {
    let r = pane.rect;
    adwaita::fill_rounded_rect(fb, r.x, r.y, r.width, r.height, adwaita::POPOVER_RADIUS, p.popover_bg);
    adwaita::stroke_rounded_rect(
        fb,
        r.x,
        r.y,
        r.width,
        r.height,
        adwaita::POPOVER_RADIUS,
        1,
        p.border,
    );

    let selected = state.selection.get(pane.depth).copied();
    for (index, (item, row)) in menu.items.iter().zip(&pane.rows).enumerate() {
        if matches!(item.kind, MenuItemKind::Separator) {
            adwaita::draw_separator(fb, row.x + 8, row.y + row.height / 2, row.width - 16, p);
            continue;
        }
        let is_selected = selected == Some(index);
        if is_selected {
            // libadwaita highlights the focused row as an inset rounded pill.
            adwaita::fill_rounded_rect(
                fb,
                row.x + 4,
                row.y + 2,
                row.width - 8,
                row.height - 4,
                6,
                p.selected_row,
            );
        }
        let text_color = if !item.state.enabled {
            p.dim_fg
        } else if is_selected {
            p.selected_fg
        } else {
            p.fg
        };
        let rgb = [text_color[0], text_color[1], text_color[2]];
        draw_mark(fb, row.x + LEFT_PAD, row.y + row.height / 2, item.state.mark, rgb);
        // Baseline centered in the row for the system font (which positions glyphs by their baseline,
        // unlike the top-left bitmap font).
        let baseline = row.y + row.height / 2 + (px * 0.34) as i32;
        font.draw_text(
            fb,
            row.x + LEFT_PAD + MARK_WIDTH,
            baseline,
            &item.label,
            px,
            text_color,
        );
        if let Some(shortcut) = &item.shortcut {
            let sc = if is_selected { text_color } else { p.dim_fg };
            let width = font.text_width(shortcut, px).ceil() as i32;
            let x = row.x + row.width - RIGHT_PAD - ARROW_WIDTH - width;
            font.draw_text(fb, x, baseline, shortcut, px, sc);
        }
        if matches!(item.kind, MenuItemKind::Submenu(_)) {
            draw_arrow(fb, row.x + row.width - RIGHT_PAD - 5, row.y + row.height / 2, rgb);
        }
    }
}

fn draw_mark(fb: &mut Framebuffer, x: i32, center_y: i32, mark: ItemMark, color: [u8; 3]) {
    match mark {
        ItemMark::None => {}
        ItemMark::Check(false) => {
            outline(
                fb,
                MenuRect {
                    x,
                    y: center_y - 4,
                    width: 9,
                    height: 9,
                },
                color,
            );
        }
        ItemMark::Check(true) => {
            outline(
                fb,
                MenuRect {
                    x,
                    y: center_y - 4,
                    width: 9,
                    height: 9,
                },
                color,
            );
            pixel(fb, x + 2, center_y, color);
            pixel(fb, x + 3, center_y + 1, color);
            pixel(fb, x + 4, center_y + 2, color);
            pixel(fb, x + 5, center_y + 1, color);
            pixel(fb, x + 6, center_y, color);
            pixel(fb, x + 7, center_y - 1, color);
        }
        ItemMark::Radio(on) => {
            pixel(fb, x + 3, center_y - 4, color);
            pixel(fb, x + 4, center_y - 4, color);
            pixel(fb, x + 1, center_y - 2, color);
            pixel(fb, x + 6, center_y - 2, color);
            pixel(fb, x, center_y, color);
            pixel(fb, x + 7, center_y, color);
            pixel(fb, x + 1, center_y + 2, color);
            pixel(fb, x + 6, center_y + 2, color);
            pixel(fb, x + 3, center_y + 4, color);
            pixel(fb, x + 4, center_y + 4, color);
            if on {
                fill(
                    fb,
                    MenuRect {
                        x: x + 3,
                        y: center_y - 1,
                        width: 2,
                        height: 3,
                    },
                    color,
                );
            }
        }
    }
}

fn draw_arrow(fb: &mut Framebuffer, x: i32, center_y: i32, color: [u8; 3]) {
    for offset in -3_i32..=3 {
        for dx in 0..=(3 - offset.abs()) {
            pixel(fb, x + dx, center_y + offset, color);
        }
    }
}

fn outline(fb: &mut Framebuffer, rect: MenuRect, color: [u8; 3]) {
    line_h(fb, rect.x, rect.y, rect.width, color);
    line_h(fb, rect.x, rect.y + rect.height - 1, rect.width, color);
    line_v(fb, rect.x, rect.y, rect.height, color);
    line_v(fb, rect.x + rect.width - 1, rect.y, rect.height, color);
}

fn fill(fb: &mut Framebuffer, rect: MenuRect, color: [u8; 3]) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn labels<A>(menu: &Menu<A>) -> Vec<&str> {
        menu.items
            .iter()
            .flat_map(|item| {
                let own = (!matches!(item.kind, MenuItemKind::Separator))
                    .then_some(item.label.as_str())
                    .into_iter();
                let children = match &item.kind {
                    MenuItemKind::Submenu(child) => labels(child),
                    _ => Vec::new(),
                };
                own.chain(children).collect::<Vec<_>>()
            })
            .collect()
    }

    fn submenu<'a, A>(menu: &'a Menu<A>, label: &str) -> &'a Menu<A> {
        menu.items
            .iter()
            .find_map(|item| match &item.kind {
                MenuItemKind::Submenu(child) if item.label == label => Some(child),
                _ => None,
            })
            .expect("submenu exists")
    }

    #[test]
    fn main_model_has_only_the_supported_hierarchy() {
        let menu = main_menu(MainMenuState {
            equalizer_open: true,
            playlist_open: true,
            time_display: TimeDisplay::Remaining,
            double_size: true,
            repeat: true,
            shuffle: true,
            ..MainMenuState::default()
        });
        let root: Vec<&str> = menu
            .items
            .iter()
            .filter(|item| !matches!(item.kind, MenuItemKind::Separator))
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(
            root,
            [
                "Play",
                "Main Window",
                "Equalizer",
                "Playlist Editor",
                "Skins",
                "Options",
                "Playback",
                "Exit"
            ]
        );
        assert_eq!(
            labels(submenu(&menu, "Skins")),
            ["Load Skin...", "<Base Skin>"]
        );
        let options = submenu(&menu, "Options");
        assert_eq!(
            labels(options),
            [
                "Preferences...",
                "Skins",
                "Load Skin...",
                "<Base Skin>",
                "Time elapsed",
                "Time remaining",
                "Double Size",
                "Repeat",
                "Shuffle"
            ]
        );
        assert_eq!(options.items[0].shortcut.as_deref(), Some("Ctrl+P"));
        assert_eq!(options.items[4].state.mark, ItemMark::Radio(false));
        assert_eq!(options.items[5].state.mark, ItemMark::Radio(true));
        assert_eq!(options.items[6].state.mark, ItemMark::Check(true));
        assert_eq!(options.items[8].state.mark, ItemMark::Check(true));
        assert_eq!(options.items[9].state.mark, ItemMark::Check(true));
        assert_eq!(
            options.items[4].shortcut.as_deref(),
            Some("(Ctrl+T toggles)")
        );
        assert_eq!(options.items[6].shortcut.as_deref(), Some("Ctrl+D"));

        let all = labels(&menu).join("|").to_ascii_lowercase();
        for omitted in [
            "webamp",
            "milkdrop",
            "video",
            "cd ripping",
            "setup",
            "plugin",
        ] {
            assert!(!all.contains(omitted), "unexpected menu entry: {omitted}");
        }
    }

    #[test]
    fn playback_and_playlist_add_models_are_complete_and_ordered() {
        let menu = main_menu(MainMenuState::default());
        assert_eq!(
            labels(submenu(&menu, "Playback")),
            [
                "Previous",
                "Play",
                "Pause",
                "Stop",
                "Next",
                "Back 5 seconds",
                "Fwd 5 seconds",
                "10 tracks back",
                "10 tracks fwd"
            ]
        );
        assert_eq!(labels(submenu(&menu, "Play")), ["File..."]);
        assert_eq!(
            labels(&playlist_add_menu()),
            ["URL...", "Directory...", "File..."]
        );
    }

    #[test]
    fn playlist_cluster_menus_match_the_classic_set() {
        assert_eq!(
            labels(&playlist_rem_menu()),
            [
                "Remove Misc",
                "Remove all dead files",
                "Remove All",
                "Crop",
                "Remove Selected",
            ]
        );
        assert_eq!(
            labels(&playlist_sel_menu()),
            ["Invert Selection", "Select None", "Select All"]
        );
        assert_eq!(
            labels(&playlist_misc_menu()),
            [
                "Sort List",
                "Sort list by title",
                "Sort list by filename",
                "Sort list by path and filename",
                "Reverse list",
                "Randomize list",
                "File Info",
            ]
        );
        // File Info is present but disabled until per-track info exists.
        let misc = playlist_misc_menu();
        let file_info = misc.items.iter().find(|i| i.label == "File Info").unwrap();
        assert!(!file_info.state.enabled);
        assert_eq!(
            labels(&playlist_list_menu()),
            ["New List", "Save List", "Load List"]
        );
    }

    #[test]
    fn equalizer_presets_are_caller_supplied_and_counted() {
        let names: Vec<String> = (0..CLASSIC_EQ_PRESET_COUNT)
            .map(|index| format!("Preset {index}"))
            .collect();
        let menu = equalizer_presets_menu(names.clone()).unwrap();
        let load = submenu(&menu, "Load");
        let loaded_names: Vec<&str> = load
            .items
            .iter()
            .take(CLASSIC_EQ_PRESET_COUNT)
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(
            loaded_names,
            names.iter().map(String::as_str).collect::<Vec<_>>()
        );
        assert_eq!(load.items[CLASSIC_EQ_PRESET_COUNT + 1].label, "From EQF...");
        assert_eq!(menu.items[1].label, "Save As...");

        assert_eq!(
            equalizer_presets_menu(["one", "two"]).unwrap_err(),
            PresetCountError {
                expected: CLASSIC_EQ_PRESET_COUNT,
                actual: 2
            }
        );
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestAction {
        One,
        Two,
        Child,
    }

    fn interaction_menu() -> Menu<TestAction> {
        Menu::new(vec![
            MenuItem::action("One", TestAction::One),
            MenuItem::separator(),
            MenuItem::action("Disabled", TestAction::Two).with_enabled(false),
            MenuItem::submenu(
                "More",
                Menu::new(vec![
                    MenuItem::separator(),
                    MenuItem::action("Child", TestAction::Child),
                ]),
            ),
            MenuItem::action("Two", TestAction::Two),
        ])
    }

    #[test]
    fn keyboard_skips_disabled_and_separators_wraps_and_activates() {
        let menu = interaction_menu();
        let mut state = MenuInteraction::default();
        state.open(&menu);
        assert_eq!(state.selected_path(), [0]);
        assert_eq!(state.key(&menu, MenuKey::Down), MenuOutcome::Redraw);
        assert_eq!(
            state.selected_path(),
            [3],
            "separator and disabled item skipped"
        );
        assert_eq!(state.key(&menu, MenuKey::Right), MenuOutcome::Redraw);
        assert_eq!(state.selected_path(), [3, 1], "child separator skipped");
        assert_eq!(
            state.key(&menu, MenuKey::Enter),
            MenuOutcome::Activated(TestAction::Child)
        );
        assert!(!state.is_open());

        state.open(&menu);
        assert_eq!(state.key(&menu, MenuKey::Up), MenuOutcome::Redraw);
        assert_eq!(state.selected_path(), [4], "up from first wraps to last");
    }

    #[test]
    fn keyboard_left_escape_home_end_and_initial_navigation_work() {
        let menu = interaction_menu();
        let mut state = MenuInteraction::default();
        state.open(&menu);
        state.key(&menu, MenuKey::End);
        assert_eq!(state.selected_path(), [4]);
        state.key(&menu, MenuKey::Home);
        assert_eq!(state.selected_path(), [0]);
        state.key(&menu, MenuKey::Character('m'));
        assert_eq!(state.selected_path(), [3]);
        state.key(&menu, MenuKey::Right);
        assert_eq!(state.selected_path(), [3, 1]);
        state.key(&menu, MenuKey::Left);
        assert_eq!(state.selected_path(), [3]);
        assert_eq!(state.key(&menu, MenuKey::Escape), MenuOutcome::Dismissed);
        assert!(!state.is_open());
    }

    #[test]
    fn flyout_layout_and_hit_testing_are_deterministic() {
        let menu = interaction_menu();
        let mut state = MenuInteraction::default();
        state.open(&menu);
        state.key(&menu, MenuKey::Character('m'));
        let first = layout_stack(&menu, &state);
        let second = layout_stack(&menu, &state);
        assert_eq!(first, second);
        assert_eq!(first.len(), 2);
        assert_eq!(first[1].path, [3]);
        assert_eq!(first[1].rect.x, first[0].rect.width - FLYOUT_OVERLAP);
        assert_eq!(first[1].rect.y, first[0].rows[3].y);
        let child_row = first[1].rows[1];
        assert_eq!(
            hit_test(&menu, &state, child_row.x + 1, child_row.y + 1),
            Some(MenuHit { depth: 1, index: 1 })
        );
    }

    #[test]
    fn pointer_activates_only_same_press_and_release() {
        let menu = interaction_menu();
        let mut state = MenuInteraction::default();
        state.open(&menu);
        let pane = &layout_stack(&menu, &state)[0];
        let one = pane.rows[0];
        state.pointer_press(&menu, one.x + 1, one.y + 1);
        assert_eq!(
            state.pointer_release(&menu, one.x + 1, one.y + 1),
            MenuOutcome::Activated(TestAction::One)
        );

        state.open(&menu);
        let pane = &layout_stack(&menu, &state)[0];
        let one = pane.rows[0];
        let two = pane.rows[4];
        state.pointer_press(&menu, one.x + 1, one.y + 1);
        assert_eq!(
            state.pointer_release(&menu, two.x + 1, two.y + 1),
            MenuOutcome::Dismissed,
            "releasing over another action must not activate it"
        );
    }

    #[test]
    fn release_outside_cancels_and_dismisses() {
        let menu = interaction_menu();
        let mut state = MenuInteraction::default();
        state.open(&menu);
        let row = layout_stack(&menu, &state)[0].rows[0];
        state.pointer_press(&menu, row.x + 1, row.y + 1);
        assert_eq!(
            state.pointer_release(&menu, -20, -20),
            MenuOutcome::Dismissed
        );
        assert!(!state.is_open());
    }

    #[test]
    fn rendered_panes_are_opaque_with_authored_chrome() {
        let menu = main_menu(MainMenuState::default());
        let mut state = MenuInteraction::default();
        state.open(&menu);
        state.key(&menu, MenuKey::Character('o'));
        let panes = layout_stack(&menu, &state);
        let fb = compose(&menu, &state, &MenuTheme::classic());
        assert!(fb.width > panes[0].rect.width as u32);
        assert!(fb.height >= panes[0].rect.height as u32);
        for pane in panes {
            for y in pane.rect.y..pane.rect.y + pane.rect.height {
                for x in pane.rect.x..pane.rect.x + pane.rect.width {
                    let alpha = fb.rgba[((y as u32 * fb.width + x as u32) * 4 + 3) as usize];
                    assert_eq!(alpha, 255, "popup pixel at ({x}, {y}) is transparent");
                }
            }
        }
        assert_eq!(&fb.rgba[..3], &LIGHT, "authored light outer edge");
    }

    #[test]
    fn closed_and_empty_menus_are_safe() {
        let menu: Menu<TestAction> = Menu::default();
        let mut state = MenuInteraction::default();
        let fb = compose(&menu, &state, &MenuTheme::classic());
        assert_eq!((fb.width, fb.height), (0, 0));
        state.open(&menu);
        assert!(state.selected_path().is_empty());
        assert_eq!(state.key(&menu, MenuKey::Down), MenuOutcome::Unchanged);
        let fb = compose(&menu, &state, &MenuTheme::classic());
        assert_eq!(fb.width, MIN_WIDTH as u32);
        assert_eq!(fb.height, (2 * BORDER) as u32);
    }

    #[test]
    fn adwaita_theme_paints_a_rounded_popover_that_differs_by_palette() {
        // Skip on a host with no system UI font; the classic path still covers rendering there.
        let Some(font) = crate::adwaita::UiFont::load_system() else {
            return;
        };
        let menu = interaction_menu();
        let mut state = MenuInteraction::default();
        state.open(&menu);
        let light = compose(&menu, &state, &MenuTheme::adwaita(Palette::light(), &font));
        let dark = compose(&menu, &state, &MenuTheme::adwaita(Palette::dark(), &font));

        // The theme only repaints; layout is shared, so the surface size is identical.
        assert_eq!((light.width, light.height), (dark.width, dark.height));
        // Rounded corners leave the extreme corner transparent.
        assert_eq!(light.rgba[3], 0, "rounded top-left corner is transparent");
        // A popover-background pixel above the first row's selection pill is opaque and its color
        // differs between the light and dark palettes.
        let cx = (light.width / 2) as usize;
        let offset = (3 * light.width as usize + cx) * 4;
        assert_eq!(light.rgba[offset + 3], 255, "popover interior is opaque");
        assert_ne!(
            &light.rgba[offset..offset + 3],
            &dark.rgba[offset..offset + 3],
            "light and dark popovers use different backgrounds"
        );
    }
}
