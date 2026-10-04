use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use eldenring::fd4::FD4PadManager;
use fromsoftware_shared::FromStatic;
use hudhook::windows::{
    Win32::{
        Devices::HumanInterfaceDevice::{GUID_SysKeyboard, GUID_SysMouse, IDirectInput8W},
        System::LibraryLoader::{GetModuleHandleW, GetProcAddress},
        UI::{
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
                KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput,
                VIRTUAL_KEY,
            },
            WindowsAndMessaging::{
                CURSORINFO, ClipCursor, GetCursorInfo, GetForegroundWindow,
                GetWindowThreadProcessId,
            },
        },
    },
    core::{GUID, Interface, s, w},
};
use ilhook::x64::{CallbackOption, HookFlags, Registers, hook_closure_retn};

use crate::{logf, state::MENU_OPEN};

pub fn key_down(vk: u16) -> bool {
    (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0
}

pub fn is_foreground() -> bool {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid)) };
    pid == std::process::id()
}

pub fn cursor_visible() -> bool {
    let mut info = CURSORINFO {
        cbSize: size_of::<CURSORINFO>() as u32,
        ..Default::default()
    };
    // CURSOR_SHOWING = 1
    unsafe { GetCursorInfo(&mut info) }.is_ok() && info.flags.0 & 1 != 0
}

pub struct Keys {
    prev: [bool; 256],
}

impl Keys {
    pub fn new() -> Self {
        Self { prev: [false; 256] }
    }

    /// True on the tick the key goes down.
    pub fn pressed(&mut self, vk: u16) -> bool {
        let idx = (vk & 0xFF) as usize;
        let down = key_down(vk);
        let edge = down && !self.prev[idx];
        self.prev[idx] = down;
        edge
    }

    pub fn sync_all(&mut self) {
        for vk in 1..255u16 {
            self.prev[vk as usize] = key_down(vk);
        }
    }

    /// First key that went down since the previous scan. Left and right mouse buttons and the
    /// generic Shift/Ctrl/Alt codes are skipped.
    pub fn scan_new(&mut self) -> Option<u16> {
        let mut found = None;
        for vk in 4..255u16 {
            let down = key_down(vk);
            let edge = down && !self.prev[vk as usize];
            self.prev[vk as usize] = down;
            if edge && found.is_none() && !(0x10..=0x12).contains(&vk) {
                found = Some(vk);
            }
        }
        found
    }
}

const HOLD: Duration = Duration::from_millis(80);
const GAP: Duration = Duration::from_millis(40);

/// Presses keyboard keys for the game, one at a time.
pub struct Injector {
    queue: VecDeque<u16>,
    held: Option<(u16, Instant)>,
    next_at: Instant,
}

impl Injector {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            held: None,
            next_at: Instant::now(),
        }
    }

    pub fn press(&mut self, vk: u16) {
        if self.queue.len() < 4 {
            self.queue.push_back(vk);
        }
    }

    pub fn tick(&mut self, now: Instant) {
        if let Some((vk, since)) = self.held {
            if now.duration_since(since) >= HOLD {
                send_key(vk, true);
                self.held = None;
                self.next_at = now + GAP;
            }
            return;
        }
        if now < self.next_at {
            return;
        }
        if let Some(vk) = self.queue.pop_front() {
            send_key(vk, false);
            self.held = Some((vk, now));
        }
    }
}

fn send_key(vk: u16, up: bool) {
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16;
    if scan == 0 {
        logf!("input: no scancode for key 0x{vk:02X}");
        return;
    }
    let mut flags = KEYEVENTF_SCANCODE;
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    // Navigation block, right Ctrl/Alt and numpad divide live on the extended scancode page.
    if matches!(vk, 0x21..=0x28 | 0x2D | 0x2E | 0xA3 | 0xA5 | 0x6F) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe { SendInput(&[input], size_of::<INPUT>() as i32) };
}

// --- Input block: nothing reaches the game while the settings window is open ---

/// Tells the game it is in the background, which makes it skip keyboard, mouse and pad input.
/// The game clears the flags by itself one frame after the last call, so nothing has to be
/// restored when the window closes.
pub fn hold_game_input() {
    if let Ok(pad) = unsafe { FD4PadManager::instance_mut() } {
        pad.exit_foreground_signaled = true;
        pad.is_back_ground_window = true;
    }
}

/// The game clips the cursor to its window while playing; the settings window needs it free.
pub fn release_cursor() {
    let _ = unsafe { ClipCursor(None) };
}

const DIRECTINPUT_VERSION: u32 = 0x0800;
const VTBL_RELEASE: usize = 2;
const VTBL_CREATE_DEVICE: usize = 3;
const VTBL_GET_DEVICE_STATE: usize = 9;

