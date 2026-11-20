//! La fenêtre de GameBooster, en Win32 pur. En haut : « Tout arrêter », « Tout relancer », l'état
//! du mode jeu et la RAM que « Tout arrêter » libérerait avec les cases cochées. Au milieu : la
//! liste des services inutiles en jeu. En bas : le détail du service sélectionné, avec son bouton
//! Arrêter ou Relancer, puis la RAM du PC.
//!
//! Arrêter ou relancer des services prend quelques secondes : c'est fait dans un fil à part, pour
//! que la fenêtre reste réactive, et les boutons sont grisés pendant ce temps. La liste se met à
//! jour toutes les 3 s (sauf fenêtre réduite). En mode jeu, un minuteur arrête aussi, toutes les
//! 30 s, les services cochés que Windows a démarrés ou relancés entre-temps ; mais pas ceux que
//! Windows relance aussitôt arrêtés : il les laisse tourner.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::thread;
use std::time::Duration;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::{GetLocalTime, GetTickCount64};
use windows::Win32::UI::Controls::{
    CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDDS_SUBITEM, CDRF_DODEFAULT, CDRF_NOTIFYITEMDRAW,
    CDRF_NOTIFYSUBITEMDRAW, DRAWITEMSTRUCT, LVN_COLUMNCLICK, LVN_ITEMCHANGED, NMHDR, NMLISTVIEW,
    NMLVCUSTOMDRAW, NM_CUSTOMDRAW, NM_RCLICK, ODS_DISABLED, ODS_FOCUS, ODS_NOFOCUSRECT,
    ODS_SELECTED,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, IsWindowEnabled, SetFocus,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::catalog::Advice;
use crate::listview::{self, Line};
use crate::memory;
use crate::model::{self, Column, Ram, Row};
use crate::services::{self, Info, Scm, Stop};
use crate::state::{self, Choices, Session};
use crate::theme::{
    self, fill, px, wide, Fonts, AMBER, BG, BLUE, DIM, DISABLED, DISABLED_TEXT, GRAY, GRAY_DOWN,
    GREEN, GREEN_DOWN, PANEL, TEXT, WHITE,
};

const CLASS: PCWSTR = w!("GameBoosterWindow");
// Identifiants des contrôles. Pas 1 ni 2 : Entrée et Échap envoient IDOK (1) et IDCANCEL (2).
const ID_STOP_ALL: usize = 101;
const ID_START_ALL: usize = 102;
const ID_LIST: usize = 103;
const ID_ONE: usize = 104;
const ID_MENU_ONE: usize = 201;
/// Fin d'un travail du fil d'arrière-plan ; LPARAM porte un `Box<Done>`.
const WM_APP_DONE: u32 = WM_APP + 1;
/// Envoyé par un second lancement de GameBooster : réafficher la fenêtre.
const WM_APP_SHOW: u32 = WM_APP + 2;
/// Mise à jour de la liste.
const REFRESH_TIMER: usize = 1;
const REFRESH_MS: u32 = 3_000;
/// Services arrêtés gardés arrêtés (et, en mode jeu, cochés arrêtés), tant qu'il y en a.
const WATCH_TIMER: usize = 2;
const WATCH_MS: u32 = 30_000;

/// Fenêtre redimensionnable. WS_CLIPCHILDREN : dessiner le fond n'efface pas les contrôles.
const STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_OVERLAPPEDWINDOW.0 | WS_CLIPCHILDREN.0);

// Mise en page, en pixels à 100 %.
const START_W: i32 = 820;
const START_H: i32 = 680;
const MIN_W: i32 = 680;
const MIN_H: i32 = 560;
const PAD: i32 = 16;
const TOP_BTN_W: i32 = 150;
const TOP_BTN_H: i32 = 44;
const GAP: i32 = 10;
const LIST_TOP: i32 = PAD + TOP_BTN_H + 16;
const DETAILS_H: i32 = 150;
const FOOTER_H: i32 = 18;
const ONE_BTN_W: i32 = 124;
const ONE_BTN_H: i32 = 36;
/// Marge intérieure du panneau de détail.
const INNER: i32 = 14;

/// Travail en cours dans le fil d'arrière-plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Working {
    Idle,
    Stopping,
    Starting,
}

/// Un service que Windows a refusé d'arrêter ou de relancer : son nom, la raison.
type Issue = (String, String);

/// Travail du fil d'arrière-plan. `all` : « Tout arrêter » ou « Tout relancer ».
enum Job {
    Stop { names: Vec<String>, all: bool },
    Start { names: Vec<String>, all: bool },
}

/// Fin d'un travail, renvoyée à la fenêtre.
enum Done {
    Stopped {
        all: bool,
        stopped: Vec<String>,
        refused: Vec<Issue>,
        freed: u64,
    },
    Started {
        all: bool,
        started: Vec<String>,
        failed: Vec<Issue>,
    },
    /// Le gestionnaire de services est inaccessible : rien n'a changé.
    Failed(String),
}

struct Ui {
    stop_all: HWND,
    start_all: HWND,
    one: HWND,
    list: HWND,
    dpi: u32,
    fonts: Fonts,
    elevated: bool,
    working: Working,
    /// Dernière lecture des services : les lignes, dans l'ordre de la liste…
    rows: Vec<Row>,
    /// … et tous les services de Windows, la RAM de chaque processus et celle du PC.
    infos: Vec<Info>,
    ram: HashMap<u32, u64>,
    system_ram: Option<(u64, u64)>,
    /// Ce que la liste affiche, ligne par ligne.
    shown: Vec<Line>,
    sort: (Column, bool),
    /// Service sélectionné, et sa description (lue une fois).
    selected: Option<String>,
    description: Option<(String, Option<String>)>,
    choices: Choices,
    session: Option<Session>,
    /// Démarrés à la main, ou cochés alors qu'ils tournaient, pendant le mode jeu : le minuteur les
    /// laisse tourner jusqu'au prochain « Tout arrêter ».
    allowed: HashSet<String>,
    refused: Vec<Issue>,
    failed: Vec<Issue>,
    /// Démarrés par Windows pendant la session, puis arrêtés par le minuteur.
    restopped: usize,
    /// Arrêtés de nouveau par le minuteur à son dernier tour : s'ils tournent encore au suivant,
    /// Windows les relance aussitôt arrêtés.
    just_restopped: HashSet<String>,
    /// Relancés par Windows aussitôt arrêtés : un autre service s'en sert (ICS pour le réseau de
    /// WSL 2, par exemple). Le minuteur et « Tout arrêter » les laissent tourner jusqu'à la
    /// fermeture de GameBooster, ou jusqu'à ce qu'on les arrête un à un.
    relaunched: HashSet<String>,
    note: Option<String>,
    /// Fermer GameBooster dès les services relancés.
    quit_after: bool,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
    /// Vrai pendant que GameBooster remplit la liste : ses notifications ne viennent pas d'un clic.
    static FILLING: Cell<bool> = const { Cell::new(false) };
}

/// Exécute `f` sur l'état de la fenêtre ; `None` s'il est déjà emprunté. Les appels Win32 qui
/// renvoient aussitôt un message à la fenêtre (SetFocus, MessageBoxW, SetWindowPos…) se font
/// donc toujours hors de `f`.
fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Lecture seule de l'état, pour dessiner.
fn read_ui<R>(f: impl FnOnce(&Ui) -> R) -> Option<R> {
    UI.with(|cell| cell.try_borrow().ok()?.as_ref().map(f))
}

