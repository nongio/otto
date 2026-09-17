use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use otto_agentsd::acp::AcpBackend;
use otto_agentsd::agent::{Backend, EchoBackend};
use otto_agentsd::dialog::Islands;
use otto_agentsd::store::Store;
use otto_agentsd::{Server, cli, config};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about = "Otto's agent service, over the Agent Host Protocol")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server (the default when no command is given).
    Serve(ServeArgs),
    /// List the server's sessions.
    Sessions {
        /// WebSocket URL of the server.
        #[arg(long, env = "OTTO_AGENTS_URL", default_value = "ws://127.0.0.1:4800")]
        url: String,
    },
    /// Print a session's transcript.
    Show {
        /// Session id, or the start of one, as listed by `otto-agentsd sessions`.
        /// Defaults to the most recent session.
        session: Option<String>,
        /// Keep printing while the agent works, until it is done.
        #[arg(long, short)]
        follow: bool,
        /// WebSocket URL of the server.
        #[arg(long, env = "OTTO_AGENTS_URL", default_value = "ws://127.0.0.1:4800")]
        url: String,
    },
}

#[derive(Parser)]
struct ServeArgs {
    /// Address to accept WebSocket connections on.
    #[arg(long, env = "OTTO_AGENTS_LISTEN", default_value = "127.0.0.1:4800")]
    listen: String,
    /// Agent configuration file. Defaults to Otto's config files.
    #[arg(long, env = "OTTO_AGENTS_CONFIG")]
    config: Option<PathBuf>,
    /// Serve a built-in echo agent instead of real agents, for trying out clients.
    #[arg(long)]
    echo: bool,
    /// Where sessions are kept between runs. Defaults to
    /// `$XDG_STATE_HOME/otto-agentsd/sessions`; with `--echo`, sessions are only
    /// kept when this is given.
    #[arg(long, env = "OTTO_AGENTS_STATE_DIR")]
    state_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let command = Cli::parse()
        .command
        .unwrap_or_else(|| Command::Serve(ServeArgs::parse_from(["otto-agentsd"])));
    match command {
        Command::Serve(args) => serve(args).await,
        Command::Sessions { url } => cli::list_sessions(&url).await,
        Command::Show {
            session,
            follow,
            url,
        } => cli::show_session(&url, session.as_deref(), follow).await,
    }
}

async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let (backend, idle_timeout): (Arc<dyn Backend>, _) = if args.echo {
        (Arc::new(EchoBackend), Some(config::DEFAULT_IDLE_TIMEOUT))
    } else {
        let config = config::load(args.config.as_deref())?;
        for agent in &config.agents {
            tracing::info!(id = %agent.id, command = %agent.command, permissions = ?agent.permissions, "agent configured");
        }
        let idle_timeout = config.idle_timeout();
        (
            Arc::new(AcpBackend::new(config.agents, config.terminal)),
            idle_timeout,
        )
    };
    // Echo sessions are for trying clients out, and are not kept alongside
    // real ones unless asked.
    let state_dir = match args.state_dir {
        Some(dir) => Some(dir),
        None if args.echo => None,
        None => Store::default_dir(),
    };
    let store = state_dir.map(|dir| {
        tracing::info!(dir = %dir.display(), "keeping sessions");
        Store::new(dir)
    });
    let server =
        Server::bind_with(&args.listen, backend, Arc::new(Islands::default()), store).await?;
    tracing::info!("listening on ws://{}", server.local_addr()?);
    let host = server.host();
    host.set_idle_timeout(idle_timeout);

    tokio::select! {
        result = server.run() => result?,
        _ = tokio::signal::ctrl_c() => tracing::info!("shutting down"),
    }
    // What changed since the last periodic save.
    host.save();
    Ok(())
}
