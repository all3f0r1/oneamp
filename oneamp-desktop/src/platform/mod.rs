//! Cross-platform abstraction layer for desktop integrations that
//! historically shipped as Linux-only (D-Bus IPC, MPRIS, libnotify).
//!
//! Each submodule wraps a multi-OS crate (`interprocess`, `souvlaki`,
//! `notify-rust`) so the rest of `oneamp-desktop` calls into it with
//! identical code on Linux, macOS, and Windows.

pub mod default_player;
pub mod ipc;
pub mod media_controls;
pub mod menu_bar;
pub mod notifications;
pub mod tray;
pub mod updater;
