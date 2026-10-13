//! Reading playlist titles and lengths in the background, so adding a folder never stalls the
//! interface.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use anyhow::{Context, Result};

use super::TrackInfo;

/// A background thread that reads the tags and length of each file it is given.
#[derive(Debug)]
pub struct Scanner {
    requests: Sender<(usize, PathBuf)>,
    results: Receiver<(usize, PathBuf, TrackInfo)>,
}

impl Scanner {
    /// Start the scanning thread. It exits when the scanner is dropped.
    ///
    /// # Errors
    /// Fails if the thread cannot be spawned.
    pub fn new() -> Result<Scanner> {
        let (requests, request_rx) = mpsc::channel::<(usize, PathBuf)>();
        let (result_tx, results) = mpsc::channel();
        thread::Builder::new()
            .name("playlist scanner".into())
            .spawn(move || {
                for (index, path) in request_rx {
                    match super::probe(&path) {
                        Ok(info) => {
                            if result_tx.send((index, path, info)).is_err() {
                                return;
                            }
                        }
                        // Left titled by file name; playing it reports the problem.
                        Err(e) => log::debug!("{e:#}"),
                    }
                }
            })
            .context("starting the playlist scanner thread")?;
        Ok(Scanner { requests, results })
    }

    /// Queue playlist entry `index`, which plays `path`.
    pub fn scan(&self, index: usize, path: PathBuf) {
        // Only fails if the thread has died; the entry then keeps its file name.
        let _ = self.requests.send((index, path));
    }

    /// A finished scan: the playlist index and path it was asked about, and what it found.
    #[must_use]
    pub fn poll(&self) -> Option<(usize, PathBuf, TrackInfo)> {
        self.results.try_recv().ok()
    }
}
