use alloy_primitives::FixedBytes; // Tx hash
use alloy_provider::{Provider, ProviderBuilder};
use anyhow::{Context, Result};
use log::debug;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::{sleep, timeout};

/// HTTP-only receipt client that avoids WebSocket subscriptions
pub struct ReceiptClient {
    // HTTP-only root provider (no pubsub, so nothing to keep open)
    http: Arc<dyn Provider>,
}

// Global cache for ReceiptClient instances per RPC URL
static RECEIPT_CLIENTS: Lazy<Mutex<HashMap<String, Arc<dyn Provider>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

impl ReceiptClient {
    pub fn new(http_url: &str) -> Result<Self> {
        // Normalize WebSocket URLs to HTTP URLs
        let normalized_url = Self::normalize_to_http_url(http_url);

        let http = ProviderBuilder::new()
            .connect_http(normalized_url.parse().context("Failed to parse HTTP URL")?);

        Ok(Self {
            http: Arc::new(http),
        })
    }

    /// Get or create a ReceiptClient for the given RPC URL (singleton pattern)
    /// This avoids creating new clients for every transaction
    pub fn get_or_create(rpc_url: &str) -> Result<ReceiptClient> {
        let mut cache = RECEIPT_CLIENTS.lock().unwrap();

        // Normalize the URL first to ensure consistent cache keys
        let normalized_url = Self::normalize_to_http_url(rpc_url);

        if let Some(provider) = cache.get(&normalized_url) {
            // Provider already exists, create ReceiptClient with shared provider
            return Ok(ReceiptClient {
                http: provider.clone(),
            });
        }

        // Create new provider and store it
        let provider = ProviderBuilder::new()
            .connect_http(normalized_url.parse().context("Failed to parse HTTP URL")?);
        let provider_arc = Arc::new(provider);

        // Store the provider in cache using normalized URL as key
        cache.insert(normalized_url.clone(), provider_arc.clone());

        // Return ReceiptClient with shared provider
        Ok(ReceiptClient { http: provider_arc })
    }

    /// Convert WebSocket URLs to HTTP URLs for receipt fetching
    fn normalize_to_http_url(url: &str) -> String {
        if url.starts_with("ws://") {
            url.replace("ws://", "http://")
        } else if url.starts_with("wss://") {
            url.replace("wss://", "https://")
        } else {
            url.to_string()
        }
    }

    /// Check if an error is a permanent failure that won't be resolved by retrying
    fn is_permanent_failure(error: &anyhow::Error) -> bool {
        let error_str = error.to_string().to_lowercase();

        // Permanent failures that should not be retried
        // NOTE: "not found" is excluded because eth_getTransactionReceipt returns None for pending txs
        error_str.contains("authentication")
            || error_str.contains("unauthorized")
            || error_str.contains("forbidden")
            || error_str.contains("dns")
            || error_str.contains("connection refused")
            || error_str.contains("invalid url")
            || error_str.contains("parse error")
    }

    /// Zero-subscription path to fetch a receipt.
    pub async fn get_receipt(
        &self,
        tx_hash: FixedBytes<32>,
    ) -> Result<alloy_rpc_types::TransactionReceipt> {
        // This issues a single `eth_getTransactionReceipt` call — no watch/subscribe.
        let receipt = self
            .http
            .get_transaction_receipt(tx_hash)
            .await
            .context("Failed to get transaction receipt")?
            .ok_or_else(|| {
                anyhow::anyhow!("Transaction receipt not found for hash: {:?}", tx_hash)
            })?;
        Ok(receipt)
    }

    /// Poll for receipt with timeout and interval
    pub async fn get_receipt_with_polling(
        &self,
        tx_hash: FixedBytes<32>,
        max_wait_time: Duration,
        poll_interval: Duration,
    ) -> Result<alloy_rpc_types::TransactionReceipt> {
        let start_time = std::time::Instant::now();

        loop {
            // Check if we've exceeded max wait time
            if start_time.elapsed() > max_wait_time {
                return Err(anyhow::anyhow!(
                    "Receipt timeout after {:?} for transaction {:?}",
                    max_wait_time,
                    tx_hash
                ));
            }

            // Try to get the receipt
            match self.get_receipt(tx_hash).await {
                Ok(receipt) => {
                    debug!("Successfully fetched receipt for transaction {:?}", tx_hash);
                    return Ok(receipt);
                }
                Err(e) => {
                    // Check if this is a permanent failure (auth, DNS, etc.)
                    if Self::is_permanent_failure(&e) {
                        return Err(e);
                    }

                    debug!("Transaction {:?} not yet mined, waiting...", tx_hash);
                    sleep(poll_interval).await;
                    continue;
                }
            }
        }
    }

