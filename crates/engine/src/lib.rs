//! Wildlife Network Studio engine: everything the desktop app does, usable from the `wlnw` command line too.
pub mod audio;
pub mod birdnet;
pub mod download;
pub mod export;
pub mod meta;
pub mod models;
pub mod perch;
pub mod publish;
pub mod review;
pub mod run;
pub mod scan;
pub mod store;
pub mod wnc;

/// Review clips are played and drawn at 48 kHz.
pub const SR: u32 = 48_000;
