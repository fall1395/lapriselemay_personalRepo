# Modules PawnIO intégrés à GameMonitor

Ces deux fichiers sont des modules officiels de PawnIO.Modules 0.2.11, tels que publiés et signés par
l'auteur de PawnIO : <https://github.com/namazso/PawnIO.Modules/releases/tag/0.2.11>.

| Fichier | Processeurs | SHA-256 |
|---|---|---|
| `IntelMSR.bin` | Intel | `D6ED85D65AB17A22F813EF98207D6D537155EE2DED5976A21CB48413C9B92E5F` |
| `AMDFamily17.bin` | AMD Ryzen (familles 17h à 1Ah) | `DAE74615761B78BDF064DFB3E136252DDCC6FC727D88F14738D0E5800D427A91` |

GameMonitor les intègre à l'exe, mais ne lit et ne confie au pilote PawnIO que celui du processeur,
pour lire sa sonde thermique. Chaque module n'autorise que l'accès à une liste de registres du
processeur (température, fréquence, consommation), et le pilote refuse tout module dont la signature
n'est pas valide.

- Sources : <https://github.com/namazso/PawnIO.Modules/tree/0.2.11> (`IntelMSR.p`, `AMDFamily17.p`)
- Licence : GNU LGPL 2.1 ou ultérieure, © namazso (texte complet dans `COPYING`).

Pour les mettre à jour : remplacer les fichiers par ceux d'une nouvelle version publiée, sans les
modifier (ils perdraient leur signature), puis mettre à jour ce texte et le test `modules_are_embedded`
de `src/metrics/pawnio.rs`.
