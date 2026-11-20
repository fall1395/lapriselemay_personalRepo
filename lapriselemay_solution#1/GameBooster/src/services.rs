//! Les appels au gestionnaire de services de Windows (SCM) : la liste des services avec leur état
//! et leur processus, leur description, leur arrêt et leur relance.
//!
//! GameBooster ne change jamais le type de démarrage d'un service : il l'arrête, c'est tout. Si
//! Windows redémarre, tout revient donc à la normale de soi-même.

use std::thread;
use std::time::{Duration, Instant};

use windows::core::{Error, HSTRING, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_DEPENDENT_SERVICES_RUNNING, ERROR_MORE_DATA,
    ERROR_SERVICE_ALREADY_RUNNING, ERROR_SERVICE_CANNOT_ACCEPT_CTRL, ERROR_SERVICE_DISABLED,
    ERROR_SERVICE_DOES_NOT_EXIST, ERROR_SERVICE_NOT_ACTIVE, HANDLE, WIN32_ERROR,
};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Services::{
    CloseServiceHandle, ControlService, EnumServicesStatusExW, OpenSCManagerW, OpenServiceW,
    QueryServiceConfig2W, QueryServiceStatus, StartServiceW, ENUM_SERVICE_STATUS_PROCESSW,
    SC_ENUM_PROCESS_INFO, SC_HANDLE, SC_MANAGER_CONNECT, SC_MANAGER_ENUMERATE_SERVICE,
    SERVICE_CONFIG_DESCRIPTION, SERVICE_CONTINUE_PENDING, SERVICE_CONTROL_STOP,
    SERVICE_DESCRIPTIONW, SERVICE_PAUSED, SERVICE_PAUSE_PENDING, SERVICE_QUERY_CONFIG,
    SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_START, SERVICE_START_PENDING, SERVICE_STATE_ALL,
    SERVICE_STATUS, SERVICE_STATUS_CURRENT_STATE, SERVICE_STOP, SERVICE_STOP_PENDING,
    SERVICE_WIN32,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// Attentes : la fin d'un démarrage en cours avant l'arrêt, puis l'arrêt lui-même.
const START_WAIT: Duration = Duration::from_secs(5);
const STOP_WAIT: Duration = Duration::from_secs(15);

/// L'état d'un service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Stopped,
    Starting,
    Stopping,
    Running,
    Paused,
}

impl State {
    fn from_raw(state: SERVICE_STATUS_CURRENT_STATE) -> State {
        if state == SERVICE_RUNNING {
            State::Running
        } else if state == SERVICE_START_PENDING || state == SERVICE_CONTINUE_PENDING {
            State::Starting
        } else if state == SERVICE_STOP_PENDING {
            State::Stopping
        } else if state == SERVICE_PAUSED || state == SERVICE_PAUSE_PENDING {
            State::Paused
        } else {
            State::Stopped
        }
    }

    /// Son processus tourne : il occupe de la RAM, et on peut l'arrêter.
    pub fn is_running(self) -> bool {
        matches!(self, State::Running | State::Starting | State::Paused)
    }

    /// En train de démarrer ou de s'arrêter : mieux vaut attendre avant d'y toucher.
    pub fn is_pending(self) -> bool {
        matches!(self, State::Starting | State::Stopping)
    }

    pub fn label(self) -> &'static str {
        match self {
            State::Running => "En cours",
            State::Starting => "Démarrage…",
            State::Stopping => "Arrêt…",
            State::Paused => "En pause",
            State::Stopped => "Arrêté",
        }
    }
}

/// Un service installé, tel que Windows le décrit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    /// Nom du service, celui de `sc query`.
    pub name: String,
    /// Nom affiché par Windows, dans sa langue.
    pub display_name: String,
    pub state: State,
    /// Processus qui l'héberge ; 0 s'il est arrêté.
    pub pid: u32,
}

