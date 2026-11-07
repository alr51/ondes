//! Ligne de station, partagée par l'exploration et les favoris.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Icon, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ondes_radio::Station;

/// Hauteur fixe : requise par la liste virtualisée.
pub const ROW_HEIGHT: f32 = 56.;

/// Action déclenchée depuis une ligne (lecture, favori).
pub type RowAction = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct StationRow {
    pub id: ElementId,
    pub station: Station,
    pub logo: Option<PathBuf>,
    pub favorite: bool,
    pub current: bool,
    pub on_play: RowAction,
    pub on_favorite: RowAction,
}

impl StationRow {
    pub fn render(self, cx: &App) -> Stateful<Div> {
        let theme = cx.theme();
        let on_play = self.on_play.clone();
        let on_favorite = self.on_favorite.clone();
        // Codecs que le moteur ne sait pas décoder : signalés d'avance.
        let unsupported = self
            .station
            .codec
            .as_deref()
            .is_some_and(|c| c.eq_ignore_ascii_case("opus"));
        let summary = match (unsupported, self.station.summary()) {
            (true, s) => format!("Opus non pris en charge · {s}"),
            (false, s) => s,
        };

        let fallback_color = theme.muted_foreground;
        let logo = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .size_10()
            .rounded(theme.radius)
            .bg(theme.muted)
            .text_color(theme.muted_foreground)
            .overflow_hidden()
            .child(match self.logo {
                Some(path) => img(path)
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .with_fallback(move || {
                        Icon::new(IconName::Radio)
                            .text_color(fallback_color)
                            .into_any_element()
                    })
                    .into_any_element(),
                None => Icon::new(IconName::Radio).into_any_element(),
            });

        h_flex()
            .id(self.id)
            .h(px(ROW_HEIGHT))
            .w_full()
            .px_3()
            .gap_3()
            .items_center()
            .rounded(theme.radius)
            .cursor_pointer()
            .when(self.current, |row| row.bg(theme.accent))
            .hover(|row| row.bg(theme.accent))
            .on_click(move |_, window, cx| on_play(window, cx))
            .child(logo)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .font_medium()
                            .truncate()
                            .when(self.current, |d| d.text_color(theme.primary))
                            .child(self.station.name.clone()),
                    )
                    .when(!summary.is_empty(), |col| {
                        col.child(
                            div()
                                .text_xs()
                                .text_color(if unsupported {
                                    theme.warning
                                } else {
                                    theme.muted_foreground
                                })
                                .truncate()
                                .child(summary),
                        )
                    }),
            )
            .when(self.current, |row| {
                row.child(
                    div()
                        .text_color(theme.primary)
                        .child(Icon::new(IconName::AudioLines).small()),
                )
            })
            .child(
                Button::new("fav")
                    .icon(if self.favorite {
                        Icon::new(IconName::Heart).text_color(theme.danger)
                    } else {
                        Icon::new(IconName::Heart)
                    })
                    .ghost()
                    .small()
                    .tooltip(if self.favorite {
                        "Retirer des favoris"
                    } else {
                        "Ajouter aux favoris"
                    })
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        on_favorite(window, cx);
                    }),
            )
    }
}
