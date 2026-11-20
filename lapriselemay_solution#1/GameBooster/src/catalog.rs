//! Le catalogue de GameBooster : les services Windows inutiles pendant une partie, chacun avec sa
//! catégorie, un conseil et ce qu'on perd tant qu'il est arrêté ; et ceux qu'il ne touche jamais.
//!
//! Tout est dans le code, et non dans un fichier à côté de l'exe : un programme lancé en
//! administrateur qui lirait sa liste dans un fichier modifiable ferait le jeu d'un logiciel
//! malveillant.

/// Regroupement affiché dans la liste.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Updates,
    Indexing,
    Telemetry,
    Printing,
    Network,
    Remote,
    Virtualization,
    Devices,
    Sync,
    Other,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Updates => "Mises à jour",
            Category::Indexing => "Indexation",
            Category::Telemetry => "Télémétrie",
            Category::Printing => "Impression",
            Category::Network => "Réseau local",
            Category::Remote => "Accès à distance",
            Category::Virtualization => "Virtualisation",
            Category::Devices => "Appareils",
            Category::Sync => "Synchronisation",
            Category::Other => "Divers",
        }
    }
}

/// Le conseil pour un service, pendant une partie.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Advice {
    /// Aucun usage en jeu : coché par défaut, donc arrêté par « Tout arrêter ».
    Safe,
    /// Utile selon ce qu'on fait du PC (imprimer, Bureau à distance…) : décoché par défaut.
    Depends,
    /// Plus utile qu'encombrant, même en jeu : décoché par défaut, et son arrêt se confirme.
    Avoid,
}

impl Advice {
    pub fn label(self) -> &'static str {
        match self {
            Advice::Safe => "Sans risque",
            Advice::Depends => "Selon ton usage",
            Advice::Avoid => "Déconseillé",
        }
    }

    pub fn checked_by_default(self) -> bool {
        self == Advice::Safe
    }
}

/// Un service du catalogue.
#[derive(Debug)]
pub struct Entry {
    /// Nom du service, celui de `sc query` ; seulement le début du nom si `prefix`.
    pub name: &'static str,
    /// Le nom finit par un numéro de version (Google Updater) ou de session (services par
    /// utilisateur, comme OneSyncSvc_4f2a1).
    pub prefix: bool,
    /// Nom affiché.
    pub label: &'static str,
    pub category: Category,
    pub advice: Advice,
    /// Ce qu'on perd tant qu'il est arrêté.
    pub note: &'static str,
}

impl Entry {
    /// La casse ne compte pas, comme pour Windows.
    pub fn matches(&self, name: &str) -> bool {
        if self.prefix {
            starts_with_ignore_case(name, self.name)
        } else {
            name.eq_ignore_ascii_case(self.name)
        }
    }
}

const fn exact(
    name: &'static str,
    label: &'static str,
    category: Category,
    advice: Advice,
    note: &'static str,
) -> Entry {
    Entry {
        name,
        prefix: false,
        label,
        category,
        advice,
        note,
    }
}

const fn prefix(
    name: &'static str,
    label: &'static str,
    category: Category,
    advice: Advice,
    note: &'static str,
) -> Entry {
    Entry {
        name,
        prefix: true,
        label,
        category,
        advice,
        note,
    }
}

use Advice::{Avoid, Depends, Safe};
use Category::{
    Devices, Indexing, Network, Other, Printing, Remote, Sync, Telemetry, Updates, Virtualization,
};