/// Poignée du gestionnaire de services ou d'un service, fermée à la fin.
struct Handle(SC_HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

/// Connexion au gestionnaire de services de Windows. Lire l'état des services ne demande aucun
/// droit particulier ; les arrêter et les relancer, les droits administrateur.
pub struct Scm(Handle);

impl Scm {
    pub fn open() -> Result<Scm, String> {
        let access = SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE;
        unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), access) }
            .map(|h| Scm(Handle(h)))
            .map_err(|e| reason(&e))
    }

    fn service(&self, name: &str, access: u32) -> windows::core::Result<Handle> {
        unsafe { OpenServiceW(self.0 .0, &HSTRING::from(name), access) }.map(Handle)
    }

    /// Tous les services, y compris ceux de chaque session d'utilisateur, avec leur état et le
    /// processus qui les héberge.
    pub fn services(&self) -> Vec<Info> {
        self.services_with(8 * 1024)
    }

    /// `services`, avec un tampon de départ de `words` × 8 octets (agrandi s'il le faut).
    fn services_with(&self, words: usize) -> Vec<Info> {
        let mut infos = Vec::new();
        let mut resume = 0u32;
        // En u64 : les entrées contiennent des pointeurs, le tampon doit être aligné sur 8 octets.
        let mut buffer = vec![0u64; words];
        loop {
            let (mut needed, mut count) = (0u32, 0u32);
            let result = unsafe {
                let bytes = std::slice::from_raw_parts_mut(
                    buffer.as_mut_ptr().cast::<u8>(),
                    buffer.len() * 8,
                );
                EnumServicesStatusExW(
                    self.0 .0,
                    SC_ENUM_PROCESS_INFO,
                    SERVICE_WIN32,
                    SERVICE_STATE_ALL,
                    Some(bytes),
                    &mut needed,
                    &mut count,
                    Some(&mut resume as *mut u32),
                    PCWSTR::null(),
                )
            };
            let entries = unsafe {
                std::slice::from_raw_parts(
                    buffer.as_ptr().cast::<ENUM_SERVICE_STATUS_PROCESSW>(),
                    count as usize,
                )
            };
            infos.extend(entries.iter().filter_map(|entry| {
                let name = unsafe { entry.lpServiceName.to_string() }.ok()?;
                let display_name = unsafe { entry.lpDisplayName.to_string() }.unwrap_or_default();
                let status = &entry.ServiceStatusProcess;
                let state = State::from_raw(status.dwCurrentState);
                Some(Info {
                    name,
                    display_name,
                    state,
                    pid: if state == State::Stopped {
                        0
                    } else {
                        status.dwProcessId
                    },
                })
            }));
            let too_small = needed as usize > buffer.len() * 8;
            match result {
                // La suite au prochain tour, avec un tampon agrandi s'il le faut.
                Err(e) if is(&e, ERROR_MORE_DATA) && (count > 0 || too_small) => {
                    if too_small {
                        buffer = vec![0u64; (needed as usize).div_ceil(8)];
                    }
                }
                _ => return infos,
            }
        }
    }

    /// La description d'un service, dans la langue de Windows.
    pub fn description(&self, name: &str) -> Option<String> {
        let service = self.service(name, SERVICE_QUERY_CONFIG).ok()?;
        let mut needed = 0u32;
        let _ = unsafe {
            QueryServiceConfig2W(service.0, SERVICE_CONFIG_DESCRIPTION, None, &mut needed)
        };
        if needed == 0 {
            return None;
        }
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
        unsafe {
            let bytes =
                std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast::<u8>(), buffer.len() * 8);
            QueryServiceConfig2W(
                service.0,
                SERVICE_CONFIG_DESCRIPTION,
                Some(bytes),
                &mut needed,
            )
            .ok()?;
        }
        let description = unsafe { &*buffer.as_ptr().cast::<SERVICE_DESCRIPTIONW>() };
        if description.lpDescription.is_null() {
            return None;
        }
        let text = unsafe { description.lpDescription.to_string() }.ok()?;
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
    }
}

