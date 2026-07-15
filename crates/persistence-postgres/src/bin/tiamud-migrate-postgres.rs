use std::{env, path::PathBuf, process::ExitCode};

use clap::Parser;
use thiserror::Error;

#[derive(Debug, Parser)]
#[command(
    name = "tiamud-migrate-postgres",
    version,
    about = "Copy an offline current Tiamud SQLite database into an empty PostgreSQL schema"
)]
struct Cli {
    /// Read-only SQLite source at the current schema version.
    #[arg(long)]
    sqlite: PathBuf,

    /// PostgreSQL connection URL. Prefer TIAMUD_POSTGRES_URL to avoid process-list exposure.
    #[arg(long)]
    postgres_url: Option<String>,

    /// Required acknowledgement that the target must be empty and no dual-write will occur.
    #[arg(long)]
    confirm_offline: bool,
}

#[derive(Debug, Error)]
enum CliError {
    #[error("--confirm-offline is required; stop Tiamud before taking the SQLite snapshot")]
    ConfirmationRequired,
    #[error("TIAMUD_POSTGRES_URL is not set and --postgres-url was not supplied")]
    MissingPostgresUrl,
    #[error(transparent)]
    Connect(#[from] tiamud_persistence_postgres::ConnectError),
    #[error(transparent)]
    Transfer(#[from] tiamud_persistence_postgres::TransferError),
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), CliError> {
    if !cli.confirm_offline {
        return Err(CliError::ConfirmationRequired);
    }
    let postgres_url = match cli.postgres_url {
        Some(url) => url,
        None => env::var("TIAMUD_POSTGRES_URL").map_err(|_| CliError::MissingPostgresUrl)?,
    };

    let mut postgres = tiamud_persistence_postgres::connect(&postgres_url)?;
    let report = tiamud_persistence_postgres::copy_sqlite_to_postgres(cli.sqlite, &mut postgres)?;

    println!(
        "offline transfer committed: SQLite v{} -> PostgreSQL v{}; {} rows across {} tables",
        report.sqlite_schema_version,
        report.postgres_schema_version,
        report.total_rows,
        report.tables.len()
    );
    for table in report.tables {
        println!("  {}: {}", table.table, table.rows);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_explicit_offline_contract() {
        let cli = Cli::try_parse_from([
            "tiamud-migrate-postgres",
            "--sqlite",
            "state.sqlite3",
            "--postgres-url",
            "postgresql://localhost/tiamud",
            "--confirm-offline",
        ])
        .unwrap();

        assert_eq!(cli.sqlite, PathBuf::from("state.sqlite3"));
        assert!(cli.confirm_offline);
        assert!(cli.postgres_url.is_some());
    }

    #[test]
    fn refuses_to_run_without_the_offline_acknowledgement() {
        let cli = Cli::try_parse_from([
            "tiamud-migrate-postgres",
            "--sqlite",
            "state.sqlite3",
            "--postgres-url",
            "postgresql://localhost/tiamud",
        ])
        .unwrap();

        assert!(matches!(run(cli), Err(CliError::ConfirmationRequired)));
    }
}
