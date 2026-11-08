//! Vérification des mises à jour : interroge la dernière release GitHub et,
//! si elle est plus récente, propose d'ouvrir sa page de téléchargement.
//!
//! Pas d'installation automatique : le `.deb` demande les droits root, et
//! les autres formats exigeraient des paquets signés. Vérification au
//! démarrage au plus une fois par jour (désactivable), ou à la demande.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::assets::IconName;
use gpui_kit::component::{WindowExt as _, button::Button, notification::Notification};
use gpui_kit::*;
use serde::Deserialize;

use crate::runtime::Rt;
use crate::settings::Settings;

const LATEST_RELEASE: &str = "https://api.github.com/repos/alr51/ondes/releases/latest";
const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// Intervalle minimal entre deux vérifications automatiques.
const INTERVAL: Duration = Duration::from_secs(24 * 3600);
/// Laisse l'interface et la lecture démarrer avant d'aller sur le réseau.
const STARTUP_DELAY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub html_url: String,
}

/// `v1.2.3` ou `1.2.3` (suffixe `-beta…` ignoré) → `(1, 2, 3)`.
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.trim().trim_start_matches('v');
    let core = text.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let version = (
        parts.next()??,
        parts.next()??,
        parts.next().unwrap_or(Some(0))?,
    );
    parts.next().is_none().then_some(version)
}

/// Vrai si `latest` est strictement plus récente que `current`.
fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

async fn fetch_latest() -> anyhow::Result<Release> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("Ondes/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .build()?;
    let release = client
        .get(LATEST_RELEASE)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json::<Release>()
        .await?;
    Ok(release)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Vérification automatique au démarrage : builds release seulement, si
/// activée et si la précédente date de plus de 24 h.
pub fn check_on_startup<T: 'static>(window: &mut Window, cx: &mut Context<T>) {
    let state = Settings::get(cx);
    let due = state
        .last_update_check
        .is_none_or(|last| now_secs().saturating_sub(last) >= INTERVAL.as_secs());
    if cfg!(debug_assertions) || !state.check_updates || !due {
        return;
    }
    cx.spawn_in(window, async move |_, cx| {
        cx.background_executor().timer(STARTUP_DELAY).await;
        cx.update(|window, cx| check(false, window, cx)).ok();
    })
    .detach();
}

/// Interroge GitHub. `manual` : l'utilisateur l'a demandé, on lui répond
/// aussi quand Ondes est à jour ou que la vérification échoue.
pub fn check(manual: bool, window: &mut Window, cx: &mut App) {
    let task = Rt::spawn(cx, fetch_latest());
    window
        .spawn(cx, async move |cx| {
            let result = match task.await {
                Ok(result) => result,
                Err(e) => Err(e.into()),
            };
            cx.update(|window, cx| {
                if result.is_ok() {
                    Settings::update(cx, |s| s.last_update_check = Some(now_secs()));
                }
                let note = match result {
                    Ok(release) if is_newer(&release.tag_name, CURRENT) => {
                        tracing::info!(version = %release.tag_name, "mise à jour disponible");
                        Some(available(release))
                    }
                    Ok(_) => {
                        manual.then(|| Notification::success(format!("Ondes {CURRENT} est à jour")))
                    }
                    Err(e) => {
                        tracing::warn!("vérification des mises à jour impossible : {e:#}");
                        manual.then(|| {
                            Notification::error(format!(
                                "Vérification des mises à jour impossible : {e}"
                            ))
                        })
                    }
                };
                if let Some(note) = note {
                    window.push_notification(note, cx);
                }
            })
            .ok();
        })
        .detach();
}

fn available(release: Release) -> Notification {
    let version = release.tag_name.trim_start_matches('v').to_string();
    let url = release.html_url;
    Notification::info(format!(
        "Ondes {version} est disponible (version installée : {CURRENT})."
    ))
    .title("Mise à jour disponible")
    .action(move |_, _, _| {
        let url = url.clone();
        Button::new("download-update")
            .icon(IconName::Download)
            .label("Télécharger")
            .on_click(move |_, _, cx| cx.open_url(&url))
    })
}

#[cfg(test)]
mod tests {
    use super::{Release, fetch_latest, is_newer, parse_version};

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("v0.2.0"), Some((0, 2, 0)));
        assert_eq!(parse_version("1.10.3"), Some((1, 10, 3)));
        assert_eq!(parse_version("v1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("v1.2.3-beta.1"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("nightly"), None);
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.3.0", "0.2.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("v1.0.0", "0.99.0"));
        assert!(!is_newer("v0.2.0", "0.2.0"));
        assert!(!is_newer("v0.1.5", "0.2.0"));
        assert!(!is_newer("n'importe quoi", "0.2.0"));
    }

    #[test]
    fn reads_github_release() {
        let json = r#"{"tag_name":"v0.2.0","html_url":"https://github.com/alr51/ondes/releases/tag/v0.2.0","assets":[]}"#;
        let release: Release = serde_json::from_str(json).unwrap();
        assert_eq!(release.tag_name, "v0.2.0");
    }

    /// Interroge vraiment GitHub : `cargo test -p ondes -- --ignored`.
    #[test]
    #[ignore]
    fn fetches_latest_release() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let release = rt.block_on(fetch_latest()).unwrap();
        assert!(parse_version(&release.tag_name).is_some());
    }
}
