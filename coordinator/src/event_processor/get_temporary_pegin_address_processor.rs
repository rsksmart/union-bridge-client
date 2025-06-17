use crate::event_processor::EventProcessor;
use anyhow::{Context, Result, bail};
use common::msg_broker::types::FromServer;
use log::info;
use reqwest::blocking::Client;
use serde_json::Value;
use std::env;

pub struct GetTemporaryPeginAddressProcessor {
    client: Client,
}

impl GetTemporaryPeginAddressProcessor {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    fn proxy_peg_in_address_request(&self, json_value: &Value) -> Result<Value> {
        let res = self
            .client
            .post(format!("{}/pegin-address", Self::get_tx_dispatcher_url())) // TODO: Remove http client
            .json(json_value)
            .send()
            .context("Failed to send request to /pegin-address")?;

        if res.status().is_success() {
            let result: Value = res.json().context("Failed to parse response as JSON")?;
            Ok(result)
        } else {
            let status = res.status();
            let text = res.text().unwrap_or_else(|_| "<no body>".to_string());
            bail!("Request failed: {status} - {text}");
        }
    }

    fn get_tx_dispatcher_url() -> String {
        // env var because the http server is temporary: defined for docker, defaulting otherwise
        env::var("TRANSACTION_DISPATCHER_URL").unwrap_or("http://0.0.0.0:3000".to_string())
    }
}

impl EventProcessor for GetTemporaryPeginAddressProcessor {
    fn process_new_bitvmx_event(&mut self, event: &FromServer) -> Result<()> {
        match event {
            FromServer::GetTemporaryPegInAddress(value) => {
                let result = self.proxy_peg_in_address_request(value)?;
                info!(
                    "Successfully proxied pegin address request. Response: {}",
                    result
                );
                // TODO notify end user about the result
                Ok(())
            }
            _ => return Ok(()), // ignore unrelated events
        }
    }

    fn shutdown(&mut self) {
        info!("Shutting down GetTemporaryPeginAddressProcessor");
    }
}
