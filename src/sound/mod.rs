//! Sound: audio control on PipeWire (formerly Hokum Sound). `model` is the contract between
//! the backend (`pipewire`, or `mock` for scenes) and the section's UI (`ui`).

pub mod mock;
// Parts of the contract the backend and the UI don't use yet.
#[allow(dead_code)]
pub mod model;
pub mod pipewire;
pub mod ui;