type RawObj = *mut *const usize;
type DInput8CreateFn =
    unsafe extern "system" fn(usize, u32, *const GUID, *mut RawObj, usize) -> i32;
type CreateDeviceFn = unsafe extern "system" fn(RawObj, *const GUID, *mut RawObj, usize) -> i32;
type ReleaseFn = unsafe extern "system" fn(RawObj) -> u32;
type GetDeviceStateFn = unsafe extern "system" fn(u64, u64, u64) -> usize;

static HOOK_SEEN: AtomicBool = AtomicBool::new(false);

/// Address of the real `GetDeviceState` behind a throwaway device of the given kind.
unsafe fn probe_get_device_state(
    create: DInput8CreateFn,
    hinstance: usize,
    guid: &GUID,
) -> Option<usize> {
    unsafe {
        let mut di8: RawObj = std::ptr::null_mut();
        let hr = create(hinstance, DIRECTINPUT_VERSION, &IDirectInput8W::IID, &mut di8, 0);
        if hr != 0 || di8.is_null() {
            logf!("dinput: DirectInput8Create failed: {hr:#010x}");
            return None;
        }
        let release_di8: ReleaseFn = std::mem::transmute(*(*di8).add(VTBL_RELEASE));
        let create_device: CreateDeviceFn = std::mem::transmute(*(*di8).add(VTBL_CREATE_DEVICE));

        let mut device: RawObj = std::ptr::null_mut();
        let hr = create_device(di8, guid, &mut device, 0);
        if hr != 0 || device.is_null() {
            logf!("dinput: CreateDevice failed: {hr:#010x}");
            release_di8(di8);
            return None;
        }
        let addr = *(*device).add(VTBL_GET_DEVICE_STATE);
        let release_device: ReleaseFn = std::mem::transmute(*(*device).add(VTBL_RELEASE));
        release_device(device);
        release_di8(di8);
        Some(addr)
    }
}

// IDirectInputDevice8::GetDeviceState(cbData, lpvData): rcx = this, rdx = cbData, r8 = lpvData
fn get_device_state_hook(reg: *mut Registers, original: usize) -> usize {
    let (this, size, data) = unsafe { ((*reg).rcx, (*reg).rdx, (*reg).r8) };
    let original: GetDeviceStateFn = unsafe { std::mem::transmute(original) };
    let hr = unsafe { original(this, size, data) };
    // 256 = keyboard state, 16 / 20 = DIMOUSESTATE / DIMOUSESTATE2; other devices are left alone
    let size = size as u32 as usize;
    if hr as u32 == 0 && data != 0 && matches!(size, 256 | 16 | 20) && MENU_OPEN.load(Ordering::Relaxed) {
        unsafe { std::ptr::write_bytes(data as *mut u8, 0, size) };
        if !HOOK_SEEN.swap(true, Ordering::Relaxed) {
            logf!("dinput: the game reads devices through the hooked GetDeviceState");
        }
    }
    hr
}

/// Hooks the code of `GetDeviceState` for the system keyboard and mouse, so every device of the
/// process is covered no matter which vtable it was created with.
pub fn install_dinput_block() {
    let create: DInput8CreateFn = unsafe {
        let Ok(dinput8) = GetModuleHandleW(w!("dinput8.dll")) else {
            logf!("dinput: dinput8.dll is not loaded");
            return;
        };
        let Some(f) = GetProcAddress(dinput8, s!("DirectInput8Create")) else {
            logf!("dinput: DirectInput8Create not found");
            return;
        };
        std::mem::transmute(f)
    };
    let hinstance = match unsafe { GetModuleHandleW(None) } {
        Ok(h) => h.0 as usize,
        Err(e) => {
            logf!("dinput: GetModuleHandleW failed: {e}");
            return;
        }
    };

    let mut addrs: Vec<usize> = Vec::new();
    for guid in [&GUID_SysKeyboard, &GUID_SysMouse] {
        if let Some(addr) = unsafe { probe_get_device_state(create, hinstance, guid) } {
            if !addrs.contains(&addr) {
                addrs.push(addr);
            }
        }
    }

    for addr in addrs {
        let hook = unsafe {
            hook_closure_retn(addr, get_device_state_hook, CallbackOption::None, HookFlags::empty())
        };
        match hook {
            // Dropping the hook point would remove the hook.
            Ok(point) => {
                std::mem::forget(point);
                logf!("dinput: GetDeviceState hooked");
            }
            Err(e) => logf!("dinput: GetDeviceState hook failed: {e:?}"),
        }
    }
}
