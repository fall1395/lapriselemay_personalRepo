//! Fenêtre overlay en Win32 pur : transparente, toujours au premier plan, et elle ne prend jamais
//! le focus du jeu. Verrouillée (par défaut), elle laisse passer les clics. Déverrouillée
//! (Ctrl+Maj+F11), elle se pilote à la souris : on la glisse, double-clic pour le mode détaillé,
//! croix pour quitter.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForSystem, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::format::{self, Freshness, Line, build_lines, hotkey_warning};
use crate::metrics::{self, Control, SharedSnapshot};
use crate::placement::{self, Placement};

const CLASS: PCWSTR = w!("GameMonitorOverlay");
/// Ctrl+Maj+F10 : masquer / afficher.
const HK_TOGGLE: i32 = 1;
/// Ctrl+Maj+F11 : déverrouiller l'overlay pour le déplacer, puis le reverrouiller.
const HK_MOVE: i32 = 2;
/// Envoyé par un second lancement de GameMonitor : réafficher l'overlay.
const WM_APP_SHOW: u32 = WM_APP + 1;
/// Mise à jour de l'overlay, une fois par seconde.
const TIMER_ID: usize = 1;
/// Pendant un glissement seulement : suit la souris, même sortie de l'overlay (voir `on_drag_timer`).
const DRAG_TIMER: usize = 2;
const DRAG_TICK_MS: u32 = 15;
const HINT: Duration = Duration::from_secs(10);
const BG: COLORREF = COLORREF(0x0014_1414);
const OPACITY: u8 = 230;
/// Marge intérieure, et espace entre étiquette et valeurs, en pixels à 100 %.
const PAD: f32 = 8.0;
const GAP: f32 = 12.0;

struct State {
    shared: SharedSnapshot,
    control: Control,
    /// Police et échelle au DPI de l'écran où s'affiche l'overlay (96 = 100 %).
    font: HFONT,
    dpi: u32,
    scale: f32,
    detailed: bool,
    visible: bool,
    /// Déverrouillé (Ctrl+Maj+F11) : l'overlay capte la souris pour être déplacé.
    unlocked: bool,
    /// Coin d'ancrage et écarts aux bords, retenus d'un lancement à l'autre.
    placement: Placement,
    /// Glissement en cours, bouton enfoncé sur l'overlay déverrouillé.
    drag: Option<Drag>,
    /// Bouton enfoncé sur la croix : GameMonitor se ferme s'il est relâché dessus.
    closing: bool,
    hint_until: Instant,
    /// Raccourcis que Windows a refusés (déjà pris par une autre application).
    missing_hotkeys: String,
    /// Mesures reçues de seconde en seconde, pour repérer une collecte bloquée.
    freshness: Freshness,
    lines: Vec<Line>,
    label_w: i32,
    line_h: i32,
    size: (i32, i32),
}

/// Glissement de l'overlay à la souris.
#[derive(Clone, Copy)]
struct Drag {
    /// Point saisi, relatif au coin haut gauche de l'overlay.
    grab: POINT,
    /// Curseur au moment de la saisie, en coordonnées d'écran.
    start: POINT,
    /// Écran de l'overlay : il n'en sort pas pendant le glissement.
    screen: RECT,
    /// Faux tant que le curseur n'a pas dépassé le seuil de glissement de Windows : un simple clic,
    /// ou un double-clic, ne déplace pas l'overlay.
    moved: bool,
    /// Dernière position donnée à l'overlay.
    last: Option<(i32, i32)>,
}

impl State {
    /// Adapte la police, la hauteur de ligne et les espacements au DPI de l'écran : le processus est
    /// Per-Monitor V2, chaque écran a donc le sien. La police n'est recréée que si le DPI change.
    fn set_dpi(&mut self, dpi: u32) {
        if dpi == self.dpi {
            return;
        }
        self.dpi = dpi;
        self.scale = dpi as f32 / 96.0;
        self.line_h = (19.0 * self.scale) as i32;
        let old = std::mem::replace(&mut self.font, create_font(self.scale));
        if !old.is_invalid() {
            unsafe {
                let _ = DeleteObject(old.into());
            }
        }
    }

