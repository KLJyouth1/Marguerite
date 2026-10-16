//! WP5: offline / on-premise export of audit records.
//!
//! Pure serialization — no file I/O. Produces a single CSV line per audit
//! entry, correctly quoting fields that contain commas, quotes, or newlines
//! (RFC 4180 style).

/// One audit record ready for offline export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    /// Event timestamp (already formatted, e.g. RFC3339).
    pub ts: String,
    /// Who performed the action.
    pub actor: String,
    /// What action was performed.
    pub action: String,
    /// What the action targeted.
    pub target: String,
    /// Outcome of the action.
    pub result: String,
}

impl AuditEntry {
    pub fn new(
        ts: impl Into<String>,
        actor: impl Into<String>,
        action: impl Into<String>,
        target: impl Into<String>,
        result: impl Into<String>,
    ) -> Self {
        Self {
            ts: ts.into(),
            actor: actor.into(),
            action: action.into(),
            target: target.into(),
            result: result.into(),
        }
    }

    /// The canonical CSV header line for [`AuditEntry`] rows.
    pub fn csv_header() -> &'static str {
        "ts,actor,action,target,result"
    }

    /// Serialize this entry into one CSV line (no trailing newline).
    pub fn to_csv_line(&self) -> String {
        let fields = [&self.ts, &self.actor, &self.action, &self.target, &self.result];
        fields
            .iter()
            .map(|f| csv_escape(f))
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// RFC 4180 escaping: quote the field if it contains a comma, double quote,
/// carriage return, or line feed; double any embedded quotes.
fn csv_escape(field: &str) -> String {
    let needs_quoting = field.contains([',', '"', '\r', '\n']);
    if needs_quoting {
        let escaped = field.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        field.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_fields_are_unquoted() {
        let e = AuditEntry::new(
            "2026-09-28T10:00:00Z",
            "admin",
            "session.login",
            "console",
            "ok",
        );
        assert_eq!(
            e.to_csv_line(),
            "2026-09-28T10:00:00Z,admin,session.login,console,ok"
        );
    }

    #[test]
    fn comma_in_field_is_quoted() {
        let e = AuditEntry::new("ts", "actor", "file.write", "C:\\data,a,b.txt", "ok");
        assert_eq!(
            e.to_csv_line(),
            "ts,actor,file.write,\"C:\\data,a,b.txt\",ok"
        );
    }

    #[test]
    fn embedded_quotes_are_doubled_and_quoted() {
        let e = AuditEntry::new("ts", "ceo", "comment", "said \"hello\" world", "ok");
        assert_eq!(
            e.to_csv_line(),
            "ts,ceo,comment,\"said \"\"hello\"\" world\",ok"
        );
    }

    #[test]
    fn comma_and_quote_together_are_escaped() {
        let e = AuditEntry::new("ts", "a", "b", "x=\"1\", y=\"2\"", "fail");
        assert_eq!(e.to_csv_line(), "ts,a,b,\"x=\"\"1\"\", y=\"\"2\"\"\",fail");
    }

    #[test]
    fn empty_field_stays_empty() {
        let e = AuditEntry::new("ts", "", "action", "", "result");
        assert_eq!(e.to_csv_line(), "ts,,action,,result");
    }

    #[test]
    fn header_matches_field_order() {
        assert_eq!(
            AuditEntry::csv_header(),
            "ts,actor,action,target,result"
        );
    }
}
