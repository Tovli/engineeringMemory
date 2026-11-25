//! Process-wide coordination for embedded store access.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs2::FileExt;

pub struct StoreLock {
    _file: File,
    pub path: PathBuf,
}

impl StoreLock {
    pub fn acquire(path: &Path, timeout: Duration) -> anyhow::Result<Self> {
        std::fs::create_dir_all(path)?;
        let path = path.canonicalize()?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.join("store.lock"))?;
        let start = Instant::now();
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(Self { _file: file, path }),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
                    || cfg!(windows) && matches!(error.raw_os_error(), Some(33 | 36)) => {
                    if start.elapsed() >= timeout {
                        anyhow::bail!("store_in_use");
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}