    /// `v` pixels à 100 %, à l'échelle de l'écran de l'overlay.
    fn px(&self, v: f32) -> i32 {
        (v * self.scale) as i32
    }

    /// Vrai si `pt` (relatif à l'overlay) est sur la croix de fermeture de l'overlay déverrouillé.
    fn on_close_box(&self, pt: POINT) -> bool {
        self.unlocked && contains(close_box(self.size, self.line_h, self.px(PAD)), pt)
    }
}

/// Zone de la croix de fermeture, dans le coin haut droit de l'overlay (taille `size`) : un carré de
/// la hauteur d'une ligne, marges comprises, pour être facile à viser.
fn close_box((w, _): (i32, i32), line_h: i32, pad: i32) -> RECT {
    let side = line_h + 2 * pad;
    RECT { left: w - side, top: 0, right: w, bottom: side }
}

fn contains(r: RECT, pt: POINT) -> bool {
    (r.left..r.right).contains(&pt.x) && (r.top..r.bottom).contains(&pt.y)
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Exécute `f` sur l'état de l'overlay. `None` s'il est déjà en cours d'utilisation : certains appels
/// Win32 (SetWindowPos, ShowWindow, SetCapture…) envoient aussitôt un message à la fenêtre, traité
/// pendant l'appel. Ce message est alors ignoré, plutôt que de faire planter GameMonitor.
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn text_width(hdc: HDC, s: &str) -> i32 {
    let mut size = SIZE::default();
    unsafe {
        let _ = GetTextExtentPoint32W(hdc, &wide(s), &mut size);
    }
    size.cx
}

/// Consolas demi-gras, 15 px à 100 %.
fn create_font(scale: f32) -> HFONT {
    unsafe {
        CreateFontW(
            -((15.0 * scale) as i32),
            0,
            0,
            0,
            FW_SEMIBOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            (FF_MODERN.0 | FIXED_PITCH.0) as u32,
            w!("Consolas"),
        )
    }
}

/// Demande à l'overlay déjà lancé de se réafficher (et de rappeler les raccourcis).
pub fn show_existing() {
    unsafe {
        if let Ok(hwnd) = FindWindowW(CLASS, PCWSTR::null()) {
            let _ = PostMessageW(Some(hwnd), WM_APP_SHOW, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn run(shared: SharedSnapshot, control: Control) {
    unsafe {
        let instance: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
        let wc = WNDCLASSW {
            // Double-clic sur l'overlay déverrouillé : mode détaillé.
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let mut state = State {
            shared,
            control,
            font: HFONT::default(),
            dpi: 0,
            scale: 1.0,
            detailed: false,
            visible: true,
            unlocked: false,
            placement: placement::load(),
            drag: None,
            closing: false,
            hint_until: Instant::now() + HINT,
            missing_hotkeys: String::new(),
            freshness: Freshness::default(),
            lines: Vec::new(),
            label_w: 0,
            line_h: 0,
            size: (10, 10),
        };
        // DPI de l'écran principal pour commencer ; refresh() passe ensuite à celui de l'écran du jeu.
        state.set_dpi(GetDpiForSystem());
        STATE.with(|s| *s.borrow_mut() = Some(state));

        // Verrouillé au départ : WS_EX_TRANSPARENT laisse passer les clics jusqu'au jeu.
        let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        let Ok(hwnd) = CreateWindowExW(
            ex,
            CLASS,
            w!("GameMonitor"),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            Some(instance),
            None,
        ) else {
            return;
        };
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), OPACITY, LWA_ALPHA);

        let mods = MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT;
        let hotkeys = [(HK_TOGGLE, VK_F10, "F10"), (HK_MOVE, VK_F11, "F11")];
        // Sans raccourci, l'overlay (qui laisse passer les clics) ne se piloterait plus : on le signale.
        let missing: Vec<&str> = hotkeys
            .iter()
            .filter(|(id, key, _)| RegisterHotKey(Some(hwnd), *id, mods, u32::from(key.0)).is_err())
            .map(|(_, _, name)| *name)
            .collect();
        with_state(|st| st.missing_hotkeys = missing.join(" "));

        refresh(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(Some(hwnd), TIMER_ID, 1000, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        // La police a pu être recréée à chaque changement de DPI : on libère celle en cours.
        if let Some(st) = STATE.with(|s| s.borrow_mut().take()) {
            let _ = DeleteObject(st.font.into());
        }
    }
}

/// Recalcule le texte, redimensionne la fenêtre et la replace à sa place, sur l'écran du jeu et à la
/// taille qui convient au DPI de cet écran.
fn refresh(hwnd: HWND) {
    let layout = with_state(|st| {
        if !st.visible {
            return None;
        }
        let (screen, dpi) = target_screen(hwnd)?;
        st.set_dpi(dpi);

        let snap = st.shared.lock().map(|s| s.clone()).unwrap_or_default();
        let now = Instant::now();
        // Masqué, l'overlay ne passe pas ici : une mesure figée n'est signalée que s'il est visible.
        let frozen = st.freshness.frozen(snap.taken, now);
        // Rappel des raccourcis 10 s au lancement, et en mode détaillé. Déverrouillé, l'overlay dit
        // plutôt ce qui se fait à la souris.
        let hint = !st.unlocked && (now < st.hint_until || st.detailed);
        st.lines = build_lines(&snap, frozen, st.detailed, hint);
        if !st.missing_hotkeys.is_empty() {
            st.lines.push(hotkey_warning(&st.missing_hotkeys));
        }
        if st.unlocked {
            st.lines.insert(0, format::unlocked_header());
            st.lines.push(format::unlocked_footer());
        }

        let gap = st.px(GAP);
        unsafe {
            let hdc = GetDC(Some(hwnd));
            let old = SelectObject(hdc, st.font.into());
            st.label_w = st
                .lines
                .iter()
                .map(|l| text_width(hdc, &l.label))
                .max()
                .unwrap_or(0);
            // Déverrouillé, la première ligne laisse à sa droite la place de la croix de fermeture.
            let close_w = if st.unlocked { gap + st.line_h } else { 0 };
            let content_w = st
                .lines
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    let values: i32 = l.parts.iter().map(|(t, _)| text_width(hdc, t)).sum();
                    let w = if l.label.is_empty() { values } else { st.label_w + gap + values };
                    if i == 0 { w + close_w } else { w }
                })
                .max()
                .unwrap_or(0);
            SelectObject(hdc, old);
            ReleaseDC(Some(hwnd), hdc);

            let pad = st.px(PAD);
            st.size = (content_w + pad * 2, st.lines.len() as i32 * st.line_h + pad * 2);
        }
        // Pendant un glissement, l'overlay reste sous la souris : seule sa taille suit le contenu.
        let pos = st.drag.is_none().then(|| st.placement.position(screen, st.size, st.scale));
        Some((pos, st.size))
    })
    .flatten();
    // SetWindowPos hors de l'emprunt de l'état : en passant sur un écran d'un autre DPI,
    // Windows envoie WM_DPICHANGED à la fenêtre pendant l'appel.
    let Some((pos, (w, h))) = layout else { return };
    let ((x, y), keep) = match pos {
        Some(p) => (p, SET_WINDOW_POS_FLAGS(0)),
        None => ((0, 0), SWP_NOMOVE),
    };
    unsafe {
        // Réaffirme « toujours au premier plan » à chaque seconde : certains jeux le retirent.
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW | keep);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// Rectangle (coordonnées du bureau virtuel) et DPI de l'écran où afficher l'overlay : celui de la
/// fenêtre au premier plan, donc du jeu. Quand c'est le bureau ou la barre des tâches (ou aucune
/// fenêtre, le temps d'un changement de focus), l'overlay reste sur son écran actuel.
fn target_screen(overlay: HWND) -> Option<(RECT, u32)> {
    let fg = unsafe { GetForegroundWindow() };
    let follow = !fg.is_invalid() && !is_shell_window(fg);
    let screen = if follow { screen_of(fg) } else { None };
    // Au lancement, l'écran actuel est l'écran principal, où la fenêtre est créée.
    screen.or_else(|| screen_of(overlay))
}

/// Rectangle et DPI de l'écran qui contient (la plus grande partie de) cette fenêtre.
fn screen_of(window: HWND) -> Option<(RECT, u32)> {
    unsafe {
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let (mut dpi, mut dpi_y) = (0, 0);
        // Échoue si l'écran vient d'être débranché.
        if GetMonitorInfoW(monitor, &mut info).as_bool()
            && GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y).is_ok()
        {
            Some((info.rcMonitor, dpi))
        } else {
            None
        }
    }
}

fn is_shell_window(hwnd: HWND) -> bool {
    let mut class = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut class) };
    is_shell_class(&String::from_utf16_lossy(&class[..usize::try_from(len).unwrap_or(0)]))
}

/// Bureau et barres des tâches (écran principal et autres écrans) : cliquer dessus ne doit pas
/// faire changer l'overlay d'écran.
fn is_shell_class(class: &str) -> bool {
    matches!(class, "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

fn paint(hwnd: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        // BeginPaint et EndPaint même sans état à dessiner : sans eux, Windows renverrait WM_PAINT
        // sans fin.
        STATE.with(|cell| {
            if let Ok(guard) = cell.try_borrow()
                && let Some(st) = guard.as_ref()
            {
                draw(hdc, st);
            }
        });
        let _ = EndPaint(hwnd, &ps);
    }
}

/// Dessine l'overlay, en double tampon pour éviter le scintillement.
fn draw(hdc: HDC, st: &State) {
    let (w, h) = st.size;
    let (pad, gap) = (st.px(PAD), st.px(GAP));
    unsafe {
        let mem = CreateCompatibleDC(Some(hdc));
        let bmp = CreateCompatibleBitmap(hdc, w, h);
        let old_bmp = SelectObject(mem, bmp.into());
        let old_font = SelectObject(mem, st.font.into());

        let brush = CreateSolidBrush(BG);
        FillRect(mem, &RECT { left: 0, top: 0, right: w, bottom: h }, brush);
        let _ = DeleteObject(brush.into());
        SetBkMode(mem, TRANSPARENT);

        for (i, line) in st.lines.iter().enumerate() {
            let y = pad + i as i32 * st.line_h;
            let mut x = pad;
            if !line.label.is_empty() {
                SetTextColor(mem, format::LABEL);
                let _ = TextOutW(mem, x, y, &wide(&line.label));
                x += st.label_w + gap;
            }
            for (text, color) in &line.parts {
                SetTextColor(mem, *color);
                let _ = TextOutW(mem, x, y, &wide(text));
                x += text_width(mem, text);
            }
        }
        if st.unlocked {
            draw_unlocked(mem, st);
        }

        let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
        SelectObject(mem, old_font);
        SelectObject(mem, old_bmp);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
    }
}

/// Overlay déverrouillé : un cadre montre qu'il capte la souris, et une croix, en haut à droite,
/// ferme GameMonitor.
fn draw_unlocked(dc: HDC, st: &State) {
    let (w, h) = st.size;
    let thick = st.px(2.0).max(1);
    // Croix dans un carré de la hauteur d'une ligne, à droite de la première ligne.
    let side = st.line_h;
    let (left, top) = (w - st.px(PAD) - side, st.px(PAD));
    let inset = side / 4;
    unsafe {
        let frame = CreateSolidBrush(format::ACCENT);
        for i in 0..thick {
            FrameRect(dc, &RECT { left: i, top: i, right: w - i, bottom: h - i }, frame);
        }
        let _ = DeleteObject(frame.into());

        let pen = CreatePen(PS_SOLID, thick, format::BAD);
        let old = SelectObject(dc, pen.into());
        let _ = MoveToEx(dc, left + inset, top + inset, None);
        let _ = LineTo(dc, left + side - inset, top + side - inset);
        let _ = MoveToEx(dc, left + side - inset, top + inset, None);
        let _ = LineTo(dc, left + inset, top + side - inset);
        SelectObject(dc, old);
        let _ = DeleteObject(pen.into());
    }
}

/// À chaque tic du minuteur, overlay masqué compris : note si la collecte a publié une nouvelle mesure.
/// Réaffiché pendant que la collecte est bloquée, l'overlay le signale ainsi tout de suite.
fn tick() {
    with_state(|st| {
        let taken = st.shared.lock().ok().and_then(|s| s.taken);
        st.freshness.tick(taken);
    });
}

/// Affiche ou masque l'overlay, et le verrouille ou le déverrouille. Masqué, il est toujours verrouillé.
fn set_mode(hwnd: HWND, visible: bool, unlocked: bool) {
    let unlocked = visible && unlocked;
    if !unlocked {
        // Verrouillé en plein glissement : l'overlay garde la place où il est.
        end_press(hwnd);
    }
    let Some(shown_or_hidden) = with_state(|st| {
        st.unlocked = unlocked;
        if st.visible == visible {
            return false;
        }
        st.visible = visible;
        // Masqué : la collecte se met en pause (session ETW comprise) pour ne rien consommer.
        if visible {
            st.control.resume();
        } else {
            st.control.pause();
        }
        true
    }) else {
        return;
    };
    // Hors de l'emprunt de l'état : Windows envoie aussitôt des messages à la fenêtre.
    unsafe {
        // Verrouillé, WS_EX_TRANSPARENT laisse passer les clics jusqu'au jeu.
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let transparent = WS_EX_TRANSPARENT.0 as isize;
        let wanted = if unlocked { ex & !transparent } else { ex | transparent };
        if wanted != ex {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, wanted);
        }
        if shown_or_hidden {
            let _ = ShowWindow(hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
    }
    refresh(hwnd);
}

fn on_hotkey(hwnd: HWND, id: i32) {
    let Some((visible, unlocked)) = with_state(|st| (st.visible, st.unlocked)) else { return };
    match id {
        // Réaffiché, l'overlay est verrouillé.
        HK_TOGGLE => set_mode(hwnd, !visible, false),
        // Masqué, l'overlay réapparaît déverrouillé, prêt à être déplacé.
        HK_MOVE => set_mode(hwnd, true, !(visible && unlocked)),
        _ => {}
    }
}

/// Point (relatif à l'overlay) d'un message de souris : négatif quand la souris, capturée pendant un
/// glissement, sort de l'overlay par la gauche ou par le haut.
fn mouse_point(lparam: LPARAM) -> POINT {
    POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 }
}

/// Point de l'overlay en coordonnées d'écran.
fn to_screen(hwnd: HWND, mut pt: POINT) -> POINT {
    unsafe {
        let _ = ClientToScreen(hwnd, &mut pt);
    }
    pt
}

/// Déverrouillé : curseur de déplacement sur l'overlay, main sur la croix. Faux si verrouillé.
fn set_cursor(hwnd: HWND) -> bool {
    let cursor = with_state(|st| {
        if !st.unlocked {
            return None;
        }
        let mut pt = POINT::default();
        unsafe {
            GetCursorPos(&mut pt).ok()?;
            let _ = ScreenToClient(hwnd, &mut pt);
        }
        Some(if st.on_close_box(pt) { IDC_HAND } else { IDC_SIZEALL })
    })
    .flatten();
    let Some(id) = cursor else { return false };
    unsafe {
        if let Ok(c) = LoadCursorW(None, id) {
            SetCursor(Some(c));
        }
    }
    true
}

/// Bouton enfoncé sur l'overlay déverrouillé : sur la croix, ou début d'un glissement.
fn on_button_down(hwnd: HWND, pt: POINT) {
    let pressed = with_state(|st| {
        if !st.unlocked {
            return None;
        }
        st.closing = st.on_close_box(pt);
        if !st.closing {
            let (screen, _) = screen_of(hwnd)?;
            let start = to_screen(hwnd, pt);
            st.drag = Some(Drag { grab: pt, start, screen, moved: false, last: None });
        }
        Some(st.closing)
    })
    .flatten();
    let Some(closing) = pressed else { return };
    // Hors de l'emprunt de l'état : SetCapture peut envoyer WM_CAPTURECHANGED.
    unsafe {
        SetCapture(hwnd);
        if !closing {
            // L'overlay n'est jamais au premier plan : une fois le curseur sorti de l'overlay, Windows
            // peut ne plus lui envoyer la souris. Le minuteur suit alors le curseur et le relâchement.
            SetTimer(Some(hwnd), DRAG_TIMER, DRAG_TICK_MS, None);
        }
    }
}

fn on_mouse_move(hwnd: HWND, pt: POINT) {
    drag_to(hwnd, to_screen(hwnd, pt));
}

/// Glissement : l'overlay suit le curseur (`cursor`, en coordonnées d'écran) sans sortir de son écran,
/// et s'aimante à ses bords.
fn drag_to(hwnd: HWND, cursor: POINT) {
    let target = with_state(|st| {
        let drag = st.drag.as_mut()?;
        if !drag.moved {
            // Seuil de glissement de Windows : un clic, ou un double-clic, ne déplace pas l'overlay.
            let (tx, ty) = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
            if (cursor.x - drag.start.x).abs() <= tx && (cursor.y - drag.start.y).abs() <= ty {
                return None;
            }
            drag.moved = true;
        }
        let wanted = (cursor.x - drag.grab.x, cursor.y - drag.grab.y);
        let margin = (placement::MARGIN * st.scale).round() as i32;
        let reach = (placement::SNAP * st.scale).round() as i32;
        let pos = placement::dragged(drag.screen, st.size, wanted, margin, reach);
        if drag.last == Some(pos) {
            return None;
        }
        drag.last = Some(pos);
        Some(pos)
    })
    .flatten();
    if let Some((x, y)) = target {
        unsafe {
            let _ = SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
}

/// Pendant un glissement, toutes les 15 ms : suit le curseur, et pose l'overlay une fois le bouton
/// relâché, même si Windows n'a pas envoyé la souris à l'overlay (curseur sorti de l'overlay).
fn on_drag_timer(hwnd: HWND) {
    // Bouton principal de la souris : celui de droite pour une souris de gaucher.
    let button = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 { VK_RBUTTON } else { VK_LBUTTON };
    let held = unsafe { GetAsyncKeyState(i32::from(button.0)) } < 0;
    let mut cursor = POINT::default();
    if !held {
        end_press(hwnd);
    } else if unsafe { GetCursorPos(&mut cursor) }.is_ok() {
        drag_to(hwnd, cursor);
    }
}

/// Bouton relâché sur l'overlay : fin du glissement, ou fermeture si le clic a commencé et fini sur
/// la croix.
fn on_button_up(hwnd: HWND, pt: POINT) {
    let close = with_state(|st| st.closing && st.on_close_box(pt)) == Some(true);
    end_press(hwnd);
    if close {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

/// Fin du bouton enfoncé sur l'overlay. Après un glissement, l'overlay garde la place où il a été
/// posé, pour ce lancement et les suivants.
fn end_press(hwnd: HWND) {
    let dropped = with_state(|st| {
        st.closing = false;
        st.drag.take()
    })
    .flatten();
    // Hors de l'emprunt de l'état : ReleaseCapture envoie WM_CAPTURECHANGED.
    unsafe {
        let _ = KillTimer(Some(hwnd), DRAG_TIMER);
        if GetCapture() == hwnd {
            let _ = ReleaseCapture();
        }
    }
    let Some(drag) = dropped.filter(|d| d.moved) else { return };
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return;
    }
    let placed = with_state(|st| {
        st.placement = Placement::from_position(drag.screen, st.size, (rect.left, rect.top), st.scale);
        st.placement
    });
    if let Some(p) = placed {
        placement::save(p);
    }
    refresh(hwnd);
}

/// Double-clic sur l'overlay déverrouillé : mode détaillé, ou retour au mode compact.
fn on_double_click(hwnd: HWND, pt: POINT) {
    let toggled = with_state(|st| {
        let toggle = st.unlocked && !st.on_close_box(pt);
        if toggle {
            st.detailed = !st.detailed;
        }
        toggle
    });
    if toggled == Some(true) {
        refresh(hwnd);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            if wparam.0 == DRAG_TIMER {
                on_drag_timer(hwnd);
            } else {
                tick();
                refresh(hwnd);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_DPICHANGED => {
            // refresh() calcule déjà la taille au DPI de l'écran visé avant d'y déplacer l'overlay,
            // et suit dans la seconde un changement d'échelle : le rectangle proposé ne sert pas.
            LRESULT(0)
        }
        WM_HOTKEY => {
            on_hotkey(hwnd, wparam.0 as i32);
            LRESULT(0)
        }
        // Déverrouillé, cliquer sur l'overlay ne prend pas non plus le focus du jeu.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // Déverrouillé : curseur de déplacement, ou main sur la croix. Sinon, celui par défaut.
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT && set_cursor(hwnd) => LRESULT(1),
        WM_LBUTTONDOWN => {
            on_button_down(hwnd, mouse_point(lparam));
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            on_mouse_move(hwnd, mouse_point(lparam));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            on_button_up(hwnd, mouse_point(lparam));
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => {
            on_double_click(hwnd, mouse_point(lparam));
            LRESULT(0)
        }
        WM_APP_SHOW => {
            // Second lancement de GameMonitor : réaffiche l'overlay et rappelle les raccourcis.
            let unlocked = with_state(|st| {
                st.hint_until = Instant::now() + HINT;
                st.unlocked
            });
            set_mode(hwnd, true, unlocked == Some(true));
            LRESULT(0)
        }
        WM_ENDSESSION => {
            // Fermeture de session ou arrêt de Windows : le processus sera tué juste après,
            // sans repasser par main. On ferme la session ETW tout de suite.
            if wparam.0 != 0 {
                metrics::emergency_stop();
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // main arrête ensuite proprement la collecte (et la session ETW).
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_and_taskbars_keep_the_screen() {
        for class in ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"] {
            assert!(is_shell_class(class), "{class}");
        }
        for class in ["UnrealWindow", "UnityWndClass", "SDL_app", ""] {
            assert!(!is_shell_class(class), "{class}");
        }
    }

    #[test]
    fn close_box_in_the_top_right_corner() {
        // Overlay de 400 × 200 à 100 % : un carré de 35 px (ligne de 19 px, marges de 8 px).
        let r = close_box((400, 200), 19, 8);
        assert_eq!(r, RECT { left: 365, top: 0, right: 400, bottom: 35 });
        assert!(contains(r, POINT { x: 394, y: 6 }));
        assert!(!contains(r, POINT { x: 20, y: 20 }));
        assert!(!contains(r, POINT { x: 394, y: 35 }));
    }

    #[test]
    fn mouse_outside_the_overlay() {
        // Souris capturée, sortie de l'overlay par la gauche : x négatif, sur 16 bits.
        let pt = mouse_point(LPARAM((300 << 16) | 0xFFFB));
        assert_eq!(pt, POINT { x: -5, y: 300 });
    }
}
