//! La liste des services : le contrôle ListView de Windows, en mode rapport avec cases à cocher,
//! en thème sombre. Windows dessinerait l'en-tête en clair : Windows Update Stopper le dessine lui-même.

use std::cell::Cell;
use std::ffi::c_void;
use std::mem::size_of;

use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    DrawTextW, InvalidateRect, SetBkMode, SetTextColor, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX,
    DT_RIGHT, DT_SINGLELINE, DT_VCENTER, TRANSPARENT,
};
use windows::Win32::UI::Controls::{
    InitCommonControlsEx, SetWindowTheme, CDDS_ITEMPREPAINT, CDDS_POSTPAINT, CDDS_PREPAINT,
    CDRF_DODEFAULT, CDRF_NOTIFYITEMDRAW, CDRF_NOTIFYPOSTPAINT, CDRF_SKIPDEFAULT,
    ICC_LISTVIEW_CLASSES, INITCOMMONCONTROLSEX, LIST_VIEW_ITEM_STATE_FLAGS, LVCFMT_LEFT,
    LVCFMT_RIGHT, LVCF_FMT, LVCF_SUBITEM, LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_STATE, LVIF_TEXT,
    LVIS_FOCUSED, LVIS_SELECTED, LVIS_STATEIMAGEMASK, LVITEMW, LVM_DELETEALLITEMS,
    LVM_ENSUREVISIBLE, LVM_GETHEADER, LVM_GETNEXTITEM, LVM_INSERTCOLUMNW, LVM_INSERTITEMW,
    LVM_SETBKCOLOR, LVM_SETCOLUMNWIDTH, LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMSTATE,
    LVM_SETITEMTEXTW, LVM_SETTEXTBKCOLOR, LVM_SETTEXTCOLOR, LVNI_SELECTED, LVS_EX_CHECKBOXES,
    LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT, LVS_REPORT, LVS_SHOWSELALWAYS, LVS_SINGLESEL,
    NMCUSTOMDRAW, NMHDR, NMLISTVIEW, NM_CUSTOMDRAW, WC_LISTVIEWW,
};
use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetClientRect, SendMessageW, HMENU, SM_CXVSCROLL, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_NOTIFY, WS_CHILD, WS_TABSTOP, WS_VISIBLE,
};

use crate::theme::{fill, px, wide, wide_z, DIM, HEADER, LINE, PANEL, TEXT};

/// Les colonnes : titre, largeur à 100 % (0 : la place qui reste), alignée à droite.
const COLUMNS: [(&str, i32, bool); 3] = [
    ("Service", 0, false),
    ("État", 96, false),
    ("RAM", 86, true),
];

const HDM_GETITEMCOUNT: u32 = 0x1200;
const HDM_GETITEMRECT: u32 = 0x1207;
/// Pas de fond derrière le texte : celui de la ligne, ou de la sélection, reste visible.
const CLR_NONE: u32 = 0xFFFF_FFFF;

/// Ce qu'affiche une ligne.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// Nom du service : la ligne en garde la trace d'une mise à jour à l'autre.
    pub name: String,
    pub cells: [String; 3],
    pub checked: bool,
}

thread_local! {
    /// Colonne triée et sens inverse, pour la flèche de l'en-tête.
    static SORT: Cell<(usize, bool)> = const { Cell::new((0, false)) };
}

fn send(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))).0 }
}

fn header(list: HWND) -> HWND {
    HWND(send(list, LVM_GETHEADER, 0, 0) as *mut c_void)
}

