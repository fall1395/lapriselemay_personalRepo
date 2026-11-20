// Pas de fenêtre de console en release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Windows Update Stopper : il arrête Windows Update pendant une partie (Windows Update, son orchestrateur,
//! BITS et l'Optimisation de la distribution), un à un ou tous d'un coup, puis relance exactement
//! ceux qu'il a arrêtés.

mod catalog;
mod listview;
mod memory;
mod model;
mod services;
mod state;
mod theme;
mod ui;

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::LibraryLoader::{
    SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

/// Vrai si Windows Update Stopper tourne déjà dans cette session Windows.
/// Le mutex reste ouvert jusqu'à la fin du processus : c'est lui qui marque l'instance.
fn already_running() -> bool {
    unsafe {
        CreateMutexW(None, false, w!("Local\\WindowsUpdateStopper")).is_ok()
            && GetLastError() == ERROR_ALREADY_EXISTS
    }
}

fn main() {
    // Comme GameMonitor : les DLL chargées en cours de route viennent de System32 seulement,
    // jamais du dossier de l'exe, où une DLL déposée tournerait avec ses droits administrateur.
    // Les DLL liées à l'exe, chargées avant `main`, sont protégées par /DEPENDENTLOADFLAG
    // (voir build.rs).
    unsafe {
        let _ = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32);
    }
    // Deux Windows Update Stopper arrêteraient et relanceraient les mêmes services chacun de leur côté :
    // un second lancement réaffiche plutôt le premier.
    if already_running() {
        ui::show_existing();
        return;
    }
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    ui::run();
}
