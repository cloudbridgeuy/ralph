//! Startup information display for the run command.
//!
//! This module provides functions to display startup information when
//! `ralph run` begins, giving users immediate feedback about the session
//! being created and the work to be done.

use std::io::IsTerminal;
use std::path::PathBuf;

/// Version of the ralph binary (from Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Information to display at startup.
#[derive(Debug)]
pub struct StartupInfo {
    /// The session slug (generated or user-provided).
    pub slug: String,
    /// Total number of stories in the PRD.
    pub total_stories: usize,
    /// Number of pending stories.
    pub pending_stories: usize,
    /// Number of completed stories.
    pub completed_stories: usize,
    /// Maximum iterations to run.
    pub max_iterations: usize,
    /// Whether max_iterations was explicitly provided by user.
    pub iterations_from_arg: bool,
    /// Path to PRD file (only shown if custom).
    pub custom_prd_path: Option<PathBuf>,
    /// Path to design file (only shown if custom).
    pub custom_design_path: Option<PathBuf>,
    /// Path to progress file (only shown if custom).
    pub custom_progress_path: Option<PathBuf>,
    /// Whether a custom command template is used.
    pub custom_command: bool,
    /// Whether a custom prompt is used.
    pub custom_prompt: bool,
    /// Whether a custom completion marker is used.
    pub custom_completion_marker: bool,
    /// Session directory path.
    pub session_dir: PathBuf,
}

impl StartupInfo {
    /// Check if any custom configuration is being used.
    fn has_custom_config(&self) -> bool {
        self.custom_prd_path.is_some()
            || self.custom_design_path.is_some()
            || self.custom_progress_path.is_some()
            || self.custom_command
            || self.custom_prompt
            || self.custom_completion_marker
    }
}

/// Display startup information to stdout.
///
/// The output format adapts based on whether stdout is a terminal:
/// - Terminal: Uses box drawing characters and colors
/// - Piped: Uses plain ASCII with no ANSI codes
pub fn display_startup_info(info: &StartupInfo) {
    let is_terminal = std::io::stdout().is_terminal();

    if is_terminal {
        display_startup_terminal(info);
    } else {
        display_startup_plain(info);
    }
}

/// Display startup info with terminal formatting.
fn display_startup_terminal(info: &StartupInfo) {
    // Header with version
    println!();
    println!("\x1b[1m\x1b[36m━━━ ralph v{} ━━━\x1b[0m", VERSION);
    println!();

    // Session info
    println!("\x1b[1mSession:\x1b[0m \x1b[33m{}\x1b[0m", info.slug);

    // PRD status
    println!(
        "\x1b[1mPRD:\x1b[0m {} pending / {} total ({} completed)",
        info.pending_stories, info.total_stories, info.completed_stories
    );

    // Iterations
    let iterations_note = if info.iterations_from_arg {
        "(from argument)"
    } else {
        "(auto: pending count)"
    };
    println!(
        "\x1b[1mIterations:\x1b[0m up to {} {}",
        info.max_iterations, iterations_note
    );

    // Custom config (only if any overrides present)
    if info.has_custom_config() {
        println!();
        println!("\x1b[2mCustom configuration:\x1b[0m");
        if let Some(ref path) = info.custom_prd_path {
            println!("  \x1b[2m--prd {}\x1b[0m", path.display());
        }
        if let Some(ref path) = info.custom_design_path {
            println!("  \x1b[2m--design {}\x1b[0m", path.display());
        }
        if let Some(ref path) = info.custom_progress_path {
            println!("  \x1b[2m--progress {}\x1b[0m", path.display());
        }
        if info.custom_command {
            println!("  \x1b[2m--command (custom)\x1b[0m");
        }
        if info.custom_prompt {
            println!("  \x1b[2m--prompt (custom)\x1b[0m");
        }
        if info.custom_completion_marker {
            println!("  \x1b[2m--completion-marker (custom)\x1b[0m");
        }
    }

    // Session directory
    println!();
    println!("\x1b[2mLogs: {}\x1b[0m", info.session_dir.display());

    // Separator before first iteration
    println!();
    println!("\x1b[36m{}\x1b[0m", "─".repeat(60));
    println!();
}

