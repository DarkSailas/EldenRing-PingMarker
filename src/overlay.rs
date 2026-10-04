use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::atomic::Ordering,
    time::Instant,
};

use hudhook::{
    ImguiRenderLoop, MessageFilter, RenderContext,
    imgui::{
        Condition, ConfigFlags, Context, DrawListMut, FontConfig, FontGlyphRanges, FontSource, Io,
        TreeNodeFlags, Ui,
    },
};

use crate::{
    config::{Config, SYMBOL_COUNT, key_name},
    game::{self, Cam, dot},
    input,
    logf,
    state::{DISABLED, KeyTarget, MENU_OPEN, shared},
};

const BADGE_RADIUS: f32 = 17.0;
const TOAST_SECONDS: f32 = 3.5;

pub struct Overlay {
    /// False when the system font with Cyrillic glyphs could not be loaded.
    cyrillic: bool,
}

impl Overlay {
    pub fn new() -> Self {
        Self { cyrillic: false }
    }
}

struct Draw {
    symbol: u8,
    color: [f32; 3],
    name: String,
    distance: f32,
    age: f32,
    left: f32,
    pos: [f32; 3],
}

fn rgba(c: [f32; 3], a: f32) -> [f32; 4] {
    [c[0], c[1], c[2], a]
}

/// Screen position of a world point and whether it had to be pushed to the screen edge.
fn project(cam: &Cam, mirror: bool, display: [f32; 2], point: [f32; 3]) -> ([f32; 2], bool) {
    let d = [point[0] - cam.pos[0], point[1] - cam.pos[1], point[2] - cam.pos[2]];
    let (x, y, z) = (dot(d, cam.right), dot(d, cam.up), dot(d, cam.fwd));

    let mut fov = cam.fov;
    if fov > 3.2 {
        fov = fov.to_radians();
    }
    if !(0.2..=2.8).contains(&fov) {
        fov = 0.84;
    }
    let (w, h) = (display[0].max(1.0), display[1].max(1.0));
    let aspect = if (0.5..=4.0).contains(&cam.aspect) { cam.aspect } else { w / h };
    // The game keeps its own aspect ratio and adds bars on wider or taller windows.
    let (vw, vh) = if w / h > aspect { (h * aspect, h) } else { (w, w / aspect) };
    let tan = (fov * 0.5).tan();

    let (mut nx, mut ny, mut edge);
    if z > 0.05 {
        nx = x / (z * tan * aspect);
        ny = y / (z * tan);
        edge = false;
    } else {
        let len = (x * x + y * y).sqrt().max(0.001);
        nx = x / len * 100.0;
        ny = y / len * 100.0;
        edge = true;
    }
    if mirror {
        nx = -nx;
    }
    let over = (nx.abs() / 0.93).max(ny.abs() / 0.88);
    if over > 1.0 {
        nx /= over;
        ny /= over;
        edge = true;
    }
    (
        [
            (w - vw) * 0.5 + (0.5 + 0.5 * nx) * vw,
            (h - vh) * 0.5 + (0.5 - 0.5 * ny) * vh,
        ],
        edge,
    )
}

fn symbol(dl: &DrawListMut, kind: u8, c: [f32; 2], r: f32, col: [f32; 4]) {
    let p = |x: f32, y: f32| [c[0] + x * r, c[1] + y * r];
    let t = (r * 0.2).max(1.5);
    match kind % SYMBOL_COUNT {
        // Map pin
        0 => {
            dl.add_circle(p(0.0, -0.22), r * 0.36, col).num_segments(20).thickness(t).build();
            dl.add_triangle(p(-0.3, 0.1), p(0.3, 0.1), p(0.0, 0.72), col).filled(true).build();
        }
        // Exclamation mark
        1 => {
            dl.add_line(p(0.0, -0.68), p(0.0, 0.18), col).thickness(t * 1.5).build();
            dl.add_circle(p(0.0, 0.56), t * 0.85, col).num_segments(12).filled(true).build();
        }
        // Arrow down
        2 => {
            dl.add_line(p(0.0, -0.66), p(0.0, 0.56), col).thickness(t).build();
            dl.add_line(p(-0.42, 0.14), p(0.0, 0.62), col).thickness(t).build();
            dl.add_line(p(0.42, 0.14), p(0.0, 0.62), col).thickness(t).build();
        }
        // Cross
        3 => {
            dl.add_line(p(-0.48, -0.48), p(0.48, 0.48), col).thickness(t * 1.2).build();
            dl.add_line(p(0.48, -0.48), p(-0.48, 0.48), col).thickness(t * 1.2).build();
        }
        // Target
        _ => {
            dl.add_circle(c, r * 0.52, col).num_segments(24).thickness(t).build();
            dl.add_circle(c, t * 0.9, col).num_segments(12).filled(true).build();
        }
    }
}

