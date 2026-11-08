//! Boîte « À propos » : version, crédits, emplacement des données, licences.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Icon, Sizable, StyledExt, WindowExt as _, button::Button, h_flex, text::TextView,
    v_flex,
};
use gpui_kit::*;

/// Liste générée par `scripts/third-party.py`.
const THIRD_PARTY: &str = include_str!("../../../THIRD-PARTY-LICENSES.md");

pub fn open(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, _, _| {
        dialog
            .title("À propos d'Ondes")
            .w(px(600.))
            .content(|content, _, cx| {
                let theme = cx.theme();
                let data_dir = crate::paths::database()
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_default();
                let log = crate::paths::log_file();
                content.child(
                    v_flex()
                        .gap_4()
                        .child(
                            h_flex()
                                .gap_3()
                                .items_center()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size_12()
                                        .rounded(theme.radius_lg)
                                        .bg(theme.primary)
                                        .text_color(theme.primary_foreground)
                                        .child(Icon::new(IconName::RadioTower).large()),
                                )
                                .child(
                                    v_flex()
                                        .child(
                                            div().text_lg().font_semibold().child(format!(
                                                "Ondes {}",
                                                env!("CARGO_PKG_VERSION")
                                            )),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(theme.muted_foreground)
                                                .child("Webradios et musique locale, en Rust."),
                                        ),
                                ),
                        )
                        .child(div().text_sm().child(
                            "Interface : GPUI (Zed) et GPUI Kit (Longbridge). Décodage : \
                                 symphonia. Annuaire : Radio Browser. Icônes : Lucide. \
                                 Distribué sous licence MIT ou Apache-2.0.",
                        ))
                        .child(
                            v_flex()
                                .gap_1()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(format!("Données : {}", data_dir.display()))
                                .child(format!("Journal : {}", log.display())),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("open-data-dir")
                                        .icon(IconName::FolderOpen)
                                        .label("Ouvrir le dossier des données")
                                        .outline()
                                        .small()
                                        .on_click(move |_, _, cx| {
                                            cx.open_url(&format!("file://{}", data_dir.display()));
                                        }),
                                )
                                .child(
                                    Button::new("check-updates")
                                        .icon(IconName::Download)
                                        .label("Rechercher les mises à jour")
                                        .outline()
                                        .small()
                                        .on_click(|_, window, cx| {
                                            crate::update::check(true, window, cx)
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .id("third-party")
                                .h(px(260.))
                                .overflow_y_scroll()
                                .p_3()
                                .rounded(theme.radius)
                                .border_1()
                                .border_color(theme.border)
                                .text_xs()
                                .child(TextView::markdown("third-party-text", THIRD_PARTY)),
                        ),
                )
            })
    });
}
