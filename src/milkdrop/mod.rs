//! MilkDrop, the visualization that came with Winamp: presets of equations and pixel shaders
//! that warp each frame into the next, drawn in a window of its own.
//!
//! The reference is butterchurn (MIT licensed), the WebGL port of MilkDrop that Webamp uses;
//! names and constants follow it, and through it MilkDrop's own.
//!
//! - [`sound`]: the waveforms, spectra and bass, mid and treble levels presets react to.
//! - [`eel`]: the expression language of presets' equations.
//! - [`preset`]: reading `.milk` presets.

pub mod eel;
pub mod preset;
pub mod sound;
