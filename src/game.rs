use std::{
    collections::HashMap,
    ffi::c_void,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use eldenring::{
    cs::{
        BlockId, CSCam, CSCamExt, CSCamera, CSHavokMan, EquipParamGoods, FieldArea, GaitemHandle,
        ItemCategory, OptionalItemId, PlayerIns, SoloParamRepository, WorldChrMan,
    },
    position::{HavokPosition, PositionDelta},
};
use fromsoftware_shared::{FromStatic, Subclass};

use crate::{
    config::Config,
    input::{self, Injector, Keys},
    log, logf,
    net::{self, Net, Ping},
    state::{self, MENU_OPEN, Marker, shared},
};

const RAY_FILTER: u32 = 0x2000058;
const RAY_LENGTH: f32 = 150.0;
const RAY_FALLBACK: f32 = 40.0;
const MAX_MARKERS: usize = 16;
const PING_COOLDOWN: Duration = Duration::from_millis(300);
const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);
const PICKUP_TIMEOUT: Duration = Duration::from_millis(2500);
const USE_TIMEOUT: Duration = Duration::from_millis(4000);

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn tr(ru: bool, r: &str, e: &str) -> String {
    if ru { r.to_owned() } else { e.to_owned() }
}

pub struct Cam {
    pub pos: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub fwd: [f32; 3],
    pub fov: f32,
    pub aspect: f32,
}

/// Current gameplay camera. `player` settles which way the forward row points: the character
/// is always in front of the camera.
pub fn camera(player: Option<[f32; 3]>) -> Option<Cam> {
    let cam = unsafe { CSCamera::instance() }.ok()?;
    let pers = &cam.pers_cam_1;
    let (p, r, u, f) = (pers.position(), pers.right(), pers.up(), pers.forward());
    let base: &CSCam = Subclass::<CSCam>::superclass(&**pers);
    let pos = [p.0, p.1, p.2];
    let mut fwd = [f.0, f.1, f.2];
    if !pos.iter().chain(fwd.iter()).all(|v| v.is_finite()) {
        return None;
    }
    if let Some(player) = player {
        if dot(sub(player, pos), fwd) < -0.5 {
            fwd = fwd.map(|v| -v);
        }
    }
    Some(Cam {
        pos,
        right: [r.0, r.1, r.2],
        up: [u.0, u.1, u.2],
        fwd,
        fov: base.fov,
        aspect: base.aspect_ratio,
    })
}

fn havok_of(player: &PlayerIns) -> [f32; 3] {
    let p = &player.chr_ins.modules.physics.position;
    [p.0, p.1, p.2]
}

fn local_of(player: &PlayerIns) -> [f32; 3] {
    let b = &player.block_position;
    [b.x, b.y, b.z]
}

static GRID_LOGGED: AtomicBool = AtomicBool::new(false);

fn center_offset(marker: &BlockId, mine: &BlockId) -> Option<[f32; 3]> {
    let area = unsafe { FieldArea::instance() }.ok()?;
    let info = &area.world_info_owner.world_res.world_info;
    let a = &info.world_block_info_by_map(marker)?.physics_center;
    let b = &info.world_block_info_by_map(mine)?.physics_center;
    Some([a.0 - b.0, a.1 - b.1, a.2 - b.2])
}

/// Offset from the origin of my block to the origin of the marker's block.
fn block_offset(marker: u32, mine: u32) -> Option<[f32; 3]> {
    if marker == mine {
        return Some([0.0; 3]);
    }
    let (m, p) = (BlockId(marker as i32), BlockId(mine as i32));
    if m.is_overworld() && p.is_overworld() && m.area() == p.area() && m.index() == p.index() {
        let grid = [
            (m.block() as i32 - p.block() as i32) as f32 * 256.0,
            0.0,
            (m.region() as i32 - p.region() as i32) as f32 * 256.0,
        ];
        if !GRID_LOGGED.swap(true, Ordering::Relaxed) {
            logf!(
                "blocks: grid offset {:?}, block centers say {:?}",
                grid,
                center_offset(&m, &p)
            );
        }
        return Some(grid);
    }
    center_offset(&m, &p)
}

