//! `dataguard-migrate`: brings the database named by `DATABASE_URL` to the
//! last migration. Takes no argument, so no address lands in a shell history
//! or a process listing.

use std::error::Error;
use std::process::ExitCode;

use dataguard_store::{DATABASE_URL_VAR, MIGRATIONS, StoreConfig, connect, migrate, validate};

fn main() -> ExitCode {
    if std::env::args_os().len() > 1 {
        eprintln!(
            "usage: dataguard-migrate (no argument; the connection comes from {DATABASE_URL_VAR})"
        );
        return ExitCode::from(2);
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("dataguard-migrate: {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    validate(MIGRATIONS)?;
    let config = StoreConfig::from_env()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let report = runtime.block_on(async {
        let mut store = connect(&config).await?;
        migrate(&mut store, MIGRATIONS)
            .await
            .map_err(Box::<dyn Error>::from)
    })?;
    println!(
        "applied {} migration(s); schema at number {}",
        report.applied, report.current
    );
    Ok(())
}
