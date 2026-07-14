//! Worker-thread bridge between classic menu actions and XDG desktop portal dialogs.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{SystemTime, UNIX_EPOCH};

use xubamp_portal::{DialogResult, FileChooser};
use xubamp_render::equalizer;
use xubamp_render::menu::ClassicMenuAction;
use xubamp_wl::MenuRequest;

#[derive(Debug)]
pub(crate) enum Completion {
    AddPaths {
        paths: Vec<PathBuf>,
        warnings: Vec<String>,
    },
    OpenPaths(Vec<PathBuf>),
    EqualizerPreset(equalizer::Preset),
    Saved(PathBuf),
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LaunchResult {
    Started,
    Busy,
    Unsupported,
}

#[derive(Clone)]
pub(crate) struct Launcher {
    sender: mpsc::Sender<Completion>,
    busy: Arc<AtomicBool>,
}

pub(crate) struct Receiver {
    receiver: mpsc::Receiver<Completion>,
    busy: Arc<AtomicBool>,
}

pub(crate) fn bridge() -> (Launcher, Receiver) {
    let (sender, receiver) = mpsc::channel();
    let busy = Arc::new(AtomicBool::new(false));
    (
        Launcher {
            sender,
            busy: Arc::clone(&busy),
        },
        Receiver { receiver, busy },
    )
}

impl Launcher {
    /// Start a portal request without blocking the Wayland event thread. Only chooser-backed menu
    /// requests are accepted; all other actions stay with the main application dispatcher.
    pub(crate) fn launch(&self, request: MenuRequest) -> LaunchResult {
        if !is_supported(&request) {
            return LaunchResult::Unsupported;
        }
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return LaunchResult::Busy;
        }

        let sender = self.sender.clone();
        let busy = Arc::clone(&self.busy);
        let result = std::thread::Builder::new()
            .name("xubamp-portal".to_owned())
            .spawn(move || {
                let _reset = BusyReset(Arc::clone(&busy));
                match execute(request) {
                    Ok(Some(completion)) => {
                        let _ = sender.send(completion);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let _ = sender.send(Completion::Error(error));
                    }
                }
            });
        if let Err(error) = result {
            self.busy.store(false, Ordering::Release);
            let _ = self.sender.send(Completion::Error(format!(
                "cannot start desktop portal worker: {error}"
            )));
        }
        LaunchResult::Started
    }
}

impl Receiver {
    /// Drain completed work without blocking and report whether a dialog is still open.
    pub(crate) fn poll(&mut self) -> (Vec<Completion>, bool) {
        let mut completions: Vec<_> = self.receiver.try_iter().collect();
        let pending = self.busy.load(Ordering::Acquire);
        if !pending {
            // The worker sends before clearing `busy`. A second drain closes the tiny race where
            // the first drain ran immediately before that send.
            completions.extend(self.receiver.try_iter());
        }
        (completions, pending)
    }
}

struct BusyReset(Arc<AtomicBool>);

