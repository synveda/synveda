//! Experimental Apalis worker process.

#![forbid(unsafe_code)]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let command = arguments
        .next()
        .ok_or("synveda-apalis-worker requires worker or migrate")?;
    if arguments.next().is_some() {
        return Err("synveda-apalis-worker accepts exactly one command".into());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    match command.to_str() {
        Some("worker") => runtime.block_on(synveda_apalis::run_worker()),
        Some("migrate") => runtime.block_on(synveda_apalis::run_migrate()),
        _ => Err("synveda-apalis-worker requires worker or migrate".into()),
    }
}
