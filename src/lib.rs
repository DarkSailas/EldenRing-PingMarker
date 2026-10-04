mod config;
mod game;
mod input;
mod log;
mod net;
mod overlay;
mod state;

use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::atomic::Ordering,
    time::Duration,
};

use eldenring::{
    cs::{CSTaskGroupIndex, CSTaskImp},
    fd4::FD4TaskData,
};
use fromsoftware_shared::SharedTaskImpExt;
use hudhook::{
    Hudhook,
    hooks::dx12::ImguiDx12Hooks,
    windows::Win32::Foundation::{HINSTANCE, HMODULE},
};

use crate::state::{DISABLED, MENU_OPEN};

fn init(module: usize) {
    state::set_module(module);
    let ini = state::ini_path();
    let cfg = config::Config::load(&ini);
    log::init(&state::log_path(), cfg.log);
    if !ini.exists() {
        if let Err(e) = cfg.save(&ini) {
            logf!("config: could not write the default ini: {e}");
        }
    }
    state::shared().cfg = cfg;
    logf!("er_ping_marker {} loaded", env!("CARGO_PKG_VERSION"));

    let Ok(cs_task) = CSTaskImp::wait_for_instance(Duration::MAX) else {
        logf!("init: CSTaskImp not found, the mod stays off");
        return;
    };

    let mut game = game::Game::new();
    let handle = cs_task.run_recurring(
        move |_: &FD4TaskData| {
            if DISABLED.load(Ordering::Relaxed) {
                return;
            }
            if catch_unwind(AssertUnwindSafe(|| game.tick())).is_err() {
                DISABLED.store(true, Ordering::Relaxed);
                MENU_OPEN.store(false, Ordering::Relaxed);
                logf!("game: panic, the mod is switched off until the game restarts");
            }
        },
        CSTaskGroupIndex::ChrIns_PostPhysics,
    );
    // Dropping the handle would unregister the task.
    std::mem::forget(handle);
    logf!("init: game task registered");

    input::install_dinput_block();

    let hooks = Hudhook::builder()
        .with::<ImguiDx12Hooks>(overlay::Overlay::new())
        .with_hmodule(HINSTANCE(module as _))
        .build()
        .apply();
    match hooks {
        Ok(()) => logf!("init: overlay hooked"),
        Err(e) => logf!("init: overlay hook failed: {e:?}"),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(hmodule: HMODULE, reason: u32, _: *mut c_void) -> bool {
    if reason == 1 {
        let module = hmodule.0 as usize;
        std::thread::spawn(move || {
            if catch_unwind(|| init(module)).is_err() {
                DISABLED.store(true, Ordering::Relaxed);
            }
        });
    }
    true
}