fn current_state(service: &Handle) -> Option<State> {
    let mut status = SERVICE_STATUS::default();
    unsafe { QueryServiceStatus(service.0, &mut status) }.ok()?;
    Some(State::from_raw(status.dwCurrentState))
}

/// Attend que le service soit dans l'état `wanted`, au plus `timeout`.
fn wait_for(service: &Handle, wanted: State, timeout: Duration) -> bool {
    let end = Instant::now() + timeout;
    loop {
        match current_state(service) {
            Some(state) if state == wanted => return true,
            None => return false,
            _ if Instant::now() >= end => return false,
            _ => thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Résultat de l'arrêt d'un service.
#[derive(Debug, PartialEq, Eq)]
pub enum Stop {
    /// Arrêté par GameBooster : à relancer avec « Tout relancer ».
    Stopped,
    /// Déjà arrêté, ou absent de ce PC : rien à faire, ni à relancer.
    Idle,
    /// Windows a refusé ; la raison, en quelques mots.
    Refused(String),
}

/// Arrête un service et attend qu'il le soit (au plus 15 s).
pub fn stop(scm: &Scm, name: &str) -> Stop {
    let service = match scm.service(name, SERVICE_QUERY_STATUS | SERVICE_STOP) {
        Ok(service) => service,
        Err(e) if is(&e, ERROR_SERVICE_DOES_NOT_EXIST) => return Stop::Idle,
        Err(e) => return Stop::Refused(reason(&e)),
    };
    match current_state(&service) {
        // Déjà arrêté, ou en train de l'être par quelqu'un d'autre.
        Some(State::Stopped | State::Stopping) => return Stop::Idle,
        // En plein démarrage, un service refuse l'arrêt : on le laisse finir.
        Some(State::Starting) => {
            wait_for(&service, State::Running, START_WAIT);
        }
        _ => {}
    }
    let mut status = SERVICE_STATUS::default();
    match unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &mut status) } {
        Ok(()) => {}
        Err(e) if is(&e, ERROR_SERVICE_NOT_ACTIVE) => return Stop::Idle,
        Err(e) => return Stop::Refused(reason(&e)),
    }
    // Même lent à s'arrêter, il l'a été par GameBooster : il sera relancé comme les autres.
    wait_for(&service, State::Stopped, STOP_WAIT);
    Stop::Stopped
}

/// En mode jeu : demande l'arrêt d'un service que Windows a démarré, sans attendre qu'il soit
/// arrêté (c'est fait sur le fil de la fenêtre). Vrai si la demande est partie.
pub fn request_stop(scm: &Scm, name: &str) -> bool {
    let Ok(service) = scm.service(name, SERVICE_QUERY_STATUS | SERVICE_STOP) else {
        return false;
    };
    if current_state(&service) != Some(State::Running) {
        return false;
    }
    let mut status = SERVICE_STATUS::default();
    unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &mut status) }.is_ok()
}

/// Démarre (ou relance) un service.
pub fn start(scm: &Scm, name: &str) -> Result<(), String> {
    let service = scm
        .service(name, SERVICE_QUERY_STATUS | SERVICE_START)
        .map_err(|e| reason(&e))?;
    // Arrêté il y a un instant et pas encore tout à fait : Windows refuserait de le relancer.
    if current_state(&service) == Some(State::Stopping) {
        wait_for(&service, State::Stopped, STOP_WAIT);
    }
    match unsafe { StartServiceW(service.0, None) } {
        Ok(()) => Ok(()),
        Err(e) if is(&e, ERROR_SERVICE_ALREADY_RUNNING) => Ok(()),
        Err(e) => Err(reason(&e)),
    }
}

/// Vrai si GameBooster a les droits administrateur : sans eux, Windows refuse d'arrêter les
/// services.
pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

