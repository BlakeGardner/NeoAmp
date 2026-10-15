//! The platform's own file dialogs: files or a folder to play, a skin to show, an equalizer
//! preset to load, and where to save one.
//!
//! A dialog runs beside the event loop, so the windows keep drawing while it is open. It is
//! started on the main thread, as macOS requires, and a helper thread waits for it to close and
//! hands over what was chosen.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use anyhow::{Context, Result};
use rfd::{AsyncFileDialog, FileHandle};
use winit::window::Window;

use crate::audio::{self, FORMATS};

/// What a dialog asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Purpose {
    /// Eject: files to play, which replace the playlist ("Open file(s)" in Winamp).
    PlayFiles,
    /// A folder whose files replace the playlist.
    PlayFolder,
    /// A `.wsz` skin.
    OpenSkin,
    /// An `.eqf` preset to load.
    LoadPreset,
    /// Where to save the equalizer as an `.eqf` preset.
    SavePreset,
}

type Picking = Pin<Box<dyn Future<Output = Vec<PathBuf>> + Send>>;

/// A file dialog that is showing.
#[derive(Debug)]
pub struct FileDialog {
    purpose: Purpose,
    picked: Receiver<Vec<PathBuf>>,
}

/// What a dialog that picks at most one file or folder hands over.
fn one(pick: impl Future<Output = Option<FileHandle>> + Send + 'static) -> Picking {
    Box::pin(async move { pick.await.into_iter().map(PathBuf::from).collect() })
}

impl FileDialog {
    /// Show a dialog for `purpose` in front of `parent`, starting in `folder` if given, and
    /// return at once; [`FileDialog::poll`] reports what was chosen.
    ///
    /// # Errors
    /// Fails if the thread that waits for the dialog cannot be started.
    pub fn show(purpose: Purpose, parent: &Window, folder: Option<&Path>) -> Result<FileDialog> {
        let mut dialog = AsyncFileDialog::new().set_parent(parent);
        if let Some(folder) = folder {
            dialog = dialog.set_directory(folder);
        }
        let pick: Picking = match purpose {
            Purpose::PlayFiles => {
                let pick = file_types()
                    .into_iter()
                    .fold(dialog.set_title("Open file(s)"), |dialog, t| {
                        dialog.add_filter(t.label, &t.extensions)
                    })
                    .pick_files();
                Box::pin(async move {
                    let files = pick.await.unwrap_or_default();
                    files.into_iter().map(PathBuf::from).collect()
                })
            }
            Purpose::PlayFolder => one(dialog.set_title("Open folder").pick_folder()),
            Purpose::OpenSkin => one(dialog
                .set_title("Open skin")
                .add_filter("Winamp skins (*.wsz)", &["wsz", "WSZ"])
                .pick_file()),
            Purpose::LoadPreset => one(dialog
                .set_title("Load EQ preset")
                .add_filter("EQ presets (*.eqf)", &["eqf", "EQF"])
                .pick_file()),
            Purpose::SavePreset => one(dialog
                .set_title("Save EQ preset")
                .add_filter("EQ presets (*.eqf)", &["eqf", "EQF"])
                .save_file()),
        };
        let (tx, picked) = mpsc::channel();
        thread::Builder::new()
            .name("file dialog".into())
            .spawn(move || {
                let paths = pollster::block_on(pick);
                // The receiver is only gone when the app is quitting.
                let _ = tx.send(paths);
            })
            .context("starting the file dialog thread")?;
        Ok(FileDialog { purpose, picked })
    }

    #[must_use]
    pub fn purpose(&self) -> Purpose {
        self.purpose
    }

    /// What was chosen, once the dialog has closed (nothing if it was cancelled); `None` while
    /// it is still open.
    #[must_use]
    pub fn poll(&self) -> Option<Vec<PathBuf>> {
        match self.picked.try_recv() {
            Ok(paths) => Some(paths),
            Err(TryRecvError::Empty) => None,
            // The helper thread ended without an answer: as good as cancelled.
            Err(TryRecvError::Disconnected) => Some(Vec::new()),
        }
    }
}

/// One entry in the dialog's file type list.
#[derive(Debug, PartialEq, Eq)]
struct FileType {
    /// What the list shows, e.g. "WAV audio (*.wav;*.wave)".
    label: String,
    /// The extensions it lets through, without the dot.
    extensions: Vec<String>,
}

/// Every playable file first (the dialog starts on that), then each format on its own.
fn file_types() -> Vec<FileType> {
    let all: Vec<&str> = audio::extensions().collect();
    let mut types = Vec::with_capacity(FORMATS.len() + 1);
    types.push(file_type("All supported types", &all));
    types.extend(FORMATS.iter().map(|f| file_type(f.name, f.extensions)));
    types
}

fn file_type(name: &str, extensions: &[&str]) -> FileType {
    let patterns: Vec<String> = extensions.iter().map(|e| format!("*.{e}")).collect();
    FileType {
        label: format!("{name} ({})", patterns.join(";")),
        // Linux file choosers may match `*.mp3` case-sensitively, so the upper-case spelling
        // common on files from Windows is listed as well. Windows and macOS ignore case anyway.
        extensions: extensions
            .iter()
            .flat_map(|e| [(*e).to_owned(), e.to_ascii_uppercase()])
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_file_type_shows_every_playable_file() {
        let types = file_types();
        assert_eq!(types.len(), FORMATS.len() + 1);
        assert_eq!(
            types[0].label,
            "All supported types (*.mp3;*.wav;*.wave;*.flac;*.ogg;*.oga)"
        );
        for ext in audio::extensions() {
            assert!(types[0].extensions.iter().any(|e| e == ext));
        }
    }

    #[test]
    fn each_format_is_listed_in_both_cases() {
        let wav = &file_types()[2];
        assert_eq!(wav.label, "WAV audio (*.wav;*.wave)");
        assert_eq!(wav.extensions, ["wav", "WAV", "wave", "WAVE"]);
    }
}
