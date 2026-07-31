//! GarageDB server: API axum + UI embebida servida en LAN.

pub mod api;
pub mod appstate;
pub mod ui;

use std::path::PathBuf;

use appstate::AppState;

static PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(8080);

/// URL LAN del servidor (para el QR y la consola).
pub fn lan_url() -> String {
    let port = PORT.load(std::sync::atomic::Ordering::Relaxed);
    match local_ip_address::local_ip() {
        Ok(ip) => format!("http://{ip}:{port}"),
        Err(_) => format!("http://localhost:{port}"),
    }
}

/// Arranca el servidor: bind 0.0.0.0, imprime URLs y QR ASCII en consola.
pub async fn serve(root: PathBuf, port: u16) -> anyhow::Result<()> {
    PORT.store(port, std::sync::atomic::Ordering::Relaxed);
    let app = AppState::open(&root)?;
    let summary = app.summary()?;
    let router = api::router(app);

    let url = lan_url();
    println!("GarageDB — réplica {}", summary.replica);
    println!("Almacén: {}", root.display());
    println!("Eventos: {} (rechazados {}, propuestas {})", summary.n_events, summary.n_rejected, summary.n_proposals);
    println!("\n  Local:  http://localhost:{port}");
    println!("  LAN:    {url}   ← móvil por navegador\n");
    if let Ok(code) = qrcode::QrCode::new(url.as_bytes()) {
        let qr = code
            .render::<qrcode::render::unicode::Dense1x2>()
            .quiet_zone(true)
            .build();
        println!("{qr}\n");
    }

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    axum::serve(listener, router).await?;
    Ok(())
}
