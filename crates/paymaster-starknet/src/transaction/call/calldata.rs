use crate::Error;
use starknet::core::types::{Call, Felt};
use std::collections::LinkedList;
use std::ops::Deref;

pub trait AsCalldata {
    fn encode(&self) -> Vec<Felt>;
}

impl AsCalldata for Felt {
    fn encode(&self) -> Vec<Felt> {
        vec![*self]
    }
}

impl AsCalldata for Call {
    fn encode(&self) -> Vec<Felt> {
        CalldataBuilder::new()
            .encode(&self.to)
            .encode(&self.selector)
            .encode(&self.calldata)
            .build()
    }
}

impl<T: AsCalldata> AsCalldata for Vec<T> {
    fn encode(&self) -> Vec<Felt> {
        let mut calldata = vec![];
        calldata.push(Felt::from(self.len()));
        calldata.extend(self.iter().flat_map(|x| x.encode()));

        calldata
    }
}

impl<T: AsCalldata> AsCalldata for &[T] {
    fn encode(&self) -> Vec<Felt> {
        let mut calldata = vec![];
        calldata.push(Felt::from(self.len()));
        calldata.extend(self.iter().flat_map(|x| x.encode()));

        calldata
    }
}

#[derive(Default)]
pub struct CalldataBuilder {
    calldata: Vec<Felt>,
}

impl CalldataBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn encode<T: AsCalldata>(mut self, value: &T) -> Self {
        self.calldata.extend(value.encode());
        self
    }

    pub fn build(self) -> Vec<Felt> {
        self.calldata
    }
}

/// Represents a decoded Starknet call with its components.
///
/// A call consists of:
/// - `to`: The target contract address
/// - `selector`: The function selector to invoke
/// - `calldata`: The arguments to pass to the function
#[derive(Debug, Clone)]
pub struct DecodedCall {
    pub to: Felt,
    pub selector: Felt,
    pub calldata: Vec<Felt>,
}

/// Decoder for sequentially encoded Starknet calldata.
///
/// This decoder parses calldata that contains multiple calls encoded sequentially,
/// where each call follows the format:
/// ```text
/// [to, selector, calldata_length, calldata_arg_1, calldata_arg_2, ..., calldata_arg_n]
/// ```
///
/// Multiple calls are simply concatenated one after another:
/// ```text
/// [call_1_to, call_1_selector, call_1_length, call_1_args..., call_2_to, call_2_selector, call_2_length, call_2_args..., ...]
/// ```
///
/// # Examples
///
/// ```rust,ignore
/// use starknet::core::types::Felt;
/// use paymaster_starknet::transaction::call::SequentialCalldataDecoder;
///
/// // Decode a single call with no arguments
/// let calldata = vec![
///     Felt::from(0x123u64),  // to
///     Felt::from(0x456u64),  // selector
///     Felt::ZERO,            // calldata_length = 0
/// ];
/// let decoder = SequentialCalldataDecoder::new(&calldata).unwrap();
/// assert_eq!(decoder.len(), 1);
/// assert_eq!(decoder[0].to, Felt::from(0x123u64));
///
/// // Decode multiple calls
/// let calldata = vec![
///     Felt::from(0x100u64), Felt::from(0x200u64), Felt::from(2u64),
///     Felt::from(0xAAu64), Felt::from(0xBBu64),  // first call with 2 args
///     Felt::from(0x300u64), Felt::from(0x400u64), Felt::ZERO,  // second call with 0 args
/// ];
/// let decoder = SequentialCalldataDecoder::new(&calldata).unwrap();
/// assert_eq!(decoder.len(), 2);
/// ```
///
/// # Errors
///
/// Returns `Error::CalldataDecoding` if:
/// - Required fields (to, selector, length) are missing
/// - The calldata length doesn't match the declared length
/// - The calldata is malformed or truncated
pub struct SequentialCalldataDecoder(Vec<DecodedCall>);

