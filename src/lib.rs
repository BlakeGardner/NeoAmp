//! NeoAmp: a pixel-exact reproduction of the classic Winamp 2.x interface, drawn from real
//! `.wsz` skins, that plays music.
//!
//! Classic skins are fixed-size sprite sheets; this crate blits rectangles out of them
//! into a software framebuffer exactly as Winamp did.
//!
//! - [`audio`]: decoding (symphonia), resampling, the equalizer and the output device (cpal),
//!   and internet radio streams, fetched by [`http`].
//! - [`vis`]: the spectrum analyser and oscilloscope; [`milkdrop`] the MilkDrop visualization.
//! - [`canvas`]: the RGBA framebuffer and clipped blits, and canvases addressed in skin pixels
//!   at any pixel density.
//! - [`sprites`]: sprite coordinates and window layout shared by every classic skin.
//! - [`dock`]: the magnetic snapping and window-attachment geometry.
//! - [`skin`]: `.wsz` loading (zip, BMP/PNG, `viscolor.txt`, `pledit.txt`, `region.txt`, and
//!   the mouse pointers in [`cursor`]), with sheets that come as SVG parsed and rasterised by
//!   `svg`.
//! - [`ui`]: state, layout, hit testing and input; the window modules draw, `gen_window` the
//!   frame of the generic windows that MilkDrop shows in.
//! - [`menu`]: popup menus drawn like Windows 2000's; [`menus`] holds NeoAmp's own, and
//!   [`shortcut`] writes their keyboard shortcuts the platform's way (⌘ on macOS).
//! - [`eqf`]: equalizer presets, built in and in Winamp's `.eqf` files.
//! - [`zoom`]: how far the skin is enlarged for today's screens.
//! - [`playlist`]: sorting, moving and shuffling playlist entries; [`playlist_file`] loads and
//!   saves playlists as M3U ([`m3u`]) or PLS ([`pls`]).
//! - [`jump`]: the Jump to file dialog, [`info`] the File info dialog and [`url_dialog`] the
//!   Open URL dialog, drawn like Windows 2000's dialogs ([`classic`]), with edit boxes from
//!   [`text_box`] and lists from [`list_box`].
//! - [`museum_dialog`]: the Skin Museum dialog, which finds skins in the Winamp Skin Museum
//!   through [`museum`] and tries them on NeoAmp's own windows; [`browser`] opens the
//!   museum's web site.
//! - [`settings`]: what is remembered between runs (`settings.toml`), with the playlist kept as
//!   an extended M3U file ([`m3u`]), and the skins and presets folders ([`library`]).
//! - [`now_playing`]: the system's media keys and "now playing" display; [`instance`] hands
//!   files opened while NeoAmp runs to the running copy, and `macos_open` takes the files the
//!   macOS Finder opens.
//! - [`app`]: the winit + pixels desktop windows, presented through `upscale`, the windows of
//!   the menus (`popup`) and of the dialogs (`dialog_window`, with `jump_window`,
//!   `info_window`, `url_window` and `museum_window`), the platform's file dialogs (`file_dialog`), the
//!   clipboard (`clipboard`), the windows' icon on Windows (`window_icon`) and MilkDrop's picture
//!   in its window (`milkdrop_view`).

pub mod app;
pub mod audio;
pub mod browser;
pub mod canvas;
pub mod classic;
mod clipboard;
pub mod cursor;
mod dialog_window;
pub mod dock;
mod eq_window;
pub mod eqf;
mod file_dialog;
pub mod font;
mod gen_window;
pub mod http;
pub mod info;
mod info_window;
pub mod instance;
pub mod jump;
mod jump_window;
pub mod library;
pub mod list_box;
pub mod m3u;
#[cfg(target_os = "macos")]
mod macos_open;
mod main_window;
pub mod menu;
pub mod menus;
pub mod milkdrop;
mod milkdrop_view;
pub mod museum;
pub mod museum_dialog;
mod museum_window;
pub mod now_playing;
pub mod playlist;
pub mod playlist_file;
mod playlist_window;
pub mod pls;
mod popup;
pub mod settings;
pub mod shortcut;
pub mod skin;
pub mod sprites;
mod svg;
pub mod text_box;
pub mod ui;
mod upscale;
pub mod url_dialog;
mod url_window;
pub mod vis;
mod window_icon;
pub mod zoom;
