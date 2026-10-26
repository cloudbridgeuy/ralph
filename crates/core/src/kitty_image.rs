use std::io::Write;
use std::ops::Range;

use flate2::write::ZlibEncoder;
use flate2::Compression;
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
const MAX_PIXELS: u64 = 25_165_824;
const MAX_WEBP_PIXELS: u64 = 6_291_456;
const MAX_SIDE_PX: u32 = 2048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KittyImage {
    pub size: (u32, u32),
    pub keys: String,
    pub payload: Vec<u8>,
}

struct Pixels {
    width: u32,
    height: u32,
    channels: usize,
    data: Vec<u8>,
}

pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || bytes[..8] != PNG_SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    (width > 0 && height > 0).then_some((width, height))
}

pub fn prepare(bytes: &[u8]) -> Option<KittyImage> {
    if let Some(size) = png_size(bytes) {
        return Some(KittyImage {
            size,
            keys: "f=100,".to_string(),
            payload: bytes.to_vec(),
        });
    }
    let pixels = if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        decode_jpeg(bytes)?
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        decode_gif(bytes)?
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        decode_webp(bytes)?
    } else {
        return None;
    };
    encode_pixels(pixels)
}

fn within_cap(width: u32, height: u32, cap: u64) -> bool {
    let pixels = u64::from(width) * u64::from(height);
    pixels > 0 && pixels <= cap
}

fn decode_jpeg(bytes: &[u8]) -> Option<Pixels> {
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    decoder.decode_headers().ok()?;
    let info = decoder.info()?;
    let (width, height) = (u32::from(info.width), u32::from(info.height));
    if !within_cap(width, height, MAX_PIXELS) {
        return None;
    }
    Some(Pixels {
        width,
        height,
        channels: 4,
        data: decoder.decode().ok()?,
    })
}

fn decode_gif(bytes: &[u8]) -> Option<Pixels> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(bytes).ok()?;
    if !within_cap(
        u32::from(decoder.width()),
        u32::from(decoder.height()),
        MAX_PIXELS,
    ) {
        return None;
    }
    let frame = decoder.read_next_frame().ok()??;
    Some(Pixels {
        width: u32::from(frame.width),
        height: u32::from(frame.height),
        channels: 4,
        data: frame.buffer.to_vec(),
    })
}

fn decode_webp(bytes: &[u8]) -> Option<Pixels> {
    let mut decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    let (width, height) = decoder.dimensions();
    if !within_cap(width, height, MAX_WEBP_PIXELS) {
        return None;
    }
    let channels = if decoder.has_alpha() { 4 } else { 3 };
    let mut data = vec![0u8; decoder.output_buffer_size()?];
    decoder.read_image(&mut data).ok()?;
    Some(Pixels {
        width,
        height,
        channels,
        data,
    })
}

fn span(out_index: usize, out_len: usize, in_len: usize) -> Range<usize> {
    let start = out_index * in_len / out_len;
    let end = ((out_index + 1) * in_len / out_len).max(start + 1);
    start..end
}

fn scaled_side(side: u32, long: u32) -> u32 {
    let scaled = (u64::from(side) * u64::from(MAX_SIDE_PX) + u64::from(long) / 2) / u64::from(long);
    u32::try_from(scaled).unwrap_or(1).max(1)
}

fn box_average(pixels: &Pixels, stride: usize, xs: Range<usize>, ys: Range<usize>) -> [u8; 4] {
    let mut sums = [0u64; 3];
    let (mut weight_sum, mut count) = (0u64, 0u64);
    for y in ys {
        for x in xs.clone() {
            let start = (y * stride + x) * pixels.channels;
            let pixel = &pixels.data[start..start + pixels.channels];
            let weight = if pixels.channels == 4 {
                u64::from(pixel[3])
            } else {
                1
            };
            for (sum, value) in sums.iter_mut().zip(pixel) {
                *sum += u64::from(*value) * weight;
            }
            weight_sum += weight;
            count += 1;
        }
    }
    let channel = |sum: u64| {
        let value = (sum + weight_sum / 2).checked_div(weight_sum).unwrap_or(0);
        u8::try_from(value).unwrap_or(u8::MAX)
    };
    let alpha = u8::try_from((weight_sum + count / 2) / count).unwrap_or(u8::MAX);
    [channel(sums[0]), channel(sums[1]), channel(sums[2]), alpha]
}

