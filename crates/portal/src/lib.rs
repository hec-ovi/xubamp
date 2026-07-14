//! XDG desktop portal file chooser support for audio, equalizer, and skin files.
//!
//! The portal is used instead of a toolkit-specific chooser so the same code works on the host and
//! in a future sandboxed package. Dialog calls are asynchronous. Matching blocking helpers are
//! provided for use on a dedicated worker thread; they must not run on the Wayland event thread.
//!
//! This follows version 4 of the official
//! [`org.freedesktop.portal.FileChooser`](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.FileChooser.html)
//! contract. File filters are only hints under that contract, so every returned URI and file type
//! is validated here before it reaches the player.

use std::ffi::{c_ulong, OsString};
use std::fmt;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use ashpd::desktop::file_chooser::{FileFilter, SelectedFiles};
use ashpd::desktop::ResponseError;
use ashpd::{Error as AshpdError, PortalError, WindowIdentifier};

/// The result of a dialog interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogResult<T> {
    /// The user accepted the dialog and the returned value passed validation.
    Selected(T),
    /// The user dismissed or cancelled the dialog.
    Cancelled,
}

/// A parent window identifier accepted by XDG desktop portals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentWindow(ParentWindowKind);

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParentWindowKind {
    X11(c_ulong),
    Wayland(String),
}

impl ParentWindow {
    /// Construct a parent from an X11 window ID.
    pub const fn x11(xid: c_ulong) -> Self {
        Self(ParentWindowKind::X11(xid))
    }

    /// Construct a parent from an xdg-foreign exported Wayland surface handle.
    ///
    /// The caller must keep the export alive until the chooser returns.
    pub fn wayland(handle: impl Into<String>) -> Result<Self, Error> {
        let handle = handle.into();
        if handle.is_empty() || handle.chars().any(char::is_control) {
            return Err(Error::InvalidParentHandle);
        }
        Ok(Self(ParentWindowKind::Wayland(handle)))
    }

    fn to_ashpd(&self) -> WindowIdentifier {
        match &self.0 {
            ParentWindowKind::X11(xid) => WindowIdentifier::from_xid(*xid),
            ParentWindowKind::Wayland(handle) => {
                WindowIdentifier::from_xdg_foreign_exported(handle.clone())
            }
        }
    }
}

/// A reusable portal chooser, optionally attached to an application window.
#[derive(Debug, Clone, Default)]
pub struct FileChooser {
    parent: Option<ParentWindow>,
}

impl FileChooser {
    /// Create an unparented chooser.
    pub const fn new() -> Self {
        Self { parent: None }
    }

    /// Create a chooser whose dialogs are modal to `parent`.
    pub const fn with_parent(parent: ParentWindow) -> Self {
        Self {
            parent: Some(parent),
        }
    }

    /// Ask for one or more supported audio files.
    pub async fn open_audio_files(
        &self,
        current_folder: Option<&Path>,
    ) -> Result<DialogResult<Vec<PathBuf>>, Error> {
        let request = SelectedFiles::open_file()
            .title("Add files to playlist")
            .accept_label("Add")
            .modal(true)
            .multiple(true)
            .directory(false)
            .filter(audio_filter());
        let request = self.configure_open_request(request, current_folder)?;
        map_selected(request.send().await.and_then(|request| request.response()))?
            .map_selected(validate_audio_paths)
    }

    /// Blocking form of [`Self::open_audio_files`] for a dedicated worker thread.
    pub fn open_audio_files_blocking(
        &self,
        current_folder: Option<&Path>,
    ) -> Result<DialogResult<Vec<PathBuf>>, Error> {
        async_io::block_on(self.open_audio_files(current_folder))
    }

    /// Ask for one directory to scan for supported audio files.
    pub async fn open_directory(
        &self,
        current_folder: Option<&Path>,
    ) -> Result<DialogResult<PathBuf>, Error> {
        let request = SelectedFiles::open_file()
            .title("Add directory to playlist")
            .accept_label("Add")
            .modal(true)
            .multiple(false)
            .directory(true);
        let request = self.configure_open_request(request, current_folder)?;
        map_selected(request.send().await.and_then(|request| request.response()))?
            .map_selected(expect_one_path)
    }

    /// Blocking form of [`Self::open_directory`] for a dedicated worker thread.
    pub fn open_directory_blocking(
        &self,
        current_folder: Option<&Path>,
    ) -> Result<DialogResult<PathBuf>, Error> {
        async_io::block_on(self.open_directory(current_folder))
    }

