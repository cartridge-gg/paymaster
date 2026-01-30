use paymaster_prices::math::convert_strk_to_token;
use paymaster_starknet::transaction::{
    CalldataBuilder, Calls, EstimatedCalls, ExecuteFromOutsideMessage, SequentialCalldataDecoder, TokenTransfer, TransactionGasEstimate,
};
use paymaster_starknet::Signature;
use starknet::accounts::Account;
use starknet::core::types::{Call, ExecuteInvocation, Felt, FunctionInvocation, InvokeTransactionResult, SimulatedTransaction, TransactionTrace, TypedData};
use starknet::macros::selector;
use std::hash::{DefaultHasher, Hash, Hasher};

use crate::execution::deploy::DeploymentParameters;
use crate::execution::ExecutionParameters;
use crate::{Client, Error};

#[derive(Debug, Hash)]
pub enum ExecutableTransactionParameters {
    Deploy {
        deployment: DeploymentParameters,
    },
    Invoke {
        invoke: ExecutableInvokeParameters,
    },
    DeployAndInvoke {
        deployment: DeploymentParameters,
        invoke: ExecutableInvokeParameters,
    },
    DirectInvoke {
        invoke: ExecutableDirectInvokeParameters,
    },
}

impl ExecutableTransactionParameters {
    pub fn get_unique_identifier(&self) -> u64 {
        match self {
            ExecutableTransactionParameters::Deploy { deployment } => deployment.get_unique_identifier(),
            ExecutableTransactionParameters::Invoke { invoke } => invoke.get_unique_identifier(),
            ExecutableTransactionParameters::DeployAndInvoke { invoke, .. } => invoke.get_unique_identifier(),
            ExecutableTransactionParameters::DirectInvoke { invoke } => invoke.get_unique_indentifier(),
        }
    }
}

#[derive(Debug, Hash)]
pub struct ExecutableInvokeParameters {
    user: Felt,
    signature: Signature,

    message: ExecuteFromOutsideMessage,
}

impl ExecutableInvokeParameters {
    pub fn new(user: Felt, typed_data: TypedData, signature: Signature) -> Result<Self, Error> {
        Ok(Self {
            user,
            signature,

            message: ExecuteFromOutsideMessage::from_typed_data(&typed_data)?,
        })
    }

    fn find_gas_token_transfer(&self, forwarder: Felt) -> Result<TokenTransfer, Error> {
        let last_call = self.message.calls().last().ok_or(Error::InvalidTypedData)?;
        if last_call.selector != selector!("transfer") {
            return Err(Error::InvalidTypedData);
        }

        let transfer_recipient = last_call.calldata.first().ok_or(Error::InvalidTypedData)?;
        if *transfer_recipient != forwarder {
            return Err(Error::InvalidTypedData);
        }

        Ok(TokenTransfer::new(
            last_call.to,
            *transfer_recipient,
            *last_call.calldata.get(1).ok_or(Error::InvalidTypedData)?,
        ))
    }

    pub fn get_unique_identifier(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.user.hash(&mut hasher);
        self.message.nonce().hash(&mut hasher);
        hasher.finish()
    }
}

#[derive(Debug, Hash)]
pub struct ExecutableDirectInvokeParameters {
    pub user: Felt,
    pub execute_from_outside_call: Call,
}

impl ExecutableDirectInvokeParameters {
    pub fn get_unique_indentifier(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.user.hash(&mut hasher);
        self.execute_from_outside_call.calldata.hash(&mut hasher);

        hasher.finish()
    }

    /// Extract gas transfer from a raw execute_from_outside call
    ///
    /// The execute_from_outside_call has calldata structure:
    /// [caller, nonce..., execute_after, execute_before, calls_len, ...calls, sig_len, sig...]
    /// where each call is [to, selector, calldata_len, ...calldata] and the nonce may be one or two felts.
    ///
    /// For non-sponsored transactions, the last call should be a transfer of gas token to the forwarder.
    fn find_gas_token_transfer(&self, forwarder: Felt) -> Result<TokenTransfer, Error> {
        fn extract_calls_segment<'a>(calldata: &'a [Felt], calls_len_index: usize) -> Option<&'a [Felt]> {
            let calls_len_felt = calldata.get(calls_len_index)?;
            let calls_len: usize = (*calls_len_felt).try_into().ok()?;
            if calls_len == 0 {
                return None;
            }

