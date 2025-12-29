use paymaster_execution::ExecutableTransaction;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use starknet::core::serde::unsigned_field_element::UfeHex;
use starknet::core::types::{Call, Felt};

use crate::endpoint::common::ExecutionParameters;
use crate::endpoint::validation::check_service_is_available;
use crate::endpoint::RequestContext;
use crate::Error;

#[derive(Serialize, Deserialize)]
pub struct ExecuteRawRequest {
    pub transaction: ExecuteRawTransactionParameters,
    pub parameters: ExecutionParameters,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecuteRawTransactionParameters {
    RawInvoke { invoke: RawInvokeParameters },
}

impl TryFrom<ExecuteRawTransactionParameters> for paymaster_execution::ExecutableTransactionParameters {
    type Error = Error;

    fn try_from(value: ExecuteRawTransactionParameters) -> Result<Self, Self::Error> {
        Ok(match value {
            ExecuteRawTransactionParameters::RawInvoke { invoke } => Self::RawInvoke {
                user: invoke.user_address,
                execute_from_outside_call: invoke.execute_from_outside_call,
                gas_token: invoke.gas_token,
                max_gas_token_amount: invoke.max_gas_token_amount,
            },
        })
    }
}

#[serde_as]
#[derive(Serialize, Deserialize)]
pub struct RawInvokeParameters {
    #[serde_as(as = "UfeHex")]
    pub user_address: Felt,

    pub execute_from_outside_call: Call,

    #[serde_as(as = "Option<UfeHex>")]
    #[serde(default)]
    pub gas_token: Option<Felt>,

    #[serde_as(as = "Option<UfeHex>")]
    #[serde(default)]
    pub max_gas_token_amount: Option<Felt>,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExecuteRawResponse {
    #[serde_as(as = "UfeHex")]
    pub transaction_hash: Felt,

    #[serde_as(as = "UfeHex")]
    pub tracking_id: Felt,
}

pub async fn execute_raw_endpoint(ctx: &RequestContext<'_>, request: ExecuteRawRequest) -> Result<ExecuteRawResponse, Error> {
    check_service_is_available(ctx).await?;

    let forwarder = ctx.configuration.forwarder;
    let gas_tank_address = ctx.configuration.gas_tank.address;

    let transaction = ExecutableTransaction {
        forwarder,
        gas_tank_address,
        parameters: request.parameters.into(),
        transaction: request.transaction.try_into()?,
    };

    let estimated_transaction = if transaction.parameters.fee_mode().is_sponsored() {
        let authenticated_api_key = ctx.validate_api_key().await?;
        transaction
            .estimate_sponsored_transaction(&ctx.execution, authenticated_api_key.sponsor_metadata)
            .await?
    } else {
        transaction.estimate_transaction(&ctx.execution).await?
    };

    let result = estimated_transaction.execute(&ctx.execution).await?;

    Ok(ExecuteRawResponse {
        transaction_hash: result.transaction_hash,
        tracking_id: Felt::ZERO,
    })
}
