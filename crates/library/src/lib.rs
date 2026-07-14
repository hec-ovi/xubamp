//! Audio-only file discovery shared by command-line input, Add File, Add Dir, and the library.
//!
//! Discovery is deterministic, does not follow symlinks, and reports inaccessible entries without
//! discarding tracks found elsewhere. The extension list deliberately matches the codecs compiled
//! into `xubamp-audio`; video, CD ripping, and playlist-container formats are not accepted here.

use std::fmt;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// Formats the current Symphonia feature set can decode.
pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav"];

/// Whether `path` has a supported audio extension. The path itself may contain non-UTF-8 bytes;
/// only an extension that cannot be represented as text is rejected.
pub fn is_audio_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            AUDIO_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanOptions {
    pub recursive: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self { recursive: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanError {
    pub path: PathBuf,
    pub kind: io::ErrorKind,
    pub message: String,
}

impl ScanError {
    fn new(path: PathBuf, error: io::Error) -> Self {
        Self {
            path,
            kind: error.kind(),
            message: error.to_string(),
        }
    }
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScanReport {
    pub tracks: Vec<PathBuf>,
    pub errors: Vec<ScanError>,
}

/// Discover supported audio under `root`. A file root is accepted directly. Directory traversal is
/// iterative so a deeply nested library cannot overflow the call stack. Symlinks are ignored even
/// when they point inside the root, preventing cycles and accidental traversal outside the chosen
/// library directory.
pub fn scan(root: &Path, options: ScanOptions) -> ScanReport {
    let mut report = ScanReport::default();
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) => {
            report
                .errors
                .push(ScanError::new(root.to_path_buf(), error));
            return report;
        }
    };

    if metadata.file_type().is_symlink() {
        return report;
    }
    if metadata.is_file() {
        if is_audio_path(root) {
            report.tracks.push(root.to_path_buf());
        }
        return report;
    }
    if !metadata.is_dir() {
        return report;
    }

    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                report.errors.push(ScanError::new(directory, error));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    report.errors.push(ScanError::new(directory.clone(), error));
                    continue;
                }
            };
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    report.errors.push(ScanError::new(path, error));
                    continue;
                }
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_file() {
                if is_audio_path(&path) {
                    report.tracks.push(path);
                }
            } else if options.recursive && file_type.is_dir() {
                pending.push(path);
            }
        }
    }

    report.tracks.sort_by(|left, right| {
        left.as_os_str()
            .as_bytes()
            .cmp(right.as_os_str().as_bytes())
    });
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::fs::File;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::symlink;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "xubamp-library-{}-{nonce}-{name}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        File::create(path).unwrap();
    }

    #[test]
    fn extension_policy_is_case_insensitive_and_audio_only() {
        for path in ["track.mp3", "TRACK.MP3", "sample.WaV"] {
            assert!(is_audio_path(Path::new(path)), "{path}");
        }
        for path in [
            "movie.mp4",
            "clip.avi",
            "list.m3u",
            "radio.pls",
            "future.flac",
            "no-extension",
        ] {
            assert!(!is_audio_path(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn recursive_scan_is_sorted_and_ignores_video_unknown_and_playlist_files() {
        let root = temp_dir("recursive");
        for relative in [
            "z.WAV",
            "a.mp3",
            "nested/c.mp3",
            "nested/movie.mp4",
            "nested/list.m3u",
            ".hidden.wav",
        ] {
            touch(&root.join(relative));
        }

        let report = scan(&root, ScanOptions::default());
        assert!(report.errors.is_empty());
        assert_eq!(
            report
                .tracks
                .iter()
                .map(|path| path.strip_prefix(&root).unwrap())
                .collect::<Vec<_>>(),
            [
                Path::new(".hidden.wav"),
                Path::new("a.mp3"),
                Path::new("nested/c.mp3"),
                Path::new("z.WAV"),
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn non_recursive_scan_stays_at_the_selected_directory() {
        let root = temp_dir("flat");
        touch(&root.join("top.mp3"));
        touch(&root.join("nested/deep.wav"));
        let report = scan(&root, ScanOptions { recursive: false });
        assert_eq!(report.tracks, [root.join("top.mp3")]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_file_root_is_classified_and_a_missing_root_is_reported() {
        let root = temp_dir("file");
        let audio = root.join("one.wav");
        touch(&audio);
        assert_eq!(scan(&audio, ScanOptions::default()).tracks, [audio]);

        let missing = root.join("missing");
        let report = scan(&missing, ScanOptions::default());
        assert!(report.tracks.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].path, missing);
        assert_eq!(report.errors[0].kind, io::ErrorKind::NotFound);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn symlinks_are_never_followed_and_cannot_form_a_cycle() {
        let root = temp_dir("links");
        let outside = temp_dir("outside");
        touch(&root.join("real.mp3"));
        touch(&outside.join("outside.wav"));
        symlink(&root, root.join("cycle")).unwrap();
        symlink(outside.join("outside.wav"), root.join("linked.wav")).unwrap();
        symlink(&outside, root.join("outside-dir")).unwrap();

        let report = scan(&root, ScanOptions::default());
        assert_eq!(report.tracks, [root.join("real.mp3")]);
        assert!(report.errors.is_empty());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn non_utf8_paths_are_sorted_without_lossy_conversion() {
        let root = temp_dir("raw");
        let first = root.join(OsString::from_vec(b"raw-\x80.mp3".to_vec()));
        let second = root.join(OsString::from_vec(b"raw-\xFF.wav".to_vec()));
        touch(&second);
        touch(&first);
        let report = scan(&root, ScanOptions::default());
        assert_eq!(report.tracks, [first, second]);
        fs::remove_dir_all(root).unwrap();
    }
}
