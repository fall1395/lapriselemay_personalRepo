//! Le contenu de la liste : les services du catalogue présents sur ce PC, leur état et la RAM
//! qu'ils occupent. Du calcul seulement, sur ce que Windows a renvoyé : testable sans Windows.

use std::collections::{HashMap, HashSet};

use crate::catalog::{self, Entry};
use crate::memory;
use crate::services::{Info, State};
use crate::state::Choices;

/// La RAM occupée par un service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ram {
    /// Arrêté, ou mémoire illisible.
    Unknown,
    /// Seul dans son processus : la RAM privée de ce processus, libérée à son arrêt.
    Own(u64),
    /// Son processus héberge d'autres services en cours (`n` en tout, lui compris) : sa RAM
    /// n'est libérée que s'ils s'arrêtent tous.
    Shared(usize),
}

impl Ram {
    pub fn bytes(self) -> u64 {
        match self {
            Ram::Own(bytes) => bytes,
            _ => 0,
        }
    }
}

/// Une ligne de la liste.
#[derive(Clone, Debug)]
pub struct Row {
    /// Nom du service, celui de `sc query`.
    pub name: String,
    /// Position de son entrée dans le catalogue : l'ordre d'arrêt.
    pub index: usize,
    pub entry: &'static Entry,
    /// Nom affiché par Windows, dans sa langue.
    pub display_name: String,
    pub state: State,
    pub ram: Ram,
    /// Arrêté par « Tout arrêter ».
    pub checked: bool,
}

impl Row {
    /// Texte de la colonne RAM.
    pub fn ram_text(&self) -> String {
        match self.ram {
            Ram::Own(bytes) => memory::format(bytes),
            Ram::Shared(_) => "partagée".into(),
            Ram::Unknown if self.state.is_running() => "?".into(),
            Ram::Unknown => "—".into(),
        }
    }
}

/// Les services du catalogue installés sur ce PC, dans l'ordre du catalogue.
pub fn rows(infos: &[Info], ram: &HashMap<u32, u64>, choices: &Choices) -> Vec<Row> {
    let mut per_process: HashMap<u32, usize> = HashMap::new();
    for info in infos.iter().filter(|i| i.state.is_running() && i.pid != 0) {
        *per_process.entry(info.pid).or_default() += 1;
    }
    let mut rows: Vec<Row> = infos
        .iter()
        .filter_map(|info| {
            let (index, entry) = catalog::find(&info.name)?;
            let in_process = per_process.get(&info.pid).copied().unwrap_or(0);
            let ram = if !info.state.is_running() || info.pid == 0 {
                Ram::Unknown
            } else if in_process > 1 {
                Ram::Shared(in_process)
            } else {
                ram.get(&info.pid)
                    .map_or(Ram::Unknown, |bytes| Ram::Own(*bytes))
            };
            Some(Row {
                name: info.name.clone(),
                index,
                entry,
                display_name: info.display_name.clone(),
                state: info.state,
                ram,
                checked: choices.is_checked(entry),
            })
        })
        .collect();
    rows.sort_by(|a, b| a.index.cmp(&b.index).then_with(|| a.name.cmp(&b.name)));
    rows
}

/// La RAM que libérerait l'arrêt des services `chosen`, parmi ceux qui tournent : celle des
/// processus dont tous les services en cours seraient arrêtés.
pub fn freeable(infos: &[Info], ram: &HashMap<u32, u64>, chosen: &HashSet<&str>) -> u64 {
    let running = |i: &&Info| i.state.is_running() && i.pid != 0;
    let pids: HashSet<u32> = infos
        .iter()
        .filter(running)
        .filter(|i| chosen.contains(i.name.as_str()))
        .map(|i| i.pid)
        .collect();
    pids.into_iter()
        .filter(|pid| {
            infos
                .iter()
                .filter(running)
                .filter(|i| i.pid == *pid)
                .all(|i| chosen.contains(i.name.as_str()))
        })
        .map(|pid| ram.get(&pid).copied().unwrap_or(0))
        .sum()
}

/// La RAM vraiment libérée par l'arrêt des services `stopped` : celle de leurs processus qui se
/// sont fermés depuis. `pids` : processus de chaque service avant l'arrêt ; `before` et `after` :
/// RAM privée par PID, avant et après.
pub fn freed(
    stopped: &[String],
    pids: &HashMap<String, u32>,
    before: &HashMap<u32, u64>,
    after: &HashMap<u32, u64>,
) -> u64 {
    let closed: HashSet<u32> = stopped
        .iter()
        .filter_map(|name| pids.get(name).copied())
        .filter(|pid| *pid != 0 && !after.contains_key(pid))
        .collect();
    closed
        .iter()
        .map(|pid| before.get(pid).copied().unwrap_or(0))
        .sum()
}

/// Les colonnes de la liste, dans l'ordre : aussi les clés de tri.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Service,
    Category,
    Advice,
    State,
    Ram,
}

