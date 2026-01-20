//! Session management types and slug generation.
//!
//! This module provides:
//! - Pure functions for generating memorable session slugs in the format
//!   "adjective-noun" (e.g., "quiet-mountain", "fuzzy-walrus")
//! - Type definitions for session metadata and the global sessions index
//!
//! Following the Functional Core pattern, I/O operations (file creation,
//! uniqueness checks against disk) happen at the shell layer.

use chrono::{DateTime, Utc};
use rand::seq::SliceRandom;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

/// Adjectives used for slug generation.
/// At least 50 adjectives for variety as per acceptance criteria.
const ADJECTIVES: &[&str] = &[
    "amber",
    "ancient",
    "autumn",
    "azure",
    "blazing",
    "bold",
    "brave",
    "bright",
    "bronze",
    "calm",
    "celestial",
    "clever",
    "copper",
    "coral",
    "crimson",
    "crystal",
    "curious",
    "daring",
    "dawn",
    "deep",
    "delicate",
    "distant",
    "dusk",
    "eager",
    "elegant",
    "ember",
    "emerald",
    "ethereal",
    "fading",
    "fierce",
    "flaming",
    "flowing",
    "foggy",
    "frozen",
    "fuzzy",
    "gentle",
    "gilded",
    "glowing",
    "golden",
    "graceful",
    "hidden",
    "hollow",
    "humble",
    "icy",
    "jade",
    "keen",
    "lively",
    "lonely",
    "lunar",
    "misty",
    "mossy",
    "nimble",
    "noble",
    "onyx",
    "pale",
    "patient",
    "peaceful",
    "phantom",
    "polished",
    "proud",
    "quiet",
    "radiant",
    "rapid",
    "regal",
    "restless",
    "rising",
    "roaming",
    "rugged",
    "rustic",
    "sacred",
    "sapphire",
    "scarlet",
    "serene",
    "shadow",
    "shining",
    "silent",
    "silver",
    "sleek",
    "snowy",
    "soaring",
    "solar",
    "solitary",
    "sparkling",
    "spring",
    "steady",
    "stellar",
    "still",
    "stormy",
    "subtle",
    "summer",
    "sunny",
    "swift",
    "tender",
    "thorny",
    "thunder",
    "twilight",
    "velvet",
    "vibrant",
    "wandering",
    "warm",
    "whispering",
    "wild",
    "winding",
    "winter",
    "wise",
];

/// Nouns used for slug generation.
/// At least 50 nouns for variety as per acceptance criteria.
const NOUNS: &[&str] = &[
    "anchor",
    "arrow",
    "aurora",
    "badger",
    "beacon",
    "bear",
    "birch",
    "blaze",
    "blossom",
    "boulder",
    "breeze",
    "brook",
    "canyon",
    "cedar",
    "cliff",
    "cloud",
    "comet",
    "coral",
    "cosmos",
    "cove",
    "crane",
    "creek",
    "crown",
    "crystal",
    "cypress",
    "dawn",
    "delta",
    "desert",
    "dew",
    "dolphin",
    "dove",
    "dune",
    "eagle",
    "elm",
    "ember",
    "falcon",
    "fern",
    "field",
    "finch",
    "flame",
    "flower",
    "forest",
    "forge",
    "fox",
    "frost",
    "garden",
    "glacier",
    "glen",
    "grove",
    "harbor",
    "hawk",
    "hearth",
    "heron",
    "hill",
    "hollow",
    "horizon",
    "island",
    "ivy",
    "jade",
    "lake",
    "lark",
    "leaf",
    "lighthouse",
    "lily",
    "lotus",
    "maple",
    "marsh",
    "meadow",
    "meteor",
    "mist",
    "moon",
    "moss",
    "mountain",
    "nebula",
    "oak",
    "ocean",
    "orchid",
    "otter",
    "owl",
    "panda",
    "panther",
    "path",
    "peak",
    "pebble",
    "pine",
    "plum",
    "pond",
    "prairie",
    "quartz",
    "rain",
    "raven",
    "reef",
    "ridge",
    "river",
    "robin",
    "rose",
    "sage",
    "salmon",
    "sand",
    "sea",
    "shadow",
    "shore",
    "sky",
    "snow",
    "sparrow",
    "spring",
    "star",
    "stone",
    "storm",
    "stream",
    "summit",
    "sun",
    "swan",
    "temple",
    "thistle",
    "thunder",
    "tiger",
    "trail",
    "tree",
    "valley",
    "violet",
    "walrus",
    "wave",
    "willow",
    "wind",
    "wolf",
    "wood",
    "wren",
    "zenith",
];

