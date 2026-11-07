# Ondes — Roadmap

Application desktop pour écouter des webradios et explorer les sons de l'ordinateur local.
Stack : Rust, [GPUI](https://www.gpui.rs/) et [GPUI Kit](https://gpui-kit.com/) pour l'UI.

## Stack technique

| Besoin | Crate(s) |
|---|---|
| UI | `gpui-kit` (=0.9.0, embarque GPUI `gpui-pre` 0.3.11 — ne jamais lister `gpui` à part) |
| Décodage audio | `symphonia` (MP3, AAC, FLAC, Vorbis, Opus, WAV) |
| Sortie audio | `cpal` (ou `rodio` au début) |
| Streaming HTTP | `reqwest` (stream) + tampon circulaire (`ringbuf`) ou `stream-download` |
| Métadonnées ICY | parsing maison ou `icy-metadata` |
| Annuaire de radios | API Radio Browser (gratuite, sans clé) — `radiobrowser` |
| Playlists | `m3u` + parser PLS maison |
| Scan disque | `walkdir` / `jwalk`, `notify` pour la surveillance |
| Tags et pochettes | `lofty` |
| Base de données | `rusqlite` + FTS5 |
| Recherche floue | `nucleo` |
| Touches média / MPRIS | `souvlaki` |
| Config | `serde` + `toml`, `directories` |
| Async | runtime `tokio` dédié, relié à l'exécuteur GPUI par des canaux |
| Logs | `tracing` |

## Architecture (workspace Cargo)

```
crates/
├── app/       # binaire GPUI : fenêtres, vues, thème
├── core/      # types partagés, pont tokio, commandes/événements, config
├── audio/     # moteur audio sur thread dédié
├── sources/   # fichiers locaux, flux HTTP, playlists
├── radio/     # Radio Browser, favoris (SQLite), import de playlists
└── library/   # scan incrémental, tags, pochettes, index SQLite, recherche
```

Principes :
- **Thread audio dédié** : l'UI envoie des commandes (`Play`, `Pause`, `Seek`, `Volume`) par canal et reçoit des événements (`Position`, `Metadata`, `Buffering`, `Error`).
- **Trait `Source` commun** à la radio et aux fichiers locaux.
- **Async** : GPUI a son propre exécuteur ; tout ce qui demande tokio (`reqwest`…) tourne sur le runtime de `ondes-core::rt` et le résultat revient par un `oneshot` attendu dans `cx.spawn`.
- **État UI en `Entity<T>`** GPUI, mis à jour par les événements du moteur.

## Composants GPUI Kit par usage

| Usage | Composant |
|---|---|
| Navigation | `sidebar::Sidebar`, `SidebarGroup`, `SidebarMenu` |
| Thème clair/sombre | `Theme::change`, `ActiveTheme`, thèmes fournis |
| Barre de lecture | `button::Button`, `slider::Slider` (volume, progression) |
| Liste des stations | `VirtualList` / `list` |
| Recherche | `input` |
| Morceaux | `table` (DataTable) |
| Onglets / panneaux | `tab`, `dock` |
| Palette de commandes | `command`, `dialog` |
| Statut | `status_bar`, `notification`, `spinner`, `progress` |
| Icônes | `gpui_kit::assets::IconName` + `AllAssets` (catalogue Lucide complet) |

## Phases

### Phase 0 — Fondations
- [x] Workspace, versions GPUI/GPUI Kit fixées
- [x] CI (fmt, clippy, tests)
- [x] Fenêtre GPUI : barre latérale + contenu + barre de lecture
- [x] Thème clair/sombre (suit le système au démarrage, bascule manuelle)
- [x] Pont tokio ↔ GPUI testé
- **Livrable** : app vide qui s'ouvre, thème clair/sombre.

### Phase 1 — MVP lecteur
- [x] Moteur audio (play/pause/stop/volume) sur thread dédié — `ondes-audio` (rodio 0.23 / symphonia)
- [x] Décodage sur thread séparé + file d'échantillons : la sortie ne bloque jamais (silence + « mise en tampon »)
- [x] Lecture d'un fichier local : boîte de dialogue, glisser-déposer, `ondes <fichier>`
- [x] Lecture d'une URL de flux HTTP (MP3/AAC/OGG), prébuffer 2 s, reconnexion automatique (5 essais)
- [x] Résolution des URL `.pls` / `.m3u` vers le flux (HLS refusé proprement)
- [x] Barre de lecture branchée : titre, état, position/durée, stop, play/pause, volume
- [x] Pause/reprise d'un direct, boîte de dialogue fichier (portail XDG) — validés à la main
- **Livrable** : on colle une URL de radio ou on ouvre un fichier, et ça joue.

### Phase 2 — Webradios
- [x] Radio Browser : recherche par nom (avec délai de frappe), filtres pays / langue / genre, bascule entre miroirs
- [x] Liste virtualisée (`uniform_list`) avec chargement par pages de 100 au défilement
- [x] Logos : téléchargés pour les lignes visibles, réduits à 96 px, cache disque (`~/.cache/ondes/logos`)
- [x] Favoris (SQLite, `~/.local/share/ondes/ondes.db`) : ♡ dans les listes et la barre de lecture, ajout manuel
- [x] Métadonnées ICY : titre en cours, affiché au moment où il devient audible
- [x] Import `.m3u` / `.pls` (titres conservés, doublons ignorés)
- [x] Comptage des écoutes Radio Browser (recommandé par l'API)
- [x] Recherche, filtres, défilement infini, favoris, ajout manuel, import — validés à la main
- **Livrable** : parcourir, chercher et mettre en favori des milliers de stations.

### Phase 3 — Bibliothèque locale
- [x] Choix des dossiers (dialogue, ou `~/Musique` proposé), scan en arrière-plan avec progression
- [x] Tags (`lofty`) et pochettes (intégrées ou `cover.jpg`…, miniatures 300 px) indexés en SQLite
- [x] Recherche instantanée **en mémoire** (tous les mots, sans accents ni casse) — FTS5 et nucleo se sont révélés inutiles : 50 000 morceaux tiennent en quelques Mo
- [x] Vues Morceaux (tri par colonne) / Albums / Artistes (avec détail) / Dossiers
- [x] Scan incrémental (date + taille) + surveillance `notify` (rescan 2 s après un changement)
- [x] File d'attente, suivant/précédent, seek (barre de progression), aléatoire, répétition (tout / un)
- [x] Lecture depuis les listes, enchaînement, seek, aléatoire/répétition, ajout/retrait de dossier — validés à la main
- **Livrable** : bibliothèque musicale utilisable au quotidien.

### Phase 4 — Intégration système
- [ ] Touches média, MPRIS/SMTC (`souvlaki`)
- [ ] Notifications au changement de morceau
- [ ] Raccourcis clavier (actions GPUI) et palette de commandes
- [ ] Icône dans la zone de notification (optionnel)
- [ ] Persistance de l'état (dernière station, volume, fenêtre)

### Phase 5 — Finitions et distribution
- [ ] Gestion d'erreurs propre (flux morts, codecs non pris en charge)
- [ ] Paquets AppImage / Flatpak, `.dmg`, `.msi` (`cargo-packager`)
- [ ] Mise à jour automatique (optionnel), page « À propos », licences

### Backlog
- Titres ICY des flux Radio France (pas d'ICY : API dédiée à étudier)
- Flux HLS (m3u8)
- Égaliseur, ReplayGain, fondu enchaîné
- Enregistrement d'un flux dans un fichier
- Visualiseur (FFT, `rustfft`)
- Historique des titres entendus
- Minuteur de mise en veille

## Risques
1. **GPUI et GPUI Kit évoluent vite** (0.6 → 0.9 en peu de temps) : versions fixées avec `=`, montées de version groupées.
2. **Flux radio hétérogènes** (redirections, ICY, HE-AAC, coupures) : garder un jeu de 20 à 30 URL de test.
3. **AAC** : symphonia ne décode que l'AAC-LC ; les flux HE-AAC (aacp) sont lus sans SBR (son plus terne). Piste : `fdk-aac` ou ffmpeg en option.
4. **Linux** : GPU compatible Vulkan requis ; audio via ALSA/PipeWire/PulseAudio (cpal). Tester tôt sur plusieurs machines.
5. **Grosses bibliothèques** (50 000+ morceaux) : listes virtualisées, pochettes asynchrones, indexation par lots.
