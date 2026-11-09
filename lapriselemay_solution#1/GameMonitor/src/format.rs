//! Transforme les mesures en lignes de texte colorées pour l'overlay.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::COLORREF;

use crate::metrics::Snapshot;

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

pub const LABEL: COLORREF = rgb(150, 155, 170);
const VALUE: COLORREF = rgb(240, 240, 240);
const DIM: COLORREF = rgb(120, 125, 140);
const GOOD: COLORREF = rgb(90, 220, 120);
const WARN: COLORREF = rgb(255, 190, 60);
pub const BAD: COLORREF = rgb(255, 90, 90);
pub const ACCENT: COLORREF = rgb(80, 200, 255);

pub struct Line {
    pub label: String,
    pub parts: Vec<(String, COLORREF)>,
}

impl Line {
    fn new(label: &str) -> Self {
        Self { label: label.to_string(), parts: Vec::new() }
    }
    fn add(mut self, text: impl Into<String>, color: COLORREF) -> Self {
        if !self.parts.is_empty() {
            self.parts.push(("  ".into(), VALUE));
        }
        self.parts.push((text.into(), color));
        self
    }
}

/// Nombre décimal à la française (virgule).
fn dec(v: f64, digits: usize) -> String {
    format!("{v:.digits$}").replace('.', ",")
}

fn bytes(b: f64) -> String {
    const K: f64 = 1024.0;
    // Seuil à 1000 (et non 1024) pour éviter d'afficher « 1000 Ko ».
    if b >= 1000.0 * K * K {
        format!("{} Go", dec(b / (K * K * K), 1))
    } else if b >= 1000.0 * K {
        format!("{} Mo", dec(b / (K * K), 1))
    } else {
        format!("{} Ko", dec(b / K, 0))
    }
}

fn rate(b: f64) -> String {
    format!("{}/s", bytes(b))
}

/// Durée : « 12 s », « 2 min 05 s », « 1 h 05 ».
fn duration(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min {:02} s", secs / 60, secs % 60),
        _ => format!("{} h {:02}", secs / 3600, secs / 60 % 60),
    }
}

fn temp_color(t: f32) -> COLORREF {
    if t < 70.0 {
        GOOD
    } else if t < 85.0 {
        WARN
    } else {
        BAD
    }
}

fn load_color(pct: f64) -> COLORREF {
    if pct < 75.0 {
        VALUE
    } else if pct < 90.0 {
        WARN
    } else {
        BAD
    }
}

/// Ligne d'avertissement quand Windows a refusé certains raccourcis (déjà pris ailleurs).
pub fn hotkey_warning(keys: &str) -> Line {
    Line::new("").add(format!("Raccourci indisponible : Ctrl+Maj+{keys}"), WARN)
}

/// Overlay déverrouillé, en tête : comment le reverrouiller. La croix de fermeture se dessine à droite.
pub fn unlocked_header() -> Line {
    Line::new("").add("Ctrl+Maj+F11 : verrouiller", ACCENT)
}

/// Overlay déverrouillé, en bas : ce qui se fait à la souris.
pub fn unlocked_footer() -> Line {
    Line::new("").add("Glisser : déplacer · double-clic : détails", ACCENT)
}

/// Ligne d'avertissement quand la collecte ne publie plus de mesure (voir [`Freshness`]).
fn frozen_warning(age: Duration) -> Line {
    Line::new("").add(format!("Mesures figées depuis {}", duration(age)), WARN)
}

/// Secondes de l'overlay (une par tic de son minuteur) sans nouvelle mesure au-delà desquelles
/// la mesure affichée est signalée comme figée.
const FROZEN_AFTER: u32 = 5;

