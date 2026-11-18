//! Tool result renderers.

mod bash;
mod default;
mod glob;
mod grep;
mod read;
mod todowrite;

pub use bash::render_bash_result;
pub use default::render_default_result;
pub use glob::render_glob_result;
pub use grep::render_grep_result;
pub use read::render_read_result;
pub use todowrite::render_todowrite_result;
