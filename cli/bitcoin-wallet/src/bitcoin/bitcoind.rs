#![allow(dead_code)]

use anyhow::Result;
use std::default::Default;

use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, RemoveContainerOptions,
};
use bollard::errors::Error;
use bollard::image::CreateImageOptions;
use bollard::models::{ContainerCreateResponse, HostConfig};
use futures_util::stream::StreamExt;
use tokio::runtime::Runtime;

use bitcoin::Network;
use bitcoincore_rpc::{Auth, Client};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct RpcConfig {
    pub network: Network,
    pub url: String,
    pub username: String,
    pub password: String,
    pub wallet: String,
}

impl RpcConfig {
    pub fn new(
        network: Network,
        url: String,
        username: String,
        password: String,
        wallet: String,
    ) -> Self {
        Self { network, url, username, password, wallet }
    }
}

pub struct Bitcoind {
    docker: Docker,
    container_name: String,
    image: String,
    runtime: Runtime,
    rpc_config: RpcConfig,
    flags: BitcoindFlags,
}

#[derive(Debug, Clone)]
pub struct BitcoindFlags {
    pub min_relay_tx_fee: f64,
    pub block_min_tx_fee: f64,
    pub debug: u8,
    pub fallback_fee: f64,
}

impl Default for BitcoindFlags {
    fn default() -> Self {
        BitcoindFlags {
            min_relay_tx_fee: 0.00001,
            block_min_tx_fee: 0.00001,
            debug: 1,
            fallback_fee: 0.0002,
        }
    }
}

impl Bitcoind {
    pub fn new(container_name: &str, image: &str, rpc_config: RpcConfig) -> Self {
        Self::new_with_flags(container_name, image, rpc_config, BitcoindFlags::default())
    }

    pub fn new_with_flags(
        container_name: &str,
        image: &str,
        rpc_config: RpcConfig,
        flags: BitcoindFlags,
    ) -> Self {
        Self {
            docker: Docker::connect_with_local_defaults().unwrap(),
            container_name: container_name.to_string(),
            image: image.to_string(),
            runtime: Runtime::new().unwrap(),
            rpc_config,
            flags,
        }
    }

    pub fn start(&self) -> Result<(), Error> {
        println!("Checking if Docker daemon is active");
        let ping_result = self.runtime.block_on(async { self.docker.ping().await });

        if ping_result.is_err() {
            return Err(Error::DockerResponseNotFoundError {
                message:
                    "Docker deamon is not running. Make sure to start it before running this test"
                        .to_string(),
            });
        }

        println!("Starting bitcoind container");
        self.runtime.block_on(async {
            self.internal_stop().await?;

            let err = self.create_and_start_container().await;
            if let Err(err) = err {
                if err.to_string().contains("No such image") {
                    self.pull_image_if_not_present().await?;
                    self.create_and_start_container().await?;
                } else {
                    return Err(err);
                }
            }

            Ok(())
        })
    }

    pub fn stop(&self) -> Result<(), Error> {
        println!("Stopping bitcoind container");
        self.runtime.block_on(async {
            self.internal_stop().await?;
            Ok(())
        })
    }

    pub fn rpc_client(&self) -> bitcoincore_rpc::Result<Client> {
        Client::new(
            &self.rpc_config.url,
            Auth::UserPass(self.rpc_config.username.clone(), self.rpc_config.password.clone()),
        )
    }

    pub fn wallet_client(&self) -> bitcoincore_rpc::Result<Client> {
        let wallet_url = format!("{}/wallet/{}", self.rpc_config.url, self.rpc_config.wallet);
        Client::new(
            &wallet_url,
            Auth::UserPass(self.rpc_config.username.clone(), self.rpc_config.password.clone()),
        )
    }

    pub fn rpc_config(&self) -> &RpcConfig {
        &self.rpc_config
    }

    async fn internal_stop(&self) -> Result<(), Error> {
        if self.is_running().await? {
            println!("Container was running. Stopping bitcoind container");
            self.docker
                .remove_container(
                    &self.container_name,
                    Some(RemoveContainerOptions { force: true, ..Default::default() }),
                )
                .await?;
            for _ in 0..10 {
                if !self.is_running().await? {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                println!("Waiting for bitcoind container to stop");
            }
        }
        Ok(())
    }

    async fn is_running(&self) -> Result<bool, Error> {
        let containers = self
            .docker
            .list_containers(Some(ListContainersOptions::<String> {
                all: true,
                ..Default::default()
            }))
            .await?;
        for container in containers {
            if let Some(names) = container.names {
                if names.contains(&format!("/{}", self.container_name)) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    async fn pull_image_if_not_present(&self) -> Result<(), Error> {
        println!("Image not found locally. Pulling image: {}", self.image);
        let options = Some(CreateImageOptions {
            from_image: self.image.clone(),
            tag: "latest".to_string(),
            ..Default::default()
        });

        let mut stream = self.docker.create_image(options, None, None);
        while let Some(result) = stream.next().await {
            match result {
                Ok(progress) => {
                    println!("Progress: {:?}", progress.progress);
                }
                Err(error) => {
                    return Err(error);
                }
            }
        }

        Ok(())
    }

    async fn create_and_start_container(&self) -> Result<(), Error> {
        println!("Creating and starting bitcoind container");

        let min_relay_tx_fee = format!("-minrelaytxfee={}", self.flags.min_relay_tx_fee);
        let block_min_tx_fee = format!("-blockmintxfee={}", self.flags.block_min_tx_fee);
        let debug = format!("-debug={}", self.flags.debug);
        let fallback_fee = format!("-fallbackfee={}", self.flags.fallback_fee);

        let config = Config {
            image: Some(self.image.clone()),
            env: Some(vec!["BITCOIN_DATA=/data".to_string()]),
            host_config: Some(HostConfig {
                auto_remove: Some(true),
                port_bindings: Some(
                    [(
                        "18443/tcp".to_string(),
                        Some(vec![bollard::service::PortBinding {
                            host_ip: Some("0.0.0.0".to_string()),
                            host_port: Some("18443".to_string()),
                        }]),
                    )]
                    .iter()
                    .cloned()
                    .collect(),
                ),
                ..Default::default()
            }),
            cmd: Some(vec![
                "-regtest=1".to_string(),
                "-printtoconsole".to_string(),
                "-rpcallowip=0.0.0.0/0".to_string(),
                "-rpcbind=0.0.0.0".to_string(),
                format!("-rpcuser={}", self.rpc_config.username).to_string(),
                format!("-rpcpassword={}", self.rpc_config.password).to_string(),
                "-server=1".to_string(),
                "-txindex=1".to_string(),
                debug,
                min_relay_tx_fee,
                block_min_tx_fee,
                fallback_fee,
            ]),
            ..Default::default()
        };
        let ContainerCreateResponse { id, .. } = self
            .docker
            .create_container::<&str, String>(
                Some(CreateContainerOptions { name: &self.container_name }),
                config,
            )
            .await?;
        self.docker.start_container::<String>(&id, None).await?;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        Ok(())
    }
}
