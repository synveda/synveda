//! Core worker executable. All lifecycle and work ownership lives in the
//! gateway library so tests and the product image exercise the same runtime.

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    synveda_gateway::worker::run().await
}
