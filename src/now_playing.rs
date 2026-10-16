//! The system's media controls: its "now playing" display shows what NeoAmp plays, and its
//! buttons and the keyboard's media keys control NeoAmp. Through `souvlaki`: MPRIS over D-Bus
//! on Linux (which desktops use for the media keys too), the System Media Transport Controls
//! on Windows, and the Now Playing center and remote commands on macOS.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};
use winit::window::Window;

use crate::audio::{self, Location, Tags};
use crate::ui::{APP_NAME, MediaRequest, PlayState, Ui};

/// How far the seek buttons move, as the arrow keys do.
const SEEK_STEP: f32 = 5.0;
/// How often the position is passed on while playing. Some displays move it on by
/// themselves, others only show what they were last told.
const PROGRESS_EVERY: Duration = Duration::from_secs(5);
/// A position this far from where the last one passed on should have got to is a seek, or a
/// new track, and is passed on at once.
const JUMP: f32 = 2.0;

/// What the system's media controls ask of the app.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Request {
    /// Something for the player.
    Ui(MediaRequest),
    /// Bring the main window to the front.
    Raise,
    /// Close NeoAmp.
    Quit,
}

/// What the system is to show about the current track. Empty strings are left out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Showing<'a> {
    title: &'a str,
    artist: &'a str,
    album: &'a str,
    duration: Option<Duration>,
}

/// What the system was last shown, kept so that an unchanged track is not sent again (and
/// checking costs no allocation each frame). Tags may hold NUL characters, which D-Bus
/// strings cannot (souvlaki would panic on them), so they are left out.
#[derive(Debug, Default)]
struct Shown {
    title: String,
    artist: String,
    album: String,
    duration: Option<Duration>,
}

impl Shown {
    fn is(&self, s: &Showing<'_>) -> bool {
        let same = |kept: &str, new: &str| kept.chars().eq(new.chars().filter(|&c| c != '\0'));
        same(&self.title, s.title)
            && same(&self.artist, s.artist)
            && same(&self.album, s.album)
            && self.duration == s.duration
    }

    fn set(&mut self, s: &Showing<'_>) {
        for (to, from) in [
            (&mut self.title, s.title),
            (&mut self.artist, s.artist),
            (&mut self.album, s.album),
        ] {
            to.clear();
            to.extend(from.chars().filter(|&c| c != '\0'));
        }
        self.duration = s.duration;
    }
}

/// NeoAmp's place in the system's media controls.
pub struct NowPlaying {
    controls: MediaControls,
    requests: Receiver<MediaControlEvent>,
    shown: Shown,
    /// The state and position last passed on, and when.
    playback: Option<(PlayState, f32, Instant)>,
    /// The volume last passed on. Only MPRIS has one.
    #[cfg(target_os = "linux")]
    volume: Option<f32>,
    /// The file whose tags `tags` holds, or is reading.
    tags_of: Option<PathBuf>,
    tags: Option<Tags>,
    reading: Option<Receiver<Tags>>,
}

impl std::fmt::Debug for NowPlaying {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NowPlaying")
            .field("shown", &self.shown)
            .finish_non_exhaustive()
    }
}

