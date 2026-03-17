//! Bundled asset files for `ralph strategy sync`.
//!
//! These are compiled into the binary via `include_str!` and written
//! to disk during strategy sync.

pub const STRATEGY_TOML: &str = include_str!("../../../../assets/strategy.toml");

/// All agent assets as (filename, content) pairs.
///
/// Used by the sync planner to determine which files to write.
pub const AGENT_ASSETS: &[(&str, &str)] = &[
    (
        "architect.md",
        include_str!("../../../../assets/agents/architect.md"),
    ),
    (
        "developer.md",
        include_str!("../../../../assets/agents/developer.md"),
    ),
    (
        "reviewer.md",
        include_str!("../../../../assets/agents/reviewer.md"),
    ),
    (
        "tester.md",
        include_str!("../../../../assets/agents/tester.md"),
    ),
    (
        "product-manager.md",
        include_str!("../../../../assets/agents/product-manager.md"),
    ),
    (
        "storyteller.md",
        include_str!("../../../../assets/agents/storyteller.md"),
    ),
    (
        "editor-agent.md",
        include_str!("../../../../assets/agents/editor-agent.md"),
    ),
    (
        "worldbuilder.md",
        include_str!("../../../../assets/agents/worldbuilder.md"),
    ),
    (
        "critic.md",
        include_str!("../../../../assets/agents/critic.md"),
    ),
];

/// All strategy assets as (filename, content) pairs.
///
/// Used by the sync planner to determine which strategy files to write.
pub const STRATEGY_ASSETS: &[(&str, &str)] = &[
    (
        "prd-loop.toml",
        include_str!("../../../../assets/strategies/prd-loop.toml"),
    ),
    (
        "fiction-loop.toml",
        include_str!("../../../../assets/strategies/fiction-loop.toml"),
    ),
];
