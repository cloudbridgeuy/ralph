//! Tests for stream parsing functionality.

#[path = "tests/core.rs"]
mod core_tests;

#[path = "tests/ndjson.rs"]
mod ndjson_tests;

#[path = "tests/extraction.rs"]
mod extraction_tests;

#[path = "tests/tool_invocation.rs"]
mod tool_invocation_tests;

#[path = "tests/text_accumulation.rs"]
mod text_accumulation_tests;

#[path = "tests/chunk_parsing.rs"]
mod chunk_parsing_tests;

#[path = "tests/iteration_costs.rs"]
mod iteration_costs_tests;

#[path = "tests/tool_correlation.rs"]
mod tool_correlation_tests;
