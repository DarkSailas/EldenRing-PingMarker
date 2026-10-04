use std::{fs, path::Path};

#[derive(Clone)]
pub struct Config {
    pub ping_key: u16,
    pub quick_key: u16,
    pub menu_key: u16,
    pub game_interact_key: u16,
    pub game_use_key: u16,

    pub marker_enabled: bool,
    pub duration: f32,
    pub symbol: u8,
    pub color: [f32; 3],
    pub show_name: bool,
    pub show_distance: bool,
    pub scale: f32,
    pub mirror_x: bool,

    pub quick_enabled: bool,

    pub ru: bool,
    pub ignore_cursor: bool,
    pub log: bool,

    pub net_enabled: bool,
    pub peer_hook: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ping_key: 0x56,
            quick_key: 0x31,
            menu_key: 0x76,
            game_interact_key: 0x45,
            game_use_key: 0x52,
            marker_enabled: true,
            duration: 10.0,
            symbol: 0,
            color: [0.61, 0.82, 0.23],
            show_name: true,
            show_distance: true,
            scale: 1.0,
            mirror_x: false,
            quick_enabled: true,
            ru: true,
            ignore_cursor: true,
            log: true,
            net_enabled: true,
            peer_hook: true,
        }
    }
}

pub const SYMBOL_COUNT: u8 = 5;

const NAMED_KEYS: &[(&str, u16)] = &[
    ("Mouse3", 0x04),
    ("Mouse4", 0x05),
    ("Mouse5", 0x06),
    ("Backspace", 0x08),
    ("Tab", 0x09),
    ("Enter", 0x0D),
    ("CapsLock", 0x14),
    ("Esc", 0x1B),
    ("Space", 0x20),
    ("PageUp", 0x21),
    ("PageDown", 0x22),
    ("End", 0x23),
    ("Home", 0x24),
    ("Left", 0x25),
    ("Up", 0x26),
    ("Right", 0x27),
    ("Down", 0x28),
    ("Insert", 0x2D),
    ("Delete", 0x2E),
    ("NumMul", 0x6A),
    ("NumAdd", 0x6B),
    ("NumSub", 0x6D),
    ("NumDot", 0x6E),
    ("NumDiv", 0x6F),
    ("LShift", 0xA0),
    ("RShift", 0xA1),
    ("LCtrl", 0xA2),
    ("RCtrl", 0xA3),
    ("LAlt", 0xA4),
    ("RAlt", 0xA5),
    ("Semicolon", 0xBA),
    ("Equals", 0xBB),
    ("Comma", 0xBC),
    ("Minus", 0xBD),
    ("Period", 0xBE),
    ("Slash", 0xBF),
    ("Tilde", 0xC0),
    ("LBracket", 0xDB),
    ("Backslash", 0xDC),
    ("RBracket", 0xDD),
    ("Quote", 0xDE),
];

pub fn key_name(vk: u16) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => ((vk as u8) as char).to_string(),
        0x60..=0x69 => format!("Num{}", vk - 0x60),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        _ => NAMED_KEYS
            .iter()
            .find(|(_, v)| *v == vk)
            .map(|(n, _)| (*n).to_string())
            .unwrap_or_else(|| format!("0x{vk:02X}")),
    }
}

pub fn parse_key(s: &str) -> Option<u16> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let up = s.to_ascii_uppercase();
    if up.len() == 1 {
        let c = up.as_bytes()[0];
        if c.is_ascii_digit() || c.is_ascii_uppercase() {
            return Some(c as u16);
        }
    }
    if let Some(hex) = up.strip_prefix("0X") {
        return u16::from_str_radix(hex, 16).ok().filter(|v| (1..=254).contains(v));
    }
    if let Some(n) = up.strip_prefix("NUM").and_then(|n| n.parse::<u16>().ok()) {
        return (n <= 9).then_some(0x60 + n);
    }
    if let Some(n) = up.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()) {
        return (1..=24).contains(&n).then_some(0x6F + n);
    }
    NAMED_KEYS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(s))
        .map(|(_, v)| *v)
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_color(s: &str) -> Option<[f32; 3]> {
    let mut it = s.split(',').map(|p| p.trim().parse::<f32>());
    let c = [it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?];
    Some(c.map(|v| (v / 255.0).clamp(0.0, 1.0)))
}