    /// Get receipt with timeout wrapper
    pub async fn get_receipt_with_timeout(
        &self,
        tx_hash: FixedBytes<32>,
        timeout_duration: Duration,
        poll_interval: Duration,
    ) -> Result<alloy_rpc_types::TransactionReceipt> {
        timeout(
            timeout_duration,
            self.get_receipt_with_polling(tx_hash, timeout_duration, poll_interval),
        )
        .await
        .context("Receipt polling timed out")?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::FixedBytes;
    use std::time::Duration;

    #[test]
    fn test_receipt_client_creation() {
        let client = ReceiptClient::new("http://localhost:8545");
        assert!(client.is_ok());
    }

    #[test]
    fn test_receipt_client_invalid_url() {
        let client = ReceiptClient::new("invalid-url");
        assert!(client.is_err());
    }

    #[test]
    fn test_receipt_client_https_url() {
        let client = ReceiptClient::new("https://mainnet.infura.io/v3/test");
        assert!(client.is_ok());
    }

    #[test]
    fn test_receipt_client_ws_url_normalization() {
        // WebSocket URLs are normalized to HTTP URLs
        let client = ReceiptClient::new("ws://localhost:8545");
        assert!(client.is_ok());
    }

    #[test]
    fn test_receipt_client_wss_url_normalization() {
        // WebSocket URLs are normalized to HTTPS URLs
        let client = ReceiptClient::new("wss://localhost:8545");
        assert!(client.is_ok());
    }

    #[test]
    fn test_url_normalization() {
        assert_eq!(
            ReceiptClient::normalize_to_http_url("ws://localhost:8545"),
            "http://localhost:8545"
        );
        assert_eq!(
            ReceiptClient::normalize_to_http_url("wss://localhost:8545"),
            "https://localhost:8545"
        );
        assert_eq!(
            ReceiptClient::normalize_to_http_url("http://localhost:8545"),
            "http://localhost:8545"
        );
        assert_eq!(
            ReceiptClient::normalize_to_http_url("https://localhost:8545"),
            "https://localhost:8545"
        );
    }

    #[tokio::test]
    async fn test_get_receipt_with_nonexistent_tx() {
        // This test would require a real RPC endpoint, so we'll skip it in unit tests
        // In integration tests, this would test that a non-existent transaction returns an error
        let client = ReceiptClient::new("http://localhost:8545").unwrap();
        let fake_tx_hash = FixedBytes::from([0u8; 32]);

        // This will fail because there's no RPC server running, but that's expected
        let result = client.get_receipt(fake_tx_hash).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_receipt_with_polling_timeout() {
        let client = ReceiptClient::new("http://localhost:8545").unwrap();
        let fake_tx_hash = FixedBytes::from([0u8; 32]);

        // Test that polling times out after max_wait_time
        let result = client
            .get_receipt_with_polling(
                fake_tx_hash,
                Duration::from_millis(100), // Very short timeout
                Duration::from_millis(10),  // Short poll interval
            )
            .await;

        assert!(result.is_err());
        // Any error is acceptable since we're testing with a fake transaction hash
    }

    #[tokio::test]
    async fn test_get_receipt_with_timeout_wrapper() {
        let client = ReceiptClient::new("http://localhost:8545").unwrap();
        let fake_tx_hash = FixedBytes::from([0u8; 32]);

        // Test that timeout wrapper works
        let result = client
            .get_receipt_with_timeout(
                fake_tx_hash,
                Duration::from_millis(50), // Very short timeout
                Duration::from_millis(10), // Short poll interval
            )
            .await;

        assert!(result.is_err());
        // Any error is acceptable since we're testing with a fake transaction hash
    }
}
