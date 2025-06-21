#![feature(async_fn_traits)]
#![feature(fn_traits, unboxed_closures)]
mod args;
mod bot;
mod conf;

use args::Args;
use clap::Parser;

pub async fn run() -> anyhow::Result<()> {
    Args::parse().init();

    let conf = conf::Conf::load_or_create()
        .map_err(|e| anyhow::anyhow!("Failed to load or create configuration: {e}"))?;

    bot::UserBot::new(conf).await?.run().await
}
