//! Crash-safe file replacement shared by every JSON / playlist writer.

use std::fs;
use std::io::Write;
use std::path::Path;

/// Replace `path` with `bytes` atomically: write `<name>.tmp` next to it,
/// fsync, then rename over the target. A crash mid-write leaves either
/// the previous file or the new one, never a truncated mix. Creates the
/// parent directory if missing.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_file_and_leaves_no_tmp() {
        let dir = std::env::temp_dir().join(format!("oneamp_atomic_{}", std::process::id()));
        let path = dir.join("sub/config.json");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        assert!(!dir.join("sub/config.json.tmp").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
