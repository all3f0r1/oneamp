//! Media from a file descriptor the host owns (Android content URIs:
//! `ContentResolver.openFileDescriptor(uri, "r").detachFd()`).

use oneamp_core::{CancelToken, PlayerError, SourceFactory};

#[derive(Debug)]
pub struct FdFactory {
    #[cfg(unix)]
    fd: std::os::fd::OwnedFd,
}

impl FdFactory {
    /// Take ownership of `fd`; it is closed when the media is dropped.
    #[cfg(unix)]
    pub fn adopt(fd: i32) -> Result<Self, PlayerError> {
        use std::os::fd::{FromRawFd, OwnedFd};
        if fd < 0 {
            return Err(PlayerError::invalid("negative file descriptor"));
        }
        // SAFETY: the host detached this descriptor for us; we are now
        // its only owner.
        Ok(Self {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
        })
    }

    #[cfg(not(unix))]
    pub fn adopt(_fd: i32) -> Result<Self, PlayerError> {
        Err(PlayerError::unsupported(
            "file descriptors need a unix host",
        ))
    }
}

impl SourceFactory for FdFactory {
    #[cfg(unix)]
    fn open(
        &self,
        _cancel: &CancelToken,
    ) -> Result<Box<dyn oneamp_core::symphonia_player::MediaSource>, PlayerError> {
        use std::io::{Seek, SeekFrom};
        use std::os::fd::AsRawFd;
        // Each reader (current media, its repeat, a resume) needs its own
        // file offset: reopen the descriptor through /proc (Linux and
        // Android) instead of dup(), whose copies share one offset.
        let path = format!("/proc/self/fd/{}", self.fd.as_raw_fd());
        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => {
                let dup = self.fd.try_clone().map_err(|e| PlayerError::from_io(&e))?;
                let mut f = std::fs::File::from(dup);
                f.seek(SeekFrom::Start(0))
                    .map_err(|e| PlayerError::from_io(&e))?;
                f
            }
        };
        Ok(Box::new(file))
    }

    #[cfg(not(unix))]
    fn open(
        &self,
        _cancel: &CancelToken,
    ) -> Result<Box<dyn oneamp_core::symphonia_player::MediaSource>, PlayerError> {
        Err(PlayerError::unsupported(
            "file descriptors need a unix host",
        ))
    }
}
