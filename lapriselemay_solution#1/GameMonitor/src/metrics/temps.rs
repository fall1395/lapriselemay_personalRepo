//! Température CPU, lue dans un thread à part.
//!
//! Windows n'expose pas la température des cœurs sans pilote noyau. On essaie donc :
//! 1. le pilote PawnIO s'il est installé (processeurs Intel et AMD Ryzen) : lecture précise et directe ;
//! 2. LibreHardwareMonitor s'il tourne (lecture précise, il publie ses capteurs dans WMI) ;
//! 3. la zone thermique ACPI de la carte mère (admin requis, pas disponible partout).
//!
//! Une requête WMI attend la réponse du fournisseur sans limite de temps : si celui de
//! LibreHardwareMonitor ne répond plus, elle ne revient pas. Les requêtes se font donc dans
//! un thread à part, que la collecte n'attend qu'un court instant avant de reprendre
//! la dernière valeur connue.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use wmi::{Variant, WMIConnection};

use super::pawnio::{PawnIo, Reading as PawnReading, Unavailable};

type Row = HashMap<String, Variant>;

const RETRY: Duration = Duration::from_secs(30);
/// Attente maximale de la réponse, à chaque mesure. Une requête prend d'habitude quelques dizaines de ms.
const WAIT: Duration = Duration::from_millis(250);
/// Au-delà de cet âge, une température que WMI tarde à renouveler n'est plus affichée.
const MAX_AGE: Duration = Duration::from_secs(5);
/// Mesure PawnIO manquée (bus PCI occupé par un autre outil…) : la précédente reste affichée jusque-là.
const SKIPPED_GRACE: Duration = Duration::from_secs(3);

/// Température CPU, et d'où elle vient.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CpuTemp {
    pub celsius: Option<f32>,
    /// « PawnIO », « LHM » ou « ACPI » ; vide sans température.
    pub source: &'static str,
    /// Processeur Intel ou Ryzen sans PawnIO : l'installer donnerait sa température.
    pub pawnio_missing: bool,
}

/// Lecture, à son heure.
#[derive(Clone, Copy)]
struct Reading {
    temp: CpuTemp,
    at: Instant,
}

/// Côté collecte : une demande de lecture à chaque mesure, et la dernière lecture reçue.
pub struct Temps {
    requests: Sender<()>,
    readings: Receiver<Reading>,
    /// Une demande est partie et sa réponse n'est pas encore arrivée.
    pending: bool,
    last: Option<Reading>,
}

impl Temps {
    /// Lance le thread de lecture. Il dort entre deux demandes : rien n'est lu pendant une pause.
    pub fn spawn() -> Self {
        Self::start(|| {
            let mut sources = Sources::new();
            move || sources.cpu()
        })
    }

    /// `reader` est appelé dans le thread de lecture, et la fonction qu'il renvoie à chaque demande :
    /// les connexions WMI, qui ne sont pas `Send`, naissent ainsi dans le thread qui s'en sert.
    fn start<F, R>(reader: F) -> Self
    where
        F: FnOnce() -> R + Send + 'static,
        R: FnMut() -> CpuTemp,
    {
        let (requests, incoming) = mpsc::channel();
        let (outgoing, readings) = mpsc::channel();
        // Le thread s'arrête avec la collecte (`Temps` libéré), ou reste bloqué dans WMI jusqu'à
        // la fin du processus sans retenir personne. S'il ne démarre pas, la température reste « n/d ».
        let _ = std::thread::Builder::new().name("temperature".into()).spawn(move || {
            let mut read = reader();
            while incoming.recv().is_ok() {
                let temp = read();
                if outgoing.send(Reading { temp, at: Instant::now() }).is_err() {
                    break;
                }
            }
        });
        Self { requests, readings, pending: false, last: None }
    }

