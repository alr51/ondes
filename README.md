# Ondes

[![CI](https://github.com/alr51/ondes/actions/workflows/ci.yml/badge.svg)](https://github.com/alr51/ondes/actions/workflows/ci.yml)

Écouter des webradios et explorer les sons de l'ordinateur local.
Rust + [GPUI](https://www.gpui.rs/) + [GPUI Kit](https://gpui-kit.com/).

Site : <https://alr51.github.io/ondes/> (sources dans `site/`). Voir [ROADMAP.md](ROADMAP.md).

## Installation

Paquets prêts à installer : [releases](https://github.com/alr51/ondes/releases).
Ou localement, dans `target/packages/` après `cargo packager --release`
(nécessite `cargo install cargo-packager --locked`) :

```sh
sudo apt install ./target/packages/ondes_*_amd64.deb   # Debian / Ubuntu
./target/packages/ondes_*_x86_64.AppImage              # autres distributions
```

Une étiquette `v*` poussée sur GitHub déclenche la CI de release
(`.deb`, AppImage, `.dmg`, installeur Windows).

## Compiler depuis les sources

### Prérequis (Linux, Debian/Ubuntu)

```sh
sudo apt install libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev libx11-xcb-dev \
  libwayland-dev libvulkan-dev libfontconfig-dev libfreetype-dev libasound2-dev pkg-config
```

La toolchain Rust (1.98) est fixée par `rust-toolchain.toml`.

### Lancer

```sh
cargo run                                   # RUST_LOG=ondes_audio=debug pour suivre le moteur
cargo run -- https://icecast.radiofrance.fr/fip-midfi.mp3
cargo run -- ~/Musique/morceau.flac
```

Dans l'application : coller une URL (flux mp3/aac/ogg, `.pls`, `.m3u`) et
appuyer sur Entrée, « Ouvrir un fichier… », ou glisser un fichier sur la fenêtre.
Les touches multimédia du clavier et le panneau média du bureau (MPRIS)
pilotent la lecture.

### Raccourcis

| Raccourci | Action |
|---|---|
| `Ctrl+K` ou `Ctrl+Maj+P` | Palette de commandes |
| `Ctrl+Espace` | Lecture / pause |
| `Alt+→` / `Alt+←` | Morceau suivant / précédent |
| `Alt+Maj+→` / `Alt+Maj+←` | Avancer / reculer de 10 s |
| `Ctrl+↑` / `Ctrl+↓` | Volume |
| `Ctrl+U` / `Ctrl+R` | Aléatoire / mode de répétition |
| `Ctrl+D` | Radio en cours : ajouter ou retirer des favoris |
| `Ctrl+L` / `Ctrl+O` | Écouter une URL / ouvrir un fichier |
| `Ctrl+1` … `Ctrl+6` | Pages (Explorer, Favoris, Morceaux, Albums, Artistes, Dossiers) |
| `Ctrl+Q` | Quitter |

## Structure

- `crates/app` : binaire GPUI (fenêtre, vues)
- `crates/core` : types partagés et pont tokio ↔ GPUI
- `crates/audio` : moteur audio (thread dédié, commandes / événements)
- `crates/sources` : fichiers locaux, flux HTTP avec reconnexion, ICY, playlists
- `crates/radio` : client Radio Browser, favoris SQLite, import de playlists
- `crates/library` : bibliothèque locale (scan incrémental, tags, pochettes, recherche)

## Données

- Favoris et index de la bibliothèque : `~/.local/share/ondes/ondes.db`
- État (volume, dernière écoute, fenêtre…) : `~/.local/share/ondes/state.toml`
- Journal : `~/.local/share/ondes/ondes.log` (session précédente : `ondes.log.1`)
- Cache des logos et pochettes : `~/.cache/ondes/{logos,covers}` (peut être supprimé sans risque)

## Développement

- `scripts/demo-music.sh` : bibliothèque de test (tags, pochettes, un fichier sans tags, un fichier cassé).
- `scripts/third-party.py` : régénère `THIRD-PARTY-LICENSES.md` (affiché dans « À propos »).

En build debug, `ONDES_PAGE=tracks|albums|artists|folders|favorites` ouvre
directement une page et `ONDES_PALETTE=1` ouvre la palette (captures d'écran
automatisées). Avec
`XDG_DATA_HOME` / `XDG_CACHE_HOME`, l'application utilise des données isolées.

## Licence

MIT ou Apache-2.0, au choix (`LICENSE-MIT`, `LICENSE-APACHE`). Les licences des
dépendances sont listées dans `THIRD-PARTY-LICENSES.md`.
