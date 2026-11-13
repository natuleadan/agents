pub mod data_views;
pub mod login;
pub mod panels;
pub mod shell;

pub use agents_styles::nsh::{web_assets, web_script};
pub use data_views::{build_cluster_and_metrics_panel, build_command_and_update_dialogs};
pub use login::build_login_screen;
pub use shell::{ClientController, render_app_shell};
