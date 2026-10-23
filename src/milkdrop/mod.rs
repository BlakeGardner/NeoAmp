//! MilkDrop, the visualization that came with Winamp: presets of equations and pixel shaders
//! that warp each frame into the next, drawn in a window of its own.
//!
//! The reference is butterchurn (MIT licensed), the WebGL port of MilkDrop that Webamp uses;
//! names and constants follow it, and through it MilkDrop's own.
//!
//! - [`sound`]: the waveforms, spectra and bass, mid and treble levels presets react to.
//! - [`eel`]: the expression language of presets' equations.
//! - [`preset`]: reading `.milk` presets.
//! - [`state`]: a preset while it runs: its tables of variables and compiled code.
//! - [`draw`]: the shapes, waves, borders and motion vectors drawn over each frame.
//! - [`engine`]: time, blending between presets, and each frame's work, for the GPU.
//! - [`hlsl`]: presets' HLSL pixel shaders translated to WGSL.

pub mod draw;
pub mod eel;
pub mod engine;
pub mod hlsl;
pub mod preset;
pub mod sound;
pub mod state;