    /// Ask for one Winamp EQF equalizer preset to load.
    pub async fn open_eqf_file(
        &self,
        current_folder: Option<&Path>,
    ) -> Result<DialogResult<PathBuf>, Error> {
        let request = SelectedFiles::open_file()
            .title("Load equalizer preset")
            .accept_label("Open")
            .modal(true)
            .multiple(false)
            .directory(false)
            .filter(eqf_filter());
        let request = self.configure_open_request(request, current_folder)?;
        map_selected(request.send().await.and_then(|request| request.response()))?
            .map_selected(expect_one_eqf_path)
    }

    /// Blocking form of [`Self::open_eqf_file`] for a dedicated worker thread.
    pub fn open_eqf_file_blocking(
        &self,
        current_folder: Option<&Path>,
    ) -> Result<DialogResult<PathBuf>, Error> {
        async_io::block_on(self.open_eqf_file(current_folder))
    }

    /// Ask for one Winamp skin archive to load.
    ///
    /// Portal filters are advisory, so the accepted result is also validated as one local `.wsz`
    /// or `.zip` path before it reaches the skin loader.
    pub async fn open_skin_archive(
        &self,
        current_folder: Option<&Path>,
    ) -> Result<DialogResult<PathBuf>, Error> {
        let request = SelectedFiles::open_file()
            .title("Load skin")
            .accept_label("Open")
            .modal(true)
            .multiple(false)
            .directory(false)
            .filter(skin_archive_filter());
        let request = self.configure_open_request(request, current_folder)?;
        map_selected(request.send().await.and_then(|request| request.response()))?
            .map_selected(expect_one_skin_archive_path)
    }

    /// Blocking form of [`Self::open_skin_archive`] for a dedicated worker thread.
    pub fn open_skin_archive_blocking(
        &self,
        current_folder: Option<&Path>,
    ) -> Result<DialogResult<PathBuf>, Error> {
        async_io::block_on(self.open_skin_archive(current_folder))
    }

    /// Ask for a destination for one Winamp EQF equalizer preset.
    ///
    /// The returned path always has an `.eqf` extension. A missing extension is appended; a
    /// different extension is rejected because portal filters are advisory.
    pub async fn save_eqf_file(
        &self,
        suggested_name: &str,
        current_folder: Option<&Path>,
    ) -> Result<DialogResult<PathBuf>, Error> {
        let suggested_name = suggested_eqf_name(suggested_name);
        let mut request = SelectedFiles::save_file()
            .title("Save equalizer preset")
            .accept_label("Save")
            .modal(true)
            .current_name(suggested_name.as_str())
            .filter(eqf_filter());
        if let Some(parent) = &self.parent {
            request = request.identifier(parent.to_ashpd());
        }
        if let Some(folder) = current_folder {
            request = request.current_folder(folder).map_err(Error::Portal)?;
        }
        map_selected(request.send().await.and_then(|request| request.response()))?
            .map_selected(expect_one_eqf_save_path)
    }

    /// Blocking form of [`Self::save_eqf_file`] for a dedicated worker thread.
    pub fn save_eqf_file_blocking(
        &self,
        suggested_name: &str,
        current_folder: Option<&Path>,
    ) -> Result<DialogResult<PathBuf>, Error> {
        async_io::block_on(self.save_eqf_file(suggested_name, current_folder))
    }

    /// Ask for one `.m3u`/`.m3u8`/`.pls` playlist file to load. The chosen path is validated to have
    /// a playlist extension (portal filters are only advisory).
    pub async fn open_playlist_file(
        &self,
        current_folder: Option<&Path>,
    ) -> Result<DialogResult<PathBuf>, Error> {
        let request = SelectedFiles::open_file()
            .title("Load playlist")
            .accept_label("Open")
            .modal(true)
            .multiple(false)
            .directory(false)
            .filter(playlist_filter());
        let request = self.configure_open_request(request, current_folder)?;
        map_selected(request.send().await.and_then(|request| request.response()))?
            .map_selected(expect_one_playlist_path)
    }

    /// Blocking form of [`Self::open_playlist_file`] for a dedicated worker thread.
    pub fn open_playlist_file_blocking(
        &self,
        current_folder: Option<&Path>,
    ) -> Result<DialogResult<PathBuf>, Error> {
        async_io::block_on(self.open_playlist_file(current_folder))
    }

