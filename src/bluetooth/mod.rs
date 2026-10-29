//! Bluetooth: a Bluetooth manager on BlueZ over D-Bus (formerly Hokum Bluetooth). `model` is
//! the contract between the backend (`bluez`, or `mock` for scenes) and the section's UI (`ui`).

pub mod bluez;
#[allow(dead_code)]
pub mod mock;
// Parts of the contract the backend and the UI don't use yet.
#[allow(dead_code)]
pub mod model;
pub mod ui;