/// Generate a session slug using the provided random number generator.
///
/// This is a pure function that generates a slug in the format "adjective-noun".
/// The caller is responsible for checking uniqueness against existing sessions.
///
/// # Arguments
///
/// * `rng` - A random number generator for selecting words
///
/// # Returns
///
/// A lowercase hyphenated slug like "quiet-mountain" or "fuzzy-walrus".
///
/// # Example
///
/// ```
/// use ralph_core::session::generate_slug;
/// use rand::thread_rng;
///
/// let slug = generate_slug(&mut thread_rng());
/// assert!(slug.contains('-'));
/// assert_eq!(slug, slug.to_lowercase());
/// ```
#[allow(clippy::expect_used)] // Intentional: ADJECTIVES/NOUNS are compile-time non-empty arrays
pub fn generate_slug<R: Rng + ?Sized>(rng: &mut R) -> String {
    // SAFETY: ADJECTIVES and NOUNS are compile-time static arrays with 100+ elements each.
    // These expect() calls cannot fail at runtime since the arrays are guaranteed non-empty.
    // This is an intentional invariant assertion, not error handling.
    let adjective = ADJECTIVES
        .choose(rng)
        .expect("compile-time invariant: ADJECTIVES array is non-empty");
    let noun = NOUNS
        .choose(rng)
        .expect("compile-time invariant: NOUNS array is non-empty");
    format!("{}-{}", adjective, noun)
}

/// Generate a unique session slug given a set of existing slugs.
///
/// Attempts to generate a slug that doesn't exist in the provided set.
/// If after `max_attempts` tries a unique slug cannot be generated,
/// returns `None`.
///
/// # Arguments
///
/// * `rng` - A random number generator for selecting words
/// * `existing_slugs` - Set of slugs that already exist
/// * `max_attempts` - Maximum number of generation attempts before giving up
///
/// # Returns
///
/// * `Some(slug)` - A unique slug not in `existing_slugs`
/// * `None` - If a unique slug couldn't be generated within `max_attempts`
///
/// # Example
///
/// ```
/// use ralph_core::session::generate_unique_slug;
/// use rand::thread_rng;
/// use std::collections::HashSet;
///
/// let existing: HashSet<String> = HashSet::new();
/// let slug = generate_unique_slug(&mut thread_rng(), &existing, 100);
/// assert!(slug.is_some());
/// ```
pub fn generate_unique_slug<R: Rng + ?Sized>(
    rng: &mut R,
    existing_slugs: &std::collections::HashSet<String>,
    max_attempts: usize,
) -> Option<String> {
    for _ in 0..max_attempts {
        let slug = generate_slug(rng);
        if !existing_slugs.contains(&slug) {
            return Some(slug);
        }
    }
    None
}

/// Validate that a slug follows the expected format.
///
/// A valid slug must:
/// - Be lowercase
/// - Contain exactly one hyphen
/// - Have non-empty parts on both sides of the hyphen
///
/// # Arguments
///
/// * `slug` - The slug string to validate
///
/// # Returns
///
/// `true` if the slug is valid, `false` otherwise.
///
/// # Example
///
/// ```
/// use ralph_core::session::is_valid_slug;
///
/// assert!(is_valid_slug("quiet-mountain"));
/// assert!(!is_valid_slug("LOUD-MOUNTAIN"));
/// assert!(!is_valid_slug("no-hyphen-here-twice"));
/// assert!(!is_valid_slug("nohyphen"));
/// ```
pub fn is_valid_slug(slug: &str) -> bool {
    // Must be lowercase
    if slug != slug.to_lowercase() {
        return false;
    }

    // Must contain exactly one hyphen
    let parts: Vec<&str> = slug.split('-').collect();
    if parts.len() != 2 {
        return false;
    }

    // Both parts must be non-empty and contain only lowercase letters
    parts
        .iter()
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase()))
}

/// Get the total number of possible unique slug combinations.
///
/// Useful for understanding the collision space when checking uniqueness.
///
/// # Returns
///
/// The product of adjective count and noun count.
pub fn total_slug_combinations() -> usize {
    ADJECTIVES.len() * NOUNS.len()
}

