//! Le catalogue de Windows Update Stopper : les services de Windows Update, chacun avec ce qu'on perd tant
//! qu'il est arrêté ; et les services qu'il ne touche jamais.
//!
//! Tout est dans le code, et non dans un fichier à côté de l'exe : un programme lancé en
//! administrateur qui lirait sa liste dans un fichier modifiable ferait le jeu d'un logiciel
//! malveillant.

/// Un service du catalogue.
#[derive(Debug)]
pub struct Entry {
    /// Nom du service, celui de `sc query`.
    pub name: &'static str,
    /// Nom affiché.
    pub label: &'static str,
    /// Ce qu'on perd tant qu'il est arrêté.
    pub note: &'static str,
}

impl Entry {
    /// La casse ne compte pas, comme pour Windows.
    pub fn matches(&self, name: &str) -> bool {
        name.eq_ignore_ascii_case(self.name)
    }
}

const fn entry(name: &'static str, label: &'static str, note: &'static str) -> Entry {
    Entry { name, label, note }
}

/// Windows Update et ce qui le fait tourner : son orchestrateur, qui planifie les recherches de
/// mises à jour, et les deux services qui font ses téléchargements. « Tout arrêter » les arrête
/// dans cet ordre, l'orchestrateur d'abord : il relancerait Windows Update. « Tout relancer » les
/// relance dans l'ordre inverse. Un service passe avant ceux dont il dépend (sinon Windows
/// refuserait de les arrêter) : un test le vérifie sur le vrai Windows.
pub const CATALOG: &[Entry] = &[
    entry(
        "UsoSvc",
        "Orchestrateur des mises à jour",
        "Windows ne planifie plus de recherche de mises à jour.",
    ),
    entry(
        "wuauserv",
        "Windows Update",
        "Aucune mise à jour Windows téléchargée ni installée pendant la partie.",
    ),
    entry(
        "BITS",
        "Transfert intelligent en arrière-plan (BITS)",
        "Les téléchargements de Windows et de certaines applis attendent.",
    ),
    entry(
        "DoSvc",
        "Optimisation de la distribution",
        "L'appli Xbox et le Microsoft Store ne téléchargent plus : mets tes jeux à jour avant.",
    ),
];

/// Jamais arrêtés, même ajoutés par erreur au catalogue : les tests le vérifient, et `find` les
/// écarte de toute façon. Le jeu, la connexion Xbox, le son, le réseau, les manettes, la diffusion
/// du jeu (Sunshine), l'affichage ou la sécurité en dépendent.
pub const NEVER: &[&str] = &[
    // Xbox et Microsoft Store : connexion Xbox de MSFS 2024, Game Pass, sauvegardes en ligne.
    "XblAuthManager",
    "XblGameSave",
    "XboxNetApiSvc",
    "XboxGipSvc",
    "GamingServices",
    "GamingServicesNet",
    "GameInputSvc",
    "wlidsvc",
    "TokenBroker",
    "InstallService",
    "ClipSVC",
    "AppXSvc",
    "LicenseManager",
    // Son.
    "Audiosrv",
    "AudioEndpointBuilder",
    // Réseau et VPN ; Teredo (iphlpsvc) et UPnP (SSDPSRV, upnphost) servent au multijoueur.
    "Dhcp",
    "Dnscache",
    "NlaSvc",
    "nsi",
    "netprofm",
    "Wcmsvc",
    "WlanSvc",
    "iphlpsvc",
    "SSDPSRV",
    "upnphost",
    "BFE",
    "mpssvc",
    "RasMan",
    "LanmanWorkstation",
    // Manettes, claviers, casques Bluetooth.
    "bthserv",
    "BthAvctpSvc",
    "BTAGService",
    "hidserv",
    "DeviceAssociationService",
    "TabletInputService",
    "TextInputManagementService",
    // Affichage et pilote NVIDIA.
    "Themes",
    "NVDisplay.ContainerLocalSystem",
    // Sécurité.
    "WinDefend",
    "WdNisSvc",
    "SecurityHealthService",
    "wscsvc",
    "SamSs",
    "CryptSvc",
    // Notifications : invitations Xbox, Discord…
    "WpnService",
    // Office : arrêter son service ferme Word, Excel et compagnie.
    "ClickToRunSvc",
    // Lanceurs, anti-triche et diffusion du jeu.
    "Steam Client Service",
    "EpicOnlineServices",
    "EasyAntiCheat",
    "EasyAntiCheat_EOS",
    "BEService",
    "vgc",
    "SunshineService",
    // Cœur de Windows, et WMI (GameMonitor s'en sert).
    "RpcSs",
    "RpcEptMapper",
    "DcomLaunch",
    "EventLog",
    "EventSystem",
    "Schedule",
    "Winmgmt",
    "PlugPlay",
    "Power",
    "ProfSvc",
    "UserManager",
    "StateRepository",
    "BrokerInfrastructure",
    "SystemEventsBroker",
    "CoreMessagingRegistrar",
    "FontCache",
    "gpsvc",
    "LSM",
];

/// Vrai pour les services que Windows Update Stopper ne touche jamais.
pub fn is_protected(name: &str) -> bool {
    NEVER.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// L'entrée du catalogue d'un service, et sa position : `None` s'il n'y est pas, ou s'il est
/// protégé.
pub fn find(name: &str) -> Option<(usize, &'static Entry)> {
    if is_protected(name) {
        return None;
    }
    CATALOG.iter().enumerate().find(|(_, e)| e.matches(name))
}

/// L'entrée dont le nom (du catalogue, pas du service) est `name` : pour les cases retenues.
pub fn by_name(name: &str) -> Option<&'static Entry> {
    CATALOG.iter().find(|e| e.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_protected_service_in_catalog() {
        for entry in CATALOG {
            assert!(!is_protected(entry.name), "{} est protégé", entry.name);
            for never in NEVER {
                assert!(
                    !entry.matches(never),
                    "{} couvrirait {never}, protégé",
                    entry.name
                );
            }
        }
    }

    #[test]
    fn no_duplicates() {
        for (i, a) in CATALOG.iter().enumerate() {
            for b in &CATALOG[i + 1..] {
                assert!(!a.name.eq_ignore_ascii_case(b.name), "{} en double", a.name);
                assert_ne!(a.label, b.label, "nom affiché en double");
            }
        }
    }

    #[test]
    fn texts_fit_the_window() {
        for entry in CATALOG {
            assert!(!entry.note.is_empty() && entry.note.chars().count() <= 90);
            assert!(entry.label.chars().count() <= 52, "{}", entry.label);
            assert!(entry.note.ends_with('.'), "{}", entry.note);
        }
    }

    #[test]
    fn windows_update_only_orchestrator_first() {
        let names: Vec<_> = CATALOG.iter().map(|e| e.name).collect();
        assert_eq!(names, ["UsoSvc", "wuauserv", "BITS", "DoSvc"]);
    }

    #[test]
    fn finds_by_name_ignoring_case() {
        assert_eq!(
            find("WUAUSERV").map(|(i, e)| (i, e.label)),
            Some((1, "Windows Update"))
        );
        assert!(find("wuauserv2").is_none());
        assert!(find("WSearch").is_none());
        assert!(find("XblAuthManager").is_none());
        assert!(by_name("BITS").is_some());
        assert!(by_name("bits").is_none());
    }
}
