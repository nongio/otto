use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use otto_agents::acp::AcpBackend;
use otto_agents::agent::{Backend, EchoBackend};
use otto_agents::dialog::Islands;
use otto_agents::server::Listen;
use otto_agents::store::Store;
use otto_agents::{Server, cli, client, config};
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
        /// The server: `unix:///path` (the default, in the runtime directory) or `ws://`.
        #[arg(long, env = "OTTO_AGENTS_URL", default_value_t = client::default_url())]
        url: String,
    },
    /// Forget stored sessions. The agent keeps its own history; this removes
    /// what otto-agents stored about them.
    Forget {
        /// Session id, or the start of one, as listed by `otto-agents sessions`.
        /// Defaults to the most recent session.
        session: Option<String>,
        /// Forget every session.
        #[arg(long, conflicts_with = "session")]
        all: bool,
        /// The server: `unix:///path` (the default, in the runtime directory) or `ws://`.
        #[arg(long, env = "OTTO_AGENTS_URL", default_value_t = client::default_url())]
        url: String,
    },
    /// Start a session with an agent and enter it in this terminal, in the
    /// agent's own interface. The session is the desktop's: it shows up in
    /// Ask, and what is said here is there the next time it is opened.
    New {
        /// The agent, by its id in `agents.toml` or the name it is shown
        /// under. Defaults to the service's first agent.
        agent: Option<String>,
        /// The folder the session works in. Defaults to this one.
        #[arg(long, short = 'C')]
        cwd: Option<PathBuf>,
        /// The server: `unix:///path` (the default, in the runtime directory) or `ws://`.
        #[arg(long, env = "OTTO_AGENTS_URL", default_value_t = client::default_url())]
        url: String,
    },
    /// Take up a session that is already there in this terminal, in the
    /// agent's own interface. The terminal writes its history from then on.
    Enter {
        /// Session id, or the start of one, as listed by `otto-agents sessions`.
        /// Defaults to the most recent session.
        session: Option<String>,
        /// The server: `unix:///path` (the default, in the runtime directory) or `ws://`.
        #[arg(long, env = "OTTO_AGENTS_URL", default_value_t = client::default_url())]
        url: String,
    },
    /// Check the things that stop Ask working, and say which one is wrong.
    Doctor {
        /// Agent configuration file. Defaults to Otto's config files.
        #[arg(long, env = "OTTO_AGENTS_CONFIG")]
        config: Option<PathBuf>,
        /// The server: `unix:///path` (the default, in the runtime directory) or `ws://`.
        #[arg(long, env = "OTTO_AGENTS_URL", default_value_t = client::default_url())]
        url: String,
    },
    /// Print a session's transcript.
    Show {
        /// Session id, or the start of one, as listed by `otto-agents sessions`.
        /// Defaults to the most recent session.
        session: Option<String>,
        /// Keep printing while the agent works, until it is done.
        #[arg(long, short)]
        follow: bool,
        /// The server: `unix:///path` (the default, in the runtime directory) or `ws://`.
        #[arg(long, env = "OTTO_AGENTS_URL", default_value_t = client::default_url())]
        url: String,
    },
    /// Put the desktop's skills and its agent where each harness looks for them.
    #[command(alias = "skills")]
    Plugins {
        #[command(subcommand)]
        command: PluginsCommand,
    },
}

