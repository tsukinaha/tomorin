mod client;

use client::TomorinClient;
use futures_util::future::{Either, select};
use std::{pin::pin, sync::Arc};
use tokio::task;

use super::conf::Conf;

pub struct UserBot {
    client: Arc<TomorinClient>,
}

impl UserBot {
    pub async fn new(conf: Conf) -> anyhow::Result<Self> {
        Ok(Self {
            client: Arc::new(TomorinClient::new(conf).await?),
        })
    }

    pub async fn run(self) -> anyhow::Result<()> {
        loop {
            let exit = pin!(async { tokio::signal::ctrl_c().await });
            let upd = pin!(async { self.client.next_update().await });

            let update = match select(exit, upd).await {
                Either::Left(_) => break,
                Either::Right((u, _)) => u,
            };

            let Ok(update) = update else {
                tracing::warn!("Failed to get update");
                continue;
            };

            let client = self.client.clone();
            task::spawn(async move {
                match client.update(update).await {
                    Ok(_) => {}
                    Err(e) => tracing::error!("Error handling update: {e}"),
                }
            });
        }

        Ok(())
    }
}