impl NowPlaying {
    /// Join the system's media controls. On Windows they belong to a window: `window`, the
    /// main one. `None` (logged) where they cannot be had, such as on Linux without a D-Bus
    /// session.
    #[must_use]
    pub fn new(window: &Window) -> Option<NowPlaying> {
        // MPRIS names: a second copy of a player tells itself apart by its process.
        let dbus_name = format!("neoamp.instance{}", std::process::id());
        // souvlaki's D-Bus thread panics if it cannot connect or have its name, which would
        // end NeoAmp (its release build aborts on a panic): try both first.
        #[cfg(target_os = "linux")]
        if let Err(e) = try_bus(&dbus_name) {
            log::info!("no media keys or now playing display without D-Bus: {e}");
            return None;
        }
        let hwnd = window_handle(window);
        if cfg!(target_os = "windows") && hwnd.is_none() {
            log::warn!("no window handle for the media controls");
            return None;
        }
        let config = PlatformConfig {
            display_name: APP_NAME,
            dbus_name: &dbus_name,
            hwnd,
        };
        let mut controls = match MediaControls::new(config) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("cannot use the system's media controls: {e}");
                return None;
            }
        };
        let (tx, requests) = mpsc::channel();
        if let Err(e) = controls.attach(move |event| {
            // The receiver goes with NowPlaying, which detaches first.
            let _ = tx.send(event);
        }) {
            log::warn!("cannot use the system's media controls: {e}");
            return None;
        }
        Some(NowPlaying {
            controls,
            requests,
            shown: Shown::default(),
            playback: None,
            #[cfg(target_os = "linux")]
            volume: None,
            tags_of: None,
            tags: None,
            reading: None,
        })
    }

    /// The next request from the media controls, if any.
    pub fn poll(&mut self) -> Option<Request> {
        while let Ok(event) = self.requests.try_recv() {
            if let Some(request) = request(event) {
                return Some(request);
            }
        }
        None
    }

    /// Show what `ui` plays, passing on only what changed.
    pub fn update(&mut self, ui: &Ui, now: Instant) {
        self.follow_tags(ui.track().map(|t| &t.location));
        let showing = showing(ui, self.tags.as_ref());
        let new_track = !self.shown.is(&showing);
        if new_track {
            self.shown.set(&showing);
            let metadata = MediaMetadata {
                title: text(&self.shown.title),
                artist: text(&self.shown.artist),
                album: text(&self.shown.album),
                cover_url: None,
                duration: self.shown.duration,
            };
            if let Err(e) = self.controls.set_metadata(metadata) {
                log::debug!("media controls: {e}");
            }
        }
        if new_track || playback_due(self.playback, ui.state, ui.elapsed, now) {
            let progress = Some(MediaPosition(seconds(ui.elapsed).unwrap_or_default()));
            let playback = match ui.state {
                PlayState::Playing => MediaPlayback::Playing { progress },
                PlayState::Paused => MediaPlayback::Paused { progress },
                PlayState::Stopped => MediaPlayback::Stopped,
            };
            if let Err(e) = self.controls.set_playback(playback) {
                log::debug!("media controls: {e}");
            }
            self.playback = Some((ui.state, ui.elapsed, now));
        }
        #[cfg(target_os = "linux")]
        if self.volume != Some(ui.volume) {
            if let Err(e) = self.controls.set_volume(f64::from(ui.volume)) {
                log::debug!("media controls: {e}");
            }
            self.volume = Some(ui.volume);
        }
    }

    /// Read the tags of the file playing, on a thread of their own, when it changes.
    fn follow_tags(&mut self, location: Option<&Location>) {
        if let Some(tags) = self.reading.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.tags = Some(tags);
            self.reading = None;
        }
        let path = location.and_then(Location::path);
        if self.tags_of.as_deref() == path {
            return;
        }
        self.tags = None;
        self.tags_of = path.map(Path::to_path_buf);
        self.reading = path.and_then(read_tags);
    }
}

/// Start reading the tags of `path`; they arrive on the receiver (nothing does if the file
/// cannot be read).
fn read_tags(path: &Path) -> Option<Receiver<Tags>> {
    let (tx, rx) = mpsc::channel();
    let path = path.to_path_buf();
    let spawned = thread::Builder::new()
        .name("now playing tags".into())
        .spawn(move || {
            if let Ok(details) = audio::details(&path) {
                // The receiver is gone if another track started meanwhile.
                let _ = tx.send(details.tags);
            }
        });
    match spawned {
        Ok(_) => Some(rx),
        Err(e) => {
            log::error!("cannot start reading the tags: {e}");
            None
        }
    }
}

