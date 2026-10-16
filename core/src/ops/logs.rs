use super::OpsError;

pub fn tail_content(content: &str, max_lines: usize, max_bytes: usize) -> Result<String, OpsError> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() > max_lines { return Err(OpsError::invalid("log line limit exceeded")); }
    if content.len() > max_bytes { return Err(OpsError::invalid("log byte limit exceeded")); }
    Ok(content.to_string())
}