fn shadow_text(dl: &DrawListMut, pos: [f32; 2], col: [f32; 4], text: &str) {
    dl.add_text([pos[0] + 1.0, pos[1] + 1.0], [0.0, 0.0, 0.0, col[3] * 0.85], text);
    dl.add_text(pos, col, text);
}

fn draw_marker(ui: &Ui, dl: &DrawListMut, cfg: &Config, cam: &Cam, m: &Draw) {
    let display = ui.io().display_size;
    let (point, edge) = project(cam, cfg.mirror_x, display, m.pos);
    let alpha = m.left.clamp(0.0, 1.0) * (m.age * 6.0).clamp(0.0, 1.0);
    let r = BADGE_RADIUS * cfg.scale * if edge { 0.7 } else { 1.0 };
    let col = rgba(m.color, alpha);

    // On screen the point itself is the foot of the beam and the badge hangs above it.
    let center = if edge { point } else { [point[0], point[1] - r * 3.0] };
    if !edge {
        let top = [center[0], center[1] - r * 2.4];
        dl.add_line(top, point, rgba(m.color, alpha * 0.35)).thickness(r * 0.28).build();
        dl.add_line(top, point, col).thickness((r * 0.09).max(1.0)).build();
        dl.add_circle(point, r * 0.16, col).num_segments(12).filled(true).build();
    }
    if m.age < 0.6 {
        let k = m.age / 0.6;
        dl.add_circle(center, r * (1.0 + 1.6 * k), rgba(m.color, alpha * (1.0 - k)))
            .num_segments(40)
            .thickness(2.0)
            .build();
    }
    dl.add_circle(center, r, [0.05, 0.07, 0.04, alpha * 0.86]).num_segments(40).filled(true).build();
    dl.add_circle(center, r, col).num_segments(40).thickness((r * 0.14).max(1.5)).build();
    symbol(dl, m.symbol, center, r * 0.78, col);

    let mut lines: Vec<String> = Vec::new();
    if cfg.show_name && !m.name.is_empty() {
        lines.push(m.name.clone());
    }
    if cfg.show_distance {
        lines.push(format!("{:.0} m", m.distance));
    }
    let mut y = center[1] + r + 4.0;
    for line in &lines {
        let size = ui.calc_text_size(line);
        let x = (center[0] - size[0] * 0.5).clamp(2.0, (display[0] - size[0] - 2.0).max(2.0));
        shadow_text(dl, [x, y], [1.0, 1.0, 1.0, alpha], line);
        y += size[1];
    }
}

impl Overlay {
    fn markers(&self, ui: &Ui, cfg: &Config) {
        let now = Instant::now();
        let (list, player, toast) = {
            let s = shared();
            let list: Vec<Draw> = s
                .markers
                .iter()
                .filter_map(|m| {
                    let age = now.duration_since(m.born).as_secs_f32();
                    Some(Draw {
                        symbol: m.symbol,
                        color: m.color,
                        name: m.name.clone(),
                        distance: m.distance,
                        age,
                        left: m.duration - age,
                        pos: m.havok?,
                    })
                })
                .collect();
            (list, s.player_pos, s.toast.clone())
        };

        let dl = ui.get_background_draw_list();
        if cfg.marker_enabled && !list.is_empty() {
            if let Some(cam) = game::camera(player) {
                for m in list.iter().filter(|m| m.left > 0.0) {
                    draw_marker(ui, &dl, cfg, &cam, m);
                }
            }
        }

        if let Some((text, at)) = toast {
            let age = now.duration_since(at).as_secs_f32();
            if age < TOAST_SECONDS && (self.cyrillic || text.is_ascii()) {
                let display = ui.io().display_size;
                let size = ui.calc_text_size(&text);
                let pos = [(display[0] - size[0]) * 0.5, display[1] * 0.16];
                let alpha = (TOAST_SECONDS - age).clamp(0.0, 1.0);
                dl.add_rect([pos[0] - 10.0, pos[1] - 6.0], [pos[0] + size[0] + 10.0, pos[1] + size[1] + 6.0], [0.05, 0.07, 0.04, alpha * 0.8])
                    .filled(true)
                    .rounding(4.0)
                    .build();
                shadow_text(&dl, pos, [1.0, 1.0, 1.0, alpha], &text);
            }
        }
    }

