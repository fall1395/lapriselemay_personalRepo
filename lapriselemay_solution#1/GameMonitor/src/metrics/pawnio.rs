//! Température du processeur par le pilote PawnIO, sans LibreHardwareMonitor.
//!
//! Windows ne donne pas la température du processeur sans pilote noyau. PawnIO (<https://pawnio.eu>)
//! en est un, signé, qui n'exécute que des modules signés par son auteur. Les deux modules officiels
//! dont GameMonitor a besoin sont intégrés à l'exe (voir `pawnio/`), mais seul celui du processeur est
//! lu et chargé : IntelMSR sur un Intel, AMDFamily17 sur un Ryzen. Chacun ne lit que quelques registres
//! du processeur, dont ceux de sa sonde thermique. PawnIO s'installe une fois ; seuls les
//! administrateurs peuvent s'en servir.

use std::arch::x86_64::__cpuid;
use std::ffi::c_void;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL,
    LocalFree, WAIT_ABANDONED, WAIT_OBJECT_0,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Threading::{
    CreateMutexW, MUTEX_MODIFY_STATE, OpenMutexW, ReleaseMutex, SYNCHRONIZATION_SYNCHRONIZE,
    WaitForSingleObject,
};
use windows::core::w;

/// Modules de PawnIO.Modules 0.2.11, tels que publiés et signés par l'auteur de PawnIO.
const INTEL_MSR: &[u8] = include_bytes!("../../pawnio/IntelMSR.bin");
const AMD_FAMILY_17: &[u8] = include_bytes!("../../pawnio/AMDFamily17.bin");

/// Code de contrôle du pilote : CTL_CODE(41394, fonction, METHOD_BUFFERED, FILE_ANY_ACCESS).
const fn control_code(function: u32) -> u32 {
    (41394 << 16) | (function << 2)
}
const IOCTL_LOAD_BINARY: u32 = control_code(0x821);
const IOCTL_EXECUTE_FN: u32 = control_code(0x841);
/// Place du nom de la fonction à exécuter, en tête de l'appel (zéro final compris).
const FN_NAME_LENGTH: usize = 32;

/// Intel : température maximale (TjMax), et état thermique du processeur entier.
const MSR_TEMPERATURE_TARGET: u64 = 0x1A2;
const IA32_PACKAGE_THERM_STATUS: u64 = 0x1B1;
/// Ryzen : registre SMN de la température de contrôle (Tctl).
const THM_TCON_CUR_TMP: u64 = 0x0005_9800;
/// Attente maximale du verrou du bus PCI, quand un autre outil de mesure le tient.
const PCI_LOCK_WAIT_MS: u32 = 100;

/// Pourquoi PawnIO ne sert pas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Unavailable {
    /// Processeur que GameMonitor ne sait pas lire par PawnIO.
    Unsupported,
    /// Pilote non installé (sur un processeur qu'il permettrait de lire).
    Missing,
    /// Installé, mais refusé : GameMonitor lancé sans les droits administrateur, pilote trop ancien…
    Refused,
}

/// Une lecture de la sonde.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reading {
    Celsius(f32),
    /// Pas de mesure cette fois : bus PCI occupé par un autre outil, lecture signalée non valide.
    Skipped,
    /// Le pilote ne répond plus (arrêté ou désinstallé).
    Failed,
}

/// Sonde thermique du processeur, lue par PawnIO avec le seul module dont il a besoin.
pub enum PawnIo {
    Intel { driver: Driver, tjmax: f32 },
    Ryzen { driver: Driver, lock: PciLock, tctl_offset: f32 },
}

impl PawnIo {
    pub fn open() -> Result<Self, Unavailable> {
        match cpu() {
            Some(Cpu::Intel) => {
                let driver = Driver::open(INTEL_MSR)?;
                let tjmax = tjmax(driver.execute("ioctl_read_msr", MSR_TEMPERATURE_TARGET));
                Ok(Self::Intel { driver, tjmax })
            }
            Some(Cpu::Ryzen) => {
                let driver = Driver::open(AMD_FAMILY_17)?;
                let lock = PciLock::open().ok_or(Unavailable::Refused)?;
                Ok(Self::Ryzen { driver, lock, tctl_offset: tctl_offset(&brand()) })
            }
            None => Err(Unavailable::Unsupported),
        }
    }

