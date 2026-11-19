//! Petite enveloppe autour de PDH (compteurs de performance Windows).
//! On utilise les noms anglais des compteurs pour que ça marche sur un Windows en français.

use windows::core::PCWSTR;
use windows::Win32::System::Performance::*;

const PDH_MORE_DATA: u32 = 0x8000_07D2;

fn valid_status(status: u32) -> bool {
    // PDH_CSTATUS_VALID_DATA et PDH_CSTATUS_NEW_DATA sont tous deux valides.
    matches!(status, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA)
}

/// Valeur décimale, sans le plafond à 100 que PDH applique par défaut aux pourcentages :
/// « % Processor Performance » dépasse 100 en turbo (absent du crate windows).
const FORMAT: PDH_FMT = PDH_FMT(PDH_FMT_DOUBLE.0 | 0x0000_8000 /* PDH_FMT_NOCAP100 */);

pub struct Pdh {
    query: PDH_HQUERY,
}

pub struct Counter(PDH_HCOUNTER);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

impl Pdh {
    pub fn new() -> Option<Self> {
        let mut query = PDH_HQUERY::default();
        let status = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) };
        (status == 0).then_some(Self { query })
    }

    pub fn add(&self, path: &str) -> Option<Counter> {
        let path = wide(path);
        let mut counter = PDH_HCOUNTER::default();
        let status =
            unsafe { PdhAddEnglishCounterW(self.query, PCWSTR(path.as_ptr()), 0, &mut counter) };
        (status == 0).then_some(Counter(counter))
    }

    pub fn collect(&self) {
        unsafe {
            PdhCollectQueryData(self.query);
        }
    }
}

impl Drop for Pdh {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.query);
        }
    }
}

impl Counter {
    /// Valeur d'un compteur simple (sans joker).
    pub fn value(&self) -> Option<f64> {
        let mut value = PDH_FMT_COUNTERVALUE::default();
        let status = unsafe { PdhGetFormattedCounterValue(self.0, FORMAT, None, &mut value) };
        if status == 0 && valid_status(value.CStatus) {
            Some(unsafe { value.Anonymous.doubleValue })
        } else {
            None
        }
    }

    /// Valeurs de toutes les instances d'un compteur avec joker (ex. `GPU Engine(*)`).
    pub fn values(&self) -> Vec<(String, f64)> {
        let mut size = 0u32;
        let mut count = 0u32;
        let status =
            unsafe { PdhGetFormattedCounterArrayW(self.0, FORMAT, &mut size, &mut count, None) };
        if status != PDH_MORE_DATA || size == 0 {
            return Vec::new();
        }
        // Tampon aligné sur 8 octets pour les structures PDH.
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        let items = buffer.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
        let status = unsafe {
            PdhGetFormattedCounterArrayW(self.0, FORMAT, &mut size, &mut count, Some(items))
        };
        if status != 0 {
            return Vec::new();
        }
        let items = unsafe { std::slice::from_raw_parts(items, count as usize) };
        items
            .iter()
            .filter(|item| valid_status(item.FmtValue.CStatus))
            .map(|item| {
                let name = unsafe { item.szName.to_string() }.unwrap_or_default();
                (name, unsafe { item.FmtValue.Anonymous.doubleValue })
            })
            .collect()
    }
}