// =============================================================================
// Session Management Types
// =============================================================================

/// The outcome/status of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionOutcome {
    /// Session is currently running
    #[default]
    InProgress,
    /// Session completed successfully (all stories done)
    Completed,
    /// Session was manually aborted by the user at the failure prompt
    Aborted,
    /// Session failed due to an error
    Failed,
    /// Session was interrupted by a signal (SIGINT/SIGTERM)
    Interrupted,
}

impl std::fmt::Display for SessionOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionOutcome::InProgress => write!(f, "in_progress"),
            SessionOutcome::Completed => write!(f, "completed"),
            SessionOutcome::Aborted => write!(f, "aborted"),
            SessionOutcome::Failed => write!(f, "failed"),
            SessionOutcome::Interrupted => write!(f, "interrupted"),
        }
    }
}

/// An entry in the global sessions.toml index.
///
/// This represents a single session across all projects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    /// Unique session identifier (e.g., "quiet-mountain")
    pub slug: String,
    /// Absolute path to the project directory
    pub project: PathBuf,
    /// When the session started
    pub started_at: DateTime<Utc>,
    /// When the session completed (if finished)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Number of iterations run
    #[serde(default)]
    pub iterations: u32,
    /// Current status of the session
    #[serde(default)]
    pub outcome: SessionOutcome,
}

impl SessionEntry {
    /// Create a new session entry with initial values.
    ///
    /// Sets started_at to now, iterations to 0, and outcome to InProgress.
    pub fn new(slug: String, project: PathBuf) -> Self {
        Self {
            slug,
            project,
            started_at: Utc::now(),
            completed_at: None,
            iterations: 0,
            outcome: SessionOutcome::InProgress,
        }
    }
}

/// The global sessions index stored at ~/.config/ralph/sessions.toml.
///
/// Contains entries for all sessions across all projects.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionsIndex {
    /// All session entries
    #[serde(default)]
    pub sessions: Vec<SessionEntry>,
}

impl SessionsIndex {
    /// Create an empty sessions index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all existing slugs as a HashSet for collision checking.
    pub fn existing_slugs(&self) -> HashSet<String> {
        self.sessions.iter().map(|s| s.slug.clone()).collect()
    }

    /// Check if a slug already exists in the index.
    pub fn slug_exists(&self, slug: &str) -> bool {
        self.sessions.iter().any(|s| s.slug == slug)
    }

    /// Add a new session entry to the index.
    pub fn add_session(&mut self, entry: SessionEntry) {
        self.sessions.push(entry);
    }

    /// Find a session by slug.
    pub fn find_by_slug(&self, slug: &str) -> Option<&SessionEntry> {
        self.sessions.iter().find(|s| s.slug == slug)
    }

    /// Find a session by slug (mutable).
    pub fn find_by_slug_mut(&mut self, slug: &str) -> Option<&mut SessionEntry> {
        self.sessions.iter_mut().find(|s| s.slug == slug)
    }

    /// Serialize the index to TOML string.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Deserialize from TOML string.
    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(content)
    }
}

/// Metadata for a single session, stored in session.toml within the session directory.
///
/// This contains the same information as SessionEntry but is stored locally
/// in the session directory for self-contained session data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    /// Unique session identifier
    pub slug: String,
    /// Absolute path to the project directory
    pub project: PathBuf,
    /// When the session started
    pub started_at: DateTime<Utc>,
    /// When the session completed (if finished)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Number of iterations run
    #[serde(default)]
    pub iterations: u32,
    /// Current status of the session
    #[serde(default)]
    pub outcome: SessionOutcome,
    /// The prompt passed to the Claude CLI (after placeholder substitution).
    /// Stored for replay to show what prompt was used for this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

impl SessionMetadata {
    /// Create new session metadata with initial values.
    pub fn new(slug: String, project: PathBuf, prompt: Option<String>) -> Self {
        Self {
            slug,
            project,
            started_at: Utc::now(),
            completed_at: None,
            iterations: 0,
            outcome: SessionOutcome::InProgress,
            prompt,
        }
    }

    /// Serialize to TOML string.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Deserialize from TOML string.
    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(content)
    }
}