/// What to show about the current track. A file shows its tags once read, and its playlist
/// title until then (or without them). A stream shows the song it names, with the station
/// as the artist, or just the station.
fn showing<'a>(ui: &'a Ui, tags: Option<&'a Tags>) -> Showing<'a> {
    let Some(track) = ui.track() else {
        return Showing::default();
    };
    let duration = seconds(ui.track_len()).filter(|d| !d.is_zero());
    if track.location.is_url() {
        return match ui.stream_song() {
            Some(song) => Showing {
                title: song,
                artist: &track.title,
                ..Showing::default()
            },
            None => Showing {
                title: &track.title,
                ..Showing::default()
            },
        };
    }
    let tag = |t: &'a Option<String>| t.as_deref().unwrap_or_default();
    match tags.filter(|t| t.title.is_some()) {
        Some(tags) => Showing {
            title: tag(&tags.title),
            artist: tag(&tags.artist),
            album: tag(&tags.album),
            duration,
        },
        None => Showing {
            title: &track.title,
            duration,
            ..Showing::default()
        },
    }
}

/// Connect to the D-Bus session bus and take, then give back, the MPRIS name `name`, as
/// souvlaki's thread is about to.
#[cfg(target_os = "linux")]
fn try_bus(name: &str) -> zbus::Result<()> {
    let connection = zbus::blocking::Connection::session()?;
    let name = format!("org.mpris.MediaPlayer2.{name}");
    connection.request_name(name.as_str())?;
    connection.release_name(name.as_str())?;
    Ok(())
}

/// Text to show, unless it is empty.
fn text(s: &str) -> Option<&str> {
    (!s.is_empty()).then_some(s)
}

/// Whether to pass the playback state on: it changed, the position jumped, or while playing
/// it has not been passed on for a while.
fn playback_due(
    last: Option<(PlayState, f32, Instant)>,
    state: PlayState,
    at: f32,
    now: Instant,
) -> bool {
    let Some((was, from, when)) = last else {
        return true;
    };
    if was != state {
        return true;
    }
    if state != PlayState::Playing {
        return (at - from).abs() > JUMP;
    }
    let since = now.saturating_duration_since(when);
    since >= PROGRESS_EVERY || (at - from - since.as_secs_f32()).abs() > JUMP
}

/// `secs` as a duration, if it is one.
fn seconds(secs: f32) -> Option<Duration> {
    Duration::try_from_secs_f32(secs).ok()
}

/// What a media control event asks for; `None` for those NeoAmp does not answer (opening a
/// URI).
fn request(event: MediaControlEvent) -> Option<Request> {
    let step = |direction| match direction {
        SeekDirection::Forward => SEEK_STEP,
        SeekDirection::Backward => -SEEK_STEP,
    };
    let ui = match event {
        MediaControlEvent::Play => MediaRequest::Play,
        MediaControlEvent::Pause => MediaRequest::Pause,
        MediaControlEvent::Toggle => MediaRequest::PlayPause,
        MediaControlEvent::Next => MediaRequest::Next,
        MediaControlEvent::Previous => MediaRequest::Previous,
        MediaControlEvent::Stop => MediaRequest::Stop,
        MediaControlEvent::Seek(direction) => MediaRequest::SeekBy(step(direction)),
        MediaControlEvent::SeekBy(direction, by) => {
            MediaRequest::SeekBy(by.as_secs_f32().copysign(step(direction)))
        }
        MediaControlEvent::SetPosition(MediaPosition(at)) => MediaRequest::SeekTo(at.as_secs_f32()),
        // A volume needs no more precision than f32 has.
        MediaControlEvent::SetVolume(v) => MediaRequest::Volume(v as f32),
        MediaControlEvent::Raise => return Some(Request::Raise),
        MediaControlEvent::Quit => return Some(Request::Quit),
        MediaControlEvent::OpenUri(_) => return None,
    };
    Some(Request::Ui(ui))
}

