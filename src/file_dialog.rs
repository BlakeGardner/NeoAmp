//! The platform's own file dialogs: files or a folder to play or add to the playlist, a
//! playlist to load and where to save one, a skin to show, an equalizer preset to load, and
//! where to save one.
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
use crate::playlist_file;

/// What a dialog asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Purpose {
    /// Eject: files (or playlists) to play, which replace the playlist ("Open file(s)" in
    /// Winamp).
    PlayFiles,
    /// A folder whose files replace the playlist.
    PlayFolder,
    /// Files (or playlists) to add to the end of the playlist.
    AddFiles,
    /// A folder whose files are added to the end of the playlist.
    AddFolder,
    /// A playlist file whose entries replace the playlist.
    LoadList,
    /// Where to save the playlist.
    SaveList,
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
        let with_types = |dialog: AsyncFileDialog, types: Vec<FileType>| {
            types.into_iter().fold(dialog, |dialog, t| {
                dialog.add_filter(t.label, &t.extensions)
            })
        };
        let pick: Picking = match purpose {
            Purpose::PlayFiles | Purpose::AddFiles => {
                let title = if purpose == Purpose::PlayFiles {
                    "Open file(s)"
                } else {
                    "Add file(s) to playlist"
                };
                let pick = with_types(dialog.set_title(title), file_types()).pick_files();
                Box::pin(async move {
                    let files = pick.await.unwrap_or_default();
                    files.into_iter().map(PathBuf::from).collect()
                })
            }
            Purpose::PlayFolder => one(dialog.set_title("Open folder").pick_folder()),
            Purpose::AddFolder => one(dialog.set_title("Add folder to playlist").pick_folder()),
            Purpose::LoadList => one(with_types(
                dialog.set_title("Load playlist"),
                vec![playlist_type()],
            )
            .pick_file()),
            Purpose::SaveList => one(with_types(
                dialog
                    .set_title("Save playlist")
                    .set_file_name("Playlist.m3u8"),
                vec![
                    file_type("M3U8 playlist", &["m3u8"]),
                    file_type("M3U playlist", &["m3u"]),
                    file_type("PLS playlist", &["pls"]),
                ],
            )
            .save_file()),
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

/// Every playable file and playlist first (the dialog starts on that), then each audio format
/// on its own, then playlists.
fn file_types() -> Vec<FileType> {
    let all: Vec<&str> = audio::extensions()
        .chain(playlist_file::EXTENSIONS)
        .collect();
    let mut types = Vec::with_capacity(FORMATS.len() + 2);
    types.push(file_type("All supported types", &all));
    types.extend(FORMATS.iter().map(|f| file_type(f.name, f.extensions)));
    types.push(playlist_type());
    types
}

fn playlist_type() -> FileType {
    file_type("Playlists", &playlist_file::EXTENSIONS)
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_first_file_type_shows_every_playable_file() {
        let types = file_types();
        assert_eq!(types.len(), FORMATS.len() + 2);
        assert_eq!(
            types[0].label,
            "All supported types (*.mp3;*.mp2;*.mp1;*.aac;*.m4a;*.m4b;*.mp4;*.wav;*.wave;*.aiff;*.aif;\
             *.aifc;*.flac;*.ogg;*.oga;*.m3u;*.m3u8;*.pls)"
        );
        for ext in audio::extensions() {
            assert!(types[0].extensions.iter().any(|e| e == ext));
        }
        assert_eq!(
            types.last().unwrap().label,
            "Playlists (*.m3u;*.m3u8;*.pls)"
        );
    }

    #[test]
    fn each_format_is_listed_in_both_cases() {
        let wav = &file_types()[5];
        assert_eq!(wav.label, "WAV audio (*.wav;*.wave)");
        assert_eq!(wav.extensions, ["wav", "WAV", "wave", "WAVE"]);
    }
}
