//! Pont entre le runtime tokio et l'exécuteur de l'UI.
//!
//! GPUI possède son propre exécuteur, mais plusieurs crates (reqwest, etc.)
//! exigent un contexte tokio. [`AsyncRuntime`] fait tourner un runtime tokio
//! sur ses propres threads ; [`AsyncRuntime::spawn`] y lance une tâche et
//! renvoie un [`Task`] qui peut être attendu depuis n'importe quel exécuteur
//! (typiquement dans `cx.spawn` côté GPUI), car le canal `oneshot` de tokio
//! ne dépend pas du runtime pour être interrogé.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::runtime::{Builder, Handle, Runtime};
use tokio::sync::oneshot;

/// Erreurs du pont async.
#[derive(Debug, thiserror::Error)]
pub enum RtError {
    #[error("impossible de démarrer le runtime tokio : {0}")]
    Start(#[from] std::io::Error),
    #[error("la tâche a été annulée ou a paniqué")]
    Canceled,
}

/// Runtime tokio dédié aux E/S réseau et disque.
pub struct AsyncRuntime {
    runtime: Runtime,
}

impl AsyncRuntime {
    /// Démarre un runtime multi-thread avec `workers` threads (2 minimum).
    pub fn new(workers: usize) -> Result<Self, RtError> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(workers.max(2))
            .thread_name("ondes-io")
            .enable_all()
            .build()?;
        Ok(Self { runtime })
    }

    /// Handle clonable, à passer aux crates qui en ont besoin.
    pub fn handle(&self) -> Handle {
        self.runtime.handle().clone()
    }

    /// Lance `fut` sur le runtime tokio. Le résultat est disponible via le
    /// [`Task`] renvoyé, attendable depuis un autre exécuteur.
    pub fn spawn<F>(&self, fut: F) -> Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        spawn_on(&self.runtime.handle().clone(), fut)
    }
}

/// Variante de [`AsyncRuntime::spawn`] à partir d'un [`Handle`].
pub fn spawn_on<F>(handle: &Handle, fut: F) -> Task<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    let join = handle.spawn(async move {
        // Le récepteur peut avoir été abandonné : ce n'est pas une erreur.
        let _ = tx.send(fut.await);
    });
    Task { rx, join }
}

/// Sonde de démarrage : n'aboutit que dans un contexte tokio (timer).
/// Renvoie le temps de l'aller-retour.
pub async fn probe() -> std::time::Duration {
    let start = std::time::Instant::now();
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    start.elapsed()
}

/// Résultat d'une tâche tokio, attendable hors du runtime.
///
/// Abandonner le `Task` annule la tâche tokio sous-jacente.
pub struct Task<T> {
    rx: oneshot::Receiver<T>,
    join: tokio::task::JoinHandle<()>,
}

impl<T> Future for Task<T> {
    type Output = Result<T, RtError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.rx)
            .poll(cx)
            .map(|res| res.map_err(|_| RtError::Canceled))
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        self.join.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Exécute un future sur le thread courant, sans tokio, comme le ferait
    /// l'exécuteur GPUI.
    fn block_on_foreign<F: Future>(fut: F) -> F::Output {
        use std::sync::Arc;
        use std::task::{Wake, Waker};
        use std::thread::{self, Thread};

        struct ThreadWaker(Thread);
        impl Wake for ThreadWaker {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }

        let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
        let mut cx = Context::from_waker(&waker);
        let mut fut = std::pin::pin!(fut);
        loop {
            match fut.as_mut().poll(&mut cx) {
                Poll::Ready(v) => return v,
                Poll::Pending => thread::park(),
            }
        }
    }

    #[test]
    fn task_resolves_outside_tokio() {
        let rt = AsyncRuntime::new(2).unwrap();
        let task = rt.spawn(async {
            // Requiert un contexte tokio : paniquerait hors du runtime.
            tokio::time::sleep(Duration::from_millis(20)).await;
            42
        });
        assert_eq!(block_on_foreign(task).unwrap(), 42);
    }

    #[test]
    fn panicking_task_reports_canceled() {
        let rt = AsyncRuntime::new(2).unwrap();
        let task = rt.spawn(async {
            panic!("boom");
        });
        assert!(matches!(block_on_foreign(task), Err(RtError::Canceled)));
    }

    #[test]
    fn dropping_task_aborts_work() {
        let rt = AsyncRuntime::new(2).unwrap();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let task = rt.spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let _ = done_tx.send(());
        });
        drop(task);
        assert!(done_rx.recv_timeout(Duration::from_millis(400)).is_err());
    }
}