/// Second lancement : réaffiche la fenêtre du premier GameBooster.
pub fn show_existing() {
    unsafe {
        if let Ok(hwnd) = FindWindowW(CLASS, PCWSTR::null()) {
            // C'est l'utilisateur qui vient de relancer GameBooster : le premier a le droit de
            // passer au premier plan.
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32));
            let _ = AllowSetForegroundWindow(pid);
            let _ = PostMessageW(Some(hwnd), WM_APP_SHOW, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn run() {
    let elevated = services::is_elevated();
    let choices = state::load_choices();
    let (session, ended) = previous_session();
    unsafe {
        let instance: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
        let background = CreateSolidBrush(BG);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
            hbrBackground: background,
            ..Default::default()
        };
        RegisterClassW(&wc);

        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS,
            w!("GameBooster"),
            STYLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        ) else {
            return;
        };
        let stop_all = button(hwnd, instance, ID_STOP_ALL, w!("Tout arrêter"));
        let start_all = button(hwnd, instance, ID_START_ALL, w!("Tout relancer"));
        let list = listview::create(hwnd, instance, ID_LIST);
        let one = button(hwnd, instance, ID_ONE, w!("Arrêter"));
        dark_title_bar(hwnd);
        // 0 si Windows ne le donne pas : on part alors de 100 %.
        let dpi = match GetDpiForWindow(hwnd) {
            0 => 96,
            dpi => dpi,
        };
        let fonts = Fonts::new(dpi);
        set_font(list, fonts.list);
        UI.with(|cell| {
            *cell.borrow_mut() = Some(Ui {
                stop_all,
                start_all,
                one,
                list,
                dpi,
                fonts,
                elevated,
                working: Working::Idle,
                rows: Vec::new(),
                infos: Vec::new(),
                ram: HashMap::new(),
                system_ram: None,
                shown: Vec::new(),
                sort: (Column::Category, false),
                selected: None,
                description: None,
                choices,
                session,
                allowed: HashSet::new(),
                refused: Vec::new(),
                failed: Vec::new(),
                restopped: 0,
                just_restopped: HashSet::new(),
                relaunched: HashSet::new(),
                note: None,
                quit_after: false,
            });
        });
        listview::set_sort(list, 1, false);
        // Taille de départ ; WM_SIZE place ensuite les contrôles.
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: px(START_W, dpi),
            bottom: px(START_H, dpi),
        };
        let _ = AdjustWindowRectExForDpi(&mut rect, STYLE, false, WINDOW_EX_STYLE::default(), dpi);
        let flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE;
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            rect.right - rect.left,
            rect.bottom - rect.top,
            flags,
        );
        refresh(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOW);
        SetTimer(Some(hwnd), REFRESH_TIMER, REFRESH_MS, None);
        sync_controls(hwnd);
        let _ = SetFocus(Some(list));
        // Windows a redémarré depuis le dernier lancement : les services sont revenus d'eux-mêmes,
        // on termine la session (relancer un service déjà en route ne fait rien).
        if ended {
            on_start_all(hwnd);
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // Tab entre les contrôles, Espace pour cocher, comme dans une boîte de dialogue.
            if !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if let Some(ui) = UI.with(|cell| cell.borrow_mut().take()) {
            ui.fonts.delete();
        }
        let _ = DeleteObject(background.into());
    }
}

/// La session laissée par un lancement précédent, et vrai si Windows a redémarré depuis.
fn previous_session() -> (Option<Session>, bool) {
    let Some(session) = state::load() else {
        return (None, false);
    };
    let uptime = unsafe { GetTickCount64() } / 1000;
    let ended = state::ended_by_reboot(session.since, state::now(), uptime);
    (Some(session), ended)
}

/// Bouton dessiné par GameBooster (BS_OWNERDRAW) : voir `draw_button`.
fn button(parent: HWND, instance: HINSTANCE, id: usize, text: PCWSTR) -> HWND {
    let style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32);
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            text,
            style,
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(id as *mut c_void)),
            Some(instance),
            None,
        )
        .unwrap_or_default()
    }
}

fn set_font(hwnd: HWND, font: HFONT) {
    unsafe {
        SendMessageW(
            hwnd,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
    }
}

/// Barre de titre sombre, de la couleur du fond (Windows 11 ; ignoré ailleurs).
fn dark_title_bar(hwnd: HWND) {
    let dark: i32 = 1;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark as *const i32).cast(),
            4,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            (&BG as *const COLORREF).cast(),
            4,
        );
    }
}

/// Les zones de la fenêtre, pour une zone client de `w` × `h` pixels au DPI `dpi`.
struct Areas {
    status: RECT,
    list: RECT,
    details: RECT,
    footer: RECT,
}

impl Areas {
    fn new(w: i32, h: i32, dpi: u32) -> Areas {
        let p = |v| px(v, dpi);
        let footer = RECT {
            left: p(PAD),
            top: h - p(PAD) - p(FOOTER_H),
            right: w - p(PAD),
            bottom: h - p(PAD),
        };
        let details = RECT {
            left: p(PAD),
            top: footer.top - p(10) - p(DETAILS_H),
            right: w - p(PAD),
            bottom: footer.top - p(10),
        };
        let list = RECT {
            left: p(PAD),
            top: p(LIST_TOP),
            right: w - p(PAD),
            bottom: details.top - p(12),
        };
        let status = RECT {
            left: p(PAD) + 2 * p(TOP_BTN_W) + p(GAP) + p(20),
            top: p(PAD) - p(2),
            right: w - p(PAD),
            bottom: p(LIST_TOP) - p(4),
        };
        Areas {
            status,
            list,
            details,
            footer,
        }
    }
}

fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    (rect.right, rect.bottom)
}

