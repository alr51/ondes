#!/usr/bin/env python3
"""Notes de release d'une version, tirées de CHANGELOG.md.

    scripts/release-notes.py 0.4.0      (ou v0.4.0)

Affiche la section `## [0.4.0]` du journal, suivie du tableau des
téléchargements. Échoue si la section est absente ou vide : une version ne
part pas sans notes.
"""
import re
import sys
from pathlib import Path

if len(sys.argv) != 2:
    sys.exit(__doc__)
version = sys.argv[1].removeprefix("v")
changelog = Path(__file__).resolve().parent.parent / "CHANGELOG.md"
text = changelog.read_text(encoding="utf-8")

match = re.search(
    rf"^## \[{re.escape(version)}\][^\n]*\n(.*?)(?=^## \[|\Z)",
    text,
    re.MULTILINE | re.DOTALL,
)
notes = match.group(1).strip() if match else ""
if not notes:
    sys.exit(f"CHANGELOG.md : pas de notes pour la version {version} (section « ## [{version}] »)")

print(f"""## Nouveautés

{notes}

## Téléchargements

| Système | Fichier |
|---|---|
| Linux (Debian, Ubuntu…) | `ondes_{version}_amd64.deb` |
| Linux (toutes distributions) | `ondes_{version}_x86_64.AppImage` |
| Windows 10 / 11 | `ondes_{version}_x64-setup.exe` |
| macOS (Apple Silicon) | `Ondes_{version}_aarch64.dmg` |

La version macOS n'a pas encore été testée.""")
