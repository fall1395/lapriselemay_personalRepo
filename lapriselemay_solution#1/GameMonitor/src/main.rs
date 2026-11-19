// Pas de fenêtre de console en release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod format;
mod metrics;
mod overlay;
mod placement;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::LibraryLoader::{
    SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

/// Vrai si GameMonitor tourne déjà dans cette session Windows.
/// Le mutex reste ouvert jusqu'à la fin du processus : c'est lui qui marque l'instance.
fn already_running() -> bool {
    unsafe {
        CreateMutexW(None, false, w!("Local\\GameMonitor")).is_ok()
            && GetLastError() == ERROR_ALREADY_EXISTS
    }
}

fn main() {
    // Initialisation du logger
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .init();

    log::info!("Démarrage GameMonitor v{}", env!("CARGO_PKG_VERSION"));
    log::debug!("Configuration : Rust edition 2021");

    // Les DLL chargées en cours de route (par GameMonitor, ou par Windows et les pilotes pour lui)
    // viennent de System32 seulement, jamais du dossier de l'exe : une DLL déposée à côté de
    // GameMonitor y tournerait avec ses droits administrateur. Les DLL liées à l'exe, chargées
    // avant `main`, sont protégées de la même façon par /DEPENDENTLOADFLAG (voir build.rs).
    unsafe {
        let _ = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32);
    }
    // Un second GameMonitor fermerait la session ETW du premier et ne pourrait pas
    // enregistrer les raccourcis : il demande plutôt au premier de se réafficher.
    if already_running() {
        log::info!("Instance déjà en cours d'exécution, affichage de l'overlay existant");
        overlay::show_existing();
        return;
    }
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    log::info!("Initialisation de l'overlay et de la collecte des métriques");
    let shared = Arc::new(Mutex::new(metrics::Snapshot::default()));
    let worker = metrics::spawn(shared.clone());
    overlay::run(shared, worker.control());
    worker.stop(Duration::from_secs(2));
    log::info!("Arrêt propre de GameMonitor");
}
