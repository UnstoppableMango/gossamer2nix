use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Rewrite every dependency in a Gossamer project's transitive graph to
    /// a `path`-kind entry (docs/deps/DESIGN.md §3.3).
    Generate {
        #[arg(long)]
        manifest_dir: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Command::Generate { manifest_dir, out } => {
            if let Err(err) = gossamer2nix::generate::generate(&manifest_dir, &out) {
                eprintln!("error: {err}");
                return ExitCode::FAILURE;
            }
        }
    }

    ExitCode::SUCCESS
}