            let mut offset = calls_len_index + 1;
            for _ in 0..calls_len {
                let length_index = offset.checked_add(2)?;
                let length_felt = calldata.get(length_index)?;
                let length: usize = (*length_felt).try_into().ok()?;
                let next_offset = offset.checked_add(3)?.checked_add(length)?;
                if calldata.len() < next_offset {
                    return None;
                }
                offset = next_offset;
            }

            let sig_len_felt = calldata.get(offset)?;
            let sig_len: usize = (*sig_len_felt).try_into().ok()?;
            let expected_end = offset.checked_add(1)?.checked_add(sig_len)?;
            if expected_end != calldata.len() {
                return None;
            }

            calldata.get((calls_len_index + 1)..offset)
        }

        let calldata = &self.execute_from_outside_call.calldata;
        for calls_len_index in [4usize, 5] {
            let Some(calls) = extract_calls_segment(calldata, calls_len_index) else {
                continue;
            };
            let Ok(decoder) = SequentialCalldataDecoder::new(calls) else {
                continue;
            };
            let Some(last_call) = decoder.last() else {
                continue;
            };

            // Validate the last call is a transfer to the forwarder.
            if last_call.selector != selector!("transfer") {
                continue;
            }

            if last_call.calldata.len() != 3 {
                continue;
            }

            let Some(recipient) = last_call.calldata.first() else {
                continue;
            };

            if *recipient != forwarder {
                continue;
            }

            let Some(amount) = last_call.calldata.get(1) else {
                continue;
            };

            return Ok(TokenTransfer::new(last_call.to, forwarder, *amount));
        }

        Err(Error::InvalidTypedData)
    }
}

/// Paymaster transaction that contains the parameters to execute the transaction on Starknet
pub struct ExecutableTransaction {
    /// The forwarder to use when executing the transaction
    pub forwarder: Felt,

    /// Gas fee recipient to use when executing the transaction
    pub gas_tank_address: Felt,

    /// Parameters of the transaction which should come out from the response of the [`buildTransaction`] endpoint
    pub transaction: ExecutableTransactionParameters,

    /// Execution parameters which should come out from the response of the [`buildTransaction`] endpoint
    pub parameters: ExecutionParameters,
}

impl ExecutableTransaction {
    /// Estimate a sponsored transaction which is a transaction that will be paid by the relayer
    pub async fn estimate_sponsored_transaction(self, client: &Client, sponsor_metadata: Vec<Felt>) -> Result<EstimatedExecutableTransaction, Error> {
        let calls = self.build_sponsored_calls(sponsor_metadata);

        let estimated_calls = client.estimate(&calls, self.parameters.tip()).await?;
        let fee_estimate = estimated_calls.estimate();

        // We recompute the real estimate fee. Validation step is not included in the fee estimate
        let paid_fee_in_strk = self.compute_paid_fee(client, Felt::from(fee_estimate.overall_fee)).await?;
        let final_fee_estimate = fee_estimate.update_overall_fee(paid_fee_in_strk);

        let estimated_final_calls = calls.with_estimate(final_fee_estimate);
        Ok(EstimatedExecutableTransaction(estimated_final_calls))
    }

    pub async fn estimate_transaction(self, client: &Client) -> Result<EstimatedExecutableTransaction, Error> {
        match &self.transaction {
            ExecutableTransactionParameters::Invoke { .. }
            | ExecutableTransactionParameters::DeployAndInvoke { .. }
            | ExecutableTransactionParameters::DirectInvoke { .. } => {},
            _ => return Err(Error::InvalidTypedData),
        };

        let gas_token = self.parameters.gas_token();

        let tip = client.get_tip(self.parameters.tip()).await?;
        let estimate_account = client.estimate_account.address();
        let nonce = client.starknet.fetch_nonce(estimate_account).await?;

        let probe_transfer = TokenTransfer::new(gas_token, self.gas_tank_address, Felt::ZERO);
        let probe_calls = self.build_calls(probe_transfer);
        let probe_tx = probe_calls.as_transaction(estimate_account, nonce, tip);

        let simulated = client.starknet.simulate_transaction(&probe_tx).await?;
        let funded_amount = extract_gas_token_transfer_from_simulation(&simulated, gas_token, self.forwarder)?;

        let fee_estimate = TransactionGasEstimate::new(simulated.fee_estimation, tip);

        let paid_fee_in_strk = self.compute_paid_fee(client, Felt::from(fee_estimate.overall_fee)).await?;
        let final_fee_estimate = fee_estimate.update_overall_fee(paid_fee_in_strk);

        let token_price = client.price.fetch_token(gas_token).await?;
        let paid_fee_in_token = convert_strk_to_token(&token_price, paid_fee_in_strk, true)?;

        if paid_fee_in_token > funded_amount {
            return Err(Error::MaxAmountTooLow(paid_fee_in_token.to_hex_string()));
        }

        let fee_transfer = TokenTransfer::new(gas_token, self.gas_tank_address, paid_fee_in_token);
        let final_calls = self.build_calls(fee_transfer);
        let estimated_final_calls = final_calls.with_estimate(final_fee_estimate);

        Ok(EstimatedExecutableTransaction(estimated_final_calls))
    }


