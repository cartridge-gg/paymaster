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
mod version;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("paymaster-service {}", version::long());
        return Ok(());
    }

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
                 falling back to Sepolia defaults for chain-derived configuration \
                 (USDC token, AVNU swap/token API, AVNU exchange address, \
                 Coingecko mapping). The configured chain id felt is preserved \
                 unchanged for transaction signing and EIP-712 domain separation."
            );
        },
    }

    let mut services = ServiceManager::new(context);
    info!("starting services...");
    services.spawn::<RPCService>();

    info!("all services started");
    services.wait()
}
