use crate::event_processor::EventProcessor;
use anyhow::{Context, Result, bail};
use common::msg_broker::types::BrokerResponses;
use log::info;
use reqwest::blocking::Client;
use serde_json::Value;

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
            .post("http://0.0.0.0:3000/pegin-address") // TODO: Remove http client
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
}

impl EventProcessor for GetTemporaryPeginAddressProcessor {
    fn process_new_bitvmx_event(&mut self, event: &BrokerResponses) -> Result<()> {
        match event {
            BrokerResponses::GetTemporaryPegInAddress(value) => {
                let result = self.proxy_peg_in_address_request(value)?;
                info!(
                    "Successfully proxied pegin address request. Response: {}",
                    result
                );
                Ok(())
            }
            _ => return Ok(()), // ignore unrelated events
        }
    }

    fn shutdown(&mut self) {
        info!("Shutting down GetTemporaryPeginAddressProcessor");
    }
}
