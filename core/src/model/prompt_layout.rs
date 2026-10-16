/// Prompt segments in fixed rendering order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptSegmentKind {
    StableSystem,
    ToolSchema,
    DirectoryTree,
    Experience,
    Conversation,
    Volatile,
}

impl PromptSegmentKind {
    fn label(self) -> &'static str {
        match self {
            Self::StableSystem => "stable_system",
            Self::ToolSchema => "tool_schema",
            Self::DirectoryTree => "directory_tree",
            Self::Experience => "experience",
            Self::Conversation => "conversation",
            Self::Volatile => "volatile",
        }
    }

    const fn is_stable(self) -> bool {
        !matches!(self, Self::Conversation | Self::Volatile)
    }
}

/// A named prompt segment. Empty/whitespace-only content is omitted at render time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSegment {
    pub kind: PromptSegmentKind,
    pub content: String,
}

/// Builder for a cache-friendly prompt.
#[derive(Debug, Clone, Default)]
pub struct PromptLayout {
    stable_system: Option<String>,
    tool_schema: Option<String>,
    directory_tree: Option<String>,
    experience: Option<String>,
    conversation: Option<String>,
    volatile: Option<String>,
}

/// Rendered prompt and fingerprints suitable for cache keys and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPrompt {
    pub text: String,
    pub segments: Vec<PromptSegmentKind>,
    /// Fingerprint of stable segments only; volatile changes cannot affect it.
    pub stable_fingerprint: String,
    /// Fingerprint of the complete redacted prompt.
    pub fingerprint: String,
}

