mod client;

use client::TomorinClient;
use std::time::Duration;
use tokio::task;

use super::conf::Conf;

pub struct UserBot {
    client: TomorinClient,
}

impl UserBot {
    pub async fn new(conf: Conf) -> anyhow::Result<Self> {
        Ok(Self {
            client: TomorinClient::new(conf).await?,
        })
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        loop {
            let update_res = tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                u = self.client.next_update() => u,
            };

            let Ok(update) = update_res else {
                tracing::warn!("Failed to get update");
                continue;
            };

            let handler = self.client.handler();
            task::spawn(async move {
                match handler.update(update).await {
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!("Error handling update: {e}");
                        tracing::error!("Tomorin will retry after 30 secs");
                        tokio::time::sleep(Duration::from_secs(30)).await;
                    }
                }
            });
        }

        Ok(())
    }
}