impl Config {
    pub fn load(path: &Path) -> Self {
        let mut cfg = Self::default();
        let Ok(text) = fs::read_to_string(path) else {
            return cfg;
        };
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim().trim_start_matches('\u{feff}');
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.trim().to_ascii_lowercase();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.split(';').next().unwrap_or("").trim();
            cfg.apply(&section, &key.trim().to_ascii_lowercase(), value);
        }
        cfg.clamp();
        cfg
    }

    fn apply(&mut self, section: &str, key: &str, v: &str) {
        fn set<T>(dst: &mut T, src: Option<T>) {
            if let Some(src) = src {
                *dst = src;
            }
        }
        match (section, key) {
            ("keys", "pingkey") => set(&mut self.ping_key, parse_key(v)),
            ("keys", "quickusekey") => set(&mut self.quick_key, parse_key(v)),
            ("keys", "menukey") => set(&mut self.menu_key, parse_key(v)),
            ("keys", "gameinteractkey") => set(&mut self.game_interact_key, parse_key(v)),
            ("keys", "gameuseitemkey") => set(&mut self.game_use_key, parse_key(v)),
            ("marker", "enabled") => set(&mut self.marker_enabled, parse_bool(v)),
            ("marker", "duration") => set(&mut self.duration, v.parse().ok()),
            ("marker", "symbol") => set(&mut self.symbol, v.parse().ok()),
            ("marker", "color") => set(&mut self.color, parse_color(v)),
            ("marker", "showname") => set(&mut self.show_name, parse_bool(v)),
            ("marker", "showdistance") => set(&mut self.show_distance, parse_bool(v)),
            ("marker", "scale") => set(&mut self.scale, v.parse().ok()),
            ("marker", "mirrorx") => set(&mut self.mirror_x, parse_bool(v)),
            ("quickuse", "enabled") => set(&mut self.quick_enabled, parse_bool(v)),
            ("general", "language") => self.ru = !v.eq_ignore_ascii_case("en"),
            ("general", "ignorewhencursorvisible") => set(&mut self.ignore_cursor, parse_bool(v)),
            ("general", "log") => set(&mut self.log, parse_bool(v)),
            ("network", "enabled") => set(&mut self.net_enabled, parse_bool(v)),
            ("network", "peerdiscoveryhook") => set(&mut self.peer_hook, parse_bool(v)),
            _ => {}
        }
    }

    pub fn clamp(&mut self) {
        if !self.duration.is_finite() {
            self.duration = 10.0;
        }
        if !self.scale.is_finite() {
            self.scale = 1.0;
        }
        self.duration = self.duration.clamp(2.0, 60.0);
        self.scale = self.scale.clamp(0.5, 2.5);
        if self.symbol >= SYMBOL_COUNT {
            self.symbol = 0;
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let b = |v: bool| if v { 1 } else { 0 };
        let c = self.color.map(|v| (v * 255.0).round() as u8);
        let text = format!(
            "; er_ping_marker settings. The in-game menu (MenuKey) rewrites this file.\n\
             ; Key names: A-Z, 0-9, F1-F24, Num0-Num9, Space, Tab, Enter, Insert, Delete, Home, End,\n\
             ; PageUp, PageDown, Left, Up, Right, Down, LShift, LCtrl, LAlt, Mouse3, Mouse4, Mouse5,\n\
             ; or a virtual-key code such as 0xC0.\n\
             \n\
             [Keys]\n\
             ; Place a marker where the camera looks\n\
             PingKey={}\n\
             ; Pick up the item in front of you and use it right away\n\
             QuickUseKey={}\n\
             ; Open the settings window\n\
             MenuKey={}\n\
             ; Keys bound in the game itself (keyboard only). Change them if you rebound them in Elden Ring.\n\
             GameInteractKey={}\n\
             GameUseItemKey={}\n\
             \n\
             [Marker]\n\
             Enabled={}\n\
             ; Lifetime in seconds, 2-60\n\
             Duration={}\n\
             ; 0 pin, 1 exclamation mark, 2 arrow, 3 cross, 4 circle\n\
             Symbol={}\n\
             ; R,G,B 0-255\n\
             Color={},{},{}\n\
             ShowName={}\n\
             ShowDistance={}\n\
             ; 0.5-2.5\n\
             Scale={:.2}\n\
             ; Set to 1 if markers appear mirrored left to right\n\
             MirrorX={}\n\
             \n\
             [QuickUse]\n\
             ; Experimental: temporarily puts the picked up consumable into the selected quick slot\n\
             Enabled={}\n\
             \n\
             [General]\n\
             ; ru or en\n\
             Language={}\n\
             ; Ignore PingKey and QuickUseKey while the mouse cursor is visible (game menus)\n\
             IgnoreWhenCursorVisible={}\n\
             Log={}\n\
             \n\
             [Network]\n\
             ; Send markers to the other players of the Seamless Co-op session (they need the mod too)\n\
             Enabled={}\n\
             ; Learn session members from the Steam messages the game sends\n\
             PeerDiscoveryHook={}\n",
            key_name(self.ping_key),
            key_name(self.quick_key),
            key_name(self.menu_key),
            key_name(self.game_interact_key),
            key_name(self.game_use_key),
            b(self.marker_enabled),
            self.duration.round() as u32,
            self.symbol,
            c[0],
            c[1],
            c[2],
            b(self.show_name),
            b(self.show_distance),
            self.scale,
            b(self.mirror_x),
            b(self.quick_enabled),
            if self.ru { "ru" } else { "en" },
            b(self.ignore_cursor),
            b(self.log),
            b(self.net_enabled),
            b(self.peer_hook),
        );
        fs::write(path, text)
    }
}
