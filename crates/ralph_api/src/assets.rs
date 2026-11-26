pub(crate) struct Asset {
    pub path: &'static str,
    pub content_type: &'static str,
    pub bytes: &'static [u8],
}

pub(crate) const SHELL_PATH: &str = "/";

pub(crate) const APP_CSS: &str = concat!(
    include_str!("../assets/css/base.css"),
    include_str!("../assets/css/table.css"),
    include_str!("../assets/css/data.css"),
    include_str!("../assets/css/states.css"),
    include_str!("../assets/css/controls.css"),
    include_str!("../assets/css/code.css"),
    include_str!("../assets/css/tree.css"),
    include_str!("../assets/css/responsive.css"),
    include_str!("../assets/css/memory.css"),
    include_str!("../assets/css/threads.css"),
);

const APP_JS: &str = concat!(
    include_str!("../assets/threads.js"),
    include_str!("../assets/app.js"),
);

const ASSETS: &[Asset] = &[
    Asset {
        path: SHELL_PATH,
        content_type: "text/html; charset=utf-8",
        bytes: include_bytes!("../assets/index.html"),
    },
    Asset {
        path: "/assets/app.css",
        content_type: "text/css; charset=utf-8",
        bytes: APP_CSS.as_bytes(),
    },
    Asset {
        path: "/app.js",
        content_type: "text/javascript; charset=utf-8",
        bytes: APP_JS.as_bytes(),
    },
    Asset {
        path: "/assets/geist-mono.woff2",
        content_type: "font/woff2",
        bytes: include_bytes!("../assets/fonts/geist-mono.woff2"),
    },
];

pub(crate) fn find(path: &str) -> Option<&'static Asset> {
    ASSETS.iter().find(|asset| asset.path == path)
}

#[cfg(test)]
mod tests;
