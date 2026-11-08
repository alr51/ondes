#!/usr/bin/env bash
# VM Windows 11 de test (QEMU/KVM via libvirt, mode session : pas besoin de root).
#
#   scripts/windows-vm.sh create <Windows11.iso>   crée la VM et lance l'installation
#   scripts/windows-vm.sh share <fichier>…         met des fichiers sur le lecteur D: de la VM
#   scripts/windows-vm.sh view                     ouvre l'écran de la VM
#   scripts/windows-vm.sh start | stop             démarre / éteint la VM
#   scripts/windows-vm.sh destroy                  supprime la VM et son disque
#
# Matériel : UEFI + Secure Boot, TPM 2.0 émulé (exigés par Windows 11), disque
# SATA et réseau e1000e (reconnus sans pilote). Le disque (64 Go maximum) ne
# consomme que l'espace réellement écrit. Rendu graphique : la VM n'a pas de
# GPU, Windows utilise le rendu DirectX logiciel (WARP) — lent mais suffisant
# pour vérifier qu'Ondes démarre.
# Mémoire : 4 Go par défaut (minimum de Windows 11), ONDES_VM_RAM=<Mo> pour changer.
set -euo pipefail

VM=ondes-win11
DIR="${ONDES_VM_DIR:-$HOME/vms}"
DISK="$DIR/$VM.qcow2"
SHARE="$DIR/$VM-partage.iso"
CONN=qemu:///session
virsh() { command virsh -c "$CONN" "$@"; }

make_share() {
    local tmp
    tmp=$(mktemp -d)
    if [ $# -eq 0 ]; then
        echo "Utilise 'scripts/windows-vm.sh share <fichier>' pour déposer des fichiers ici." > "$tmp/LISEZ-MOI.txt"
    else
        cp -- "$@" "$tmp/"
    fi
    xorriso -as mkisofs -quiet -J -joliet-long -R -V PARTAGE -o "$SHARE" "$tmp"
    rm -rf "$tmp"
}

case "${1:-}" in
create)
    iso="${2:?chemin de l’ISO Windows 11 requis}"
    mkdir -p "$DIR"
    [ -e "$DISK" ] || qemu-img create -q -f qcow2 "$DISK" 64G
    make_share
    virt-install --connect "$CONN" \
        --name "$VM" \
        --osinfo win11 \
        --memory "${ONDES_VM_RAM:-4096}" --vcpus 4 --cpu host-passthrough \
        --boot uefi,firmware.feature0.name=secure-boot,firmware.feature0.enabled=yes,firmware.feature1.name=enrolled-keys,firmware.feature1.enabled=yes \
        --tpm backend.type=emulator,backend.version=2.0,model=tpm-crb \
        --disk "path=$DISK,bus=sata,boot.order=2" \
        --disk "path=$iso,device=cdrom,bus=sata,boot.order=1" \
        --disk "path=$SHARE,device=cdrom,bus=sata" \
        --network user,model=e1000e \
        --graphics spice --video qxl \
        --sound ich9 \
        --import \
        --noautoconsole
    echo "VM créée. Écran : scripts/windows-vm.sh view"
    ;;
share)
    shift
    [ $# -gt 0 ] || { echo "fichier(s) à partager requis" >&2; exit 1; }
    make_share "$@"
    # Troisième lecteur SATA (sdc) : le partage.
    virsh change-media "$VM" sdc "$SHARE" --update --live --config >/dev/null 2>&1 \
        || virsh change-media "$VM" sdc "$SHARE" --update --config >/dev/null
    echo "Fichiers disponibles dans la VM sur le lecteur « PARTAGE » : $*"
    ;;
view) virt-viewer --connect "$CONN" "$VM" & ;;
start) virsh start "$VM" ;;
stop) virsh shutdown "$VM" ;;
destroy)
    virsh destroy "$VM" 2>/dev/null || true
    virsh undefine "$VM" --nvram --tpm 2>/dev/null || virsh undefine "$VM" --nvram
    rm -f "$DISK" "$SHARE"
    ;;
*) sed -n '2,15p' "$0"; exit 1 ;;
esac
