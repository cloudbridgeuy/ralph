pub(crate) struct Asset {
    pub path: &'static str,
    pub content_type: &'static str,
    pub bytes: &'static [u8],
}

pub(crate) const SHELL_PATH: &str = "/";

const ASSETS: &[Asset] = &[
    Asset {
        path: SHELL_PATH,
        content_type: "text/html; charset=utf-8",
        bytes: include_bytes!("../assets/index.html"),
    },
    Asset {
        path: "/assets/app.css",
        content_type: "text/css; charset=utf-8",
        bytes: include_bytes!("../assets/app.css"),
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
mod tests {
    use super::*;

    fn bytes_of(path: &str) -> &'static [u8] {
        find(path).unwrap().bytes
    }

    #[test]
    fn the_shell_is_html() {
        assert_eq!(find("/").unwrap().content_type, "text/html; charset=utf-8");
        assert!(bytes_of("/").starts_with(b"<!doctype html>"));
    }

    #[test]
    fn the_stylesheet_is_css() {
        assert_eq!(
            find("/assets/app.css").unwrap().content_type,
            "text/css; charset=utf-8"
        );
        assert!(!bytes_of("/assets/app.css").is_empty());
    }

    #[test]
    fn the_font_is_the_upstream_woff2() {
        let font = find("/assets/geist-mono.woff2").unwrap();
        assert_eq!(font.content_type, "font/woff2");
        assert!(font.bytes.starts_with(b"wOF2"));
        assert_eq!(font.bytes.len(), 71_212);
    }

    #[test]
    fn unknown_and_traversal_paths_find_nothing() {
        for path in [
            "/nope",
            "/assets/",
            "/assets/../index.html",
            "/assets/app.css/",
            "",
            "/index.html",
        ] {
            assert!(find(path).is_none(), "{path}");
        }
    }

    #[test]
    fn every_path_is_unique_and_absolute() {
        assert!(!ASSETS.is_empty());
        for (at, asset) in ASSETS.iter().enumerate() {
            assert!(asset.path.starts_with('/'), "{}", asset.path);
            assert!(!asset.bytes.is_empty(), "{}", asset.path);
            assert!(ASSETS[at + 1..]
                .iter()
                .all(|other| other.path != asset.path));
        }
    }
}
