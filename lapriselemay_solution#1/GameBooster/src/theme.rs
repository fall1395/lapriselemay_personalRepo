//! Les couleurs, polices et mesures de GameBooster : un thème sombre, comme l'overlay de
//! GameMonitor.

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DeleteObject, FillRect, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, FF_SWISS, FW_NORMAL, FW_SEMIBOLD, HDC, HFONT, OUT_DEFAULT_PRECIS,
    VARIABLE_PITCH,
};

use crate::catalog::Advice;

// Couleurs, en 0x00BBGGRR.
pub const BG: COLORREF = COLORREF(0x0014_1414);
/// Fond de la liste et du panneau de détail.
pub const PANEL: COLORREF = COLORREF(0x001B_1B1B);
pub const HEADER: COLORREF = COLORREF(0x0025_2525);
pub const LINE: COLORREF = COLORREF(0x0038_3838);
pub const TEXT: COLORREF = COLORREF(0x00E6_E6E6);
pub const DIM: COLORREF = COLORREF(0x009A_9A9A);
pub const WHITE: COLORREF = COLORREF(0x00FF_FFFF);
pub const GREEN: COLORREF = COLORREF(0x0043_A02E);
pub const GREEN_DOWN: COLORREF = COLORREF(0x0033_7D23);
pub const GRAY: COLORREF = COLORREF(0x003C_3C3C);
pub const GRAY_DOWN: COLORREF = COLORREF(0x002C_2C2C);
pub const DISABLED: COLORREF = COLORREF(0x0026_2626);
pub const DISABLED_TEXT: COLORREF = COLORREF(0x005C_5C5C);
/// Service arrêté par GameBooster.
pub const BLUE: COLORREF = COLORREF(0x00FF_A658);
/// Refus de Windows.
pub const AMBER: COLORREF = COLORREF(0x0022_99D2);

pub fn advice_color(advice: Advice) -> COLORREF {
    match advice {
        Advice::Safe => COLORREF(0x0050_B93F),
        Advice::Depends => AMBER,
        Advice::Avoid => COLORREF(0x003E_88F0),
    }
}

/// Les polices, au DPI de l'écran de la fenêtre.
#[derive(Clone, Copy)]
pub struct Fonts {
    pub button: HFONT,
    pub title: HFONT,
    pub text: HFONT,
    pub list: HFONT,
}

impl Fonts {
    pub fn new(dpi: u32) -> Fonts {
        Fonts {
            button: font(px(15, dpi), FW_SEMIBOLD.0 as i32),
            title: font(px(15, dpi), FW_SEMIBOLD.0 as i32),
            text: font(px(13, dpi), FW_NORMAL.0 as i32),
            list: font(px(14, dpi), FW_NORMAL.0 as i32),
        }
    }

    pub fn delete(self) {
        for font in [self.button, self.title, self.text, self.list] {
            unsafe {
                let _ = DeleteObject(font.into());
            }
        }
    }
}

fn font(height: i32, weight: i32) -> HFONT {
    unsafe {
        CreateFontW(
            -height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            (FF_SWISS.0 | VARIABLE_PITCH.0) as u32,
            w!("Segoe UI"),
        )
    }
}

/// `v` pixels à 100 %, au DPI `dpi` (96 = 100 %).
pub fn px(v: i32, dpi: u32) -> i32 {
    (v * dpi as i32 + 48) / 96
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Avec le zéro final, pour les structures de Windows.
pub fn wide_z(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn fill(hdc: HDC, rect: &RECT, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(hdc, rect, brush);
        let _ = DeleteObject(brush.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_with_dpi() {
        assert_eq!(px(16, 96), 16);
        assert_eq!(px(16, 144), 24);
        assert_eq!(px(360, 120), 450);
    }

    #[test]
    fn strings_for_windows() {
        assert_eq!(wide("é"), [0xE9]);
        assert_eq!(wide_z("ab"), [0x61, 0x62, 0]);
    }
}
