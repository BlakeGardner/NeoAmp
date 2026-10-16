//! Output devices: the list the Output Device menu offers, finding the one the user chose, and
//! noticing when a chosen device that went away (headphones switched off, a USB DAC
//! unplugged) comes back.
//!
//! "System default" is the platform's default output. On Windows and macOS cpal moves a
//! stream on the default device to the new default by itself when the user switches outputs
//! (Windows 10's automatic stream routing, Core Audio's default output unit). On Linux it is
//! ALSA's `default` device, which the sound server (PipeWire or PulseAudio) routes to the
//! desktop's chosen output.
//!
//! On Linux the devices listed by name are the sound server's, reached through cpal's
//! PulseAudio host: a pure-Rust client of the PulseAudio protocol, which PipeWire speaks too.
//! Without a sound server they are ALSA's hardware devices (`plughw:`, one per device, which
//! converts formats as needed).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait};

/// How often a missing device is looked for.
const WATCH_PERIOD: Duration = Duration::from_secs(2);

/// An output device the menu can offer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputDevice {
    /// cpal's identifier, `host:device` (`wasapi:{0.0.0.00000000}.{…}`,
    /// `pulseaudio:bluez_output.…`), which stays the same between runs. `settings.toml` keeps
    /// it.
    pub id: String,
    /// The name the system gives the device.
    pub name: String,
}

/// The output devices that can be chosen by name, in the order the system lists them. Takes
/// a few milliseconds (it asks the sound server on Linux), so it is called when a menu opens,
/// not every frame.
#[must_use]
pub fn output_devices() -> Vec<OutputDevice> {
    let Some(host) = named_host() else {
        return Vec::new();
    };
    let devices = match host.output_devices() {
        Ok(devices) => devices,
        Err(e) => {
            log::warn!("cannot list the audio output devices: {e}");
            return Vec::new();
        }
    };
    devices
        .filter_map(|device| {
            let id = device.id().ok()?;
            if !is_listed(&id) {
                return None;
            }
            let name = device.description().ok()?.name().to_owned();
            Some(OutputDevice {
                id: id.to_string(),
                name,
            })
        })
        .collect()
}

/// The device with identifier `id` (as [`OutputDevice::id`] gives it), if it is connected.
pub(super) fn find(id: &str) -> Option<cpal::Device> {
    let id: cpal::DeviceId = id.parse().ok()?;
    #[cfg(target_os = "linux")]
    if id.host() == cpal::HostId::PulseAudio && !sound_server_running() {
        return None;
    }
    cpal::host_from_id(id.host()).ok()?.device_by_id(&id)
}

/// The system's default output device.
pub(super) fn default_device() -> Option<cpal::Device> {
    // ALSA's `default`, not the PulseAudio host's: that one names the sink that is the default
    // now, and a stream opened on it stays there when the desktop switches outputs.
    #[cfg(target_os = "linux")]
    let host = cpal::host_from_id(cpal::HostId::Alsa).ok()?;
    #[cfg(not(target_os = "linux"))]
    let host = cpal::default_host();
    host.default_output_device()
}

/// The host whose devices are listed by name.
fn named_host() -> Option<cpal::Host> {
    #[cfg(target_os = "linux")]
    {
        let id = if sound_server_running() {
            cpal::HostId::PulseAudio
        } else {
            cpal::HostId::Alsa
        };
        match cpal::host_from_id(id) {
            Ok(host) => Some(host),
            Err(e) => {
                log::warn!("cannot reach the {} audio host: {e}", id.name());
                None
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    Some(cpal::default_host())
}

/// Whether a PulseAudio (or PipeWire) server is there to ask.
#[cfg(target_os = "linux")]
fn sound_server_running() -> bool {
    cpal::available_hosts().contains(&cpal::HostId::PulseAudio)
}

/// Whether the menu lists device `id`. ALSA lists every plugin and every way of opening each
/// card (`front:`, `surround51:`, `dmix:`, ...); only `plughw:` is kept, one entry per device.
/// Other hosts list devices only.
fn is_listed(id: &cpal::DeviceId) -> bool {
    #[cfg(target_os = "linux")]
    if id.host() == cpal::HostId::Alsa {
        return id.id().starts_with("plughw:");
    }
    let _ = id;
    true
}

/// Looks, on a thread of its own, for a chosen device that is not connected, and says when it
/// is back. The thread ends when it finds it or the watch is dropped.
#[derive(Debug)]
pub(super) struct Watch {
    found: Arc<AtomicBool>,
    /// Dropped with the watch, which wakes the thread to end it.
    _stop: mpsc::Sender<()>,
}

impl Watch {
    /// Start looking for device `id` every couple of seconds.
    pub(super) fn spawn(id: String) -> Result<Watch> {
        let found = Arc::new(AtomicBool::new(false));
        let (stop, stopped) = mpsc::channel::<()>();
        let flag = Arc::clone(&found);
        thread::Builder::new()
            .name("audio device watch".into())
            .spawn(move || {
                while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(WATCH_PERIOD) {
                    if find(&id).is_some() {
                        flag.store(true, Ordering::Release);
                        return;
                    }
                }
            })
            .context("starting the audio device watch")?;
        Ok(Watch { found, _stop: stop })
    }

    /// The device is connected again.
    pub(super) fn found(&self) -> bool {
        self.found.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_or_malformed_id_finds_nothing() {
        assert!(find("no colon here").is_none());
        assert!(find("nosuchhost:device").is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn alsa_lists_one_entry_per_device() {
        let id = |s: &str| cpal::DeviceId::new(cpal::HostId::Alsa, s);
        assert!(is_listed(&id("plughw:CARD=Audio,DEV=0")));
        assert!(!is_listed(&id("front:CARD=Audio,DEV=0")));
        assert!(!is_listed(&id("default")));
        assert!(!is_listed(&id("jack")));
    }
}
