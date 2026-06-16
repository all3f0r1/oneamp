//! Platform detection module for smart window chrome configuration
//!
//! This module detects the platform, desktop environment, and display server
//! to determine whether custom window chrome should be enabled.

#![allow(dead_code)]

// Only the Linux desktop-environment / display-server probes read env vars;
// the rest of the module is cfg'd out off-Linux, so the import is too.
#[cfg(target_os = "linux")]
use std::env;

/// Platform information
#[derive(Debug, Clone, PartialEq)]
pub struct PlatformInfo {
    pub os: OperatingSystem,
    pub desktop_environment: Option<DesktopEnvironment>,
    pub display_server: Option<DisplayServer>,
}

/// Operating system
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OperatingSystem {
    Linux,
    Windows,
    MacOS,
    Other,
}

/// Desktop environment (Linux only)
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(clippy::upper_case_acronyms)]
pub enum DesktopEnvironment {
    GNOME,
    KDE,
    XFCE,
    MATE,
    Cinnamon,
    LXDE,
    LXQt,
    Budgie,
    Pantheon,
    Unknown,
}

/// Display server (Linux only)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DisplayServer {
    X11,
    Wayland,
}

impl PlatformInfo {
    /// Detect the current platform
    pub fn detect() -> Self {
        let os = Self::detect_os();
        let desktop_environment = if os == OperatingSystem::Linux {
            Some(Self::detect_desktop_environment())
        } else {
            None
        };
        let display_server = if os == OperatingSystem::Linux {
            Self::detect_display_server()
        } else {
            None
        };

        Self {
            os,
            desktop_environment,
            display_server,
        }
    }

    /// Detect the operating system
    fn detect_os() -> OperatingSystem {
        #[cfg(target_os = "linux")]
        return OperatingSystem::Linux;

        #[cfg(target_os = "windows")]
        return OperatingSystem::Windows;

        #[cfg(target_os = "macos")]
        return OperatingSystem::MacOS;

        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        return OperatingSystem::Other;
    }

    /// Detect the desktop environment (Linux only)
    #[cfg(target_os = "linux")]
    fn detect_desktop_environment() -> DesktopEnvironment {
        // Check XDG_CURRENT_DESKTOP first (most reliable)
        if let Ok(desktop) = env::var("XDG_CURRENT_DESKTOP") {
            let desktop_lower = desktop.to_lowercase();

            if desktop_lower.contains("gnome") {
                return DesktopEnvironment::GNOME;
            } else if desktop_lower.contains("kde") || desktop_lower.contains("plasma") {
                return DesktopEnvironment::KDE;
            } else if desktop_lower.contains("xfce") {
                return DesktopEnvironment::XFCE;
            } else if desktop_lower.contains("mate") {
                return DesktopEnvironment::MATE;
            } else if desktop_lower.contains("cinnamon") {
                return DesktopEnvironment::Cinnamon;
            } else if desktop_lower.contains("lxde") {
                return DesktopEnvironment::LXDE;
            } else if desktop_lower.contains("lxqt") {
                return DesktopEnvironment::LXQt;
            } else if desktop_lower.contains("budgie") {
                return DesktopEnvironment::Budgie;
            } else if desktop_lower.contains("pantheon") {
                return DesktopEnvironment::Pantheon;
            }
        }

        // Fallback to DESKTOP_SESSION
        if let Ok(session) = env::var("DESKTOP_SESSION") {
            let session_lower = session.to_lowercase();

            if session_lower.contains("gnome") {
                return DesktopEnvironment::GNOME;
            } else if session_lower.contains("kde") || session_lower.contains("plasma") {
                return DesktopEnvironment::KDE;
            } else if session_lower.contains("xfce") {
                return DesktopEnvironment::XFCE;
            } else if session_lower.contains("mate") {
                return DesktopEnvironment::MATE;
            } else if session_lower.contains("cinnamon") {
                return DesktopEnvironment::Cinnamon;
            }
        }

        // Check for specific environment variables
        if env::var("GNOME_DESKTOP_SESSION_ID").is_ok() {
            return DesktopEnvironment::GNOME;
        }
        if env::var("KDE_FULL_SESSION").is_ok() {
            return DesktopEnvironment::KDE;
        }

        DesktopEnvironment::Unknown
    }

    #[cfg(not(target_os = "linux"))]
    fn detect_desktop_environment() -> DesktopEnvironment {
        DesktopEnvironment::Unknown
    }

    /// Detect the display server (Linux only)
    #[cfg(target_os = "linux")]
    fn detect_display_server() -> Option<DisplayServer> {
        // Check WAYLAND_DISPLAY first
        if env::var("WAYLAND_DISPLAY").is_ok() {
            return Some(DisplayServer::Wayland);
        }

        // Check XDG_SESSION_TYPE
        if let Ok(session_type) = env::var("XDG_SESSION_TYPE") {
            let session_lower = session_type.to_lowercase();
            if session_lower == "wayland" {
                return Some(DisplayServer::Wayland);
            } else if session_lower == "x11" {
                return Some(DisplayServer::X11);
            }
        }

        // Check DISPLAY (X11)
        if env::var("DISPLAY").is_ok() {
            return Some(DisplayServer::X11);
        }

        None
    }

    #[cfg(not(target_os = "linux"))]
    fn detect_display_server() -> Option<DisplayServer> {
        None
    }

