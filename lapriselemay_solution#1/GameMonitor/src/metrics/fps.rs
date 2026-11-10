//! FPS via ETW, la même source que PresentMon : on suit les appels « Present » de DirectX
//! (DXGI pour DX10/11/12, D3D9) de chaque processus, et on compte ceux qui affichent une image.
//! Nécessite les droits administrateur.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use ferrisetw::parser::Parser;
use ferrisetw::provider::{EventFilter, Provider};
use ferrisetw::trace::{UserTrace, stop_trace_by_name};
use ferrisetw::{EventRecord, SchemaLocator};
use windows::Win32::Foundation::{
    DXGI_STATUS_MODE_CHANGE_IN_PROGRESS, DXGI_STATUS_NO_DESKTOP_ACCESS, DXGI_STATUS_OCCLUDED,
};
use windows::Win32::Graphics::Dxgi::DXGI_PRESENT_TEST;
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::core::HRESULT;

/// Horodatages ETW en unités de 100 ns.
const SECOND: i64 = 10_000_000;

/// D3D9 : Present réussi, mais la fenêtre est cachée (`S_PRESENT_OCCLUDED`, d3d9.h).
const S_PRESENT_OCCLUDED: HRESULT = HRESULT(0x0876_0868);

/// Les deux runtimes suivis, chacun avec son fournisseur ETW.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Runtime {
    Dxgi,
    D3d9,
}

impl Runtime {
    fn guid(self) -> &'static str {
        match self {
            Self::Dxgi => "CA11C036-0102-4A2D-A6AD-F03CFED5D3C9",
            Self::D3d9 => "783ACA0A-790E-4D7F-8451-AA850511C6B9",
        }
    }

    /// Identifiants des événements Present_Start et Present_Stop.
    fn events(self) -> (u16, u16) {
        match self {
            Self::Dxgi => (42, 43),
            Self::D3d9 => (1, 2),
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct FpsInfo {
    pub fps: f32,
    pub frametime_ms: f32,
    pub worst_ms: f32,
}

struct Frames {
    times: VecDeque<i64>,
    last_seen: Instant,
}

/// Un Present commencé, en attente de sa fin pour savoir s'il a affiché une image.
struct Pending {
    pid: u32,
    /// Début du Present : le temps d'image est l'écart entre deux débuts, comme dans PresentMon.
    start: i64,
    since: Instant,
}

#[derive(Default)]
struct State {
    /// Images affichées par chaque processus.
    frames: HashMap<u32, Frames>,
    /// Present en cours, par thread : le début et la fin d'un Present viennent du même thread.
    pending: HashMap<u32, Pending>,
}

type Shared = Arc<Mutex<State>>;

/// Vrai si un Present terminé par ce code a affiché une image, comme le juge PresentMon : ni échec
/// (Present non bloquant refusé parce que le GPU est encore occupé, par exemple), ni fenêtre cachée,
/// ni changement de mode d'affichage en cours.
fn shown(runtime: Runtime, result: u32) -> bool {
    let result = HRESULT(result as i32);
    if result.is_err() {
        return false;
    }
    match runtime {
        Runtime::Dxgi => {
            ![DXGI_STATUS_OCCLUDED, DXGI_STATUS_MODE_CHANGE_IN_PROGRESS, DXGI_STATUS_NO_DESKTOP_ACCESS]
                .contains(&result)
        }
        Runtime::D3d9 => result != S_PRESENT_OCCLUDED,
    }
}

impl State {
    /// Début d'un Present. Un Present de test (`DXGI_PRESENT_TEST`) ne fait que vérifier l'état de la
    /// fenêtre, sans rien afficher : il n'est pas suivi.
    fn start(&mut self, tid: u32, pid: u32, start: i64, test: bool) {
        // Un Present encore en attente sur ce thread a perdu sa fin : il est oublié, comme dans PresentMon.
        if test {
            self.pending.remove(&tid);
        } else {
            self.pending.insert(tid, Pending { pid, start, since: Instant::now() });
        }
    }

    /// Fin d'un Present : l'image compte s'il l'a affichée. Si le code de retour est illisible,
    /// elle compte quand même, plutôt que de perdre les FPS.
    fn stop(&mut self, runtime: Runtime, tid: u32, result: Option<u32>) {
        let Some(present) = self.pending.remove(&tid) else { return };
        if result.is_none_or(|r| shown(runtime, r)) {
            self.record(present.pid, present.start);
        }
    }

    fn record(&mut self, pid: u32, ts: i64) {
        let entry = self.frames.entry(pid).or_insert_with(|| Frames {
            times: VecDeque::with_capacity(512),
            last_seen: Instant::now(),
        });
        entry.last_seen = Instant::now();
        // Rangées dans l'ordre : deux threads d'un même jeu peuvent finir leurs Present dans le désordre.
        let at = entry.times.partition_point(|&t| t <= ts);
        entry.times.insert(at, ts);
        let newest = entry.times.back().copied().unwrap_or(ts);
        while entry.times.front().is_some_and(|&t| t < newest - 2 * SECOND) {
            entry.times.pop_front();
        }
    }

    fn clear(&mut self) {
        self.frames.clear();
        self.pending.clear();
    }
}

/// Suit les images présentées par chaque processus, tant que la session ETW est ouverte.
pub struct FpsTracker {
    state: Shared,
    trace: Option<UserTrace>,
}

/// Nom de la session ETW, propre à la session Windows (« GameMonitorFPS-1 ») : les sessions ETW sont
/// communes à toute la machine, alors qu'il peut tourner un GameMonitor dans chaque session Windows
/// (l'instance unique ne vaut que dans une session). Avec un seul nom, chacun fermerait celle de l'autre.
fn session_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        let mut session = 0;
        unsafe {
            let _ = ProcessIdToSessionId(std::process::id(), &mut session);
        }
        format!("GameMonitorFPS-{session}")
    })
}