    /// Température du processeur entier (Intel) ou de contrôle corrigée (Ryzen), en °C.
    pub fn read(&self) -> Reading {
        match self {
            Self::Intel { driver, tjmax } => match driver.execute("ioctl_read_msr", IA32_PACKAGE_THERM_STATUS) {
                Some(status) => package_celsius(status, *tjmax).map_or(Reading::Skipped, Reading::Celsius),
                None => Reading::Failed,
            },
            Self::Ryzen { driver, lock, tctl_offset } => {
                match lock.hold(|| driver.execute("ioctl_read_smn", THM_TCON_CUR_TMP)) {
                    Some(Some(raw)) => Reading::Celsius(ryzen_celsius(raw as u32, *tctl_offset)),
                    Some(None) => Reading::Failed,
                    None => Reading::Skipped,
                }
            }
        }
    }
}

/// Le pilote PawnIO, avec un module chargé.
pub struct Driver(HANDLE);

impl Driver {
    fn open(module: &[u8]) -> Result<Self, Unavailable> {
        let handle = unsafe {
            CreateFileW(
                w!(r"\\?\GLOBALROOT\Device\PawnIO"),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .map_err(|e| {
            let missing = [ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND].map(|c| c.to_hresult());
            if missing.contains(&e.code()) { Unavailable::Missing } else { Unavailable::Refused }
        })?;
        // Fermé par Drop si le module ne se charge pas.
        let driver = Self(handle);
        driver.control(IOCTL_LOAD_BINARY, module, &mut []).ok_or(Unavailable::Refused)?;
        Ok(driver)
    }

    /// Exécute la fonction `function` du module, avec un argument et une valeur en retour. Le module
    /// refuse les registres qui n'ont rien à voir avec la mesure.
    fn execute(&self, function: &str, arg: u64) -> Option<u64> {
        let mut value = [0u8; 8];
        let read = self.control(IOCTL_EXECUTE_FN, &call(function, &[arg]), &mut value)?;
        (read == value.len()).then(|| u64::from_le_bytes(value))
    }

    /// Envoie `input` au pilote et reçoit sa réponse dans `output`. Nombre d'octets reçus.
    fn control(&self, code: u32, input: &[u8], output: &mut [u8]) -> Option<usize> {
        let mut returned = 0u32;
        unsafe {
            DeviceIoControl(
                self.0,
                code,
                Some(input.as_ptr().cast::<c_void>()),
                u32::try_from(input.len()).ok()?,
                (!output.is_empty()).then(|| output.as_mut_ptr().cast::<c_void>()),
                u32::try_from(output.len()).ok()?,
                Some(&mut returned),
                None,
            )
        }
        .ok()?;
        usize::try_from(returned).ok()
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Verrou partagé du bus PCI (« Global\Access_PCI »), que prennent aussi LibreHardwareMonitor, HWiNFO
/// et d'autres outils de mesure : lire un registre SMN demande deux accès successifs au bus, qu'un autre
/// outil ne doit pas entrecouper.
pub struct PciLock(HANDLE);

impl PciLock {
    fn open() -> Option<Self> {
        let name = w!(r"Global\Access_PCI");
        unsafe {
            // Créé ouvert à tous, comme le font les autres outils : sinon, un outil lancé sans les droits
            // administrateur ne pourrait plus le prendre.
            let mut sd = PSECURITY_DESCRIPTOR::default();
            let created = ConvertStringSecurityDescriptorToSecurityDescriptorW(
                w!("D:(A;;GA;;;WD)"),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
            .ok()
            .and_then(|()| {
                let sa = SECURITY_ATTRIBUTES {
                    nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: sd.0,
                    bInheritHandle: false.into(),
                };
                let handle = CreateMutexW(Some(&raw const sa), false, name).ok();
                let _ = LocalFree(Some(HLOCAL(sd.0)));
                handle
            });
            // Déjà créé par un outil qui n'en laisse que l'usage.
            created
                .or_else(|| OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE | MUTEX_MODIFY_STATE, false, name).ok())
                .map(Self)
        }
    }

    /// Exécute `read` en tenant le verrou. `None` si un autre outil le garde trop longtemps.
    fn hold<R>(&self, read: impl FnOnce() -> R) -> Option<R> {
        let wait = unsafe { WaitForSingleObject(self.0, PCI_LOCK_WAIT_MS) };
        // Abandonné : son dernier détenteur s'est terminé sans le rendre ; il nous revient quand même.
        if wait != WAIT_OBJECT_0 && wait != WAIT_ABANDONED {
            return None;
        }
        let value = read();
        unsafe {
            let _ = ReleaseMutex(self.0);
        }
        Some(value)
    }
}

impl Drop for PciLock {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cpu {
    Intel,
    Ryzen,
}

/// Processeur dont PawnIO sait lire la sonde : Intel avec une sonde pour le processeur entier
/// (tous depuis 2011), ou AMD Ryzen (familles 17h à 1Ah, depuis 2017).
fn cpu() -> Option<Cpu> {
    let id = __cpuid(0);
    match (id.ebx, id.edx, id.ecx) {
        // « GenuineIntel » ; sonde du processeur entier : CPUID 6, EAX bit 6.
        (0x756e_6547, 0x4965_6e69, 0x6c65_746e) => {
            (id.eax >= 6 && __cpuid(6).eax & (1 << 6) != 0).then_some(Cpu::Intel)
        }
        // « AuthenticAMD »
        (0x6874_7541, 0x6974_6e65, 0x444d_4163) => {
            (0x17..=0x1A).contains(&family(__cpuid(1).eax)).then_some(Cpu::Ryzen)
        }
        _ => None,
    }
}

/// Famille du processeur d'après CPUID 1 : la famille de base, plus la famille étendue quand elle
/// vaut 0xF.
fn family(eax: u32) -> u32 {
    let base = (eax >> 8) & 0xF;
    if base == 0xF { base + ((eax >> 20) & 0xFF) } else { base }
}

/// Nom commercial du processeur (« AMD Ryzen 7 2700X Eight-Core Processor »).
fn brand() -> String {
    if __cpuid(0x8000_0000).eax < 0x8000_0004 {
        return String::new();
    }
    let bytes: Vec<u8> = (0x8000_0002..=0x8000_0004u32)
        .map(__cpuid)
        .flat_map(|r| [r.eax, r.ebx, r.ecx, r.edx])
        .flat_map(u32::to_le_bytes)
        .collect();
    String::from_utf8_lossy(&bytes).trim_matches(['\0', ' ']).to_string()
}

/// Appel d'une fonction du module : son nom sur 32 octets complétés de zéros, puis ses arguments.
fn call(name: &str, args: &[u64]) -> Vec<u8> {
    let mut input = vec![0u8; FN_NAME_LENGTH];
    let len = name.len().min(FN_NAME_LENGTH - 1);
    input[..len].copy_from_slice(&name.as_bytes()[..len]);
    for arg in args {
        input.extend_from_slice(&arg.to_le_bytes());
    }
    input
}

/// TjMax d'après le registre de température cible (bits 23 à 16) ; 100 °C s'il ne le donne pas,
/// comme LibreHardwareMonitor.
fn tjmax(target: Option<u64>) -> f32 {
    match target.map(|v| (v >> 16) & 0xFF) {
        Some(t) if t > 0 => t as f32,
        _ => 100.0,
    }
}

/// Température d'après l'état thermique du processeur entier : TjMax moins l'écart lu par la sonde
/// (bits 22 à 16), si la lecture est valide (bit 31).
fn package_celsius(status: u64, tjmax: f32) -> Option<f32> {
    (status & 0x8000_0000 != 0).then_some(tjmax - ((status >> 16) & 0x7F) as f32)
}

/// Température d'un Ryzen d'après THM_TCON_CUR_TMP, comme le pilote k10temp de Linux : bits 31 à 21 en
/// huitièmes de degré, moins 49 °C quand le registre le signale (bit 19, ou bits 17-16 à 11), moins
/// l'écart entre température de contrôle et vraie température de certains modèles.
fn ryzen_celsius(raw: u32, tctl_offset: f32) -> f32 {
    let mut celsius = (raw >> 21) as f32 * 0.125;
    if raw & (1 << 19) != 0 || raw & 0x3_0000 == 0x3_0000 {
        celsius -= 49.0;
    }
    celsius - tctl_offset
}

/// Écart entre température de contrôle (Tctl) et vraie température (Tdie) des quelques Ryzen qui en
/// ont un, d'après le pilote k10temp de Linux. GameMonitor affiche la vraie.
fn tctl_offset(brand: &str) -> f32 {
    const TABLE: [(&str, f32); 6] = [
        ("AMD Ryzen 5 1600X", 20.0),
        ("AMD Ryzen 7 1700X", 20.0),
        ("AMD Ryzen 7 1800X", 20.0),
        ("AMD Ryzen 7 2700X", 10.0),
        ("AMD Ryzen Threadripper 19", 27.0),
        ("AMD Ryzen Threadripper 29", 27.0),
    ];
    TABLE.iter().find(|(model, _)| brand.contains(model)).map_or(0.0, |(_, offset)| *offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_codes_of_the_driver() {
        assert_eq!(IOCTL_LOAD_BINARY, 0xA1B2_2084);
        assert_eq!(IOCTL_EXECUTE_FN, 0xA1B2_2104);
    }

    #[test]
    fn function_call_layout() {
        let input = call("ioctl_read_msr", &[0x1B1]);
        assert_eq!(input.len(), 32 + 8);
        assert_eq!(&input[..14], b"ioctl_read_msr");
        assert!(input[14..32].iter().all(|&b| b == 0));
        assert_eq!(input[32..], 0x1B1u64.to_le_bytes());
    }

    #[test]
    fn intel_temperature() {
        // TjMax : bits 23 à 16 (0x64 = 100 °C) ; l'écart de TCC (bits 29 à 24) ne compte pas.
        assert_eq!(tjmax(Some(0x0064_0000)), 100.0);
        assert_eq!(tjmax(Some(0x0F5F_0000)), 95.0);
        assert_eq!(tjmax(Some(0)), 100.0);
        assert_eq!(tjmax(None), 100.0);
        // Lecture valide (bit 31), 38 °C (0x26) sous TjMax : 62 °C.
        assert_eq!(package_celsius(0x8826_0000, 100.0), Some(62.0));
        // Lecture non valide : pas de température.
        assert_eq!(package_celsius(0x0826_0000, 100.0), None);
    }

    #[test]
    fn ryzen_temperature() {
        // 480 huitièmes de degré : 60 °C.
        let raw = 480 << 21;
        assert_eq!(ryzen_celsius(raw, 0.0), 60.0);
        // Moins 49 °C quand le registre le signale : bit 19, ou bits 17-16 à 11 (pas à 01).
        assert_eq!(ryzen_celsius(raw | 1 << 19, 0.0), 11.0);
        assert_eq!(ryzen_celsius(raw | 0x3_0000, 0.0), 11.0);
        assert_eq!(ryzen_celsius(raw | 0x1_0000, 0.0), 60.0);
        // Ryzen 7 1800X : la température de contrôle a 20 °C de trop.
        assert_eq!(ryzen_celsius(raw, 20.0), 40.0);
    }

    #[test]
    fn ryzen_models_with_a_tctl_offset() {
        assert_eq!(tctl_offset("AMD Ryzen 7 1800X Eight-Core Processor"), 20.0);
        assert_eq!(tctl_offset("AMD Ryzen 7 2700X Eight-Core Processor"), 10.0);
        assert_eq!(tctl_offset("AMD Ryzen Threadripper 2990WX 32-Core Processor"), 27.0);
        // Les autres, dont tous les Ryzen depuis la série 3000 : pas d'écart.
        assert_eq!(tctl_offset("AMD Ryzen 5 3600 6-Core Processor"), 0.0);
        assert_eq!(tctl_offset("AMD Ryzen 7 5800X3D 8-Core Processor"), 0.0);
        assert_eq!(tctl_offset("AMD Ryzen Threadripper 7960X 24-Cores"), 0.0);
        assert_eq!(tctl_offset(""), 0.0);
    }

    #[test]
    fn cpu_family() {
        assert_eq!(family(0x0087_0F10), 0x17); // Ryzen 5 3600 (Zen 2)
        assert_eq!(family(0x00A2_0F12), 0x19); // Ryzen 5000 (Zen 3)
        assert_eq!(family(0x00B4_0F00), 0x1A); // Ryzen 9000 (Zen 5)
        assert_eq!(family(0x0008_06C1), 6); // Intel Core i5-1135G7
    }

    #[test]
    fn modules_are_embedded() {
        assert_eq!(INTEL_MSR.len(), 5324);
        assert_eq!(AMD_FAMILY_17.len(), 10652);
    }
}
