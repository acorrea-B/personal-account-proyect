//! Local development server.
//!
//! It serves the same router as the function, over TCP, so the quickstart
//! works without Scaleway. It is not deployed: the package patterns of
//! `serverless.yml` exclude it from the artifact.

use std::net::SocketAddr;
use std::process::ExitCode;

use tokio::net::TcpListener;

use perfil_api::application;

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(err) = run().await {
        eprintln!("bootstrap error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let application = application().await?;
    let address = application.config.bind_address();

    let listener = TcpListener::bind(&address).await?;
    tracing::info!(%address, "the api is listening");

    axum::serve(
        listener,
        application
            .router
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
