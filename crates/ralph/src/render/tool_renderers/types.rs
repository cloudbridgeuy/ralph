//! Shared parameter types for tool rendering.

/// Configuration for Grep tool invocation display.
pub struct GrepInvocationParams<'a> {
    pub pattern: &'a str,
    pub path: Option<&'a str>,
    pub output_mode: Option<&'a str>,
    pub glob: Option<&'a str>,
    pub file_type: Option<&'a str>,
    pub case_insensitive: bool,
}

/// Item for TodoWrite display.
pub struct TodoDisplayItem<'a> {
    pub content: &'a str,
    pub status: &'a str,
    pub active_form: Option<&'a str>,
}

/// Parameters for rendering a NotebookEdit result.
pub struct NotebookEditParams<'a> {
    pub notebook_path: &'a str,
    pub cell_identifier: &'a str,
    pub cell_type: Option<&'a str>,
    pub edit_mode: &'a str,
    pub diff_content: &'a str,
}
