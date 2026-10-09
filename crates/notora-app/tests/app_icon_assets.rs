use image::{DynamicImage, ImageFormat};

const MAXIMUM_EMPTY_EDGE_FRACTION: f32 = 0.03;
const VISIBLE_ALPHA_THRESHOLD: u8 = 16;

fn icns_png<'a>(icon_bytes: &'a [u8], resource_type: &[u8; 4]) -> &'a [u8] {
    assert_eq!(&icon_bytes[..4], b"icns");
    let mut offset = 8;
    while offset < icon_bytes.len() {
        let chunk_length = u32::from_be_bytes(
            icon_bytes[offset + 4..offset + 8].try_into().expect("ICNS chunk must have a length"),
        ) as usize;
        if &icon_bytes[offset..offset + 4] == resource_type {
            return &icon_bytes[offset + 8..offset + chunk_length];
        }
        offset += chunk_length;
    }
    panic!("ICNS must contain the requested PNG size");
}

fn assert_artwork_fills_canvas(image: &DynamicImage) {
    let pixels = image.to_rgba8();
    let (width, height) = pixels.dimensions();
    let mut left = width;
    let mut top = height;
    let mut right = 0;
    let mut bottom = 0;

    for (x, y, pixel) in pixels.enumerate_pixels() {
        if pixel[3] < VISIBLE_ALPHA_THRESHOLD {
            continue;
        }
        left = left.min(x);
        top = top.min(y);
        right = right.max(x);
        bottom = bottom.max(y);
    }

    let edge_margin = left.max(top).max(width - right - 1).max(height - bottom - 1);
    let allowed_margin = width as f32 * MAXIMUM_EMPTY_EDGE_FRACTION;
    assert!(edge_margin as f32 <= allowed_margin, "app icon leaves {edge_margin} empty pixels");
}

#[test]
fn notora_mac_and_windows_icons_fill_their_canvases() {
    let mac_icon = include_bytes!("../../../assets/NotoraAppIcon.icns");
    let windows_icon = include_bytes!("../../../assets/NotoraAppIcon.ico");
    let mac_artwork =
        image::load_from_memory_with_format(icns_png(mac_icon, b"ic10"), ImageFormat::Png)
            .expect("macOS icon must contain a 1024-pixel PNG");
    let mac_artwork_at_windows_size =
        image::load_from_memory_with_format(icns_png(mac_icon, b"ic08"), ImageFormat::Png)
            .expect("macOS icon must contain a 256-pixel PNG");
    let windows_artwork = image::load_from_memory_with_format(windows_icon, ImageFormat::Ico)
        .expect("Windows icon must decode");

    assert_artwork_fills_canvas(&mac_artwork);
    assert_artwork_fills_canvas(&windows_artwork);
    assert_eq!(mac_artwork_at_windows_size.to_rgba8(), windows_artwork.to_rgba8());
    assert_ne!(mac_icon.as_slice(), include_bytes!("../../../assets/AppIcon.icns").as_slice());
    let textora_windows_artwork = image::load_from_memory_with_format(
        include_bytes!("../../../assets/AppIcon.ico"),
        ImageFormat::Ico,
    )
    .expect("textora Windows icon must decode");
    assert_ne!(windows_artwork.to_rgba8(), textora_windows_artwork.to_rgba8());
}
