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
//! - [`gpu`]: drawing each frame with wgpu, with the noise textures from [`noise`].
//! - [`presets`]: the presets to choose from, [`built_in`] and the user's own; [`visualizer`]
//!   chooses among them and moves on from one to the next.
//! - [`textures`]: the pictures presets use, found in the user's presets folder.

pub mod built_in;
pub mod draw;
pub mod eel;
pub mod engine;
pub mod gpu;
pub mod hlsl;
pub mod noise;
pub mod preset;
pub mod presets;
pub mod sound;
pub mod state;
pub mod textures;
pub mod visualizer;
