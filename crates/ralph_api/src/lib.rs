mod addr;
mod admit;
mod assets;
mod catalogue;
mod memory;
mod memory_api;
pub(crate) mod render;
mod render_api;
mod route;
mod server;
mod tailnet;
mod token;
mod tokens;

pub use addr::{allowed_hosts, bind_hosts, BindError, BindPolicy};
pub use admit::MAX_HANDLERS;
pub use route::ApiConfig;
pub use server::{bind, bind_with_limits, serve, ApiError, Limits};
/// Notice for the bundled Geist Mono font: tag, source and SHA-256.
pub const FONT_NOTICE: &str = include_str!("../assets/fonts/NOTICE");
/// SIL Open Font License text shipped with Geist Mono.
pub const FONT_LICENCE: &str = include_str!("../assets/fonts/OFL.txt");

pub use tailnet::{
    classify, parse_ip_output, parse_status_json, pick_outcome, tailscale_candidates, SpawnOutcome,
    TailnetError, TailnetNames,
};
pub use token::{cookie_name, Token};
pub use tokens::{tokens_css, ColorSpec, Rgb, Slot, TokensInput};
