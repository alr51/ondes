//! Sous Windows, intègre à `ondes.exe` le manifeste et l'icône.
//!
//! GPUI a besoin de Common Controls v6 et de la prise en charge du DPI par
//! écran, déclarés dans le manifeste. Le script de build de GPUI ne
//! l'applique qu'à ses propres exemples : sans ce fichier, l'exécutable
//! échoue au chargement, avant même d'ouvrir une fenêtre.

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/windows");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("../../packaging/windows/ondes.rc", embed_resource::NONE)
            .manifest_required()
            .expect("ressources Windows (manifeste, icône)");
    }
}