/// Display startup info without terminal formatting.
fn display_startup_plain(info: &StartupInfo) {
    // Header with version
    println!();
    println!("=== ralph v{} ===", VERSION);
    println!();

    // Session info
    println!("Session: {}", info.slug);

    // PRD status
    println!(
        "PRD: {} pending / {} total ({} completed)",
        info.pending_stories, info.total_stories, info.completed_stories
    );

    // Iterations
    let iterations_note = if info.iterations_from_arg {
        "(from argument)"
    } else {
        "(auto: pending count)"
    };
    println!(
        "Iterations: up to {} {}",
        info.max_iterations, iterations_note
    );

    // Custom config (only if any overrides present)
    if info.has_custom_config() {
        println!();
        println!("Custom configuration:");
        if let Some(ref path) = info.custom_prd_path {
            println!("  --prd {}", path.display());
        }
        if let Some(ref path) = info.custom_design_path {
            println!("  --design {}", path.display());
        }
        if let Some(ref path) = info.custom_progress_path {
            println!("  --progress {}", path.display());
        }
        if info.custom_command {
            println!("  --command (custom)");
        }
        if info.custom_prompt {
            println!("  --prompt (custom)");
        }
        if info.custom_completion_marker {
            println!("  --completion-marker (custom)");
        }
    }

    // Session directory
    println!();
    println!("Logs: {}", info.session_dir.display());

    // Separator before first iteration
    println!();
    println!("{}", "-".repeat(60));
    println!();
}

/// Information for iteration header display.
#[derive(Debug)]
pub struct IterationHeader {
    /// Current iteration number (1-indexed).
    pub iteration: usize,
    /// Maximum iterations, if known.
    pub max_iterations: Option<usize>,
    /// Number of pending stories at the start of this iteration.
    pub pending_stories: usize,
}

/// Display iteration header to stdout.
///
/// The output format adapts based on whether stdout is a terminal:
/// - Terminal: Uses box drawing characters and colors
/// - Piped: Uses plain ASCII with no ANSI codes
pub fn display_iteration_header(header: &IterationHeader) {
    let is_terminal = std::io::stdout().is_terminal();

    if is_terminal {
        display_iteration_header_terminal(header);
    } else {
        display_iteration_header_plain(header);
    }
}

/// Display iteration header with terminal formatting.
fn display_iteration_header_terminal(header: &IterationHeader) {
    // Iteration indicator
    let iteration_text = match header.max_iterations {
        Some(max) => format!("Iteration {}/{}", header.iteration, max),
        None => format!("Iteration {}", header.iteration),
    };

    // Story count
    let stories_text = if header.pending_stories == 1 {
        "1 story remaining".to_string()
    } else {
        format!("{} stories remaining", header.pending_stories)
    };

    // Print header with visual separator
    println!();
    println!(
        "\x1b[1m\x1b[34m━━━ {} • {} ━━━\x1b[0m",
        iteration_text, stories_text
    );
    println!();
}