/// Repère une collecte qui ne publie plus : la même mesure, seconde après seconde.
///
/// On compte les tics du minuteur de l'overlay plutôt que l'âge de la mesure : sous Windows,
/// `Instant` avance aussi pendant la veille. Au réveil, la mesure d'avant la veille paraîtrait figée
/// depuis des heures, le temps que la collecte en prenne une nouvelle. Le processus ne tournant pas
/// pendant la veille, le réveil ne compte ici que pour un tic.
#[derive(Default)]
pub struct Freshness {
    /// Heure de la dernière mesure vue.
    seen: Option<Instant>,
    /// Tics passés depuis sans nouvelle mesure.
    unchanged: u32,
}

impl Freshness {
    /// À chaque tic du minuteur (une fois par seconde, overlay masqué compris).
    pub fn tick(&mut self, taken: Option<Instant>) {
        if taken == self.seen {
            self.unchanged = self.unchanged.saturating_add(1);
        } else {
            *self = Self { seen: taken, unchanged: 0 };
        }
    }

    /// Âge de la mesure `taken` si elle est figée. Pas de mesure (lancement, reprise après une pause) :
    /// rien de figé.
    pub fn frozen(&self, taken: Option<Instant>, now: Instant) -> Option<Duration> {
        match taken {
            Some(t) if taken == self.seen && self.unchanged >= FROZEN_AFTER => {
                Some(now.saturating_duration_since(t))
            }
            _ => None,
        }
    }
}

