//! Accès au runtime tokio depuis GPUI.

use std::future::Future;
use std::sync::Arc;

use gpui_kit::{App, Global};
use ondes_core::rt::{AsyncRuntime, RtError, Task};

/// Runtime tokio partagé, enregistré comme `Global` GPUI.
#[derive(Clone)]
pub struct Rt(Arc<AsyncRuntime>);

impl Global for Rt {}

impl Rt {
    pub fn start() -> Result<Self, RtError> {
        let workers = std::thread::available_parallelism()
            .map(|n| n.get().min(4))
            .unwrap_or(2);
        Ok(Self(Arc::new(AsyncRuntime::new(workers)?)))
    }

    /// Lance `fut` sur tokio ; le [`Task`] s'attend dans un `cx.spawn` GPUI.
    pub fn spawn<F>(cx: &App, fut: F) -> Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        cx.global::<Rt>().0.spawn(fut)
    }

    /// Handle tokio, pour les crates qui ouvrent des connexions elles-mêmes.
    pub fn handle(cx: &App) -> tokio::runtime::Handle {
        cx.global::<Rt>().0.handle()
    }
}
