//! Place de l'overlay sur l'écran du jeu : un coin d'ancrage et l'écart aux deux bords de ce coin,
//! en pixels à 100 %. L'overlay garde ainsi sa place d'un écran à l'autre, quelle que soit leur mise
//! à l'échelle, et s'agrandit vers le centre de l'écran en mode détaillé.
//!
//! La place est retenue d'un lancement à l'autre dans `%LOCALAPPDATA%\GameMonitor\position.txt`.

use std::path::PathBuf;

use windows::Win32::Foundation::RECT;

/// Écart aux bords par défaut, en pixels à 100 % ; aussi celui où l'overlay s'aimante quand on le
/// glisse près d'un bord.
pub const MARGIN: f32 = 10.0;
/// Distance au bord (en pixels à 100 %) en deçà de laquelle l'overlay glissé s'aimante à ce bord.
pub const SNAP: f32 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// Coin d'ancrage : 0 en haut à gauche, puis dans le sens des aiguilles d'une montre.
    pub corner: u8,
    /// Écart au bord gauche (coins 0 et 3) ou droit (coins 1 et 2), en pixels à 100 %.
    pub dx: f32,
    /// Écart au bord haut (coins 0 et 1) ou bas (coins 2 et 3), en pixels à 100 %.
    pub dy: f32,
}

impl Default for Placement {
    /// En haut à gauche.
    fn default() -> Self {
        Self { corner: 0, dx: MARGIN, dy: MARGIN }
    }
}

impl Placement {
    /// Position de l'overlay (taille `size`) sur cet écran, mis à l'échelle `scale` (1,0 = 100 %).
    /// L'overlay reste entier sur l'écran, même plus petit que celui où il a été placé.
    pub fn position(&self, screen: RECT, size: (i32, i32), scale: f32) -> (i32, i32) {
        let gaps = ((self.dx * scale).round() as i32, (self.dy * scale).round() as i32);
        clamp(screen, size, corner_position(screen, size, gaps, self.corner))
    }

    /// Place de l'overlay (taille `size`) posé en `pos` sur cet écran : ancré au coin le plus proche
    /// de son centre, pour qu'en grandissant il s'étende vers le centre de l'écran.
    pub fn from_position(screen: RECT, (w, h): (i32, i32), (x, y): (i32, i32), scale: f32) -> Self {
        let right = 2 * x + w > screen.left + screen.right;
        let bottom = 2 * y + h > screen.top + screen.bottom;
        let dx = if right { screen.right - (x + w) } else { x - screen.left };
        let dy = if bottom { screen.bottom - (y + h) } else { y - screen.top };
        let corner = match (right, bottom) {
            (false, false) => 0,
            (true, false) => 1,
            (true, true) => 2,
            (false, true) => 3,
        };
        Self { corner, dx: dx.max(0) as f32 / scale, dy: dy.max(0) as f32 / scale }
    }

    /// Contenu du fichier de sauvegarde.
    fn to_text(self) -> String {
        format!(
            "# Place de l'overlay GameMonitor, retenue d'un lancement à l'autre.\r\n\
             # coin : 0 en haut à gauche, 1 en haut à droite, 2 en bas à droite, 3 en bas à gauche.\r\n\
             # ecart_x, ecart_y : distance aux bords de ce coin, en pixels à 100 %.\r\n\
             coin={}\r\necart_x={:.1}\r\necart_y={:.1}\r\n",
            self.corner, self.dx, self.dy
        )
    }

    /// Lit le fichier de sauvegarde. `None` s'il est incomplet ou invalide.
    fn parse(text: &str) -> Option<Self> {
        let gap = |v: &str| v.parse::<f32>().ok().filter(|v| v.is_finite() && *v >= 0.0);
        let (mut corner, mut dx, mut dy) = (None, None, None);
        for line in text.trim_start_matches('\u{feff}').lines() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            match key.trim() {
                "coin" => corner = value.parse::<u8>().ok().filter(|c| *c < 4),
                "ecart_x" => dx = gap(value),
                "ecart_y" => dy = gap(value),
                _ => {}
            }
        }
        Some(Self { corner: corner?, dx: dx?, dy: dy? })
    }
}

