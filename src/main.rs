use agno::config::Config;
use agno::site::{self, BuildOptions};
use agno::{check, scaffold, serve};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "agno",
    version,
    about = "A fast, simple static site generator",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new site
    New {
        path: PathBuf,
        /// Write into a non-empty directory
        #[arg(long)]
        force: bool,
    },
    /// Build the site into dist/
    Build {
        /// Include drafts
        #[arg(long)]
        drafts: bool,
        /// Print per-phase timings
        #[arg(long)]
        stats: bool,
        /// Delete the output directory before building
        #[arg(long)]
        clean: bool,
        /// Deprecated: builds are incremental by default
        #[arg(long, hide = true)]
        no_clean: bool,
        /// Site root (defaults to the current directory)
        #[arg(short, long)]
        root: Option<PathBuf>,
    },
    /// Build and serve with live reload
    Dev {
        /// Port to listen on
        #[arg(short, long, default_value_t = 1111)]
        port: u16,
        /// Include drafts
        #[arg(long)]
        drafts: bool,
        /// Site root (defaults to the current directory)
        #[arg(short, long)]
        root: Option<PathBuf>,
    },
    /// Remove build output and cache
    Clean {
        /// Site root (defaults to the current directory)
        #[arg(short, long)]
        root: Option<PathBuf>,
    },
    /// Check content for broken links, missing templates and other problems
    Check {
        /// Include drafts
        #[arg(long)]
        drafts: bool,
        /// Site root (defaults to the current directory)
        #[arg(short, long)]
        root: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::New { path, force } => {
            scaffold::create(&path, force)?;
            println!("Created {}", path.display());
            println!("\n  cd {}\n  agno dev\n", path.display());
        }
        Command::Build {
            drafts,
            stats,
            clean,
            no_clean: _,
            root,
        } => {
            let root = root.unwrap_or_else(|| PathBuf::from("."));
            let report = site::build(&root, &BuildOptions { drafts, clean })?;
            if stats {
                println!(
                    "  content {} ms · pages {} ms · lists+feeds {} ms",
                    report.ms_load, report.ms_pages, report.ms_rest
                );
            }
            println!(
                "Built {} pages in {} ms → {}",
                report.pages,
                report.ms_total,
                report.output.display()
            );
        }
        Command::Dev { port, drafts, root } => {
            let root = root.unwrap_or_else(|| PathBuf::from("."));
            serve::dev(root, port, drafts)?;
        }
        Command::Clean { root } => {
            let root = root.unwrap_or_else(|| PathBuf::from("."));
            let cfg = Config::load(&root)?;
            cfg.validate()?;
            let output = cfg.output();
            if output.exists() {
                agno::site::remove_all(&output)
                    .with_context(|| format!("removing {}", output.display()))?;
            }
            let cache = root.join(".agno-cache");
            if cache.exists() {
                agno::site::remove_all(&cache)
                    .with_context(|| format!("removing {}", cache.display()))?;
            }
            println!("Cleaned {}", output.display());
        }
        Command::Check { drafts, root } => {
            let root = root.unwrap_or_else(|| PathBuf::from("."));
            let report = check::check(&root, drafts)?;
            print!("{}", check::render(&report));
            if !report.ok() {
                eprintln!(
                    "\n{} error(s) across {} pages ({} internal links checked)",
                    report.errors.len(),
                    report.pages,
                    report.links
                );
                std::process::exit(1);
            }
            println!(
                "Checked {} pages and {} internal links: no problems",
                report.pages, report.links
            );
        }
    }

    Ok(())
}
