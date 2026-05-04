use std::{fmt, net::IpAddr, str::FromStr};

use ipnet::IpNet;
use schemars::{json_schema, JsonSchema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IpNetwork(IpNet);

impl IpNetwork {
    pub fn get_ref(&self) -> &IpNet {
        &self.0
    }

    pub fn contains(&self, ip: &IpAddr) -> bool {
        self.0.contains(ip)
    }
}

impl From<&str> for IpNetwork {
    fn from(value: &str) -> Self {
        Self(
            IpNet::from_str(value)
                .unwrap_or_else(|e| panic!("Invalid IP network '{}': {}", value, e)),
        )
    }
}

impl From<String> for IpNetwork {
    fn from(value: String) -> Self {
        value.as_str().into()
    }
}

impl Serialize for IpNetwork {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl JsonSchema for IpNetwork {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "IpNetwork".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        json_schema!({
            "type": "string",
            "description": "An IPv4 or IPv6 network in CIDR notation, or a single IP address.",
            "examples": [
                "10.0.0.0/8",
                "192.168.1.10/32",
                "2001:db8::/32",
                "127.0.0.1"
            ]
        })
    }

    fn inline_schema() -> bool {
        true
    }
}

struct IpNetworkVisitor;

impl<'de> serde::de::Visitor<'de> for IpNetworkVisitor {
    type Value = IpNetwork;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(
            "an IPv4/IPv6 CIDR network or single IP address, e.g. \"10.0.0.0/8\" or \"127.0.0.1\"",
        )
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if let Ok(network) = IpNet::from_str(value) {
            return Ok(IpNetwork(network));
        }

        IpAddr::from_str(value)
            .map(IpNet::from)
            .map(IpNetwork)
            .map_err(serde::de::Error::custom)
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_str(value)
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_str(&value)
    }
}

impl<'de> Deserialize<'de> for IpNetwork {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(IpNetworkVisitor)
    }
}