impl PromptLayout {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    #[must_use]
    pub fn stable_system(mut self, value: impl Into<String>) -> Self {
        self.stable_system = Some(value.into());
        self
    }
    #[must_use]
    pub fn tool_schema(mut self, value: impl Into<String>) -> Self {
        self.tool_schema = Some(value.into());
        self
    }
    #[must_use]
    pub fn directory_tree(mut self, value: impl Into<String>) -> Self {
        self.directory_tree = Some(value.into());
        self
    }
    #[must_use]
    pub fn experience(mut self, value: impl Into<String>) -> Self {
        self.experience = Some(value.into());
        self
    }
    #[must_use]
    pub fn conversation(mut self, value: impl Into<String>) -> Self {
        self.conversation = Some(value.into());
        self
    }
    #[must_use]
    pub fn volatile(mut self, value: impl Into<String>) -> Self {
        self.volatile = Some(value.into());
        self
    }
    #[must_use]
    pub fn render(&self) -> RenderedPrompt {
        let ordered = [
            (
                PromptSegmentKind::StableSystem,
                self.stable_system.as_deref(),
            ),
            (PromptSegmentKind::ToolSchema, self.tool_schema.as_deref()),
            (
                PromptSegmentKind::DirectoryTree,
                self.directory_tree.as_deref(),
            ),
            (PromptSegmentKind::Experience, self.experience.as_deref()),
            (
                PromptSegmentKind::Conversation,
                self.conversation.as_deref(),
            ),
            (PromptSegmentKind::Volatile, self.volatile.as_deref()),
        ];
        let segments: Vec<PromptSegment> = ordered
            .into_iter()
            .filter_map(|(kind, content)| {
                let content = content?.trim();
                (!content.is_empty()).then(|| PromptSegment {
                    kind,
                    content: content.to_string(),
                })
            })
            .collect();
        let redacted: Vec<String> = segments
            .iter()
            .map(|segment| {
                format!(
                    "[{}]\n{}",
                    segment.kind.label(),
                    redact_sensitive(&segment.content)
                )
            })
            .collect();
        let stable_input = segments
            .iter()
            .filter(|segment| segment.kind.is_stable())
            .map(|segment| {
                format!(
                    "[{}]\n{}",
                    segment.kind.label(),
                    redact_sensitive(&segment.content)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let text = redacted.join("\n\n");
        RenderedPrompt {
            text,
            segments: segments.iter().map(|segment| segment.kind).collect(),
            stable_fingerprint: fingerprint(&stable_input),
            fingerprint: fingerprint(&redacted.join("\n\n")),
        }
    }
}

fn fingerprint(value: &str) -> String {
    // FNV-1a avoids a randomized process hash and requires no dependency.
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn redact_sensitive(value: &str) -> String {
    value
        .lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            let sensitive = ["api_key", "apikey", "secret", "password", "token"]
                .iter()
                .any(|needle| lower.contains(needle));
            if sensitive {
                line.split_once(['=', ':']).map_or_else(
                    || "[REDACTED]".to_string(),
                    |(name, _)| format!("{name}: [REDACTED]"),
                )
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Honest three-state result for provider prompt-cache capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderCacheState {
    Supported,
    Unsupported,
    Unknown,
}

/// Provider capability probe adapter. No network access is performed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCacheProbe {
    pub provider: String,
    pub state: ProviderCacheState,
}

impl ProviderCacheProbe {
    #[must_use]
    pub fn from_observation(provider: impl Into<String>, observed: Option<bool>) -> Self {
        let state = match observed {
            Some(true) => ProviderCacheState::Supported,
            Some(false) => ProviderCacheState::Unsupported,
            None => ProviderCacheState::Unknown,
        };
        Self {
            provider: provider.into(),
            state,
        }
    }
    #[must_use]
    pub const fn is_supported(&self) -> bool {
        matches!(self.state, ProviderCacheState::Supported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_fingerprint_ignores_volatile_content() {
        let a = PromptLayout::new()
            .stable_system("system-v1")
            .tool_schema("tools-v1")
            .conversation("question-a")
            .render();
        let b = PromptLayout::new()
            .stable_system("system-v1")
            .tool_schema("tools-v1")
            .conversation("question-b")
            .render();
        assert_eq!(a.stable_fingerprint, b.stable_fingerprint);
        assert_ne!(a.fingerprint, b.fingerprint);
    }

    #[test]
    fn stable_segments_render_in_fixed_order() {
        let text = PromptLayout::new()
            .conversation("conversation")
            .experience("experience")
            .directory_tree("tree")
            .tool_schema("tools")
            .stable_system("system")
            .render()
            .text;
        assert!(text.find("system").unwrap() < text.find("tools").unwrap());
        assert!(text.find("tools").unwrap() < text.find("tree").unwrap());
        assert!(text.find("tree").unwrap() < text.find("experience").unwrap());
        assert!(text.find("experience").unwrap() < text.find("conversation").unwrap());
    }

    #[test]
    fn empty_segments_are_omitted() {
        let rendered = PromptLayout::new()
            .stable_system(" ")
            .conversation("hello")
            .render();
        assert_eq!(rendered.segments, vec![PromptSegmentKind::Conversation]);
    }

    #[test]
    fn fingerprint_redacts_secret_values() {
        let one = PromptLayout::new()
            .stable_system("system")
            .conversation("api_key=alpha-secret")
            .render();
        let two = PromptLayout::new()
            .stable_system("system")
            .conversation("api_key=beta-secret")
            .render();
        assert!(!one.text.contains("alpha-secret"));
        assert!(!two.text.contains("beta-secret"));
        assert_eq!(one.fingerprint, two.fingerprint);
    }

    #[test]
    fn unknown_provider_cache_is_not_supported() {
        let result = ProviderCacheProbe::from_observation("acme", None);
        assert_eq!(result.state, ProviderCacheState::Unknown);
        assert!(!result.is_supported());
    }

    #[test]
    fn provider_probe_preserves_explicit_states() {
        assert_eq!(
            ProviderCacheProbe::from_observation("a", Some(true)).state,
            ProviderCacheState::Supported
        );
        assert_eq!(
            ProviderCacheProbe::from_observation("b", Some(false)).state,
            ProviderCacheState::Unsupported
        );
    }
}
