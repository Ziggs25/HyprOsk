use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(name = "hyprosk")]
#[command(author = "Ziggs25")]
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
    Show {
        /// Force tiled/exclusive mode
        #[arg(long, short = 't', conflicts_with = "floating")]
        tiled: bool,
        /// Force floating/overlay mode
        #[arg(long, short = 'f', alias = "overlay", conflicts_with = "tiled")]
        floating: bool,
        /// Switch to mobile layout (HeliBoard / Gboard style)
        #[arg(long, conflicts_with = "desktop")]
        mobile: bool,
        /// Switch to desktop layout (Windows 11 style)
        #[arg(long, conflicts_with = "mobile")]
        desktop: bool,
    },
    /// Hide the on-screen keyboard
    Hide,
    /// Toggle on-screen keyboard visibility
    Toggle {
        /// Force tiled/exclusive mode
        #[arg(long, short = 't', conflicts_with = "floating")]
        tiled: bool,
        /// Force floating/overlay mode
        #[arg(long, short = 'f', alias = "overlay", conflicts_with = "tiled")]
        floating: bool,
        /// Switch to mobile layout (HeliBoard / Gboard style)
        #[arg(long, conflicts_with = "desktop")]
        mobile: bool,
        /// Switch to desktop layout (Windows 11 style)
        #[arg(long, conflicts_with = "mobile")]
        desktop: bool,
    },
    /// Switch keyboard layout mode directly (e.g. desktop, mobile)
    Mode {
        /// Layout mode name: "desktop" or "mobile"
        mode: String,
    },
    /// Load a custom keyboard layout JSON from ~/.config/hyprosk/layouts/<name>.json or file path
    Layout {
        /// Name of the layout in ~/.config/hyprosk/layouts/<name>.json or path to .json file
        name: String,
    },
    /// Toggle or switch to tiled/exclusive mode
    Tiled,
    /// Toggle or switch to floating/overlay mode
    Floating,
    /// Toggle between overlay mode (floating on top of windows) and exclusive mode (tiled windows)
    Exclusive,
    /// Toggle between overlay mode (floating on top of windows) and exclusive mode (tiled windows)
    Overlay,
    /// Reload configuration from ~/.config/hyprosk/config.toml
    Reload,
    /// Switch active keyboard layer (lower, upper, symbols)
    Layer { name: String },
    /// Toggle clipboard history view
    Clipboard,
    /// Print folio/keyboard detection diagnostics for debugging
    Status,
    /// Stop the running HyprOsk daemon
    Quit,
    /// Show HyprOsk and swipe engine attributions
    About,
    /// Compile an AOSP .combined wordlist into a fast binary .hyprosk.dict file
    CompileDict {
        /// Path to the input .combined wordlist
        input: PathBuf,
        /// Path to the output .hyprosk.dict binary file
        output: PathBuf,
        /// Maximum words to include (default: 55000)
        #[arg(long, default_value = "55000")]
        max_words: usize,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();

    let cli = Cli::parse();

    #[cfg(feature = "futo-swipe")]
    tracing::info!("Swipe typing powered by FUTO Swipe");

    match cli.command {
        None => {
            hyprosk::run_daemon(cli.config.as_deref())?;
        }
        Some(Commands::Daemon { config }) => {
            let conf_path = config.as_deref().or(cli.config.as_deref());
            hyprosk::run_daemon(conf_path)?;
        }
        Some(Commands::Show {
            tiled,
            floating,
            mobile,
            desktop,
        }) => {
            let cmd = if mobile {
                "show --mobile"
            } else if desktop {
                "show --desktop"
            } else if tiled {
                "show-tiled"
            } else if floating {
                "show-floating"
            } else {
                "show"
            };
            let resp = hyprosk::ipc::IpcServer::send_command(cmd)?;
            println!("{}", resp.trim());
        }
        Some(Commands::Hide) => {
            let resp = hyprosk::ipc::IpcServer::send_command("hide")?;
            println!("{}", resp.trim());
        }
        Some(Commands::Toggle {
            tiled,
            floating,
            mobile,
            desktop,
        }) => {
            let cmd = if mobile {
                "toggle --mobile"
            } else if desktop {
                "toggle --desktop"
            } else if tiled {
                "toggle-tiled"
            } else if floating {
                "toggle-floating"
            } else {
                "toggle"
            };
            let resp = hyprosk::ipc::IpcServer::send_command(cmd)?;
            println!("{}", resp.trim());
        }
        Some(Commands::Mode { mode }) => {
            let resp = hyprosk::ipc::IpcServer::send_command(&format!("mode {}", mode))?;
            println!("{}", resp.trim());
        }
        Some(Commands::Layout { name }) => {
            let resp = hyprosk::ipc::IpcServer::send_command(&format!("layout {}", name))?;
            println!("{}", resp.trim());
        }
        Some(Commands::Tiled) => {
            let resp = hyprosk::ipc::IpcServer::send_command("toggle-tiled")?;
            println!("{}", resp.trim());
        }
        Some(Commands::Floating) => {
            let resp = hyprosk::ipc::IpcServer::send_command("toggle-floating")?;
            println!("{}", resp.trim());
        }
        Some(Commands::Exclusive) | Some(Commands::Overlay) => {
            let resp = hyprosk::ipc::IpcServer::send_command("exclusive")?;
            println!("{}", resp.trim());
        }
        Some(Commands::Reload) => {
            let resp = hyprosk::ipc::IpcServer::send_command("reload")?;
            println!("{}", resp.trim());
        }
        Some(Commands::Layer { name }) => {
            let resp = hyprosk::ipc::IpcServer::send_command(&format!("layer {}", name))?;
            println!("{}", resp.trim());
        }
        Some(Commands::Clipboard) => {
            let resp = hyprosk::ipc::IpcServer::send_command("clipboard")?;
            println!("{}", resp.trim());
        }
        Some(Commands::Status) => {
            // One snapshot so the printed sources and the verdict cannot come
            // from different points in time (matters right after resume).
            let status = hyprosk::folio::FolioStatus::detect();
            println!("folio USB (044e:1218):       {}", status.usb_folio);
            println!("tablet_mode (SW_TABLET_MODE): {}", status.tablet_mode);
            println!("external keyboard:           {}", status.external_keyboard);
            println!("built-in i8042/acpi keyboard:{}", status.builtin_keyboard);
            println!("folio attached (verdict):    {}", status.attached);
            println!("folio source:                {}", status.source.label());
            let config = hyprosk::config::Config::load_or_create(cli.config.as_deref());
            println!(
                "folio_mode (config):         {}",
                config.behavior.folio_mode
            );
            println!(
                "auto-show will be:           {}",
                if !config.behavior.folio_mode || !status.attached {
                    "ENABLED"
                } else {
                    "SUPPRESSED"
                }
            );
            if status.builtin_keyboard && !status.external_keyboard {
                println!(
                    "note: the built-in i8042/ACPI keyboard keeps letter keys after the folio"
                );
                println!("      is removed, so it never counts as an attachment on its own.");
            }
            if config.behavior.folio_mode && status.source == hyprosk::folio::FolioSource::UsbFolio
            {
                println!("note: 044e:1218 stays powered when the folio is folded 360 degrees,");
                println!(
                    "      so this device cannot be told apart from laptop posture by USB ID."
                );
                println!("      Use 'hyprosk toggle' or 'hyprosk hide' in that posture.");
            }
        }
        Some(Commands::Quit) => {
            let resp = hyprosk::ipc::IpcServer::send_command("quit")?;
            println!("{}", resp.trim());
        }
        Some(Commands::About) => {
            println!("HyprOsk — native Wayland on-screen keyboard");
            #[cfg(feature = "futo-swipe")]
            println!("Swipe typing powered by FUTO Swipe technology.");
            #[cfg(not(feature = "futo-swipe"))]
            println!("Swipe typing uses the geometric fallback engine in this build.");
        }
        Some(Commands::CompileDict {
            input,
            output,
            max_words,
        }) => {
            println!(
                "Compiling {:?} -> {:?} (max {} words)...",
                input, output, max_words
            );
            let (words, bigrams, size) =
                hyprosk::suggest::compiler::compile_combined(&input, &output, max_words, 3)?;
            println!(
                "Success! Compiled {} words and {} bigrams into binary dictionary ({:.2} MB).",
                words,
                bigrams,
                size as f64 / (1024.0 * 1024.0)
            );
        }
    }

    Ok(())
}
