//! NeoAmp: a pixel-exact reproduction of the classic Winamp 2.x interface, drawn from real
//! `.wsz` skins, that plays music.
//!
//! Classic skins are fixed-size sprite sheets; this crate blits rectangles out of them
//! into a software framebuffer exactly as Winamp did.
//!
//! - [`audio`]: decoding (symphonia), resampling, the equalizer and the output device (cpal).
//! - [`vis`]: the spectrum analyser and oscilloscope.
//! - [`canvas`]: the RGBA framebuffer and clipped blits.
//! - [`sprites`]: sprite coordinates and window layout shared by every classic skin.
//! - [`dock`]: the magnetic snapping and window-attachment geometry.
//! - [`skin`]: `.wsz` loading (zip, BMP/PNG, `viscolor.txt`, `pledit.txt`, `region.txt`).
//! - [`ui`]: state, layout, hit testing and input; the window modules draw.
//! - [`menu`]: popup menus drawn like Windows 2000's; [`menus`] holds NeoAmp's own, and
//!   [`shortcut`] writes their keyboard shortcuts the platform's way (⌘ on macOS).
//! - [`eqf`]: equalizer presets, built in and in Winamp's `.eqf` files.
//! - [`zoom`]: how far the skin is enlarged for today's screens.
//! - [`playlist`]: sorting, moving and shuffling playlist entries; [`playlist_file`] loads and
//!   saves playlists as M3U ([`m3u`]) or PLS ([`pls`]).
//! - [`jump`]: the Jump to file dialog, drawn like Windows 2000's dialogs ([`classic`]).
//! - [`settings`]: what is remembered between runs (`settings.toml`), with the playlist kept as
//!   an extended M3U file ([`m3u`]), and the skins and presets folders ([`library`]).
//! - [`app`]: the winit + pixels desktop windows, presented through `upscale`, the windows of
//!   the menus (`popup`) and of the Jump to file dialog (`jump_window`), and the platform's
//!   file dialogs (`file_dialog`).

pub mod app;
pub mod audio;
pub mod canvas;
pub mod classic;
pub mod dock;
mod eq_window;
pub mod eqf;
mod file_dialog;
pub mod font;
pub mod jump;
mod jump_window;
pub mod library;
pub mod m3u;
mod main_window;
pub mod menu;
pub mod menus;
pub mod playlist;
pub mod playlist_file;
mod playlist_window;
pub mod pls;
mod popup;
pub mod settings;
pub mod shortcut;
pub mod skin;
pub mod sprites;
pub mod ui;
mod upscale;
pub mod vis;
pub mod zoom;
