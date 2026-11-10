use super::*;
use resvg::tiny_skia::Pixmap;

const SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"90\" height=\"42\"><rect width=\"100%\" height=\"100%\" fill=\"#1e1f29\"/><text x=\"5\" y=\"20\" fill=\"#fff\" font-size=\"12\">hi</text></svg>";

#[test]
fn an_svg_rasterizes_to_a_png_of_its_own_size() {
    let png = rasterize_svg(SVG).unwrap_or_default();
    let size = Pixmap::decode_png(&png).map(|pixmap| (pixmap.width(), pixmap.height()));
    assert_eq!(size.ok(), Some((90, 42)));
}

#[test]
fn a_background_rect_fills_the_corner() {
    let png = rasterize_svg(SVG).unwrap_or_default();
    let corner = Pixmap::decode_png(&png)
        .ok()
        .and_then(|pixmap| pixmap.pixel(0, 0))
        .map(|pixel| (pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()));
    assert_eq!(corner, Some((0x1e, 0x1f, 0x29, 0xff)));
}

#[test]
fn text_that_is_not_an_svg_is_refused() {
    assert!(rasterize_svg("not svg").is_err());
}
