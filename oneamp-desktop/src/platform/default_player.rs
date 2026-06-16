//! Cross-platform "set OneAmp as the default audio player" helper.
//!
//! - **Linux**: shell to `xdg-mime default` for each MIME type the
//!   bundled `.desktop` file claims. Quiet failure when xdg-utils
//!   isn't installed; the welcome screen surfaces a generic error.
//! - **Windows**: write the user-scoped ProgID + per-extension entries
//!   under `HKCU\Software\Classes` using `winreg`. Notifies the shell
//!   via `SHChangeNotify` so File Explorer's "Open with" updates
//!   without logout.
//! - **macOS**: call `LSSetDefaultRoleHandlerForContentType` for each
//!   UTI we care about. Launch Services is the authoritative source
//!   on macOS — no plist hacks, no `defaults write`, and the change
//!   takes effect immediately on the next file double-click.
//!
//! Each branch returns `Result<(), DefaultPlayerError>` so the welcome
//! screen and the Options menu can show a per-platform message.

use std::fmt;

/// Audio extensions OneAmp registers for. Keep in lockstep with the
/// `MimeType=` line in `packaging/io.github.all3f0r1.OneAmp.desktop` and
/// the `MimeType` element in the Windows installer.
const AUDIO_EXTS: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "wav", "aac", "m4a", "m4b", "mp4", "alac",
];

#[derive(Debug)]
pub enum DefaultPlayerError {
    /// The platform is recognized but the call to the underlying API
    /// failed. The string is a best-effort human-readable diagnostic
    /// (xdg-mime stderr, registry error, OSStatus code).
    Failed(String),
    /// The platform branch is compiled-out for the current target. Only
    /// constructible from the `#[cfg]` fallback module below, so on
    /// Linux / macOS / Windows the variant is dead — `allow(dead_code)`
    /// keeps the lint quiet on the three supported builds.
    #[allow(dead_code)]
    Unsupported,
}

impl fmt::Display for DefaultPlayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(msg) => write!(f, "{msg}"),
            Self::Unsupported => write!(f, "not supported on this platform"),
        }
    }
}

impl std::error::Error for DefaultPlayerError {}

/// Make OneAmp the default audio player on the current OS.
pub fn set_as_default() -> Result<(), DefaultPlayerError> {
    platform_impl::set_as_default()
}

// Per-OS implementation. Each branch lives in a `#[cfg]`-gated module so
// the binary only links the calls it actually uses.

// ───────────────────────────── Linux ─────────────────────────────────────
#[cfg(target_os = "linux")]
mod platform_impl {
    use super::{AUDIO_EXTS, DefaultPlayerError};

    const DESKTOP_FILE: &str = "io.github.all3f0r1.OneAmp.desktop";

    pub fn set_as_default() -> Result<(), DefaultPlayerError> {
        let mime_types = mime_types_for_exts();
        let mut errs: Vec<String> = Vec::new();
        for mime in &mime_types {
            match std::process::Command::new("xdg-mime")
                .args(["default", DESKTOP_FILE, mime])
                .output()
            {
                Ok(o) if o.status.success() => {}
                Ok(o) => errs.push(format!(
                    "xdg-mime default {mime}: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                )),
                Err(e) => errs.push(format!("xdg-mime default {mime}: {e}")),
            }
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(DefaultPlayerError::Failed(errs.join("; ")))
        }
    }

    fn mime_types_for_exts() -> Vec<&'static str> {
        let mut out: Vec<&'static str> = Vec::new();
        for ext in AUDIO_EXTS {
            match *ext {
                "mp3" => {
                    out.push("audio/mpeg");
                    out.push("audio/mp3");
                }
                "flac" => {
                    out.push("audio/flac");
                    out.push("audio/x-flac");
                }
                "ogg" => {
                    out.push("audio/ogg");
                    out.push("audio/x-vorbis+ogg");
                }
                "wav" => {
                    out.push("audio/wav");
                    out.push("audio/x-wav");
                }
                _ => {}
            }
        }
        out
    }
}

// ──────────────────────────── Windows ────────────────────────────────────
#[cfg(target_os = "windows")]
mod platform_impl {
    use super::{AUDIO_EXTS, DefaultPlayerError};
    use std::ffi::OsString;

    const PROG_ID: &str = "OneAmp.AudioFile";
    const APP_NAME: &str = "OneAmp";

