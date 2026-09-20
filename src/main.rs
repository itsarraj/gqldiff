use std::fs;
use std::path::PathBuf;

use clap::Parser;
use gqldiff::{diff, schema};

#[derive(Parser)]
#[command(
    name = "gqldiff",
    about = "Diffs two GraphQL SDL schemas and reports breaking changes"
)]
struct Cli {
    old: PathBuf,
    new: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let old_sdl = fs::read_to_string(&cli.old)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", cli.old.display()))?;
    let new_sdl = fs::read_to_string(&cli.new)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", cli.new.display()))?;

    let old = schema::parse_schema(&old_sdl)?;
    let new = schema::parse_schema(&new_sdl)?;

    let changes = diff::diff_schemas(&old, &new);
    if changes.is_empty() {
        println!("no breaking changes");
        Ok(())
    } else {
        println!("{} breaking change(s):", changes.len());
        for c in &changes {
            println!("  {}", c.describe());
        }
        std::process::exit(1);
    }
}
