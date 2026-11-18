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
            let report = site::build(&root, &build_options(drafts, clean, minify))?;
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

/// Map the parsed build flags to the options `site::build` consumes. Kept
/// separate from `main` so the flag-to-option wiring can be tested without
/// running a real build.
fn build_options(drafts: bool, clean: bool, minify: bool) -> BuildOptions {
    BuildOptions {
        drafts,
        clean,
        minify,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args.iter().copied()).expect("valid CLI input")
    }

    fn parse_err(args: &[&str]) -> clap::error::Error {
        match Cli::try_parse_from(args.iter().copied()) {
            Ok(_) => panic!("expected parse error for {args:?}"),
            Err(err) => err,
        }
    }

    #[test]
    fn build_flags_reach_build_options() {
        let cli = parse(&[
            "agno", "build", "--drafts", "--minify", "--clean", "--root", "site",
        ]);
        let Command::Build {
            drafts,
            stats,
            clean,
            minify,
            no_clean,
            root,
        } = cli.command
        else {
            panic!("expected build command");
        };
        assert!(drafts);
        assert!(clean);
        assert!(minify);
        assert!(!stats);
        assert!(!no_clean);
        assert_eq!(root, Some(PathBuf::from("site")));

        let opts = build_options(drafts, clean, minify);
        assert!(opts.drafts);
        assert!(opts.clean);
        assert!(opts.minify);
    }

    #[test]
    fn build_defaults_are_off() {
        let cli = parse(&["agno", "build"]);
        let Command::Build {
            drafts,
            stats,
            clean,
            minify,
            no_clean,
            root,
        } = cli.command
        else {
            panic!("expected build command");
        };
        assert!(!drafts && !clean && !minify && !stats && !no_clean);
        assert_eq!(root, None);

        let opts = build_options(drafts, clean, minify);
        assert!(!opts.drafts && !opts.clean && !opts.minify);
    }

    #[test]
    fn no_clean_is_accepted_and_ignored() {
        let cli = parse(&["agno", "build", "--no-clean"]);
        let Command::Build {
            clean, no_clean, ..
        } = cli.command
        else {
            panic!("expected build command");
        };
        assert!(no_clean);
        assert!(!clean);
    }

    #[test]
    fn build_root_accepts_long_and_short_forms() {
        let cli = parse(&["agno", "build", "--root", "site"]);
        let Command::Build { root, .. } = cli.command else {
            panic!("expected build command");
        };
        assert_eq!(root, Some(PathBuf::from("site")));

        let cli = parse(&["agno", "build", "-r", "../docs"]);
        let Command::Build { root, .. } = cli.command else {
            panic!("expected build command");
        };
        assert_eq!(root, Some(PathBuf::from("../docs")));
    }

    #[test]
    fn dev_parses_port_drafts_and_root() {
        let cli = parse(&[
            "agno", "dev", "--port", "2222", "--drafts", "--root", "docs",
        ]);
        let Command::Dev { port, drafts, root } = cli.command else {
            panic!("expected dev command");
        };
        assert_eq!(port, 2222);
        assert!(drafts);
        assert_eq!(root, Some(PathBuf::from("docs")));
    }

    #[test]
    fn dev_defaults_and_short_port() {
        let cli = parse(&["agno", "dev"]);
        let Command::Dev { port, drafts, root } = cli.command else {
            panic!("expected dev command");
        };
        assert_eq!(port, 1111);
        assert!(!drafts);
        assert_eq!(root, None);

        let cli = parse(&["agno", "dev", "-p", "8080"]);
        let Command::Dev { port, .. } = cli.command else {
            panic!("expected dev command");
        };
        assert_eq!(port, 8080);
    }

    #[test]
    fn other_subcommands_parse() {
        let cli = parse(&["agno", "new", "site", "--force"]);
        let Command::New { path, force } = cli.command else {
            panic!("expected new command");
        };
        assert_eq!(path, PathBuf::from("site"));
        assert!(force);

        let cli = parse(&["agno", "clean", "-r", "site"]);
        let Command::Clean { root } = cli.command else {
            panic!("expected clean command");
        };
        assert_eq!(root, Some(PathBuf::from("site")));

        let cli = parse(&["agno", "check", "--drafts", "--root", "site"]);
        let Command::Check { drafts, root } = cli.command else {
            panic!("expected check command");
        };
        assert!(drafts);
        assert_eq!(root, Some(PathBuf::from("site")));
    }

    #[test]
    fn help_and_no_args_are_reported() {
        let err = parse_err(&["agno", "--help"]);
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
        assert!(err.to_string().contains("Usage:"));

        let err = parse_err(&["agno", "build", "--help"]);
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);

        let err = parse_err(&["agno"]);
        assert_eq!(
            err.kind(),
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
    }

    #[test]
    fn invalid_args_produce_an_error() {
        let err = parse_err(&["agno", "build", "--bogus"]);
        assert_eq!(err.kind(), ErrorKind::UnknownArgument);

        let err = parse_err(&["agno", "dev", "--port", "not-a-number"]);
        assert_eq!(err.kind(), ErrorKind::ValueValidation);

        let err = parse_err(&["agno", "build", "--root"]);
        assert_eq!(err.kind(), ErrorKind::InvalidValue);

        let err = parse_err(&["agno", "bogus-subcommand"]);
        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
    }
}
