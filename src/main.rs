//! `flux` CLI binary.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "flux",
    version,
    about = "Code-first audiovisual rendering engine"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Increase log verbosity (-v info, -vv debug, -vvv trace).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Render a project to a video file.
    Render {
        /// Path to the project `.ron` file.
        project: PathBuf,

        /// Audio file (WAV) to drive reactive parameters.
        #[arg(long)]
        audio: PathBuf,

        /// Output video file. The extension determines the container.
        #[arg(long, default_value = "out.mp4")]
        out: PathBuf,

        /// Output framerate. Defaults to the project's framerate.
        #[arg(long)]
        fps: Option<u32>,

        /// Render only the first N seconds (useful for previews).
        #[arg(long)]
        duration: Option<f32>,
    },

    /// Validate a project file without rendering.
    Check { project: PathBuf },

    /// List built-in node types.
    Nodes,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match cli.command {
        Command::Render {
            project,
            audio,
            out,
            fps,
            duration,
        } => {
            let proj = flux::Project::load(&project)
                .with_context(|| format!("loading project {}", project.display()))?;
            let mut engine = flux::Engine::new(&proj)?;
            engine.render_to_file(&audio, &out, fps, duration)?;
            tracing::info!("Wrote {}", out.display());
        }
        Command::Check { project } => {
            let proj = flux::Project::load(&project)?;
            // Full engine init catches param-shape, missing-input, cycle, and
            // shader-compile errors. It needs a GPU device, same as rendering.
            let _engine = flux::Engine::new(&proj)?;
            println!("OK — {} nodes, output `{}`", proj.nodes.len(), proj.output);
        }
        Command::Nodes => {
            for name in flux::nodes::registered_names() {
                println!("{name}");
            }
        }
    }

    Ok(())
}

fn init_tracing(verbosity: u8) {
    let level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("flux={level}")));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}
