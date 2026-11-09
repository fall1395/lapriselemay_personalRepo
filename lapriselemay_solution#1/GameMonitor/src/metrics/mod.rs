//! Collecte de toutes les mesures, dans un thread séparé, une fois par seconde.

mod fps;
mod gpu;
mod pawnio;
mod pdh;
mod temps;

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sysinfo::{Disks, Networks, Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

pub use fps::FpsInfo;
pub use gpu::GpuInfo;

#[derive(Clone)]
pub struct Drive {
    pub mount: String,
    pub free: u64,
    pub total: u64,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    /// Heure de la mesure. `None` tant qu'il n'y en a pas : au lancement, et après une pause.
    pub taken: Option<Instant>,
    pub cpu_name: String,
    pub cpu_usage: f32,
    pub cpu_cores: Vec<f32>,
    pub cpu_mhz: Option<f64>,
    pub cpu_temp: Option<f32>,
    pub cpu_temp_source: &'static str,
    /// Processeur Intel ou Ryzen sans PawnIO : l'installer donnerait la température CPU.
    pub pawnio_missing: bool,
    pub ram_used: u64,
    pub ram_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub gpu: GpuInfo,
    pub disk_read: Option<f64>,
    pub disk_write: Option<f64>,
    pub disk_active: Option<f64>,
    pub drives: Vec<Drive>,
    pub net_down: f64,
    pub net_up: f64,
    pub fps_available: bool,
    pub fps: Option<FpsInfo>,
    pub foreground: String,
    pub self_cpu: f32,
    pub self_mem: u64,
    pub uptime: u64,
}

pub type SharedSnapshot = Arc<Mutex<Snapshot>>;

const PERIOD: Duration = Duration::from_secs(1);

enum Command {
    Pause,
    Resume,
    Quit,
}

/// Pause et reprise de la collecte, depuis l'overlay.
#[derive(Clone)]
pub struct Control(Sender<Command>);

impl Control {
    /// Overlay masqué : plus aucune mesure, session ETW comprise.
    pub fn pause(&self) {
        let _ = self.0.send(Command::Pause);
    }

    pub fn resume(&self) {
        let _ = self.0.send(Command::Resume);
    }
}

/// Le thread de collecte, à arrêter avec [`Worker::stop`] avant de quitter.
pub struct Worker {
    control: Control,
    done: Receiver<()>,
}

impl Worker {
    pub fn control(&self) -> Control {
        self.control.clone()
    }

    /// Arrête la collecte et attend (au plus `timeout`) que le thread ait libéré ses ressources.
    /// Indispensable : à la sortie du processus, les autres threads sont tués sans rien libérer,
    /// et la session ETW resterait ouverte.
    pub fn stop(self, timeout: Duration) {
        let _ = self.control.0.send(Command::Quit);
        let _ = self.done.recv_timeout(timeout);
        // Filet de sécurité si le thread est bloqué (pilote qui ne répond pas, par exemple).
        fps::stop_session();
    }
}

/// À appeler quand Windows ferme la session : le processus sera tué sans repasser par `main`.
pub fn emergency_stop() {
    fps::stop_session();
}

/// Lance le thread de collecte.
pub fn spawn(shared: SharedSnapshot) -> Worker {
    let (control, commands) = mpsc::channel();
    let (finished, done) = mpsc::channel();
    std::thread::spawn(move || {
        run(&shared, &commands);
        let _ = finished.send(());
    });
    Worker { control: Control(control), done }
}

fn publish(shared: &SharedSnapshot, snap: Snapshot) {
    if let Ok(mut s) = shared.lock() {
        *s = snap;
    }
}

fn run(shared: &SharedSnapshot, commands: &Receiver<Command>) {
    let mut collector = Collector::new();
    let mut paused = false;
    let mut next = Instant::now() + PERIOD;
    loop {
        let command = if paused {
            // En pause, le thread dort jusqu'à la prochaine commande : aucune consommation.
            commands.recv().unwrap_or(Command::Quit)
        } else {
            match commands.recv_timeout(next.saturating_duration_since(Instant::now())) {
                Ok(command) => command,
                Err(RecvTimeoutError::Disconnected) => Command::Quit,
                Err(RecvTimeoutError::Timeout) => {
                    publish(shared, collector.collect());
                    // Cadence régulière, mais au moins une demi-période entre deux mesures
                    // si l'une a pris du retard (les débits seraient sinon calculés sur un instant).
                    next = (next + PERIOD).max(Instant::now() + PERIOD / 2);
                    continue;
                }
            }
        };
        match command {
            Command::Quit => break,
            Command::Pause if !paused => {
                paused = true;
                collector.pause();
                // À la réapparition, on n'affichera pas des valeurs d'avant la pause.
                publish(shared, Snapshot::default());
            }
            Command::Resume if paused => {
                paused = false;
                collector.resume();
                next = Instant::now() + PERIOD;
            }
            Command::Pause | Command::Resume => {}
        }
    }
    // `collector` est libéré ici : session ETW fermée, requête PDH et NVML aussi.
}

struct Collector {
    sys: System,
    networks: Option<Networks>,
    disks: Option<Disks>,
    last_disks: Instant,
    last_tick: Instant,
    pdh: Option<pdh::Pdh>,
    counters: HashMap<&'static str, pdh::Counter>,
    gpu: gpu::Gpu,
    temps: temps::Temps,
    fps: fps::FpsTracker,
    self_pid: Pid,
    cpu_name: String,
}

const PDH_COUNTERS: &[(&str, &str)] = &[
    ("perf", r"\Processor Information(_Total)\% Processor Performance"),
    ("freq", r"\Processor Information(_Total)\Processor Frequency"),
    ("read", r"\PhysicalDisk(_Total)\Disk Read Bytes/sec"),
    ("write", r"\PhysicalDisk(_Total)\Disk Write Bytes/sec"),
    ("idle", r"\PhysicalDisk(_Total)\% Idle Time"),
];

fn foreground_pid() -> u32 {
    let mut pid = 0u32;
    unsafe {
        let hwnd = GetForegroundWindow();
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    pid
}

impl Collector {
    fn new() -> Self {
        let mut sys = System::new().expect("sysinfo");
        sys.refresh_cpu_all();
        let cpu_name = sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .unwrap_or_default();

        let pdh = pdh::Pdh::new();
        let mut counters = HashMap::new();
        if let Some(p) = &pdh {
            for (key, path) in PDH_COUNTERS {
                if let Some(c) = p.add(path) {
                    counters.insert(*key, c);
                }
            }
        }
        let gpu = gpu::Gpu::new(pdh.as_ref());

        let mut collector = Self {
            sys,
            networks: Networks::new_with_refreshed_list().ok(),
            disks: Disks::new_with_refreshed_list().ok(),
            last_disks: Instant::now(),
            last_tick: Instant::now(),
            pdh,
            counters,
            gpu,
            temps: temps::Temps::spawn(),
            fps: fps::FpsTracker::start(),
            self_pid: Pid::from_u32(std::process::id()),
            cpu_name,
        };
        collector.prime();
        collector
    }

    /// Mesure de référence : la collecte suivante portera sur l'intervalle qui commence ici
    /// (charge CPU, débits), et non sur toute la durée d'une pause.
    fn prime(&mut self) {
        self.sys.refresh_cpu_usage();
        if let Some(nets) = &mut self.networks {
            nets.refresh(true);
        }
        if let Some(p) = &self.pdh {
            p.collect();
        }
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.self_pid]),
            true,
            ProcessRefreshKind::nothing().with_cpu(),
        );
        self.last_tick = Instant::now();
    }

    fn pause(&mut self) {
        self.fps.pause();
    }

    fn resume(&mut self) {
        self.fps.resume();
        self.prime();
    }

    fn counter(&self, key: &str) -> Option<f64> {
        self.counters.get(key)?.value()
    }

    fn collect(&mut self) -> Snapshot {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_tick).as_secs_f64().max(0.001);
        self.last_tick = now;

        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        if let Some(p) = &self.pdh {
            p.collect();
        }

        let mut s = Snapshot {
            taken: Some(now),
            cpu_name: self.cpu_name.clone(),
            cpu_usage: self.sys.global_cpu_usage(),
            cpu_cores: self.sys.cpus().iter().map(|c| c.usage()).collect(),
            ram_used: self.sys.used_memory(),
            ram_total: self.sys.total_memory(),
            swap_used: self.sys.used_swap(),
            swap_total: self.sys.total_swap(),
            uptime: System::uptime().unwrap_or(0),
            ..Default::default()
        };

        // Fréquence réelle = fréquence de base × % de performance (inclut le turbo).
        if let (Some(base), Some(perf)) = (self.counter("freq"), self.counter("perf")) {
            s.cpu_mhz = Some(base * perf / 100.0);
        }
        s.disk_read = self.counter("read");
        s.disk_write = self.counter("write");
        s.disk_active = self.counter("idle").map(|idle| (100.0 - idle).clamp(0.0, 100.0));

        let temp = self.temps.cpu();
        s.cpu_temp = temp.celsius;
        s.cpu_temp_source = temp.source;
        s.pawnio_missing = temp.pawnio_missing;

        s.gpu = self.gpu.read();

        if let Some(nets) = &mut self.networks {
            nets.refresh(true);
            let (mut down, mut up) = (0u64, 0u64);
            for (name, data) in nets.iter() {
                if name.to_string_lossy().contains("Loopback") {
                    continue;
                }
                down += data.received();
                up += data.transmitted();
            }
            s.net_down = down as f64 / elapsed;
            s.net_up = up as f64 / elapsed;
        }

        if let Some(disks) = &mut self.disks {
            if self.last_disks.elapsed() > Duration::from_secs(10) {
                disks.refresh(true);
                self.last_disks = Instant::now();
            }
            s.drives = disks
                .iter()
                .map(|d| Drive {
                    mount: d.mount_point().to_string_lossy().trim_end_matches('\\').to_string(),
                    free: d.available_space(),
                    total: d.total_space(),
                })
                .collect();
        }

        // FPS du programme au premier plan (le jeu), et consommation de l'overlay lui-même.
        let fg = foreground_pid();
        s.fps_available = self.fps.available();
        s.fps = self.fps.sample(fg);
        let fg_pid = Pid::from_u32(fg);
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.self_pid, fg_pid]),
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        if let Some(p) = self.sys.process(fg_pid) {
            s.foreground = p.name().to_string_lossy().trim_end_matches(".exe").to_string();
        }
        if let Some(p) = self.sys.process(self.self_pid) {
            s.self_cpu = p.cpu_usage() / self.sys.cpus().len().max(1) as f32;
            s.self_mem = p.memory();
        }
        s
    }
}
