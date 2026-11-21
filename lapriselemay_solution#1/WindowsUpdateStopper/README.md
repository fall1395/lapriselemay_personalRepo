# Windows Update Stopper

Arrête Windows Update pendant que tu joues, puis le relance. Natif et léger, il gère les quatre
services qui font tourner les mises à jour de Windows :

- l'**orchestrateur des mises à jour**, qui planifie les recherches ;
- **Windows Update** lui-même ;
- **BITS** et l'**Optimisation de la distribution**, qui font les téléchargements.

Pour chacun, il affiche son **état**, la **RAM** qu'il occupe et ce qu'on **perd** tant qu'il est
arrêté. « Tout arrêter » les arrête, puis « Tout relancer » relance exactement ceux que Windows Update Stopper
a arrêtés, rien d'autre. Windows Update Stopper affiche la RAM réellement libérée.

Il consomme le moins possible :

- Un seul fichier `WindowsUpdateStopper.exe` d'environ 370 Ko, sans installation ni runtime à ajouter.
- Environ 2 Mo de mémoire privée, et rien du processeur quand sa fenêtre est réduite.
- Win32 pur, comme GameMonitor : aucune autre dépendance que la caisse `windows`.

## Utilisation

1. Copier `WindowsUpdateStopper.exe` sur le PC de jeu (n'importe quel dossier).
2. Le lancer : Windows demande les droits administrateur (sans eux, il refuse d'arrêter ses services).
3. Les quatre services sont cochés : un service décoché reste en marche avec « Tout arrêter ». Les
   cases sont retenues d'un lancement à l'autre. Cocher un service ne l'arrête pas : en haut à
   droite, Windows Update Stopper indique la RAM que « Tout arrêter » libérerait avec les cases cochées.
4. **Tout arrêter** avant de lancer le jeu, **Tout relancer** après.

Pour un seul service : le sélectionner, puis **Arrêter** (ou **Relancer**) dans le panneau du bas, ou
clic droit sur sa ligne.

La liste se met à jour toutes les 3 s. Un clic sur un titre de colonne la trie : par nom, état ou
RAM (la plus grosse d'abord). Le panneau du bas détaille le service sélectionné : son nom Windows,
ce qu'on perd, la RAM qu'il occupe et la description de Windows.

### Mode jeu

« Tout arrêter » passe en mode jeu. Laisser Windows Update Stopper ouvert (réduit dans la barre des tâches) :
toutes les 5 s, il arrête les services cochés que Windows a démarrés ou relancés entre-temps ;
fenêtre ouverte, il les arrête même dès que la liste en voit un. En haut à droite, il note le
dernier, avec l'heure : « BITS relancé par Windows à 14 h 32 : arrêté de nouveau. » Chaque
vérification ne lit que l'état de ces services : un quart de milliseconde, dix fois moins que la
mise à jour de la liste quand la fenêtre est ouverte.
Windows relance volontiers Windows Update de lui-même, pour une recherche planifiée ou à la demande
d'une appli : tout programme a le droit de le démarrer. « Tout relancer » les relance aussi.

Un service relancé à la main pendant le mode jeu, ou coché alors qu'il tourne, est laissé
tranquille jusqu'au prochain « Tout arrêter ».

Un service que Windows relance sans arrêt (arrêté 3 fois en 2 minutes, il tourne encore) est
laissé tourner, même par « Tout arrêter », jusqu'à la fermeture de Windows Update Stopper : son état
passe en orange et son détail l'explique. Mieux vaut alors le décocher.

### La RAM affichée

C'est la RAM **privée** du processus du service : celle qu'il libère en s'arrêtant, comme la colonne
Mémoire du Gestionnaire des tâches. La plupart des services n'en occupent que quelques mégaoctets :
le reste de leur mémoire est partagé avec Windows et reste en place. « partagée » : le processus
héberge d'autres services en cours, sa RAM n'est libérée que s'ils s'arrêtent tous. Après un arrêt,
Windows Update Stopper compte la RAM des processus qui se sont vraiment fermés.

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
- Les services arrêtés sont notés dans `%LOCALAPPDATA%\WindowsUpdateStopper\etat.txt`, après chaque arrêt.
  Si Windows Update Stopper plante ou est fermé, le lancement suivant les retrouve, et les relance tout seul si
  Windows a redémarré depuis. Les cases décochées sont dans `choix.txt`, au même endroit. Seuls les
  services du catalogue sont relus dans ces fichiers : les modifier ne peut rien faire arrêter ni
  relancer d'autre à Windows Update Stopper.
- Le catalogue est dans le code (`src/catalog.rs`), pas dans un fichier de configuration : Windows Update Stopper
  tourne en administrateur, et un fichier modifiable à côté de l'exe ferait le jeu d'un logiciel
  malveillant. Les tests vérifient qu'aucun service de la liste « jamais touchés » n'y figure.
- Un seul Windows Update Stopper à la fois : le relancer réaffiche la fenêtre déjà ouverte.
- Comme GameMonitor, il ne charge ses DLL que depuis System32.

## Le catalogue

Dans l'ordre où « Tout arrêter » les arrête : l'orchestrateur d'abord, sinon il relancerait Windows
Update.

| Service | Nom technique | Tant qu'il est arrêté |
|---|---|---|
| Orchestrateur des mises à jour | `UsoSvc` | Windows ne planifie plus de recherche de mises à jour. |
| Windows Update | `wuauserv` | Aucune mise à jour Windows téléchargée ni installée pendant la partie. |
| Transfert intelligent en arrière-plan (BITS) | `BITS` | Les téléchargements de Windows et de certaines applis attendent. |
| Optimisation de la distribution | `DoSvc` | L'appli Xbox et le Microsoft Store ne téléchargent plus : mets tes jeux à jour avant. |

Le service de réparation de Windows Update (`WaaSMedicSvc`) n'y est pas : il ne tourne presque
jamais, et Windows Update Stopper ne change rien qu'il aurait à réparer.

## Jamais touchés

- Xbox, Gaming Services, GameInput, compte Microsoft et Microsoft Store : la connexion Xbox de
  MSFS 2024, le Game Pass et les sauvegardes en ligne en dépendent.
- Son, réseau, VPN, Teredo et UPnP (multijoueur), Bluetooth et HID (manettes, casques).
- Affichage, saisie au clavier, pilote NVIDIA, Windows Defender, pare-feu, notifications.
- Office (`ClickToRunSvc`), Steam, Epic, anti-triche (EasyAntiCheat, BattlEye, Vanguard), Sunshine
  (diffusion du jeu), et WMI, dont GameMonitor se sert.

## À quoi s'attendre

Arrêter Windows Update ne fait pas gagner d'images par seconde, et libère peu de RAM. Le vrai gain :
aucune mise à jour qui se télécharge ou s'installe en pleine partie, donc moins d'activité disque,
réseau et processeur en arrière-plan, et moins de saccades. MSFS 2024, qui télécharge son décor
pendant le vol, garde toute la bande passante.

## Compiler

Il faut Rust (`rustup`, chaîne MSVC) et les outils de compilation C++ de Visual Studio.

```bash
cargo build --release
```

L'exe se trouve dans `target/release/WindowsUpdateStopper.exe`. Le CRT est lié statiquement (`.cargo/config.toml`),
donc l'exe n'a pas besoin du Visual C++ Redistributable.

Pour tester sans la demande d'administrateur : `cargo run` (debug), ou définir `WINDOWS_UPDATE_STOPPER_NO_ADMIN=1`
avant `cargo build --release`. La liste s'affiche alors en lecture seule.

Tests : `cargo test`. Ils couvrent le catalogue (aucun service protégé, pas de doublon, textes qui
tiennent dans la fenêtre), le calcul de la RAM libérable et libérée, le tri, l'état sur disque, les
textes de la fenêtre et le minuteur du mode jeu, et lisent, sans rien arrêter, le vrai gestionnaire
de services et la mémoire des processus. L'un d'eux vérifie sur le vrai Windows que le catalogue
arrête toujours un service avant ceux dont il dépend.

L'intégration continue ([`.github/workflows/windows-update-stopper.yml`](../../.github/workflows/windows-update-stopper.yml))
vérifie la mise en forme, Clippy, les tests et la compilation avec Rust 1.95.0 et la version stable.
Chaque exécution joint `WindowsUpdateStopper.exe` à ses artefacts, à télécharger pour le PC de jeu.

## Structure

```
src/
  main.rs       démarrage, instance unique
  catalog.rs    les services de Windows Update, et ceux jamais touchés
  services.rs   gestionnaire de services de Windows : liste, description, arrêt, relance
  memory.rs     RAM privée de chaque processus, RAM du PC
  model.rs      lignes de la liste, RAM libérable et libérée, tri
  state.rs      services arrêtés et cases décochées, gardés sur disque
  ui.rs         fenêtre : boutons, état, détail, travail en arrière-plan, mode jeu
  listview.rs   liste Windows à cases à cocher, en thème sombre
  theme.rs      couleurs, polices, mesures
```
