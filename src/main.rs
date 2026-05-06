//! `flux` CLI binary.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use flux::project::ToneMap;
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

        /// Override the project's tone-map setting.
        #[arg(long, value_enum)]
        tone_map: Option<ToneMapArg>,
    },

    /// Validate a project file without rendering.
    Check { project: PathBuf },

    /// List built-in node types.
    Nodes,

    /// Open a live preview window.
    Preview {
        /// Path to the project `.ron` file.
        project: PathBuf,
        /// Audio file (WAV) to drive reactive parameters. Audio is not
        /// played back yet — only its features feed the graph.
        #[arg(long)]
        audio: PathBuf,
    },
}

/// CLI mirror of `project::ToneMap`. Kept separate so clap doesn't need
/// derive support on the schema type.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum ToneMapArg {
    Aces,
    Reinhard,
    None,
}

impl From<ToneMapArg> for ToneMap {
    fn from(value: ToneMapArg) -> Self {
        match value {
            ToneMapArg::Aces => ToneMap::Aces,
            ToneMapArg::Reinhard => ToneMap::Reinhard,
            ToneMapArg::None => ToneMap::None,
        }
    }
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
            tone_map,
        } => {
            let proj = flux::Project::load(&project)
                .with_context(|| format!("loading project {}", project.display()))?;
            let mut engine = flux::Engine::new(&proj)?;
            if let Some(tm) = tone_map {
                engine.tone_map = tm.into();
            }
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
        Command::Preview { project, audio } => {
            let proj = flux::Project::load(&project)
                .with_context(|| format!("loading project {}", project.display()))?;
            flux::preview::run(&proj, &project, &audio)?;
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
