//! Session slug generation and validation.
//!
//! This module provides pure functions for generating memorable session slugs
//! in the format "adjective-noun" (e.g., "quiet-mountain", "fuzzy-walrus").
//! Following the Functional Core pattern, uniqueness checks against existing
//! sessions happen at the shell layer.

use rand::seq::SliceRandom;
use rand::Rng;

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
pub fn generate_slug<R: Rng + ?Sized>(rng: &mut R) -> String {
    let adjective = ADJECTIVES
        .choose(rng)
        .expect("adjectives list is not empty");
    let noun = NOUNS.choose(rng).expect("nouns list is not empty");
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
}
