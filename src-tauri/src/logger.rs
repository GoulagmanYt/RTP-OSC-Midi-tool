use std::{
    fs::{create_dir_all, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock,
    },
};

use chrono::Local;
use directories::ProjectDirs;
use parking_lot::Mutex;
use tauri::{Emitter, Manager, Window};

use crate::types::LogEvent;

static LOG_FILE: LazyLock<Mutex<Option<std::fs::File>>> = LazyLock::new(|| Mutex::new(None));
static GLOBAL_DEV_MODE: AtomicBool = AtomicBool::new(false);
static LOG_ALL_TO_FILE: AtomicBool = AtomicBool::new(false);
static LOGS_ENABLED: AtomicBool = AtomicBool::new(true);

fn refresh_log_level() {
    if !LOGS_ENABLED.load(Ordering::Relaxed) {
        log::set_max_level(log::LevelFilter::Off);
    } else if GLOBAL_DEV_MODE.load(Ordering::Relaxed) || LOG_ALL_TO_FILE.load(Ordering::Relaxed) {
        log::set_max_level(log::LevelFilter::Debug);
    } else {
        log::set_max_level(log::LevelFilter::Info);
    }
}

pub fn set_global_dev_mode(enabled: bool) {
    GLOBAL_DEV_MODE.store(enabled, Ordering::Relaxed);
    refresh_log_level();
}

pub fn set_log_all_to_file(enabled: bool) {
    LOG_ALL_TO_FILE.store(enabled, Ordering::Relaxed);
    refresh_log_level();
}

pub fn set_logs_enabled(enabled: bool) {
    LOGS_ENABLED.store(enabled, Ordering::Relaxed);
    refresh_log_level();
}

pub fn logs_enabled() -> bool {
    LOGS_ENABLED.load(Ordering::Relaxed)
}

pub fn should_log_debug() -> bool {
    LOGS_ENABLED.load(Ordering::Relaxed)
        && (GLOBAL_DEV_MODE.load(Ordering::Relaxed) || LOG_ALL_TO_FILE.load(Ordering::Relaxed))
}

#[derive(Clone)]
pub struct FrontendLogger {
    window: Window,
    dev_enabled: Arc<AtomicBool>,
}

impl FrontendLogger {
    pub fn new(window: Window, dev_enabled: Arc<AtomicBool>) -> Self {
        let _ = &*LOG_DISPATCH;
        // Sync global state on init
        set_global_dev_mode(dev_enabled.load(Ordering::Relaxed));
        Self {
            window,
            dev_enabled,
        }
    }

    pub fn app_handle(&self) -> tauri::AppHandle {
        self.window.app_handle().clone()
    }

    pub fn info(&self, message: impl Into<String>) {
        self.emit("info", message);
    }

    pub fn warn(&self, message: impl Into<String>) {
        self.emit("warn", message);
    }

    pub fn error(&self, message: impl Into<String>) {
        self.emit("error", message);
    }

    pub fn debug(&self, message: impl Into<String>) {
        self.emit("debug", message);
    }

    fn emit(&self, level: &str, message: impl Into<String>) {
        if !LOGS_ENABLED.load(Ordering::Relaxed) {
            return;
        }
        let dev = self.dev_enabled.load(Ordering::Relaxed);
        if dev != GLOBAL_DEV_MODE.load(Ordering::Relaxed) {
            set_global_dev_mode(dev);
        }
        dispatch_log(LogJob {
            level: level.into(),
            message: message.into(),
            window: Some((
                self.window.app_handle().clone(),
                self.window.label().to_owned(),
            )),
            dev,
        });
    }
}

struct LogJob {
    level: String,
    message: String,
    window: Option<(tauri::AppHandle, String)>,
    dev: bool,
}
pub static DROPPED_LOGS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LOG_DISPATCH: LazyLock<crossbeam_channel::Sender<LogJob>> = LazyLock::new(|| {
    let (tx, rx) = crossbeam_channel::bounded::<LogJob>(256);
    std::thread::Builder::new()
        .name("diagnostic-logs".into())
        .spawn(move || {
            ensure_file_logger();
            for job in rx {
                if let Some((app, label)) = job.window {
                    if job.level != "debug" || job.dev {
                        let _ = app.emit_to(
                            label,
                            "log:entry",
                            LogEvent::new(&job.level, job.message.clone()),
                        );
                    }
                }
                if job.dev
                    || LOG_ALL_TO_FILE.load(Ordering::Relaxed)
                    || job.level == "error"
                    || job.level == "warn"
                {
                    append_file_log(&job.level, &job.message);
                }
                match job.level.as_str() {
                    "error" => log::error!("{}", job.message),
                    "warn" => log::warn!("{}", job.message),
                    "debug" => log::debug!("{}", job.message),
                    _ => log::info!("{}", job.message),
                }
            }
        })
        .expect("diagnostic logging thread");
    tx
});
fn dispatch_log(job: LogJob) {
    if LOG_DISPATCH.try_send(job).is_err() {
        DROPPED_LOGS.fetch_add(1, Ordering::Relaxed);
    }
}

fn ensure_file_logger() {
    let mut guard = LOG_FILE.lock();
    if guard.is_some() {
        return;
    }
    let path = log_file_path();
    if let Some(path) = path {
        if let Some(parent) = path.parent() {
            let _ = create_dir_all(parent);
        }
        if let Ok(file) = OpenOptions::new().create(true).append(true).open(path) {
            *guard = Some(file);
        }
    }
}

/// Logger utilisable hors UI (ex: thread audio) pour consigner dans app.log et dans env_logger.
pub fn background_log(level: &str, message: impl Into<String>) {
    if !LOGS_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    dispatch_log(LogJob {
        level: level.into(),
        message: message.into(),
        window: None,
        dev: GLOBAL_DEV_MODE.load(Ordering::Relaxed),
    });
}

pub fn clear_log_file() -> Result<(), String> {
    let path = log_file_path().ok_or_else(|| "Log file path unavailable".to_string())?;
    if let Some(parent) = path.parent() {
        create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    {
        let mut guard = LOG_FILE.lock();
        *guard = None;
    }
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    ensure_file_logger();
    Ok(())
}

fn log_file_path() -> Option<PathBuf> {
    ProjectDirs::from("com", "OSCMIDI", "OSCMIDI").map(|dirs| dirs.config_dir().join("app.log"))
}

fn append_file_log(level: &str, message: &str) {
    if let Some(file) = LOG_FILE.lock().as_mut() {
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let _ = writeln!(file, "[{ts}] {level}: {message}");
    }
}