    /// Ask for a destination to save a playlist. A path with no extension gets `.m3u`; `.m3u8` and
    /// `.pls` are accepted as-is, and any other extension is rejected.
    pub async fn save_playlist_file(
        &self,
        suggested_name: &str,
        current_folder: Option<&Path>,
    ) -> Result<DialogResult<PathBuf>, Error> {
        let suggested_name = suggested_playlist_name(suggested_name);
        let mut request = SelectedFiles::save_file()
            .title("Save playlist")
            .accept_label("Save")
            .modal(true)
            .current_name(suggested_name.as_str())
            .filter(playlist_filter());
        if let Some(parent) = &self.parent {
            request = request.identifier(parent.to_ashpd());
        }
        if let Some(folder) = current_folder {
            request = request.current_folder(folder).map_err(Error::Portal)?;
        }
        map_selected(request.send().await.and_then(|request| request.response()))?
            .map_selected(expect_one_playlist_save_path)
    }

    /// Blocking form of [`Self::save_playlist_file`] for a dedicated worker thread.
    pub fn save_playlist_file_blocking(
        &self,
        suggested_name: &str,
        current_folder: Option<&Path>,
    ) -> Result<DialogResult<PathBuf>, Error> {
        async_io::block_on(self.save_playlist_file(suggested_name, current_folder))
    }

    fn configure_open_request(
        &self,
        mut request: ashpd::desktop::file_chooser::OpenFileRequest,
        current_folder: Option<&Path>,
    ) -> Result<ashpd::desktop::file_chooser::OpenFileRequest, Error> {
        if let Some(parent) = &self.parent {
            request = request.identifier(parent.to_ashpd());
        }
        if let Some(folder) = current_folder {
            request = request.current_folder(folder).map_err(Error::Portal)?;
        }
        Ok(request)
    }
}

trait MapSelected<T> {
    fn map_selected<U>(
        self,
        convert: impl FnOnce(T) -> Result<U, Error>,
    ) -> Result<DialogResult<U>, Error>;
}

impl<T> MapSelected<T> for DialogResult<T> {
    fn map_selected<U>(
        self,
        convert: impl FnOnce(T) -> Result<U, Error>,
    ) -> Result<DialogResult<U>, Error> {
        match self {
            Self::Selected(value) => convert(value).map(DialogResult::Selected),
            Self::Cancelled => Ok(DialogResult::Cancelled),
        }
    }
}

fn map_selected(
    result: ashpd::Result<SelectedFiles>,
) -> Result<DialogResult<SelectedFiles>, Error> {
    map_portal_result(result)
}

fn map_portal_result<T>(result: ashpd::Result<T>) -> Result<DialogResult<T>, Error> {
    match result {
        Ok(value) => Ok(DialogResult::Selected(value)),
        Err(
            AshpdError::Response(ResponseError::Cancelled)
            | AshpdError::Portal(PortalError::Cancelled(_)),
        ) => Ok(DialogResult::Cancelled),
        Err(error) => Err(Error::Portal(error)),
    }
}

fn selected_paths(selected: SelectedFiles) -> Result<Vec<PathBuf>, Error> {
    selected
        .uris()
        .iter()
        .map(|uri| {
            file_uri_to_path(uri.as_str()).map_err(|source| Error::InvalidUri {
                uri: uri.as_str().to_owned(),
                source,
            })
        })
        .collect()
}

fn validate_audio_paths(selected: SelectedFiles) -> Result<Vec<PathBuf>, Error> {
    validate_audio_path_list(selected_paths(selected)?).map_err(Error::InvalidSelection)
}