impl Deref for SequentialCalldataDecoder {
    type Target = Vec<DecodedCall>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl SequentialCalldataDecoder {
    /// Creates a new decoder from raw calldata.
    ///
    /// Parses the provided calldata slice and extracts all sequentially encoded calls.
    /// Each call must follow the format: `[to, selector, length, args...]` where `length`
    /// specifies the number of calldata arguments that follow.
    ///
    /// # Arguments
    ///
    /// * `calldata` - A slice of `Felt` values representing the encoded calls
    ///
    /// # Returns
    ///
    /// Returns a `SequentialCalldataDecoder` containing all decoded calls on success.
    ///
    /// # Errors
    ///
    /// Returns `Error::CalldataDecoding` if:
    /// - A call is missing required fields (to, selector, or length)
    /// - The declared calldata length doesn't match available data
    /// - The data cannot be parsed correctly
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use starknet::core::types::Felt;
    /// use paymaster_starknet::transaction::call::SequentialCalldataDecoder;
    ///
    /// // Empty calldata is valid and returns no calls
    /// let decoder = SequentialCalldataDecoder::new(&[]).unwrap();
    /// assert_eq!(decoder.len(), 0);
    ///
    /// // Single call with arguments
    /// let calldata = vec![
    ///     Felt::from(0x1234u64),  // to address
    ///     Felt::from(0x5678u64),  // selector
    ///     Felt::from(2u64),       // 2 arguments
    ///     Felt::from(100u64),     // arg 1
    ///     Felt::from(200u64),     // arg 2
    /// ];
    /// let decoder = SequentialCalldataDecoder::new(&calldata).unwrap();
    /// assert_eq!(decoder[0].calldata.len(), 2);
    /// ```
    pub fn new(calldata: &[Felt]) -> Result<Self, Error> {
        fn parse_next_value<T: TryFrom<Felt>>(call_stack: &mut LinkedList<Felt>, identifier: &str) -> Result<T, Error> {
            let value = call_stack
                .pop_front()
                .ok_or(Error::CalldataDecoding(format!("{identifier} missing")))?;

            value
                .try_into()
                .map_err(|_| Error::CalldataDecoding(format!("{identifier} missing")))
        }

        fn parse_call(call_stack: &mut LinkedList<Felt>) -> Result<DecodedCall, Error> {
            let to = parse_next_value(call_stack, "to")?;
            let selector = parse_next_value(call_stack, "selector")?;
            let length: usize = parse_next_value(call_stack, "length")?;
            let mut calldata = Vec::with_capacity(length);
            for _ in 0..length {
                calldata.push(parse_next_value(call_stack, "calldata")?);
            }

            Ok(DecodedCall { to, selector, calldata })
        }

        let mut call_stack: LinkedList<_> = calldata.iter().cloned().collect();

        let mut calls = vec![];
        while !call_stack.is_empty() {
            calls.push(parse_call(&mut call_stack)?)
        }

        Ok(Self(calls))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_empty_calldata() {
        let calldata = vec![];

        let decoder = SequentialCalldataDecoder::new(&calldata).unwrap();
        assert_eq!(decoder.len(), 0);
    }

    #[test]
    fn decode_single_call_with_no_calldata() {
        let to = Felt::from(123u64);
        let selector = Felt::from(456u64);
        let calldata_length = Felt::ZERO;

        let calldata = vec![to, selector, calldata_length];

        let decoder = SequentialCalldataDecoder::new(&calldata).unwrap();
        assert_eq!(decoder.len(), 1);
        assert_eq!(decoder[0].to, to);
        assert_eq!(decoder[0].selector, selector);
        assert_eq!(decoder[0].calldata.len(), 0);
    }

    #[test]
    fn decode_single_call_with_calldata() {
        let to = Felt::from(123u64);
        let selector = Felt::from(456u64);
        let calldata_length = Felt::from(3u64);
        let arg1 = Felt::from(111u64);
        let arg2 = Felt::from(222u64);
        let arg3 = Felt::from(333u64);

        let calldata = vec![to, selector, calldata_length, arg1, arg2, arg3];

        let decoder = SequentialCalldataDecoder::new(&calldata).unwrap();
        assert_eq!(decoder.len(), 1);
        assert_eq!(decoder[0].to, to);
        assert_eq!(decoder[0].selector, selector);
        assert_eq!(decoder[0].calldata, vec![arg1, arg2, arg3]);
    }

    #[test]
    fn decode_multiple_calls() {
        let to1 = Felt::from(100u64);
        let selector1 = Felt::from(200u64);
        let calldata_length1 = Felt::from(2u64);
        let arg1_1 = Felt::from(11u64);
        let arg1_2 = Felt::from(22u64);

        let to2 = Felt::from(300u64);
        let selector2 = Felt::from(400u64);
        let calldata_length2 = Felt::ZERO;

        let to3 = Felt::from(500u64);
        let selector3 = Felt::from(600u64);
        let calldata_length3 = Felt::from(1u64);
        let arg3_1 = Felt::from(777u64);

        let calldata = vec![
            to1,
            selector1,
            calldata_length1,
            arg1_1,
            arg1_2,
            to2,
            selector2,
            calldata_length2,
            to3,
            selector3,
            calldata_length3,
            arg3_1,
        ];

        let decoder = SequentialCalldataDecoder::new(&calldata).unwrap();
        assert_eq!(decoder.len(), 3);

        assert_eq!(decoder[0].to, to1);
        assert_eq!(decoder[0].selector, selector1);
        assert_eq!(decoder[0].calldata, vec![arg1_1, arg1_2]);

        assert_eq!(decoder[1].to, to2);
        assert_eq!(decoder[1].selector, selector2);
        assert_eq!(decoder[1].calldata.len(), 0);

        assert_eq!(decoder[2].to, to3);
        assert_eq!(decoder[2].selector, selector3);
        assert_eq!(decoder[2].calldata, vec![arg3_1]);
    }

    #[test]
    fn error_when_selector_is_missing() {
        let to = Felt::from(123u64);
        let calldata = vec![to];

        match SequentialCalldataDecoder::new(&calldata) {
            Err(Error::CalldataDecoding(msg)) => assert!(msg.contains("selector missing")),
            _ => panic!("Expected CalldataDecoding error"),
        }
    }

    #[test]
    fn error_when_length_is_missing() {
        let to = Felt::from(123u64);
        let selector = Felt::from(456u64);
        let calldata = vec![to, selector];

        match SequentialCalldataDecoder::new(&calldata) {
            Err(Error::CalldataDecoding(msg)) => assert!(msg.contains("length missing")),
            _ => panic!("Expected CalldataDecoding error"),
        }
    }

    #[test]
    fn error_when_calldata_elements_are_missing() {
        let to = Felt::from(123u64);
        let selector = Felt::from(456u64);
        let calldata_length = Felt::from(3u64);
        let arg1 = Felt::from(111u64);

        // Only provide 1 argument but length says 3
        let calldata = vec![to, selector, calldata_length, arg1];
        match SequentialCalldataDecoder::new(&calldata) {
            Err(Error::CalldataDecoding(msg)) => assert!(msg.contains("calldata missing")),
            _ => panic!("Expected CalldataDecoding error"),
        }
    }

    #[test]
    fn error_in_middle_of_multiple_calls() {
        let to1 = Felt::from(100u64);
        let selector1 = Felt::from(200u64);
        let calldata_length1 = Felt::from(1u64);
        let arg1 = Felt::from(11u64);

        let to2 = Felt::from(300u64);
        let selector2 = Felt::from(400u64);
        let calldata_length2 = Felt::from(2u64);
        // Missing the 2 calldata arguments for the second call

        let calldata = vec![to1, selector1, calldata_length1, arg1, to2, selector2, calldata_length2];

        match SequentialCalldataDecoder::new(&calldata) {
            Err(Error::CalldataDecoding(msg)) => assert!(msg.contains("calldata missing")),
            _ => panic!("Expected CalldataDecoding error"),
        }
    }
}
