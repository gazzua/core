use crate::{DynSolValue, Error as CrateError, ResolveSolType, Result};
use alloc::vec::Vec;
use alloy_json_abi::{Constructor, Error, Function, Param};
use alloy_primitives::Selector;
use alloy_sol_types::Decoder;

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::Constructor {}
    impl Sealed for super::Error {}
    impl Sealed for super::Function {}
}
use sealed::Sealed;

/// Provides ABI encoding and decoding functionality.
///
/// This trait is sealed and cannot be implemented for types outside of this
/// crate. It is implemented only for the following types:
///
/// - [`Constructor`]
/// - [`Error`]
/// - [`Function`]
pub trait JsonAbiExt: Sealed {
    /// ABI-encodes the given values, prefixed by this item's selector, if any.
    ///
    /// The selector is:
    /// - `None` for [`Constructor`],
    /// - `Some(self.selector())` for [`Error`] and [`Function`].
    ///
    /// This behaviour is to ensure consistency with `ethabi`.
    ///
    /// To encode the data without the selector, use
    /// [`encode_input_raw`](JsonAbiExt::encode_input_raw).
    ///
    /// # Errors
    ///
    /// This function will return an error if the given values do not match the
    /// expected input types.
    fn encode_input(&self, values: &[DynSolValue]) -> Result<Vec<u8>>;

    /// ABI-encodes the given values, without prefixing the data with the item's
    /// selector.
    ///
    /// For [`Constructor`], this is the same as
    /// [`encode_input`](JsonAbiExt::encode_input).
    ///
    /// # Errors
    ///
    /// This function will return an error if the given values do not match the
    /// expected input types.
    fn encode_input_raw(&self, values: &[DynSolValue]) -> Result<Vec<u8>>;

    /// ABI-decodes the given data according to this item's input types.
    ///
    /// # Errors
    ///
    /// This function will return an error if the decoded data does not match
    /// the expected input types.
    fn decode_input(&self, data: &[u8]) -> Result<Vec<DynSolValue>>;
}

/// Provide ABI encoding and decoding for the [`Function`] type.
///
/// This trait is sealed and cannot be implemented for types outside of this
/// crate. It is implemented only for [`Function`].
pub trait FunctionExt: JsonAbiExt + Sealed {
    /// ABI-encodes the given values.
    ///
    /// Note that, contrary to [`encode_input`](JsonAbiExt::encode_input), this
    /// method does not prefix the return data with the function selector.
    ///
    /// # Errors
    ///
    /// This function will return an error if the given values do not match the
    /// expected input types.
    fn encode_output(&self, values: &[DynSolValue]) -> Result<Vec<u8>>;

    /// ABI-decodes the given data according to this functions's output types.
    ///
    /// This method does not check for any prefixes or selectors.
    fn decode_output(&self, data: &[u8]) -> Result<Vec<DynSolValue>>;
}

impl JsonAbiExt for Constructor {
    #[inline]
    fn encode_input(&self, values: &[DynSolValue]) -> Result<Vec<u8>> {
        encode_typeck(&self.inputs, values)
    }

    #[inline]
    fn encode_input_raw(&self, values: &[DynSolValue]) -> Result<Vec<u8>> {
        encode_typeck(&self.inputs, values)
    }

    #[inline]
    fn decode_input(&self, data: &[u8]) -> Result<Vec<DynSolValue>> {
        decode(data, &self.inputs)
    }
}

impl JsonAbiExt for Error {
    #[inline]
    fn encode_input(&self, values: &[DynSolValue]) -> Result<Vec<u8>> {
        encode_typeck(&self.inputs, values).map(prefix_selector(self.selector()))
    }

    #[inline]
    fn encode_input_raw(&self, values: &[DynSolValue]) -> Result<Vec<u8>> {
        encode_typeck(&self.inputs, values)
    }

    #[inline]
    fn decode_input(&self, data: &[u8]) -> Result<Vec<DynSolValue>> {
        decode(data, &self.inputs)
    }
}

impl JsonAbiExt for Function {
    #[inline]
    fn encode_input(&self, values: &[DynSolValue]) -> Result<Vec<u8>> {
        encode_typeck(&self.inputs, values).map(prefix_selector(self.selector()))
    }