    fn key_row(&self, ui: &Ui, label: &str, vk: u16, target: KeyTarget, capture: Option<KeyTarget>, wait: &str) {
        ui.text(label);
        ui.same_line_with_pos(250.0);
        let caption = if capture == Some(target) { wait.to_owned() } else { key_name(vk) };
        if ui.button_with_size(format!("{caption}##key{}", target as u8), [170.0, 0.0]) {
            shared().capture = Some(target);
        }
    }

    fn menu(&self, ui: &Ui, cfg: &mut Config) -> bool {
        let ru = cfg.ru && self.cyrillic;
        let t = |r: &'static str, e: &'static str| if ru { r } else { e };
        let (capture, net_ready, peers, with_mod) = {
            let s = shared();
            (s.capture, s.net_ready, s.peers, s.peers_with_mod)
        };
        let mut changed = false;
        let mut open = true;

        ui.window(format!("{}###er_ping_marker", t("Метки и быстрое применение", "Ping marker and quick use")))
            .size([470.0, 600.0], Condition::FirstUseEver)
            .position([ui.io().display_size[0] * 0.5, 24.0], Condition::FirstUseEver)
            .position_pivot([0.5, 0.0])
            .opened(&mut open)
            .build(|| {
                if self.cyrillic {
                    let mut lang = usize::from(!cfg.ru);
                    ui.set_next_item_width(170.0);
                    if ui.combo_simple_string(t("Язык", "Language"), &mut lang, &["Русский", "English"]) {
                        cfg.ru = lang == 0;
                        changed = true;
                    }
                }

                if ui.collapsing_header(t("Клавиши", "Keys"), TreeNodeFlags::DEFAULT_OPEN) {
                    let wait = t("нажмите клавишу...", "press a key...");
                    self.key_row(ui, t("Поставить метку", "Place a marker"), cfg.ping_key, KeyTarget::Ping, capture, wait);
                    self.key_row(ui, t("Поднять и применить", "Pick up and use"), cfg.quick_key, KeyTarget::Quick, capture, wait);
                    self.key_row(ui, t("Это окно", "This window"), cfg.menu_key, KeyTarget::Menu, capture, wait);
                    ui.text_disabled(t("Как назначено в самой игре:", "As bound in the game itself:"));
                    self.key_row(ui, t("Действие (подобрать)", "Interact (pick up)"), cfg.game_interact_key, KeyTarget::GameInteract, capture, wait);
                    self.key_row(ui, t("Использовать предмет", "Use item"), cfg.game_use_key, KeyTarget::GameUse, capture, wait);
                    ui.text_disabled(t("Esc отменяет выбор клавиши.", "Esc cancels the key choice."));
                }

                if ui.collapsing_header(t("Метка", "Marker"), TreeNodeFlags::DEFAULT_OPEN) {
                    changed |= ui.checkbox(t("Включена", "Enabled"), &mut cfg.marker_enabled);
                    let names = [
                        t("Точка на карте", "Map pin"),
                        t("Внимание", "Attention"),
                        t("Стрелка", "Arrow"),
                        t("Крест", "Cross"),
                        t("Цель", "Target"),
                    ];
                    let mut idx = (cfg.symbol % SYMBOL_COUNT) as usize;
                    ui.set_next_item_width(170.0);
                    if ui.combo_simple_string(t("Символ", "Symbol"), &mut idx, &names) {
                        cfg.symbol = idx as u8;
                        changed = true;
                    }
                    ui.set_next_item_width(170.0);
                    changed |= ui.slider(t("Время, с", "Duration, s"), 3.0, 60.0, &mut cfg.duration);
                    ui.set_next_item_width(170.0);
                    changed |= ui.slider(t("Размер", "Size"), 0.5, 2.5, &mut cfg.scale);
                    ui.set_next_item_width(170.0);
                    changed |= ui.color_edit3(t("Цвет", "Colour"), &mut cfg.color);
                    changed |= ui.checkbox(t("Имя игрока", "Player name"), &mut cfg.show_name);
                    changed |= ui.checkbox(t("Расстояние", "Distance"), &mut cfg.show_distance);
                    changed |= ui.checkbox(
                        t("Отразить по горизонтали (если метка уезжает не туда)", "Mirror horizontally (if the marker drifts the wrong way)"),
                        &mut cfg.mirror_x,
                    );
                }

                if ui.collapsing_header(t("Быстрое применение", "Quick use"), TreeNodeFlags::DEFAULT_OPEN) {
                    changed |= ui.checkbox(t("Включено (экспериментально)", "Enabled (experimental)"), &mut cfg.quick_enabled);
                    ui.text_wrapped(t(
                        "Подбирает предмет под ногами и сразу применяет его, если это расходник. Нужен хотя бы один предмет в быстрых слотах.",
                        "Picks up the item at your feet and uses it at once if it is a consumable. At least one item must sit in the quick slots.",
                    ));
                }

                if ui.collapsing_header(t("Прочее", "Other"), TreeNodeFlags::empty()) {
                    changed |= ui.checkbox(t("Не срабатывать, пока виден курсор", "Ignore keys while the cursor is visible"), &mut cfg.ignore_cursor);
                    changed |= ui.checkbox(t("Отправлять метки другим игрокам", "Send markers to other players"), &mut cfg.net_enabled);
                    changed |= ui.checkbox(t("Журнал er_ping_marker.log", "Log file er_ping_marker.log"), &mut cfg.log);
                }

                ui.separator();
                ui.text(format!(
                    "{}: {}   {}: {peers}   {}: {with_mod}",
                    t("Сеть", "Network"),
                    if net_ready { t("готова", "ready") } else { t("нет", "off") },
                    t("игроков", "players"),
                    t("с модом", "with the mod"),
                ));
                ui.text_disabled(t("Метку видят только игроки с этим модом.", "Only players with this mod see the marker."));
            });

        if !open {
            MENU_OPEN.store(false, Ordering::Relaxed);
        }
        changed
    }

