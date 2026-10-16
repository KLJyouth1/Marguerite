use super::OpsError;

pub fn inspect_process_name(name: &str) -> Result<String, OpsError> {
    let value = name.trim();
    if value.is_empty() { return Err(OpsError::invalid("process name is required")); }
    if value.len() > 128 { return Err(OpsError::invalid("process name too long")); }
    Ok(value.to_string())
}