/// Les services inutiles pendant une partie. « Tout arrêter » les arrête dans cet ordre, et
/// « Tout relancer » les relance dans l'ordre inverse : un service passe avant ceux dont il dépend
/// (sinon Windows refuserait de les arrêter). Un test le vérifie sur le vrai Windows.
pub const CATALOG: &[Entry] = &[
    // Mises à jour et téléchargements en arrière-plan : réseau, disque et processeur.
    exact(
        "wuauserv",
        "Windows Update",
        Updates,
        Safe,
        "Aucune mise à jour Windows téléchargée ni installée pendant la partie.",
    ),
    exact(
        "UsoSvc",
        "Orchestrateur des mises à jour",
        Updates,
        Safe,
        "Windows ne planifie plus de recherche de mises à jour.",
    ),
    exact(
        "BITS",
        "Transfert intelligent en arrière-plan (BITS)",
        Updates,
        Safe,
        "Les téléchargements de Windows et de certaines applis attendent.",
    ),
    exact(
        "DoSvc",
        "Optimisation de la distribution",
        Updates,
        Safe,
        "L'appli Xbox et le Microsoft Store ne téléchargent plus : mets tes jeux à jour avant.",
    ),
    exact(
        "edgeupdate",
        "Mise à jour de Microsoft Edge",
        Updates,
        Safe,
        "Edge ne se met pas à jour.",
    ),
    exact(
        "edgeupdatem",
        "Mise à jour de Microsoft Edge (tâche)",
        Updates,
        Safe,
        "Edge ne se met pas à jour.",
    ),
    exact(
        "MicrosoftEdgeElevationService",
        "Élévation de Microsoft Edge",
        Updates,
        Safe,
        "Ne sert qu'aux réglages système d'Edge.",
    ),
    exact(
        "OneDrive Updater Service",
        "Mise à jour de OneDrive",
        Updates,
        Safe,
        "OneDrive synchronise toujours ; seules ses mises à jour attendent.",
    ),
    exact(
        "gupdate",
        "Mise à jour de Google",
        Updates,
        Safe,
        "Chrome ne se met pas à jour.",
    ),
    exact(
        "gupdatem",
        "Mise à jour de Google (tâche)",
        Updates,
        Safe,
        "Chrome ne se met pas à jour.",
    ),
    prefix(
        "GoogleUpdaterService",
        "Mise à jour de Google Chrome",
        Updates,
        Safe,
        "Chrome ne se met pas à jour.",
    ),
    prefix(
        "GoogleUpdaterInternalService",
        "Mise à jour de Google Chrome (interne)",
        Updates,
        Safe,
        "Chrome ne se met pas à jour.",
    ),
    exact(
        "brave",
        "Mise à jour de Brave",
        Updates,
        Safe,
        "Brave ne se met pas à jour.",
    ),
    exact(
        "bravem",
        "Mise à jour de Brave (tâche)",
        Updates,
        Safe,
        "Brave ne se met pas à jour.",
    ),
    exact(
        "MozillaMaintenance",
        "Mise à jour de Mozilla Firefox",
        Updates,
        Safe,
        "Firefox ne se met pas à jour.",
    ),
    exact(
        "AdobeARMservice",
        "Mise à jour d'Adobe Acrobat",
        Updates,
        Safe,
        "Acrobat ne se met pas à jour.",
    ),
    exact(
        "AdobeUpdateService",
        "Mise à jour d'Adobe Creative Cloud",
        Updates,
        Safe,
        "Les applis Adobe ne se mettent pas à jour.",
    ),
    exact(
        "dbupdate",
        "Mise à jour de Dropbox",
        Updates,
        Safe,
        "Dropbox ne se met pas à jour.",
    ),
    exact(
        "dbupdatem",
        "Mise à jour de Dropbox (tâche)",
        Updates,
        Safe,
        "Dropbox ne se met pas à jour.",
    ),
    // Indexation et préchargement : lectures disque en rafale.
    exact(
        "WMPNetworkSvc",
        "Partage réseau du Lecteur Windows Media",
        Indexing,
        Safe,
        "Ta bibliothèque multimédia n'est plus partagée sur le réseau.",
    ),
    exact(
        "WSearch",
        "Windows Search (indexation)",
        Indexing,
        Safe,
        "La recherche de fichiers est plus lente ; celle des applis fonctionne toujours.",
    ),
    exact(
        "SysMain",
        "Préchargement (SysMain)",
        Indexing,
        Avoid,
        "Il compresse la mémoire quand elle manque : l'arrêter peut causer plus de saccades.",
    ),
    // Télémétrie, diagnostics et rapports d'erreurs.
    exact(
        "DiagTrack",
        "Télémétrie de Windows",
        Telemetry,
        Safe,
        "Les données de diagnostic ne partent plus vers Microsoft.",
    ),
    exact(
        "dmwappushservice",
        "Routage des messages Push WAP",
        Telemetry,
        Safe,
        "Sert surtout à la télémétrie.",
    ),
    exact(
        "WerSvc",
        "Rapport d'erreurs Windows",
        Telemetry,
        Safe,
        "Les plantages ne sont pas signalés à Microsoft.",
    ),
    exact(
        "PcaSvc",
        "Assistant Compatibilité des programmes",
        Telemetry,
        Safe,
        "Plus de conseils de compatibilité pour les vieux programmes.",
    ),
    exact(
        "DPS",
        "Stratégie de diagnostic",
        Telemetry,
        Safe,
        "Les utilitaires de résolution des problèmes ne détectent plus rien.",
    ),
    exact(
        "WdiServiceHost",
        "Hôte du service de diagnostic",
        Telemetry,
        Safe,
        "Diagnostics de Windows en pause.",
    ),
    exact(
        "WdiSystemHost",
        "Hôte système de diagnostic",
        Telemetry,
        Safe,
        "Diagnostics de Windows en pause.",
    ),
    exact(
        "diagsvc",
        "Exécution des diagnostics",
        Telemetry,
        Safe,
        "Diagnostics de Windows en pause.",
    ),
    exact(
        "TrkWks",
        "Suivi de liens distribués",
        Telemetry,
        Safe,
        "Les raccourcis vers des fichiers déplacés ne se réparent plus d'eux-mêmes.",
    ),
    exact(
        "NvTelemetryContainer",
        "Télémétrie NVIDIA",
        Telemetry,
        Safe,
        "Anciens pilotes NVIDIA seulement.",
    ),
    // Impression, numérisation et télécopie.
    exact(
        "Fax",
        "Télécopie",
        Printing,
        Depends,
        "Impossible d'envoyer ou de recevoir des télécopies.",
    ),
    exact(
        "PrintNotify",
        "Extensions et notifications des imprimantes",
        Printing,
        Depends,
        "Plus de notifications des imprimantes.",
    ),
    exact(
        "Spooler",
        "Spouleur d'impression",
        Printing,
        Depends,
        "Impossible d'imprimer tant qu'il est arrêté.",
    ),
    exact(
        "stisvc",
        "Acquisition d'images Windows (WIA)",
        Printing,
        Depends,
        "Les scanneurs et certains appareils photo ne répondent plus.",
    ),
    exact(
        "WiaRpc",
        "Événements d'acquisition d'images",
        Printing,
        Depends,
        "Les boutons des scanneurs ne lancent plus rien.",
    ),
    // Réseau local et partage.
    exact(
        "lmhosts",
        "Assistance NetBIOS",
        Network,
        Depends,
        "Partage de fichiers avec les vieux appareils du réseau local.",
    ),
    exact(
        "FDResPub",
        "Publication des ressources (découverte réseau)",
        Network,
        Depends,
        "Ce PC n'apparaît plus dans le réseau local des autres.",
    ),
    exact(
        "fdPHost",
        "Découverte des appareils du réseau",
        Network,
        Depends,
        "Les imprimantes et partages du réseau local ne sont plus découverts.",
    ),
    exact(
        "LanmanServer",
        "Serveur (partage de fichiers)",
        Network,
        Depends,
        "Les dossiers partagés de ce PC ne sont plus accessibles du réseau.",
    ),
    exact(
        "WebClient",
        "Client WebDAV",
        Network,
        Safe,
        "Plus d'accès aux dossiers web (WebDAV).",
    ),
    exact(
        "SharedAccess",
        "Partage de connexion Internet (ICS)",
        Network,
        Depends,
        "Peut couper le réseau de WSL et des machines virtuelles.",
    ),
    exact(
        "icssvc",
        "Point d'accès sans fil mobile",
        Network,
        Safe,
        "Le partage de connexion de ce PC est coupé.",
    ),
    exact(
        "WFDSConMgrSvc",
        "Wi-Fi Direct",
        Network,
        Safe,
        "Plus de projection sans fil (Miracast).",
    ),
    // Accès à distance.
    exact(
        "RemoteRegistry",
        "Registre à distance",
        Remote,
        Safe,
        "Personne ne peut modifier le registre de ce PC à distance (plus sûr).",
    ),
    exact(
        "WinRM",
        "Gestion à distance de Windows (WinRM)",
        Remote,
        Safe,
        "Plus d'administration à distance par PowerShell.",
    ),
    exact(
        "UmRdpService",
        "Redirecteur de ports du Bureau à distance",
        Remote,
        Depends,
        "Plus d'imprimantes ni de lecteurs redirigés en Bureau à distance.",
    ),
    exact(
        "TermService",
        "Bureau à distance",
        Remote,
        Depends,
        "Impossible de se connecter à ce PC en Bureau à distance.",
    ),
    exact(
        "SessionEnv",
        "Configuration du Bureau à distance",
        Remote,
        Depends,
        "Bureau à distance seulement.",
    ),
    exact(
        "TeamViewer",
        "TeamViewer",
        Remote,
        Depends,
        "Personne ne peut se connecter à ce PC avec TeamViewer.",
    ),
    exact(
        "AnyDesk",
        "AnyDesk",
        Remote,
        Depends,
        "Personne ne peut se connecter à ce PC avec AnyDesk.",
    ),
    // Virtualisation : WSL, Hyper-V, bac à sable.
    exact(
        "LxssManager",
        "Sous-système Windows pour Linux (WSL)",
        Virtualization,
        Depends,
        "Ferme WSL et libère la mémoire de sa machine virtuelle.",
    ),
    exact(
        "WSLService",
        "Service WSL",
        Virtualization,
        Depends,
        "Ferme WSL et libère la mémoire de sa machine virtuelle.",
    ),
    exact(
        "vmcompute",
        "Calcul hôte Hyper-V",
        Virtualization,
        Depends,
        "Arrête les conteneurs, WSL 2 et le bac à sable Windows.",
    ),
    exact(
        "vmms",
        "Gestion des machines virtuelles Hyper-V",
        Virtualization,
        Depends,
        "Les machines virtuelles Hyper-V s'arrêtent.",
    ),
    exact(
        "HvHost",
        "Service hôte Hyper-V",
        Virtualization,
        Depends,
        "Sert aux machines virtuelles et à WSL 2.",
    ),
    // Appareils, capteurs et téléphone.
    exact(
        "PhoneSvc",
        "Service Téléphone",
        Devices,
        Safe,
        "Plus d'appels avec Phone Link.",
    ),
    exact(
        "SmsRouter",
        "Routeur SMS",
        Devices,
        Safe,
        "Plus de messages texte avec Phone Link.",
    ),
    exact(
        "lfsvc",
        "Géolocalisation",
        Devices,
        Safe,
        "Les applis ne connaissent plus la position du PC.",
    ),
    exact(
        "SensorDataService",
        "Données des capteurs",
        Devices,
        Safe,
        "Rotation et luminosité automatiques (portables, tablettes).",
    ),
    exact(
        "SensrSvc",
        "Surveillance des capteurs",
        Devices,
        Safe,
        "Luminosité automatique (portables, tablettes).",
    ),
    exact(
        "SensorService",
        "Capteurs",
        Devices,
        Safe,
        "Rotation et luminosité automatiques (portables, tablettes).",
    ),
    exact(
        "WbioSrvc",
        "Biométrie Windows (Windows Hello)",
        Devices,
        Depends,
        "Plus de connexion par empreinte ou par visage.",
    ),
    exact(
        "SCardSvr",
        "Carte à puce",
        Devices,
        Safe,
        "Les lecteurs de cartes à puce ne répondent plus.",
    ),
    exact(
        "ScDeviceEnum",
        "Énumération des cartes à puce",
        Devices,
        Safe,
        "Cartes à puce seulement.",
    ),
    exact(
        "SCPolicySvc",
        "Retrait des cartes à puce",
        Devices,
        Safe,
        "Cartes à puce seulement.",
    ),
    exact(
        "CertPropSvc",
        "Propagation des certificats",
        Devices,
        Safe,
        "Cartes à puce seulement.",
    ),
    exact(
        "SEMgrSvc",
        "Paiements et NFC",
        Devices,
        Safe,
        "Plus de paiement sans contact depuis ce PC.",
    ),
    exact(
        "WalletService",
        "Portefeuille",
        Devices,
        Safe,
        "Portefeuille Windows, rarement utilisé.",
    ),
    exact(
        "NvContainerLocalSystem",
        "Conteneur NVIDIA (app NVIDIA)",
        Devices,
        Depends,
        "Plus d'overlay NVIDIA ni de capture instantanée (ShadowPlay).",
    ),
    // Synchronisation des comptes (services par utilisateur).
    prefix(
        "OneSyncSvc_",
        "Synchronisation (courrier, contacts, calendrier)",
        Sync,
        Safe,
        "Courrier, contacts et calendrier se synchroniseront après.",
    ),
    prefix(
        "PimIndexMaintenanceSvc_",
        "Indexation des contacts",
        Sync,
        Safe,
        "La recherche de contacts est plus lente.",
    ),
    prefix(
        "MessagingService_",
        "Messagerie texte",
        Sync,
        Safe,
        "Messages texte (Phone Link) seulement.",
    ),
    exact(
        "workfolderssvc",
        "Dossiers de travail",
        Sync,
        Safe,
        "Dossiers de travail d'entreprise seulement.",
    ),
    // Divers.
    prefix(
        "BcastDVRUserService_",
        "Enregistrement de la Xbox Game Bar",
        Other,
        Depends,
        "Plus de captures avec la Game Bar (Win+Alt+R).",
    ),
    exact(
        "MapsBroker",
        "Gestionnaire des cartes téléchargées",
        Other,
        Safe,
        "L'appli Cartes ne met plus ses cartes hors ligne à jour.",
    ),
    exact(
        "RetailDemo",
        "Démonstration en magasin",
        Other,
        Safe,
        "Ne sert qu'aux PC de démonstration en magasin.",
    ),
    exact(
        "wisvc",
        "Programme Windows Insider",
        Other,
        Safe,
        "Builds Insider seulement : le service reprendra après.",
    ),
    exact(
        "WpcMonSvc",
        "Contrôle parental",
        Other,
        Depends,
        "Les limites de Microsoft Family ne s'appliquent plus.",
    ),
    exact(
        "tzautoupdate",
        "Fuseau horaire automatique",
        Other,
        Safe,
        "Le fuseau horaire ne change plus de lui-même.",
    ),
    exact(
        "AJRouter",
        "Routeur AllJoyn",
        Other,
        Safe,
        "Objets connectés AllJoyn, rares.",
    ),
    exact(
        "MSiSCSI",
        "Initiateur iSCSI",
        Other,
        Safe,
        "Disques réseau iSCSI, rares.",
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

fn starts_with_ignore_case(name: &str, start: &str) -> bool {
    name.get(..start.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(start))
}

/// Vrai pour les services que GameBooster ne touche jamais.
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
    fn dependents_stop_before_their_dependencies() {
        let position = |name: &str| CATALOG.iter().position(|e| e.name == name).unwrap();
        assert!(position("Fax") < position("Spooler"));
        assert!(position("WMPNetworkSvc") < position("WSearch"));
        assert!(position("FDResPub") < position("fdPHost"));
        assert!(position("UmRdpService") < position("TermService"));
    }

    #[test]
    fn finds_by_name_case_and_prefix() {
        assert_eq!(
            find("wsearch").map(|(_, e)| e.label),
            Some("Windows Search (indexation)")
        );
        assert!(find("GoogleUpdaterService131.0.6778.86").is_some());
        assert_eq!(
            find("GoogleUpdaterInternalService131.0.6778.86").map(|(_, e)| e.label),
            Some("Mise à jour de Google Chrome (interne)")
        );
        assert!(find("OneSyncSvc_79fad").is_some());
        // Le modèle d'un service par utilisateur, sans numéro de session, ne tourne jamais.
        assert!(find("OneSyncSvc").is_none());
        assert!(find("GoogleUpdater").is_none());
        assert!(find("XblAuthManager").is_none());
        assert!(by_name("OneSyncSvc_").is_some());
        assert!(by_name("OneSyncSvc_79fad").is_none());
    }

    #[test]
    fn only_safe_services_are_checked_by_default() {
        assert!(Advice::Safe.checked_by_default());
        assert!(!Advice::Depends.checked_by_default());
        assert!(!Advice::Avoid.checked_by_default());
        assert!(CATALOG.len() >= 75, "{} services seulement", CATALOG.len());
    }
}
