//! A pixel-exact Winamp 2.x classic interface, drawn from real `.wsz` skins.
//!
//! Classic skins are fixed-size sprite sheets; this crate blits rectangles out of them
//! into a software framebuffer exactly as Winamp did.
//!
//! - [`canvas`]: the RGBA framebuffer and clipped blits.
//! - [`sprites`]: sprite coordinates and window layout shared by every classic skin.
//! - [`skin`]: `.wsz` loading (zip, BMP/PNG, `viscolor.txt`, `pledit.txt`, `region.txt`).
//! - [`ui`]: state, layout, hit testing and input; the window modules draw.
//! - [`app`]: the winit + pixels desktop window.

pub mod app;
pub mod canvas;
mod eq_window;
pub mod font;
mod main_window;
mod playlist_window;
pub mod skin;
pub mod sprites;
pub mod ui;
pub mod vis;
