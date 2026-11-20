//! Ce que GameBooster retient d'un lancement à l'autre, dans `%LOCALAPPDATA%\GameBooster` :
//!
//! - `etat.txt` : la session en cours (services arrêtés par GameBooster, mode jeu, RAM libérée).
//!   Si GameBooster est fermé, ou plante, le lancement suivant sait quoi relancer ;
//! - `choix.txt` : les cases cochées ou décochées à la main.
//!
//! Seuls les services du catalogue sont relus : ces fichiers, que tout programme de la session peut
//! modifier, ne peuvent rien faire arrêter ni relancer d'autre à GameBooster.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs, io};

use crate::catalog::{self, Entry};

/// Les services arrêtés par GameBooster, en attendant de les relancer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// Début, en secondes depuis le 1er janvier 1970 (UTC).
    pub since: u64,
    /// Heure locale du début, prête à afficher : « 14 h 32 ».
    pub since_text: String,
    /// « Tout arrêter » a été cliqué : les services cochés que Windows démarre sont arrêtés aussi.
    pub game_mode: bool,
    /// Arrêtés par GameBooster, dans l'ordre de leur arrêt.
    pub stopped: Vec<String>,
    /// RAM libérée par ces arrêts, en octets.
    pub freed: u64,
}

impl Session {
    pub fn new(since: u64, since_text: String) -> Session {
        Session {
            since,
            since_text,
            game_mode: false,
            stopped: Vec::new(),
            freed: 0,
        }
    }

    fn to_text(&self) -> String {
        let mut text = String::from("# GameBooster : services arrêtés, à relancer.\n");
        let _ = writeln!(text, "depuis={}", self.since);
        let _ = writeln!(text, "heure={}", self.since_text);
        let _ = writeln!(text, "jeu={}", u8::from(self.game_mode));
        let _ = writeln!(text, "libere={}", self.freed);
        for name in &self.stopped {
            let _ = writeln!(text, "service={name}");
        }
        text
    }

    fn parse(text: &str) -> Option<Session> {
        let mut since = None;
        let mut session = Session::new(0, String::new());
        for (key, value) in pairs(text) {
            match key {
                "depuis" => since = value.parse().ok(),
                "heure" => session.since_text = value.to_string(),
                "jeu" => session.game_mode = value == "1",
                "libere" => session.freed = value.parse().unwrap_or(0),
                "service" if catalog::find(value).is_some() => {
                    session.stopped.push(value.to_string())
                }
                _ => {}
            }
        }
        session.since = since?;
        Some(session)
    }
}

/// Les cases changées à la main : nom de l'entrée du catalogue → cochée. Les autres suivent le
/// conseil (cochées si « sans risque »).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choices(BTreeMap<String, bool>);

impl Choices {
    pub fn is_checked(&self, entry: &Entry) -> bool {
        self.0
            .get(entry.name)
            .copied()
            .unwrap_or_else(|| entry.advice.checked_by_default())
    }

    pub fn set(&mut self, entry: &Entry, checked: bool) {
        if checked == entry.advice.checked_by_default() {
            self.0.remove(entry.name);
        } else {
            self.0.insert(entry.name.to_string(), checked);
        }
    }

    fn to_text(&self) -> String {
        let mut text = String::from("# GameBooster : cases changées à la main.\n");
        for (name, checked) in &self.0 {
            let key = if *checked { "coche" } else { "decoche" };
            let _ = writeln!(text, "{key}={name}");
        }
        text
    }

    fn parse(text: &str) -> Choices {
        let mut choices = Choices::default();
        for (key, value) in pairs(text) {
            let checked = match key {
                "coche" => true,
                "decoche" => false,
                _ => continue,
            };
            if let Some(entry) = catalog::by_name(value) {
                choices.set(entry, checked);
            }
        }
        choices
    }
}

/// Les lignes `clé=valeur` d'un fichier, espaces en trop retirés.
fn pairs(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines().filter_map(|line| {
        let (key, value) = line.split_once('=')?;
        Some((key.trim(), value.trim()))
    })
}

fn dir() -> Option<PathBuf> {
    Some(PathBuf::from(env::var_os("LOCALAPPDATA")?).join("GameBooster"))
}