    async fn compute_paid_fee(&self, client: &Client, base_estimate: Felt) -> Result<Felt, Error> {
        match &self.transaction {
            ExecutableTransactionParameters::Deploy { .. } => Ok(client.compute_paid_fee_in_strk(base_estimate)),
            ExecutableTransactionParameters::Invoke { invoke, .. } => client.compute_paid_fee_with_overhead_in_strk(invoke.user, base_estimate).await,
            ExecutableTransactionParameters::DeployAndInvoke { invoke, .. } => client.compute_paid_fee_with_overhead_in_strk(invoke.user, base_estimate).await,
            ExecutableTransactionParameters::DirectInvoke { invoke, .. } => client.compute_paid_fee_with_overhead_in_strk(invoke.user, base_estimate).await,
        }
    }

    // Build the calls that needs to be performed
    fn build_calls(&self, fee_transfer: TokenTransfer) -> Calls {
        let calls = [self.build_deploy_call(), self.build_execute_call(fee_transfer)]
            .into_iter()
            .flatten()
            .collect();

        Calls::new(calls)
    }

    // Build the calls that needs to be performed
    fn build_sponsored_calls(&self, sponsor_metadata: Vec<Felt>) -> Calls {
        let calls = [self.build_deploy_call(), self.build_sponsored_execute_call(sponsor_metadata)]
            .into_iter()
            .flatten()
            .collect();

        Calls::new(calls)
    }

    fn build_deploy_call(&self) -> Option<Call> {
        match &self.transaction {
            ExecutableTransactionParameters::Deploy { deployment, .. } => Some(deployment.as_call()),
            ExecutableTransactionParameters::DeployAndInvoke { deployment, .. } => Some(deployment.as_call()),
            _ => None,
        }
    }

    fn build_execute_call(&self, fee_transfer: TokenTransfer) -> Option<Call> {
        let execute_from_outside_call = match &self.transaction {
            ExecutableTransactionParameters::Invoke { invoke, .. } => invoke.message.to_call(invoke.user, &invoke.signature),
            ExecutableTransactionParameters::DeployAndInvoke { invoke, .. } => invoke.message.to_call(invoke.user, &invoke.signature),
            ExecutableTransactionParameters::DirectInvoke { invoke, .. } => invoke.execute_from_outside_call.clone(),
            _ => return None,
        };

        Some(Call {
            to: self.forwarder,
            selector: selector!("execute"),
            calldata: CalldataBuilder::new()
                .encode(&execute_from_outside_call)
                .encode(&fee_transfer.token())
                .encode(&fee_transfer.amount())
                .encode(&Felt::ZERO)
                .build(),
        })
    }

    fn build_sponsored_execute_call(&self, sponsor_metadata: Vec<Felt>) -> Option<Call> {
        let execute_from_outside_call = match &self.transaction {
            ExecutableTransactionParameters::Invoke { invoke, .. } => invoke.message.to_call(invoke.user, &invoke.signature),
            ExecutableTransactionParameters::DeployAndInvoke { invoke, .. } => invoke.message.to_call(invoke.user, &invoke.signature),
            ExecutableTransactionParameters::DirectInvoke { invoke, .. } => invoke.execute_from_outside_call.clone(),
            _ => return None,
        };

        Some(Call {
            to: self.forwarder,
            selector: selector!("execute_sponsored"),
            calldata: CalldataBuilder::new()
                .encode(&execute_from_outside_call)
                .encode(&sponsor_metadata)
                .build(),
        })
    }
}

#[derive(Debug, Clone)]
struct EmittedEvent {
    from_address: Felt,
    keys: Vec<Felt>,
    data: Vec<Felt>,
}

