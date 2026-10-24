//! Files opened from the macOS Finder: double-clicked, chosen with Open With, or dropped onto
//! NeoAmp's Dock icon. macOS sends them to the app as an Open Documents Apple Event rather
//! than on its command line, even when it launches the app to open them, and winit passes no
//! such event on, so NeoAmp handles the event itself. Apple's documentation says to install
//! the handler as the app finishes launching: AppKit has installed its own by then (which
//! would answer that NeoAmp opens no documents), and has not yet delivered the event that
//! launched the app.
//!
//! NeoAmp's only unsafe code is here: an Objective-C class whose methods AppKit calls, named
//! by selector to the notification centre and to the Apple Event Manager.

#![allow(unsafe_code)]

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::NSApplicationWillFinishLaunchingNotification;
use objc2_foundation::{
    MainThreadMarker, NSAppleEventDescriptor, NSAppleEventManager, NSNotification,
    NSNotificationCenter, NSObject, NSObjectProtocol,
};

use crate::audio::Location;
use crate::instance::Request;

/// Apple Event codes (`kCoreEventClass`, `kAEOpenDocuments`, `keyDirectObject`,
/// `typeAEList`), from the Apple Event Manager's headers.
const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
const LIST: u32 = u32::from_be_bytes(*b"list");

/// The files the Finder opens, as requests like those from a later copy of NeoAmp.
#[derive(Debug)]
pub struct Finder {
    requests: Receiver<Request>,
    /// What AppKit calls. Neither the notification centre nor the Apple Event Manager keeps
    /// it alive.
    handler: Retained<Handler>,
}

impl Finder {
    /// Start taking the Finder's Open Documents events. Called before the event loop runs,
    /// so that the files NeoAmp was launched to open are taken too. `None` off the main
    /// thread (logged), where AppKit cannot be reached.
    pub fn listen() -> Option<Finder> {
        let Some(mtm) = MainThreadMarker::new() else {
            log::warn!("files opened from the Finder will not reach NeoAmp: not the main thread");
            return None;
        };
        let (tx, requests) = mpsc::channel();
        let handler = Handler::new(mtm, tx);
        // SAFETY: the selector names `Handler`'s notification method, which takes the
        // notification; `handler` is kept by the `Finder`, which removes it from the centre
        // when dropped. The name is AppKit's own constant.
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &handler,
                sel!(applicationWillFinishLaunching:),
                Some(NSApplicationWillFinishLaunchingNotification),
                None,
            );
        }
        Some(Finder { requests, handler })
    }

    /// The next files the Finder opened, if some have come.
    pub fn poll(&self) -> Option<Request> {
        self.requests.try_recv().ok()
    }
}

impl Drop for Finder {
    fn drop(&mut self) {
        // AppKit must not call the handler once it is gone.
        NSAppleEventManager::sharedAppleEventManager()
            .removeEventHandlerForEventClass_andEventID(CORE_EVENT_CLASS, OPEN_DOCUMENTS);
        // SAFETY: `handler` is the object that was added as an observer.
        unsafe { NSNotificationCenter::defaultCenter().removeObserver(&self.handler) };
    }
}

#[derive(Debug)]
struct Ivars {
    requests: Sender<Request>,
}

define_class!(
    // SAFETY:
    // - NSObject has no subclassing requirements.
    // - `Handler` does not implement `Drop`.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "NeoAmpOpenDocuments"]
    #[ivars = Ivars]
    #[derive(Debug)]
    struct Handler;

    // SAFETY: `NSObjectProtocol` has no safety requirements.
    unsafe impl NSObjectProtocol for Handler {}

    impl Handler {
        /// Install the Open Documents handler, now that AppKit has installed its own.
        // SAFETY: a notification observer's method takes the notification.
        #[unsafe(method(applicationWillFinishLaunching:))]
        fn will_finish_launching(&self, _notification: &NSNotification) {
            // SAFETY: the selector names the method below, which takes the event and the
            // reply, as the Apple Event Manager calls it; the `Finder` that keeps `self`
            // removes the handler when dropped.
            unsafe {
                NSAppleEventManager::sharedAppleEventManager()
                    .setEventHandler_andSelector_forEventClass_andEventID(
                        self,
                        sel!(openDocuments:withReplyEvent:),
                        CORE_EVENT_CLASS,
                        OPEN_DOCUMENTS,
                    );
            }
        }

        /// The Finder opened files: skins are shown, and the rest replace the playlist.
        // SAFETY: an Apple Event handler's method takes the event and the reply.
        #[unsafe(method(openDocuments:withReplyEvent:))]
        fn open_documents(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            let paths = event
                .paramDescriptorForKeyword(DIRECT_OBJECT)
                .map(|files| file_paths(&files))
                .unwrap_or_default();
            log::debug!("the Finder opened {} files", paths.len());
            let mut request = Request::default();
            for path in paths {
                if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("wsz")) {
                    request.skin = Some(path);
                } else {
                    request.files.push(Location::File(path));
                }
            }
            if request != Request::default() {
                // Gone only as NeoAmp closes.
                let _ = self.ivars().requests.send(request);
            }
        }
    }
);

impl Handler {
    fn new(mtm: MainThreadMarker, requests: Sender<Request>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { requests });
        // SAFETY: NSObject's `init` takes nothing and returns the object.
        unsafe { msg_send![super(this), init] }
    }
}

/// The files in an Open Documents event's list (or the one file, if it is not a list), each
/// as macOS names it, Unicode or not. Items that are not files are skipped.
fn file_paths(files: &NSAppleEventDescriptor) -> Vec<PathBuf> {
    let file_path = |item: &NSAppleEventDescriptor| item.fileURLValue()?.to_file_path();
    if files.descriptorType() != LIST {
        return file_path(files).into_iter().collect();
    }
    // Lists count from 1.
    (1..=files.numberOfItems())
        .filter_map(|i| files.descriptorAtIndex(i))
        .filter_map(|item| file_path(&item))
        .collect()
}