/// Ferme la session ETW par son nom, quel que soit le thread ou le processus qui l'a ouverte.
/// Une session ETW survit au processus qui l'a créée : sans arrêt explicite, elle resterait
/// active (et DirectX continuerait d'y écrire) jusqu'au redémarrage de Windows.
pub fn stop_session() {
    let _ = stop_trace_by_name(session_name());
}

fn present_provider(runtime: Runtime, state: &Shared) -> Provider {
    let state = state.clone();
    let (start, stop) = runtime.events();
    Provider::by_guid(runtime.guid())
        // Seuls le début et la fin de chaque Present comptent : le filtre écarte le reste à la source.
        .add_filter(EventFilter::ByEventIds(vec![start, stop]))
        .add_callback(move |r: &EventRecord, schemas: &SchemaLocator| {
            let field = |name: &str| {
                let schema = schemas.event_schema(r).ok()?;
                let parser = Parser::create(r, &schema);
                parser.try_parse::<u32>(name).ok()
            };
            let id = r.event_id();
            if id == start {
                let test = runtime == Runtime::Dxgi
                    && field("Flags").is_some_and(|flags| flags & DXGI_PRESENT_TEST.0 != 0);
                if let Ok(mut state) = state.lock() {
                    state.start(r.thread_id(), r.process_id(), r.raw_timestamp(), test);
                }
            } else if id == stop {
                let result = field("Result");
                if let Ok(mut state) = state.lock() {
                    state.stop(runtime, r.thread_id(), result);
                }
            }
        })
        .build()
}

impl FpsTracker {
    pub fn start() -> Self {
        let mut tracker = Self { state: Shared::default(), trace: None };
        tracker.resume();
        tracker
    }

    /// Ouvre la session ETW, si elle ne l'est pas déjà.
    pub fn resume(&mut self) {
        if self.trace.is_some() {
            return;
        }
        // Une session restée ouverte après un plantage bloquerait la nouvelle.
        stop_session();
        self.trace = UserTrace::new()
            .named(session_name().to_string())
            .enable(present_provider(Runtime::Dxgi, &self.state))
            .enable(present_provider(Runtime::D3d9, &self.state))
            .start_and_process()
            .ok();
    }

