#[macro_use]
extern crate log;

use a653rs_linux_hypervisor::run_hypervisor;
use log::LevelFilter;

/// Helper to print top-level errors through [log::error]
#[quit::main]
fn main() {
    let level = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into());
    std::env::set_var("RUST_LOG", level.clone());

    pretty_env_logger::formatted_builder()
        .parse_filters(&level)
        //.format(a653rs_linux_core::log_helper::format)
        .filter_module("polling", LevelFilter::Off)
        .format_timestamp_secs()
        .init();

    match run_hypervisor() {
        Ok(_) => {}
        Err(e) => {
            error!("{e}");
            quit::with_code(1);
        }
    }
}