impl Column {
    pub const ALL: [Column; 5] = [
        Column::Service,
        Column::Category,
        Column::Advice,
        Column::State,
        Column::Ram,
    ];
}

/// Trie la liste. Sens naturel : par nom, catégorie et conseil ; ceux qui tournent d'abord ; la
/// plus grosse RAM d'abord. `reverse` l'inverse.
pub fn sort(rows: &mut [Row], column: Column, reverse: bool) {
    rows.sort_by(|a, b| {
        let order = match column {
            Column::Service => a
                .entry
                .label
                .to_lowercase()
                .cmp(&b.entry.label.to_lowercase()),
            // L'ordre du catalogue regroupe déjà les catégories.
            Column::Category => a.index.cmp(&b.index),
            Column::Advice => a.entry.advice.cmp(&b.entry.advice),
            Column::State => b.state.is_running().cmp(&a.state.is_running()),
            Column::Ram => b.ram.bytes().cmp(&a.ram.bytes()),
        };
        let order = order
            .then_with(|| a.index.cmp(&b.index))
            .then_with(|| a.name.cmp(&b.name));
        if reverse {
            order.reverse()
        } else {
            order
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str, state: State, pid: u32) -> Info {
        Info {
            name: name.into(),
            display_name: format!("{name} (Windows)"),
            state,
            pid,
        }
    }

    /// Windows Update seul dans son processus ; Windows Search et la télémétrie partagent le leur
    /// avec un service hors catalogue ; le spouleur est arrêté.
    fn sample() -> (Vec<Info>, HashMap<u32, u64>) {
        let infos = vec![
            info("Spooler", State::Stopped, 0),
            info("WSearch", State::Running, 20),
            info("DiagTrack", State::Running, 20),
            info("Audiosrv", State::Running, 30),
            info("wuauserv", State::Running, 10),
            info("PcaSvc", State::Running, 40),
            info("Dnscache", State::Running, 40),
        ];
        let ram = HashMap::from([(10, 30 << 20), (20, 50 << 20), (30, 9 << 20), (40, 7 << 20)]);
        (infos, ram)
    }

    #[test]
    fn rows_follow_catalog_with_ram() {
        let (infos, ram) = sample();
        let rows = rows(&infos, &ram, &Choices::default());
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        // Audiosrv et Dnscache ne sont pas dans le catalogue ; ordre du catalogue.
        assert_eq!(
            names,
            ["wuauserv", "WSearch", "DiagTrack", "PcaSvc", "Spooler"]
        );
        assert_eq!(rows[0].ram, Ram::Own(30 << 20));
        assert_eq!(rows[1].ram, Ram::Shared(2));
        assert_eq!(rows[3].ram, Ram::Shared(2));
        assert_eq!(rows[4].ram, Ram::Unknown);
        assert_eq!(rows[0].ram_text(), "30 Mo");
        assert_eq!(rows[1].ram_text(), "partagée");
        assert_eq!(rows[4].ram_text(), "—");
        // Cochés par défaut : les services sans risque ; pas le spouleur.
        assert!(rows[0].checked && !rows[4].checked);
    }

    #[test]
    fn freeable_counts_only_processes_left_empty() {
        let (infos, ram) = sample();
        let chosen = HashSet::from(["wuauserv", "WSearch"]);
        // DiagTrack reste dans le processus de Windows Search : seul wuauserv libère sa RAM.
        assert_eq!(freeable(&infos, &ram, &chosen), 30 << 20);
        let chosen = HashSet::from(["wuauserv", "WSearch", "DiagTrack", "PcaSvc", "Spooler"]);
        // Le processus de PcaSvc garde Dnscache : il ne se ferme pas.
        assert_eq!(freeable(&infos, &ram, &chosen), 80 << 20);
    }

    #[test]
    fn freed_counts_closed_processes_once() {
        let pids = HashMap::from([
            ("wuauserv".to_string(), 10),
            ("WSearch".to_string(), 20),
            ("DiagTrack".to_string(), 20),
            ("PcaSvc".to_string(), 40),
        ]);
        let before = HashMap::from([(10, 30), (20, 50), (40, 7)]);
        let after = HashMap::from([(40, 7)]);
        let stopped: Vec<String> = ["wuauserv", "WSearch", "DiagTrack", "PcaSvc"]
            .map(String::from)
            .to_vec();
        assert_eq!(freed(&stopped, &pids, &before, &after), 80);
    }

    #[test]
    fn sorts_by_ram_then_reverses() {
        let (infos, ram) = sample();
        let mut rows = rows(&infos, &ram, &Choices::default());
        sort(&mut rows, Column::Ram, false);
        assert_eq!(rows[0].name, "wuauserv");
        sort(&mut rows, Column::State, false);
        assert_eq!(rows.last().unwrap().name, "Spooler");
        sort(&mut rows, Column::Category, true);
        assert_eq!(rows[0].name, "Spooler");
    }
}
