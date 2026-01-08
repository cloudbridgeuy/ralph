//! VerboseToolsConfig tests for StreamProcessor.
//!
//! Tests the configuration of verbose tool output settings.

use crate::highlight::ThemeConfig;
use crate::stream_processor::{StreamProcessor, VerboseToolsConfig};

#[test]
fn test_verbose_tools_config_new() {
    let config = VerboseToolsConfig::new();
    assert!(!config.is_verbose("Read"));
    assert!(!config.is_verbose("Bash"));
    assert!(!config.has_any());
    assert!(config.warnings().is_empty());
}

#[test]
fn test_verbose_tools_config_all() {
    let config = VerboseToolsConfig::all();
    assert!(config.is_verbose("Read"));
    assert!(config.is_verbose("Bash"));
    assert!(config.is_verbose("AnyTool"));
    assert!(config.has_any());
}

#[test]
fn test_verbose_tools_config_from_arg_none() {
    let config = VerboseToolsConfig::from_arg(None);
    assert!(!config.has_any());
    assert!(!config.is_verbose("Read"));
}

#[test]
fn test_verbose_tools_config_from_arg_star() {
    let config = VerboseToolsConfig::from_arg(Some("*"));
    assert!(config.has_any());
    assert!(config.is_verbose("Read"));
    assert!(config.is_verbose("AnyTool"));
}

#[test]
fn test_verbose_tools_config_from_arg_single() {
    let config = VerboseToolsConfig::from_arg(Some("grep"));
    assert!(config.has_any());
    assert!(config.is_verbose("grep"));
    assert!(config.is_verbose("Grep")); // Case insensitive
    assert!(config.is_verbose("GREP")); // Case insensitive
    assert!(!config.is_verbose("read"));
}

#[test]
fn test_verbose_tools_config_from_arg_multiple() {
    let config = VerboseToolsConfig::from_arg(Some("grep,bash,read"));
    assert!(config.has_any());
    assert!(config.is_verbose("grep"));
    assert!(config.is_verbose("Bash"));
    assert!(config.is_verbose("READ"));
    assert!(!config.is_verbose("edit"));
    assert!(config.warnings().is_empty()); // All known tools
}

#[test]
fn test_verbose_tools_config_from_arg_with_spaces() {
    let config = VerboseToolsConfig::from_arg(Some("grep, bash , read"));
    assert!(config.is_verbose("grep"));
    assert!(config.is_verbose("bash"));
    assert!(config.is_verbose("read"));
}

#[test]
fn test_verbose_tools_config_from_arg_unknown_tool() {
    let config = VerboseToolsConfig::from_arg(Some("unknowntool"));
    assert!(config.has_any());
    assert!(config.is_verbose("unknowntool"));
    assert!(!config.warnings().is_empty());
    assert!(config.warnings()[0].contains("Unknown tool name"));
    assert!(config.warnings()[0].contains("unknowntool"));
}

#[test]
fn test_verbose_tools_config_from_arg_mixed_known_unknown() {
    let config = VerboseToolsConfig::from_arg(Some("grep,foobar,bash"));
    assert!(config.is_verbose("grep"));
    assert!(config.is_verbose("foobar"));
    assert!(config.is_verbose("bash"));
    assert_eq!(config.warnings().len(), 1); // Only foobar is unknown
    assert!(config.warnings()[0].contains("foobar"));
}

#[test]
fn test_verbose_tools_config_from_arg_empty_entries() {
    let config = VerboseToolsConfig::from_arg(Some("grep,,bash"));
    assert!(config.is_verbose("grep"));
    assert!(config.is_verbose("bash"));
    // Empty entries should be ignored, not cause warnings
}

#[test]
fn test_stream_processor_with_verbose_tools() {
    let config = VerboseToolsConfig::from_arg(Some("grep,read"));
    let processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), true, true, config).unwrap();

    assert!(processor.is_tool_verbose("grep"));
    assert!(processor.is_tool_verbose("Read")); // Case insensitive
    assert!(!processor.is_tool_verbose("bash"));
}

#[test]
fn test_stream_processor_default_no_verbose_tools() {
    let processor = StreamProcessor::new();
    assert!(!processor.is_tool_verbose("Read"));
    assert!(!processor.is_tool_verbose("Bash"));
    assert!(!processor.verbose_tools_config().has_any());
}
