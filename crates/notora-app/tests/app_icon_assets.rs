use image::{DynamicImage, ImageFormat};

const ICON_DIRECTORY_HEADER_BYTES: usize = 6;
const ICON_DIRECTORY_ENTRY_BYTES: usize = 16;
const MAX_ANTIALIASED_CORNER_OPACITY: u8 = 64;
const MONOGRAM_SAMPLE_COLOR_TOLERANCE: u8 = 2;
const MONOGRAM_SAMPLE_POSITIONS: [(u32, u32); 2] = [(96, 128), (160, 128)];

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
    for (x, y, pixel) in pixels.enumerate_pixels() {
        assert_eq!(
            pixel[3],
            u8::MAX,
            "app icon must fill pixel ({x}, {y}); transparency triggers a macOS backdrop"
        );
    }
}

fn assert_notebook_outline_is_rounded(image: &DynamicImage) {
    let pixels = image.to_rgba8();
    let last_column = pixels.width() - 1;
    let last_row = pixels.height() - 1;
    for (x, y) in [(0, 0), (last_column, 0), (0, last_row), (last_column, last_row)] {
        assert!(
            pixels.get_pixel(x, y)[3] <= MAX_ANTIALIASED_CORNER_OPACITY,
            "Windows must show the notebook's complete rounded outline at ({x}, {y})"
        );
    }
    let center_column = pixels.width() / 2;
    let center_row = pixels.height() / 2;
    for (x, y) in [
        (center_column, 0),
        (center_column, last_row),
        (0, center_row),
        (last_column, center_row),
        (center_column, center_row),
    ] {
        assert_eq!(pixels.get_pixel(x, y)[3], u8::MAX, "notebook must fill its bounds");
    }
}

fn assert_mac_icon_sizes_fill(icon_bytes: &[u8]) {
    for (resource_type, expected_size) in [
        (b"icp4", 16),
        (b"icp5", 32),
        (b"ic12", 64),
        (b"ic07", 128),
        (b"ic08", 256),
        (b"ic09", 512),
        (b"ic10", 1024),
        (b"ic11", 32),
        (b"ic13", 256),
        (b"ic14", 512),
    ] {
        let artwork = image::load_from_memory_with_format(
            icns_png(icon_bytes, resource_type),
            ImageFormat::Png,
        )
        .expect("macOS icon size must decode");
        assert_eq!(artwork.width(), expected_size);
        assert_eq!(artwork.height(), expected_size);
        assert_artwork_fills_canvas(&artwork);
    }
}

fn assert_windows_icon_sizes_preserve_notebook_outline(icon_bytes: &[u8]) {
    let expected_sizes = [16, 24, 32, 48, 64, 128, 256];
    let image_count = u16::from_le_bytes([icon_bytes[4], icon_bytes[5]]) as usize;
    assert_eq!(image_count, expected_sizes.len());

    for (index, expected_size) in expected_sizes.into_iter().enumerate() {
        let entry = ICON_DIRECTORY_HEADER_BYTES + index * ICON_DIRECTORY_ENTRY_BYTES;
        let length = u32::from_le_bytes(
            icon_bytes[entry + 8..entry + 12].try_into().expect("ICO image length"),
        ) as usize;
        let start = u32::from_le_bytes(
            icon_bytes[entry + 12..entry + 16].try_into().expect("ICO image offset"),
        ) as usize;
        let artwork = image::load_from_memory_with_format(
            &icon_bytes[start..start + length],
            ImageFormat::Png,
        )
        .expect("Windows icon size must decode");
        assert_eq!(artwork.width(), expected_size);
        assert_eq!(artwork.height(), expected_size);
        assert_notebook_outline_is_rounded(&artwork);
    }
}

#[test]
fn notora_mac_and_windows_icons_use_platform_appropriate_outlines() {
    let mac_icon = include_bytes!("../../../assets/NotoraAppIcon.icns");
    let windows_icon = include_bytes!("../../../assets/NotoraAppIcon.ico");
    assert_mac_icon_sizes_fill(mac_icon);
    assert_windows_icon_sizes_preserve_notebook_outline(windows_icon);

    let mac_artwork_at_windows_size =
        image::load_from_memory_with_format(icns_png(mac_icon, b"ic08"), ImageFormat::Png)
            .expect("macOS icon must contain a 256-pixel PNG");
    let windows_artwork = image::load_from_memory_with_format(windows_icon, ImageFormat::Ico)
        .expect("Windows icon must decode");

    let mac_pixels = mac_artwork_at_windows_size.to_rgba8();
    let windows_pixels = windows_artwork.to_rgba8();
    for (x, y) in MONOGRAM_SAMPLE_POSITIONS {
        for (mac_channel, windows_channel) in
            mac_pixels.get_pixel(x, y).0.into_iter().zip(windows_pixels.get_pixel(x, y).0)
        {
            assert!(
                mac_channel.abs_diff(windows_channel) <= MONOGRAM_SAMPLE_COLOR_TOLERANCE,
                "both platforms must retain the same monogram colors"
            );
        }
    }
    assert_ne!(mac_icon.as_slice(), include_bytes!("../../../assets/AppIcon.icns").as_slice());
    let textora_windows_artwork = image::load_from_memory_with_format(
        include_bytes!("../../../assets/AppIcon.ico"),
        ImageFormat::Ico,
    )
    .expect("textora Windows icon must decode");
    assert_ne!(windows_artwork.to_rgba8(), textora_windows_artwork.to_rgba8());
}
