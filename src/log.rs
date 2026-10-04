use std::{
    fs::File,
    io::Write,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

static ENABLED: AtomicBool = AtomicBool::new(false);
static FILE: Mutex<Option<(File, Instant)>> = Mutex::new(None);

pub fn init(path: &Path, enabled: bool) {
    if let Ok(file) = File::create(path) {
        *FILE.lock().unwrap_or_else(|e| e.into_inner()) = Some((file, Instant::now()));
    }
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn write(args: std::fmt::Arguments) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let mut guard = FILE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((file, start)) = guard.as_mut() {
        let _ = writeln!(file, "[{:9.3}] {}", start.elapsed().as_secs_f32(), args);
    }
}

#[macro_export]
macro_rules! logf {
    ($($arg:tt)*) => {
        $crate::log::write(format_args!($($arg)*))
    };
}
