use super::OpsError;
use serde::{de::Error as DeError, Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostRef {
    pub id: String,
    pub address: String,
    pub credential_ref: String,
}
impl<'de> Deserialize<'de> for HostRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input { id: String, address: String, credential_ref: Option<String> }
        let input = Input::deserialize(deserializer)?;
        HostRef::new(input.id, input.address, input.credential_ref.unwrap_or_default()).map_err(D::Error::custom)
    }
}

impl HostRef {
    pub fn new(id: impl Into<String>, address: impl Into<String>, credential_ref: impl Into<String>) -> Result<Self, OpsError> {
        let value = Self { id: id.into(), address: address.into(), credential_ref: credential_ref.into() };
        if value.id.trim().is_empty() || value.address.trim().is_empty() { return Err(OpsError::invalid("host id and address are required")); }
        if value.credential_ref.trim().is_empty() { return Err(OpsError::credential_required()); }
        Ok(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostStatus {
    pub host_id: String,
    pub online: bool,
    pub detail: String,
}