/// Place les contrôles selon la taille de la fenêtre.
fn layout(hwnd: HWND) {
    let Some((dpi, stop_all, start_all, list, one)) =
        read_ui(|ui| (ui.dpi, ui.stop_all, ui.start_all, ui.list, ui.one))
    else {
        return;
    };
    let (w, h) = client_size(hwnd);
    let areas = Areas::new(w, h, dpi);
    let p = |v| px(v, dpi);
    let flags = SWP_NOZORDER | SWP_NOACTIVATE;
    let (l, d) = (areas.list, areas.details);
    unsafe {
        let _ = SetWindowPos(
            stop_all,
            None,
            p(PAD),
            p(PAD),
            p(TOP_BTN_W),
            p(TOP_BTN_H),
            flags,
        );
        let x = p(PAD) + p(TOP_BTN_W) + p(GAP);
        let _ = SetWindowPos(
            start_all,
            None,
            x,
            p(PAD),
            p(TOP_BTN_W),
            p(TOP_BTN_H),
            flags,
        );
        let _ = SetWindowPos(
            list,
            None,
            l.left,
            l.top,
            l.right - l.left,
            l.bottom - l.top,
            flags,
        );
        let x = d.right - p(INNER) - p(ONE_BTN_W);
        let y = d.top + p(INNER) - p(2);
        let _ = SetWindowPos(one, None, x, y, p(ONE_BTN_W), p(ONE_BTN_H), flags);
    }
    listview::fit_columns(list, l.right - l.left, dpi);
    invalidate(hwnd);
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// Ce qu'affiche chaque ligne de la liste.
fn lines(rows: &[Row]) -> Vec<Line> {
    rows.iter()
        .map(|row| Line {
            name: row.name.clone(),
            cells: [
                row.entry.label.to_string(),
                row.entry.category.label().to_string(),
                row.entry.advice.label().to_string(),
                row.state.label().to_string(),
                row.ram_text(),
            ],
            checked: row.checked,
        })
        .collect()
}

/// Relit les services et la mémoire, puis met la liste à jour.
fn refresh(hwnd: HWND) {
    if unsafe { IsIconic(hwnd) }.as_bool() {
        return;
    }
    let Ok(scm) = Scm::open() else {
        return;
    };
    let infos = scm.services();
    let ram = memory::private_ram();
    let system_ram = memory::system_ram();
    let Some((list, old, new, selected)) = with_ui(|ui| {
        let mut rows = model::rows(&infos, &ram, &ui.choices);
        model::sort(&mut rows, ui.sort.0, ui.sort.1);
        let new = lines(&rows);
        let old = std::mem::replace(&mut ui.shown, new.clone());
        let selected = ui
            .selected
            .as_ref()
            .and_then(|name| rows.iter().position(|r| &r.name == name));
        ui.rows = rows;
        ui.infos = infos;
        ui.ram = ram;
        ui.system_ram = system_ram;
        (ui.list, old, new, selected)
    }) else {
        return;
    };
    FILLING.set(true);
    if listview::sync(list, &old, &new) {
        if let Some(index) = selected {
            listview::select(list, index);
        }
    }
    FILLING.set(false);
    update_one_button(hwnd);
    invalidate(hwnd);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            if ((wparam.0 >> 16) & 0xFFFF) as u32 == BN_CLICKED {
                match wparam.0 & 0xFFFF {
                    ID_STOP_ALL => on_stop_all(hwnd),
                    ID_START_ALL => on_start_all(hwnd),
                    ID_ONE => on_one(hwnd),
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_NOTIFY => on_notify(hwnd, lparam),
        WM_DRAWITEM => {
            draw_button(unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) });
            LRESULT(1)
        }
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        // Tout est dessiné dans WM_PAINT, en double tampon : pas de scintillement.
        WM_ERASEBKGND => LRESULT(1),
        WM_SIZE => {
            if wparam.0 as u32 != SIZE_MINIMIZED {
                layout(hwnd);
            }
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: px(MIN_W, dpi),
                bottom: px(MIN_H, dpi),
            };
            unsafe {
                let _ = AdjustWindowRectExForDpi(
                    &mut rect,
                    STYLE,
                    false,
                    WINDOW_EX_STYLE::default(),
                    dpi,
                );
                let info = &mut *(lparam.0 as *mut MINMAXINFO);
                info.ptMinTrackSize = POINT {
                    x: rect.right - rect.left,
                    y: rect.bottom - rect.top,
                };
            }
            LRESULT(0)
        }
        WM_TIMER => {
            match wparam.0 {
                REFRESH_TIMER => refresh(hwnd),
                WATCH_TIMER => watch(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_APP_DONE => {
            let done = unsafe { Box::from_raw(lparam.0 as *mut Done) };
            on_done(hwnd, *done);
            LRESULT(0)
        }
        WM_APP_SHOW => {
            unsafe {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let suggested = unsafe { *(lparam.0 as *const RECT) };
            on_dpi_changed(hwnd, (wparam.0 & 0xFFFF) as u32, suggested);
            LRESULT(0)
        }
        WM_CLOSE => {
            on_close(hwnd);
            LRESULT(0)
        }
        WM_ENDSESSION => {
            if wparam.0 != 0 {
                restore_now();
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Notifications de la liste : couleurs des cellules, cases, sélection, tri, clic droit.
fn on_notify(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let hdr = unsafe { &*(lparam.0 as *const NMHDR) };
    if hdr.idFrom != ID_LIST {
        return LRESULT(0);
    }
    match hdr.code {
        NM_CUSTOMDRAW => {
            let draw = unsafe { &mut *(lparam.0 as *mut NMLVCUSTOMDRAW) };
            LRESULT(list_colors(draw) as isize)
        }
        LVN_ITEMCHANGED => {
            if !FILLING.get() {
                on_item_changed(hwnd, unsafe { &*(lparam.0 as *const NMLISTVIEW) });
            }
            LRESULT(0)
        }
        LVN_COLUMNCLICK => {
            on_column_click(hwnd, unsafe { &*(lparam.0 as *const NMLISTVIEW) }.iSubItem);
            LRESULT(0)
        }
        NM_RCLICK => {
            on_right_click(hwnd);
            LRESULT(0)
        }
        _ => LRESULT(0),
    }
}

/// Couleur du texte de chaque cellule : le conseil en couleur, le reste selon l'état.
fn list_colors(draw: &mut NMLVCUSTOMDRAW) -> u32 {
    let stage = draw.nmcd.dwDrawStage.0;
    if stage == CDDS_PREPAINT.0 {
        return CDRF_NOTIFYITEMDRAW;
    }
    if stage == CDDS_ITEMPREPAINT.0 {
        return CDRF_NOTIFYSUBITEMDRAW;
    }
    if stage == CDDS_ITEMPREPAINT.0 | CDDS_SUBITEM.0 {
        let (index, column) = (draw.nmcd.dwItemSpec, draw.iSubItem);
        if let Some(color) = read_ui(|ui| cell_color(ui, index, column)).flatten() {
            draw.clrText = color;
        }
    }
    CDRF_DODEFAULT
}

fn cell_color(ui: &Ui, index: usize, column: i32) -> Option<COLORREF> {
    let row = ui.rows.get(index)?;
    let running = row.state.is_running();
    Some(match column {
        1 => DIM,
        2 => theme::advice_color(row.entry.advice),
        3 if ui.relaunched.contains(&row.name) => AMBER,
        3 if running => TEXT,
        3 if by_app(ui, &row.name) => BLUE,
        3 | 4 if !running => DIM,
        _ => TEXT,
    })
}

/// Vrai si le service a été arrêté par GameBooster, et attend d'être relancé.
fn by_app(ui: &Ui, name: &str) -> bool {
    ui.session
        .as_ref()
        .is_some_and(|s| s.stopped.iter().any(|n| n == name))
}

fn selected_row(ui: &Ui) -> Option<&Row> {
    let name = ui.selected.as_ref()?;
    ui.rows.iter().find(|r| &r.name == name)
}

fn on_item_changed(hwnd: HWND, change: &NMLISTVIEW) {
    let Ok(index) = usize::try_from(change.iItem) else {
        return;
    };
    if let Some(checked) = listview::check_change(change) {
        let others = with_ui(|ui| {
            let others = check_row(ui, index, checked)?;
            let _ = state::save_choices(&ui.choices);
            Some((ui.list, others))
        })
        .flatten();
        if let Some((list, others)) = others {
            FILLING.set(true);
            for i in others {
                listview::set_checked(list, i, checked);
            }
            FILLING.set(false);
        }
        sync_controls(hwnd);
    }
    if listview::selection_changed(change) {
        on_selection(hwnd);
    }
}

/// Coche ou décoche la ligne `index`, et avec elle toutes celles de la même entrée (services à
/// numéro de version). Cocher n'arrête rien, même en mode jeu : le minuteur laisse tourner un
/// service coché en cours jusqu'au prochain « Tout arrêter ». Renvoie les autres lignes de
/// l'entrée, dont la case est à mettre à jour.
fn check_row(ui: &mut Ui, index: usize, checked: bool) -> Option<Vec<usize>> {
    let entry = ui.rows.get(index)?.entry;
    ui.choices.set(entry, checked);
    let game = ui.session.as_ref().filter(|s| s.game_mode);
    let mut others = Vec::new();
    for (i, row) in ui.rows.iter_mut().enumerate() {
        if !std::ptr::eq(row.entry, entry) {
            continue;
        }
        row.checked = checked;
        if let Some(line) = ui.shown.get_mut(i) {
            line.checked = checked;
        }
        let running = row.state.is_running();
        if checked && running && game.is_some_and(|s| !s.stopped.contains(&row.name)) {
            ui.allowed.insert(row.name.clone());
        }
        if i != index {
            others.push(i);
        }
    }
    Some(others)
}

fn on_selection(hwnd: HWND) {
    let Some(list) = read_ui(|ui| ui.list) else {
        return;
    };
    let index = listview::selected(list);
    let unread = with_ui(|ui| {
        ui.selected = index.and_then(|i| ui.rows.get(i)).map(|r| r.name.clone());
        let name = ui.selected.clone()?;
        let known = ui.description.as_ref().is_some_and(|(n, _)| *n == name);
        (!known).then_some(name)
    })
    .flatten();
    if let Some(name) = unread {
        let description = Scm::open().ok().and_then(|scm| scm.description(&name));
        with_ui(|ui| ui.description = Some((name, description)));
    }
    update_one_button(hwnd);
    invalidate(hwnd);
}

fn on_column_click(hwnd: HWND, column: i32) {
    let Some(&kind) = usize::try_from(column)
        .ok()
        .and_then(|c| Column::ALL.get(c))
    else {
        return;
    };
    let Some((list, reverse)) = with_ui(|ui| {
        ui.sort = if ui.sort.0 == kind {
            (kind, !ui.sort.1)
        } else {
            (kind, false)
        };
        // Liste refaite dans le nouvel ordre.
        ui.shown.clear();
        (ui.list, ui.sort.1)
    }) else {
        return;
    };
    listview::set_sort(list, column as usize, reverse);
    refresh(hwnd);
}

/// Ce que fait le bouton du service sélectionné : son texte, et s'il est utilisable.
fn one_action(ui: &Ui) -> Option<(&'static str, bool)> {
    let row = selected_row(ui)?;
    let label = if row.state.is_running() {
        "Arrêter"
    } else if by_app(ui, &row.name) {
        "Relancer"
    } else {
        "Démarrer"
    };
    let enabled = ui.elevated && ui.working == Working::Idle && !row.state.is_pending();
    Some((label, enabled))
}

fn window_text(hwnd: HWND) -> String {
    let mut buffer = [0u16; 64];
    let len = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..usize::try_from(len).unwrap_or(0)])
}

fn update_one_button(hwnd: HWND) {
    let Some((one, list, action)) = read_ui(|ui| (ui.one, ui.list, one_action(ui))) else {
        return;
    };
    let (label, enabled) = action.unwrap_or(("Arrêter", false));
    unsafe {
        if window_text(one) != label {
            let _ = SetWindowTextW(one, &HSTRING::from(label));
            let _ = InvalidateRect(Some(one), None, false);
        }
        let _ = EnableWindow(one, enabled);
        // Bouton grisé qui avait le focus clavier : le focus passe à la liste. Seulement si
        // GameBooster est au premier plan : SetFocus activerait sinon sa fenêtre, par-dessus le jeu.
        if !enabled && GetFocus() == one && GetForegroundWindow() == hwnd {
            let _ = SetFocus(Some(list));
        }
    }
}

fn on_right_click(hwnd: HWND) {
    let Some((label, enabled, service)) = read_ui(|ui| {
        let (label, enabled) = one_action(ui)?;
        Some((label, enabled, selected_row(ui)?.entry.label))
    })
    .flatten() else {
        return;
    };
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let flags = if enabled {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED
        };
        let text = HSTRING::from(format!("{label} « {service} »"));
        let _ = AppendMenuW(menu, flags, ID_MENU_ONE, &text);
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
        if command.0 as usize == ID_MENU_ONE {
            on_one(hwnd);
        }
    }
}

/// Arrête ou démarre le service sélectionné.
fn on_one(hwnd: HWND) {
    let Some((name, running, avoid, label, note)) = read_ui(|ui| {
        let (_, enabled) = one_action(ui)?;
        let row = selected_row(ui)?;
        enabled.then(|| {
            (
                row.name.clone(),
                row.state.is_running(),
                row.entry.advice == Advice::Avoid,
                row.entry.label,
                row.entry.note,
            )
        })
    })
    .flatten() else {
        return;
    };
    if running && avoid {
        let text = format!("{label} : déconseillé.\n\n{note}\n\nL'arrêter quand même ?");
        let answer = unsafe {
            MessageBoxW(
                Some(hwnd),
                &HSTRING::from(text),
                w!("GameBooster"),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
            )
        };
        if answer != IDYES {
            return;
        }
    }
    let names = vec![name];
    let job = if running {
        Job::Stop { names, all: false }
    } else {
        Job::Start { names, all: false }
    };
    launch(hwnd, job);
}

fn on_stop_all(hwnd: HWND) {
    let names = read_ui(|ui| {
        if ui.working != Working::Idle || !ui.elevated {
            return None;
        }
        Some(stop_all_names(ui))
    })
    .flatten();
    if let Some(names) = names {
        launch(hwnd, Job::Stop { names, all: true });
    }
}

/// Ce qu'arrête « Tout arrêter » : les services cochés en cours, sauf ceux que Windows relance
/// aussitôt ; dans l'ordre du catalogue, un service avant ceux dont il dépend.
fn stop_all_names(ui: &Ui) -> Vec<String> {
    let mut chosen: Vec<&Row> = ui
        .rows
        .iter()
        .filter(|r| stoppable(ui, r) && r.state.is_running())
        .collect();
    chosen.sort_by_key(|r| r.index);
    chosen.into_iter().map(|r| r.name.clone()).collect()
}

/// Coché, et pas relancé par Windows aussitôt arrêté : « Tout arrêter » l'arrête s'il tourne.
fn stoppable(ui: &Ui, row: &Row) -> bool {
    row.checked && !ui.relaunched.contains(&row.name)
}

fn on_start_all(hwnd: HWND) {
    let names = read_ui(|ui| {
        if ui.working != Working::Idle {
            return None;
        }
        let session = ui.session.as_ref()?;
        // Ordre inverse de l'arrêt : un service après ceux dont il dépend.
        Some(session.stopped.iter().rev().cloned().collect())
    })
    .flatten();
    if let Some(names) = names {
        launch(hwnd, Job::Start { names, all: true });
    }
}

/// Lance `job` dans un fil à part ; le résultat revient à la fenêtre par WM_APP_DONE.
fn launch(hwnd: HWND, job: Job) {
    let working = match job {
        Job::Stop { .. } => Working::Stopping,
        Job::Start { .. } => Working::Starting,
    };
    with_ui(|ui| {
        ui.working = working;
        ui.note = None;
        ui.refused.clear();
        ui.failed.clear();
    });
    sync_controls(hwnd);
    let target = hwnd.0 as isize;
    thread::spawn(move || {
        let done = Box::into_raw(Box::new(run_job(job)));
        let hwnd = HWND(target as *mut c_void);
        let posted =
            unsafe { PostMessageW(Some(hwnd), WM_APP_DONE, WPARAM(0), LPARAM(done as isize)) };
        if posted.is_err() {
            // Fenêtre déjà fermée : personne ne récupérera le résultat.
            drop(unsafe { Box::from_raw(done) });
        }
    });
}

fn run_job(job: Job) -> Done {
    let scm = match Scm::open() {
        Ok(scm) => scm,
        Err(error) => return Done::Failed(error),
    };
    match job {
        Job::Stop { names, all } => {
            // Processus et RAM de chaque service avant l'arrêt, pour mesurer ce qui est libéré.
            let pids: HashMap<String, u32> = scm
                .services()
                .into_iter()
                .filter(|i| i.state.is_running())
                .map(|i| (i.name, i.pid))
                .collect();
            let before = memory::private_ram();
            let mut stopped = Vec::new();
            let mut refused = Vec::new();
            for name in names {
                match services::stop(&scm, &name) {
                    Stop::Stopped => stopped.push(name),
                    Stop::Idle => {}
                    Stop::Refused(why) => refused.push((name, why)),
                }
            }
            let freed = if stopped.is_empty() {
                0
            } else {
                // Le temps que les processus vidés de leurs services se ferment.
                thread::sleep(Duration::from_millis(500));
                model::freed(&stopped, &pids, &before, &memory::private_ram())
            };
            Done::Stopped {
                all,
                stopped,
                refused,
                freed,
            }
        }
        Job::Start { names, all } => {
            let mut started = Vec::new();
            let mut failed = Vec::new();
            for name in names {
                match services::start(&scm, &name) {
                    Ok(()) => started.push(name),
                    Err(why) => failed.push((name, why)),
                }
            }
            Done::Started {
                all,
                started,
                failed,
            }
        }
    }
}

fn persist(session: Option<&Session>) {
    match session {
        Some(session) => {
            let _ = state::save(session);
        }
        None => state::clear(),
    }
}

fn on_done(hwnd: HWND, done: Done) {
    let quit = with_ui(|ui| {
        ui.working = Working::Idle;
        // Un arrêt ou une relance entre deux tours du minuteur : il repart de zéro.
        ui.just_restopped.clear();
        match done {
            Done::Stopped {
                all,
                stopped,
                refused,
                freed,
            } => {
                let session = ui
                    .session
                    .get_or_insert_with(|| Session::new(state::now(), local_time()));
                for name in stopped {
                    ui.allowed.remove(&name);
                    ui.relaunched.remove(&name);
                    if !session.stopped.contains(&name) {
                        session.stopped.push(name);
                    }
                }
                session.freed += freed;
                if all {
                    session.game_mode = true;
                    ui.allowed.clear();
                    ui.restopped = 0;
                }
                ui.refused = refused;
            }
            Done::Started {
                all,
                started,
                failed,
            } => {
                if all {
                    ui.session = None;
                    ui.allowed.clear();
                    ui.restopped = 0;
                    if failed.is_empty() {
                        ui.note = Some(format!(
                            "Mode jeu terminé : {}.",
                            count(started.len(), "service relancé", "services relancés")
                        ));
                    }
                } else {
                    let mut ended = false;
                    if let Some(session) = ui.session.as_mut() {
                        session.stopped.retain(|n| !started.contains(n));
                        if session.game_mode {
                            ui.allowed.extend(started);
                        }
                        ended = session.stopped.is_empty() && !session.game_mode;
                    }
                    if ended {
                        ui.session = None;
                    }
                }
                ui.failed = failed;
            }
            Done::Failed(error) => ui.note = Some(format!("Échec : {error}")),
        }
        persist(ui.session.as_ref());
        // Relance ratée : GameBooster reste ouvert, la session aussi.
        if ui.session.is_some() {
            ui.quit_after = false;
        }
        ui.quit_after
    });
    refresh(hwnd);
    sync_controls(hwnd);
    if quit == Some(true) {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

/// Toutes les 30 s pendant une session : arrête à nouveau les services que Windows a relancés
/// et, en mode jeu, les services cochés qu'il a démarrés (sur beaucoup de PC, Windows Update ou
/// BITS ne tournent pas au moment du clic, puis démarrent en pleine partie). Ces derniers
/// rejoignent la session : « Tout relancer » les relancera aussi. Rapide : la demande d'arrêt
/// part sans attendre qu'il soit fait.
///
/// Un service arrêté à un tour qui tourne de nouveau au suivant, Windows le relance aussitôt
/// arrêté : un autre service s'en sert. Le réarrêter toutes les 30 s ne servirait à rien et
/// dérangerait ce service : il est laissé tourner (voir `Ui::relaunched`).
fn watch(hwnd: HWND) {
    let Ok(scm) = Scm::open() else {
        return;
    };
    let running: HashSet<String> = scm
        .services()
        .into_iter()
        .filter(|i| i.state.is_running())
        .map(|i| i.name)
        .collect();
    let Some((to_stop, relaunched)) = read_ui(|ui| watch_plan(ui, &running)).flatten() else {
        return;
    };
    let stopped: Vec<String> = to_stop
        .into_iter()
        .filter(|name| services::request_stop(&scm, name))
        .collect();
    let Some((changed, ended)) = with_ui(|ui| {
        let (changed, ended) = after_watch(ui, stopped, relaunched);
        if changed {
            persist(ui.session.as_ref());
        }
        (changed, ended)
    }) else {
        return;
    };
    if ended {
        sync_controls(hwnd);
    }
    if changed {
        refresh(hwnd);
    }
}

/// Le tour du minuteur, d'après les services qui tournent (`running`) : ceux à arrêter de nouveau,
/// dans l'ordre, et ceux que Windows a relancés depuis le tour précédent, qui les avait arrêtés :
/// à laisser tourner. `None` : pas de tour (travail en cours, pas de session ou pas les droits).
fn watch_plan(ui: &Ui, running: &HashSet<String>) -> Option<(Vec<String>, Vec<String>)> {
    if ui.working != Working::Idle || !ui.elevated {
        return None;
    }
    let session = ui.session.as_ref()?;
    let watched = |name: &str| !ui.allowed.contains(name) && !ui.relaunched.contains(name);
    let mut names: Vec<String> = session
        .stopped
        .iter()
        .filter(|n| watched(n))
        .cloned()
        .collect();
    if session.game_mode {
        let mut extra: Vec<&Row> = ui
            .rows
            .iter()
            .filter(|r| r.checked && !session.stopped.contains(&r.name) && watched(&r.name))
            .collect();
        extra.sort_by_key(|r| r.index);
        names.extend(extra.into_iter().map(|r| r.name.clone()));
    }
    let (relaunched, to_stop): (Vec<String>, Vec<String>) = names
        .into_iter()
        .filter(|n| running.contains(n))
        .partition(|n| ui.just_restopped.contains(n));
    Some((to_stop, relaunched))
}

/// Ce que retient un tour du minuteur : `stopped`, arrêtés de nouveau ; `relaunched`, relancés
/// aussitôt par Windows, laissés tourner et retirés de la session. Renvoie : quelque chose a
/// changé, et la session se termine faute de service à relancer.
fn after_watch(ui: &mut Ui, stopped: Vec<String>, relaunched: Vec<String>) -> (bool, bool) {
    let changed = !stopped.is_empty() || !relaunched.is_empty();
    ui.restopped += stopped.len();
    ui.just_restopped = stopped.iter().cloned().collect();
    let Some(session) = ui.session.as_mut() else {
        return (changed, false);
    };
    session.stopped.retain(|n| !relaunched.contains(n));
    for name in stopped {
        if !session.stopped.contains(&name) {
            session.stopped.push(name);
        }
    }
    let ended = session.stopped.is_empty() && !session.game_mode;
    ui.relaunched.extend(relaunched);
    if ended {
        ui.session = None;
    }
    (changed, ended)
}

/// Fin de la session Windows (déconnexion, arrêt, redémarrage) : relance tout de suite les
/// services arrêtés. Sans ça, après un arrêt avec le démarrage rapide de Windows, ils resteraient
/// arrêtés au prochain allumage.
fn restore_now() {
    let names = with_ui(|ui| {
        if ui.working != Working::Idle {
            return None;
        }
        ui.session.take().map(|s| s.stopped)
    })
    .flatten();
    let Some(names) = names else {
        return;
    };
    if let Ok(scm) = Scm::open() {
        for name in names.iter().rev() {
            let _ = services::start(&scm, name);
        }
    }
    state::clear();
}

/// Fermeture avec des services encore arrêtés : propose de les relancer d'abord.
fn on_close(hwnd: HWND) {
    let destroy = || unsafe {
        let _ = DestroyWindow(hwnd);
    };
    let Some((working, session)) = read_ui(|ui| (ui.working, ui.session.is_some())) else {
        destroy();
        return;
    };
    // Quelques secondes de travail en cours : on attend qu'il finisse.
    if working != Working::Idle {
        return;
    }
    if !session {
        destroy();
        return;
    }
    let answer = unsafe {
        MessageBoxW(
            Some(hwnd),
            w!("Des services arrêtés par GameBooster le sont encore.\n\n\
                Oui : les relancer, puis quitter.\n\
                Non : quitter sans les relancer. Le prochain lancement de GameBooster les \
                retrouvera ; un redémarrage de Windows les relance aussi.\n\
                Annuler : revenir à GameBooster."),
            w!("GameBooster"),
            MB_YESNOCANCEL | MB_ICONQUESTION,
        )
    };
    if answer == IDYES {
        with_ui(|ui| ui.quit_after = true);
        on_start_all(hwnd);
    } else if answer == IDNO {
        destroy();
    }
}

/// Grise ou dégrise les boutons selon l'état, et lance ou arrête le minuteur de la session.
fn sync_controls(hwnd: HWND) {
    let Some((stop_all, start_all, list, can_stop, can_start, session)) = read_ui(|ui| {
        let idle = ui.working == Working::Idle && ui.elevated;
        (
            ui.stop_all,
            ui.start_all,
            ui.list,
            idle && ui.rows.iter().any(|r| r.checked),
            idle && ui.session.is_some(),
            ui.session.is_some(),
        )
    }) else {
        return;
    };
    unsafe {
        let _ = EnableWindow(stop_all, can_stop);
        let _ = EnableWindow(start_all, can_start);
        if session {
            SetTimer(Some(hwnd), WATCH_TIMER, WATCH_MS, None);
        } else {
            let _ = KillTimer(Some(hwnd), WATCH_TIMER);
        }
        // Bouton grisé qui avait le focus clavier : le focus passe à la liste (seulement au
        // premier plan, comme dans `update_one_button`).
        let focus = GetFocus();
        let lost = (focus == stop_all || focus == start_all) && !IsWindowEnabled(focus).as_bool();
        if lost && GetForegroundWindow() == hwnd {
            let _ = SetFocus(Some(list));
        }
    }
    update_one_button(hwnd);
    invalidate(hwnd);
}

/// La fenêtre change d'écran, ou l'écran change d'échelle : polices et mise en page au nouveau
/// DPI, à la place proposée par Windows.
fn on_dpi_changed(hwnd: HWND, dpi: u32, suggested: RECT) {
    let Some((list, fonts, old)) = with_ui(|ui| {
        let old = ui.fonts;
        ui.fonts = Fonts::new(dpi);
        ui.dpi = dpi;
        (ui.list, ui.fonts, old)
    }) else {
        return;
    };
    set_font(list, fonts.list);
    old.delete();
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            suggested.left,
            suggested.top,
            suggested.right - suggested.left,
            suggested.bottom - suggested.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    layout(hwnd);
}

fn paint(hwnd: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let (w, h) = client_size(hwnd);
        if w > 0 && h > 0 {
            // Double tampon : la fenêtre est redessinée toutes les 3 s.
            let memory_dc = CreateCompatibleDC(Some(hdc));
            let bitmap = CreateCompatibleBitmap(hdc, w, h);
            let old = SelectObject(memory_dc, bitmap.into());
            let client = RECT {
                left: 0,
                top: 0,
                right: w,
                bottom: h,
            };
            fill(memory_dc, &client, BG);
            SetBkMode(memory_dc, TRANSPARENT);
            read_ui(|ui| draw(memory_dc, ui, w, h));
            let _ = BitBlt(hdc, 0, 0, w, h, Some(memory_dc), 0, 0, SRCCOPY);
            SelectObject(memory_dc, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory_dc);
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

fn draw(hdc: HDC, ui: &Ui, w: i32, h: i32) {
    let areas = Areas::new(w, h, ui.dpi);
    draw_status(hdc, ui, &areas.status);
    draw_details(hdc, ui, &areas.details);
    draw_footer(hdc, ui, &areas.footer);
}

/// Écrit `text` dans `rect`, avec la police et la couleur données.
fn put(hdc: HDC, font: HFONT, color: COLORREF, text: &str, rect: RECT, format: DRAW_TEXT_FORMAT) {
    unsafe {
        let old = SelectObject(hdc, font.into());
        SetTextColor(hdc, color);
        let mut rect = rect;
        DrawTextW(hdc, &mut wide(text), &mut rect, format | DT_NOPREFIX);
        SelectObject(hdc, old);
    }
}

/// Largeur de `text` dans la police donnée.
fn text_width(hdc: HDC, font: HFONT, text: &str) -> i32 {
    let mut size = SIZE::default();
    unsafe {
        let old = SelectObject(hdc, font.into());
        let _ = GetTextExtentPoint32W(hdc, &wide(text), &mut size);
        SelectObject(hdc, old);
    }
    size.cx
}

const ONE_LINE: DRAW_TEXT_FORMAT =
    DRAW_TEXT_FORMAT(DT_LEFT.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0);
const WRAPPED: DRAW_TEXT_FORMAT =
    DRAW_TEXT_FORMAT(DT_LEFT.0 | DT_WORDBREAK.0 | DT_EDITCONTROL.0 | DT_END_ELLIPSIS.0);

fn draw_status(hdc: HDC, ui: &Ui, area: &RECT) {
    let (title, detail) = status(ui);
    let p = |v| px(v, ui.dpi);
    let title_rect = RECT {
        bottom: area.top + p(22),
        ..*area
    };
    put(hdc, ui.fonts.title, TEXT, &title, title_rect, ONE_LINE);
    let detail_rect = RECT {
        top: area.top + p(23),
        ..*area
    };
    put(hdc, ui.fonts.text, DIM, &detail, detail_rect, WRAPPED);
}

fn draw_details(hdc: HDC, ui: &Ui, area: &RECT) {
    let p = |v| px(v, ui.dpi);
    fill(hdc, area, PANEL);
    let Some(row) = selected_row(ui) else {
        let format = DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS;
        let hint = "Sélectionne un service dans la liste pour voir à quoi il sert.";
        put(hdc, ui.fonts.text, DIM, hint, *area, format);
        return;
    };
    let left = area.left + p(INNER);
    let beside_button = area.right - p(INNER) - p(ONE_BTN_W) - p(12);
    let right = area.right - p(INNER);
    let line = |top: i32, height: i32, right: i32| RECT {
        left,
        top,
        right,
        bottom: top + height,
    };
    let mut y = area.top + p(INNER) - p(2);
    put(
        hdc,
        ui.fonts.title,
        TEXT,
        row.entry.label,
        line(y, p(20), beside_button),
        ONE_LINE,
    );
    y += p(22);
    let names = format!("{} · {}", row.name, row.display_name);
    put(
        hdc,
        ui.fonts.text,
        DIM,
        &names,
        line(y, p(17), beside_button),
        ONE_LINE,
    );
    y += p(19);
    let advice = row.entry.advice.label();
    let color = theme::advice_color(row.entry.advice);
    put(
        hdc,
        ui.fonts.text,
        color,
        advice,
        line(y, p(17), right),
        ONE_LINE,
    );
    let note = format!(" · {}", row.entry.note);
    let note_rect = RECT {
        left: left + text_width(hdc, ui.fonts.text, advice),
        ..line(y, p(17), right)
    };
    put(hdc, ui.fonts.text, TEXT, &note, note_rect, ONE_LINE);
    y += p(19);
    let (state, color) = state_line(ui, row);
    put(
        hdc,
        ui.fonts.text,
        color,
        &state,
        line(y, p(17), right),
        ONE_LINE,
    );
    y += p(21);
    let description = ui
        .description
        .as_ref()
        .filter(|(name, _)| *name == row.name)
        .and_then(|(_, text)| text.clone())
        .unwrap_or_else(|| "Windows ne donne pas de description pour ce service.".into());
    let rect = RECT {
        left,
        top: y,
        right,
        bottom: area.bottom - p(8),
    };
    put(hdc, ui.fonts.text, DIM, &description, rect, WRAPPED);
}

/// L'état du service sélectionné, en une ligne, et sa couleur.
fn state_line(ui: &Ui, row: &Row) -> (String, COLORREF) {
    let issue = |issues: &[Issue]| {
        issues
            .iter()
            .find(|(name, _)| *name == row.name)
            .map(|(_, why)| why.clone())
    };
    if let Some(why) = issue(&ui.refused) {
        return (format!("Windows a refusé de l'arrêter : {why}."), AMBER);
    }
    if let Some(why) = issue(&ui.failed) {
        let text =
            format!("Windows a refusé de le relancer : {why}. Un redémarrage s'en chargera.");
        return (text, AMBER);
    }
    if ui.relaunched.contains(&row.name) {
        let advice = if row.checked {
            " Mieux vaut le décocher."
        } else {
            ""
        };
        let text =
            format!("Windows le relance aussitôt arrêté : un autre service s'en sert.{advice}");
        return (text, AMBER);
    }
    match row.ram {
        _ if !row.state.is_running() && by_app(ui, &row.name) => (
            "Arrêté par GameBooster : « Relancer » ou « Tout relancer » le remet en marche.".into(),
            BLUE,
        ),
        _ if !row.state.is_running() => (format!("{}.", row.state.label()), DIM),
        Ram::Own(bytes) => (
            format!("{} · occupe {} de RAM.", row.state.label(), memory::format(bytes)),
            TEXT,
        ),
        Ram::Shared(n) => (
            format!(
                "{} · partage son processus avec {} : sa RAM n'est libérée que s'ils s'arrêtent tous.",
                row.state.label(),
                count(n.saturating_sub(1), "autre service", "autres services")
            ),
            TEXT,
        ),
        Ram::Unknown => (format!("{}.", row.state.label()), TEXT),
    }
}

fn draw_footer(hdc: HDC, ui: &Ui, area: &RECT) {
    let ram = ui
        .system_ram
        .map(|(used, total)| {
            format!(
                "RAM du PC : {} / {}",
                memory::format(used),
                memory::format(total)
            )
        })
        .unwrap_or_default();
    let ram_width = text_width(hdc, ui.fonts.text, &ram);
    put(
        hdc,
        ui.fonts.text,
        DIM,
        &ram,
        *area,
        DT_RIGHT | DT_SINGLELINE,
    );
    let hint = "Cochés : arrêtés par « Tout arrêter ». Clic droit sur un service : l'arrêter ou le relancer.";
    let rect = RECT {
        right: area.right - ram_width - px(24, ui.dpi),
        ..*area
    };
    put(hdc, ui.fonts.text, DIM, hint, rect, ONE_LINE);
}

/// Les deux lignes d'état, en haut à droite.
fn status(ui: &Ui) -> (String, String) {
    match ui.working {
        Working::Stopping => {
            return (
                "Arrêt en cours…".into(),
                "Les services s'arrêtent un à un.".into(),
            )
        }
        Working::Starting => {
            return (
                "Relance en cours…".into(),
                "Les services redémarrent.".into(),
            )
        }
        Working::Idle => {}
    }
    if !ui.elevated {
        let text = preview(ui, false).unwrap_or_default();
        return (
            "Lecture seule".into(),
            sentences(
                &text,
                "Relance GameBooster en administrateur pour arrêter des services.",
            ),
        );
    }
    let (title, mut detail) = match &ui.session {
        Some(session) if session.game_mode => {
            let mut parts = vec![
                count(session.stopped.len(), "service arrêté", "services arrêtés"),
                format!("≈ {} libérés", memory::format(session.freed)),
            ];
            if ui.restopped > 0 {
                parts.push(count(
                    ui.restopped,
                    "démarré par Windows, arrêté",
                    "démarrés par Windows, arrêtés",
                ));
            }
            (
                format!("Mode jeu actif depuis {}", session.since_text),
                parts.join(" · ") + ".",
            )
        }
        Some(session) => (
            count(
                session.stopped.len(),
                "service arrêté par GameBooster",
                "services arrêtés par GameBooster",
            ),
            format!(
                "≈ {} libérés. « Tout relancer » les remet en marche.",
                memory::format(session.freed)
            ),
        ),
        None => ("Mode jeu inactif".to_string(), String::new()),
    };
    if !ui.refused.is_empty() {
        let refused = count(
            ui.refused.len(),
            "service refusé par Windows",
            "services refusés par Windows",
        );
        detail = sentences(
            &detail,
            &format!("{refused} : sélectionne-les pour voir pourquoi."),
        );
    }
    if !ui.failed.is_empty() {
        detail = count(
            ui.failed.len(),
            "service non relancé : un redémarrage de Windows s'en chargera.",
            "services non relancés : un redémarrage de Windows s'en chargera.",
        );
    }
    if let Some(note) = &ui.note {
        detail.clone_from(note);
    }
    // Toujours en dernier : suit les cases à mesure qu'on les coche.
    if let Some(text) = preview(ui, ui.session.is_some()) {
        detail = sentences(&detail, &text);
    }
    (title, detail)
}

/// RAM que libérerait « Tout arrêter », d'après les cases cochées.
fn freeable(ui: &Ui) -> u64 {
    let chosen: HashSet<&str> = ui
        .rows
        .iter()
        .filter(|r| stoppable(ui, r))
        .map(|r| r.name.as_str())
        .collect();
    model::freeable(&ui.infos, &ui.ram, &chosen)
}

/// Ce que libérerait « Tout arrêter » avec les cases cochées : la RAM des services cochés en
/// cours. `more` : des services sont déjà arrêtés, ce serait en plus ; rien alors si aucun
/// service coché ne tourne.
fn preview(ui: &Ui, more: bool) -> Option<String> {
    let running = ui
        .rows
        .iter()
        .filter(|r| stoppable(ui, r) && r.state.is_running())
        .count();
    match (running, more) {
        (0, true) => None,
        (0, false) => Some("Aucun service coché ne tourne en ce moment.".into()),
        _ => Some(format!(
            "« Tout arrêter » libérerait ≈ {}{} ({}).",
            memory::format(freeable(ui)),
            if more { " de plus" } else { "" },
            count(
                running,
                "service coché en cours",
                "services cochés en cours"
            )
        )),
    }
}

/// Deux phrases à la suite ; la seconde seule si la première est vide.
fn sentences(first: &str, second: &str) -> String {
    if first.is_empty() {
        second.to_string()
    } else {
        format!("{first} {second}")
    }
}

/// « 1 service arrêté », « 21 services arrêtés ». Zéro s'accorde au singulier, en français.
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n > 1 { many } else { one })
}

/// L'heure locale : « 14 h 32 ».
fn local_time() -> String {
    let now = unsafe { GetLocalTime() };
    format!("{} h {:02}", now.wHour, now.wMinute)
}

/// Dessine un bouton : vert pour « Tout arrêter », gris pour les autres, sombre s'il est grisé,
/// cerclé de clair s'il a le focus clavier.
fn draw_button(item: &DRAWITEMSTRUCT) {
    let Some((font, dpi)) = read_ui(|ui| (ui.fonts.button, ui.dpi)) else {
        return;
    };
    let id = item.CtlID as usize;
    let state = item.itemState.0;
    let disabled = state & ODS_DISABLED.0 != 0;
    let pressed = state & ODS_SELECTED.0 != 0;
    let focused = state & ODS_FOCUS.0 != 0 && state & ODS_NOFOCUSRECT.0 == 0;
    let (fill_color, text_color) = match (id == ID_STOP_ALL, disabled, pressed) {
        (_, true, _) => (DISABLED, DISABLED_TEXT),
        (true, _, false) => (GREEN, WHITE),
        (true, _, true) => (GREEN_DOWN, WHITE),
        (false, _, false) => (GRAY, TEXT),
        (false, _, true) => (GRAY_DOWN, TEXT),
    };
    let background = if id == ID_ONE { PANEL } else { BG };
    let label = window_text(item.hwndItem);
    let (hdc, rect) = (item.hDC, item.rcItem);
    let radius = px(10, dpi);
    unsafe {
        fill(hdc, &rect, background);
        let brush = CreateSolidBrush(fill_color);
        let pen = CreatePen(
            PS_SOLID,
            px(2, dpi),
            if focused { TEXT } else { fill_color },
        );
        let old_brush = SelectObject(hdc, brush.into());
        let old_pen = SelectObject(hdc, pen.into());
        let _ = RoundRect(
            hdc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        let _ = DeleteObject(pen.into());
        let _ = DeleteObject(brush.into());
        SetBkMode(hdc, TRANSPARENT);
    }
    let centered = DT_CENTER | DT_VCENTER | DT_SINGLELINE;
    put(hdc, font, text_color, &label, rect, centered);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::State;

    #[test]
    fn french_plurals() {
        assert_eq!(
            count(0, "service arrêté", "services arrêtés"),
            "0 service arrêté"
        );
        assert_eq!(
            count(1, "service arrêté", "services arrêtés"),
            "1 service arrêté"
        );
        assert_eq!(
            count(2, "service arrêté", "services arrêtés"),
            "2 services arrêtés"
        );
    }

    fn sample() -> Ui {
        let infos = vec![
            Info {
                name: "wuauserv".into(),
                display_name: "Windows Update".into(),
                state: State::Running,
                pid: 10,
            },
            Info {
                name: "Spooler".into(),
                display_name: "Spouleur d'impression".into(),
                state: State::Running,
                pid: 20,
            },
        ];
        let ram = HashMap::from([(10, 30 << 20), (20, 8 << 20)]);
        let rows = model::rows(&infos, &ram, &Choices::default());
        Ui {
            stop_all: HWND::default(),
            start_all: HWND::default(),
            one: HWND::default(),
            list: HWND::default(),
            dpi: 96,
            fonts: Fonts {
                button: HFONT::default(),
                title: HFONT::default(),
                text: HFONT::default(),
                list: HFONT::default(),
            },
            elevated: true,
            working: Working::Idle,
            rows,
            infos,
            ram,
            system_ram: None,
            shown: Vec::new(),
            sort: (Column::Category, false),
            selected: None,
            description: None,
            choices: Choices::default(),
            session: None,
            allowed: HashSet::new(),
            refused: Vec::new(),
            failed: Vec::new(),
            restopped: 0,
            just_restopped: HashSet::new(),
            relaunched: HashSet::new(),
            note: None,
            quit_after: false,
        }
    }

    #[test]
    fn status_lines() {
        let mut ui = sample();
        // Le spouleur n'est pas coché : seule la RAM de Windows Update est libérable.
        assert_eq!(
            status(&ui),
            (
                "Mode jeu inactif".into(),
                "« Tout arrêter » libérerait ≈ 30 Mo (1 service coché en cours).".into()
            )
        );
        // « Tout arrêter » a arrêté Windows Update : rien de plus à libérer.
        ui.infos[0].state = State::Stopped;
        ui.infos[0].pid = 0;
        ui.rows = model::rows(&ui.infos, &ui.ram, &ui.choices);
        ui.session = Some(Session {
            since: 0,
            since_text: "14 h 32".into(),
            game_mode: true,
            stopped: vec!["wuauserv".into()],
            freed: 30 << 20,
        });
        ui.restopped = 2;
        assert_eq!(
            status(&ui),
            (
                "Mode jeu actif depuis 14 h 32".into(),
                "1 service arrêté · ≈ 30 Mo libérés · 2 démarrés par Windows, arrêtés.".into()
            )
        );
        // Le spouleur coché en plus : ce que « Tout arrêter » libérerait encore.
        ui.rows[1].checked = true;
        assert_eq!(
            status(&ui).1,
            "1 service arrêté · ≈ 30 Mo libérés · 2 démarrés par Windows, arrêtés. \
             « Tout arrêter » libérerait ≈ 8,0 Mo de plus (1 service coché en cours)."
        );
        ui.working = Working::Starting;
        assert_eq!(status(&ui).0, "Relance en cours…");

        // Après « Tout relancer », le message ne cache pas ce que libérerait le prochain arrêt.
        let mut ui = sample();
        ui.note = Some("Mode jeu terminé : 1 service relancé.".into());
        assert_eq!(
            status(&ui).1,
            "Mode jeu terminé : 1 service relancé. \
             « Tout arrêter » libérerait ≈ 30 Mo (1 service coché en cours)."
        );
        ui.note = None;
        ui.rows[0].checked = false;
        assert_eq!(status(&ui).1, "Aucun service coché ne tourne en ce moment.");

        let mut ui = sample();
        ui.elevated = false;
        assert_eq!(
            status(&ui),
            (
                "Lecture seule".into(),
                "« Tout arrêter » libérerait ≈ 30 Mo (1 service coché en cours). \
                 Relance GameBooster en administrateur pour arrêter des services."
                    .into()
            )
        );
    }

    #[test]
    fn checking_never_stops_a_service() {
        let mut ui = sample();
        // Hors mode jeu, la case prépare seulement « Tout arrêter ».
        assert_eq!(check_row(&mut ui, 1, true), Some(Vec::new()));
        assert!(ui.rows[1].checked && ui.allowed.is_empty());
        check_row(&mut ui, 1, false);
        // En mode jeu, le minuteur laisse tourner le spouleur, coché alors qu'il tourne…
        let mut session = Session::new(0, String::new());
        session.game_mode = true;
        ui.session = Some(session);
        check_row(&mut ui, 1, true);
        assert!(ui.allowed.contains("Spooler"));
        // … mais arrêtera Windows Update, coché arrêté, si Windows le démarre.
        ui.rows[0].state = State::Stopped;
        check_row(&mut ui, 0, false);
        check_row(&mut ui, 0, true);
        assert!(!ui.allowed.contains("wuauserv"));
    }

    #[test]
    fn one_button_follows_selected_service() {
        let mut ui = sample();
        assert_eq!(one_action(&ui), None);
        ui.selected = Some("wuauserv".into());
        assert_eq!(one_action(&ui), Some(("Arrêter", true)));
        ui.rows[0].state = State::Stopped;
        assert_eq!(one_action(&ui), Some(("Démarrer", true)));
        let mut session = Session::new(0, String::new());
        session.stopped.push("wuauserv".into());
        ui.session = Some(session);
        assert_eq!(one_action(&ui), Some(("Relancer", true)));
        ui.working = Working::Stopping;
        assert_eq!(one_action(&ui), Some(("Relancer", false)));
    }

    #[test]
    fn details_explain_ram_and_refusals() {
        let mut ui = sample();
        let row = ui.rows[0].clone();
        assert_eq!(state_line(&ui, &row).0, "En cours · occupe 30 Mo de RAM.");
        ui.refused = vec![("wuauserv".into(), "accès refusé par Windows".into())];
        assert_eq!(
            state_line(&ui, &row).0,
            "Windows a refusé de l'arrêter : accès refusé par Windows."
        );
    }

    #[test]
    fn watcher_leaves_what_windows_relaunches_at_once() {
        let mut ui = sample();
        let mut session = Session::new(0, String::new());
        session.stopped.push("Spooler".into());
        ui.session = Some(session);
        let running = HashSet::from(["wuauserv".to_string(), "Spooler".to_string()]);
        // Le spouleur, arrêté par GameBooster, tourne de nouveau : le minuteur l'arrête.
        let plan = watch_plan(&ui, &running);
        assert_eq!(plan, Some((vec!["Spooler".to_string()], Vec::new())));
        assert_eq!(
            after_watch(&mut ui, vec!["Spooler".into()], Vec::new()),
            (true, false)
        );
        // Revenu dès le tour suivant : Windows le relance aussitôt. Il est laissé tourner, et la
        // session, sans plus rien à relancer, se termine.
        let plan = watch_plan(&ui, &running);
        assert_eq!(plan, Some((Vec::new(), vec!["Spooler".to_string()])));
        assert_eq!(
            after_watch(&mut ui, Vec::new(), vec!["Spooler".into()]),
            (true, true)
        );
        assert!(ui.session.is_none() && ui.relaunched.contains("Spooler"));

        // Coché, il reste en dehors de « Tout arrêter », de son estimation et du minuteur ; son
        // détail l'explique.
        ui.rows[1].checked = true;
        let mut session = Session::new(0, String::new());
        session.game_mode = true;
        ui.session = Some(session);
        assert_eq!(stop_all_names(&ui), ["wuauserv"]);
        assert!(status(&ui)
            .1
            .ends_with("≈ 30 Mo de plus (1 service coché en cours)."));
        let plan = watch_plan(&ui, &running);
        assert_eq!(plan, Some((vec!["wuauserv".to_string()], Vec::new())));
        let printer = ui.rows[1].clone();
        assert_eq!(
            state_line(&ui, &printer).0,
            "Windows le relance aussitôt arrêté : un autre service s'en sert. \
             Mieux vaut le décocher."
        );
    }
}
