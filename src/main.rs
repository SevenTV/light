use std::process::exit;

use log::{debug, error};

mod api;
mod config;
mod context;
mod errors;
mod types;
mod utils;

use utils::logger;

#[tokio::main]
async fn main() {
    let logger = logger::init();
    let mut config = config::args::parse();

    config::env::parse().merge(&mut config);

    let config_file = config.get_config_file();
    let was_set = config_file.is_some();
    let path = config_file.unwrap_or_else(|| "config.yaml".to_string());
    if !path.is_empty() {
        let result = config::file::parse(path);
        if let Ok(result) = result {
            result.merge(&mut config);
        } else {
            error!("Failed to parse config file: {}", result.unwrap_err());
            if was_set {
                exit(1);
            }
        }
    }

    if config.get_debug() {
        logger.set_level(log::LevelFilter::Debug);
        debug!("debug mode enabled");
    } else {
        logger.set_level(log::LevelFilter::Info);
    }

    let context = context::Context { config };

    let result = context.run().await;
    if let Err(err) = result {
        error!("{:?}", err);
        exit(1);
    }

    exit(0);
}
