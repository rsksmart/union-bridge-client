use crate::key_manager::KeyManager;
use anyhow::{Ok, Result};
use clap::{Parser, Subcommand};
use std::path::Path;

pub struct Cli {}

#[derive(Parser)]
#[command(about = "Key Manager CLI", long_about = None)]
#[command(arg_required_else_help = true)]
pub struct Menu {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    NewKey {
        #[arg(value_name = "password", short = 'p', long = "password")]
        password: String,
        #[arg(value_name = "destination", short = 'd', long = "destination")]
        destination: String,
    },

    DerivePublicData {
        #[arg(value_name = "password", short = 'p', long = "password")]
        password: String,
        #[arg(value_name = "key_file", short = 'k', long = "key_file")]
        key_file: String,
    },
}

impl Default for Cli {
    fn default() -> Self {
        Self::new()
    }
}

impl Cli {
    pub fn new() -> Self {
        Self {}
    }

    pub fn run(&self) -> Result<()> {
        let menu = Menu::parse();

        match &menu.command {
            Commands::NewKey {
                password,
                destination,
            } => {
                let path = Path::new(destination);
                let (file, public_key, address) = KeyManager::generate_key(path, password)?;
                println!(
                    "Generated key @ {}, public '{}', address '{}'",
                    file, public_key, address
                );
                Ok(())
            }
            Commands::DerivePublicData { password, key_file } => {
                let path = Path::new(key_file);
                let (public_key, address) =
                    KeyManager::derive_public_key_and_address(path, password)?;
                println!(
                    "Derived key @ {}, public '{}', address '{}'",
                    path.to_str().unwrap(),
                    public_key,
                    address
                );
                Ok(())
            }
        }
    }
}