/// The window handle souvlaki ties the System Media Transport Controls to.
#[cfg(target_os = "windows")]
fn window_handle(window: &Window) -> Option<*mut c_void> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as *mut c_void),
        _ => None,
    }
}

/// Only Windows ties the media controls to a window.
#[cfg(not(target_os = "windows"))]
fn window_handle(_window: &Window) -> Option<*mut c_void> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::Location;
    use crate::ui::Track;

    #[test]
    fn requests_follow_the_events() {
        assert_eq!(
            request(MediaControlEvent::Toggle),
            Some(Request::Ui(MediaRequest::PlayPause))
        );
        assert_eq!(
            request(MediaControlEvent::Seek(SeekDirection::Backward)),
            Some(Request::Ui(MediaRequest::SeekBy(-5.0)))
        );
        assert_eq!(
            request(MediaControlEvent::SeekBy(
                SeekDirection::Forward,
                Duration::from_millis(1500)
            )),
            Some(Request::Ui(MediaRequest::SeekBy(1.5)))
        );
        assert_eq!(
            request(MediaControlEvent::SetPosition(MediaPosition(
                Duration::from_secs(42)
            ))),
            Some(Request::Ui(MediaRequest::SeekTo(42.0)))
        );
        assert_eq!(request(MediaControlEvent::Raise), Some(Request::Raise));
        assert_eq!(request(MediaControlEvent::OpenUri("x".into())), None);
    }

    #[test]
    fn a_file_shows_its_tags_once_read_and_its_title_until_then() {
        let ui = Ui::demo();
        let before = showing(&ui, None);
        assert_eq!(before.title, "DJ Mike Llama - Llama Whippin' Intro");
        assert_eq!(before.artist, "");
        assert_eq!(before.duration, Some(Duration::from_secs(5)));
        let tags = Tags {
            title: Some("Llama Whippin' Intro".into()),
            artist: Some("DJ Mike Llama".into()),
            album: Some("Winamp".into()),
            ..Tags::default()
        };
        let after = showing(&ui, Some(&tags));
        assert_eq!(
            (after.title, after.artist, after.album),
            ("Llama Whippin' Intro", "DJ Mike Llama", "Winamp")
        );
        let empty = Ui::new();
        assert_eq!(showing(&empty, None), Showing::default());
        let mut shown = Shown::default();
        assert!(shown.is(&Showing::default()));
        shown.set(&after);
        assert!(shown.is(&after) && !shown.is(&before));
        let nul = Showing {
            title: "Song\0",
            ..after
        };
        shown.set(&nul);
        assert_eq!(shown.title, "Song");
        assert!(shown.is(&nul), "kept without its NUL, and not sent again");
    }

    #[test]
    fn a_stream_shows_its_song_by_its_station() {
        let mut ui = Ui::new();
        ui.tracks = vec![Track {
            title: "Radio Llama".into(),
            seconds: None,
            location: Location::Url("http://radio.example/".into()),
        }];
        let station = showing(&ui, None);
        assert_eq!((station.title, station.duration), ("Radio Llama", None));
    }

    #[test]
    fn the_position_is_passed_on_when_it_jumps_or_now_and_then() {
        let t = Instant::now();
        let at = |secs| t + Duration::from_secs(secs);
        let playing = Some((PlayState::Playing, 10.0, t));
        assert!(playback_due(None, PlayState::Stopped, 0.0, t));
        assert!(!playback_due(playing, PlayState::Playing, 11.0, at(1)));
        assert!(
            playback_due(playing, PlayState::Playing, 40.0, at(1)),
            "a seek"
        );
        assert!(
            playback_due(playing, PlayState::Playing, 15.0, at(5)),
            "a while"
        );
        assert!(playback_due(playing, PlayState::Paused, 11.0, at(1)));
        let paused = Some((PlayState::Paused, 10.0, t));
        assert!(!playback_due(paused, PlayState::Paused, 10.0, at(60)));
    }
}
