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
- [x] Résolution des URL `.pls` / `.m3u` vers le flux
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
- [x] Flux HLS (`.m3u8`) : choix de la variante audio (ou de la piste audio séparée), relecture de la playlist au fil du direct, segments MPEG-TS (démultiplexeur maison), AAC/MP3 bruts ou fMP4 — flux chiffrés refusés — validé à la main (FIP, RTL)
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
- [x] Touches multimédia et panneau média du bureau : MPRIS via `souvlaki` (zbus, sans libdbus) — lecture, pause, suivant, précédent, position, volume, titre, pochette
- [x] Notifications au changement de morceau / de titre radio, quand la fenêtre est en arrière-plan (désactivables)
- [x] Raccourcis clavier (actions GPUI, sans conflit avec la saisie) et palette de commandes (Ctrl+K)
- [x] Persistance (`state.toml`) : volume, dernière source (proposée à la reprise), page, thème, fenêtre, aléatoire, répétition
- [ ] Icône dans la zone de notification — reportée : GNOME n'en affiche pas sans extension, et MPRIS couvre l'essentiel
- [x] Raccourcis, palette, notifications, touches multimédia — validés à la main

### Phase 5 — Finitions et distribution
- [x] Erreurs lisibles : serveur introuvable, délai, refus, 404/403/5xx, TLS ; Opus signalé (dans l'annuaire et à la lecture)
- [x] Morceau illisible dans la file : notification et passage au suivant
- [x] Journal dans `~/.local/share/ondes/ondes.log` (session précédente en `.1`), plantages consignés avec trace
- [x] Paquets `.deb` et AppImage (`cargo-packager`, `Packager.toml`), icône, fichier `.desktop` (types audio, playlists)
- [x] CI de release (étiquette `v*`) : `.deb`/AppImage, `.dmg` (Apple Silicon), installeur NSIS — v0.2.0 publiée
- [x] Windows : exe sans console (manifeste, icône, CRT statique), touches multimédia (SMTC), test de démarrage en CI — validé à la main
- [ ] macOS : `.dmg` jamais testé, pas de build Intel
- [x] Page « À propos » : version, crédits, dossiers de données, licences des 617 dépendances (`scripts/third-party.py`)
- [x] Licence du projet : `LICENSE-MIT` et `LICENSE-APACHE`
- [ ] Flatpak — nécessite `flatpak-builder` et des sources Cargo vendorisées (`flatpak-cargo-generator`)
- [x] Mises à jour : vérification de la dernière release GitHub au démarrage (une fois par jour, désactivable) ou à la demande (palette, « À propos »), notification avec lien de téléchargement — pas d'installation automatique (le `.deb` demande root, les autres formats des paquets signés)
- [x] Installation du `.deb`, AppImage, passage d'un morceau illisible, « À propos » — validés à la main

### Phase 6 — Confort d'écoute
- [x] Égaliseur 10 bandes (biquads, ±12 dB, préampli automatique, préréglages), appliqué à la sortie : effet immédiat, même sur un direct
- [x] Normalisation ReplayGain (par morceau / par album, limitée par la crête), tags lus avec `lofty`
- [x] Morceau suivant préchargé : enchaînement sans blanc, ou fondu enchaîné réglable (0–10 s, pas entre deux morceaux d'un même album sauf réglage)
- [x] Spectre dans la barre de lecture (FFT maison, sans `rustfft`)
- [x] Enregistrement d'un direct (MP3/AAC/HLS, sans réencodage) dans `Musique/Ondes`, Ctrl+Maj+R
- [x] Fenêtre « Réglages audio » (barre latérale, palette, Ctrl+,)
- [x] Égaliseur, ReplayGain, fondu, spectre, enregistrement — validés à la main
- [x] Vue compacte : petite fenêtre du lecteur (fenêtre principale réduite), modes pochette / logo, spectre, titre, file d'attente ; commandes, volume, favori, enregistrement ; mode et position mémorisés — Ctrl+Maj+M, bouton de la barre de lecture — validée à la main
- [x] Visualiseurs par shaders (`ondes-visual`) : effets WGSL (Nébuleuse, Anneau, Tunnel, Ondes) rendus hors écran avec `wgpu` sur un thread dédié, relus et affichés comme image (GPUI n'accepte pas de shaders) ; entrées : 64 bandes, niveaux graves / médiums / aigus, temps forts, horloge musicale
- [x] Fenêtre « Visualiseur » (Ctrl+Maj+V) : plein écran (F11, double-clic), ← → pour changer d'effet, commandes qui s'effacent ; 5e mode de la vue compacte
- [x] Spectre de la barre de lecture à 30 images/s et pages en cache : de 31 % à 19 % de processeur pendant la lecture (release)
- [x] Page « En cours » dans la fenêtre principale (Ctrl+0) : mêmes modes que la vue compacte (pochette / logo, spectre, visualiseur, titre, file d'attente), composant partagé `stage.rs` ; le visualiseur s'arrête quand on quitte la page
- [x] Visualiseurs et page « En cours » — validés à la main
- [ ] « Toujours au-dessus » pour la vue compacte — GPUI ne le permet pas (pas d'état `_NET_WM_STATE_ABOVE`)
- [x] Visualiseur rendu jusqu'à 1920 px de large (800 auparavant)
- [x] Barre latérale pliable (icônes seules, Ctrl+B), mémorisée ; réglages, « À propos » et thème en liste ; statut du runtime async retiré (journal, notification en cas d'échec)
- [x] Une seule fenêtre : vue compacte et visualiseur deviennent des dispositions de la fenêtre principale (`Layout` dans `shell.rs`) ; vue compacte automatique sous 600 × 420 px ; mode immersif (Échap ou « Vue principale » pour revenir) — taille de chaque disposition mémorisée
- [x] Page d'accueil (clic sur le logo, Ctrl+L) : adresse à écouter et ouverture de fichier, retirées des autres pages ; titres des pages alignés sur le logo

### Phase 7 — Radio
- [x] Historique d'écoute (SQLite, table `history` de `ondes.db`) : titres ICY et morceaux écoutés ≥ 30 s, page « Historique » (Ctrl+7) par jour, recherche, relance, copie, effacement ; 10 000 entrées, pause possible

### Backlog
- Flatpak (Flathub) (voir phase 5)
- Installation automatique des mises à jour (`cargo-packager-updater`, paquets signés) pour AppImage, NSIS et `.dmg`
- Build macOS à vérifier, build Intel ou universel
- Icône dans la zone de notification (`tray-icon`) pour les bureaux qui en ont une
- Titres ICY des flux Radio France (pas d'ICY : API dédiée à étudier)
- Minuteur de mise en veille
- Édition des enregistrements de directs :
  - découpage sans réencodage (début / fin, découpe en plusieurs morceaux), aux frontières de trames MP3 / ADTS ;
  - découpe proposée d'après les changements de titre ICY notés pendant l'enregistrement ;
  - tags (titre, artiste, album, année, pochette) avec `lofty` ;
  - AAC brut (ADTS) : pas de tags possibles, remuxer en `.m4a` pour les porter.
- Fiche artiste : informations et discographie
  - accessible depuis la bibliothèque (vue Artistes), le morceau en cours et le titre ICY d'une radio ;
  - discographie (albums, singles, années) via MusicBrainz (gratuit, sans clé, 1 requête/s, User-Agent obligatoire), pochettes via Cover Art Archive ;
  - biographie et photo via Wikidata / Wikipédia (en français si disponible) ;
  - albums déjà présents dans la bibliothèque mis en évidence ; cache disque des réponses.
- Visualiseurs : plus d'effets, pochette intégrée aux effets, effets écrits par l'utilisateur (fichiers `.wgsl` rechargés à chaud)

## Risques
1. **GPUI et GPUI Kit évoluent vite** (0.6 → 0.9 en peu de temps) : versions fixées avec `=`, montées de version groupées.
2. **Flux radio hétérogènes** (redirections, ICY, HE-AAC, coupures) : garder un jeu de 20 à 30 URL de test.
3. **AAC** : symphonia ne décode que l'AAC-LC ; les flux HE-AAC (aacp) sont lus sans SBR (son plus terne). Piste : `fdk-aac` ou ffmpeg en option.
4. **Linux** : GPU compatible Vulkan requis ; audio via ALSA/PipeWire/PulseAudio (cpal). Tester tôt sur plusieurs machines.
5. **Grosses bibliothèques** (50 000+ morceaux) : listes virtualisées, pochettes asynchrones, indexation par lots.