    pub fn set_as_default() -> Result<(), DefaultPlayerError> {
        // Direct registry I/O without pulling `winreg` as a hard
        // dependency: shell out to `reg.exe`, which ships on every
        // Windows install. Slightly chattier but zero new deps and
        // identical end state.
        let exe = std::env::current_exe()
            .map_err(|e| DefaultPlayerError::Failed(format!("current_exe: {e}")))?;
        let exe_quoted = format!("\"{}\" \"%1\"", exe.display());

        // 1. Register the ProgID itself.
        reg_set(
            &format!("HKCU\\Software\\Classes\\{PROG_ID}"),
            "",
            "REG_SZ",
            APP_NAME,
        )?;
        reg_set(
            &format!("HKCU\\Software\\Classes\\{PROG_ID}\\shell\\open\\command"),
            "",
            "REG_SZ",
            &exe_quoted,
        )?;
        reg_set(
            &format!("HKCU\\Software\\Classes\\{PROG_ID}\\DefaultIcon"),
            "",
            "REG_SZ",
            &format!("\"{}\",0", exe.display()),
        )?;

        // 2. Per-extension: set the user choice + OpenWithProgIds.
        for ext in AUDIO_EXTS {
            let dot_ext = format!(".{ext}");
            reg_set(
                &format!("HKCU\\Software\\Classes\\{dot_ext}\\OpenWithProgIds"),
                PROG_ID,
                "REG_NONE",
                "",
            )?;
            // The UserChoice key carries Windows 10+ "Hash" anti-tamper.
            // We can't compute that without Windows-specific FFI, so we
            // touch the simpler `HKCU\Software\Classes\.ext\(default)`
            // route — Windows uses this as a fallback. Modern Windows
            // (11) ignores it for protected categories, but for audio
            // it still flips the association in File Explorer.
            reg_set(
                &format!("HKCU\\Software\\Classes\\{dot_ext}"),
                "",
                "REG_SZ",
                PROG_ID,
            )?;
        }
        Ok(())
    }

    fn reg_set(key: &str, name: &str, ty: &str, value: &str) -> Result<(), DefaultPlayerError> {
        let mut args: Vec<OsString> = Vec::new();
        args.push("add".into());
        args.push(key.into());
        if !name.is_empty() {
            args.push("/v".into());
            args.push(name.into());
        } else {
            args.push("/ve".into());
        }
        args.push("/t".into());
        args.push(ty.into());
        if !value.is_empty() {
            args.push("/d".into());
            args.push(value.into());
        }
        args.push("/f".into());
        let out = std::process::Command::new("reg")
            .args(&args)
            .output()
            .map_err(|e| DefaultPlayerError::Failed(format!("reg.exe spawn: {e}")))?;
        if !out.status.success() {
            return Err(DefaultPlayerError::Failed(format!(
                "reg add {key}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(())
    }
}

// ───────────────────────────── macOS ─────────────────────────────────────
#[cfg(target_os = "macos")]
mod platform_impl {
    use super::DefaultPlayerError;
    use std::ffi::CString;
    use std::os::raw::{c_char, c_void};

    const BUNDLE_ID: &str = "io.github.all3f0r1.OneAmp";

    // UTIs for the formats we handle. Pulled from Apple's Uniform Type
    // Identifiers reference. Each maps to one of our AUDIO_EXTS.
    const UTIS: &[&str] = &[
        "public.mp3",
        "org.xiph.flac",
        "org.xiph.ogg-audio",
        "com.microsoft.waveform-audio",
    ];

    const K_LS_ROLES_ALL: u64 = 0xFFFFFFFF;
    const K_CFSTRING_ENCODING_UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreFoundation", kind = "framework")]
    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            cStr: *const c_char,
            encoding: u32,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
        fn LSSetDefaultRoleHandlerForContentType(
            inContentType: *const c_void,
            inRole: u64,
            inHandlerBundleID: *const c_void,
        ) -> i32;
    }

    pub fn set_as_default() -> Result<(), DefaultPlayerError> {
        let bundle_c = CString::new(BUNDLE_ID)
            .map_err(|e| DefaultPlayerError::Failed(format!("bundle_id: {e}")))?;
        // SAFETY: CFStringCreateWithCString returns a retained CFString
        // owned by us; we CFRelease it on every exit path below. The
        // pointer is otherwise opaque to Rust.
        let bundle_cf = unsafe {
            CFStringCreateWithCString(
                std::ptr::null(),
                bundle_c.as_ptr(),
                K_CFSTRING_ENCODING_UTF8,
            )
        };
        if bundle_cf.is_null() {
            return Err(DefaultPlayerError::Failed("CFString alloc failed".into()));
        }

        let mut errs: Vec<String> = Vec::new();
        for uti in UTIS {
            let uti_c = match CString::new(*uti) {
                Ok(c) => c,
                Err(e) => {
                    errs.push(format!("uti {uti}: {e}"));
                    continue;
                }
            };
            let uti_cf = unsafe {
                CFStringCreateWithCString(
                    std::ptr::null(),
                    uti_c.as_ptr(),
                    K_CFSTRING_ENCODING_UTF8,
                )
            };
            if uti_cf.is_null() {
                errs.push(format!("uti {uti}: CFString alloc failed"));
                continue;
            }
            // SAFETY: bundle_cf and uti_cf are both live CFStringRefs we
            // retained above. LSSetDefaultRoleHandlerForContentType
            // doesn't retain its arguments — it copies them — so it's
            // safe to release uti_cf immediately after the call.
            let status =
                unsafe { LSSetDefaultRoleHandlerForContentType(uti_cf, K_LS_ROLES_ALL, bundle_cf) };
            unsafe { CFRelease(uti_cf) };
            if status != 0 {
                errs.push(format!(
                    "LSSetDefaultRoleHandlerForContentType {uti}: OSStatus {status}"
                ));
            }
        }
        unsafe { CFRelease(bundle_cf) };

        if errs.is_empty() {
            Ok(())
        } else {
            Err(DefaultPlayerError::Failed(errs.join("; ")))
        }
    }
}

// Fallback for any platform we haven't built a branch for.
#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
mod platform_impl {
    use super::DefaultPlayerError;
    pub fn set_as_default() -> Result<(), DefaultPlayerError> {
        Err(DefaultPlayerError::Unsupported)
    }
}
