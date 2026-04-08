use std::str::FromStr;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use starknet::core::chain_id::{MAINNET, SEPOLIA};
use starknet::core::types::Felt;

use crate::Error;

/// Represent the chain id which is either Sepolia, Mainnet, or an arbitrary
/// (Unknown) chain id supplied by configuration.
///
/// Unknown chain ids preserve the configured felt value (so transaction
/// signing and domain separation use the real chain id), while every other
/// chain-derived default falls back to the Sepolia values.
#[derive(Debug, Clone, Copy, Hash)]
pub enum ChainID {
    Sepolia,
    Mainnet,
    Unknown(Felt),
}

impl ChainID {
    /// Convert the identifier representation to a ChainID
    /// - SN_SEPOLIA -> ChainID::Sepolia
    /// - SN_MAIN -> ChainID::Mainnet
    ///
    /// Falls back to `Unknown` if the value can be parsed as a hex felt.
    pub fn from_identifier(s: &str) -> Result<Self, Error> {
        match s {
            "SN_SEPOLIA" => Ok(Self::Sepolia),
            "SN_MAIN" => Ok(Self::Mainnet),
            other => Felt::from_hex(other)
                .map(Self::Unknown)
                .map_err(|_| Error::TypedDataDecoding(format!("invalid domain {}", other))),
        }
    }

    /// Convert the ChainID to the identifier representation
    /// - ChainID::Sepolia -> "SN_SEPOLIA"
    /// - ChainID::Mainnet -> "SN_MAIN"
    /// - ChainID::Unknown(f) -> hex string of f
    pub fn as_identifier(&self) -> String {
        match self {
            Self::Sepolia => String::from_str("SN_SEPOLIA").unwrap(),
            Self::Mainnet => String::from_str("SN_MAIN").unwrap(),
            Self::Unknown(f) => f.to_hex_string(),
        }
    }

    /// Convert a valid chain-id string into a ChainID
    /// - sepolia -> ChainID::Sepolia
    /// - mainnet -> ChainID::Mainnet
    ///
    /// Any other string is parsed as a hex felt and returned as
    /// `ChainID::Unknown`. If parsing fails, an error is returned.
    pub fn from_string(s: &str) -> Result<Self, Error> {
        match s {
            "sepolia" | "SEPOLIA" | "Sepolia" | "SN_SEPOLIA" => Ok(Self::Sepolia),
            "mainnet" | "Mainnet" | "SN_MAINNET" | "SN_MAIN" | "main" | "MAIN" => Ok(Self::Mainnet),
            other => Felt::from_hex(other)
                .map(Self::Unknown)
                .map_err(|_| Error::TypedDataDecoding(format!("invalid domain {}", other))),
        }
    }

    /// Convert a Felt into a ChainID. Unrecognized felts are preserved as
    /// `ChainID::Unknown` so the original value is kept intact.
    pub fn from_felt(value: Felt) -> Result<Self, Error> {
        if value == SEPOLIA {
            Ok(Self::Sepolia)
        } else if value == MAINNET {
            Ok(Self::Mainnet)
        } else {
            Ok(Self::Unknown(value))
        }
    }

    pub fn as_felt(&self) -> Felt {
        match self {
            Self::Sepolia => SEPOLIA,
            Self::Mainnet => MAINNET,
            Self::Unknown(f) => *f,
        }
    }
}

impl Serialize for ChainID {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Sepolia => serializer.serialize_str("sepolia"),
            Self::Mainnet => serializer.serialize_str("mainnet"),
            Self::Unknown(f) => serializer.serialize_str(&f.to_hex_string()),
        }
    }
}

impl<'de> Deserialize<'de> for ChainID {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ChainIDVisitor;

        impl<'de> Visitor<'de> for ChainIDVisitor {
            type Value = ChainID;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a chain id string (e.g. \"sepolia\", \"mainnet\", or a hex felt)")
            }

            fn visit_str<E>(self, value: &str) -> Result<ChainID, E>
            where
                E: de::Error,
            {
                ChainID::from_string(value).map_err(|e| de::Error::custom(e.to_string()))
            }
        }

        deserializer.deserialize_str(ChainIDVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_string_unknown_returns_unknown_variant() {
        let raw = "0x534e5f4b41545241";
        let parsed = ChainID::from_string(raw).expect("hex felt should parse");
        match parsed {
            ChainID::Unknown(f) => assert_eq!(f, Felt::from_hex(raw).unwrap()),
            other => panic!("expected Unknown, got {:?}", other),
        }
    }

    #[test]
    fn from_string_garbage_errors() {
        assert!(ChainID::from_string("not-a-chain").is_err());
    }

    #[test]
    fn from_felt_unknown_preserves_value() {
        let felt = Felt::from_hex("0x534e5f4b41545241").unwrap();
        match ChainID::from_felt(felt).unwrap() {
            ChainID::Unknown(f) => assert_eq!(f, felt),
            other => panic!("expected Unknown, got {:?}", other),
        }
    }

    #[test]
    fn as_felt_unknown_returns_inner() {
        let felt = Felt::from_hex("0x534e5f4b41545241").unwrap();
        assert_eq!(ChainID::Unknown(felt).as_felt(), felt);
    }

    #[test]
    fn serde_round_trip_unknown() {
        let felt = Felt::from_hex("0x534e5f4b41545241").unwrap();
        let chain = ChainID::Unknown(felt);
        let json = serde_json::to_string(&chain).unwrap();
        assert_eq!(json, format!("\"{}\"", felt.to_hex_string()));

        let back: ChainID = serde_json::from_str(&json).unwrap();
        assert_eq!(back.as_felt(), felt);
    }

    #[test]
    fn serde_round_trip_known() {
        let json = serde_json::to_string(&ChainID::Sepolia).unwrap();
        assert_eq!(json, "\"sepolia\"");
        let back: ChainID = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, ChainID::Sepolia));

        let json = serde_json::to_string(&ChainID::Mainnet).unwrap();
        assert_eq!(json, "\"mainnet\"");
        let back: ChainID = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, ChainID::Mainnet));
    }
}