fn validate_audio_path_list(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>, SelectionError> {
    if paths.is_empty() {
        return Err(SelectionError::Empty);
    }
    if let Some(path) = paths
        .iter()
        .find(|path| !xubamp_library::is_audio_path(path))
    {
        return Err(SelectionError::UnsupportedAudio(path.clone()));
    }
    Ok(paths)
}

fn expect_one_path(selected: SelectedFiles) -> Result<PathBuf, Error> {
    expect_one(selected_paths(selected)?).map_err(Error::InvalidSelection)
}

fn expect_one_eqf_path(selected: SelectedFiles) -> Result<PathBuf, Error> {
    let path = expect_one_path(selected)?;
    if has_eqf_extension(&path) {
        Ok(path)
    } else {
        Err(Error::InvalidSelection(SelectionError::NotEqf(path)))
    }
}

fn expect_one_eqf_save_path(selected: SelectedFiles) -> Result<PathBuf, Error> {
    normalize_eqf_save_path(expect_one_path(selected)?).map_err(Error::InvalidSelection)
}

fn expect_one_skin_archive_path(selected: SelectedFiles) -> Result<PathBuf, Error> {
    validate_skin_archive_path(expect_one_path(selected)?).map_err(Error::InvalidSelection)
}

fn validate_skin_archive_path(path: PathBuf) -> Result<PathBuf, SelectionError> {
    if has_skin_archive_extension(&path) {
        Ok(path)
    } else {
        Err(SelectionError::NotSkinArchive(path))
    }
}

fn normalize_eqf_save_path(mut path: PathBuf) -> Result<PathBuf, SelectionError> {
    match path.extension() {
        None => {
            path.set_extension("eqf");
            if has_eqf_extension(&path) {
                Ok(path)
            } else {
                Err(SelectionError::NotEqf(path))
            }
        }
        Some(_) if has_eqf_extension(&path) => Ok(path),
        Some(_) => Err(SelectionError::NotEqf(path)),
    }
}

fn expect_one(mut paths: Vec<PathBuf>) -> Result<PathBuf, SelectionError> {
    if paths.len() == 1 {
        Ok(paths.remove(0))
    } else {
        Err(SelectionError::ExpectedOne {
            actual: paths.len(),
        })
    }
}

fn has_eqf_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("eqf"))
}

fn has_skin_archive_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("wsz") || extension.eq_ignore_ascii_case("zip")
        })
}

fn has_playlist_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("m3u")
                || extension.eq_ignore_ascii_case("m3u8")
                || extension.eq_ignore_ascii_case("pls")
        })
}

fn expect_one_playlist_path(selected: SelectedFiles) -> Result<PathBuf, Error> {
    let path = expect_one_path(selected)?;
    if has_playlist_extension(&path) {
        Ok(path)
    } else {
        Err(Error::InvalidSelection(SelectionError::NotPlaylist(path)))
    }
}

fn expect_one_playlist_save_path(selected: SelectedFiles) -> Result<PathBuf, Error> {
    normalize_playlist_save_path(expect_one_path(selected)?).map_err(Error::InvalidSelection)
}

fn normalize_playlist_save_path(mut path: PathBuf) -> Result<PathBuf, SelectionError> {
    match path.extension() {
        None => {
            path.set_extension("m3u");
            Ok(path)
        }
        Some(_) if has_playlist_extension(&path) => Ok(path),
        Some(_) => Err(SelectionError::NotPlaylist(path)),
    }
}

fn suggested_playlist_name(name: &str) -> String {
    let name = Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("playlist");
    if has_playlist_extension(Path::new(name)) {
        name.to_owned()
    } else {
        format!("{name}.m3u")
    }
}

fn suggested_eqf_name(name: &str) -> String {
    let name = Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("preset");
    if Path::new(name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("eqf"))
    {
        name.to_owned()
    } else {
        format!("{name}.eqf")
    }
}

fn audio_filter() -> FileFilter {
    FileFilter::new("Audio files")
        .glob("*.[mM][pP]3")
        .glob("*.[wW][aA][vV]")
        .glob("*.[fF][lL][aA][cC]")
        .glob("*.[oO][gG][gG]")
        .glob("*.[oO][gG][aA]")
        .mimetype("audio/mpeg")
        .mimetype("audio/wav")
        .mimetype("audio/x-wav")
        .mimetype("audio/flac")
        .mimetype("audio/x-flac")
        .mimetype("audio/ogg")
        .mimetype("audio/vorbis")
}

fn eqf_filter() -> FileFilter {
    FileFilter::new("Winamp equalizer presets").glob("*.[eE][qQ][fF]")
}

fn skin_archive_filter() -> FileFilter {
    FileFilter::new("Winamp skin archives")
        .glob("*.[wW][sS][zZ]")
        .glob("*.[zZ][iI][pP]")
}

fn playlist_filter() -> FileFilter {
    FileFilter::new("Playlists")
        .glob("*.[mM]3[uU]")
        .glob("*.[mM]3[uU]8")
        .glob("*.[pP][lL][sS]")
}

