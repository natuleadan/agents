//! Boxer worker daemon entrypoint.

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    tracing::info!("Agents Boxer sandbox runner starting up...");
}
