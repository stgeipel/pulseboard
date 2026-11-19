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

#[cfg(all(target_os = "windows", feature = "desktop"))]
pub static MONITOR_THREAD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[cfg(any(target_os = "windows", test))]
pub struct NativeException<'a> {
    pub code: u32,
    pub thread: u32,
    pub role: &'static str,
    pub operation: Option<usize>,
    pub module: Option<(&'a str, usize)>,
}

#[cfg(any(target_os = "windows", test))]
impl NativeException<'_> {
    pub fn write(&self, output: &mut impl Write) -> io::Result<()> {
        write!(
            output,
            "native exception code=0x{:08X} thread={} role={}",
            self.code, self.thread, self.role
        )?;
        if self.code == 0xC0000005
            && let Some(operation) = self.operation
        {
            let operation = match operation {
                0 => "read",
                1 => "write",
                8 => "execute",
                _ => "unknown",
            };
            write!(output, " operation={operation}")?;
        }
        if let Some((module, offset)) = self.module {
            write!(output, " module={module} offset=0x{offset:X}")?;
        }
        output.write_all(b"\r\n")
    }
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
    fn writes_fault_context_into_a_fixed_buffer() -> io::Result<()> {
        let exception = NativeException {
            code: 0xC0000005,
            thread: 101,
            role: "ui",
            operation: Some(1),
            module: Some(("dwrite.dll", 0x124)),
        };
        let mut buffer = [0; 256];
        let mut output = io::Cursor::new(buffer.as_mut_slice());
        exception.write(&mut output)?;
        let len = output.position() as usize;
        assert_eq!(
            &buffer[..len],
            b"native exception code=0xC0000005 thread=101 role=ui operation=write module=dwrite.dll offset=0x124\r\n"
        );
        Ok(())
    }
}