    /// Dernière température lue. Si le thread de lecture tarde, la précédente, tant qu'elle n'a pas
    /// plus de [`MAX_AGE`].
    pub fn cpu(&mut self) -> CpuTemp {
        let reading = if self.pending {
            // Toujours pas de réponse à la demande précédente : le thread est lent ou bloqué.
            // On ne l'attend plus, et on ne lui envoie rien d'autre en attendant.
            self.readings.try_recv().ok()
        } else if self.requests.send(()).is_ok() {
            self.pending = true;
            self.readings.recv_timeout(WAIT).ok()
        } else {
            None
        };
        if let Some(r) = reading {
            self.pending = false;
            self.last = Some(r);
        }
        self.current(Instant::now())
    }

    fn current(&self, now: Instant) -> CpuTemp {
        match self.last {
            Some(r) if now.saturating_duration_since(r.at) <= MAX_AGE => r.temp,
            _ => CpuTemp::default(),
        }
    }
}

/// Côté thread de lecture : le pilote PawnIO et, seulement quand il ne sert pas, les connexions WMI
/// aux deux autres sources.
struct Sources {
    pawnio: Option<PawnIo>,
    /// Dernière température lue par PawnIO, et quand.
    last_pawnio: Option<(f32, Instant)>,
    /// PawnIO n'est pas installé, sur un processeur qu'il permettrait de lire.
    pawnio_missing: bool,
    lhm: Option<WMIConnection>,
    acpi: Option<WMIConnection>,
    next_retry: Instant,
}

fn as_f64(v: Option<&Variant>) -> Option<f64> {
    match v? {
        Variant::R4(x) => Some(*x as f64),
        Variant::R8(x) => Some(*x),
        Variant::UI4(x) => Some(*x as f64),
        Variant::I4(x) => Some(*x as f64),
        _ => None,
    }
}

fn as_str(v: Option<&Variant>) -> &str {
    match v {
        Some(Variant::String(s)) => s,
        _ => "",
    }
}

impl Sources {
    fn new() -> Self {
        let mut s = Self {
            pawnio: None,
            last_pawnio: None,
            pawnio_missing: false,
            lhm: None,
            acpi: None,
            next_retry: Instant::now(),
        };
        s.connect();
        s
    }

    fn connect(&mut self) {
        if self.pawnio.is_none() {
            match PawnIo::open() {
                Ok(p) => {
                    self.pawnio = Some(p);
                    self.pawnio_missing = false;
                }
                Err(why) => self.pawnio_missing = why == Unavailable::Missing,
            }
        }
        if self.pawnio.is_some() {
            // PawnIO suffit : ni connexion WMI ni COM tant qu'il répond.
            self.lhm = None;
            self.acpi = None;
        } else {
            if self.lhm.is_none() {
                self.lhm = WMIConnection::with_namespace_path(r"ROOT\LibreHardwareMonitor").ok();
            }
            if self.acpi.is_none() {
                self.acpi = WMIConnection::with_namespace_path(r"ROOT\WMI").ok();
            }
        }
        self.next_retry = Instant::now() + RETRY;
    }

    fn cpu(&mut self) -> CpuTemp {
        // Réessai périodique tant que PawnIO ne sert pas, même quand une autre source répond :
        // PawnIO a pu être installé, ou LibreHardwareMonitor lancé, après GameMonitor.
        if self.pawnio.is_none() && Instant::now() >= self.next_retry {
            self.connect();
        }
        let (celsius, source) = if let Some(t) = self.read_pawnio() {
            (Some(t), "PawnIO")
        } else if let Some(t) = self.read_lhm() {
            (Some(t), "LHM")
        } else if let Some(t) = self.read_acpi() {
            (Some(t), "ACPI")
        } else {
            (None, "")
        };
        CpuTemp { celsius, source, pawnio_missing: self.pawnio_missing }
    }

    fn read_pawnio(&mut self) -> Option<f32> {
        match self.pawnio.as_ref()?.read() {
            PawnReading::Celsius(t) => {
                self.last_pawnio = Some((t, Instant::now()));
                Some(t)
            }
            PawnReading::Skipped => {
                self.last_pawnio.filter(|(_, at)| at.elapsed() < SKIPPED_GRACE).map(|(t, _)| t)
            }
            PawnReading::Failed => {
                // Pilote arrêté ou désinstallé entre-temps : dès la mesure suivante, on le rouvre ou,
                // à défaut, on se connecte aux autres sources.
                self.pawnio = None;
                self.next_retry = Instant::now();
                None
            }
        }
    }

