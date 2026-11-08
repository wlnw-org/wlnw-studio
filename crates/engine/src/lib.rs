//! Wildlife Network Studio engine: everything the desktop app does, usable from the `wlnw` command line too.
pub mod agreement;
pub mod audio;
pub mod batdetect;
pub mod birdnet;
pub mod camtrap;
pub mod download;
pub mod export;
pub mod files;
pub mod history;
pub mod images;
pub mod import;
pub mod meta;
pub mod models;
pub mod online;
pub mod perch;
pub mod photo;
pub mod publish;
pub mod review;
pub mod run;
pub mod scan;
pub mod store;
pub mod wnc;

/// Review clips are played and drawn at 48 kHz.
pub const SR: u32 = 48_000;
