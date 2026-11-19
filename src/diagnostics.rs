use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

const LOG_LIMIT: u64 = 512 * 1024;

pub struct SessionLog {
    directory: PathBuf,
}

impl SessionLog {
    pub fn start(directory: &Path) -> io::Result<Self> {
        fs::create_dir_all(directory)?;
        let log = Self {
            directory: directory.to_path_buf(),
        };
        let unclean = log.marker().try_exists()?;
        let path = log.path();
        if fs::metadata(&path).is_ok_and(|metadata| metadata.len() >= LOG_LIMIT) {
            // A log viewer can keep the old file open on Windows. Continue recording if rotation is blocked.
            let _ = fs::rename(&path, directory.join("crash.previous.log"));
        }

        let mut file = log.open()?;
        fs::write(log.marker(), b"running\n")?;
        if unclean {
            log.write(
                &mut file,
                format_args!("previous session ended without a shutdown marker"),
            )?;
        }
        log.write(
            &mut file,
            format_args!("session start · Pulseboard {}", env!("CARGO_PKG_VERSION")),
        )?;
        Ok(log)
    }

    pub fn path(&self) -> PathBuf {
        self.directory.join("crash.log")
    }

    fn marker(&self) -> PathBuf {
        self.directory.join("session.running")
    }

    pub fn open(&self) -> io::Result<File> {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path())
    }

    pub fn record(&self, event: std::fmt::Arguments<'_>) -> io::Result<()> {
        self.write(&mut self.open()?, event)
    }

    pub fn finish(&self) -> io::Result<()> {
        self.record(format_args!("session shutdown"))?;
        fs::remove_file(self.marker())
    }

    fn write(&self, file: &mut File, event: std::fmt::Arguments<'_>) -> io::Result<()> {
        let now = SystemTime::now();
        let pid = std::process::id();
        file.write_all(format!("{now:?} [pid {pid}] {event}\n").as_bytes())
    }
}

#[cfg(any(target_os = "windows", test))]
pub fn exception_line(code: u32) -> [u8; 34] {
    let mut line = *b"native exception code=0x00000000\r\n";
    let digits = b"0123456789ABCDEF";
    let start = line.len() - 10;
    for (index, byte) in line[start..start + 8].iter_mut().enumerate() {
        let shift = (7 - index) * 4;
        *byte = digits[((code >> shift) & 0xF) as usize];
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_normal_shutdown_from_an_interrupted_session() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let first = SessionLog::start(directory.path())?;
        first.finish()?;

        let second = SessionLog::start(directory.path())?;
        assert!(!fs::read_to_string(second.path())?.contains("previous session ended"));

        let restarted = SessionLog::start(directory.path())?;
        let contents = fs::read_to_string(restarted.path())?;
        assert_eq!(contents.matches("previous session ended").count(), 1);
        assert_eq!(contents.matches("session start").count(), 3);
        restarted.finish()?;
        Ok(())
    }

    #[test]
    fn rotates_large_logs_and_preserves_the_previous_file() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("crash.log");
        fs::write(&path, vec![b'x'; LOG_LIMIT as usize])?;
        let log = SessionLog::start(directory.path())?;
        assert!(fs::read_to_string(log.path())?.contains("session start"));
        assert_eq!(
            fs::metadata(directory.path().join("crash.previous.log"))?.len(),
            LOG_LIMIT
        );
        log.finish()
    }

    #[test]
    fn formats_windows_exception_code_without_a_heap_allocation() {
        assert_eq!(
            &exception_line(0xC0000005),
            b"native exception code=0xC0000005\r\n"
        );
    }
}