/// Crée la liste, vide, avec ses colonnes.
pub fn create(parent: HWND, instance: HINSTANCE, id: usize) -> HWND {
    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES,
        };
        let _ = InitCommonControlsEx(&icc);
        let style = WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WINDOW_STYLE(LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS);
        let Ok(list) = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            WC_LISTVIEWW,
            PCWSTR::null(),
            style,
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(id as *mut c_void)),
            Some(instance),
            None,
        ) else {
            return HWND::default();
        };
        let extended = LVS_EX_CHECKBOXES | LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER;
        send(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            extended as usize,
            extended as isize,
        );
        // Barres de défilement, sélection et cases à cocher sombres : le thème de l'Explorateur
        // de Windows en mode sombre. Sans lui (Windows trop ancien), elles restent claires.
        let _ = SetWindowTheme(list, w!("DarkMode_Explorer"), PCWSTR::null());
        let _ = SetWindowTheme(header(list), w!("DarkMode_ItemsView"), PCWSTR::null());
        send(list, LVM_SETBKCOLOR, 0, PANEL.0 as isize);
        send(list, LVM_SETTEXTBKCOLOR, 0, CLR_NONE as isize);
        send(list, LVM_SETTEXTCOLOR, 0, TEXT.0 as isize);
        for (i, (title, _, right)) in COLUMNS.iter().enumerate() {
            let mut text = wide_z(title);
            let column = LVCOLUMNW {
                mask: LVCF_TEXT | LVCF_WIDTH | LVCF_FMT | LVCF_SUBITEM,
                fmt: if *right { LVCFMT_RIGHT } else { LVCFMT_LEFT },
                cx: 100,
                pszText: PWSTR(text.as_mut_ptr()),
                iSubItem: i as i32,
                ..Default::default()
            };
            send(
                list,
                LVM_INSERTCOLUMNW,
                i,
                &column as *const LVCOLUMNW as isize,
            );
        }
        let _ = SetWindowSubclass(list, Some(list_proc), 1, 0);
        list
    }
}

/// Largeurs des colonnes pour une liste de `width` pixels : « Service » prend la place qui reste.
pub fn fit_columns(list: HWND, width: i32, dpi: u32) {
    let scrollbar = unsafe { GetSystemMetricsForDpi(SM_CXVSCROLL, dpi) };
    let fixed: i32 = COLUMNS.iter().map(|(_, w, _)| px(*w, dpi)).sum();
    let first = (width - fixed - scrollbar - px(4, dpi)).max(px(180, dpi));
    for (i, (_, w, _)) in COLUMNS.iter().enumerate() {
        let w = if i == 0 { first } else { px(*w, dpi) };
        send(list, LVM_SETCOLUMNWIDTH, i, w as isize);
    }
}

/// Met la liste à jour. Mêmes services dans le même ordre : seulement les cellules et les cases
/// qui ont changé, sans scintillement. Sinon, tout est refait : renvoie vrai.
pub fn sync(list: HWND, old: &[Line], new: &[Line]) -> bool {
    let same = old.len() == new.len() && old.iter().zip(new).all(|(a, b)| a.name == b.name);
    if !same {
        send(list, LVM_DELETEALLITEMS, 0, 0);
        for (i, line) in new.iter().enumerate() {
            insert(list, i, line);
        }
        return true;
    }
    for (i, (a, b)) in old.iter().zip(new).enumerate() {
        for column in 0..COLUMNS.len() {
            if a.cells[column] != b.cells[column] {
                set_text(list, i, column, &b.cells[column]);
            }
        }
        if a.checked != b.checked {
            set_checked(list, i, b.checked);
        }
    }
    false
}

fn insert(list: HWND, index: usize, line: &Line) {
    let mut text = wide_z(&line.cells[0]);
    let item = LVITEMW {
        mask: LVIF_TEXT,
        iItem: index as i32,
        pszText: PWSTR(text.as_mut_ptr()),
        ..Default::default()
    };
    send(list, LVM_INSERTITEMW, 0, &item as *const LVITEMW as isize);
    for column in 1..COLUMNS.len() {
        set_text(list, index, column, &line.cells[column]);
    }
    set_checked(list, index, line.checked);
}

fn set_text(list: HWND, index: usize, column: usize, text: &str) {
    let mut text = wide_z(text);
    let item = LVITEMW {
        iSubItem: column as i32,
        pszText: PWSTR(text.as_mut_ptr()),
        ..Default::default()
    };
    send(
        list,
        LVM_SETITEMTEXTW,
        index,
        &item as *const LVITEMW as isize,
    );
}

pub fn set_checked(list: HWND, index: usize, checked: bool) {
    // Image d'état 2 : case cochée ; 1 : décochée.
    let image: u32 = if checked { 2 } else { 1 };
    let item = LVITEMW {
        stateMask: LVIS_STATEIMAGEMASK,
        state: LIST_VIEW_ITEM_STATE_FLAGS(image << 12),
        ..Default::default()
    };
    send(
        list,
        LVM_SETITEMSTATE,
        index,
        &item as *const LVITEMW as isize,
    );
}

/// Sélectionne une ligne et la fait défiler jusqu'à elle.
pub fn select(list: HWND, index: usize) {
    let both = LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0);
    let item = LVITEMW {
        stateMask: both,
        state: both,
        ..Default::default()
    };
    send(
        list,
        LVM_SETITEMSTATE,
        index,
        &item as *const LVITEMW as isize,
    );
    send(list, LVM_ENSUREVISIBLE, index, 0);
}