/// Convert a local `file://` URI returned by the portal into a Unix path.
///
/// Empty and `localhost` authorities are accepted. Remote authorities, malformed percent escapes,
/// query strings, fragments, relative paths, and embedded NUL bytes are rejected.
pub fn file_uri_to_path(uri: &str) -> Result<PathBuf, UriError> {
    let Some(after_scheme) = uri
        .get(..7)
        .filter(|prefix| prefix.eq_ignore_ascii_case("file://"))
    else {
        return Err(UriError::NotFileUri);
    };
    let remainder = &uri[after_scheme.len()..];
    let Some(path_start) = remainder.find('/') else {
        return Err(UriError::NotAbsolute);
    };
    let authority = &remainder[..path_start];
    if !authority.is_empty() && !authority.eq_ignore_ascii_case("localhost") {
        return Err(UriError::RemoteAuthority(authority.to_owned()));
    }
    let encoded_path = &remainder[path_start..];
    if encoded_path.contains('?') || encoded_path.contains('#') {
        return Err(UriError::QueryOrFragment);
    }

    let bytes = percent_decode(encoded_path.as_bytes())?;
    if bytes.contains(&0) {
        return Err(UriError::NulByte);
    }
    Ok(PathBuf::from(OsString::from_vec(bytes)))
}

fn percent_decode(input: &[u8]) -> Result<Vec<u8>, UriError> {
    let mut decoded = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        if input[index] == b'%' {
            let Some(high) = input.get(index + 1).and_then(|byte| hex_value(*byte)) else {
                return Err(UriError::InvalidPercentEscape { offset: index });
            };
            let Some(low) = input.get(index + 2).and_then(|byte| hex_value(*byte)) else {
                return Err(UriError::InvalidPercentEscape { offset: index });
            };
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(input[index]);
            index += 1;
        }
    }
    Ok(decoded)
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Failure while converting a portal file URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UriError {
    NotFileUri,
    NotAbsolute,
    RemoteAuthority(String),
    QueryOrFragment,
    InvalidPercentEscape { offset: usize },
    NulByte,
}

impl fmt::Display for UriError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFileUri => formatter.write_str("expected a file:// URI"),
            Self::NotAbsolute => formatter.write_str("file URI does not contain an absolute path"),
            Self::RemoteAuthority(authority) => {
                write!(
                    formatter,
                    "remote file URI authority is not supported: {authority}"
                )
            }
            Self::QueryOrFragment => {
                formatter.write_str("file URI must not contain a query string or fragment")
            }
            Self::InvalidPercentEscape { offset } => {
                write!(formatter, "invalid percent escape at byte {offset}")
            }
            Self::NulByte => formatter.write_str("file URI decodes to an embedded NUL byte"),
        }
    }
}

impl std::error::Error for UriError {}

/// Failure while validating the files selected in a successful portal response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionError {
    Empty,
    ExpectedOne { actual: usize },
    UnsupportedAudio(PathBuf),
    NotEqf(PathBuf),
    NotSkinArchive(PathBuf),
    NotPlaylist(PathBuf),
}

