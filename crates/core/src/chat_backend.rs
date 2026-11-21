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
}
