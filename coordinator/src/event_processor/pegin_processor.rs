use crate::{
    event_processor::{
        EventProcessor,
        blockchain_tracker::{BlockConfirmations, BlockchainObserver, BlockchainView},
    },
    types::{RegisteredPegInRequestEvent, RskPegManagerEvents},
};
use anyhow::{Context, Result, bail};
use bitvmx_client::{program::variables::VariableTypes, types::IncomingBitVMXApiMessages};
use common::{
    msg_broker::{
        broker::{BROKER_SERVER_ID, BrokerClientApi},
        types::{FromServer, ToServer},
    },
    types::RskBlockAndUncles,
};
use log::info;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::{cell::RefCell, env, rc::Rc};
use uuid::Uuid;

const REGISTERED_PEGIN_REQUEST_CONFIRMATIONS: u32 = 5;

#[derive(Debug, Clone)]
struct UnconfirmedEvent<T: Clone> {
    event_id: Uuid,
    data: T,
    confirmations: BlockConfirmations,
}

impl<T: Clone> UnconfirmedEvent<T> {
    fn new(data: T, required_confirmations: u32) -> Self {
        let event_id = Uuid::new_v4();
        let confirmations = BlockConfirmations::new(event_id.to_string(), required_confirmations);

        Self {
            event_id,
            data,
            confirmations,
        }
    }

    fn is_confirmed(&self) -> bool {
        self.confirmations.is_confirmed()
    }
}

impl<T: Clone> BlockchainObserver for UnconfirmedEvent<T> {
    fn get_id(&self) -> String {
        self.event_id.to_string()
    }

    fn on_block_added(&mut self, block: &RskBlockAndUncles) {
        self.confirmations.on_block_added(block);
    }

    fn on_block_removed(&mut self, block: &RskBlockAndUncles) {
        self.confirmations.on_block_removed(block);
    }
}

pub struct PeginProcessor<T: BrokerClientApi> {
    http_client: Client,
    bitvmx_broker: T,
    blockchain: BlockchainView,
    register_pegin_events: Vec<UnconfirmedEvent<RegisteredPegInRequestEvent>>,
}

impl<T: BrokerClientApi> PeginProcessor<T> {
    pub fn new(bitvmx_broker: T) -> Self {
        Self {
            http_client: Client::new(),
            bitvmx_broker,
            blockchain: BlockchainView::new(),
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
        match method {
            "pegin-address" => {
                let msg = ToServer::TemporaryPegInAddressMockedBitVMX(payload);
                Ok(self.bitvmx_broker.send(BROKER_SERVER_ID, msg)?)
            }
            "register-pegin" => {
                // No response needed for register-pegin
                Ok(true)
            }
            _ => bail!("Unsupported method name for BitVMX response: {}", method),
        }
    }

    fn process_confirmed_register_pegin_events(&mut self) -> Result<()> {
        let mut retained = Vec::new();
        let events: Vec<_> = self.register_pegin_events.drain(..).collect();

        for event in events {
            if event.is_confirmed() {
                let event_id = event.event_id;
                let data = event.data.inner;
                self.handle_confirmed_event(event_id, "RegisteredPegInRequest", data)?;
            } else {
                retained.push(event);
            }
        }

        self.register_pegin_events = retained;
        Ok(())
    }

    fn handle_confirmed_event<E: serde::Serialize>(
        &mut self,
        event_id: Uuid,
        variable_name: &str,
        data: E,
    ) -> Result<()> {
        let data = json!(data).to_string();

        self.bitvmx_broker.send(
            BROKER_SERVER_ID,
            ToServer::ToBitVMX(IncomingBitVMXApiMessages::SetVar(
                event_id,
                variable_name.to_string(),
                VariableTypes::String(data),
            )),
        )?;

        self.blockchain
            .remove_observer(event_id.to_string().as_str());

        Ok(())
    }

    fn get_tx_dispatcher_url() -> String {
        // Env var because the http server is temporary: defined for docker, defaulting otherwise
        env::var("TRANSACTION_DISPATCHER_URL").unwrap_or("http://0.0.0.0:3000".to_string())
    }
}

impl<T: BrokerClientApi> EventProcessor for PeginProcessor<T> {
    fn process_new_bitvmx_event(&mut self, event: &FromServer) -> Result<()> {
        match event {
            FromServer::FromBitVMX(method, data)
                if matches!(method.as_str(), "pegin-address" | "register-pegin") =>
            {
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
            _ => {}
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
                self.blockchain
                    .add_observer(Rc::new(RefCell::new(unconfirmed_event.clone())));

                info!(
                    "Successfully added RegisteredPegInRequest event to unconfirmed queue. Event: {:?}",
                    unconfirmed_event
                );
            }
            _ => {}
        }

        Ok(())
    }

    fn process_new_block(&mut self, block: &RskBlockAndUncles) -> Result<()> {
        if self.register_pegin_events.is_empty() {
            return Ok(());
        }

        self.blockchain.update(block.clone());

        self.process_confirmed_register_pegin_events()?;

        Ok(())
    }

    fn shutdown(&mut self) {
        info!("Shutting down GetTemporaryPeginAddressProcessor");

        self.blockchain.clear();
    }
}
