use crate::{
    event_processor::EventProcessor,
    types::{RegisteredPegInRequestEvent, RskPegManagerEvents},
};
use anyhow::{Context, Result, bail};
use bitvmx_client::types::IncomingBitVMXApiMessages;
use common::{
    msg_broker::{
        broker::{BROKER_SERVER_ID, BrokerClientApi},
        types::{FromServer, ToServer},
    },
    types::RskBlockAndUncles,
};
use log::info;
use reqwest::blocking::Client;
use serde_json::Value;
use std::env;
use uuid::Uuid;

const REGISTERED_PEGIN_REQUEST_CONFIRMATIONS: u32 = 5;

pub struct PeginProcessor<T: BrokerClientApi> {
    http_client: Client,
    bitvmx_broker: T,
    register_pegin_events: Vec<UnconfirmedEvent<RegisteredPegInRequestEvent>>,
}

#[derive(Debug, Clone)]
struct UnconfirmedEvent<T: Clone> {
    event_id: Uuid,
    data: T,
    confirmations: Confirmations,
}

impl<T: Clone> UnconfirmedEvent<T> {
    fn new(data: T, required_confirmations: u32) -> Self {
        let event_id = Uuid::new_v4();
        let confirmations = Confirmations::new(event_id.to_string(), required_confirmations);

        Self {
            event_id,
            data,
            confirmations,
        }
    }

    fn register_confirmation(&mut self) {
        self.confirmations.update(false);
    }

    fn is_confirmed(&self) -> bool {
        self.confirmations.is_confirmed()
    }
}

#[derive(Debug, Clone)]
struct Confirmations {
    flow_id: String,
    accum: u32,
    req: u32,
}

impl Confirmations {
    pub fn new(flow_id: String, req_confirmations: u32) -> Self {
        Self {
            flow_id,
            accum: 0,
            req: req_confirmations,
        }
    }

    pub fn update(&mut self, removed: bool) {
        if removed {
            self.accum = self.accum.saturating_sub(1);
            info!(
                "Removed confirmation for {}. Status: {}/{}",
                self.flow_id, self.accum, self.req
            );
        } else {
            self.accum = self.accum.saturating_add(1);
            info!(
                "Added confirmation to {}. Status: {}/{}",
                self.flow_id, self.accum, self.req
            );
        }
    }

    pub fn is_confirmed(&self) -> bool {
        self.accum >= self.req
    }
}

impl<T: BrokerClientApi> PeginProcessor<T> {
    pub fn new(bitvmx_broker: T) -> Self {
        Self {
            http_client: Client::new(),
            bitvmx_broker,
            register_pegin_events: Vec::new(),
        }
    }

    fn proxy_request(&self, method_name: &str, json_value: &Value) -> Result<Value> {
        let url = format!("{}/{}", Self::get_tx_dispatcher_url(), method_name);

        let res = self
            .http_client
            .post(url)
            .json(json_value)
            .send()
            .context("Failed to send request")?;

        if res.status().is_success() {
            let result: Value = res.json().context("Failed to parse response as JSON")?;
            Ok(result)
        } else {
            let status = res.status();
            let text = res.text().unwrap_or_else(|_| "<no body>".to_string());
            bail!("Request failed: {status} - {text}");
        }
    }

    fn send_response_to_bitvmx(&self, method: &str, payload: Value) -> Result<bool> {
        let msg = match method {
            // For now send the result back to bitvmx client mock, in the future
            // this will probably go to the end user
            "pegin-address" => ToServer::TemporaryPegInAddressMockedBitVMX(payload),
            // TODO: figure out what goes here
            "register-pegin" => ToServer::ToBitVMX(IncomingBitVMXApiMessages::Ping()),
            _ => bail!("Unsupported method name for BitVMX response: {}", method),
        };

        Ok(self.bitvmx_broker.send(BROKER_SERVER_ID, msg)?)
    }

    fn get_tx_dispatcher_url() -> String {
        // env var because the http server is temporary: defined for docker, defaulting otherwise
        env::var("TRANSACTION_DISPATCHER_URL").unwrap_or("http://0.0.0.0:3000".to_string())
    }
}

impl<T: BrokerClientApi> EventProcessor for PeginProcessor<T> {
    fn process_new_bitvmx_event(&mut self, event: &FromServer) -> Result<()> {
        match event {
            FromServer::FromBitVMX(method, data) => {
                info!(
                    "Handling BitVMX event. Method: {}, Payload: {:?}",
                    method, data
                );

                let result = self.proxy_request(method, data)?;
                info!(
                    "Successfully proxied request for method '{}'. Response: {}",
                    method, result
                );

                self.send_response_to_bitvmx(method, result.clone())?;
                info!(
                    "Successfully sent response to BitVMX broker for method '{}'. Payload: {}",
                    method, result
                );
            }
            _ => {} // ignore unrelated events
        }

        Ok(())
    }

    fn process_new_event(&mut self, event: &RskPegManagerEvents) -> Result<()> {
        match event {
            RskPegManagerEvents::RegisteredPegInRequest(data) => {
                info!(
                    "Handling Union Bridge RegisteredPegInRequest event: {:?}",
                    data
                );

                let unconfirmed_event =
                    UnconfirmedEvent::new(data.clone(), REGISTERED_PEGIN_REQUEST_CONFIRMATIONS);

                self.register_pegin_events.push(unconfirmed_event.clone());

                info!(
                    "Successfully added RegisteredPegInRequest event to unconfirmed queue. Event: {:?}",
                    unconfirmed_event
                );
            }
            _ => {} // ignore unrelated events
        }

        Ok(())
    }

    fn process_new_block(&mut self, _block_with_uncles: &RskBlockAndUncles) -> Result<()> {
        // TODO: handle a reorg case, we should remove from queue?

        if self.register_pegin_events.is_empty() {
            return Ok(());
        }

        let mut retained = Vec::new();

        for mut event in self.register_pegin_events.drain(..) {
            event.register_confirmation();

            if event.is_confirmed() {
                // TODO: Replace with actual message, figure out what goes inside Setup
                self.bitvmx_broker.send(
                    BROKER_SERVER_ID,
                    ToServer::ToBitVMX(IncomingBitVMXApiMessages::Ping()),
                )?;
            } else {
                retained.push(event);
            }
        }

        self.register_pegin_events = retained;

        Ok(())
    }

    fn shutdown(&mut self) {
        info!("Shutting down GetTemporaryPeginAddressProcessor");
    }
}