fn extract_gas_token_transfer_from_simulation(simulated: &SimulatedTransaction, gas_token: Felt, forwarder: Felt) -> Result<Felt, Error> {
    let mut events = Vec::new();
    collect_events_from_trace(&simulated.transaction_trace, &mut events)?;

    let mut total = Felt::ZERO;
    let mut found = false;

    for event in events {
        if !is_gas_transfer_event(&event, gas_token, forwarder) {
            continue;
        }

        total += event.data[0];
        found = true;
    }

    if !found {
        return Err(Error::MissingGasFeeTransferEvent);
    }

    Ok(total)
}

fn collect_events_from_trace(trace: &TransactionTrace, out: &mut Vec<EmittedEvent>) -> Result<(), Error> {
    match trace {
        TransactionTrace::Invoke(invoke_trace) => {
            if let Some(invocation) = &invoke_trace.validate_invocation {
                collect_events_from_invocation(invocation, out);
            }

            match &invoke_trace.execute_invocation {
                ExecuteInvocation::Success(invocation) => collect_events_from_invocation(invocation, out),
                ExecuteInvocation::Reverted(reverted) => return Err(Error::Execution(reverted.revert_reason.clone())),
            }

            if let Some(invocation) = &invoke_trace.fee_transfer_invocation {
                collect_events_from_invocation(invocation, out);
            }
        },
        _ => {},
    }

    Ok(())
}

fn collect_events_from_invocation(invocation: &FunctionInvocation, out: &mut Vec<EmittedEvent>) {
    for event in &invocation.events {
        out.push(EmittedEvent {
            from_address: invocation.contract_address,
            keys: event.keys.clone(),
            data: event.data.clone(),
        });
    }

    for call in &invocation.calls {
        collect_events_from_invocation(call, out);
    }
}

fn is_gas_transfer_event(event: &EmittedEvent, gas_token: Felt, forwarder: Felt) -> bool {
    if event.from_address != gas_token {
        return false;
    }

    if event.keys.len() < 3 || event.data.len() < 2 {
        return false;
    }

    if event.keys[0] != selector!("Transfer") {
        return false;
    }

    if event.keys[2] != forwarder {
        return false;
    }

    true
}

/// Paymaster executable transaction that can be sent to Starknet
#[derive(Debug)]
pub struct EstimatedExecutableTransaction(EstimatedCalls);

