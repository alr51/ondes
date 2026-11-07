#!/usr/bin/env python3
"""Génère THIRD-PARTY-LICENSES.md : les dépendances compilées dans Ondes,
regroupées par licence. À relancer après une mise à jour des dépendances :

    scripts/third-party.py [cible]     (défaut : x86_64-unknown-linux-gnu)
"""
import json, subprocess, sys
from collections import defaultdict
from pathlib import Path

target = sys.argv[1] if len(sys.argv) > 1 else "x86_64-unknown-linux-gnu"
root_dir = Path(__file__).resolve().parent.parent
meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target],
    cwd=root_dir,
))
packages = {p["id"]: p for p in meta["packages"]}
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
workspace = set(meta["workspace_members"])

# Parcours depuis le binaire, sans les dépendances de développement ni de build.
start = next(i for i in workspace if packages[i]["name"] == "ondes")
seen, stack = set(), [start]
while stack:
    pid = stack.pop()
    if pid in seen:
        continue
    seen.add(pid)
    for dep in nodes[pid]["deps"]:
        kinds = {k["kind"] for k in dep["dep_kinds"]}
        if None in kinds:  # dépendance normale
            stack.append(dep["pkg"])

by_license = defaultdict(list)
for pid in seen - workspace:
    p = packages[pid]
    lic = " ".join((p.get("license") or "voir le dépôt du projet").replace("/", " OR ").split())
    by_license[lic].append(f'{p["name"]} {p["version"]}')

lines = [
    "# Licences des dépendances",
    "",
    f"Ondes est distribué sous licence MIT OR Apache-2.0. Il inclut {len(seen - workspace)} "
    f"bibliothèques tierces (cible {target}), listées ci-dessous par licence.",
    "Les icônes proviennent de Lucide (ISC). L'annuaire de radios est fourni par",
    "Radio Browser (https://www.radio-browser.info).",
    "",
    "Les bibliothèques sous MPL-2.0 (décodeurs symphonia) sont utilisées sans",
    "modification ; leur code source est disponible sur https://crates.io.",
    "",
]
for lic in sorted(by_license, key=lambda l: (-len(by_license[l]), l)):
    names = sorted(set(by_license[lic]), key=str.lower)
    lines += [f"## {lic} ({len(names)})", "", ", ".join(names), ""]
(root_dir / "THIRD-PARTY-LICENSES.md").write_text("\n".join(lines))
print(f"{len(seen - workspace)} dépendances, {len(by_license)} licences → THIRD-PARTY-LICENSES.md")
