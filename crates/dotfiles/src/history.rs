//! `dotfiles history seed` — ports update-history-commands.sh: inserts every
//! curated command from commands.yaml into the atuin SQLite history db,
//! deduplicated by command text. Pure Rust parsing (no yq/awk/sed).

use crate::ctx::Ctx;
use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser, Debug)]
pub struct HistoryArgs {
    #[command(subcommand)]
    pub command: HistoryCommand,
}

#[derive(Parser, Debug)]
pub enum HistoryCommand {
    /// Seed the atuin history database from commands.yaml
    Seed,
}

pub fn run(ctx: &Ctx, args: HistoryArgs) -> Result<()> {
    match args.command {
        HistoryCommand::Seed => seed(ctx),
    }
}

/// One INSERT-equivalent statement, pre-escaped (`'` doubled).
fn insert_statement(
    id: &str,
    timestamp: i128,
    command: &str,
    cwd: &str,
    session: &str,
    hostname: &str,
) -> String {
    let q = |s: &str| format!("'{}'", s.replace('\'', "''"));
    format!(
        "INSERT INTO history (id, timestamp, duration, exit, command, cwd, session, hostname, deleted_at)\n\
         SELECT {id}, {ts}, -1, -1, {command}, {cwd}, {session}, {hostname}, NULL\n\
         WHERE NOT EXISTS (SELECT id FROM history WHERE command = {command});",
        id = q(id),
        ts = timestamp,
        command = q(command),
        cwd = q(cwd),
        session = q(session),
        hostname = q(hostname),
    )
}

fn seed(ctx: &Ctx) -> Result<()> {
    let db = ctx.env.home.join(".local/share/atuin/history.db");
    if !db.is_file() {
        println!(
            "atuin history db {} does not exist yet — start atuin once first. Skipping.",
            db.display()
        );
        return Ok(());
    }
    let commands = ctx.commands()?;
    let session =
        std::env::var("ATUIN_SESSION").context("ATUIN_SESSION not set (script parity: set -u)")?;
    let hostname_out = ctx.env.output("hostname", &[])?.stdout.trim().to_string();
    let user = std::env::var("USER").unwrap_or_else(|_| "unknown".into());
    let hostname = format!("{}:{}", hostname_out, user);
    let cwd = ctx.env.home.display().to_string();

    let mut sql = String::from("BEGIN TRANSACTION;\n");
    let mut total = 0usize;
    for (_, section) in commands.sections() {
        for entry in &section.commands {
            // Print-script normalization: multiline → single line, squeezed spaces
            let command = entry
                .command
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if command.is_empty() {
                continue;
            }
            let id = ulid::Ulid::new().to_string();
            let ts = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as i128;
            sql.push_str(&insert_statement(
                &id, ts, &command, &cwd, &session, &hostname,
            ));
            sql.push('\n');
            total += 1;
        }
    }
    sql.push_str("COMMIT;\n");

    // The bash script ended by deleting its own invocation from history; a
    // native binary leaves no such entry, so there is nothing to clean up.
    let out = ctx
        .env
        .output_stdin("sqlite3", &[db.to_str().unwrap()], &sql)?;
    if !out.ok() {
        anyhow::bail!("sqlite3 failed: {}", out.stderr.trim());
    }
    println!("seeded {} commands into {}", total, db.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_statement_is_deduped_and_escaped() {
        let sql = insert_statement("01ABC", 42, "echo 'hi'", "/home/u", "sess", "host:user");
        assert!(
            sql.contains("SELECT '01ABC', 42, -1, -1, 'echo ''hi'''"),
            "{}",
            sql
        );
        assert!(
            sql.contains("WHERE NOT EXISTS (SELECT id FROM history WHERE command = 'echo ''hi''')")
        );
    }

    #[test]
    fn seed_skips_when_db_absent() {
        let t = dotfiles_testkit::TestEnv::new();
        t.write("dotfiles/apps.yaml", "install: {}\n");
        t.write("dotfiles/commands.yaml", "git:\n  description: git\n  commands:\n    - { description: d, command: git status }\n");
        let ctx = Ctx::sandbox(t.root(), false).unwrap();
        seed(&ctx).unwrap(); // no sqlite3 call, no error
        assert!(t.calls_of("sqlite3").is_empty());
    }

    #[test]
    fn seed_writes_normalized_commands_via_sqlite3() {
        let t = dotfiles_testkit::TestEnv::new();
        t.write("dotfiles/apps.yaml", "install: {}\n");
        t.write("dotfiles/commands.yaml", "git:\n  description: git\n  commands:\n    - description: d\n      command: |\n        git   log\n          --oneline\n");
        t.write("home/.local/share/atuin/history.db", "");
        // capture stdin
        t.stub("sqlite3", "cat > \"$SQLLOG\"; exit 0");
        let ctx = Ctx::sandbox(t.root(), false).unwrap();
        std::env::set_var("ATUIN_SESSION", "test-session");
        std::env::set_var("SQLLOG", t.root().join("sql.log"));
        t.stub_ok("hostname", "testhost");
        seed(&ctx).unwrap();
        std::env::remove_var("ATUIN_SESSION");
        let sql = std::fs::read_to_string(t.root().join("sql.log")).unwrap();
        assert!(sql.contains("'git log --oneline'"), "{}", sql);
        assert!(
            sql.contains("'testhost:unknown'")
                || sql.contains(&format!(
                    "'testhost:{}'",
                    std::env::var("USER").unwrap_or_default()
                )),
            "{}",
            sql
        );
        assert!(sql.contains("BEGIN TRANSACTION;"));
        assert!(sql.contains("COMMIT;"));
    }
}
