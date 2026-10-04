use std::{
    collections::VecDeque,
    ffi::c_void,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

use hudhook::windows::{
    Win32::{
        Devices::HumanInterfaceDevice::{
            DirectInput8Create, GUID_SysKeyboard, IDirectInput8A, IDirectInput8W,
            IDirectInputDevice8A, IDirectInputDevice8W,
        },
        Foundation::HINSTANCE,
        System::{
            LibraryLoader::GetModuleHandleW,
            Memory::{PAGE_PROTECTION_FLAGS, PAGE_READWRITE, VirtualProtect},
        },
        UI::{
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
                KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput,
                VIRTUAL_KEY,
            },
            WindowsAndMessaging::{
                CURSORINFO, GetCursorInfo, GetForegroundWindow, GetWindowThreadProcessId,
            },
        },
    },
    core::Interface,
};

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

// --- DirectInput: hide keyboard and mouse from the game while the settings window is open ---

type GetDeviceStateFn = unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32;
type GetDeviceDataFn =
    unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut u32, u32) -> i32;

const SLOT_GET_DEVICE_STATE: usize = 9;
const SLOT_GET_DEVICE_DATA: usize = 10;

// Index 0: ANSI device vtable, 1: Unicode device vtable.
static ORIG_STATE: [AtomicUsize; 2] = [AtomicUsize::new(0), AtomicUsize::new(0)];
static ORIG_DATA: [AtomicUsize; 2] = [AtomicUsize::new(0), AtomicUsize::new(0)];

unsafe extern "system" fn get_device_state_hook<const N: usize>(
    this: *mut c_void,
    size: u32,
    data: *mut c_void,
) -> i32 {
    let orig: GetDeviceStateFn =
        unsafe { std::mem::transmute(ORIG_STATE[N].load(Ordering::Relaxed)) };
    let hr = unsafe { orig(this, size, data) };
    if hr >= 0 && !data.is_null() && MENU_OPEN.load(Ordering::Relaxed) {
        unsafe { std::ptr::write_bytes(data as *mut u8, 0, size as usize) };
    }
    hr
}

unsafe extern "system" fn get_device_data_hook<const N: usize>(
    this: *mut c_void,
    object_size: u32,
    objects: *mut c_void,
    in_out: *mut u32,
    flags: u32,
) -> i32 {
    let orig: GetDeviceDataFn =
        unsafe { std::mem::transmute(ORIG_DATA[N].load(Ordering::Relaxed)) };
    let hr = unsafe { orig(this, object_size, objects, in_out, flags) };
    if hr >= 0 && !in_out.is_null() && MENU_OPEN.load(Ordering::Relaxed) {
        unsafe { *in_out = 0 };
    }
    hr
}

fn is_our_hook(addr: usize) -> bool {
    addr == get_device_state_hook::<0> as *const () as usize
        || addr == get_device_state_hook::<1> as *const () as usize
        || addr == get_device_data_hook::<0> as *const () as usize
        || addr == get_device_data_hook::<1> as *const () as usize
}

/// Replaces one vtable entry. The original goes into `orig` before the swap so the hook never
/// runs without it.
unsafe fn patch_slot(vtable: *mut usize, slot: usize, hook: usize, orig: &AtomicUsize) -> bool {
    unsafe {
        let entry = vtable.add(slot);
        let current = entry.read();
        if is_our_hook(current) {
            return true;
        }
        let mut old = PAGE_PROTECTION_FLAGS(0);
        if VirtualProtect(entry as *const c_void, 8, PAGE_READWRITE, &mut old).is_err() {
            return false;
        }
        orig.store(current, Ordering::SeqCst);
        entry.write(hook);
        let mut tmp = PAGE_PROTECTION_FLAGS(0);
        let _ = VirtualProtect(entry as *const c_void, 8, old, &mut tmp);
        true
    }
}

unsafe fn patch_device(raw: *mut c_void, n: usize) -> bool {
    unsafe {
        let vtable = *(raw as *const *mut usize);
        let (state, data) = if n == 0 {
            (
                get_device_state_hook::<0> as *const () as usize,
                get_device_data_hook::<0> as *const () as usize,
            )
        } else {
            (
                get_device_state_hook::<1> as *const () as usize,
                get_device_data_hook::<1> as *const () as usize,
            )
        };
        patch_slot(vtable, SLOT_GET_DEVICE_STATE, state, &ORIG_STATE[n])
            && patch_slot(vtable, SLOT_GET_DEVICE_DATA, data, &ORIG_DATA[n])
    }
}

/// Creates throwaway keyboard devices to reach the device vtables inside dinput8.dll and patches
/// GetDeviceState/GetDeviceData there. Every device of the process shares those vtables.
pub fn install_dinput_block() {
    let hinst: HINSTANCE = match unsafe { GetModuleHandleW(None) } {
        Ok(h) => h.into(),
        Err(e) => {
            logf!("dinput: GetModuleHandleW failed: {e}");
            return;
        }
    };

    unsafe {
        let mut raw: *mut c_void = std::ptr::null_mut();
        match DirectInput8Create(hinst, 0x0800, &IDirectInput8A::IID, &mut raw, None) {
            Ok(()) if !raw.is_null() => {
                let di = IDirectInput8A::from_raw(raw);
                let mut dev: Option<IDirectInputDevice8A> = None;
                match di.CreateDevice(&GUID_SysKeyboard, &mut dev, None) {
                    Ok(()) => {
                        if let Some(dev) = dev.as_ref() {
                            logf!("dinput: ANSI device patched: {}", patch_device(dev.as_raw(), 0));
                        }
                    }
                    Err(e) => logf!("dinput: CreateDevice (ANSI) failed: {e}"),
                }
            }
            Ok(()) => logf!("dinput: DirectInput8Create (ANSI) returned null"),
            Err(e) => logf!("dinput: DirectInput8Create (ANSI) failed: {e}"),
        }

        let mut raw: *mut c_void = std::ptr::null_mut();
        match DirectInput8Create(hinst, 0x0800, &IDirectInput8W::IID, &mut raw, None) {
            Ok(()) if !raw.is_null() => {
                let di = IDirectInput8W::from_raw(raw);
                let mut dev: Option<IDirectInputDevice8W> = None;
                match di.CreateDevice(&GUID_SysKeyboard, &mut dev, None) {
                    Ok(()) => {
                        if let Some(dev) = dev.as_ref() {
                            logf!("dinput: Unicode device patched: {}", patch_device(dev.as_raw(), 1));
                        }
                    }
                    Err(e) => logf!("dinput: CreateDevice (Unicode) failed: {e}"),
                }
            }
            Ok(()) => logf!("dinput: DirectInput8Create (Unicode) returned null"),
            Err(e) => logf!("dinput: DirectInput8Create (Unicode) failed: {e}"),
        }
    }
}