/// Display iteration header without terminal formatting.
fn display_iteration_header_plain(header: &IterationHeader) {
    // Iteration indicator
    let iteration_text = match header.max_iterations {
        Some(max) => format!("Iteration {}/{}", header.iteration, max),
        None => format!("Iteration {}", header.iteration),
    };

    // Story count
    let stories_text = if header.pending_stories == 1 {
        "1 story remaining".to_string()
    } else {
        format!("{} stories remaining", header.pending_stories)
    };

    // Print header with visual separator
    println!();
    println!("--- {} | {} ---", iteration_text, stories_text);
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_info() -> StartupInfo {
        StartupInfo {
            slug: "test-session".to_string(),
            total_stories: 10,
            pending_stories: 5,
            completed_stories: 5,
            max_iterations: 5,
            iterations_from_arg: false,
            custom_prd_path: None,
            custom_design_path: None,
            custom_progress_path: None,
            custom_command: false,
            custom_prompt: false,
            custom_completion_marker: false,
            session_dir: PathBuf::from("/home/user/.config/ralph/sessions/test-session"),
        }
    }

    #[test]
    fn test_startup_info_creation() {
        let info = create_test_info();
        assert_eq!(info.slug, "test-session");
        assert_eq!(info.pending_stories, 5);
        assert_eq!(info.total_stories, 10);
        assert_eq!(info.completed_stories, 5);
    }

    #[test]
    fn test_has_custom_config_none() {
        let info = create_test_info();
        assert!(!info.has_custom_config());
    }

    #[test]
    fn test_has_custom_config_prd() {
        let mut info = create_test_info();
        info.custom_prd_path = Some(PathBuf::from("/custom/prd.toml"));
        assert!(info.has_custom_config());
    }

    #[test]
    fn test_has_custom_config_design() {
        let mut info = create_test_info();
        info.custom_design_path = Some(PathBuf::from("/custom/design.md"));
        assert!(info.has_custom_config());
    }

    #[test]
    fn test_has_custom_config_progress() {
        let mut info = create_test_info();
        info.custom_progress_path = Some(PathBuf::from("/custom/progress.txt"));
        assert!(info.has_custom_config());
    }

    #[test]
    fn test_has_custom_config_command() {
        let mut info = create_test_info();
        info.custom_command = true;
        assert!(info.has_custom_config());
    }

    #[test]
    fn test_has_custom_config_prompt() {
        let mut info = create_test_info();
        info.custom_prompt = true;
        assert!(info.has_custom_config());
    }

    #[test]
    fn test_has_custom_config_completion_marker() {
        let mut info = create_test_info();
        info.custom_completion_marker = true;
        assert!(info.has_custom_config());
    }

    #[test]
    fn test_has_custom_config_multiple() {
        let mut info = create_test_info();
        info.custom_command = true;
        info.custom_prompt = true;
        info.custom_prd_path = Some(PathBuf::from("/custom/prd.toml"));
        assert!(info.has_custom_config());
    }

    #[test]
    fn test_iterations_from_arg_flag() {
        let mut info = create_test_info();
        assert!(!info.iterations_from_arg);

        info.iterations_from_arg = true;
        assert!(info.iterations_from_arg);
    }

    // Tests for IterationHeader

    fn create_test_header() -> IterationHeader {
        IterationHeader {
            iteration: 1,
            max_iterations: Some(5),
            pending_stories: 3,
        }
    }

    #[test]
    fn test_iteration_header_creation() {
        let header = create_test_header();
        assert_eq!(header.iteration, 1);
        assert_eq!(header.max_iterations, Some(5));
        assert_eq!(header.pending_stories, 3);
    }

    #[test]
    fn test_iteration_header_without_max() {
        let header = IterationHeader {
            iteration: 2,
            max_iterations: None,
            pending_stories: 5,
        };
        assert_eq!(header.iteration, 2);
        assert!(header.max_iterations.is_none());
        assert_eq!(header.pending_stories, 5);
    }

    #[test]
    fn test_iteration_header_singular_story() {
        let header = IterationHeader {
            iteration: 1,
            max_iterations: Some(3),
            pending_stories: 1,
        };
        assert_eq!(header.pending_stories, 1);
    }

    #[test]
    fn test_iteration_header_zero_stories() {
        // Edge case: 0 stories remaining (shouldn't normally happen but should handle it)
        let header = IterationHeader {
            iteration: 5,
            max_iterations: Some(5),
            pending_stories: 0,
        };
        assert_eq!(header.pending_stories, 0);
    }

    #[test]
    fn test_iteration_header_large_numbers() {
        let header = IterationHeader {
            iteration: 100,
            max_iterations: Some(1000),
            pending_stories: 500,
        };
        assert_eq!(header.iteration, 100);
        assert_eq!(header.max_iterations, Some(1000));
        assert_eq!(header.pending_stories, 500);
    }
}
