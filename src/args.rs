use clap::Parser;

use tracing_subscriber::fmt::time::ChronoLocal;

#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Args {
    #[clap(short, long)]
    pub debug: bool,
}

impl Args {
    pub fn init_debug(&self) {
        let mut builder = tracing_subscriber::fmt().with_timer(ChronoLocal::rfc_3339());

        if self.debug {
            builder = builder.with_max_level(tracing::Level::DEBUG);
        } else {
            builder = builder.with_max_level(tracing::Level::INFO);
        }

        builder.init();
    }

    pub fn init(&self) {
        self.init_debug();
        tracing::info!("Args initialized: {:#?}", self);
    }
}
