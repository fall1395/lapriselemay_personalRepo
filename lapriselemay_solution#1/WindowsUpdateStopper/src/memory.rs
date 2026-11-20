//! La mémoire : la RAM privée de chaque processus (celle de la colonne Mémoire du Gestionnaire des
//! tâches) et celle du PC.

use std::collections::HashMap;
use std::mem::size_of;

use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation};
use windows::Win32::Foundation::STATUS_INFO_LENGTH_MISMATCH;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::WindowsProgramming::SYSTEM_PROCESS_INFORMATION;

/// La RAM privée de chaque processus, en octets, par PID : ce qu'un processus libère en se
/// fermant (la mémoire partagée avec d'autres, comme les DLL de Windows, reste). Une seule
/// demande à Windows pour tous les processus, sans droits particuliers.
pub fn private_ram() -> HashMap<u32, u64> {
    let mut size = 1usize << 20;
    for _ in 0..5 {
        // En u64 : les entrées sont alignées sur 8 octets.
        let mut buffer = vec![0u64; size / 8];
        let mut needed = 0u32;
        let status = unsafe {
            NtQuerySystemInformation(
                SystemProcessInformation,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 8) as u32,
                &mut needed,
            )
        };
        if status == STATUS_INFO_LENGTH_MISMATCH {
            // Des processus de plus entre-temps : un peu de marge.
            size = (needed as usize).max(size) * 2;
            continue;
        }
        if status.is_err() {
            break;
        }
        return parse(&buffer);
    }
    HashMap::new()
}

fn parse(buffer: &[u64]) -> HashMap<u32, u64> {
    let len = buffer.len() * 8;
    let base = buffer.as_ptr().cast::<u8>();
    let mut ram = HashMap::new();
    let mut offset = 0usize;
    while offset + size_of::<SYSTEM_PROCESS_INFORMATION>() <= len {
        // Entrée entièrement dans le tampon (vérifié ci-dessus) et alignée : Windows aligne
        // chaque entrée sur 8 octets.
        let info = unsafe { &*base.add(offset).cast::<SYSTEM_PROCESS_INFORMATION>() };
        // La RAM privée (WorkingSetPrivateSize) occupe les 8 premiers octets de Reserved1 :
        // champ non documenté, mais stable depuis Windows Vista, et celui du Gestionnaire des
        // tâches.
        let mut private = [0u8; 8];
        private.copy_from_slice(&info.Reserved1[..8]);
        ram.insert(
            info.UniqueProcessId.0 as usize as u32,
            u64::from_ne_bytes(private),
        );
        if info.NextEntryOffset == 0 {
            break;
        }
        offset += info.NextEntryOffset as usize;
    }
    ram
}

/// La RAM du PC : (utilisée, totale), en octets.
pub fn system_ram() -> Option<(u64, u64)> {
    let mut status = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    Some((
        status.ullTotalPhys.saturating_sub(status.ullAvailPhys),
        status.ullTotalPhys,
    ))
}

/// « 1,3 Mo », « 24 Mo », « 1,2 Go » : en unités de 1024, avec la virgule décimale. Une
/// décimale sous 10 Mo : la plupart des services n'occupent que quelques mégaoctets en propre.
pub fn format(bytes: u64) -> String {
    const MO: f64 = 1024.0 * 1024.0;
    let mo = bytes as f64 / MO;
    if bytes == 0 {
        "0 Mo".into()
    } else if mo < 0.1 {
        "< 0,1 Mo".into()
    } else if mo < 10.0 {
        format!("{mo:.1} Mo").replace('.', ",")
    } else if mo < 1000.0 {
        format!("{mo:.0} Mo")
    } else {
        format!("{:.1} Go", mo / 1024.0).replace('.', ",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_this_process() {
        let ram = private_ram();
        assert!(ram.len() > 10, "{} processus seulement", ram.len());
        let own = ram.get(&std::process::id()).copied().unwrap_or(0);
        assert!(own > 100 * 1024, "RAM privée de ce test : {own} octets");
        let (used, total) = system_ram().unwrap();
        assert!(used > 0 && used <= total);
    }

    #[test]
    fn formats_in_french() {
        assert_eq!(format(0), "0 Mo");
        assert_eq!(format(50 * 1024), "< 0,1 Mo");
        assert_eq!(format(300 * 1024), "0,3 Mo");
        assert_eq!(format(1363 * 1024), "1,3 Mo");
        assert_eq!(format(24 * 1024 * 1024), "24 Mo");
        assert_eq!(format(1536 * 1024 * 1024), "1,5 Go");
        assert_eq!(format(16 * 1024 * 1024 * 1024), "16,0 Go");
    }
}
