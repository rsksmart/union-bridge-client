use std::path::PathBuf;

use clap::Parser;

use anyhow::Result;
use rustyline::completion::{Completer, Pair};
use rustyline::config::{CompletionType, Config as RustyConfig, EditMode};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Context as RustyContext, Editor, Helper, Result as RustyResult};

#[derive(Parser, Debug, Clone, Default)]
#[command(author, version, about = "Simple P2WPKH wallet CLI", long_about = None)]
pub struct CliOpts {
    #[arg(long, env = "WALLET_CONFIG", value_name = "PATH")]
    pub config: Option<PathBuf>,
    #[arg(long, env = "WALLET_CONFIG_DIR", value_name = "DIR")]
    pub config_dir: Option<PathBuf>,
    #[arg(long, env = "WALLET_UTXO_DB", value_name = "PATH")]
    pub utxo_db: Option<PathBuf>,
    #[arg(long, env = "WALLET_NETWORK", value_name = "NETWORK")]
    pub network: Option<String>,
    #[arg(long, env = "WALLET_SATS_PER_BYTE", value_name = "SAT_PER_BYTE")]
    pub sats_per_byte: Option<u64>,
    #[arg(long = "private-key", env = "WALLET_PRIVATE_KEY", value_name = "WIF")]
    pub private_key_wif: Option<String>,
    #[arg(long, env = "WALLET_RPC_URL", value_name = "URL")]
    pub rpc_url: Option<String>,
    #[arg(long, env = "WALLET_RPC_USER", value_name = "USER")]
    pub rpc_user: Option<String>,
    #[arg(long, env = "WALLET_RPC_PASSWORD", value_name = "PASS")]
    pub rpc_password: Option<String>,
}

const COMMANDS: &[&str] = &[
    "help",
    "exit",
    "quit",
    "import_private_key",
    "generate_address",
    "list_addresses",
    "switch_address",
    "start_regtest_client",
    "register_utxo",
    "list_funds",
    "send_to_pubkey",
    "send_to_address",
    "mine_block",
    "mine_utxo",
    "tx_status",
    "clear_db",
];

#[derive(Default)]
pub struct CliHelper;

impl CliHelper {
    fn command_pairs<'a>(items: impl Iterator<Item = &'a str>, prefix: &str) -> Vec<Pair> {
        items
            .filter(|candidate| candidate.starts_with(prefix))
            .map(|candidate| Pair {
                display: candidate.to_string(),
                replacement: candidate.to_string(),
            })
            .collect()
    }

    fn complete_for_tokens(&self, tokens: &[&str], prefix: &str) -> Vec<Pair> {
        if tokens.is_empty() {
            return Self::command_pairs(COMMANDS.iter().copied(), prefix);
        }

        match tokens[0] {
            "list_funds" if tokens.len() == 1 => {
                const LIST_FUNDS_ARGS: [&str; 1] = ["all"];
                Self::command_pairs(LIST_FUNDS_ARGS.iter().copied(), prefix)
            }
            _ => Vec::new(),
        }
    }
}

impl Helper for CliHelper {}

impl Hinter for CliHelper {
    type Hint = String;
}

impl Highlighter for CliHelper {}

impl Validator for CliHelper {}

impl Completer for CliHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &RustyContext<'_>,
    ) -> RustyResult<(usize, Vec<Pair>)> {
        let upto_cursor = &line[..pos];
        let start = upto_cursor
            .rfind(char::is_whitespace)
            .map(|idx| idx + 1)
            .unwrap_or(0);
        let prefix = &upto_cursor[start..];
        let head = upto_cursor[..start].trim_end();
        let tokens: Vec<&str> = head.split_whitespace().collect();

        let candidates = self.complete_for_tokens(&tokens, prefix);

        Ok((start, candidates))
    }
}

pub fn setup_editor(
    history_path: &PathBuf,
) -> Result<Editor<CliHelper, rustyline::history::FileHistory>, anyhow::Error> {
    let rl_config = RustyConfig::builder()
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .edit_mode(EditMode::Emacs)
        .auto_add_history(false)
        .max_history_size(100)?
        .build();
    let mut editor: Editor<CliHelper, DefaultHistory> = Editor::with_config(rl_config)?;
    editor.set_helper(Some(CliHelper::default()));
    if history_path.exists() {
        if let Err(err) = editor.load_history(&history_path) {
            eprintln!("Failed to load command history: {err}");
        }
    }
    Ok(editor)
}