#[derive(Subcommand)]
enum PluginsCommand {
    /// Link every skill Otto ships into the skills directory, as
    /// `<dir>/<name>` pointing at the skill's own directory, and write the
    /// plugin's agent in each harness's own dialect where that harness is set
    /// up. Anything already there that is not ours is left alone. Safe to run
    /// again: an unchanged source writes nothing.
    Install {
        /// The skills directory. Defaults to `~/.agents/skills`, which agents
        /// with skill discovery read.
        #[arg(long)]
        dir: Option<PathBuf>,
        /// The home directory the harnesses' files are placed under. Defaults
        /// to `$HOME`; for trying the installer out somewhere harmless.
        #[arg(long)]
        home: Option<PathBuf>,
        /// Only this harness's agent file (opencode, hermes, codex or pi), and
        /// no skills.
        #[arg(long)]
        only: Option<String>,
    },
    /// Show the plugins and skills found, whether each skill is linked, and
    /// where each harness's copy of the agent stands.
    Status {
        /// The skills directory to check. Defaults to `~/.agents/skills`.
        #[arg(long)]
        dir: Option<PathBuf>,
        /// The home directory to check the harnesses' files under. Defaults
        /// to `$HOME`.
        #[arg(long)]
        home: Option<PathBuf>,
        /// Only this harness (opencode, hermes, codex or pi).
        #[arg(long)]
        only: Option<String>,
    },
}

#[derive(Parser)]
struct ServeArgs {
    /// Where to accept connections: a socket path (the default, in the runtime
    /// directory) or `host:port`. TCP is unauthenticated: for development.
    #[arg(long, env = "OTTO_AGENTS_LISTEN")]
    listen: Option<Listen>,
    /// Agent configuration file. Defaults to Otto's config files.
    #[arg(long, env = "OTTO_AGENTS_CONFIG")]
    config: Option<PathBuf>,
    /// Serve a built-in echo agent instead of real agents, for trying out clients.
    #[arg(long)]
    echo: bool,
    /// Where sessions are kept between runs. Defaults to
    /// `$XDG_STATE_HOME/otto-agents/sessions`; with `--echo`, sessions are only
    /// kept when this is given.
    #[arg(long, env = "OTTO_AGENTS_STATE_DIR")]
    state_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let command = Cli::parse()
        .command
        .unwrap_or_else(|| Command::Serve(ServeArgs::parse_from(["otto-agents"])));
    // Every subcommand, not just `serve`: a `plugins install` that cannot read
    // something says so through `tracing` like everything else, and `RUST_LOG`
    // is how anyone would think to ask for it.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    match command {
        Command::Serve(args) => serve(args).await,
        Command::Sessions { url } => cli::list_sessions(&url).await,
        Command::Forget { session, all, url } => {
            cli::forget_sessions(&url, session.as_deref(), all).await
        }
        Command::New { agent, cwd, url } => {
            cli::new_session(&url, agent.as_deref(), cwd.as_deref()).await
        }
        Command::Enter { session, url } => cli::enter(&url, session.as_deref()).await,
        Command::Doctor { config, url } => cli::doctor(&url, config.as_deref()).await,
        Command::Show {
            session,
            follow,
            url,
        } => cli::show_session(&url, session.as_deref(), follow).await,
        Command::Plugins {
            command: PluginsCommand::Install { dir, home, only },
        } => cli::install_plugins(dir.as_deref(), home.as_deref(), only.as_deref()),
        Command::Plugins {
            command: PluginsCommand::Status { dir, home, only },
        } => cli::plugins_status(dir.as_deref(), home.as_deref(), only.as_deref()),
    }
}

async fn serve(args: ServeArgs) -> anyhow::Result<()> {
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
    let listen = match args.listen {
        Some(listen) => listen,
        None => Listen::default_endpoint()?,
    };
    let server = Server::listen(&listen, backend, Arc::new(Islands::default()), store).await?;
    tracing::info!("listening on {}", server.endpoint()?.url());
    let host = server.host();
    host.set_idle_timeout(idle_timeout);

    // SIGTERM is what `systemctl --user stop` sends, so it is the path that
    // actually runs; without it the last second of state goes unsaved.
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = server.run() => result?,
        _ = tokio::signal::ctrl_c() => tracing::info!("shutting down"),
        _ = terminate.recv() => tracing::info!("asked to stop"),
    }
    // What changed since the last periodic save.
    host.save();
    Ok(())
}
