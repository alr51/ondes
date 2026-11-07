//! Sous Windows, intègre l'icône de l'application à `ondes.exe`.
//!
//! Le manifeste requis par GPUI (Common Controls v6, DPI par écran) n'est pas
//! ajouté ici : le script de build de `gpui-pre` l'intègre déjà au binaire
//! final (feature `windows-manifest`, active par défaut). Un second manifeste
//! ferait échouer l'édition de liens (CVT1100 : duplicate resource).

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/windows");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("../../packaging/windows/ondes.rc", embed_resource::NONE)
            .manifest_required()
            .expect("ressources Windows (icône)");
    }
}
