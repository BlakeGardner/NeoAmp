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
//! - [`menu`]: popup menus drawn like Windows 2000's; [`menus`] holds NeoAmp's own.
//! - [`eqf`]: equalizer presets, built in and in Winamp's `.eqf` files.
//! - [`zoom`]: how far the skin is enlarged for today's screens.
//! - [`settings`]: what is remembered between runs (`settings.toml`), with the playlist kept as
//!   an extended M3U file ([`m3u`]), and the skins and presets folders ([`library`]).
//! - [`app`]: the winit + pixels desktop windows, presented through `upscale`, the menus'
//!   windows (`popup`) and the platform's file dialogs (`file_dialog`).

pub mod app;
pub mod audio;
pub mod canvas;
pub mod dock;
mod eq_window;
pub mod eqf;
mod file_dialog;
pub mod font;
pub mod library;
pub mod m3u;
mod main_window;
pub mod menu;
pub mod menus;
mod playlist_window;
mod popup;
pub mod settings;
pub mod skin;
pub mod sprites;
pub mod ui;
mod upscale;
pub mod vis;
pub mod zoom;
