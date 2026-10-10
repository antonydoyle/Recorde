#[cfg(all(feature = "ui-inspection", not(debug_assertions)))]
compile_error!("ui-inspection is development-only and must not be enabled in release builds");

pub mod app;
pub mod auth;
pub mod config;
pub mod crash;
pub mod git;
pub mod icons;
pub mod providers;
pub mod state;
pub mod theme;
pub mod ui;

pub mod external;

pub mod commands;
pub mod update;