    fn read_lhm(&mut self) -> Option<f32> {
        let con = self.lhm.as_ref()?;
        let rows: Vec<Row> = match con.raw_query(
            "SELECT Name, Value, Identifier FROM Sensor WHERE SensorType='Temperature'",
        ) {
            Ok(r) => r,
            Err(_) => {
                self.lhm = None;
                return None;
            }
        };
        let cpu: Vec<&Row> = rows
            .iter()
            .filter(|r| as_str(r.get("Identifier")).contains("cpu"))
            .collect();
        // « CPU Package » en priorité, sinon le capteur CPU le plus chaud.
        cpu.iter()
            .find(|r| as_str(r.get("Name")).contains("Package"))
            .and_then(|r| as_f64(r.get("Value")))
            .or_else(|| {
                cpu.iter()
                    .filter_map(|r| as_f64(r.get("Value")))
                    .reduce(f64::max)
            })
            .map(|t| t as f32)
    }

    fn read_acpi(&mut self) -> Option<f32> {
        let con = self.acpi.as_ref()?;
        let rows: Vec<Row> =
            match con.raw_query("SELECT CurrentTemperature FROM MSAcpi_ThermalZoneTemperature") {
                Ok(r) => r,
                Err(_) => {
                    // Pas disponible sur ce PC (ou pas admin) : inutile de réessayer à chaque seconde.
                    self.acpi = None;
                    return None;
                }
            };
        rows.iter()
            .filter_map(|r| as_f64(r.get("CurrentTemperature")))
            .map(|dk| dk / 10.0 - 273.15) // dixièmes de kelvin → °C
            .filter(|c| (1.0..125.0).contains(c))
            .reduce(f64::max)
            .map(|t| t as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(celsius: f32, source: &'static str) -> CpuTemp {
        CpuTemp { celsius: Some(celsius), source, pawnio_missing: false }
    }

    /// Appelle `cpu()` comme la collecte, une fois toutes les 10 ms, jusqu'à obtenir `expected`.
    fn wait_for(temps: &mut Temps, expected: CpuTemp) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while temps.cpu() != expected {
            assert!(Instant::now() < deadline, "toujours pas de lecture {expected:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_stuck_reader_does_not_hold_up_the_collection() {
        // Lecture qui ne revient pas, comme une requête WMI dont le fournisseur ne répond plus.
        let (unblock, blocked) = mpsc::channel::<()>();
        let mut temps = Temps::start(move || {
            move || {
                let _ = blocked.recv();
                reading(55.0, "LHM")
            }
        });

        // La collecte n'attend la réponse qu'une fois, et pas plus de WAIT…
        let start = Instant::now();
        assert_eq!(temps.cpu(), CpuTemp::default());
        assert!(start.elapsed() < 4 * WAIT, "{:?}", start.elapsed());
        // … puis plus du tout tant que le thread est bloqué.
        let start = Instant::now();
        assert_eq!(temps.cpu(), CpuTemp::default());
        assert!(start.elapsed() < WAIT, "{:?}", start.elapsed());

        // La réponse tardive est reprise dès qu'elle arrive.
        unblock.send(()).unwrap();
        wait_for(&mut temps, reading(55.0, "LHM"));
    }

    #[test]
    fn the_last_reading_is_kept_up_to_max_age() {
        let mut temps = Temps::start(|| || reading(61.0, "ACPI"));
        wait_for(&mut temps, reading(61.0, "ACPI"));
        let at = temps.last.unwrap().at;
        assert_eq!(temps.current(at + MAX_AGE), reading(61.0, "ACPI"));
        // Au-delà, une température qui ne se renouvelle plus n'est plus affichée.
        assert_eq!(temps.current(at + MAX_AGE + Duration::from_millis(1)), CpuTemp::default());
    }
}