    #[inline]
    fn encode_input_raw(&self, values: &[DynSolValue]) -> Result<Vec<u8>> {
        encode_typeck(&self.inputs, values)
    }

    #[inline]
    fn decode_input(&self, data: &[u8]) -> Result<Vec<DynSolValue>> {
        decode(data, &self.inputs)
    }
}

impl FunctionExt for Function {
    #[inline]
    fn encode_output(&self, values: &[DynSolValue]) -> Result<Vec<u8>> {
        encode_typeck(&self.outputs, values)
    }

    #[inline]
    fn decode_output(&self, data: &[u8]) -> Result<Vec<DynSolValue>> {
        decode(data, &self.outputs)
    }
}

#[inline]
fn prefix_selector(selector: Selector) -> impl FnOnce(Vec<u8>) -> Vec<u8> {
    move |data| {
        let mut new = Vec::with_capacity(data.len() + 4);
        new.extend_from_slice(&selector[..]);
        new.extend_from_slice(&data[..]);
        new
    }
}

fn encode_typeck(params: &[Param], values: &[DynSolValue]) -> Result<Vec<u8>> {
    if values.len() != params.len() {
        return Err(CrateError::EncodeLengthMismatch {
            expected: params.len(),
            actual: values.len(),
        })
    }
    for (value, param) in core::iter::zip(values, params) {
        let ty = param.resolve()?;
        if !ty.matches(value) {
            return Err(CrateError::TypeMismatch {
                expected: ty.sol_type_name().into_owned(),
                actual: value
                    .sol_type_name()
                    .unwrap_or_else(|| "<none>".into())
                    .into_owned(),
            })
        }
    }

    Ok(encode(values))
}

#[inline]
fn encode(values: &[DynSolValue]) -> Vec<u8> {
    DynSolValue::encode_seq(values)
}

fn decode(data: &[u8], params: &[Param]) -> Result<Vec<DynSolValue>> {
    let mut values = Vec::with_capacity(params.len());
    let mut decoder = Decoder::new(data, false);
    for param in params {
        let ty = param.resolve()?;
        let value = ty._decode(&mut decoder, crate::DynToken::decode_single_populate)?;
        values.push(value);
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, U256};

    #[test]
    fn can_encode_decode_functions() {
        let json = r#"{
            "inputs": [
                {
                    "internalType": "address",
                    "name": "",
                    "type": "address"
                },
                {
                    "internalType": "address",
                    "name": "",
                    "type": "address"
                }
            ],
            "name": "allowance",
            "outputs": [
                {
                    "internalType": "uint256",
                    "name": "",
                    "type": "uint256"
                }
            ],
            "stateMutability": "view",
            "type": "function"
        }"#;

        let func: Function = serde_json::from_str(json).unwrap();
        assert_eq!(2, func.inputs.len());
        assert_eq!(1, func.outputs.len());
        assert_eq!(func.signature(), "allowance(address,address)");

        // encode
        let expected = hex_literal::hex!(
            "dd62ed3e"
            "0000000000000000000000001111111111111111111111111111111111111111"
            "0000000000000000000000002222222222222222222222222222222222222222"
        );
        let input = [
            DynSolValue::Address(Address::repeat_byte(0x11)),
            DynSolValue::Address(Address::repeat_byte(0x22)),
        ];
        let result = func.encode_input(&input).unwrap();
        assert_eq!(expected[..], result);

        // Fail on unexpected input
        let wrong_input = [
            DynSolValue::Uint(U256::from(10u8), 256),
            DynSolValue::Address(Address::repeat_byte(2u8)),
        ];
        assert!(func.encode_input(&wrong_input).is_err());

        // decode
        let response = U256::from(1u8).to_be_bytes_vec();
        let decoded = func.decode_output(&response).unwrap();
        assert_eq!(decoded, [DynSolValue::Uint(U256::from(1u8), 256)]);

        // Fail on wrong response type
        let bad_response = Address::repeat_byte(3u8).to_vec();
        assert!(func.decode_output(&bad_response).is_err());
    }
}