mod addr;
mod admit;
mod assets;
mod catalogue;
pub mod render;
mod route;
mod server;
mod token;
mod tokens;

pub use addr::{allowed_hosts, bind_hosts, BindError, BindPolicy};
pub use admit::MAX_HANDLERS;
pub use route::ApiConfig;
pub use server::{bind, serve, ApiError};
/// Notice for the bundled Geist Mono font: tag, source and SHA-256.
pub const FONT_NOTICE: &str = include_str!("../assets/fonts/NOTICE");
/// SIL Open Font License text shipped with Geist Mono.
pub const FONT_LICENCE: &str = include_str!("../assets/fonts/OFL.txt");

pub use token::{cookie_name, Token};
pub use tokens::{tokens_css, ColorSpec, Rgb, Slot, TokensInput};