    fn frame(&self, ui: &Ui) {
        let mut cfg = shared().cfg.clone();
        self.markers(ui, &cfg);
        if MENU_OPEN.load(Ordering::Relaxed) && self.menu(ui, &mut cfg) {
            cfg.clamp();
            let mut s = shared();
            // Key bindings are written by the game thread while a key is being captured.
            let keys = s.cfg.clone();
            cfg.ping_key = keys.ping_key;
            cfg.quick_key = keys.quick_key;
            cfg.menu_key = keys.menu_key;
            cfg.game_interact_key = keys.game_interact_key;
            cfg.game_use_key = keys.game_use_key;
            s.cfg = cfg;
            s.dirty = Some(Instant::now());
        }
    }
}

impl ImguiRenderLoop for Overlay {
    fn initialize<'a>(&'a mut self, ctx: &mut Context, _render_context: &'a mut dyn RenderContext) {
        ctx.set_ini_filename(None);
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_owned());
        match std::fs::read(format!("{windir}\\Fonts\\segoeui.ttf")) {
            Ok(data) => {
                ctx.fonts().add_font(&[FontSource::TtfData {
                    data: &data,
                    size_pixels: 20.0,
                    config: Some(FontConfig {
                        glyph_ranges: FontGlyphRanges::cyrillic(),
                        ..FontConfig::default()
                    }),
                }]);
                self.cyrillic = true;
            }
            Err(e) => logf!("overlay: segoeui.ttf not loaded ({e}), interface stays in English"),
        }
    }

    fn before_render<'a>(&'a mut self, ctx: &mut Context, _render_context: &'a mut dyn RenderContext) {
        let open = MENU_OPEN.load(Ordering::Relaxed);
        let io = ctx.io_mut();
        io.mouse_draw_cursor = open;
        io.config_flags.set(ConfigFlags::NAV_ENABLE_KEYBOARD, open);
        if open {
            // The game task does not run on every screen, so the block is refreshed here too.
            input::hold_game_input();
        }
        input::cursor_frame(open);
    }

    fn render(&mut self, ui: &mut Ui) {
        if DISABLED.load(Ordering::Relaxed) {
            return;
        }
        if catch_unwind(AssertUnwindSafe(|| self.frame(ui))).is_err() {
            DISABLED.store(true, Ordering::Relaxed);
            MENU_OPEN.store(false, Ordering::Relaxed);
            logf!("overlay: panic, the mod is switched off until the game restarts");
        }
    }

    fn message_filter(&self, _io: &Io) -> MessageFilter {
        if MENU_OPEN.load(Ordering::Relaxed) && !DISABLED.load(Ordering::Relaxed) {
            MessageFilter::InputAll
        } else {
            MessageFilter::empty()
        }
    }
}
