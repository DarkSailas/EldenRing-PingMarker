use std::{
    collections::HashMap,
    ffi::{CStr, c_char, c_void},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use hudhook::windows::{
    Win32::{
        Foundation::HMODULE,
        System::{
            LibraryLoader::{GetModuleHandleA, GetProcAddress},
            Memory::{
                MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS,
                PAGE_PROTECTION_FLAGS, PAGE_READWRITE, VirtualProtect, VirtualQuery,
            },
        },
    },
    core::PCSTR,
};

use crate::{config::Config, logf};

/// True when `len` bytes at `ptr` sit inside one committed, accessible region.
pub fn readable(ptr: *const c_void, len: usize) -> bool {
    if (ptr as usize) < 0x10000 {
        return false;
    }
    let mut mbi = MEMORY_BASIC_INFORMATION::default();
    let got = unsafe { VirtualQuery(Some(ptr), &mut mbi, size_of::<MEMORY_BASIC_INFORMATION>()) };
    if got == 0 || mbi.State != MEM_COMMIT {
        return false;
    }
    if mbi.Protect.0 == 0 || mbi.Protect.0 & (PAGE_NOACCESS.0 | PAGE_GUARD.0) != 0 {
        return false;
    }
    ptr as usize + len <= mbi.BaseAddress as usize + mbi.RegionSize
}

// --- wire format ---

pub const PACKET_SIZE: usize = 60;
const MAGIC: &[u8; 4] = b"ERPM";
const VERSION: u8 = 1;
const KIND_HELLO: u8 = 0;
const KIND_PING: u8 = 1;
const NAME_MAX: usize = 32;

#[derive(Clone)]
pub struct Ping {
    pub symbol: u8,
    pub block: u32,
    pub local: [f32; 3],
    pub color: [f32; 3],
    pub duration: f32,
    pub name: String,
}

pub struct Incoming {
    pub sender: u64,
    pub ping: Ping,
}

fn encode(kind: u8, ping: Option<&Ping>) -> [u8; PACKET_SIZE] {
    let mut b = [0u8; PACKET_SIZE];
    b[0..4].copy_from_slice(MAGIC);
    b[4] = VERSION;
    b[5] = kind;
    if let Some(p) = ping {
        b[6] = p.symbol;
        let mut n = p.name.len().min(NAME_MAX);
        while !p.name.is_char_boundary(n) {
            n -= 1;
        }
        b[7] = n as u8;
        b[8..12].copy_from_slice(&p.block.to_le_bytes());
        for i in 0..3 {
            b[12 + i * 4..16 + i * 4].copy_from_slice(&p.local[i].to_le_bytes());
            b[24 + i] = (p.color[i].clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        b[27] = p.duration.clamp(2.0, 60.0).round() as u8;
        b[28..28 + n].copy_from_slice(&p.name.as_bytes()[..n]);
    }
    b
}

/// Returns the packet kind and, for pings, the payload. Anything malformed is dropped.
fn decode(b: &[u8]) -> Option<(u8, Option<Ping>)> {
    if b.len() != PACKET_SIZE || &b[0..4] != MAGIC || b[4] != VERSION {
        return None;
    }
    match b[5] {
        KIND_HELLO => Some((KIND_HELLO, None)),
        KIND_PING => {
            let f = |o: usize| f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
            let local = [f(12), f(16), f(20)];
            if local.iter().any(|v| !v.is_finite() || v.abs() > 100_000.0) {
                return None;
            }
            let n = (b[7] as usize).min(NAME_MAX);
            let name: String = String::from_utf8_lossy(&b[28..28 + n])
                .chars()
                .filter(|c| !c.is_control() && *c != '\u{fffd}')
                .take(24)
                .collect();
            Some((
                KIND_PING,
                Some(Ping {
                    symbol: b[6] % crate::config::SYMBOL_COUNT,
                    block: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
                    local,
                    color: [b[24], b[25], b[26]].map(|v| v as f32 / 255.0),
                    duration: (b[27] as f32).clamp(2.0, 60.0),
                    name,
                }),
            ))
        }
        _ => None,
    }
}

// --- Steam flat API ---

/// SteamNetworkingIdentity: type, size, 128 bytes of payload. Type 16 is a SteamID64.
#[repr(C)]
struct Identity {
    e_type: i32,
    cb_size: i32,
    data: [u8; 128],
}

impl Identity {
    fn steam(id: u64) -> Self {
        let mut data = [0u8; 128];
        data[..8].copy_from_slice(&id.to_le_bytes());
        Self {
            e_type: 16,
            cb_size: 8,
            data,
        }
    }
}

type SendFn = unsafe extern "C" fn(*mut c_void, *const Identity, *const c_void, u32, i32, i32) -> i32;
type RecvFn = unsafe extern "C" fn(*mut c_void, i32, *mut *mut c_void, i32) -> i32;
type AcceptFn = unsafe extern "C" fn(*mut c_void, *const Identity) -> bool;
type ReleaseFn = unsafe extern "C" fn(*mut c_void);

struct Api {
    iface: usize,
    send: SendFn,
    recv: RecvFn,
    accept: AcceptFn,
    release: ReleaseFn,
}

const CHANNEL: i32 = 0x5047;
// Reliable | AutoRestartBrokenSession
const SEND_FLAGS: i32 = 8 | 32;

unsafe fn sym(module: HMODULE, name: &[u8]) -> Option<usize> {
    unsafe { GetProcAddress(module, PCSTR(name.as_ptr())) }.map(|f| f as usize)
}

unsafe fn load_api() -> Option<(Api, u64, String)> {
    unsafe {
        let module = GetModuleHandleA(PCSTR(b"steam_api64.dll\0".as_ptr())).ok()?;
        type Getter = unsafe extern "C" fn() -> *mut c_void;

        let get: Getter =
            std::mem::transmute(sym(module, b"SteamAPI_SteamNetworkingMessages_SteamAPI_v002\0")?);
        let iface = get();
        if iface.is_null() {
            return None;
        }
        let api = Api {
            iface: iface as usize,
            send: std::mem::transmute::<usize, SendFn>(sym(
                module,
                b"SteamAPI_ISteamNetworkingMessages_SendMessageToUser\0",
            )?),
            recv: std::mem::transmute::<usize, RecvFn>(sym(
                module,
                b"SteamAPI_ISteamNetworkingMessages_ReceiveMessagesOnChannel\0",
            )?),
            accept: std::mem::transmute::<usize, AcceptFn>(sym(
                module,
                b"SteamAPI_ISteamNetworkingMessages_AcceptSessionWithUser\0",
            )?),
            release: std::mem::transmute::<usize, ReleaseFn>(sym(
                module,
                b"SteamAPI_SteamNetworkingMessage_t_Release\0",
            )?),
        };

        let get_user: Getter = std::mem::transmute(sym(module, b"SteamAPI_SteamUser_v021\0")?);
        let user = get_user();
        if user.is_null() {
            return None;
        }
        let get_id: unsafe extern "C" fn(*mut c_void) -> u64 =
            std::mem::transmute(sym(module, b"SteamAPI_ISteamUser_GetSteamID\0")?);
        let my_id = get_id(user);

        let mut name = String::new();
        if let (Some(f), Some(g)) = (
            sym(module, b"SteamAPI_SteamFriends_v017\0"),
            sym(module, b"SteamAPI_ISteamFriends_GetPersonaName\0"),
        ) {
            let get_friends: Getter = std::mem::transmute(f);
            let get_name: unsafe extern "C" fn(*mut c_void) -> *const c_char = std::mem::transmute(g);
            let friends = get_friends();
            if !friends.is_null() {
                let p = get_name(friends);
                if !p.is_null() {
                    name = CStr::from_ptr(p).to_string_lossy().chars().take(24).collect();
                }
            }
        }
        Some((api, my_id, name))
    }
}

// --- peer discovery through the game's own sends ---

static ORIG_SEND: AtomicUsize = AtomicUsize::new(0);
static OWN_SEND: AtomicBool = AtomicBool::new(false);
static SEEN: Mutex<Vec<(u64, Instant)>> = Mutex::new(Vec::new());

unsafe extern "system" fn send_hook(
    this: *mut c_void,
    identity: *const Identity,
    data: *const c_void,
    size: u32,
    flags: i32,
    channel: i32,
) -> i32 {
    unsafe {
        if !OWN_SEND.load(Ordering::Relaxed) && !identity.is_null() && (*identity).e_type == 16 {
            let id = std::ptr::read_unaligned((*identity).data.as_ptr() as *const u64);
            if let Ok(mut seen) = SEEN.try_lock() {
                let now = Instant::now();
                if let Some(pos) = seen.iter().position(|(i, _)| *i == id) {
                    seen[pos].1 = now;
                } else if seen.len() < 32 {
                    seen.push((id, now));
                }
            }
        }
        let orig: SendFn = std::mem::transmute(ORIG_SEND.load(Ordering::Relaxed));
        orig(this, identity, data, size, flags, channel)
    }
}

/// SendMessageToUser is the first virtual method of ISteamNetworkingMessages.
unsafe fn install_send_hook(iface: usize) -> bool {
    unsafe {
        let vtable = *(iface as *const *mut usize);
        if !readable(vtable as *const c_void, 8) {
            return false;
        }
        let current = vtable.read();
        if current == send_hook as *const () as usize {
            return true;
        }
        let mut old = PAGE_PROTECTION_FLAGS(0);
        if VirtualProtect(vtable as *const c_void, 8, PAGE_READWRITE, &mut old).is_err() {
            return false;
        }
        ORIG_SEND.store(current, Ordering::SeqCst);
        vtable.write(send_hook as *const () as usize);
        let mut tmp = PAGE_PROTECTION_FLAGS(0);
        let _ = VirtualProtect(vtable as *const c_void, 8, old, &mut tmp);
        true
    }
}

// --- session ---

struct Peer {
    last_seen: Instant,
    has_mod: bool,
    hello_at: Option<Instant>,
    last_ping: Option<Instant>,
}

pub struct Net {
    api: Option<Api>,
    tried: Option<Instant>,
    hook_tried: bool,
    my_id: u64,
    my_name: String,
    peers: HashMap<u64, Peer>,
    accept_at: Option<Instant>,
}

const PEER_TTL: Duration = Duration::from_secs(60);
const HELLO_EVERY: Duration = Duration::from_secs(30);
const ACCEPT_EVERY: Duration = Duration::from_secs(5);
const RETRY_EVERY: Duration = Duration::from_secs(5);
const PING_MIN_GAP: Duration = Duration::from_millis(200);

impl Net {
    pub fn new() -> Self {
        Self {
            api: None,
            tried: None,
            hook_tried: false,
            my_id: 0,
            my_name: String::new(),
            peers: HashMap::new(),
            accept_at: None,
        }
    }

    pub fn ready(&self) -> bool {
        self.api.is_some()
    }

    pub fn my_id(&self) -> u64 {
        self.my_id
    }

    pub fn my_name(&self) -> &str {
        &self.my_name
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    pub fn mod_count(&self) -> usize {
        self.peers.values().filter(|p| p.has_mod).count()
    }

    fn send(api: &Api, to: u64, packet: &[u8; PACKET_SIZE]) -> i32 {
        let ident = Identity::steam(to);
        OWN_SEND.store(true, Ordering::Relaxed);
        let r = unsafe {
            (api.send)(
                api.iface as *mut c_void,
                &ident,
                packet.as_ptr() as *const c_void,
                PACKET_SIZE as u32,
                SEND_FLAGS,
                CHANNEL,
            )
        };
        OWN_SEND.store(false, Ordering::Relaxed);
        r
    }

    /// `session_ids` are the Steam ids found on the other player characters this second.
    pub fn tick(&mut self, now: Instant, cfg: &Config, session_ids: &[u64]) -> Vec<Incoming> {
        let mut out = Vec::new();
        if !cfg.net_enabled {
            return out;
        }
        if self.api.is_none() {
            if self.tried.is_some_and(|t| now.duration_since(t) < RETRY_EVERY) {
                return out;
            }
            self.tried = Some(now);
            match unsafe { load_api() } {
                Some((api, id, name)) => {
                    logf!("net: Steam messages interface ready, own name length {}", name.len());
                    self.api = Some(api);
                    self.my_id = id;
                    self.my_name = name;
                }
                None => return out,
            }
        }
        let Some(api) = self.api.as_ref() else {
            return out;
        };

        if cfg.peer_hook && !self.hook_tried {
            self.hook_tried = true;
            logf!("net: peer discovery hook installed: {}", unsafe {
                install_send_hook(api.iface)
            });
        }

        let mut found: Vec<(u64, Instant)> = session_ids.iter().map(|id| (*id, now)).collect();
        if let Ok(seen) = SEEN.try_lock() {
            found.extend(seen.iter().copied());
        }
        for (id, at) in found {
            if id == 0 || id == self.my_id || now.saturating_duration_since(at) > PEER_TTL {
                continue;
            }
            let peer = self.peers.entry(id).or_insert_with(|| {
                logf!("net: new peer");
                Peer {
                    last_seen: at,
                    has_mod: false,
                    hello_at: None,
                    last_ping: None,
                }
            });
            if at > peer.last_seen {
                peer.last_seen = at;
            }
        }
        self.peers
            .retain(|_, p| now.saturating_duration_since(p.last_seen) <= PEER_TTL);

        let hello = encode(KIND_HELLO, None);
        for (id, peer) in self.peers.iter_mut() {
            if peer.hello_at.is_none_or(|t| now.duration_since(t) >= HELLO_EVERY) {
                peer.hello_at = Some(now);
                let r = Self::send(api, *id, &hello);
                if r != 1 {
                    logf!("net: hello send result {r}");
                }
            }
        }

        if self.accept_at.is_none_or(|t| now.duration_since(t) >= ACCEPT_EVERY) {
            self.accept_at = Some(now);
            for id in self.peers.keys() {
                let ident = Identity::steam(*id);
                unsafe { (api.accept)(api.iface as *mut c_void, &ident) };
            }
        }

        for _ in 0..4 {
            let mut msgs = [std::ptr::null_mut::<c_void>(); 16];
            let n = unsafe { (api.recv)(api.iface as *mut c_void, CHANNEL, msgs.as_mut_ptr(), 16) };
            if n <= 0 {
                break;
            }
            for &msg in msgs.iter().take(n as usize) {
                if msg.is_null() {
                    continue;
                }
                // SteamNetworkingMessage_t: data @0, size @8, connection @12, peer identity @16.
                let (data, size, id_type, sender) = unsafe {
                    let p = msg as *const u8;
                    (
                        std::ptr::read_unaligned(p as *const *const u8),
                        std::ptr::read_unaligned(p.add(8) as *const i32),
                        std::ptr::read_unaligned(p.add(16) as *const i32),
                        std::ptr::read_unaligned(p.add(24) as *const u64),
                    )
                };
                if id_type == 16 && size as usize == PACKET_SIZE && !data.is_null() {
                    let bytes = unsafe { std::slice::from_raw_parts(data, PACKET_SIZE) };
                    if let (Some(peer), Some((_, ping))) = (self.peers.get_mut(&sender), decode(bytes)) {
                        peer.has_mod = true;
                        peer.last_seen = now;
                        if let Some(ping) = ping {
                            if peer.last_ping.is_none_or(|t| now.duration_since(t) >= PING_MIN_GAP) {
                                peer.last_ping = Some(now);
                                out.push(Incoming { sender, ping });
                            }
                        }
                    }
                }
                unsafe { (api.release)(msg) };
            }
            if n < 16 {
                break;
            }
        }
        out
    }

    /// Sends the ping to every peer that answered with the mod's hello. Returns how many got it.
    pub fn send_ping(&mut self, ping: &Ping) -> usize {
        let Some(api) = self.api.as_ref() else {
            return 0;
        };
        let packet = encode(KIND_PING, Some(ping));
        let mut sent = 0;
        for (id, peer) in self.peers.iter() {
            if peer.has_mod && Self::send(api, *id, &packet) == 1 {
                sent += 1;
            }
        }
        sent
    }
}
