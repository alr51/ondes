# Journal des modifications

Les notes de chaque release GitHub sont tirées de ce fichier par
`scripts/release-notes.py` (le tableau des téléchargements est ajouté
automatiquement). Avant de publier une version, renommer la section
« Non publié » en `## [x.y.z] — AAAA-MM-JJ`.

## [Non publié]

## [0.7.0] — 2026-11-11

### Historique d'écoute
- Nouvelle page « Historique » (barre latérale, Ctrl+7) : chaque titre annoncé
  par une radio, et chaque morceau écouté au moins 30 s (ou la moitié s'il est
  plus court), avec l'heure, regroupés par jour.
- Recherche sans accents ni casse ; clic pour relancer la radio ou le morceau ;
  copier « artiste - titre », retirer une entrée, tout effacer.
- Les 10 000 dernières écoutes sont gardées ; historique en pause depuis la
  palette (« Activer / désactiver l'historique »).

### Infos sur les artistes, albums et titres
- Nouvelle page « Infos » (barre latérale, Ctrl+I pour ce qui joue), ouverte
  aussi depuis la barre de lecture, « En cours », l'historique et les fiches
  album / artiste de la bibliothèque. On passe d'une fiche à l'autre (titre →
  album → artiste) avec retour arrière.
- Artiste : photo, origine, années d'activité, genres, biographie Wikipédia en
  français (sinon en anglais), discographie (albums, EP, autres parutions)
  avec les albums de la bibliothèque marqués.
- Album : pochette, date, label, durée, pistes ; « Écouter l'album » s'il est
  dans la bibliothèque.
- Titre : durée, première sortie, crédits (composition, paroles, chant,
  instruments, production…), albums où il paraît.
- Sources : MusicBrainz (une requête par seconde, comme demandé), Cover Art
  Archive, Wikidata et Wikipédia ; réponses et images gardées deux semaines en
  cache (`~/.cache/ondes/info`, un jour pour une recherche sans résultat), puis
  supprimées au démarrage.
- Recherche tolérante aux écarts des titres annoncés par les radios
  (apostrophes, tirets, « feat. », précisions entre parenthèses).

### Titres des radios Radio France
- FIP et ses webradios, France Inter, franceinfo, France Musique et ses
  webradios, France Culture, Mouv' et les 45 stations ICI affichent le morceau
  ou l'émission en cours : leurs flux n'ont pas de métadonnées ICY, le titre
  vient de l'API publique de Radio France, interrogée à la fin de chaque
  morceau (au plus toutes les 15 s, au moins toutes les 3 min).
- Ces titres entrent aussi dans l'historique.

## [0.6.0] — 2026-11-09

### Visualiseur
Des effets visuels animés par la musique, calculés par la carte graphique.

- Le visualiseur occupe toute la fenêtre (**mode immersif**) : bouton ✨ de la barre de lecture, « Mode immersif » de la page En cours, palette de commandes ou **Ctrl+Maj+V**.
- Quatre effets : **Nébuleuse**, **Anneau**, **Tunnel** et **Ondes**. Ils réagissent au spectre, au volume et aux temps forts de la musique.
- **Plein écran** avec F11 ou un double-clic ; Échap quitte le plein écran, puis le mode immersif. Les flèches ← → changent d'effet ; les commandes s'effacent quand la souris ne bouge plus.
- Le visualiseur est aussi un mode de la vue compacte : un clic passe à l'effet suivant.

### Page « En cours »
Une nouvelle page **En cours** dans la fenêtre principale (barre latérale, palette de commandes ou **Ctrl+0**) affiche en grand ce qui est joué, au choix : pochette ou logo de la radio, spectre, visualiseur, titre, file d'attente. Le mode choisi est mémorisé.

### Une seule fenêtre
La vue compacte et le visualiseur ne s'ouvrent plus dans des fenêtres à part : c'est la fenêtre principale qui change de disposition.

- **Vue compacte** (Ctrl+Maj+M, bouton de la barre de lecture) : la fenêtre se réduit au lecteur, puis reprend sa taille. La vue compacte s'affiche aussi d'elle-même quand on rétrécit la fenêtre.
- **Mode immersif** (Ctrl+Maj+V) : le visualiseur seul ; Échap ou le bouton « Vue principale » pour revenir à la vue précédente.

### Page d'accueil
L'adresse à écouter et l'ouverture de fichier ne sont plus en haut de chaque page : elles ont leur page **Accueil**, ouverte d'un clic sur le logo d'Ondes, avec **Ctrl+L** ou la palette de commandes. C'est la page affichée au premier lancement. **Ctrl+O** ouvre toujours un fichier depuis n'importe quelle page.

### Barre latérale pliable
La barre latérale se replie pour ne garder que ses icônes, et laisser plus de place aux pages : bouton à gauche du titre de la page, palette de commandes ou **Ctrl+B**. Le nom de chaque page s'affiche au survol, et l'état est mémorisé.

### Améliorations
- Ondes consomme moins de processeur pendant la lecture : le spectre de la barre de lecture ne redessine plus toute la fenêtre à chaque image de l'écran.
- Le visualiseur est plus net sur les grands écrans : les effets sont calculés jusqu'à 1920 pixels de large (800 auparavant).

## [0.5.0] — 2026-11-08

### Vue compacte
Une petite fenêtre du lecteur, pour écouter sans la fenêtre principale : bouton à côté du volume dans la barre de lecture, palette de commandes ou **Ctrl+Maj+M**. La fenêtre principale est réduite et revient quand la vue compacte est fermée.

- Quatre modes d'affichage : pochette (ou logo de la radio), spectre, titre en grand, file d'attente.
- Lecture, morceau précédent et suivant, volume, favori et enregistrement de la radio.
- Le mode choisi, la taille et la position de la fenêtre sont mémorisés.

## [0.4.0] — 2026-11-08

### Réglages audio
Une nouvelle fenêtre **Réglages audio** regroupe les options d'écoute : icône en bas de la barre latérale, palette de commandes (Ctrl+K) ou **Ctrl+,**.

### Égaliseur
- 10 bandes, de 31 Hz à 16 kHz, réglables de −12 à +12 dB.
- Préréglages : Neutre, Graves, Aigus, Voix, Loudness.
- L'effet est immédiat, sur la musique comme sur les radios. Le volume est réduit d'autant que la bande la plus amplifiée, pour éviter la saturation.

### Normalisation du volume (ReplayGain)
Les morceaux de la bibliothèque peuvent être joués à un volume homogène, d'après leurs tags ReplayGain.

- **Par morceau** : chaque morceau au même niveau.
- **Par album** : garde les écarts voulus entre les morceaux d'un album.

Les fichiers sans tags ReplayGain et les radios ne sont pas modifiés.

### Enchaînement des morceaux
- Le morceau suivant est préparé à l'avance : les morceaux s'enchaînent sans blanc.
- **Fondu enchaîné** réglable de 0,5 à 10 secondes. Par défaut, il n'est pas appliqué entre deux morceaux d'un même album, pour respecter les albums enchaînés et les concerts. Un interrupteur permet de l'activer quand même.

### Spectre
Un spectre animé s'affiche dans la barre de lecture, à côté du volume. Il peut être masqué dans les réglages audio.

### Enregistrement des radios
- Pendant l'écoute d'une radio, le bouton **○** à côté du cœur (ou **Ctrl+Maj+R**) enregistre le direct.
- Le son est copié tel quel, sans perte de qualité, dans `Musique/Ondes` (dossier modifiable dans les réglages audio).
- L'enregistrement s'arrête au changement de station ou à la pause. Une notification indique le fichier créé.
- Fonctionne avec les radios MP3, AAC et HLS.

## [0.3.0] — 2026-11-08

### Flux HLS (m3u8)
Ondes lit désormais les radios diffusées en HLS, un format de plus en plus courant (Radio France, RTL…). Jusqu'ici, ces stations étaient refusées.

- Collez l'URL `.m3u8` d'une radio, ou lancez-la depuis l'annuaire ou vos favoris : elle se lit comme n'importe quelle autre.
- Quand la radio propose plusieurs qualités, Ondes choisit automatiquement la meilleure version audio.
- Le direct est suivi en continu, avec reconnexion automatique en cas de coupure.
- Les flux chiffrés ne sont pas pris en charge et sont signalés par un message clair.

Les titres en cours ne s'affichent pas encore pour ces radios.

## [0.2.1] — 2026-11-08

### Vérification des mises à jour
Ondes vous prévient désormais quand une nouvelle version est disponible.

- Au démarrage, au plus une fois par jour, Ondes consulte la dernière version publiée sur GitHub. Si elle est plus récente, une notification s'affiche avec un bouton **Télécharger** qui ouvre la page de la release.
- Vérification à la demande : **« Rechercher les mises à jour »** dans la palette de commandes (Ctrl+K) ou dans la fenêtre **À propos**.
- Pour ne plus être prévenu : **« Activer / désactiver la vérification des mises à jour »** dans la palette.

L'installation reste manuelle : téléchargez le paquet adapté à votre système ci-dessous.

> Si vous utilisez la v0.2.0, installez cette version à la main : la v0.2.0 ne contient pas encore la vérification. Les prochaines versions vous seront signalées automatiquement.

## [0.2.0] — 2026-11-08

### Ondes sous Windows
Ondes fonctionne désormais sous Windows 10 et 11, avec un installateur.

- Application Windows à part entière : icône, pas de fenêtre de console au lancement.
- Touches multimédia du clavier et panneau média de Windows : lecture, pause, morceau suivant et précédent, titre en cours et pochette.
- Un test de démarrage sous Windows est désormais lancé automatiquement à chaque modification du code.

### macOS
Un paquet `.dmg` pour les Mac Apple Silicon (M1 et suivants) est désormais publié. Il n'a pas encore été testé.

## [0.1.0] — 2026-11-07

Première version publiée, pour Linux (`.deb` et AppImage).

- Webradios : recherche dans l'annuaire Radio Browser (filtres pays, langue, genre), favoris, import de playlists `.m3u` / `.pls`, titre en cours.
- Bibliothèque locale : scan des dossiers, vues Morceaux, Albums, Artistes et Dossiers, recherche instantanée, file d'attente, aléatoire et répétition.
- Intégration au bureau : touches multimédia, notifications, raccourcis clavier et palette de commandes (Ctrl+K).
