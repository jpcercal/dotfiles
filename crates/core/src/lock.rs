use std::path::Path;

#[derive(Debug)]
pub struct FileLock {
    path: std::path::PathBuf,
}

/// mkdir-based lock (macOS-friendly, no flock)
impl FileLock {
    pub fn acquire(lock_dir: &Path) -> anyhow::Result<Option<Self>> {
        match std::fs::create_dir(lock_dir) {
            Ok(()) => {
                let pid_path = lock_dir.join("pid");
                let pid = std::process::id().to_string();
                let _ = std::fs::write(&pid_path, pid);
                Ok(Some(Self {
                    path: lock_dir.to_path_buf(),
                }))
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let pid_path = lock_dir.join("pid");
                if let Ok(raw) = std::fs::read_to_string(&pid_path) {
                    if let Ok(pid) = raw.trim().parse::<i32>() {
                        let rc = unsafe { libc::kill(pid, 0) };
                        let is_stale = if rc == 0 {
                            false
                        } else {
                            let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                            errno == libc::ESRCH
                        };
                        if is_stale {
                            let _ = std::fs::remove_dir_all(lock_dir);
                            return Self::acquire(lock_dir);
                        }
                    }
                }
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn release(self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// Minimal libc binding without adding crate
mod libc {
    pub const ESRCH: i32 = 3;
    #[allow(dead_code)]
    pub const EPERM: i32 = 1;
    extern "C" {
        pub fn kill(pid: i32, sig: i32) -> i32;
    }
}
