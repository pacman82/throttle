use tokio::signal::ctrl_c;

/// Registers signal handlers for termination and interrupt signals. I.e. this application will
/// gracefully shutdown with Ctrl+C as well as container stop.
///
/// Awaiting the result of this function will return a future which completes if a signal to
/// shutdown is received. I.e. after the first call to `await` the signal handlers are registered.
/// The second call to `await` waits for the signal itself.
pub async fn shutdown_signal() -> impl Future<Output = ()> {
    let ctrl_c = async {
        ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    use tokio::signal::unix;

    #[cfg(unix)]
    let terminate = async {
        unix::signal(unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    async move {
        tokio::select! {
            () = ctrl_c => {},
            () = terminate => {},
        }
    }
}
