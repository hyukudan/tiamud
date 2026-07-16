use std::{path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "tiamud-admin", version, about = "Tiamud operator tools")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate a SQLite database and its WorldPack without modifying them.
    Check {
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        world_pack: PathBuf,
    },
    /// Migrate an existing database after creating a verified safety backup.
    Migrate {
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        backup: PathBuf,
    },
    /// Recover one registered account as Implementor when no administrator/implementor profile exists.
    RecoverAdmin {
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        account: String,
    },
    /// Create, verify, or restore SQLite backups.
    Backup {
        #[command(subcommand)]
        command: BackupCommand,
    },
}

#[derive(Debug, Subcommand)]
enum BackupCommand {
    /// Create and verify a consistent snapshot without overwriting files.
    Create {
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Verify the integrity and schema of an existing snapshot.
    Verify {
        #[arg(long)]
        backup: PathBuf,
    },
    /// Restore a verified snapshot to a new database path.
    Restore {
        #[arg(long)]
        backup: PathBuf,
        #[arg(long)]
        database: PathBuf,
    },
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

fn run(cli: Cli) -> Result<(), tiamud_admin::AdminError> {
    match cli.command {
        Command::Check {
            database,
            world_pack,
        } => {
            let report = tiamud_admin::check(database, world_pack)?;
            println!(
                "check passed: database schema v{}, WorldPack {:?} ({} rooms, {} items)",
                report.database_schema_version,
                report.world_pack_id,
                report.room_count,
                report.item_count
            );
        }
        Command::Migrate { database, backup } => {
            let report = tiamud_admin::migrate(database, backup)?;
            println!(
                "database {:?} migrated from schema v{} to v{}; safety backup: {:?}",
                report.database, report.migration.from, report.migration.to, report.backup
            );
        }
        Command::RecoverAdmin { database, account } => {
            let report = tiamud_admin::recover_admin(database, &account)?;
            println!(
                "recovered Implementor {:?} (account id {}) in {:?}; staff and compatibility audits recorded",
                report.account_name, report.account_id, report.database
            );
        }
        Command::Backup { command } => match command {
            BackupCommand::Create { database, output } => {
                let report = tiamud_admin::backup(database, output)?;
                println!(
                    "backup created at {:?}: database schema v{}; integrity and foreign keys verified",
                    report.output, report.database_schema_version
                );
            }
            BackupCommand::Verify { backup } => {
                let report = tiamud_admin::verify_backup(backup)?;
                println!(
                    "backup {:?} verified: database schema v{}; integrity and foreign keys passed",
                    report.output, report.database_schema_version
                );
            }
            BackupCommand::Restore { backup, database } => {
                let report = tiamud_admin::restore(backup, database)?;
                println!(
                    "backup restored to {:?}: database schema v{}",
                    report.database, report.database_schema_version
                );
            }
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_check_command() {
        let cli = Cli::try_parse_from([
            "tiamud-admin",
            "check",
            "--database",
            "state.sqlite3",
            "--world-pack",
            "world.json",
        ])
        .unwrap();
        assert!(matches!(cli.command, Command::Check { .. }));
    }

    #[test]
    fn parses_backup_command() {
        let cli = Cli::try_parse_from([
            "tiamud-admin",
            "backup",
            "create",
            "--database",
            "state.sqlite3",
            "--output",
            "backup.sqlite3",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Backup {
                command: BackupCommand::Create { .. }
            }
        ));
    }

    #[test]
    fn parses_migrate_verify_and_restore_commands() {
        assert!(matches!(
            Cli::try_parse_from([
                "tiamud-admin",
                "migrate",
                "--database",
                "state.sqlite3",
                "--backup",
                "before.sqlite3",
            ])
            .unwrap()
            .command,
            Command::Migrate { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "tiamud-admin",
                "backup",
                "verify",
                "--backup",
                "backup.sqlite3",
            ])
            .unwrap()
            .command,
            Command::Backup {
                command: BackupCommand::Verify { .. }
            }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "tiamud-admin",
                "backup",
                "restore",
                "--backup",
                "backup.sqlite3",
                "--database",
                "restored.sqlite3",
            ])
            .unwrap()
            .command,
            Command::Backup {
                command: BackupCommand::Restore { .. }
            }
        ));
    }

    #[test]
    fn parses_explicit_admin_recovery() {
        assert!(matches!(
            Cli::try_parse_from([
                "tiamud-admin",
                "recover-admin",
                "--database",
                "state.sqlite3",
                "--account",
                "Keeper",
            ])
            .unwrap()
            .command,
            Command::RecoverAdmin { .. }
        ));
    }
}
