use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

use crate::paths;

const MAX_LOG_FILES: usize = 7;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 40 * 1024 * 1024;
const CAPPED_NOTE: &[u8] = b"log hari ini mencapai batas ukuran, baris berikutnya tidak dicatat sampai besok\n";

fn utc_day() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)
}

fn day_string(day: u64) -> String {
    let z = day as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn today_file_size(dir: &Path, component: &str, day: u64) -> u64 {
    std::fs::metadata(dir.join(format!("{component}.{}.log", day_string(day))))
        .map(|m| m.len())
        .unwrap_or(0)
}

fn prune_total(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "log"))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            meta.is_file()
                .then(|| (meta.modified().unwrap_or(UNIX_EPOCH), meta.len(), e.path()))
        })
        .collect();
    files.sort_by_key(|(modified, _, _)| *modified);
    let mut total: u64 = files.iter().map(|(_, len, _)| *len).sum();
    let today = day_string(utc_day());
    for (_, len, path) in files {
        if total <= MAX_TOTAL_BYTES {
            break;
        }
        let current = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.contains(&today));
        if current {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

struct CappedWriter {
    inner: RollingFileAppender,
    day: u64,
    written: u64,
    noted: bool,
}

impl Write for CappedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let day = utc_day();
        if day != self.day {
            self.day = day;
            self.written = 0;
            self.noted = false;
        }
        if self.written >= MAX_FILE_BYTES {
            if !self.noted {
                self.noted = true;
                let _ = self.inner.write_all(CAPPED_NOTE);
            }
            return Ok(buf.len());
        }
        let n = self.inner.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

pub struct LogGuard {
    _file: WorkerGuard,
    pub dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error(transparent)]
    Path(#[from] paths::PathError),
    #[error("gagal membuat file log: {0}")]
    Appender(#[from] tracing_appender::rolling::InitError),
    #[error("logger sudah terpasang: {0}")]
    Init(#[from] tracing_subscriber::util::TryInitError),
}

pub fn init(component: &str, console: bool) -> Result<LogGuard, LogError> {
    init_at(component, paths::log_dir()?, console)
}

pub fn init_at(component: &str, dir: PathBuf, console: bool) -> Result<LogGuard, LogError> {
    std::fs::create_dir_all(&dir).map_err(|source| paths::PathError::Create {
        path: dir.clone(),
        source,
    })?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(component)
        .filename_suffix("log")
        .max_log_files(MAX_LOG_FILES)
        .build(&dir)?;
    prune_total(&dir);
    let day = utc_day();
    let written = today_file_size(&dir, component, day);
    let (writer, guard) = tracing_appender::non_blocking(CappedWriter {
        inner: appender,
        day,
        written,
        noted: written >= MAX_FILE_BYTES,
    });

    let filter =
        || EnvFilter::try_from_env("TAUTRA_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    let file_layer = fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_target(true)
        .with_thread_names(true)
        .with_filter(filter());

    let console_layer = console.then(|| {
        fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(true)
            .with_filter(filter())
    });

    tracing_subscriber::registry()
        .with(file_layer)
        .with(console_layer)
        .try_init()?;

    install_panic_hook();

    tracing::info!(
        component,
        version = crate::APP_VERSION,
        log_dir = %dir.display(),
        "logging aktif"
    );

    Ok(LogGuard { _file: guard, dir })
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(
            thread = thread.name().unwrap_or("<tanpa nama>"),
            "PANIC: {info}\n{backtrace}"
        );
        default_hook(info);
    }));
}