fn goods(player: &PlayerIns) -> HashMap<u32, u32> {
    let data = unsafe { player.player_game_data.as_ref() };
    data.equipment
        .equip_inventory_data
        .items_data
        .items()
        .filter(|e| e.item_id.category() == ItemCategory::Goods)
        .map(|e| (e.item_id.into_inner(), e.quantity))
        .collect()
}

fn is_consumable(param_id: u32) -> bool {
    unsafe { SoloParamRepository::instance() }
        .ok()
        .and_then(|repo| repo.get::<EquipParamGoods>(param_id))
        .is_some_and(|row| row.goods_type() == 0 && row.is_consume())
}

enum Quick {
    Idle,
    WaitPickup {
        since: Instant,
        before: HashMap<u32, u32>,
    },
    WaitUse {
        since: Instant,
        item: u32,
        quantity: u32,
        slot: usize,
        handle: GaitemHandle,
        index: i32,
        id: OptionalItemId,
    },
}

pub struct Game {
    keys: Keys,
    injector: Injector,
    net: Net,
    quick: Quick,
    last_ping: Option<Instant>,
    ids: Vec<u64>,
    ids_at: Option<Instant>,
    capturing: bool,
}

impl Game {
    pub fn new() -> Self {
        Self {
            keys: Keys::new(),
            injector: Injector::new(),
            net: Net::new(),
            quick: Quick::Idle,
            last_ping: None,
            ids: Vec::new(),
            ids_at: None,
            capturing: false,
        }
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        let cfg = shared().cfg.clone();
        self.injector.tick(now);
        self.save_if_dirty(now);

        let chr_man = unsafe { WorldChrMan::instance() }.ok();
        let player = chr_man.and_then(|w| w.main_player.as_ref()).map(|p| &**p);

        if self.ids_at.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1)) {
            self.ids_at = Some(now);
            self.ids.clear();
            if let (Some(w), Some(_)) = (chr_man, player) {
                for chr in w.player_chr_set.characters() {
                    let entry = chr.session_manager_player_entry.as_ptr();
                    if net::readable(entry as *const c_void, 24) {
                        let id = unsafe { (*entry).steam_id };
                        if id != 0 && !self.ids.contains(&id) {
                            self.ids.push(id);
                        }
                    }
                }
            }
        }

        let incoming = self.net.tick(now, &cfg, &self.ids);
        {
            let mut s = shared();
            s.net_ready = self.net.ready();
            s.peers = self.net.peer_count();
            s.peers_with_mod = self.net.mod_count();
            for msg in incoming {
                if cfg.marker_enabled {
                    push_marker(&mut s.markers, msg.sender, &msg.ping, now);
                }
            }
        }

        self.hotkeys(now, &cfg, player);
        self.quick_use(now, &cfg, player);
        update_markers(now, player);
    }

    fn save_if_dirty(&mut self, now: Instant) {
        let mut s = shared();
        if s.dirty.is_some_and(|t| now.duration_since(t) >= SAVE_DEBOUNCE) {
            s.dirty = None;
            let cfg = s.cfg.clone();
            drop(s);
            log::set_enabled(cfg.log);
            if let Err(e) = cfg.save(&state::ini_path()) {
                logf!("config: save failed: {e}");
            }
        }
    }

    fn hotkeys(&mut self, now: Instant, cfg: &Config, player: Option<&PlayerIns>) {
        if !input::is_foreground() {
            self.keys.sync_all();
            return;
        }

        let capture = shared().capture;
        // Keys already held when the capture starts must not count as the new binding.
        if capture.is_some() != self.capturing {
            self.capturing = capture.is_some();
            self.keys.sync_all();
            return;
        }
        if let Some(target) = capture {
            if let Some(vk) = self.keys.scan_new() {
                let mut s = shared();
                if vk == 0x1B {
                    s.capture = None;
                } else if !(target.is_game_key() && vk < 8) {
                    match target {
                        state::KeyTarget::Ping => s.cfg.ping_key = vk,
                        state::KeyTarget::Quick => s.cfg.quick_key = vk,
                        state::KeyTarget::Menu => s.cfg.menu_key = vk,
                        state::KeyTarget::GameInteract => s.cfg.game_interact_key = vk,
                        state::KeyTarget::GameUse => s.cfg.game_use_key = vk,
                    }
                    s.capture = None;
                    s.dirty = Some(now);
                }
            }
            return;
        }

        let menu = self.keys.pressed(cfg.menu_key);
        let ping = self.keys.pressed(cfg.ping_key);
        let quick = self.keys.pressed(cfg.quick_key);

        if menu {
            let open = !MENU_OPEN.load(Ordering::Relaxed);
            MENU_OPEN.store(open, Ordering::Relaxed);
            return;
        }
        if MENU_OPEN.load(Ordering::Relaxed) || (cfg.ignore_cursor && input::cursor_visible()) {
            return;
        }
        let Some(player) = player else {
            return;
        };

        if ping && cfg.marker_enabled && self.last_ping.is_none_or(|t| now.duration_since(t) >= PING_COOLDOWN) {
            self.last_ping = Some(now);
            self.place_ping(now, cfg, player);
        }
        if quick && cfg.quick_enabled && matches!(self.quick, Quick::Idle) {
            self.quick = Quick::WaitPickup {
                since: now,
                before: goods(player),
            };
            self.injector.press(cfg.game_interact_key);
        }
    }

    fn place_ping(&mut self, now: Instant, cfg: &Config, player: &PlayerIns) {
        let me = havok_of(player);
        let Some(cam) = camera(Some(me)) else {
            logf!("ping: no camera");
            return;
        };
        let origin = HavokPosition(cam.pos[0], cam.pos[1], cam.pos[2], 0.0);
        let delta = PositionDelta(
            cam.fwd[0] * RAY_LENGTH,
            cam.fwd[1] * RAY_LENGTH,
            cam.fwd[2] * RAY_LENGTH,
        );
        let hit = unsafe { CSHavokMan::instance() }
            .ok()
            .and_then(|h| h.phys_world.cast_ray(RAY_FILTER, &origin, delta, player))
            .map(|h| [h.0, h.1, h.2])
            .filter(|h| h.iter().all(|v| v.is_finite()));
        let target = hit.unwrap_or_else(|| add(cam.pos, cam.fwd.map(|v| v * RAY_FALLBACK)));

        let ping = Ping {
            symbol: cfg.symbol,
            block: player.current_block_id.0 as u32,
            local: add(local_of(player), sub(target, me)),
            color: cfg.color,
            duration: cfg.duration,
            name: self.net.my_name().to_owned(),
        };
        let sent = self.net.send_ping(&ping);
        logf!(
            "ping: hit {} block {:08x} local {:?} sent to {sent}",
            hit.is_some(),
            ping.block,
            ping.local
        );
        push_marker(&mut shared().markers, self.net.my_id(), &ping, now);
    }

    fn quick_use(&mut self, now: Instant, cfg: &Config, player: Option<&PlayerIns>) {
        let Some(player) = player else {
            self.quick = Quick::Idle;
            return;
        };
        match &self.quick {
            Quick::Idle => {}
            Quick::WaitPickup { since, before } => {
                let picked = goods(player)
                    .into_iter()
                    .find(|(id, qty)| *qty > before.get(id).copied().unwrap_or(0));
                if let Some((item, quantity)) = picked {
                    self.quick = Quick::Idle;
                    self.begin_use(now, cfg, player, item, quantity);
                } else if now.duration_since(*since) >= PICKUP_TIMEOUT {
                    self.quick = Quick::Idle;
                }
            }
            Quick::WaitUse {
                since,
                item,
                quantity,
                slot,
                handle,
                index,
                id,
            } => {
                let left = goods(player).get(item).copied().unwrap_or(0);
                if left < *quantity || now.duration_since(*since) >= USE_TIMEOUT {
                    let mut ptr = player.player_game_data;
                    let data = unsafe { ptr.as_mut() };
                    let quick_slot = &mut data.equipment.equip_item_data.quick_slots[*slot];
                    quick_slot.gaitem_handle = *handle;
                    quick_slot.index = *index;
                    data.equipment.equipment_entries.quick_tems[*slot] = *id;
                    logf!("quick use: slot {slot} restored, used {}", left < *quantity);
                    self.quick = Quick::Idle;
                }
            }
        }
    }

    fn begin_use(&mut self, now: Instant, cfg: &Config, player: &PlayerIns, item: u32, quantity: u32) {
        let mut ptr = player.player_game_data;
        let data = unsafe { ptr.as_mut() };
        let entry = data
            .equipment
            .equip_inventory_data
            .items_data
            .items()
            .find(|e| e.item_id.into_inner() == item)
            .map(|e| (e.gaitem_handle, e.item_id));
        let Some((new_handle, item_id)) = entry else {
            return;
        };
        if !is_consumable(item_id.param_id()) {
            logf!("quick use: item {item:08x} is not a consumable, left in the inventory");
            return;
        }
        let items = &data.equipment.equip_inventory_data.items_data;
        // The swap relies on quick slots holding an inventory slot number. Prove that on a slot
        // the game filled itself before writing anything.
        let proven = data.equipment.equip_item_data.quick_slots.iter().any(|s| {
            s.index >= 0
                && items
                    .entry_at_slot(s.index as u32)
                    .and_then(|e| e.as_option())
                    .is_some_and(|e| e.gaitem_handle == s.gaitem_handle)
        });
        let new_index = items.find_item_idx(item_id);
        let (true, Some(new_index)) = (proven, new_index) else {
            logf!("quick use: quick slot layout not confirmed (proven {proven}), nothing written");
            state::toast(tr(
                cfg.ru,
                "Быстрое применение: положите любой предмет в быстрый слот",
                "Quick use: put any item into a quick slot first",
            ));
            return;
        };

        let slot = data.equipment.equip_item_data.selected_quick_slot.clamp(0, 9) as usize;
        let quick_slot = &mut data.equipment.equip_item_data.quick_slots[slot];
        let (handle, index) = (quick_slot.gaitem_handle, quick_slot.index);
        let id = data.equipment.equipment_entries.quick_tems[slot];
        quick_slot.gaitem_handle = new_handle;
        quick_slot.index = new_index as i32;
        data.equipment.equipment_entries.quick_tems[slot] = OptionalItemId::from(item_id);
        self.injector.press(cfg.game_use_key);
        logf!("quick use: item {item:08x} put into slot {slot}, use key sent");
        self.quick = Quick::WaitUse {
            since: now,
            item,
            quantity,
            slot,
            handle,
            index,
            id,
        };
    }
}

fn push_marker(markers: &mut Vec<Marker>, sender: u64, ping: &Ping, now: Instant) {
    markers.retain(|m| m.sender != sender);
    if markers.len() >= MAX_MARKERS {
        markers.remove(0);
    }
    markers.push(Marker {
        sender,
        name: ping.name.clone(),
        symbol: ping.symbol,
        color: ping.color,
        block: ping.block,
        local: ping.local,
        born: now,
        duration: ping.duration,
        havok: None,
        distance: 0.0,
    });
}

fn update_markers(now: Instant, player: Option<&PlayerIns>) {
    let me = player.map(|p| (havok_of(p), local_of(p), p.current_block_id.0 as u32));
    let mut s = shared();
    s.player_pos = me.map(|m| m.0);
    s.markers
        .retain(|m| now.duration_since(m.born).as_secs_f32() < m.duration);
    for m in s.markers.iter_mut() {
        m.havok = me.and_then(|(havok, local, block)| {
            let offset = block_offset(m.block, block)?;
            Some(add(add(havok, sub(m.local, local)), offset))
        });
        if let (Some(pos), Some((havok, _, _))) = (m.havok, me) {
            let d = sub(pos, havok);
            m.distance = dot(d, d).sqrt();
        }
    }
}
