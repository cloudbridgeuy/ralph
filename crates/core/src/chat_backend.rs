use crate::chat_process::CACHE_TTL_SECS;
use crate::worker::Support;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub harness: &'static str,
    pub cost: Support,
    pub cache_tokens: Support,
    pub cache_ttl: Support,
    pub context_tokens: Support,
    pub cancel: Support,
    pub mcp_tools: Support,
    pub system_prompt: Support,
    pub tool_blocks: Support,
}

pub const CLAUDE: Caps = Caps {
    harness: "claude",
    cost: Support::Full,
    cache_tokens: Support::Full,
    cache_ttl: Support::Full,
    context_tokens: Support::Full,
    cancel: Support::Full,
    mcp_tools: Support::Full,
    system_prompt: Support::Full,
    tool_blocks: Support::Full,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryHarness {
    Claude,
    Codex,
    Agy,
}

impl MemoryHarness {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "agy" => Some(Self::Agy),
            _ => None,
        }
    }

    pub fn caps(self) -> Caps {
        match self {
            Self::Claude => CLAUDE,
            Self::Codex => Caps {
                harness: "codex",
                cost: Support::None,
                ..CLAUDE
            },
            Self::Agy => Caps {
                harness: "agy",
                cost: Support::None,
                ..CLAUDE
            },
        }
    }

    pub fn inlines_instructions(self) -> bool {
        matches!(self, Self::Codex | Self::Agy)
    }

    pub fn needs_explicit_model(self) -> bool {
        matches!(self, Self::Codex | Self::Agy)
    }
}

pub fn cache_ttl_secs(caps: &Caps) -> Option<i64> {
    match caps.cache_ttl {
        Support::None => None,
        Support::Full | Support::Partial(_) => Some(CACHE_TTL_SECS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_supports_every_capability_in_full() {
        let Caps {
            harness: _,
            cost,
            cache_tokens,
            cache_ttl,
            context_tokens,
            cancel,
            mcp_tools,
            system_prompt,
            tool_blocks,
        } = CLAUDE;
        for support in [
            cost,
            cache_tokens,
            cache_ttl,
            context_tokens,
            cancel,
            mcp_tools,
            system_prompt,
            tool_blocks,
        ] {
            assert_eq!(support, Support::Full);
        }
    }

    #[test]
    fn memory_harness_parses_known_names_only() {
        assert_eq!(MemoryHarness::parse("agy"), Some(MemoryHarness::Agy));
        assert_eq!(MemoryHarness::parse("codex"), Some(MemoryHarness::Codex));
        assert_eq!(MemoryHarness::parse("claude"), Some(MemoryHarness::Claude));
        assert_eq!(MemoryHarness::parse("other"), None);
    }

    #[test]
    fn memory_harness_caps_name_the_harness_and_drop_cost() {
        assert_eq!(MemoryHarness::Claude.caps(), CLAUDE);
        for (harness, name) in [(MemoryHarness::Codex, "codex"), (MemoryHarness::Agy, "agy")] {
            let caps = harness.caps();
            assert_eq!((caps.harness, caps.cost), (name, Support::None));
        }
    }

    #[test]
    fn only_claude_keeps_instructions_out_of_the_prompt_and_default_model() {
        for harness in [MemoryHarness::Codex, MemoryHarness::Agy] {
            assert!(harness.inlines_instructions() && harness.needs_explicit_model());
        }
        let claude = MemoryHarness::Claude;
        assert!(!claude.inlines_instructions() && !claude.needs_explicit_model());
    }
}
