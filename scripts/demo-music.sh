#!/usr/bin/env bash
# Génère une petite bibliothèque de test (tags, pochettes, un fichier sans tags).
# Usage : scripts/demo-music.sh [dossier]   (défaut : target/demo-music)
# Nécessite ffmpeg. Les sons sont des sinusoïdes de 20 à 50 s.
set -euo pipefail
M="${1:-$(dirname "$0")/../target/demo-music}"
rm -rf "$M"; mkdir -p "$M"
mk() { # artiste album année couleur nombre préfixe
  local d="$M/$1/$3 - $2"; mkdir -p "$d"
  ffmpeg -loglevel error -f lavfi -i "color=c=$4:s=600x600" -frames:v 1 "$d/cover.jpg"
  for i in $(seq 1 "$5"); do
    ffmpeg -loglevel error -f lavfi -i "sine=frequency=$((220 + i*55)):duration=$((20 + i*7))" \
      -i "$d/cover.jpg" -map 0:a -map 1:v -c:a libmp3lame -b:a 96k -c:v mjpeg \
      -id3v2_version 3 -disposition:v attached_pic \
      -metadata title="$6 $i" -metadata artist="$1" -metadata album="$2" \
      -metadata album_artist="$1" -metadata track="$i" -metadata date="$3" \
      -metadata genre="Test" "$d/0$i - $6 $i.mp3"
  done
  rm "$d/cover.jpg"
}
mk "Les Ondes Sinusoïdales" "Fréquences" 2021 darkorange 4 "Onde"
mk "Les Ondes Sinusoïdales" "Harmoniques" 2023 teal 3 "Harmonie"
mk "Beyoncé Test" "Déjà Vu Sessions" 2019 purple 3 "Session"
mkdir -p "$M/Divers"
ffmpeg -loglevel error -f lavfi -i "sine=frequency=500:duration=15" -c:a flac "$M/Divers/sans tags.flac"
# Fichier illisible, pour vérifier que la lecture passe au morceau suivant.
head -c 20000 /dev/urandom > "$M/Divers/fichier cassé.mp3"
echo "Bibliothèque de test : $(cd "$M" && pwd)"
