pub mod avnu;

#[cfg(feature = "testing")]
pub mod mock;
#[cfg(feature = "testing")]
use std::sync::Arc;

use async_trait::async_trait;
use paymaster_common::service::Error as ServiceError;
use paymaster_starknet::ChainID;
use serde::{Deserialize, Serialize};
use starknet::core::types::{Call, Felt};

use crate::swap::client::avnu::{AVNUSwapClient, DEFAULT_MAINNET_AVNU_SWAP_ENDPOINT, DEFAULT_SEPOLIA_AVNU_SWAP_ENDPOINT};
#[cfg(feature = "testing")]
use crate::swap::client::mock::MockSwapClient;

// Trait to be implemented by any swap client
#[async_trait]
pub trait Swap: 'static + Send + Sync + Clone {
    // Swap tokens and return the calls needed to execute the swap, and the minimum amount of token received
    async fn swap(
        &self,
        sell_token: Felt,
        buy_token: Felt,
        sell_amount: Felt,
        taker_address: Felt,
        slippage: f64,
        max_price_impact: f64,
        min_usd_sell_amount: f64,
    ) -> Result<(Vec<Call>, Felt), ServiceError>;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SwapClientConfiguration {
    pub endpoint: String,
    pub chain_id: ChainID,
}

impl SwapClientConfiguration {
    pub fn default_from_chain(chain_id: ChainID) -> Self {
        match chain_id {
            ChainID::Mainnet => Self::default_mainnet(),
            // Unknown chains fall back to the Sepolia AVNU swap config.
            ChainID::Sepolia | ChainID::Unknown(_) => Self::default_sepolia(),
        }
    }

    pub fn default_mainnet() -> Self {
        Self {
            endpoint: DEFAULT_MAINNET_AVNU_SWAP_ENDPOINT.to_string(),
            chain_id: ChainID::Sepolia,
        }
    }

    pub fn default_sepolia() -> Self {
        Self {
            endpoint: DEFAULT_SEPOLIA_AVNU_SWAP_ENDPOINT.to_string(),
            chain_id: ChainID::Sepolia,
        }
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<(), ServiceError> {
        // Validate endpoint
        if self.endpoint.is_empty() {
            return Err(ServiceError::new("AVNU endpoint cannot be empty"));
        }
        // Any chain id is accepted: Mainnet/Sepolia use their respective AVNU
        // deployments, and Unknown chains reuse the Sepolia configuration.
        Ok(())
    }
}

#[derive(Clone)]
pub enum SwapClient {
    #[cfg(feature = "testing")]
    Mock(Arc<dyn mock::MockSwapClient>),

    AVNU(AVNUSwapClient),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum SwapClientConfigurator {
    #[cfg(feature = "testing")]
    #[serde(skip)]
    Mock(Arc<dyn mock::MockSwapClient>),

    #[serde(rename = "avnu")]
    AVNU(SwapClientConfiguration),
}

#[cfg(feature = "testing")]
impl SwapClientConfigurator {
    pub fn mock<T: mock::MockSwapClient>() -> Self {
        Self::Mock(Arc::new(T::new()))
    }
}

impl SwapClientConfigurator {
    pub fn validate(&self) -> Result<(), ServiceError> {
        match self {
            #[cfg(feature = "testing")]
            SwapClientConfigurator::Mock(_) => Ok(()), // Mock doesn't need validation
            SwapClientConfigurator::AVNU(config) => config.validate(),
        }
    }
}

impl SwapClient {
    pub fn new(configuration: &SwapClientConfigurator) -> Self {
        match configuration {
            #[cfg(feature = "testing")]
            SwapClientConfigurator::Mock(x) => Self::Mock(x.clone()),
            SwapClientConfigurator::AVNU(x) => Self::AVNU(AVNUSwapClient::new(x)),
        }
    }

    #[cfg(feature = "testing")]
    pub fn mock<I: 'static + MockSwapClient>() -> Self {
        Self::Mock(Arc::new(I::new()))
    }

    pub async fn swap(
        &self,
        sell_token: Felt,
        buy_token: Felt,
        sell_amount: Felt,
        taker_address: Felt,
        slippage: f64,
        max_price_impact: f64,
        min_usd_sell_amount: f64,
    ) -> Result<(Vec<Call>, Felt), ServiceError> {
        match self {
            #[cfg(feature = "testing")]
            SwapClient::Mock(x) => {
                x.swap(sell_token, buy_token, sell_amount, taker_address, slippage, max_price_impact, min_usd_sell_amount)
                    .await
            },
            SwapClient::AVNU(x) => {
                x.swap(sell_token, buy_token, sell_amount, taker_address, slippage, max_price_impact, min_usd_sell_amount)
                    .await
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_unknown_chain_id() {
        let config = SwapClientConfiguration {
            endpoint: DEFAULT_SEPOLIA_AVNU_SWAP_ENDPOINT.to_string(),
            chain_id: ChainID::Unknown(Felt::from_hex("0x534e5f4b41545241").unwrap()),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn default_from_chain_unknown_falls_back_to_sepolia() {
        let config = SwapClientConfiguration::default_from_chain(ChainID::Unknown(Felt::from_hex("0x534e5f4b41545241").unwrap()));
        assert_eq!(config.endpoint, DEFAULT_SEPOLIA_AVNU_SWAP_ENDPOINT);
    }
}
