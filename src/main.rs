use agno::content::Drafts;
use agno::scaffold::Overwrite;
use agno::site::{self, BuildOptions};
use agno::{check, scaffold, serve};
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

// The musl build gets mimalloc because musl's malloc takes a global lock, and
// that build is where it shows: 1000 plain pages went from 1.07s to 47ms on
// twelve threads. It does *not* fix everything - see the note about syntax
// highlighting in the README - and it costs peak RSS, so the gnu, macOS and
// Windows builds keep their system allocator.
#[cfg(target_env = "musl")]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

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
        /// Minify the HTML output
        #[arg(long)]
        minify: bool,
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
            scaffold::create(&path, Overwrite::from(force))?;
            println!("Created {}", path.display());
            println!("\n  cd {}\n  agno dev\n", path.display());
        }
        Command::Build {
            drafts,
            stats,
            clean,
            minify,
            no_clean: _,
            root,
        } => {
            let root = root.unwrap_or_else(|| PathBuf::from("."));
            let report = site::build(
                &root,
                &BuildOptions {
                    drafts,
                    clean,
                    minify,
                },
            )?;
            if stats {
                println!(
                    "  content {} ms · pages {} ms · lists+feeds {} ms",
                    report.load.as_millis(),
                    report.render.as_millis(),
                    report.rest.as_millis()
                );
            }
            println!(
                "Built {} pages in {} ms → {}",
                report.pages,
                report.total.as_millis(),
                report.output.display()
            );
        }
        Command::Dev { port, drafts, root } => {
            let root = root.unwrap_or_else(|| PathBuf::from("."));
            serve::dev(&root, port, Drafts::from(drafts))?;
        }
        Command::Clean { root } => {
            let root = root.unwrap_or_else(|| PathBuf::from("."));
            let output = site::clean(&root)?;
            println!("Cleaned {}", output.display());
        }
        Command::Check { drafts, root } => {
            let root = root.unwrap_or_else(|| PathBuf::from("."));
            let report = check::check(&root, Drafts::from(drafts))?;
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