fn is(error: &Error, code: WIN32_ERROR) -> bool {
    error.code() == code.to_hresult()
}

/// La raison d'un refus de Windows, en quelques mots.
fn reason(error: &Error) -> String {
    let known = [
        (ERROR_ACCESS_DENIED, "accès refusé par Windows"),
        (
            ERROR_DEPENDENT_SERVICES_RUNNING,
            "d'autres services actifs en dépendent",
        ),
        (
            ERROR_SERVICE_CANNOT_ACCEPT_CTRL,
            "occupé, à réessayer plus tard",
        ),
        (ERROR_SERVICE_DISABLED, "désactivé dans Windows"),
    ];
    if let Some((_, text)) = known.iter().find(|(code, _)| is(error, *code)) {
        return text.to_string();
    }
    let message = error.message();
    let message = message.trim().trim_end_matches('.');
    if message.is_empty() {
        format!("erreur 0x{:08X}", error.code().0)
    } else {
        message.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog;
    use windows::Win32::System::Services::{
        EnumDependentServicesW, ENUM_SERVICE_STATUSW, SERVICE_ENUMERATE_DEPENDENTS,
    };

    impl Scm {
        /// Les services qui dépendent de `name`, directement ou non.
        fn dependents(&self, name: &str) -> Vec<String> {
            let Ok(service) = self.service(name, SERVICE_ENUMERATE_DEPENDENTS) else {
                return Vec::new();
            };
            let mut buffer = vec![0u64; 8 * 1024];
            let (mut needed, mut count) = (0u32, 0u32);
            let result = unsafe {
                EnumDependentServicesW(
                    service.0,
                    SERVICE_STATE_ALL,
                    Some(buffer.as_mut_ptr().cast::<ENUM_SERVICE_STATUSW>()),
                    (buffer.len() * 8) as u32,
                    &mut needed,
                    &mut count,
                )
            };
            if result.is_err() {
                return Vec::new();
            }
            let entries = unsafe {
                std::slice::from_raw_parts(
                    buffer.as_ptr().cast::<ENUM_SERVICE_STATUSW>(),
                    count as usize,
                )
            };
            entries
                .iter()
                .filter_map(|e| unsafe { e.lpServiceName.to_string() }.ok())
                .collect()
        }
    }

    /// Lecture seule, sur le vrai gestionnaire de services de Windows : aucun service n'est
    /// arrêté, et les droits administrateur ne sont pas nécessaires.
    #[test]
    fn reads_real_service_manager() {
        let scm = Scm::open().expect("gestionnaire de services");
        let infos = scm.services();
        assert!(infos.len() > 50, "{} services seulement", infos.len());
        let update = infos
            .iter()
            .find(|i| i.name.eq_ignore_ascii_case("wuauserv"));
        assert!(update.is_some_and(|i| !i.display_name.is_empty()));
        assert!(infos
            .iter()
            .all(|i| i.state != State::Stopped || i.pid == 0));
        // Un tampon minuscule oblige à le reprendre en plusieurs fois : mêmes services.
        let names = |infos: &[Info]| infos.iter().map(|i| i.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&scm.services_with(16)), names(&infos));
        assert!(scm
            .description("wuauserv")
            .is_some_and(|d| d.chars().count() > 20));
    }

    /// Sur le vrai Windows : dans le catalogue, un service passe avant ceux dont il dépend.
    #[test]
    fn catalog_stops_dependents_first() {
        let scm = Scm::open().expect("gestionnaire de services");
        for info in scm.services() {
            let Some((position, _)) = catalog::find(&info.name) else {
                continue;
            };
            for dependent in scm.dependents(&info.name) {
                if let Some((dependent_position, _)) = catalog::find(&dependent) {
                    assert!(
                        dependent_position < position,
                        "{dependent} dépend de {} : il doit passer avant dans le catalogue",
                        info.name
                    );
                }
            }
        }
    }
}
