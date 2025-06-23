use crate::event_processor::EventProcessor;
use anyhow::{Context, Result, bail};
use common::msg_broker::{
    broker::{BROKER_SERVER_ID, BrokerClientApi},
    types::{FromServer, ToServer},
};
use log::info;
use reqwest::blocking::Client;
use serde_json::Value;
use std::env;

pub struct GetTemporaryPeginAddressProcessor<BC: BrokerClientApi> {
    http_client: Client,
    bitvmx_broker: BC,
}

impl<BC: BrokerClientApi> GetTemporaryPeginAddressProcessor<BC> {
    pub fn new(bitvmx_broker: BC) -> Self {
        Self {
            http_client: Client::new(),
            bitvmx_broker,
        }
    }

    fn proxy_peg_in_address_request(&self, json_value: &Value) -> Result<Value> {
        let res = self
            .http_client
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

impl<BC: BrokerClientApi> EventProcessor for GetTemporaryPeginAddressProcessor<BC> {
    fn process_new_bitvmx_event(&mut self, event: &FromServer) -> Result<()> {
        match event {
            FromServer::GetTemporaryPegInAddress(value) => {
                let result = self.proxy_peg_in_address_request(value)?;
                info!(
                    "Successfully proxied pegin address request. Response: {}",
                    result
                );

                // For now send the result back to bitvmx client mock, in the future
                // this will probably go to the end user
                self.bitvmx_broker.send(
                    BROKER_SERVER_ID,
                    ToServer::TemporaryPegInAddressMockedBitVMX(result),
                )?;
            }
            _ => {} // ignore unrelated events
        }

        Ok(())
    }

    fn shutdown(&mut self) {
        info!("Shutting down GetTemporaryPeginAddressProcessor");
    }
}
