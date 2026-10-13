//! A pixel-exact Winamp 2.x classic interface, drawn from real `.wsz` skins, that plays music.
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
//! - [`zoom`]: how far the skin is enlarged for today's screens.
//! - [`app`]: the winit + pixels desktop windows, presented through `upscale`.

pub mod app;
pub mod audio;
pub mod canvas;
pub mod dock;
mod eq_window;
pub mod font;
mod main_window;
mod playlist_window;
pub mod skin;
pub mod sprites;
pub mod ui;
mod upscale;
pub mod vis;
pub mod zoom;