pub fn selected(list: HWND) -> Option<usize> {
    // usize::MAX : -1, chercher depuis le début.
    let index = send(list, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as isize);
    usize::try_from(index).ok()
}

/// Case cochée (`Some(true)`) ou décochée par l'utilisateur, d'après LVN_ITEMCHANGED.
pub fn check_change(change: &NMLISTVIEW) -> Option<bool> {
    let mask = LVIS_STATEIMAGEMASK.0;
    if change.uChanged.0 & LVIF_STATE.0 == 0 || (change.uNewState ^ change.uOldState) & mask == 0 {
        return None;
    }
    // Pas d'image avant : la ligne vient d'être ajoutée, ce n'est pas un clic.
    if change.uOldState & mask == 0 {
        return None;
    }
    Some((change.uNewState & mask) >> 12 == 2)
}

/// Vrai si LVN_ITEMCHANGED signale un changement de sélection.
pub fn selection_changed(change: &NMLISTVIEW) -> bool {
    change.uChanged.0 & LVIF_STATE.0 != 0
        && (change.uNewState ^ change.uOldState) & LVIS_SELECTED.0 != 0
}

/// Colonne triée, et sens inverse : la flèche de l'en-tête suit.
pub fn set_sort(list: HWND, column: usize, reverse: bool) {
    SORT.with(|sort| sort.set((column, reverse)));
    unsafe {
        let _ = InvalidateRect(Some(header(list)), None, true);
    }
}

/// La liste reçoit les notifications de son en-tête : celle qui demande à le dessiner est
/// interceptée ici.
unsafe extern "system" fn list_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_NOTIFY {
        let hdr = unsafe { &*(lparam.0 as *const NMHDR) };
        if hdr.code == NM_CUSTOMDRAW && hdr.hwndFrom == header(hwnd) {
            let draw = unsafe { &*(lparam.0 as *const NMCUSTOMDRAW) };
            return LRESULT(draw_header(hdr.hwndFrom, draw) as isize);
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

fn client_rect(hwnd: HWND) -> RECT {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    rect
}

fn draw_header(header: HWND, draw: &NMCUSTOMDRAW) -> u32 {
    let stage = draw.dwDrawStage;
    if stage == CDDS_PREPAINT {
        fill(draw.hdc, &client_rect(header), HEADER);
        return CDRF_NOTIFYITEMDRAW | CDRF_NOTIFYPOSTPAINT;
    }
    if stage == CDDS_ITEMPREPAINT {
        draw_header_item(draw);
        return CDRF_SKIPDEFAULT;
    }
    if stage == CDDS_POSTPAINT {
        // À droite de la dernière colonne, Windows a pu repeindre le fond en clair.
        let count = send(header, HDM_GETITEMCOUNT, 0, 0);
        if count > 0 {
            let mut last = RECT::default();
            send(
                header,
                HDM_GETITEMRECT,
                (count - 1) as usize,
                &mut last as *mut RECT as isize,
            );
            let mut rest = client_rect(header);
            rest.left = last.right;
            if rest.left < rest.right {
                fill(draw.hdc, &rest, HEADER);
            }
        }
    }
    CDRF_DODEFAULT
}

fn draw_header_item(draw: &NMCUSTOMDRAW) {
    let (hdc, rect) = (draw.hdc, draw.rc);
    fill(hdc, &rect, HEADER);
    let separator = RECT {
        left: rect.right - 1,
        top: rect.top + 6,
        right: rect.right,
        bottom: rect.bottom - 6,
    };
    fill(hdc, &separator, LINE);
    let column = draw.dwItemSpec;
    let Some((title, _, right)) = COLUMNS.get(column) else {
        return;
    };
    let (sorted, reverse) = SORT.with(Cell::get);
    // État et RAM se trient d'abord du plus grand au plus petit (en cours, puis arrêtés).
    let descending = (column >= 1) != reverse;
    let arrow = match (sorted == column, descending) {
        (false, _) => "",
        (true, false) => "  ▲",
        (true, true) => "  ▼",
    };
    let mut text_rect = RECT {
        left: rect.left + 8,
        right: rect.right - 8,
        ..rect
    };
    let align = if *right { DT_RIGHT } else { DT_LEFT };
    unsafe {
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, DIM);
        DrawTextW(
            hdc,
            &mut wide(&format!("{title}{arrow}")),
            &mut text_rect,
            align | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }
}
