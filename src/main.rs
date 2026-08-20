use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Parser, Debug)]
#[command(name = "hyprosk")]
#[command(author = "CheeseGuru & Antigravity")]
#[command(version = "0.1.0")]
#[command(about = "Fast, lightweight native Wayland On-Screen Keyboard for Hyprland with auto input-field detection", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Path to custom config file
    #[arg(short, long)]
    config: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the HyprOsk on-screen keyboard daemon (default)
    Daemon {
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Show the on-screen keyboard
    Show,
    /// Hide the on-screen keyboard
    Hide,
    /// Toggle on-screen keyboard visibility
    Toggle,
    /// Switch active keyboard layer (lower, upper, symbols)
    Layer {
        name: String,
    },
    /// Print folio/keyboard detection diagnostics for debugging
    Status,
    /// Stop the running HyprOsk daemon
    Quit,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();

    let cli = Cli::parse();

    match cli.command {
        None => {
            hyprosk::run_daemon(cli.config.as_deref())?;
        }
        Some(Commands::Daemon { config }) => {
            let conf_path = config.as_deref().or(cli.config.as_deref());
            hyprosk::run_daemon(conf_path)?;
        }
        Some(Commands::Show) => {
            let resp = hyprosk::ipc::IpcServer::send_command("show")?;
            println!("{}", resp.trim());
        }
        Some(Commands::Hide) => {
            let resp = hyprosk::ipc::IpcServer::send_command("hide")?;
            println!("{}", resp.trim());
        }
        Some(Commands::Toggle) => {
            let resp = hyprosk::ipc::IpcServer::send_command("toggle")?;
            println!("{}", resp.trim());
        }
        Some(Commands::Layer { name }) => {
            let resp = hyprosk::ipc::IpcServer::send_command(&format!("layer {}", name))?;
            println!("{}", resp.trim());
        }
        Some(Commands::Status) => {
            let attached = hyprosk::folio::physical_keyboard_attached();
            let tablet = hyprosk::folio::tablet_mode_active();
            let evdev = hyprosk::folio::evdev_keyboard_present();
            let proc_kbd = hyprosk::folio::proc_keyboard_present();
            println!("tablet_mode (SW_TABLET_MODE): {tablet}");
            println!("evdev letter-key keyboard:   {evdev}");
            println!("/proc kbd-letter keyboard:   {proc_kbd}");
            println!("folio attached (verdict):    {attached}");
            let config = hyprosk::config::Config::load_or_create(cli.config.as_deref());
            println!("folio_mode (config):         {}", config.behavior.folio_mode);
            println!("auto-show will be:           {}", if !config.behavior.folio_mode || !attached { "ENABLED" } else { "SUPPRESSED" });
        }
        Some(Commands::Quit) => {
            let resp = hyprosk::ipc::IpcServer::send_command("quit")?;
            println!("{}", resp.trim());
        }
    }

    Ok(())
}