fn downscale(pixels: Pixels) -> Option<Pixels> {
    let long = pixels.width.max(pixels.height);
    if long <= MAX_SIDE_PX {
        return Some(pixels);
    }
    let width = scaled_side(pixels.width, long);
    let height = scaled_side(pixels.height, long);
    let stride = usize::try_from(pixels.width).ok()?;
    let (in_height, out_width, out_height) = (
        usize::try_from(pixels.height).ok()?,
        usize::try_from(width).ok()?,
        usize::try_from(height).ok()?,
    );
    let mut data = Vec::with_capacity(out_width * out_height * pixels.channels);
    for oy in 0..out_height {
        for ox in 0..out_width {
            let average = box_average(
                &pixels,
                stride,
                span(ox, out_width, stride),
                span(oy, out_height, in_height),
            );
            data.extend_from_slice(&average[..pixels.channels]);
        }
    }
    Some(Pixels {
        width,
        height,
        channels: pixels.channels,
        data,
    })
}

fn encode_pixels(pixels: Pixels) -> Option<KittyImage> {
    let expected =
        usize::try_from(u64::from(pixels.width) * u64::from(pixels.height)).ok()? * pixels.channels;
    if !within_cap(pixels.width, pixels.height, MAX_PIXELS) || pixels.data.len() != expected {
        return None;
    }
    let Pixels {
        width,
        height,
        channels,
        data,
    } = downscale(pixels)?;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&data).ok()?;
    let format = if channels == 4 { 32 } else { 24 };
    Some(KittyImage {
        size: (width, height),
        keys: format!("f={format},s={width},v={height},o=z,"),
        payload: encoder.finish().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use flate2::read::ZlibDecoder;

    use super::*;

    const JPEG: &str = "ffd8ffe000104a46494600010100000100010000ffdb004300030202020202030202020303030304060404040404080606050609080a0a090809090a0c0f0c0a0b0e0b09090d110d0e0f101011100a0c12131210130f101010ffdb00430103030304030408040408100b090b1010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010ffc00011080002000203011100021101031101ffc40014000100000000000000000000000000000008ffc4001a100003000301000000000000000000000001020300113141ffc4001501010100000000000000000000000000000709ffc4001a110101000301010000000000000000000001020304211100ffda000c03010002110311003f00394e738cd6524544401555468281c0078304f6b6b3ef67bdadab6f25ad5552b554beb54beaaafaaf57afd5375b5b0e9e18d7d78231c04ccc813321e001c00e01c0fbffd9";
    const GIF: &str = "47494638396103000300f00000ff000000000021ff0b4e45545343415045322e30030100000021f904000a0000002c0000000003000300000203847f050021f904000a0000002c0000000003000300800000ff0000000203847f05003b";
    const WEBP_RGB: &str = "524946463c0000005745425056503820300000007001009d012a0200020002003425a0027401400000fef0c4afe8c24fc5f098f456bffda59fffda59fffda59fe2a00000";
    const WEBP_RGBA: &str = "524946465c00000057454250565038580a00000010000000010000010000414c5048050000000080000000005650382030000000d001009d012a0200020002003425a00274ba01f80003b000fef0c40bff20b96175c8d7ff203fe407fc80fff8f2000000";

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .filter_map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
            .collect()
    }

    fn inflate(image: &KittyImage) -> Vec<u8> {
        let mut out = Vec::new();
        ZlibDecoder::new(image.payload.as_slice())
            .read_to_end(&mut out)
            .unwrap();
        out
    }

    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes
    }

    #[test]
    fn png_size_reads_ihdr() {
        assert_eq!(png_size(&png_header(640, 480)), Some((640, 480)));
        assert_eq!(png_size(&png_header(0, 480)), None);
        assert_eq!(png_size(&png_header(1, 1)[..20]), None);
        assert_eq!(png_size(b"GIF89a................................"), None);
    }

    #[test]
    fn png_passes_through_unchanged() {
        let bytes = png_header(640, 480);
        let image = prepare(&bytes).unwrap();
        assert_eq!(image.size, (640, 480));
        assert_eq!(image.keys, "f=100,");
        assert_eq!(image.payload, bytes);
    }

    #[test]
    fn jpeg_decodes_to_compressed_rgba() {
        let image = prepare(&hex(JPEG)).unwrap();
        assert_eq!(image.size, (2, 2));
        assert_eq!(image.keys, "f=32,s=2,v=2,o=z,");
        assert_eq!(inflate(&image).len(), 16);
    }

    #[test]
    fn gif_draws_only_the_first_frame() {
        let image = prepare(&hex(GIF)).unwrap();
        assert_eq!(image.size, (3, 3));
        assert_eq!(image.keys, "f=32,s=3,v=3,o=z,");
        assert_eq!(inflate(&image), [255, 0, 0, 255].repeat(9));
    }

    #[test]
    fn webp_keeps_alpha_only_when_present() {
        let rgb = prepare(&hex(WEBP_RGB)).unwrap();
        assert_eq!((rgb.size, rgb.keys.as_str()), ((2, 2), "f=24,s=2,v=2,o=z,"));
        assert_eq!(inflate(&rgb).len(), 12);
        let rgba = prepare(&hex(WEBP_RGBA)).unwrap();
        assert_eq!(rgba.keys, "f=32,s=2,v=2,o=z,");
        assert_eq!(inflate(&rgba).len(), 16);
    }

    #[test]
    fn truncated_or_unknown_bytes_give_none() {
        for source in [JPEG, GIF, WEBP_RGB] {
            let bytes = hex(source);
            assert_eq!(prepare(&bytes[..bytes.len() / 2]), None);
        }
        assert_eq!(prepare(b"not an image at all"), None);
        assert_eq!(prepare(b""), None);
    }

    #[test]
    fn header_over_the_pixel_cap_gives_none() {
        let mut jpeg = hex(JPEG);
        let sof = jpeg.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
        jpeg[sof + 5..sof + 9].copy_from_slice(&[0x20, 0x00, 0x20, 0x00]);
        assert_eq!(prepare(&jpeg), None);
        let mut gif = hex(GIF);
        gif[6..10].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(prepare(&gif), None);
    }

    fn pixels(width: u32, height: u32, channels: usize, data: Vec<u8>) -> Pixels {
        Pixels {
            width,
            height,
            channels,
            data,
        }
    }

    fn flat(width: u32, height: u32, channels: usize) -> Pixels {
        let len = usize::try_from(width * height).unwrap() * channels;
        pixels(width, height, channels, vec![9; len])
    }

    #[test]
    fn downscale_halves_a_4096_by_2048_image() {
        let out = downscale(flat(4096, 2048, 4)).unwrap();
        assert_eq!((out.width, out.height), (2048, 1024));
        assert_eq!(out.data.len(), 2048 * 1024 * 4);
        let out = downscale(flat(2048, 4096, 3)).unwrap();
        assert_eq!(
            (out.width, out.height, out.data.len()),
            (1024, 2048, 1024 * 2048 * 3)
        );
    }

    #[test]
    fn downscale_averages_blocks_and_weights_colour_by_alpha() {
        let data = (0..4096usize * 3)
            .map(|i| if (i / 3) % 2 == 0 { 0 } else { 100 })
            .collect();
        let out = downscale(pixels(4096, 1, 3, data)).unwrap();
        assert_eq!(out.data[..3], [50, 50, 50]);
        let rgba = pixels(
            4096,
            1,
            4,
            [[200, 0, 0, 255], [0, 200, 0, 0]].concat().repeat(2048),
        );
        let out = downscale(rgba).unwrap();
        assert_eq!(out.data[..4], [200, 0, 0, 128]);
    }

    #[test]
    fn downscale_leaves_fitting_images_untouched() {
        let image = pixels(2048, 1, 3, (0..2048 * 3).map(|i| (i % 251) as u8).collect());
        let data = image.data.clone();
        assert_eq!(downscale(image).unwrap().data, data);
    }

    #[test]
    fn downscale_handles_odd_ratios_and_extreme_aspect() {
        let out = downscale(flat(3001, 2999, 4)).unwrap();
        assert_eq!((out.width, out.height), (2048, 2047));
        let out = downscale(flat(10000, 1, 3)).unwrap();
        assert_eq!((out.width, out.height, out.data.len()), (2048, 1, 2048 * 3));
        let out = downscale(flat(1, 5000, 4)).unwrap();
        assert_eq!((out.width, out.height), (1, 2048));
    }

    #[test]
    fn prepare_reports_the_downscaled_size_in_keys() {
        let image = encode_pixels(flat(4096, 2048, 4)).unwrap();
        assert_eq!(image.size, (2048, 1024));
        assert_eq!(image.keys, "f=32,s=2048,v=1024,o=z,");
    }
}