impl fmt::Display for SelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("the portal returned no selected files"),
            Self::ExpectedOne { actual } => {
                write!(formatter, "expected one selected path, got {actual}")
            }
            Self::UnsupportedAudio(path) => {
                write!(formatter, "unsupported audio file: {}", path.display())
            }
            Self::NotEqf(path) => {
                write!(formatter, "expected an .eqf file: {}", path.display())
            }
            Self::NotSkinArchive(path) => {
                write!(
                    formatter,
                    "expected a .wsz or .zip file: {}",
                    path.display()
                )
            }
            Self::NotPlaylist(path) => {
                write!(
                    formatter,
                    "expected an .m3u, .m3u8, or .pls file: {}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for SelectionError {}

/// File chooser setup, transport, response, or validation failure.
#[derive(Debug)]
pub enum Error {
    InvalidParentHandle,
    Portal(AshpdError),
    InvalidUri { uri: String, source: UriError },
    InvalidSelection(SelectionError),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParentHandle => formatter.write_str("invalid Wayland parent handle"),
            Self::Portal(error) => write!(formatter, "file chooser portal failed: {error}"),
            Self::InvalidUri { uri, source } => {
                write!(formatter, "invalid file URI {uri:?}: {source}")
            }
            Self::InvalidSelection(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Portal(error) => Some(error),
            Self::InvalidUri { source, .. } => Some(source),
            Self::InvalidSelection(error) => Some(error),
            Self::InvalidParentHandle => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn file_uri_decodes_spaces_unicode_and_non_utf8_bytes() {
        let path = file_uri_to_path("file:///Music/one%20two/%E2%99%AB/%FF.mp3").unwrap();
        assert_eq!(
            path.as_os_str().as_bytes(),
            b"/Music/one two/\xE2\x99\xAB/\xFF.mp3"
        );
    }

    #[test]
    fn file_uri_accepts_localhost_and_case_insensitive_scheme() {
        assert_eq!(
            file_uri_to_path("FILE://localhost/tmp/preset.eqf").unwrap(),
            Path::new("/tmp/preset.eqf")
        );
    }

    #[test]
    fn file_uri_rejects_non_file_remote_and_ambiguous_uris() {
        assert_eq!(
            file_uri_to_path("https://example.test/a.mp3"),
            Err(UriError::NotFileUri)
        );
        assert_eq!(
            file_uri_to_path("file://server/share/a.mp3"),
            Err(UriError::RemoteAuthority("server".into()))
        );
        assert_eq!(
            file_uri_to_path("file://localhost"),
            Err(UriError::NotAbsolute)
        );
        assert_eq!(
            file_uri_to_path("file:///tmp/a.mp3?download=1"),
            Err(UriError::QueryOrFragment)
        );
        assert_eq!(
            file_uri_to_path("file:///tmp/a%2"),
            Err(UriError::InvalidPercentEscape { offset: 6 })
        );
        assert_eq!(
            file_uri_to_path("file:///tmp/a%00.mp3"),
            Err(UriError::NulByte)
        );
    }

    #[test]
    fn audio_eqf_and_skin_filters_cover_case_variants() {
        let audio = audio_filter();
        assert_eq!(
            audio.pattern_filters(),
            [
                "*.[mM][pP]3",
                "*.[wW][aA][vV]",
                "*.[fF][lL][aA][cC]",
                "*.[oO][gG][gG]",
                "*.[oO][gG][aA]",
            ]
        );
        assert_eq!(
            audio.mimetype_filters(),
            [
                "audio/mpeg",
                "audio/wav",
                "audio/x-wav",
                "audio/flac",
                "audio/x-flac",
                "audio/ogg",
                "audio/vorbis",
            ]
        );
        assert_eq!(eqf_filter().pattern_filters(), ["*.[eE][qQ][fF]"]);
        assert_eq!(
            skin_archive_filter().pattern_filters(),
            ["*.[wW][sS][zZ]", "*.[zZ][iI][pP]"]
        );
    }

    #[test]
    fn playlist_extension_and_save_normalization() {
        assert_eq!(
            playlist_filter().pattern_filters(),
            ["*.[mM]3[uU]", "*.[mM]3[uU]8", "*.[pP][lL][sS]"]
        );
        for name in ["list.m3u", "LIST.M3U8", "radio.pls", "Radio.PLS"] {
            assert!(has_playlist_extension(Path::new(name)), "{name}");
        }
        for name in ["song.mp3", "cover.png", "noext"] {
            assert!(!has_playlist_extension(Path::new(name)), "{name}");
        }
        // No extension defaults to .m3u; .pls is kept; a wrong extension is rejected.
        assert_eq!(
            normalize_playlist_save_path(PathBuf::from("/m/set")),
            Ok(PathBuf::from("/m/set.m3u"))
        );
        assert_eq!(
            normalize_playlist_save_path(PathBuf::from("/m/set.pls")),
            Ok(PathBuf::from("/m/set.pls"))
        );
        assert!(normalize_playlist_save_path(PathBuf::from("/m/set.txt")).is_err());
        assert_eq!(suggested_playlist_name("mix"), "mix.m3u");
        assert_eq!(suggested_playlist_name("mix.pls"), "mix.pls");
    }

    #[test]
    fn audio_validation_rejects_empty_and_advisory_filter_bypasses() {
        assert_eq!(
            validate_audio_path_list(Vec::new()),
            Err(SelectionError::Empty)
        );
        assert_eq!(
            validate_audio_path_list(vec![PathBuf::from("/tmp/movie.mp4")]),
            Err(SelectionError::UnsupportedAudio(PathBuf::from(
                "/tmp/movie.mp4"
            )))
        );
        assert_eq!(
            validate_audio_path_list(vec![
                PathBuf::from("/tmp/a.MP3"),
                PathBuf::from("/tmp/b.wav"),
            ]),
            Ok(vec![
                PathBuf::from("/tmp/a.MP3"),
                PathBuf::from("/tmp/b.wav")
            ])
        );
    }

    #[test]
    fn save_path_appends_missing_eqf_but_rejects_another_extension() {
        assert_eq!(
            normalize_eqf_save_path(PathBuf::from("/tmp/voice")),
            Ok(PathBuf::from("/tmp/voice.eqf"))
        );
        assert_eq!(
            normalize_eqf_save_path(PathBuf::from("/tmp/voice.EQF")),
            Ok(PathBuf::from("/tmp/voice.EQF"))
        );
        assert_eq!(
            normalize_eqf_save_path(PathBuf::from("/tmp/voice.txt")),
            Err(SelectionError::NotEqf(PathBuf::from("/tmp/voice.txt")))
        );
        assert_eq!(suggested_eqf_name("Rock"), "Rock.eqf");
        assert_eq!(suggested_eqf_name("Rock.EQF"), "Rock.EQF");
        assert_eq!(suggested_eqf_name("../Rock"), "Rock.eqf");
        assert_eq!(suggested_eqf_name(""), "preset.eqf");
        assert_eq!(
            normalize_eqf_save_path(PathBuf::from("/")),
            Err(SelectionError::NotEqf(PathBuf::from("/")))
        );
    }

    #[test]
    fn exactly_one_path_is_required_for_directory_and_eqf_dialogs() {
        assert_eq!(
            expect_one(Vec::new()),
            Err(SelectionError::ExpectedOne { actual: 0 })
        );
        assert_eq!(
            expect_one(vec![PathBuf::from("a"), PathBuf::from("b")]),
            Err(SelectionError::ExpectedOne { actual: 2 })
        );
        assert_eq!(
            expect_one(vec![PathBuf::from("a")]).unwrap(),
            Path::new("a")
        );
    }

    #[test]
    fn skin_archive_validation_accepts_wsz_and_zip_only() {
        for path in ["/tmp/Classic.wsz", "/tmp/Classic.WSZ", "/tmp/Classic.zip"] {
            let path = PathBuf::from(path);
            assert_eq!(validate_skin_archive_path(path.clone()), Ok(path));
        }

        for path in ["/tmp/Classic", "/tmp/Classic.tar", "/tmp/Classic.wsz.exe"] {
            let path = PathBuf::from(path);
            assert_eq!(
                validate_skin_archive_path(path.clone()),
                Err(SelectionError::NotSkinArchive(path))
            );
        }
    }

    #[test]
    fn cancelled_dialog_skips_skin_selection_validation() {
        let result: Result<DialogResult<PathBuf>, Error> = DialogResult::<()>::Cancelled
            .map_selected(|_| panic!("selection validation must not run after cancellation"));
        assert!(matches!(result, Ok(DialogResult::Cancelled)));
    }

    #[test]
    fn portal_cancellation_is_not_an_error_but_other_responses_are() {
        let cancelled: Result<DialogResult<()>, Error> =
            map_portal_result(Err(AshpdError::Response(ResponseError::Cancelled)));
        assert_eq!(cancelled.unwrap(), DialogResult::Cancelled);

        let cancelled_method: Result<DialogResult<()>, Error> = map_portal_result(Err(
            AshpdError::Portal(PortalError::Cancelled("cancelled".into())),
        ));
        assert_eq!(cancelled_method.unwrap(), DialogResult::Cancelled);

        let other: Result<DialogResult<()>, Error> =
            map_portal_result(Err(AshpdError::Response(ResponseError::Other)));
        assert!(matches!(
            other,
            Err(Error::Portal(AshpdError::Response(ResponseError::Other)))
        ));
    }

    #[test]
    fn wayland_parent_rejects_empty_or_control_character_handles() {
        assert!(matches!(
            ParentWindow::wayland(""),
            Err(Error::InvalidParentHandle)
        ));
        assert!(matches!(
            ParentWindow::wayland("bad\nhandle"),
            Err(Error::InvalidParentHandle)
        ));
        let wayland = ParentWindow::wayland("valid-export-token").unwrap();
        assert_eq!(wayland.to_ashpd().to_string(), "wayland:valid-export-token");
        assert_eq!(ParentWindow::x11(0x1234).to_ashpd().to_string(), "x11:1234");
    }
}