    /// Determine if custom window chrome should be enabled.
    ///
    /// Rules:
    /// - Windows / macOS: always enabled.
    /// - Linux + Wayland: enabled (compositor honours `xdg_toplevel.move`).
    /// - Linux + X11 + known DE (KDE / GNOME / XFCE / MATE / Cinnamon /
    ///   Budgie / Pantheon / LXDE / LXQt): enabled. All major DEs honour
    ///   `_NET_WM_MOVERESIZE`; the previous "disabled on GNOME family"
    ///   rule was a workaround for our own `StartDrag` flood (now fixed).
    /// - Linux + X11 + Unknown DE: disabled (conservative — user can force
    ///   it back on with `ONEAMP_CUSTOM_CHROME=1`).
    pub fn should_use_custom_chrome(&self) -> bool {
        match self.os {
            OperatingSystem::Windows => true,
            OperatingSystem::MacOS => true,
            OperatingSystem::Linux => {
                if self.display_server == Some(DisplayServer::Wayland) {
                    return true;
                }
                matches!(
                    self.desktop_environment,
                    Some(
                        DesktopEnvironment::KDE
                            | DesktopEnvironment::MATE
                            | DesktopEnvironment::GNOME
                            | DesktopEnvironment::XFCE
                            | DesktopEnvironment::Cinnamon
                            | DesktopEnvironment::Budgie
                            | DesktopEnvironment::Pantheon
                            | DesktopEnvironment::LXDE
                            | DesktopEnvironment::LXQt
                    )
                )
            }
            OperatingSystem::Other => false,
        }
    }

    /// Get a human-readable description of the platform
    pub fn description(&self) -> String {
        let mut parts = Vec::new();

        parts.push(
            match self.os {
                OperatingSystem::Linux => "Linux",
                OperatingSystem::Windows => "Windows",
                OperatingSystem::MacOS => "macOS",
                OperatingSystem::Other => "Other OS",
            }
            .to_string(),
        );

        if let Some(de) = self.desktop_environment {
            parts.push(format!("{:?}", de));
        }

        if let Some(ds) = self.display_server {
            parts.push(format!("{:?}", ds));
        }

        parts.join(" / ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_detection() {
        let platform = PlatformInfo::detect();

        // Should detect current OS
        #[cfg(target_os = "linux")]
        assert_eq!(platform.os, OperatingSystem::Linux);

        #[cfg(target_os = "windows")]
        assert_eq!(platform.os, OperatingSystem::Windows);

        #[cfg(target_os = "macos")]
        assert_eq!(platform.os, OperatingSystem::MacOS);
    }

    #[test]
    fn test_windows_always_custom_chrome() {
        let platform = PlatformInfo {
            os: OperatingSystem::Windows,
            desktop_environment: None,
            display_server: None,
        };

        assert!(platform.should_use_custom_chrome());
    }

    #[test]
    fn test_macos_always_custom_chrome() {
        let platform = PlatformInfo {
            os: OperatingSystem::MacOS,
            desktop_environment: None,
            display_server: None,
        };

        assert!(platform.should_use_custom_chrome());
    }

    #[test]
    fn test_linux_wayland_custom_chrome() {
        let platform = PlatformInfo {
            os: OperatingSystem::Linux,
            desktop_environment: Some(DesktopEnvironment::GNOME),
            display_server: Some(DisplayServer::Wayland),
        };

        // Wayland should enable custom chrome even on GNOME
        assert!(platform.should_use_custom_chrome());
    }

    #[test]
    fn test_linux_x11_gnome_custom_chrome() {
        let platform = PlatformInfo {
            os: OperatingSystem::Linux,
            desktop_environment: Some(DesktopEnvironment::GNOME),
            display_server: Some(DisplayServer::X11),
        };

        // Mutter honours _NET_WM_MOVERESIZE; the historical "disabled on
        // GNOME" rule was a workaround for our own StartDrag-per-frame bug,
        // not a Mutter limitation.
        assert!(platform.should_use_custom_chrome());
    }

    #[test]
    fn test_linux_x11_kde_custom_chrome() {
        let platform = PlatformInfo {
            os: OperatingSystem::Linux,
            desktop_environment: Some(DesktopEnvironment::KDE),
            display_server: Some(DisplayServer::X11),
        };

        // KDE + X11 should enable custom chrome
        assert!(platform.should_use_custom_chrome());
    }

    #[test]
    fn test_linux_x11_xfce_custom_chrome() {
        let platform = PlatformInfo {
            os: OperatingSystem::Linux,
            desktop_environment: Some(DesktopEnvironment::XFCE),
            display_server: Some(DisplayServer::X11),
        };

        // xfwm4 honours _NET_WM_MOVERESIZE; previously disabled by mistake.
        assert!(platform.should_use_custom_chrome());
    }

    #[test]
    fn test_linux_x11_unknown_no_custom_chrome() {
        let platform = PlatformInfo {
            os: OperatingSystem::Linux,
            desktop_environment: Some(DesktopEnvironment::Unknown),
            display_server: Some(DisplayServer::X11),
        };

        // Unknown DE + X11 should disable custom chrome (safe default)
        assert!(!platform.should_use_custom_chrome());
    }

    #[test]
    fn test_description() {
        let platform = PlatformInfo {
            os: OperatingSystem::Linux,
            desktop_environment: Some(DesktopEnvironment::GNOME),
            display_server: Some(DisplayServer::Wayland),
        };

        let desc = platform.description();
        assert!(desc.contains("Linux"));
        assert!(desc.contains("GNOME"));
        assert!(desc.contains("Wayland"));
    }
}
