//! GPU : NVML (fourni avec le pilote NVIDIA) si disponible,
//! sinon compteurs Windows génériques (marche aussi pour Intel et AMD) et nom donné par DXGI.

use std::collections::HashMap;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use nvml_wrapper::Nvml;
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1};
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
use windows::core::w;

use super::pdh::{Counter, Pdh};

#[derive(Clone, Default)]
pub struct GpuInfo {
    pub name: String,
    pub usage: Option<f32>,
    pub vram_used: Option<u64>,
    pub vram_total: Option<u64>,
    pub temp: Option<u32>,
    pub power_w: Option<f32>,
    pub clock_mhz: Option<u32>,
    pub mem_clock_mhz: Option<u32>,
    pub fan: Option<u32>,
}

pub struct Gpu {
    nvml: Option<Nvml>,
    name: String,
    engine: Option<Counter>,
    dedicated: Option<Counter>,
}

/// Nom de la carte graphique de l'écran principal, d'après DXGI. Un appel local au pilote,
/// qui revient tout de suite : contrairement à une requête WMI, il ne peut pas bloquer la collecte.
fn adapter_name() -> Option<String> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.ok()?;
    (0..)
        .map_while(|i| unsafe { factory.EnumAdapters1(i) }.ok())
        .filter_map(|adapter| unsafe { adapter.GetDesc1() }.ok())
        // Le rendu logiciel de Windows (« Microsoft Basic Render Driver ») n'est pas une carte.
        .find(|desc| desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 == 0)
        .map(|desc| {
            let len = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
            String::from_utf16_lossy(&desc.Description[..len])
        })
}

/// NVML, installée par le pilote NVIDIA dans System32, et chargée de là seulement, ses dépendances aussi.
///
/// Par son seul nom (ce que fait `Nvml::init`), Windows chercherait d'abord `nvml.dll` dans le dossier
/// de l'exe : une DLL déposée à côté de GameMonitor (dans Téléchargements, par exemple) s'y exécuterait
/// avec ses droits administrateur. Seul un administrateur peut écrire dans System32.
fn load_nvml() -> Option<Nvml> {
    let module = unsafe { LoadLibraryExW(w!("nvml.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.ok()?;
    // nvml-wrapper la charge à son tour, par son chemin complet : Windows lui rend celle qui est
    // déjà chargée, sans rien chercher d'autre. Elle reste chargée tant qu'il s'en sert.
    let nvml = module_path(module).and_then(|path| Nvml::builder().lib_path(path.as_os_str()).init().ok());
    unsafe {
        let _ = FreeLibrary(module);
    }
    nvml
}

/// Chemin complet d'un module chargé dans le processus.
fn module_path(module: HMODULE) -> Option<PathBuf> {
    let mut buffer = [0u16; 1024];
    let len = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
    (len > 0 && len < buffer.len()).then(|| PathBuf::from(OsString::from_wide(&buffer[..len])))
}

impl Gpu {
    pub fn new(pdh: Option<&Pdh>) -> Self {
        let nvml = load_nvml().filter(|n| n.device_count().map(|c| c > 0).unwrap_or(false));
        let name = nvml
            .as_ref()
            .and_then(|n| n.device_by_index(0).ok()?.name().ok())
            .or_else(adapter_name)
            .unwrap_or_else(|| "GPU".to_string());
        // Les compteurs génériques ne servent que sans NVIDIA.
        let pdh = pdh.filter(|_| nvml.is_none());
        let engine = pdh.and_then(|p| p.add(r"\GPU Engine(*engtype_3D)\Utilization Percentage"));
        let dedicated = pdh.and_then(|p| p.add(r"\GPU Adapter Memory(*)\Dedicated Usage"));
        Self { nvml, name, engine, dedicated }
    }

    pub fn read(&self) -> GpuInfo {
        let mut info = GpuInfo { name: self.name.clone(), ..Default::default() };
        if let Some(nvml) = &self.nvml {
            if let Ok(dev) = nvml.device_by_index(0) {
                info.usage = dev.utilization_rates().ok().map(|u| u.gpu as f32);
                if let Ok(mem) = dev.memory_info() {
                    info.vram_used = Some(mem.total.saturating_sub(mem.free));
                    info.vram_total = Some(mem.total);
                }
                info.temp = dev.temperature(TemperatureSensor::Gpu).ok();
                info.power_w = dev.power_usage().ok().map(|mw| mw as f32 / 1000.0);
                info.clock_mhz = dev.clock_info(Clock::Graphics).ok();
                info.mem_clock_mhz = dev.clock_info(Clock::Memory).ok();
                info.fan = dev.fan_speed(0).ok();
            }
            return info;
        }
        if let Some(engine) = &self.engine {
            info.usage = Some(busiest_engine(&engine.values()) as f32);
        }
        if let Some(dedicated) = &self.dedicated {
            let max = dedicated.values().iter().map(|(_, v)| *v).fold(0.0, f64::max);
            if max > 0.0 {
                info.vram_used = Some(max as u64);
            }
        }
        info
    }
}

/// Charge GPU comme le Gestionnaire des tâches : chaque moteur additionne ses processus
/// (`pid_1234_luid_0x…_phys_0_eng_0_engtype_3D`), et la charge est celle du moteur le plus occupé.
/// Additionner toutes les instances compterait deux fois un PC à deux GPU (iGPU + carte graphique).
fn busiest_engine(instances: &[(String, f64)]) -> f64 {
    let mut engines: HashMap<&str, f64> = HashMap::new();
    for (name, value) in instances {
        let engine = name.find("luid_").map_or(name.as_str(), |i| &name[i..]);
        *engines.entry(engine).or_default() += value;
    }
    engines.into_values().fold(0.0, f64::max).min(100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(pid: u32, engine: &str, value: f64) -> (String, f64) {
        (format!("pid_{pid}_luid_0x00000000_{engine}_engtype_3D"), value)
    }

    #[test]
    fn processes_add_up_on_the_same_engine() {
        let values = [instance(10, "0x0000C5E2_phys_0_eng_0", 30.0), instance(20, "0x0000C5E2_phys_0_eng_0", 25.0)];
        assert_eq!(busiest_engine(&values), 55.0);
    }

    #[test]
    fn two_gpus_are_not_added_together() {
        let values = [instance(10, "0x0000C5E2_phys_0_eng_0", 40.0), instance(20, "0x0000D1A0_phys_0_eng_0", 70.0)];
        assert_eq!(busiest_engine(&values), 70.0);
    }

    #[test]
    fn capped_at_100_and_zero_without_data() {
        let values = [instance(10, "0x1_phys_0_eng_0", 80.0), instance(20, "0x1_phys_0_eng_0", 40.0)];
        assert_eq!(busiest_engine(&values), 100.0);
        assert_eq!(busiest_engine(&[]), 0.0);
    }

    #[test]
    fn nvml_comes_from_system32_only() {
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        // Sans pilote NVIDIA (PC à carte Intel ou AMD), rien n'est chargé ; avec, c'est celle de System32.
        let _nvml = load_nvml();
        if let Ok(module) = unsafe { GetModuleHandleW(w!("nvml.dll")) } {
            let path = module_path(module).unwrap();
            let system32 = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
            let folder = path.parent().unwrap().as_os_str();
            assert!(folder.eq_ignore_ascii_case(system32.as_os_str()), "{}", path.display());
        }
    }
}
