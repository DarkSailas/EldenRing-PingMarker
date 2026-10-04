use std::{
    path::PathBuf,
    sync::{
        LazyLock, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Instant,
};

use hudhook::windows::Win32::{Foundation::HMODULE, System::LibraryLoader::GetModuleFileNameW};

use crate::config::Config;

/// Read by the DirectInput hooks and the overlay's message filter.
pub static MENU_OPEN: AtomicBool = AtomicBool::new(false);
/// Set after a panic inside the mod: the game task and the overlay stop doing anything.
pub static DISABLED: AtomicBool = AtomicBool::new(false);

static MODULE: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KeyTarget {
    Ping,
    Quick,
    Menu,
    GameInteract,
    GameUse,
}

impl KeyTarget {
    pub fn is_game_key(self) -> bool {
        matches!(self, Self::GameInteract | Self::GameUse)
    }
}

pub struct Marker {
    pub sender: u64,
    pub name: String,
    pub symbol: u8,
    pub color: [f32; 3],
    /// Raw block id of the sender at the moment of the ping.
    pub block: u32,
    /// Position relative to that block.
    pub local: [f32; 3],
    pub born: Instant,
    pub duration: f32,
    /// Position in the local physics space, recomputed every game tick.
    pub havok: Option<[f32; 3]>,
    pub distance: f32,
}

pub struct Shared {
    pub cfg: Config,
    pub markers: Vec<Marker>,
    pub capture: Option<KeyTarget>,
    pub dirty: Option<Instant>,
    pub toast: Option<(String, Instant)>,
    pub player_pos: Option<[f32; 3]>,
    pub net_ready: bool,
    pub peers: usize,
    pub peers_with_mod: usize,
}

static SHARED: LazyLock<Mutex<Shared>> = LazyLock::new(|| {
    Mutex::new(Shared {
        cfg: Config::default(),
        markers: Vec::new(),
        capture: None,
        dirty: None,
        toast: None,
        player_pos: None,
        net_ready: false,
        peers: 0,
        peers_with_mod: 0,
    })
});

pub fn shared() -> MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn toast(text: String) {
    shared().toast = Some((text, Instant::now()));
}

pub fn set_module(raw: usize) {
    MODULE.store(raw, Ordering::Relaxed);
}

fn dll_dir() -> PathBuf {
    let mut buf = [0u16; 1024];
    let module = HMODULE(MODULE.load(Ordering::Relaxed) as _);
    let len = unsafe { GetModuleFileNameW(Some(module), &mut buf) } as usize;
    let mut path = PathBuf::from(String::from_utf16_lossy(&buf[..len.min(buf.len())]));
    path.pop();
    path
}

pub fn ini_path() -> PathBuf {
    dll_dir().join("er_ping_marker.ini")
}

pub fn log_path() -> PathBuf {
    dll_dir().join("er_ping_marker.log")
}
