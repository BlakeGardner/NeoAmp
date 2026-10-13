//! The "Open file(s)" dialog behind the eject button: the platform's own file chooser, showing
//! the formats the player decodes, with several files selectable at once.
//!
//! The dialog runs beside the event loop, so the windows keep drawing while it is open. It is
//! started on the main thread, as macOS requires, and a helper thread waits for it to close and
//! hands over the chosen files.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use anyhow::{Context, Result};
use rfd::AsyncFileDialog;
use winit::window::Window;

use crate::audio::{self, FORMATS};

const TITLE: &str = "Open file(s)";

/// A file dialog that is showing.
#[derive(Debug)]
pub struct OpenDialog {
    picked: Receiver<Vec<PathBuf>>,
}

impl OpenDialog {
    /// Show the dialog in front of `parent` and return at once; [`OpenDialog::poll`] reports
    /// what was chosen.
    ///
    /// # Errors
    /// Fails if the thread that waits for the dialog cannot be started.
    pub fn show(parent: &Window) -> Result<OpenDialog> {
        let dialog = file_types().into_iter().fold(
            AsyncFileDialog::new().set_title(TITLE).set_parent(parent),
            |dialog, t| dialog.add_filter(t.label, &t.extensions),
        );
        let pick = dialog.pick_files();
        let (tx, picked) = mpsc::channel();
        thread::Builder::new()
            .name("open dialog".into())
            .spawn(move || {
                let files = pollster::block_on(pick).unwrap_or_default();
                // The receiver is only gone when the app is quitting.
                let _ = tx.send(files.into_iter().map(PathBuf::from).collect());
            })
            .context("starting the file dialog thread")?;
        Ok(OpenDialog { picked })
    }

    /// The files chosen, once the dialog has closed (none if it was cancelled); `None` while
    /// it is still open.
    #[must_use]
    pub fn poll(&self) -> Option<Vec<PathBuf>> {
        match self.picked.try_recv() {
            Ok(files) => Some(files),
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