/// Position dans le coin `corner` de l'écran, à `dx` et `dy` pixels de ses deux bords. Coordonnées
/// négatives pour un écran à gauche ou au-dessus de l'écran principal.
fn corner_position(screen: RECT, (w, h): (i32, i32), (dx, dy): (i32, i32), corner: u8) -> (i32, i32) {
    let (left, top) = (screen.left + dx, screen.top + dy);
    let (right, bottom) = (screen.right - w - dx, screen.bottom - h - dy);
    match corner {
        0 => (left, top),
        1 => (right, top),
        2 => (right, bottom),
        _ => (left, bottom),
    }
}

/// Ramène l'overlay entier sur l'écran (contre son bord haut gauche s'il est plus grand que lui).
fn clamp(screen: RECT, (w, h): (i32, i32), (x, y): (i32, i32)) -> (i32, i32) {
    (x.min(screen.right - w).max(screen.left), y.min(screen.bottom - h).max(screen.top))
}

/// Position de l'overlay glissé à la souris jusqu'en `pos` : il reste entier sur son écran et,
/// approché à moins de `reach` pixels d'un bord, s'y aimante à `margin` pixels.
pub fn dragged(screen: RECT, size: (i32, i32), pos: (i32, i32), margin: i32, reach: i32) -> (i32, i32) {
    let (w, h) = size;
    let (mut x, mut y) = clamp(screen, size, pos);
    if x - screen.left < reach {
        x = screen.left + margin;
    } else if screen.right - (x + w) < reach {
        x = screen.right - w - margin;
    }
    if y - screen.top < reach {
        y = screen.top + margin;
    } else if screen.bottom - (y + h) < reach {
        y = screen.bottom - h - margin;
    }
    clamp(screen, size, (x, y))
}

/// `%LOCALAPPDATA%\GameMonitor\position.txt`
fn path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("GameMonitor").join("position.txt"))
}

/// Place retenue au dernier déplacement ; en haut à gauche s'il n'y en a pas.
pub fn load() -> Placement {
    path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| Placement::parse(&String::from_utf8_lossy(&bytes)))
        .unwrap_or_default()
}

/// Retient la place pour les prochains lancements. Sans conséquence si l'écriture échoue :
/// l'overlay reprendra simplement sa place par défaut.
pub fn save(placement: Placement) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, placement.to_text());
}

#[cfg(test)]
mod tests {
    use super::*;

    const FHD: RECT = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
    const SIZE: (i32, i32) = (300, 200);

    fn at(corner: u8, dx: f32, dy: f32) -> Placement {
        Placement { corner, dx, dy }
    }

    #[test]
    fn corners_of_the_screen() {
        assert_eq!(Placement::default().position(FHD, SIZE, 1.0), (10, 10));
        assert_eq!(at(1, 10.0, 10.0).position(FHD, SIZE, 1.0), (1610, 10));
        assert_eq!(at(2, 10.0, 10.0).position(FHD, SIZE, 1.0), (1610, 870));
        assert_eq!(at(3, 10.0, 10.0).position(FHD, SIZE, 1.0), (10, 870));
    }

    #[test]
    fn screen_left_of_or_above_the_primary_one() {
        // Écran à gauche de l'écran principal, et plus haut que lui : coordonnées négatives.
        let left = RECT { left: -2560, top: -360, right: 0, bottom: 1080 };
        assert_eq!(at(0, 15.0, 15.0).position(left, SIZE, 1.0), (-2545, -345));
        assert_eq!(at(2, 15.0, 15.0).position(left, SIZE, 1.0), (-315, 865));
        let above = RECT { left: 0, top: -1080, right: 1920, bottom: 0 };
        assert_eq!(at(1, 10.0, 10.0).position(above, SIZE, 1.0), (1610, -1070));
        assert_eq!(at(3, 10.0, 10.0).position(above, SIZE, 1.0), (10, -210));
    }