impl From<&SessionEntry> for SessionMetadata {
    fn from(entry: &SessionEntry) -> Self {
        Self {
            slug: entry.slug.clone(),
            project: entry.project.clone(),
            started_at: entry.started_at,
            completed_at: entry.completed_at,
            iterations: entry.iterations,
            outcome: entry.outcome,
            // SessionEntry doesn't store prompt, so default to None
            prompt: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use std::collections::HashSet;

    // Use a seeded RNG for deterministic tests
    fn test_rng() -> rand::rngs::StdRng {
        rand::rngs::StdRng::seed_from_u64(12345)
    }

    #[test]
    fn test_generate_slug_format() {
        let mut rng = test_rng();
        let slug = generate_slug(&mut rng);

        // Should be lowercase
        assert_eq!(slug, slug.to_lowercase());

        // Should contain exactly one hyphen
        assert_eq!(slug.matches('-').count(), 1);

        // Should have two non-empty parts
        let parts: Vec<&str> = slug.split('-').collect();
        assert_eq!(parts.len(), 2);
        assert!(!parts[0].is_empty());
        assert!(!parts[1].is_empty());
    }

    #[test]
    fn test_generate_slug_deterministic_with_seed() {
        let mut rng1 = rand::rngs::StdRng::seed_from_u64(42);
        let mut rng2 = rand::rngs::StdRng::seed_from_u64(42);

        let slug1 = generate_slug(&mut rng1);
        let slug2 = generate_slug(&mut rng2);

        assert_eq!(slug1, slug2);
    }

    #[test]
    fn test_generate_slug_variety() {
        let mut rng = rand::thread_rng();
        let mut slugs = HashSet::new();

        // Generate 100 slugs and check we get variety
        for _ in 0..100 {
            slugs.insert(generate_slug(&mut rng));
        }

        // With 100+ adjectives and 100+ nouns, we should get many unique slugs
        assert!(slugs.len() > 50, "Expected variety in generated slugs");
    }

    #[test]
    fn test_generate_unique_slug_avoids_existing() {
        // First, generate a slug to know what the seeded RNG will produce
        let first_slug = generate_slug(&mut rand::rngs::StdRng::seed_from_u64(12345));

        // Create a set with that slug
        let mut existing = HashSet::new();
        existing.insert(first_slug.clone());

        // Reset the RNG with the same seed
        let mut rng = rand::rngs::StdRng::seed_from_u64(12345);

        // Should generate a different slug since the first one is taken
        let result = generate_unique_slug(&mut rng, &existing, 100);

        assert!(result.is_some());
        let new_slug = result.unwrap();
        assert_ne!(new_slug, first_slug);
        assert!(!existing.contains(&new_slug));
    }

    #[test]
    fn test_generate_unique_slug_returns_none_when_exhausted() {
        // Create a "full" set (we'll fake this by having max_attempts = 1
        // and making sure the first generated slug is in the set)
        let first_slug = generate_slug(&mut rand::rngs::StdRng::seed_from_u64(12345));
        let mut existing = HashSet::new();
        existing.insert(first_slug);

        // With only 1 attempt and that slug taken, should return None
        let result =
            generate_unique_slug(&mut rand::rngs::StdRng::seed_from_u64(12345), &existing, 1);

        assert!(result.is_none());
    }

    #[test]
    fn test_generate_unique_slug_empty_existing() {
        let mut rng = test_rng();
        let existing = HashSet::new();

        let result = generate_unique_slug(&mut rng, &existing, 100);

        assert!(result.is_some());
    }

    #[test]
    fn test_is_valid_slug_accepts_valid() {
        assert!(is_valid_slug("quiet-mountain"));
        assert!(is_valid_slug("fuzzy-walrus"));
        assert!(is_valid_slug("a-b"));
    }

    #[test]
    fn test_is_valid_slug_rejects_uppercase() {
        assert!(!is_valid_slug("Quiet-mountain"));
        assert!(!is_valid_slug("quiet-Mountain"));
        assert!(!is_valid_slug("QUIET-MOUNTAIN"));
    }

    #[test]
    fn test_is_valid_slug_rejects_wrong_hyphen_count() {
        assert!(!is_valid_slug("nohyphen"));
        assert!(!is_valid_slug("too-many-hyphens"));
        assert!(!is_valid_slug("also-too-many-parts"));
    }

    #[test]
    fn test_is_valid_slug_rejects_empty_parts() {
        assert!(!is_valid_slug("-mountain"));
        assert!(!is_valid_slug("quiet-"));
        assert!(!is_valid_slug("-"));
    }

    #[test]
    fn test_is_valid_slug_rejects_non_alpha() {
        assert!(!is_valid_slug("quiet-mountain123"));
        assert!(!is_valid_slug("quiet123-mountain"));
        assert!(!is_valid_slug("quiet_mountain"));
    }

    #[test]
    fn test_word_list_minimums() {
        // Acceptance criteria: at least 50 adjectives and 50 nouns
        assert!(
            ADJECTIVES.len() >= 50,
            "Expected at least 50 adjectives, got {}",
            ADJECTIVES.len()
        );
        assert!(
            NOUNS.len() >= 50,
            "Expected at least 50 nouns, got {}",
            NOUNS.len()
        );
    }

    #[test]
    fn test_all_adjectives_lowercase() {
        for adj in ADJECTIVES {
            assert_eq!(
                *adj,
                adj.to_lowercase(),
                "Adjective '{}' is not lowercase",
                adj
            );
        }
    }

    #[test]
    fn test_all_nouns_lowercase() {
        for noun in NOUNS {
            assert_eq!(
                *noun,
                noun.to_lowercase(),
                "Noun '{}' is not lowercase",
                noun
            );
        }
    }

    #[test]
    fn test_total_slug_combinations() {
        let total = total_slug_combinations();
        // With 100+ adjectives and 100+ nouns, should have 10,000+ combinations
        assert!(
            total >= 10000,
            "Expected at least 10000 combinations, got {}",
            total
        );
    }

    #[test]
    fn test_generated_slugs_are_valid() {
        let mut rng = rand::thread_rng();

        for _ in 0..100 {
            let slug = generate_slug(&mut rng);
            assert!(
                is_valid_slug(&slug),
                "Generated slug '{}' is not valid",
                slug
            );
        }
    }

    // =========================================================================
    // Session Management Tests
    // =========================================================================

    #[test]
    fn test_session_outcome_default() {
        assert_eq!(SessionOutcome::default(), SessionOutcome::InProgress);
    }

    #[test]
    fn test_session_outcome_display() {
        assert_eq!(SessionOutcome::InProgress.to_string(), "in_progress");
        assert_eq!(SessionOutcome::Completed.to_string(), "completed");
        assert_eq!(SessionOutcome::Aborted.to_string(), "aborted");
        assert_eq!(SessionOutcome::Failed.to_string(), "failed");
        assert_eq!(SessionOutcome::Interrupted.to_string(), "interrupted");
    }

    #[test]
    fn test_session_outcome_serialization() {
        // Test via a wrapper struct since TOML requires table context
        #[derive(Serialize, Deserialize)]
        struct Wrapper {
            outcome: SessionOutcome,
        }

        // Serialize
        let wrapper = Wrapper {
            outcome: SessionOutcome::InProgress,
        };
        let serialized = toml::to_string(&wrapper).unwrap();
        assert!(serialized.contains("in_progress"));

        // Deserialize
        let parsed: Wrapper = toml::from_str("outcome = \"completed\"").unwrap();
        assert_eq!(parsed.outcome, SessionOutcome::Completed);
    }

    #[test]
    fn test_session_entry_new() {
        let entry = SessionEntry::new("quiet-mountain".to_string(), PathBuf::from("/test/project"));

        assert_eq!(entry.slug, "quiet-mountain");
        assert_eq!(entry.project, PathBuf::from("/test/project"));
        assert_eq!(entry.iterations, 0);
        assert_eq!(entry.outcome, SessionOutcome::InProgress);
        assert!(entry.completed_at.is_none());
    }

    #[test]
    fn test_sessions_index_empty() {
        let index = SessionsIndex::new();
        assert!(index.sessions.is_empty());
        assert!(index.existing_slugs().is_empty());
    }

    #[test]
    fn test_sessions_index_add_and_find() {
        let mut index = SessionsIndex::new();
        let entry = SessionEntry::new("fuzzy-walrus".to_string(), PathBuf::from("/test"));

        index.add_session(entry);

        assert!(index.slug_exists("fuzzy-walrus"));
        assert!(!index.slug_exists("quiet-mountain"));
        assert!(index.find_by_slug("fuzzy-walrus").is_some());
        assert!(index.find_by_slug("nonexistent").is_none());
    }

    #[test]
    fn test_sessions_index_existing_slugs() {
        let mut index = SessionsIndex::new();
        index.add_session(SessionEntry::new(
            "one-fish".to_string(),
            PathBuf::from("/a"),
        ));
        index.add_session(SessionEntry::new(
            "two-fish".to_string(),
            PathBuf::from("/b"),
        ));

        let slugs = index.existing_slugs();
        assert_eq!(slugs.len(), 2);
        assert!(slugs.contains("one-fish"));
        assert!(slugs.contains("two-fish"));
    }

    #[test]
    fn test_sessions_index_toml_roundtrip() {
        let mut index = SessionsIndex::new();
        index.add_session(SessionEntry::new(
            "quiet-mountain".to_string(),
            PathBuf::from("/test/project"),
        ));

        let toml_str = index.to_toml().unwrap();
        let parsed = SessionsIndex::from_toml(&toml_str).unwrap();

        assert_eq!(parsed.sessions.len(), 1);
        assert_eq!(parsed.sessions[0].slug, "quiet-mountain");
    }

    #[test]
    fn test_session_metadata_new() {
        let meta = SessionMetadata::new(
            "bright-river".to_string(),
            PathBuf::from("/my/project"),
            None,
        );

        assert_eq!(meta.slug, "bright-river");
        assert_eq!(meta.project, PathBuf::from("/my/project"));
        assert_eq!(meta.iterations, 0);
        assert_eq!(meta.outcome, SessionOutcome::InProgress);
        assert_eq!(meta.prompt, None);
    }

    #[test]
    fn test_session_metadata_new_with_prompt() {
        let meta = SessionMetadata::new(
            "sunny-day".to_string(),
            PathBuf::from("/my/project"),
            Some("Test prompt content".to_string()),
        );

        assert_eq!(meta.slug, "sunny-day");
        assert_eq!(meta.prompt, Some("Test prompt content".to_string()));
    }

    #[test]
    fn test_session_metadata_toml_roundtrip() {
        let meta = SessionMetadata::new("calm-ocean".to_string(), PathBuf::from("/test"), None);

        let toml_str = meta.to_toml().unwrap();
        let parsed = SessionMetadata::from_toml(&toml_str).unwrap();

        assert_eq!(parsed.slug, "calm-ocean");
        assert_eq!(parsed.project, PathBuf::from("/test"));
        assert_eq!(parsed.prompt, None);
    }

    #[test]
    fn test_session_metadata_prompt_toml_roundtrip() {
        let prompt = "Work on the highest priority feature.\n\nContext:\n- PRD: path/to/prd.toml"
            .to_string();
        let meta = SessionMetadata::new(
            "swift-wind".to_string(),
            PathBuf::from("/test"),
            Some(prompt.clone()),
        );

        let toml_str = meta.to_toml().unwrap();
        // Verify prompt appears in serialized TOML
        assert!(toml_str.contains("prompt = "));
        assert!(toml_str.contains("Work on the highest priority feature"));

        let parsed = SessionMetadata::from_toml(&toml_str).unwrap();
        assert_eq!(parsed.prompt, Some(prompt));
    }

    #[test]
    fn test_session_metadata_prompt_not_serialized_when_none() {
        let meta = SessionMetadata::new("clear-sky".to_string(), PathBuf::from("/test"), None);

        let toml_str = meta.to_toml().unwrap();
        // Verify prompt is NOT in serialized TOML when None
        assert!(
            !toml_str.contains("prompt"),
            "prompt field should not appear when None"
        );
    }

    #[test]
    fn test_session_metadata_from_entry() {
        let entry = SessionEntry::new("gentle-breeze".to_string(), PathBuf::from("/project"));
        let meta = SessionMetadata::from(&entry);

        assert_eq!(meta.slug, entry.slug);
        assert_eq!(meta.project, entry.project);
        assert_eq!(meta.iterations, entry.iterations);
        assert_eq!(meta.outcome, entry.outcome);
    }

    #[test]
    fn test_sessions_index_find_by_slug_mut() {
        let mut index = SessionsIndex::new();
        index.add_session(SessionEntry::new(
            "test-slug".to_string(),
            PathBuf::from("/test"),
        ));

        // Modify through mutable reference
        if let Some(entry) = index.find_by_slug_mut("test-slug") {
            entry.iterations = 5;
            entry.outcome = SessionOutcome::Completed;
        }

        // Verify modification
        let entry = index.find_by_slug("test-slug").unwrap();
        assert_eq!(entry.iterations, 5);
        assert_eq!(entry.outcome, SessionOutcome::Completed);
    }
}