impl Drop for BusyReset {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn is_supported(request: &MenuRequest) -> bool {
    matches!(
        request,
        MenuRequest::OpenMedia
            | MenuRequest::Action(
                ClassicMenuAction::PlaylistAddDirectory
                    | ClassicMenuAction::PlaylistAddFile
                    | ClassicMenuAction::EqualizerLoadEqf
            )
            | MenuRequest::SaveEqualizer(_)
    )
}

fn execute(request: MenuRequest) -> Result<Option<Completion>, String> {
    let chooser = FileChooser::new();
    match request {
        MenuRequest::OpenMedia => chooser
            .open_audio_files_blocking(None)
            .map_err(|error| format!("cannot open audio file chooser: {error}"))
            .map(|result| match result {
                DialogResult::Selected(paths) => Some(Completion::OpenPaths(paths)),
                DialogResult::Cancelled => None,
            }),
        MenuRequest::Action(ClassicMenuAction::PlaylistAddFile) => chooser
            .open_audio_files_blocking(None)
            .map_err(|error| format!("cannot open audio file chooser: {error}"))
            .map(|result| match result {
                DialogResult::Selected(paths) => Some(Completion::AddPaths {
                    paths,
                    warnings: Vec::new(),
                }),
                DialogResult::Cancelled => None,
            }),
        MenuRequest::Action(ClassicMenuAction::PlaylistAddDirectory) => chooser
            .open_directory_blocking(None)
            .map_err(|error| format!("cannot open directory chooser: {error}"))
            .map(|result| match result {
                DialogResult::Selected(root) => {
                    let report = xubamp_library::scan(&root, playlist_directory_scan_options());
                    Some(Completion::AddPaths {
                        paths: report.tracks,
                        warnings: report
                            .errors
                            .into_iter()
                            .map(|error| error.to_string())
                            .collect(),
                    })
                }
                DialogResult::Cancelled => None,
            }),
        MenuRequest::Action(ClassicMenuAction::EqualizerLoadEqf) => {
            let result = chooser
                .open_eqf_file_blocking(None)
                .map_err(|error| format!("cannot open equalizer file chooser: {error}"))?;
            let DialogResult::Selected(path) = result else {
                return Ok(None);
            };
            let bytes = fs::read(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            let library = xubamp_dsp::eqf::Library::parse(&bytes)
                .map_err(|error| format!("cannot load {}: {error}", path.display()))?;
            let preset = library
                .presets
                .first()
                .ok_or_else(|| format!("{} contains no equalizer presets", path.display()))?;
            let settings = preset.settings(true);
            Ok(Some(Completion::EqualizerPreset(equalizer::Preset {
                name: preset.name.clone(),
                preamp_db: settings.preamp_db,
                bands_db: settings.bands_db,
            })))
        }
        MenuRequest::SaveEqualizer(preset) => {
            let result = chooser
                .save_eqf_file_blocking(&preset.name, None)
                .map_err(|error| format!("cannot open equalizer save dialog: {error}"))?;
            let DialogResult::Selected(path) = result else {
                return Ok(None);
            };
            let settings = xubamp_dsp::EqSettings {
                enabled: true,
                preamp_db: preset.preamp_db,
                bands_db: preset.bands_db,
            };
            let bytes = xubamp_dsp::eqf::Library {
                presets: vec![xubamp_dsp::eqf::Preset::from_settings(
                    preset.name,
                    settings,
                )],
            }
            .to_bytes();
            save_atomic(&path, &bytes)
                .map_err(|error| format!("cannot save {}: {error}", path.display()))?;
            Ok(Some(Completion::Saved(path)))
        }
        _ => Ok(None),
    }
}

/// Playlist Add Directory is a one-shot recursive import. Library recursion is a separate catalog
/// preference and must never change what this classic playlist command discovers.
fn playlist_directory_scan_options() -> xubamp_library::ScanOptions {
    xubamp_library::ScanOptions { recursive: true }
}

/// Create and sync an adjacent temporary file before replacing the destination. This prevents a
/// crash or full disk from leaving a partially-written EQF file behind.
fn save_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("preset.eqf"));
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut temp_name = OsString::from(".");
    temp_name.push(name);
    temp_name.push(format!(".{}.{nonce}.tmp", std::process::id()));
    let temp_path = parent.join(temp_name);

    let write_result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp_path, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    write_result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "xubamp-portal-actions-{}-{nonce}-{name}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn atomic_eqf_save_round_trips_and_leaves_no_temporary_file() {
        let dir = temp_dir("save");
        let path = dir.join("Custom.eqf");
        let bytes = xubamp_dsp::eqf::Library {
            presets: vec![xubamp_dsp::eqf::Preset::from_settings(
                "Custom",
                xubamp_dsp::EqSettings {
                    enabled: true,
                    preamp_db: 4.5,
                    bands_db: [-12.0, -9.0, -6.0, -3.0, 0.0, 3.0, 6.0, 9.0, 12.0, 1.5],
                },
            )],
        }
        .to_bytes();

        save_atomic(&path, &bytes).unwrap();

        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(
            xubamp_dsp::eqf::Library::parse(&fs::read(&path).unwrap())
                .unwrap()
                .presets
                .len(),
            1
        );
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unsupported_menu_actions_do_not_start_a_worker() {
        let (launcher, mut receiver) = bridge();
        assert_eq!(
            launcher.launch(MenuRequest::Action(ClassicMenuAction::PlaylistAddUrl)),
            LaunchResult::Unsupported
        );
        let (completions, pending) = receiver.poll();
        assert!(completions.is_empty());
        assert!(!pending);
    }

    #[test]
    fn playlist_directory_add_is_always_recursive() {
        assert!(playlist_directory_scan_options().recursive);
    }
}