/// Écrit dans un fichier temporaire, puis le renomme : le fichier n'est jamais à moitié écrit.
fn write(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

fn no_dir() -> io::Error {
    io::Error::other("LOCALAPPDATA introuvable")
}

/// La session laissée par un lancement précédent, s'il y en a une.
pub fn load() -> Option<Session> {
    Session::parse(&fs::read_to_string(dir()?.join("etat.txt")).ok()?)
}

pub fn save(session: &Session) -> io::Result<()> {
    write(
        &dir().ok_or_else(no_dir)?.join("etat.txt"),
        &session.to_text(),
    )
}

/// Oublie la session : ses services ont été relancés.
pub fn clear() {
    if let Some(dir) = dir() {
        let _ = fs::remove_file(dir.join("etat.txt"));
    }
}

pub fn load_choices() -> Choices {
    dir()
        .and_then(|dir| fs::read_to_string(dir.join("choix.txt")).ok())
        .map(|text| Choices::parse(&text))
        .unwrap_or_default()
}

pub fn save_choices(choices: &Choices) -> io::Result<()> {
    write(
        &dir().ok_or_else(no_dir)?.join("choix.txt"),
        &choices.to_text(),
    )
}

/// Maintenant, en secondes depuis le 1er janvier 1970 (UTC).
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Vrai si Windows a redémarré depuis le début de la session (`since`). `uptime` : secondes depuis
/// le démarrage de Windows. Après un arrêt avec le démarrage rapide de Windows (arrêt hybride), ce
/// compteur continue, et les services gardent leur état : la session est alors toujours en cours.
pub fn ended_by_reboot(since: u64, now: u64, uptime: u64) -> bool {
    let boot = now.saturating_sub(uptime);
    // Une minute de marge : l'horloge et le compteur ne sont pas lus au même instant.
    boot > since + 60
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_round_trip() {
        let session = Session {
            since: 1_763_650_000,
            since_text: "14 h 32".into(),
            game_mode: true,
            stopped: vec!["wuauserv".into(), "OneSyncSvc_79fad".into()],
            freed: 123_456_789,
        };
        assert_eq!(Session::parse(&session.to_text()), Some(session));
    }

    #[test]
    fn session_keeps_only_catalog_services() {
        let text = "depuis=5\nheure=9 h 05\nservice=wuauserv\nservice=WinDefend\n\
                    service=Malveillant\nservice=GoogleUpdaterService1.2.3\nautre=1\nsans égal\n";
        let session = Session::parse(text).unwrap();
        assert_eq!(session.since, 5);
        assert!(!session.game_mode);
        assert_eq!(session.stopped, ["wuauserv", "GoogleUpdaterService1.2.3"]);
        assert_eq!(Session::parse("service=wuauserv\n"), None);
    }

    #[test]
    fn choices_store_only_changes() {
        let update = catalog::by_name("wuauserv").unwrap();
        let printer = catalog::by_name("Spooler").unwrap();
        let mut choices = Choices::default();
        assert!(choices.is_checked(update));
        assert!(!choices.is_checked(printer));

        choices.set(update, false);
        choices.set(printer, true);
        assert!(!choices.is_checked(update));
        assert!(choices.is_checked(printer));
        assert_eq!(Choices::parse(&choices.to_text()), choices);

        // Revenir au conseil efface le choix.
        choices.set(update, true);
        choices.set(printer, false);
        assert_eq!(choices, Choices::default());

        // Seules les entrées du catalogue sont relues.
        let read = Choices::parse("coche=Spooler\ncoche=WinDefend\ndecoche=Inconnu\n");
        assert!(read.is_checked(printer));
        assert_eq!(read.0.len(), 1);
    }

    #[test]
    fn detects_reboot_since_session() {
        // Session à 1000 s ; Windows redémarré à 9900 s.
        assert!(ended_by_reboot(1000, 10_000, 100));
        // Windows démarré à 500 s, avant la session : toujours en cours.
        assert!(!ended_by_reboot(1000, 10_000, 9_500));
        // Session ouverte 30 s après le démarrage : toujours en cours.
        assert!(!ended_by_reboot(1000, 1_030, 60));
    }
}