impl EstimatedExecutableTransaction {
    pub async fn execute(self, client: &Client) -> Result<InvokeTransactionResult, Error> {
        let result = client.execute(&self.0).await?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use crate::execution::build::{InvokeParameters, Transaction, TransactionParameters};
    use crate::execution::deploy::DeploymentParameters;
    use crate::execution::execute::{ExecutableInvokeParameters, ExecutableTransaction, ExecutableTransactionParameters};
    use crate::execution::{ExecutionParameters, FeeMode, TipPriority};
    use crate::testing::transaction::{an_eth_approve, an_eth_transfer};
    use crate::testing::{StarknetTestEnvironment, TestEnvironment};
    use crate::ExecutableDirectInvokeParameters;
    use crate::Error;
    use paymaster_starknet::transaction::{Calls, TokenTransfer};
    use rand::Rng;
    use starknet::accounts::{Account, AccountFactory};
    use starknet::core::types::{
        Call, CallType, EntryPointType, ExecuteInvocation, ExecutionResources, FeeEstimate, Felt, FunctionInvocation, InnerCallExecutionResources,
        InvokeTransactionTrace, OrderedEvent, SimulatedTransaction, TransactionTrace,
    };
    use starknet::macros::{felt, selector};
    use starknet::signers::SigningKey;

    fn dummy_fee_estimate() -> FeeEstimate {
        FeeEstimate {
            l1_gas_consumed: 0,
            l1_gas_price: 0,
            l2_gas_consumed: 0,
            l2_gas_price: 0,
            l1_data_gas_consumed: 0,
            l1_data_gas_price: 0,
            overall_fee: 0,
        }
    }

    fn make_invocation(contract_address: Felt, events: Vec<OrderedEvent>, calls: Vec<FunctionInvocation>) -> FunctionInvocation {
        FunctionInvocation {
            contract_address,
            entry_point_selector: Felt::ZERO,
            calldata: vec![],
            caller_address: Felt::ZERO,
            class_hash: Felt::ZERO,
            entry_point_type: EntryPointType::External,
            call_type: CallType::Call,
            result: vec![],
            calls,
            events,
            messages: vec![],
            execution_resources: InnerCallExecutionResources { l1_gas: 0, l2_gas: 0 },
            is_reverted: false,
        }
    }

    fn make_simulated_transaction(root_invocation: FunctionInvocation) -> SimulatedTransaction {
        SimulatedTransaction {
            transaction_trace: TransactionTrace::Invoke(InvokeTransactionTrace {
                validate_invocation: None,
                execute_invocation: ExecuteInvocation::Success(root_invocation),
                fee_transfer_invocation: None,
                state_diff: None,
                execution_resources: ExecutionResources {
                    l1_gas: 0,
                    l1_data_gas: 0,
                    l2_gas: 0,
                },
            }),
            fee_estimation: dummy_fee_estimate(),
        }
    }

    #[test]
    fn extract_gas_transfer_from_raw_call_works() {
        let forwarder = felt!("0x123");
        let token = felt!("0x456");
        let amount = felt!("0x789");

        // Build a simple execute_from_outside call with one user call + gas transfer
        // Structure: [caller, nonce, execute_after, execute_before, num_calls, call1..., call2..., sig_len, sig...]
        let calldata = vec![
            felt!("0x1"), // caller
            felt!("0x2"), // nonce
            felt!("0x3"), // execute_after
            felt!("0x4"), // execute_before
            Felt::TWO,    // num_calls = 2
            // First call (user's transfer)
            felt!("0xAAA"),        // to
            selector!("transfer"), // selector
            Felt::THREE,           // calldata_len
            felt!("0xBBB"),        // recipient
            felt!("0xCCC"),        // amount_low
            Felt::ZERO,            // amount_high
            // Second call (gas transfer to forwarder)
            token,                 // to (token address)
            selector!("transfer"), // selector
            Felt::THREE,           // calldata_len
            forwarder,             // recipient (forwarder)
            amount,                // amount_low
            Felt::ZERO,            // amount_high
            Felt::TWO,             // signature length
            felt!("0xDEAD"),       // signature part 1
            felt!("0xBEEF"),       // signature part 2
        ];

        let parameters = ExecutableDirectInvokeParameters {
            user: Felt::ZERO,
            execute_from_outside_call: Call {
                to: felt!("0x999"),
                selector: selector!("execute_from_outside"),
                calldata,
            },
        };

        let result = parameters.find_gas_token_transfer(forwarder);
        assert!(result.is_ok());

        let transfer = result.unwrap();
        assert_eq!(transfer.token(), token);
        assert_eq!(transfer.recipient(), forwarder);
        assert_eq!(transfer.amount(), amount);
    }

    #[test]
    fn extract_gas_transfer_from_raw_call_v3_with_signature_works() {
        let forwarder = felt!("0x123");
        let token = felt!("0x456");
        let amount = felt!("0x789");

        // Structure: [caller, nonce_low, nonce_high, execute_after, execute_before, num_calls, call1..., call2..., sig_len, sig...]
        let calldata = vec![
            felt!("0x1"), // caller
            felt!("0x2"), // nonce_low
            felt!("0x3"), // nonce_high
            felt!("0x4"), // execute_after
            felt!("0x5"), // execute_before
            Felt::TWO,    // num_calls = 2
            // First call (user's transfer)
            felt!("0xAAA"),        // to
            selector!("transfer"), // selector
            Felt::THREE,           // calldata_len
            felt!("0xBBB"),        // recipient
            felt!("0xCCC"),        // amount_low
            Felt::ZERO,            // amount_high
            // Second call (gas transfer to forwarder)
            token,                 // to (token address)
            selector!("transfer"), // selector
            Felt::THREE,           // calldata_len
            forwarder,             // recipient (forwarder)
            amount,                // amount_low
            Felt::ZERO,            // amount_high
            Felt::TWO,             // signature length
            felt!("0xDEAD"),       // signature part 1
            felt!("0xBEEF"),       // signature part 2
        ];

        let parameters = ExecutableDirectInvokeParameters {
            user: Felt::ZERO,
            execute_from_outside_call: Call {
                to: felt!("0x999"),
                selector: selector!("execute_from_outside_v3"),
                calldata,
            },
        };

        let result = parameters.find_gas_token_transfer(forwarder);
        assert!(result.is_ok());

        let transfer = result.unwrap();
        assert_eq!(transfer.token(), token);
        assert_eq!(transfer.recipient(), forwarder);
        assert_eq!(transfer.amount(), amount);
    }

    #[test]
    fn extract_gas_transfer_fails_when_last_call_not_transfer() {
        let forwarder = felt!("0x123");

        let calldata = vec![
            felt!("0x1"), // caller
            felt!("0x2"), // nonce
            felt!("0x3"), // execute_after
            felt!("0x4"), // execute_before
            Felt::ONE,    // num_calls = 1
            // Call with wrong selector
            felt!("0x456"),       // to
            selector!("approve"), // wrong selector
            Felt::THREE,          // calldata_len
            forwarder,            // recipient
            felt!("0x789"),       // amount_low
            Felt::ZERO,           // amount_high
            Felt::TWO,            // signature length
            felt!("0xDEAD"),      // signature part 1
            felt!("0xBEEF"),      // signature part 2
        ];

        let parameters = ExecutableDirectInvokeParameters {
            user: Felt::ZERO,
            execute_from_outside_call: Call {
                to: felt!("0x999"),
                selector: selector!("execute_from_outside"),
                calldata,
            },
        };

        let result = parameters.find_gas_token_transfer(forwarder);
        assert!(result.is_err());
    }

    #[test]
    fn extract_gas_transfer_fails_when_recipient_not_forwarder() {
        let forwarder = felt!("0x123");
        let wrong_recipient = felt!("0x456");

        let calldata = vec![
            felt!("0x1"), // caller
            felt!("0x2"), // nonce
            felt!("0x3"), // execute_after
            felt!("0x4"), // execute_before
            Felt::ONE,    // num_calls = 1
            // Transfer to wrong recipient
            felt!("0x789"),        // to
            selector!("transfer"), // selector
            Felt::THREE,           // calldata_len
            wrong_recipient,       // wrong recipient
            felt!("0xAAA"),        // amount_low
            Felt::ZERO,            // amount_high
            Felt::TWO,             // signature length
            felt!("0xDEAD"),       // signature part 1
            felt!("0xBEEF"),       // signature part 2
        ];

        let parameters = ExecutableDirectInvokeParameters {
            user: Felt::ZERO,
            execute_from_outside_call: Call {
                to: felt!("0x999"),
                selector: selector!("execute_from_outside"),
                calldata,
            },
        };

        let result = parameters.find_gas_token_transfer(forwarder);
        assert!(result.is_err());
    }

    #[test]
    fn extract_gas_transfer_fails_when_no_calls() {
        let forwarder = felt!("0x123");

        let calldata = vec![
            felt!("0x1"), // caller
            felt!("0x2"), // nonce
            felt!("0x3"), // execute_after
            felt!("0x4"), // execute_before
            Felt::ZERO,   // num_calls = 0
        ];

        let parameters = ExecutableDirectInvokeParameters {
            user: Felt::ZERO,
            execute_from_outside_call: Call {
                to: felt!("0x999"),
                selector: selector!("execute_from_outside"),
                calldata,
            },
        };

        let result = parameters.find_gas_token_transfer(forwarder);
        assert!(result.is_err());
    }

    #[test]
    fn extract_gas_transfer_fails_when_insufficient_calldata() {
        let forwarder = felt!("0x123");

        // Not enough data
        let calldata = vec![
            felt!("0x1"), // caller
            felt!("0x2"), // nonce
            felt!("0x3"), // execute_after
        ];

        let parameters = ExecutableDirectInvokeParameters {
            user: Felt::ZERO,
            execute_from_outside_call: Call {
                to: felt!("0x999"),
                selector: selector!("execute_from_outside"),
                calldata,
            },
        };

        let result = parameters.find_gas_token_transfer(forwarder);
        assert!(result.is_err());
    }

    #[test]
    fn transfer_event_sum_matches() {
        let forwarder = felt!("0x123");
        let gas_token = felt!("0x456");
        let other_token = felt!("0x789");

        let transfer_event_1 = OrderedEvent {
            order: 0,
            keys: vec![selector!("Transfer"), felt!("0x1"), forwarder],
            data: vec![Felt::from(10u8), Felt::ZERO],
        };

        let transfer_event_2 = OrderedEvent {
            order: 1,
            keys: vec![selector!("Transfer"), felt!("0x2"), forwarder],
            data: vec![Felt::from(7u8), Felt::ZERO],
        };

        let ignored_event = OrderedEvent {
            order: 2,
            keys: vec![selector!("Transfer"), felt!("0x3"), forwarder],
            data: vec![Felt::from(4u8), Felt::ZERO],
        };

        let token_invocation = make_invocation(gas_token, vec![transfer_event_1, transfer_event_2], vec![]);
        let other_invocation = make_invocation(other_token, vec![ignored_event], vec![]);
        let root_invocation = make_invocation(Felt::ZERO, vec![], vec![token_invocation, other_invocation]);

        let simulated = make_simulated_transaction(root_invocation);
        let result = extract_gas_token_transfer_from_simulation(&simulated, gas_token, forwarder).unwrap();

        assert_eq!(result, Felt::from(17u8));
    }

    #[test]
    fn transfer_event_ignores_non_matching() {
        let forwarder = felt!("0x123");
        let gas_token = felt!("0x456");

        let wrong_recipient_event = OrderedEvent {
            order: 0,
            keys: vec![selector!("Transfer"), felt!("0x1"), felt!("0x999")],
            data: vec![Felt::from(5u8), Felt::ZERO],
        };

        let invocation = make_invocation(gas_token, vec![wrong_recipient_event], vec![]);
        let root_invocation = make_invocation(Felt::ZERO, vec![], vec![invocation]);

        let simulated = make_simulated_transaction(root_invocation);
        let result = extract_gas_token_transfer_from_simulation(&simulated, gas_token, forwarder);

        assert!(matches!(result, Err(Error::MissingGasFeeTransferEvent)));
    }

    #[test]
    fn transfer_event_missing_errors() {
        let forwarder = felt!("0x123");
        let gas_token = felt!("0x456");

        let non_transfer_event = OrderedEvent {
            order: 0,
            keys: vec![selector!("Approval"), felt!("0x1"), forwarder],
            data: vec![Felt::from(5u8), Felt::ZERO],
        };

        let invocation = make_invocation(gas_token, vec![non_transfer_event], vec![]);
        let root_invocation = make_invocation(Felt::ZERO, vec![], vec![invocation]);

        let simulated = make_simulated_transaction(root_invocation);
        let result = extract_gas_token_transfer_from_simulation(&simulated, gas_token, forwarder);

        assert!(matches!(result, Err(Error::MissingGasFeeTransferEvent)));
    }

    // TODO: enable when we can fix starknet image
    #[ignore]
    #[tokio::test]
    async fn execute_deploy_transaction_sponsored_works_properly() {
        let test = TestEnvironment::new().await;
        let account = test.starknet.initialize_account(&StarknetTestEnvironment::ACCOUNT_1);

        let new_account = test.starknet.initialize_argent_account(Felt::ONE).await;
        let salt = Felt::from(rand::rng().random_range(1..1_000_000_000));
        let new_account_address = new_account.deploy_v3(salt).address();

        test.starknet
            .transfer_token(
                &account,
                &TokenTransfer::new(StarknetTestEnvironment::ETH, new_account_address, Felt::from(1e16 as u128)),
            )
            .await;

        let deployment = DeploymentParameters {
            version: 2,
            address: new_account_address,
            class_hash: new_account.class_hash(),
            unique: Felt::ZERO,
            salt,
            calldata: new_account.calldata(),
            sigdata: None,
        };

        let client = test.default_client();

        let transaction = ExecutableTransaction {
            forwarder: StarknetTestEnvironment::FORWARDER,
            gas_tank_address: StarknetTestEnvironment::FORWARDER,

            transaction: ExecutableTransactionParameters::Deploy { deployment },
            parameters: ExecutionParameters::V1 {
                fee_mode: FeeMode::Sponsored { tip: TipPriority::Normal },
                time_bounds: None,
            },
        };

        let estimate = transaction.estimate_sponsored_transaction(&client, vec![]).await.unwrap();
        let result = estimate.execute(&client).await;
        assert!(result.is_ok())
    }

    // TODO: enable when we can fix starknet image
    #[ignore]
    #[tokio::test]
    async fn execute_invoke_transaction_works_properly() {
        let test = TestEnvironment::new().await;
        let account = test.starknet.initialize_account(&StarknetTestEnvironment::ACCOUNT_1);

        let user = StarknetTestEnvironment::ACCOUNT_ARGENT_1;

        let transaction = Transaction {
            forwarder: StarknetTestEnvironment::FORWARDER,

            transaction: TransactionParameters::Invoke {
                invoke: InvokeParameters {
                    user_address: user.address,
                    calls: Calls::new(vec![an_eth_transfer(account.address(), Felt::ONE)]),
                },
            },
            parameters: ExecutionParameters::V1 {
                fee_mode: FeeMode::Default {
                    gas_token: StarknetTestEnvironment::ETH,
                    tip: TipPriority::Normal,
                },
                time_bounds: None,
            },
        };

        let client = test.default_client();

        let estimated_transaction = transaction.estimate(&client).await.unwrap();
        let versioned_estimated_transaction = estimated_transaction.resolve_version(&client).await.unwrap();

        let typed_data = versioned_estimated_transaction
            .to_execute_from_outside()
            .to_typed_data()
            .unwrap();
        let message_hash = typed_data.message_hash(user.address).unwrap();
        let signed_message = SigningKey::from_secret_scalar(user.private_key).sign(&message_hash).unwrap();

        let transaction = ExecutableTransaction {
            forwarder: StarknetTestEnvironment::FORWARDER,
            gas_tank_address: StarknetTestEnvironment::FORWARDER,

            transaction: ExecutableTransactionParameters::Invoke {
                invoke: ExecutableInvokeParameters::new(user.address, typed_data, vec![signed_message.r, signed_message.s]).unwrap(),
            },
            parameters: ExecutionParameters::V1 {
                fee_mode: FeeMode::Default {
                    gas_token: StarknetTestEnvironment::ETH,
                    tip: TipPriority::Normal,
                },
                time_bounds: None,
            },
        };

        let estimate = transaction.estimate_transaction(&client).await.unwrap();
        let result = estimate.execute(&client).await;
        assert!(result.is_ok())
    }

    // TODO: enable when we can fix starknet image
    #[ignore]
    #[tokio::test]
    async fn execute_deploy_and_invoke_transaction_works_properly() {
        let test = TestEnvironment::new().await;
        let account = test.starknet.initialize_account(&StarknetTestEnvironment::ACCOUNT_1);

        let new_account = test.starknet.initialize_argent_account(Felt::ONE).await;
        let salt = Felt::from(rand::rng().random_range(1..1_000_000_000));
        let new_account_address = new_account.deploy_v3(salt).address();

        test.starknet
            .transfer_token(
                &account,
                &TokenTransfer::new(StarknetTestEnvironment::ETH, new_account_address, Felt::from(1e16 as u128)),
            )
            .await;

        let deployment = DeploymentParameters {
            version: 2,
            address: new_account_address,
            class_hash: new_account.class_hash(),
            unique: Felt::ZERO,
            salt,
            calldata: new_account.calldata(),
            sigdata: None,
        };

        let transaction = Transaction {
            forwarder: StarknetTestEnvironment::FORWARDER,

            transaction: TransactionParameters::DeployAndInvoke {
                deployment: deployment.clone(),
                invoke: InvokeParameters {
                    user_address: new_account_address,
                    calls: Calls::new(vec![an_eth_approve(account.address(), Felt::ZERO)]),
                },
            },
            parameters: ExecutionParameters::V1 {
                fee_mode: FeeMode::Default {
                    gas_token: StarknetTestEnvironment::ETH,
                    tip: TipPriority::Normal,
                },
                time_bounds: None,
            },
        };

        let client = test.default_client();

        let estimated_transaction = transaction.estimate(&client).await.unwrap();
        let versioned_estimated_transaction = estimated_transaction.resolve_version(&client).await.unwrap();

        let typed_data = versioned_estimated_transaction
            .to_execute_from_outside()
            .to_typed_data()
            .unwrap();
        let message_hash = typed_data.message_hash(new_account_address).unwrap();
        let signed_message = SigningKey::from_secret_scalar(Felt::ONE).sign(&message_hash).unwrap();

        let transaction = ExecutableTransaction {
            forwarder: StarknetTestEnvironment::FORWARDER,
            gas_tank_address: StarknetTestEnvironment::FORWARDER,

            transaction: ExecutableTransactionParameters::DeployAndInvoke {
                deployment,
                invoke: ExecutableInvokeParameters::new(new_account_address, typed_data, vec![signed_message.r, signed_message.s]).unwrap(),
            },
            parameters: ExecutionParameters::V1 {
                fee_mode: FeeMode::Default {
                    gas_token: StarknetTestEnvironment::ETH,
                    tip: TipPriority::Normal,
                },
                time_bounds: None,
            },
        };

        let estimate = transaction.estimate_transaction(&client).await.unwrap();
        let result = estimate.execute(&client).await;
        assert!(result.is_ok())
    }
}
