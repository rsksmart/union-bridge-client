use crate::types::{RskBlock, RskRpcBlock};
use alloy_provider::{Provider, ProviderBuilder, RootProvider, WsConnect};
use alloy_pubsub::{PubSubFrontend, Subscription, SubscriptionItem};
use alloy_rpc_types::Header;
use anyhow::{anyhow, bail, Ok, Result};
use async_trait::async_trait;
use log::debug;
use serde_json::{json, Value};

#[async_trait]
pub trait RskWsProvider {
    type SubscribedTo: RskBlockSubscription;

    async fn subscribe_blocks(&self) -> Result<Self::SubscribedTo>;
    async fn disconnect(self) -> Result<()>;
}

#[async_trait]
pub trait RskBlockSubscription {
    async fn next(&mut self) -> Result<RskBlock>;
    async fn unsubscribe(self) -> Result<()>;
}

pub struct AlloyRskWsProvider {
    provider: RootProvider<PubSubFrontend>,
}

impl AlloyRskWsProvider {
    pub async fn new(url: &str) -> Result<AlloyRskWsProvider> {
        let ws = WsConnect::new(url);
        let provider = ProviderBuilder::new().on_ws(ws).await?;
        Ok(AlloyRskWsProvider { provider })
    }
}

#[async_trait]
impl RskWsProvider for AlloyRskWsProvider {
    type SubscribedTo = AlloyBlockSubscription;

    async fn subscribe_blocks(&self) -> Result<Self::SubscribedTo> {
        let sub = self.provider.subscribe_blocks().await?;
        Ok(AlloyBlockSubscription {
            sub,
            provider: self.provider.clone(),
        })
    }

    async fn disconnect(self) -> Result<()> {
        drop(self.provider);
        Ok(())
    }
}

pub struct AlloyBlockSubscription {
    sub: Subscription<Header>,
    provider: RootProvider<PubSubFrontend>,
}

#[async_trait]
impl RskBlockSubscription for AlloyBlockSubscription {
    async fn next(&mut self) -> Result<RskBlock> {
        // need to use recv_any to bypass certain field mismatch between Rootstock and Ethereum that allow is not handling
        let header = self.sub.recv_any().await?;
        debug!("Received header: {:?}", header);

        let new_block_header_raw = match header {
            SubscriptionItem::Other(raw_json) => raw_json.get().to_string(),
            _ => {
                bail!("Unexpected SubscriptionItem: {:?}", header);
            }
        };

        let new_block_header: Value = serde_json::from_str(&*new_block_header_raw)?;
        let new_block_hash = new_block_header["hash"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing hash field"))?;

        let new_block = fetch_block_data(&self.provider, Some(new_block_hash), None).await?;
        Ok(new_block)
    }

    async fn unsubscribe(self) -> Result<()> {
        drop(self.sub);
        Ok(())
    }
}

async fn fetch_block_data(
    provider: &RootProvider<PubSubFrontend>,
    block_hash: Option<&str>,
    block_number: Option<&u64>, // TODO improve with wrapper type like BlockId
) -> Result<RskBlock> {
    if block_hash.is_none() && block_number.is_none() {
        bail!("Either block_hash or block_number_or_ref must be provided");
    }

    if block_hash.is_some() && block_number.is_some() {
        bail!("Only one of block_hash or block_number_or_ref must be provided");
    }

    let (method, block_id) = if block_hash.is_some() {
        ("eth_getBlockByHash", block_hash.unwrap().to_string())
    } else {
        (
            "eth_getBlockByNumber",
            format!("0x{:x}", block_number.unwrap()),
        )
    };

    let response: Value = provider
        .client()
        .request(method, vec![json!(block_id), json!(false)])
        .await?;

    // TODO resilience when response is not a block (ie not found)

    let rpc_block: RskRpcBlock = serde_json::from_value(response)?;
    let rsk_block: RskBlock = RskBlock::from(rpc_block);

    Ok(rsk_block)
}
