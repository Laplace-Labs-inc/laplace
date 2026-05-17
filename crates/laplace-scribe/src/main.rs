mod config;
mod context;
mod engine;
mod error;
mod index;
mod md_parser;
mod rust_parser;

#[cfg(feature = "db-ingest")]
mod db;

use std::path::PathBuf;

use clap::Parser;
use config::ScribeConfig;
use engine::ScribeEngine;

/// laplace-scribe — AST-based knowledge extraction engine.
///
/// Parses Rust source and Markdown documentation under a Laplace project root,
/// splits extracted knowledge into ≤ 15 KB L1-cache chunks, and writes them
/// into the LKS workspace hierarchy (`<root>/LKS/<workspace>/`).
#[derive(Parser, Debug)]
#[command(name = "laplace-scribe", version, about)]
struct Args {
    /// Project root directory (defaults to current working directory).
    #[arg(short, long, default_value = ".")]
    root: PathBuf,

    /// Override the LKS output directory.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Override the per-chunk byte limit (default: 15360 = 15 KB).
    #[arg(short, long)]
    chunk_limit: Option<usize>,

    /// PostgreSQL connection URL for direct DB ingestion.
    /// Format: postgres://user:password@host/dbname
    /// When provided: writes to DB only, no LKS/*.md files generated.
    /// When omitted: writes LKS/*.md files (default behavior).
    #[cfg(feature = "db-ingest")]
    #[arg(long, env = "LKS_DB_URL")]
    db_url: Option<String>,
}

fn main() {
    let args = Args::parse();

    let root = match args.root.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: cannot resolve project root: {}", e);
            std::process::exit(1);
        }
    };

    let mut cfg = ScribeConfig::new(root);

    if let Some(out) = args.output {
        cfg.output_dir = out;
    }
    if let Some(limit) = args.chunk_limit {
        cfg.chunk_limit = limit;
    }

    #[cfg(feature = "db-ingest")]
    if let Some(url) = args.db_url {
        cfg.db_url = Some(url);
    }

    println!(
        "laplace-scribe: extracting knowledge → {}",
        cfg.output_dir.display()
    );

    let mut engine = match ScribeEngine::new(&cfg) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    };

    match engine.run() {
        Ok(stats) => {
            println!("Done.");
            println!(
                "  Markdown : {} files  → {} chunks",
                stats.md_files, stats.md_chunks
            );
            println!(
                "  Rust     : {} files  → {} chunks",
                stats.rs_files, stats.rs_chunks
            );
        }
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}
