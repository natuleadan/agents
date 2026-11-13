#![forbid(unsafe_code)]

use agents_client::core::backend::BackendTarget;
use agents_client::core::state::{AppAction, AppState, dispatch};
use agents_client::styles::nsh::{SurfaceMetrics, TargetPlatform};
use agents_client::ui::render_app_shell;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "agents-client",
    about = "Multi-platform NSH & TUI client for agents"
)]
struct Cli {
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    server_url: String,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Launch interactive ratatui terminal interface.
    Tui,
    /// Render declarative NSH UI projection for a target platform.
    Ui {
        #[arg(long, default_value = "linux")]
        target: String,
    },
    /// Show resolved client profile badge and supported capabilities.
    Status,
}

fn main() {
    let cli = Cli::parse();
    let target = BackendTarget::local_server(&cli.server_url);
    let mut state = AppState::default();
    dispatch(
        &mut state,
        AppAction::UpdateEffectiveProfile(target.effective_profile()),
    );

    match cli.command.unwrap_or(Commands::Status) {
        Commands::Status => {
            println!("profile={}", state.effective_profile.label);
        }
        Commands::Ui { target } => {
            let platform = TargetPlatform::parse(&target).unwrap_or(TargetPlatform::Linux);
            let metrics = match platform.idiom() {
                agents_client::styles::nsh::Idiom::Mobile => SurfaceMetrics::mobile(platform),
                agents_client::styles::nsh::Idiom::Web => SurfaceMetrics::web(1280.0, 800.0),
                _ => SurfaceMetrics::desktop(platform),
            };
            let tree = render_app_shell(&state, metrics);
            println!("{}", tree.render_html_fragment());
        }
        Commands::Tui => {
            #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
            {
                if let Err(err) = agents_client::tui::run_terminal_tui(state) {
                    eprintln!("tui error: {err}");
                    std::process::exit(1);
                }
            }
            #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
            {
                eprintln!("interactive terminal TUI is only available on desktop targets");
            }
        }
    }
}