/// Lignes de l'overlay pour la mesure `s`. `frozen` : son âge, si elle est figée (voir [`Freshness`]).
/// `hint` : avec le rappel des raccourcis.
pub fn build_lines(s: &Snapshot, frozen: Option<Duration>, detailed: bool, hint: bool) -> Vec<Line> {
    let mut lines = Vec::new();
    if s.taken.is_none() {
        // Au lancement, et à la réapparition après une pause : la première mesure prend 1 s.
        // Il n'y a encore rien de mesuré, donc rien de figé.
        lines.push(Line::new("").add("GameMonitor : mesure en cours…", DIM));
        return lines;
    }

    // FPS du jeu au premier plan
    let mut fps = Line::new("FPS");
    fps = match (s.fps_available, s.fps) {
        (false, _) => fps.add("lancer en admin", WARN),
        (true, None) => fps.add("–", DIM),
        (true, Some(f)) => {
            let color = if f.fps >= 60.0 { GOOD } else if f.fps >= 30.0 { WARN } else { BAD };
            fps.add(format!("{:.0}", f.fps), color)
                .add(format!("{} ms", dec(f.frametime_ms as f64, 1)), VALUE)
                .add(format!("pire {} ms", dec(f.worst_ms as f64, 1)), DIM)
        }
    };
    if !s.foreground.is_empty() && s.fps.is_some() {
        fps = fps.add(s.foreground.clone(), DIM);
    }
    lines.push(fps);

    // CPU
    let mut cpu = Line::new("CPU").add(format!("{:.0} %", s.cpu_usage), load_color(s.cpu_usage as f64));
    if let Some(mhz) = s.cpu_mhz {
        cpu = cpu.add(format!("{} GHz", dec(mhz / 1000.0, 2)), VALUE);
    }
    cpu = match s.cpu_temp {
        // Toujours avec sa source : PawnIO et LHM lisent la sonde du processeur, l'ACPI un capteur
        // de la carte mère, moins fiable.
        Some(t) if !s.cpu_temp_source.is_empty() => {
            cpu.add(format!("{t:.0} °C"), temp_color(t)).add(s.cpu_temp_source, DIM)
        }
        Some(t) => cpu.add(format!("{t:.0} °C"), temp_color(t)),
        None => cpu.add("temp. n/d", DIM),
    };
    lines.push(cpu);

    // GPU
    let g = &s.gpu;
    let mut gpu = Line::new("GPU");
    gpu = match g.usage {
        Some(u) => gpu.add(format!("{u:.0} %"), VALUE),
        None => gpu.add("–", DIM),
    };
    if let Some(c) = g.clock_mhz {
        gpu = gpu.add(format!("{c} MHz"), VALUE);
    }
    if let Some(t) = g.temp {
        gpu = gpu.add(format!("{t} °C"), temp_color(t as f32));
    }
    if let Some(p) = g.power_w {
        gpu = gpu.add(format!("{p:.0} W"), VALUE);
    }
    if let Some(f) = g.fan {
        gpu = gpu.add(format!("ventil. {f} %"), VALUE);
    }
    lines.push(gpu);

    if let Some(used) = g.vram_used {
        let text = match g.vram_total {
            Some(total) => format!("{} / {}", bytes(used as f64), bytes(total as f64)),
            None => bytes(used as f64),
        };
        let color = g
            .vram_total
            .map(|t| load_color(used as f64 * 100.0 / t as f64))
            .unwrap_or(VALUE);
        lines.push(Line::new("VRAM").add(text, color));
    }

    // RAM
    let ram_pct = s.ram_used as f64 * 100.0 / s.ram_total.max(1) as f64;
    lines.push(
        Line::new("RAM")
            .add(format!("{} / {}", bytes(s.ram_used as f64), bytes(s.ram_total as f64)), VALUE)
            .add(format!("{ram_pct:.0} %"), load_color(ram_pct)),
    );

    // Disque
    let mut disk = Line::new("Disque");
    if let (Some(r), Some(w)) = (s.disk_read, s.disk_write) {
        disk = disk.add(format!("L {}", rate(r)), VALUE).add(format!("É {}", rate(w)), VALUE);
    }
    if let Some(a) = s.disk_active {
        disk = disk.add(format!("actif {a:.0} %"), load_color(a));
    }
    lines.push(disk);

    // Réseau
    lines.push(
        Line::new("Réseau")
            .add(format!("↓ {}", rate(s.net_down)), VALUE)
            .add(format!("↑ {}", rate(s.net_up)), VALUE),
    );

    if detailed {
        lines.push(Line::new("").add(s.cpu_name.clone(), DIM));
        if s.cpu_temp.is_none() && s.pawnio_missing {
            lines.push(Line::new("").add("Temp. CPU : installer PawnIO (pawnio.eu)", DIM));
        }
        lines.push(Line::new("").add(g.name.clone(), DIM));
        if let Some(m) = g.mem_clock_mhz {
            lines.push(Line::new("GPU mém.").add(format!("{m} MHz"), VALUE));
        }
        for (i, chunk) in s.cpu_cores.chunks(8).enumerate() {
            let label = if i == 0 { "Cœurs" } else { "" };
            let mut line = Line::new(label);
            for (j, c) in chunk.iter().enumerate() {
                if j > 0 {
                    line.parts.push((" ".into(), VALUE));
                }
                line.parts.push((format!("{c:>3.0}"), load_color(*c as f64)));
            }
            if label.is_empty() {
                // Aligne les lignes suivantes sous la première.
                line.label = " ".into();
            }
            lines.push(line);
        }
        if s.swap_total > 0 {
            lines.push(Line::new("Pagin.").add(
                format!("{} / {}", bytes(s.swap_used as f64), bytes(s.swap_total as f64)),
                VALUE,
            ));
        }
        for d in &s.drives {
            lines.push(Line::new(&d.mount).add(
                format!("{} libres / {}", bytes(d.free as f64), bytes(d.total as f64)),
                VALUE,
            ));
        }
        let up = s.uptime;
        lines.push(Line::new("Allumé").add(format!("{} h {:02}", up / 3600, up / 60 % 60), VALUE));
        lines.push(
            Line::new("Overlay")
                .add(format!("{} % CPU", dec(s.self_cpu as f64, 1)), ACCENT)
                .add(bytes(s.self_mem as f64), ACCENT),
        );
    }

    if hint {
        lines.push(Line::new("").add("Ctrl+Maj+F10 masquer · F11 déplacer", DIM));
    }

    // Plus de nouvelle mesure alors que l'overlay est affiché : la collecte est bloquée.
    // Les valeurs passent en gris pour ne pas les faire passer pour actuelles, et on dit depuis quand.
    if let Some(age) = frozen {
        for (_, color) in lines.iter_mut().flat_map(|l| &mut l.parts) {
            *color = DIM;
        }
        lines.insert(0, frozen_warning(age));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::FpsInfo;

    fn text(line: &Line) -> String {
        line.parts.iter().map(|(t, _)| t.as_str()).collect()
    }

    fn find<'a>(lines: &'a [Line], label: &str) -> &'a Line {
        lines.iter().find(|l| l.label == label).expect(label)
    }

    /// Une mesure qui vient d'être prise.
    fn measured() -> Snapshot {
        Snapshot { taken: Some(Instant::now()), ..Default::default() }
    }

    /// Lignes d'une mesure qui n'est pas figée.
    fn fresh(s: &Snapshot, detailed: bool, hint: bool) -> Vec<Line> {
        build_lines(s, None, detailed, hint)
    }

    #[test]
    fn french_decimals_and_units() {
        assert_eq!(dec(12.3456, 2), "12,35");
        assert_eq!(bytes(999.0 * 1024.0), "999 Ko");
        // Jamais « 1000 Ko » : on passe aux Mo dès 1000 Ko.
        assert_eq!(bytes(1000.0 * 1024.0), "1,0 Mo");
        assert_eq!(bytes(8.0 * 1024.0 * 1024.0 * 1024.0), "8,0 Go");
        assert_eq!(rate(2.5 * 1024.0 * 1024.0), "2,5 Mo/s");
    }

    #[test]
    fn waiting_for_the_first_measure() {
        // Au lancement et après une pause, rien n'est encore mesuré : jamais signalé comme figé.
        let lines = build_lines(&Snapshot::default(), Some(Duration::from_secs(60)), true, true);
        assert_eq!(lines.len(), 1);
        assert!(text(&lines[0]).contains("mesure en cours"));
    }

    #[test]
    fn fps_line() {
        let mut s = measured();
        assert_eq!(text(find(&fresh(&s, false, false), "FPS")), "lancer en admin");

        s.fps_available = true;
        assert_eq!(text(find(&fresh(&s, false, false), "FPS")), "–");

        s.fps = Some(FpsInfo { fps: 143.6, frametime_ms: 6.96, worst_ms: 12.04 });
        s.foreground = "Game".into();
        assert_eq!(text(find(&fresh(&s, false, false), "FPS")), "144  7,0 ms  pire 12,0 ms  Game");
    }

    #[test]
    fn cpu_line_and_temperature_source() {
        let mut s = Snapshot { cpu_name: "Intel Core i5-14400F".into(), ..measured() };
        s.cpu_usage = 42.4;
        s.cpu_mhz = Some(4_650.0);
        assert_eq!(text(find(&fresh(&s, false, false), "CPU")), "42 %  4,65 GHz  temp. n/d");

        // La source est affichée à côté de la température, en mode compact comme détaillé.
        s.cpu_temp = Some(61.6);
        s.cpu_temp_source = "PawnIO";
        assert_eq!(text(find(&fresh(&s, false, false), "CPU")), "42 %  4,65 GHz  62 °C  PawnIO");
        s.cpu_temp_source = "ACPI";
        let lines = fresh(&s, true, false);
        assert_eq!(text(find(&lines, "CPU")), "42 %  4,65 GHz  62 °C  ACPI");
        assert!(lines.iter().any(|l| text(l) == "Intel Core i5-14400F"));
    }

    #[test]
    fn suggests_installing_pawnio() {
        // Processeur Intel sans PawnIO, et aucune température : le mode détaillé dit quoi faire.
        let s = Snapshot { pawnio_missing: true, ..measured() };
        let hint = |lines: &[Line]| lines.iter().any(|l| text(l).contains("installer PawnIO"));
        assert!(hint(&fresh(&s, true, false)));
        assert!(!hint(&fresh(&s, false, false)));
        // Une température lue autrement (LibreHardwareMonitor, ACPI) suffit.
        let s = Snapshot { cpu_temp: Some(50.0), cpu_temp_source: "LHM", ..s };
        assert!(!hint(&fresh(&s, true, false)));
    }

    #[test]
    fn hint_and_hotkey_warning() {
        let s = measured();
        let hint = |lines: &[Line]| lines.iter().any(|l| text(l) == "Ctrl+Maj+F10 masquer · F11 déplacer");
        assert!(hint(&fresh(&s, false, true)));
        assert!(!hint(&fresh(&s, false, false)));
        // En mode détaillé aussi, c'est l'overlay qui décide d'afficher le rappel ou non.
        assert!(!hint(&fresh(&s, true, false)));
        assert_eq!(text(&hotkey_warning("F11")), "Raccourci indisponible : Ctrl+Maj+F11");
    }

    #[test]
    fn frozen_measures_are_greyed_with_a_warning() {
        let s = Snapshot { cpu_usage: 95.0, cpu_temp: Some(88.0), ..measured() };

        // Mesure actuelle : pas d'avertissement, couleurs habituelles.
        let lines = fresh(&s, false, false);
        assert_eq!(lines[0].label, "FPS");
        assert!(find(&lines, "CPU").parts.iter().any(|(_, c)| *c == BAD));

        // Figée : avertissement en tête, et toutes les valeurs en gris.
        let lines = build_lines(&s, Some(Duration::from_secs(12)), false, true);
        assert_eq!(text(&lines[0]), "Mesures figées depuis 12 s");
        assert!(lines[0].parts.iter().all(|(_, c)| *c == WARN));
        assert_eq!(text(find(&lines, "CPU")), "95 %  88 °C");
        assert!(lines[1..].iter().flat_map(|l| &l.parts).all(|(_, c)| *c == DIM));
    }

    /// Instant `secs` secondes après `t0`.
    fn at(t0: Instant, secs: f64) -> Instant {
        t0 + Duration::from_secs_f64(secs)
    }

    #[test]
    fn frozen_after_five_ticks_without_a_new_measure() {
        let t0 = Instant::now();
        let mut f = Freshness::default();
        // Une mesure par seconde : jamais figée.
        for i in 0..10 {
            let taken = Some(at(t0, i as f64));
            f.tick(taken);
            assert_eq!(f.frozen(taken, at(t0, i as f64 + 0.5)), None);
        }
        // Plus rien après celle de 9 s : figée au cinquième tic sans nouvelle mesure.
        let last = Some(at(t0, 9.0));
        for i in 1..5 {
            f.tick(last);
            assert_eq!(f.frozen(last, at(t0, 9.5 + i as f64)), None);
        }
        f.tick(last);
        assert_eq!(f.frozen(last, at(t0, 14.5)), Some(Duration::from_millis(5_500)));
        // Une nouvelle mesure n'est pas figée, même avant le tic suivant.
        assert_eq!(f.frozen(Some(at(t0, 15.0)), at(t0, 15.2)), None);
    }

    #[test]
    fn waking_up_from_sleep_is_not_frozen() {
        // Instant a avancé d'une heure pendant la veille, mais rien n'a tourné :
        // un seul tic passe sans nouvelle mesure avant que la collecte reprenne.
        let t0 = Instant::now();
        let mut f = Freshness::default();
        f.tick(Some(t0));
        f.tick(Some(t0));
        assert_eq!(f.frozen(Some(t0), at(t0, 3_600.0)), None);
    }

    #[test]
    fn no_measure_is_never_frozen() {
        let mut f = Freshness::default();
        for _ in 0..60 {
            f.tick(None);
        }
        assert_eq!(f.frozen(None, Instant::now()), None);
    }

    #[test]
    fn frozen_warning_duration() {
        assert_eq!(text(&frozen_warning(Duration::from_millis(12_900))), "Mesures figées depuis 12 s");
        assert_eq!(text(&frozen_warning(Duration::from_secs(125))), "Mesures figées depuis 2 min 05 s");
        assert_eq!(text(&frozen_warning(Duration::from_secs(3_900))), "Mesures figées depuis 1 h 05");
    }
}