    #[test]
    fn gaps_follow_the_screen_scaling() {
        // 10 px à 100 % : 15 px sur un écran à 150 %.
        assert_eq!(at(1, 10.0, 10.0).position(FHD, (450, 300), 1.5), (1455, 15));
    }

    #[test]
    fn anchored_to_the_nearest_corner() {
        // Posé en bas à droite : ancré à ce coin, écarts mesurés depuis les bords droit et bas.
        assert_eq!(Placement::from_position(FHD, SIZE, (1500, 800), 1.0), at(2, 120.0, 80.0));
        // En haut à gauche d'un écran à 150 % : écarts ramenés en pixels à 100 %.
        assert_eq!(Placement::from_position(FHD, (450, 300), (60, 30), 1.5), at(0, 40.0, 20.0));
    }

    #[test]
    fn stays_where_it_was_dropped() {
        for pos in [(0, 0), (37, 512), (1200, 40), (1620, 880), (700, 300)] {
            let p = Placement::from_position(FHD, SIZE, pos, 1.0);
            assert_eq!(p.position(FHD, SIZE, 1.0), pos, "{pos:?}");
        }
        let p = Placement::from_position(FHD, (450, 300), (101, 707), 1.5);
        assert_eq!(p.position(FHD, (450, 300), 1.5), (101, 707));
    }

    #[test]
    fn grows_away_from_its_corner() {
        // Ancré en bas à droite : agrandi (mode détaillé), il garde ses bords droit et bas.
        let p = at(2, 10.0, 10.0);
        for (w, h) in [SIZE, (500, 400)] {
            let (x, y) = p.position(FHD, (w, h), 1.0);
            assert_eq!((x + w, y + h), (1910, 1070));
        }
    }

    #[test]
    fn always_entirely_on_the_screen() {
        // Placé loin de son coin sur un grand écran, puis affiché sur un plus petit.
        let small = RECT { left: 0, top: 0, right: 1366, bottom: 768 };
        assert_eq!(at(0, 1800.0, 900.0).position(small, SIZE, 1.0), (1066, 568));
    }

    #[test]
    fn dragged_follows_the_mouse_within_the_screen() {
        assert_eq!(dragged(FHD, SIZE, (700, 300), 10, 20), (700, 300));
        // Tiré hors de l'écran : il reste entier dessus, aimanté aux bords atteints.
        assert_eq!(dragged(FHD, SIZE, (-500, 2000), 10, 20), (10, 870));
    }

    #[test]
    fn dragged_near_an_edge_snaps_to_the_margin() {
        assert_eq!(dragged(FHD, SIZE, (16, 400), 10, 20), (10, 400));
        assert_eq!(dragged(FHD, SIZE, (1605, 3), 10, 20), (1610, 10));
        // Un peu plus loin du bord : pas d'aimantation.
        assert_eq!(dragged(FHD, SIZE, (25, 400), 10, 20), (25, 400));
    }

    #[test]
    fn saved_then_read_back() {
        let p = at(2, 120.0, 37.5);
        assert_eq!(Placement::parse(&p.to_text()), Some(p));
    }

    #[test]
    fn invalid_saved_place_is_ignored() {
        assert_eq!(Placement::parse(""), None);
        assert_eq!(Placement::parse("coin=4\necart_x=10\necart_y=10"), None);
        assert_eq!(Placement::parse("coin=1\necart_x=-5\necart_y=10"), None);
        assert_eq!(Placement::parse("coin=1\necart_x=abc\necart_y=10"), None);
        assert_eq!(Placement::parse("coin=1\necart_x=NaN\necart_y=10"), None);
        assert_eq!(Placement::parse("coin=1\necart_x=10"), None);
        // Espaces, fins de ligne Windows et BOM d'un fichier retouché à la main : acceptés.
        let edited = "\u{feff}# commentaire\r\ncoin = 3\r\n ecart_x = 12.5 \r\necart_y=0\r\n";
        assert_eq!(Placement::parse(edited), Some(at(3, 12.5, 0.0)));
    }
}