    /// Ferme la session ETW : DirectX n'émet plus rien pour nous et plus rien n'est traité.
    pub fn pause(&mut self) {
        if let Some(trace) = self.trace.take() {
            let _ = trace.stop();
        }
        if let Ok(mut state) = self.state.lock() {
            state.clear();
        }
    }

    pub fn available(&self) -> bool {
        self.trace.is_some()
    }

    /// FPS de la dernière seconde pour le processus donné (normalement le jeu au premier plan).
    pub fn sample(&self, pid: u32) -> Option<FpsInfo> {
        let mut state = self.state.lock().ok()?;
        // On oublie les processus qui n'affichent plus rien, et les Present dont la fin s'est perdue.
        state.frames.retain(|_, f| f.last_seen.elapsed() < Duration::from_secs(5));
        state.pending.retain(|_, p| p.since.elapsed() < Duration::from_secs(5));
        let frames = state.frames.get(&pid)?;
        if frames.last_seen.elapsed() > Duration::from_secs(3) {
            return None;
        }
        summarize(&frames.times)
    }
}

impl Drop for FpsTracker {
    fn drop(&mut self) {
        self.pause();
    }
}

/// Moyenne et pire temps d'image sur la seconde qui précède le dernier horodatage. Un temps d'image va
/// d'une image à la suivante : celui qui se termine dans cette seconde compte en entier, même s'il a
/// commencé avant (un long gel qui vient de finir, par exemple).
fn summarize(times: &VecDeque<i64>) -> Option<FpsInfo> {
    let newest = *times.back()?;
    // L'image qui précède la première de la seconde : elle ouvre son temps d'image.
    let first = times.partition_point(|&t| t <= newest - SECOND).saturating_sub(1);
    let intervals: Vec<i64> = times.range(first..).zip(times.range(first + 1..)).map(|(a, b)| b - a).collect();
    let worst = *intervals.iter().max()?;
    let span = (newest - times[first]) as f32;
    let frametime_ms = span / intervals.len() as f32 / 10_000.0;
    Some(FpsInfo {
        fps: if frametime_ms > 0.0 { 1000.0 / frametime_ms } else { 0.0 },
        frametime_ms,
        worst_ms: worst as f32 / 10_000.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: i64 = 10_000;

    fn times(stamps: impl IntoIterator<Item = i64>) -> VecDeque<i64> {
        stamps.into_iter().collect()
    }

    #[test]
    fn steady_60_fps() {
        // 2 s d'images toutes les 16,667 ms : seule la dernière seconde compte.
        let info = summarize(&times((0..120).map(|i| i * 166_667))).unwrap();
        assert!((info.fps - 60.0).abs() < 0.1, "{}", info.fps);
        assert!((info.frametime_ms - 16.667).abs() < 0.01);
        assert!((info.worst_ms - 16.667).abs() < 0.01);
    }

    #[test]
    fn worst_frame_is_the_longest_interval() {
        let info = summarize(&times([0, 10 * MS, 20 * MS, 70 * MS, 80 * MS])).unwrap();
        assert!((info.worst_ms - 50.0).abs() < 0.001);
        assert!((info.frametime_ms - 20.0).abs() < 0.001);
        assert!((info.fps - 50.0).abs() < 0.01);
    }

    #[test]
    fn stall_ending_in_the_last_second_is_the_worst_frame() {
        // 60 FPS, puis un gel de 1,3 s : il a commencé avant la dernière seconde, mais il s'y termine.
        let info = summarize(&times([0, 166_667, 333_333, 13_333_333, 13_500_000])).unwrap();
        assert!((info.worst_ms - 1300.0).abs() < 0.01, "{}", info.worst_ms);
        assert!((info.frametime_ms - 658.333).abs() < 0.01, "{}", info.frametime_ms);
        assert!(info.fps < 2.0, "{}", info.fps);
    }

    #[test]
    fn needs_two_frames() {
        assert!(summarize(&times([])).is_none());
        assert!(summarize(&times([0])).is_none());
        // Une seule image dans la dernière seconde : son temps d'image part de l'image d'avant.
        let info = summarize(&times([0, 2 * SECOND])).unwrap();
        assert!((info.worst_ms - 2000.0).abs() < 0.001);
        assert!((info.fps - 0.5).abs() < 0.001);
    }

    const S_OK: u32 = 0;
    const DXGI_ERROR_WAS_STILL_DRAWING: u32 = 0x887A_000A;

    /// Images retenues pour le processus `pid`.
    fn recorded(state: &State, pid: u32) -> Vec<i64> {
        state.frames.get(&pid).map(|f| f.times.iter().copied().collect()).unwrap_or_default()
    }

    #[test]
    fn only_presents_that_show_an_image_count() {
        let mut state = State::default();
        state.start(1, 10, 0, false);
        state.stop(Runtime::Dxgi, 1, Some(S_OK));
        // Present non bloquant refusé : le GPU est encore occupé.
        state.start(1, 10, 10 * MS, false);
        state.stop(Runtime::Dxgi, 1, Some(DXGI_ERROR_WAS_STILL_DRAWING));
        // Fenêtre cachée.
        state.start(1, 10, 20 * MS, false);
        state.stop(Runtime::Dxgi, 1, Some(DXGI_STATUS_OCCLUDED.0 as u32));
        state.start(1, 10, 30 * MS, false);
        state.stop(Runtime::Dxgi, 1, Some(S_OK));
        assert_eq!(recorded(&state, 10), [0, 30 * MS]);
    }

    #[test]
    fn test_presents_are_ignored() {
        let mut state = State::default();
        // DXGI_PRESENT_TEST : sa fin ne doit pas faire compter une image.
        state.start(1, 10, 0, true);
        state.stop(Runtime::Dxgi, 1, Some(S_OK));
        assert!(recorded(&state, 10).is_empty());
    }

    #[test]
    fn presents_are_matched_by_thread() {
        let mut state = State::default();
        state.start(1, 10, 0, false);
        state.start(2, 10, 5 * MS, false);
        state.stop(Runtime::Dxgi, 2, Some(S_OK));
        state.stop(Runtime::Dxgi, 1, Some(DXGI_ERROR_WAS_STILL_DRAWING));
        // Une fin sans début (session ouverte au milieu d'un Present) ne compte pas.
        state.stop(Runtime::Dxgi, 3, Some(S_OK));
        assert_eq!(recorded(&state, 10), [5 * MS]);
        assert!(state.pending.is_empty());
    }

    #[test]
    fn frames_stay_in_order() {
        let mut state = State::default();
        state.start(1, 10, 20 * MS, false);
        state.start(2, 10, 10 * MS, false);
        state.stop(Runtime::Dxgi, 1, Some(S_OK));
        state.stop(Runtime::Dxgi, 2, Some(S_OK));
        assert_eq!(recorded(&state, 10), [10 * MS, 20 * MS]);
    }

    #[test]
    fn unreadable_result_still_counts() {
        let mut state = State::default();
        state.start(1, 10, 0, false);
        state.stop(Runtime::D3d9, 1, None);
        assert_eq!(recorded(&state, 10), [0]);
    }

    #[test]
    fn result_codes() {
        assert!(shown(Runtime::Dxgi, S_OK));
        assert!(!shown(Runtime::Dxgi, DXGI_ERROR_WAS_STILL_DRAWING));
        assert!(!shown(Runtime::Dxgi, DXGI_STATUS_OCCLUDED.0 as u32));
        assert!(!shown(Runtime::Dxgi, DXGI_STATUS_MODE_CHANGE_IN_PROGRESS.0 as u32));
        assert!(!shown(Runtime::Dxgi, DXGI_STATUS_NO_DESKTOP_ACCESS.0 as u32));
        assert!(shown(Runtime::D3d9, S_OK));
        assert!(!shown(Runtime::D3d9, S_PRESENT_OCCLUDED.0 as u32));
        // D3DERR_WASSTILLDRAWING
        assert!(!shown(Runtime::D3d9, 0x8876_021C));
    }

    #[test]
    fn session_name_is_per_windows_session() {
        let name = session_name();
        let session = name.strip_prefix("GameMonitorFPS-").unwrap();
        assert!(session.parse::<u32>().is_ok(), "{name}");
    }
}
