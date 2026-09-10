//! Core worker executable. All lifecycle and work ownership lives in the
//! gateway library so tests and the product image exercise the same runtime.

#[cfg(all(feature = "test-support", not(test), not(debug_assertions)))]
compile_error!("the worker release binary cannot include the test-support feature");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    synveda_gateway::worker::run().await
}
