use paymaster_common::service::monitoring::Metric;
use paymaster_common::service::{Error, ServiceManager};
use paymaster_starknet::ChainID;
use tracing::{info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Registry;

use crate::core::context::Context;
use crate::core::Fmt;
use crate::rpc::RPCService;

mod core;
mod rpc;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let context = Context::load()?;

    let metric_layer = context.configuration.prometheus.clone().map(|x| Metric::layer(&x));
    let fmt_layer = Fmt::layer(&context.configuration.verbosity);

    let subscriber = Registry::default().with(fmt_layer).with(metric_layer);

    tracing::subscriber::set_global_default(subscriber).unwrap();

    let chain_id = &context.configuration.starknet.chain_id;
    match chain_id {
        ChainID::Sepolia | ChainID::Mainnet => {
            info!(
                chain_id = %chain_id.as_identifier(),
                chain_id_felt = %chain_id.as_felt().to_hex_string(),
                "configured chain id"
            );
        },
        ChainID::Unknown(felt) => {
            warn!(
                chain_id = %felt.to_hex_string(),
                "configured chain id is NOT natively supported by the paymaster: \
                 falling back to Sepolia defaults (USDC token, AVNU swap/token API, \
                 AVNU exchange address, Coingecko mapping, default RPC URL). \
                 The configured chain id felt is preserved for transaction signing."
            );
        },
    }

    let mut services = ServiceManager::new(context);
    info!("starting services...");
    services.spawn::<RPCService>();

    info!("all services started");
    services.wait()
}
