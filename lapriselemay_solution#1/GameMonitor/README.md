# GameMonitor

Overlay de monitoring natif pour PC de jeu Windows (pensé pour l'Acer Nitro N50-656).
Il affiche en superposition, par-dessus le jeu, les FPS et l'état de la machine, en consommant le moins possible.

- Un seul fichier `GameMonitor.exe` d'environ 600 Ko, sans installation ni runtime à ajouter.
- Environ 0,2 % de CPU et 10 Mo de mémoire privée (mesuré sur un i5-1135G7).
- Win32 pur, sans moteur web ni framework graphique.

## Mesures affichées

| Ligne | Contenu | Source |
|---|---|---|
| FPS | images/s, temps d'image moyen, pire temps d'image, nom du jeu | ETW (DXGI et D3D9), comme PresentMon |
| CPU | utilisation, fréquence réelle (turbo inclus), température et sa source | sysinfo, compteurs PDH, PawnIO ou WMI |
| GPU | utilisation, fréquence, température, consommation (W), ventilateur | NVML (pilote NVIDIA), sinon compteurs Windows |
| VRAM | mémoire vidéo utilisée / totale | NVML |
| RAM | utilisée / totale, pourcentage | sysinfo |
| Disque | lecture, écriture, % d'activité | compteurs PDH |
| Réseau | débit descendant et montant | sysinfo |

Le mode détaillé ajoute : nom du CPU et du GPU, fréquence mémoire du GPU, charge de chaque cœur,
fichier d'échange, espace libre de chaque disque, durée depuis l'allumage et la consommation de l'overlay lui-même.

## Utilisation

1. Copier `GameMonitor.exe` sur le PC de jeu (n'importe quel dossier).
2. Le lancer : Windows demande les droits administrateur (nécessaires pour les FPS et la température CPU).
3. Mettre le jeu en mode **fenêtré sans bordure** (« borderless »). En plein écran exclusif, Windows
   ne laisse aucune fenêtre s'afficher par-dessus le jeu.

| Raccourci | Action |
|---|---|
| Ctrl+Maj+F10 | Masquer / afficher (masqué, plus rien n'est mesuré, FPS compris) |
| Ctrl+Maj+F11 | Déverrouiller l'overlay pour le déplacer ; le même raccourci le reverrouille |

Verrouillé (par défaut), l'overlay laisse passer les clics. Déverrouillé, il s'entoure d'un cadre bleu
et se pilote à la souris :

- le glisser pour le déplacer : près d'un bord de l'écran, il s'y aimante ;
- double-cliquer dessus pour passer au mode détaillé, et en revenir ;
- cliquer sur la croix rouge, en haut à droite, pour quitter GameMonitor.

Verrouillé ou non, il ne prend jamais le focus du jeu. Le masquer le reverrouille.
Sa place est retenue d'un lancement à l'autre, dans `%LOCALAPPDATA%\GameMonitor\position.txt`.

Relancer `GameMonitor.exe` alors qu'il tourne déjà réaffiche l'overlay et rappelle les raccourcis
(un seul GameMonitor à la fois). Si une autre application a déjà pris un des raccourcis,
l'overlay l'indique en orange.

Avec plusieurs écrans, l'overlay s'affiche sur l'écran du jeu (la fenêtre au premier plan), à la même place
(même coin, même écart aux bords), et le suit dans la seconde s'il change d'écran. Cliquer sur le bureau ou
la barre des tâches ne le déplace pas. Sa taille suit la mise à l'échelle de chaque écran (100 %, 150 %…) ;
en mode détaillé, il s'agrandit vers le centre de l'écran.

Si les mesures cessent d'être mises à jour pendant plus de 5 s, l'overlay ne les fait pas passer
pour actuelles : les valeurs passent en gris sous la ligne orange « Mesures figées depuis 12 s ».

Pour le lancer à l'ouverture de session : Windows ne démarre pas depuis le dossier Démarrage une
application qui demande les droits administrateur. Il faut une tâche du Planificateur de tâches,
déclenchée « À l'ouverture de session », avec « Exécuter avec les autorisations maximales ».

### Température du CPU

Windows ne donne pas la température des cœurs du processeur sans pilote spécial. GameMonitor essaie, dans l'ordre :

1. **PawnIO**, pour les processeurs Intel (depuis 2011) et AMD Ryzen (depuis 2017). C'est la mesure la plus
   précise, et rien d'autre n'a besoin de tourner. PawnIO est un pilote signé, à installer une fois :
   <https://pawnio.eu>, ou `winget install namazso.PawnIO`. GameMonitor intègre les deux modules officiels
   dont il a besoin (voir `pawnio/`), mais ne charge que celui du processeur : IntelMSR ou AMDFamily17.
   Chacun ne lit que quelques registres du processeur. Sur les rares Ryzen dont la sonde affiche plus que
   la vraie température (1600X, 1700X, 1800X, 2700X, Threadripper 1000 et 2000), GameMonitor la corrige
   comme le pilote k10temp de Linux. Il trouve PawnIO même s'il est installé après son lancement.
2. **LibreHardwareMonitor**, s'il tourne en arrière-plan : utile pour les processeurs AMD plus anciens.
   Il passe lui aussi par PawnIO : <https://github.com/LibreHardwareMonitor/LibreHardwareMonitor>.
   GameMonitor lit ses capteurs automatiquement, même si LibreHardwareMonitor est lancé après lui.
3. La zone thermique ACPI de la carte mère, si elle en a une. Elle est moins précise et absente sur beaucoup de PC.

LibreHardwareMonitor et l'ACPI ne servent qu'en secours : tant que PawnIO répond, GameMonitor ne s'y
connecte pas du tout. La source est affichée à côté de la température : « 62 °C PawnIO », « LHM » ou « ACPI ».
Sans aucune, la ligne affiche « temp. n/d » ; sur un processeur Intel ou Ryzen sans PawnIO, le mode
détaillé rappelle de l'installer. La température du GPU NVIDIA, elle, fonctionne toujours.

La température CPU est lue à part : si LibreHardwareMonitor ne répond plus, le reste de l'overlay
continue d'être mis à jour et la température passe à « temp. n/d » au bout de 5 s,
jusqu'à ce qu'il réponde de nouveau.

### Limites connues

- FPS : les jeux DirectX 9 à 12 sont couverts. Les jeux Vulkan ou OpenGL peuvent ne pas en afficher.
- L'overlay ne s'affiche pas en plein écran exclusif (voir ci-dessus).
- Avec plusieurs écrans, l'overlay suit la fenêtre au premier plan, comme la mesure des FPS : cliquer dans une
  application de l'autre écran (navigateur, Discord…) l'y emmène, jusqu'au retour dans le jeu.
- Comme toute fenêtre posée par-dessus un jeu en fenêtré sans bordure, l'overlay peut obliger Windows
  à composer l'image au lieu de la laisser au jeu (« Independent Flip »), ce qui peut ajouter jusqu'à
  une image de latence. En partie compétitive, Ctrl+Maj+F10 le masque.

## Compiler

Il faut Rust (`rustup`, chaîne MSVC) et les outils de compilation C++ de Visual Studio.

```bash
cargo build --release
```

L'exe se trouve dans `target/release/GameMonitor.exe`. Le CRT est lié statiquement (`.cargo/config.toml`),
donc l'exe n'a pas besoin du Visual C++ Redistributable.

Pour tester sans la demande d'administrateur : `cargo run` (debug), ou définir `GAMEMONITOR_NO_ADMIN=1` avant
`cargo build --release`.

Tests : `cargo test` (mise en forme et mesures figées, calcul des FPS, charge GPU, température
qui ne répond plus, placement et déplacement de l'overlay, place retenue).

Test de fumée : `ci/smoke-test.ps1`, à lancer à la main sur un vrai Windows, dans un PowerShell
ouvert en administrateur. Il ferme les GameMonitor déjà lancés.

```powershell
powershell -ExecutionPolicy Bypass -File ci\smoke-test.ps1 -Exe target\release\GameMonitor.exe
```

Il vérifie le démarrage, la session ETW, le déplacement à la souris et la place retrouvée au lancement
suivant, le mode détaillé, l'instance unique, la pause quand l'overlay est masqué, et la fermeture par
la croix sans session ETW laissée ouverte.

## Structure

```
src/
  main.rs          démarrage, instance unique, arrêt
  overlay.rs       fenêtre Win32 sur l'écran du jeu, dessin GDI, raccourcis et souris
  placement.rs     place de l'overlay sur l'écran, aimantation aux bords, place retenue
  format.rs        mise en forme et couleurs des lignes
  metrics/
    mod.rs         thread de collecte (1 fois par seconde, en pause quand l'overlay est masqué)
    fps.rs         FPS via ETW
    gpu.rs         NVML ou compteurs génériques
    temps.rs       température CPU (PawnIO, sinon WMI), dans un thread à part
    pawnio.rs      température des processeurs Intel et Ryzen par le pilote PawnIO
    pdh.rs         compteurs de performance Windows
pawnio/
  *.bin            modules officiels de PawnIO, intégrés à l'exe (LGPL 2.1, voir pawnio/LISEZMOI.md)
ci/
  smoke-test.ps1   test de fumée Windows (à lancer en administrateur)
```
