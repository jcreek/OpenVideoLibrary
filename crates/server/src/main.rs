use ovl_server::{
    App, Config,
    transport::{CancellableListener, PeerAddress},
};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|a| a == "--version-json") {
        println!(
            "{}",
            serde_json::json!({"server_version":env!("CARGO_PKG_VERSION"),"sqlite_version":rusqlite::version(),"os":std::env::consts::OS,"architecture":std::env::consts::ARCH})
        );
        return Ok(());
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ovl_server=info,tower_http=info".into()),
        )
        .init();
    let config = Config::from_env();
    let listen = config.listen.clone();
    let app = tokio::task::spawn_blocking(move || App::open(config)).await??;
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    tracing::info!(%listen,"Rust preview listening; hardware and browser certification pending");
    app.background();
    let shutdown = app.clone();
    axum::serve(
        CancellableListener::new(listener, app.cancel.clone()),
        app.router()
            .into_make_service_with_connect_info::<PeerAddress>(),
    )
    .with_graceful_shutdown(async move {
        #[cfg(unix)]
        {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("SIGTERM handler");
            tokio::select! { _=tokio::signal::ctrl_c()=>{}, _=term.recv()=>{} }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        shutdown.cancel.cancel();
    })
    .await?;
    app.shutdown().await;
    Ok(())
}
