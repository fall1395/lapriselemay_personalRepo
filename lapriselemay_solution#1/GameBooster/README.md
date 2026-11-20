# GameBooster

Gestionnaire de services Windows orienté jeu, natif et léger. Il liste les services qui ne servent à
rien pendant une partie (environ 80), avec pour chacun :

- un **conseil** : *sans risque*, *selon ton usage* ou *déconseillé* ;
- ce qu'on **perd** tant qu'il est arrêté ;
- son **état** et la **RAM** qu'il occupe en ce moment.

On les arrête **un à un**, ou **tous d'un coup** avec « Tout arrêter », puis « Tout relancer » relance
exactement ceux que GameBooster a arrêtés, rien d'autre. GameBooster affiche la RAM réellement libérée.

Il consomme le moins possible :

- Un seul fichier `GameBooster.exe` d'environ 370 Ko, sans installation ni runtime à ajouter.
- Environ 3 Mo de mémoire privée.
- Win32 pur, comme GameMonitor : aucune autre dépendance que la caisse `windows`.

## Utilisation

1. Copier `GameBooster.exe` sur le PC de jeu (n'importe quel dossier).
2. Le lancer : Windows demande les droits administrateur (sans eux, il refuse d'arrêter ses services).
3. Cocher les services à inclure dans « Tout arrêter » : les services *sans risque* le sont déjà.
   Les cases sont retenues d'un lancement à l'autre.
4. **Tout arrêter** avant de lancer le jeu, **Tout relancer** après.

Pour un seul service : le sélectionner, puis **Arrêter** (ou **Relancer**) dans le panneau du bas, ou
clic droit sur sa ligne. Un service *déconseillé* demande une confirmation.

La liste se met à jour toutes les 3 s. Un clic sur un titre de colonne la trie : par nom, catégorie,
conseil, état, ou RAM (la plus grosse d'abord). Le panneau du bas détaille le service sélectionné :
son nom Windows, ce qu'on perd, la RAM qu'il occupe et la description de Windows.

### Mode jeu

« Tout arrêter » passe en mode jeu. Laisser GameBooster ouvert (réduit dans la barre des tâches) :
toutes les 30 s, il arrête les services cochés que Windows a démarrés ou relancés entre-temps. Sur
beaucoup de PC, Windows Update, BITS ou l'Optimisation de la distribution ne tournent pas au moment
du clic, puis démarrent d'eux-mêmes en pleine partie. « Tout relancer » les relance aussi.

Un service relancé à la main pendant le mode jeu est laissé tranquille jusqu'à « Tout relancer ».

### La RAM affichée

C'est la RAM **privée** du processus du service : celle qu'il libère en s'arrêtant, comme la colonne
Mémoire du Gestionnaire des tâches. La plupart des services n'en occupent que quelques mégaoctets :
le reste de leur mémoire est partagé avec Windows et reste en place. « partagée » : le processus
héberge d'autres services en cours, sa RAM n'est libérée que s'ils s'arrêtent tous. Après un arrêt,
GameBooster compte la RAM des processus qui se sont vraiment fermés.

### À savoir

- **Mettre ses jeux à jour avant « Tout arrêter ».** L'application Xbox et le Microsoft Store
  téléchargent par l'Optimisation de la distribution et BITS. Le jeu lui-même et sa connexion Xbox
  n'en ont pas besoin.
- Une mise à jour de Windows en cours reprendra après.

## Sécurités

- **Il ne change jamais le type de démarrage d'un service** : il l'arrête, c'est tout. Un redémarrage
  de Windows remet donc toujours tout à la normale.
- Fermer la fenêtre avec des services encore arrêtés : il propose de les relancer d'abord.
- Fin de session ou arrêt de Windows : il relance les services avant. Sans ça, avec le démarrage
  rapide de Windows, ils resteraient arrêtés au prochain allumage.
- Les services arrêtés sont notés dans `%LOCALAPPDATA%\GameBooster\etat.txt`, après chaque arrêt.
  Si GameBooster plante ou est fermé, le lancement suivant les retrouve, et les relance tout seul si
  Windows a redémarré depuis. Les cases cochées sont dans `choix.txt`, au même endroit. Seuls les
  services du catalogue sont relus dans ces fichiers : les modifier ne peut rien faire arrêter ni
  relancer d'autre à GameBooster.
- Le catalogue est dans le code (`src/catalog.rs`), pas dans un fichier de configuration : GameBooster
  tourne en administrateur, et un fichier modifiable à côté de l'exe ferait le jeu d'un logiciel
  malveillant. Les tests vérifient qu'aucun service de la liste « jamais touchés » n'y figure.
- Un seul GameBooster à la fois : le relancer réaffiche la fenêtre déjà ouverte.
- Comme GameMonitor, il ne charge ses DLL que depuis System32.

## Le catalogue

| Catégorie | Exemples | Conseil |
|---|---|---|
| Mises à jour | Windows Update, Orchestrateur, BITS, Optimisation de la distribution, mises à jour d'Edge, OneDrive, Chrome, Brave, Firefox, Adobe, Dropbox | sans risque |
| Indexation | Windows Search, partage du Lecteur Windows Media | sans risque |
| Indexation | Préchargement (SysMain) : il compresse la mémoire quand elle manque | déconseillé |
| Télémétrie | Télémétrie de Windows, rapport d'erreurs, diagnostics, Assistant Compatibilité, suivi de liens, télémétrie NVIDIA | sans risque |
| Impression | Spouleur, télécopie, scanneurs (WIA) | selon ton usage |
| Réseau local | Partage de fichiers, découverte réseau, NetBIOS, partage de connexion | selon ton usage |
| Réseau local | WebDAV, point d'accès mobile, Wi-Fi Direct | sans risque |
| Accès à distance | Bureau à distance, TeamViewer, AnyDesk | selon ton usage |
| Accès à distance | Registre à distance, WinRM | sans risque |
| Virtualisation | WSL, Hyper-V | selon ton usage |
| Appareils | Téléphone et SMS (Phone Link), géolocalisation, capteurs, cartes à puce, paiements NFC, portefeuille | sans risque |
| Appareils | Windows Hello, conteneur NVIDIA (overlay, ShadowPlay) | selon ton usage |
| Synchronisation | Courrier, contacts et calendrier, indexation des contacts, messagerie texte | sans risque |
| Divers | Cartes hors ligne, démonstration en magasin, Windows Insider, fuseau horaire automatique, AllJoyn, iSCSI | sans risque |
| Divers | Enregistrement de la Game Bar, contrôle parental | selon ton usage |

La liste complète, avec ce qu'on perd pour chaque service, est dans `src/catalog.rs` et dans
l'application. Un service absent du PC n'apparaît pas.

## Jamais touchés

- Xbox, Gaming Services, GameInput, compte Microsoft et Microsoft Store : la connexion Xbox de
  MSFS 2024, le Game Pass et les sauvegardes en ligne en dépendent.
- Son, réseau, VPN, Teredo et UPnP (multijoueur), Bluetooth et HID (manettes, casques).
- Affichage, saisie au clavier, pilote NVIDIA, Windows Defender, pare-feu, notifications.
- Office (`ClickToRunSvc`), Steam, Epic, anti-triche (EasyAntiCheat, BattlEye, Vanguard), Sunshine
  (diffusion du jeu), et WMI, dont GameMonitor se sert.

## À quoi s'attendre

Arrêter des services ne fait pas gagner des dizaines d'images par seconde, et la RAM libérée se
compte souvent en dizaines de mégaoctets plutôt qu'en gigaoctets. Le vrai gain : moins d'activité
disque, réseau et processeur en arrière-plan (donc moins de saccades), et aucune mise à jour qui
démarre en pleine partie.

## Compiler

Il faut Rust (`rustup`, chaîne MSVC) et les outils de compilation C++ de Visual Studio.

```bash
cargo build --release
```

L'exe se trouve dans `target/release/GameBooster.exe`. Le CRT est lié statiquement (`.cargo/config.toml`),
donc l'exe n'a pas besoin du Visual C++ Redistributable.

Pour tester sans la demande d'administrateur : `cargo run` (debug), ou définir `GAMEBOOSTER_NO_ADMIN=1`
avant `cargo build --release`. La liste s'affiche alors en lecture seule.

Tests : `cargo test`. Ils couvrent le catalogue (aucun service protégé, pas de doublon, textes qui
tiennent dans la fenêtre), le calcul de la RAM libérable et libérée, le tri, l'état sur disque, les
textes de la fenêtre, et lisent, sans rien arrêter, le vrai gestionnaire de services et la mémoire
des processus. L'un d'eux vérifie sur le vrai Windows que le catalogue arrête toujours un service
avant ceux dont il dépend.

L'intégration continue ([`.github/workflows/gamebooster.yml`](../../.github/workflows/gamebooster.yml))
vérifie la mise en forme, Clippy, les tests et la compilation avec Rust 1.95.0 et la version stable.
Chaque exécution joint `GameBooster.exe` à ses artefacts, à télécharger pour le PC de jeu.

## Structure

```
src/
  main.rs       démarrage, instance unique
  catalog.rs    les services inutiles en jeu (conseil, ce qu'on perd) et ceux jamais touchés
  services.rs   gestionnaire de services de Windows : liste, description, arrêt, relance
  memory.rs     RAM privée de chaque processus, RAM du PC
  model.rs      lignes de la liste, RAM libérable et libérée, tri
  state.rs      services arrêtés et cases cochées, gardés sur disque
  ui.rs         fenêtre : boutons, état, détail, travail en arrière-plan, mode jeu
  listview.rs   liste Windows à cases à cocher, en thème sombre
  theme.rs      couleurs, polices, mesures
```
