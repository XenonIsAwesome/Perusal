//! Recognises the picture formats the viewer can show, from the file's bytes (the extension only decides SVG, which is text).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind { Png, Jpeg, Gif, Webp, Bmp, Tiff, Ico, Svg }

/// Largest picture (per side) the viewer loads; bigger ones keep the binary-file note so a review never stalls on a huge blob.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Extensions worth trying for a path before its bytes are looked at (used to decide whether to load the file at all).
pub fn looks_like_image(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "ico" | "svg")
}

/// The format of `bytes`, or `None` when it is not a supported picture (then the caller shows the normal binary-file note).
pub fn sniff(path: &str, bytes: &[u8]) -> Option<ImageKind> {
    let b = bytes;
    if b.starts_with(b"\x89PNG\r\n\x1a\n") { return Some(ImageKind::Png); }
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) { return Some(ImageKind::Jpeg); }
    if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") { return Some(ImageKind::Gif); }
    if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" { return Some(ImageKind::Webp); }
    if b.starts_with(b"BM") && b.len() > 26 { return Some(ImageKind::Bmp); }
    if b.starts_with(b"II*\0") || b.starts_with(b"MM\0*") { return Some(ImageKind::Tiff); }
    if b.len() > 22 && b.starts_with(&[0, 0, 1, 0]) && (1..=256).contains(&u16::from_le_bytes([b[4], b[5]])) { return Some(ImageKind::Ico); }
    if path.to_ascii_lowercase().ends_with(".svg") {
        let head = String::from_utf8_lossy(&b[..b.len().min(4096)]).to_ascii_lowercase();
        if head.contains("<svg") { return Some(ImageKind::Svg); }
    }
    None
}

/// Pixel size (width, height) read from the header without decoding the picture; `None` for SVG and for anything unreadable.
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn sniffs_by_content() {
        assert_eq!(sniff("a.bin", b"\x89PNG\r\n\x1a\n...."), Some(ImageKind::Png));
        assert_eq!(sniff("a.jpg", &[0xFF, 0xD8, 0xFF, 0xE0]), Some(ImageKind::Jpeg));
        assert_eq!(sniff("a.gif", b"GIF89a.."), Some(ImageKind::Gif));
        assert_eq!(sniff("a.webp", b"RIFF\0\0\0\0WEBPVP8 "), Some(ImageKind::Webp));
        let mut ico = vec![0, 0, 1, 0, 1, 0]; ico.resize(30, 0); assert_eq!(sniff("a.ico", &ico), Some(ImageKind::Ico));
        assert_eq!(sniff("a.ico", &[0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), None); // zero images
        assert_eq!(sniff("a.svg", b"<?xml version=\"1.0\"?>\n<SVG xmlns=\"x\"/>"), Some(ImageKind::Svg));
        assert_eq!(sniff("a.txt", b"<svg/>"), None); // SVG needs the extension, text is not an image by default
        assert_eq!(sniff("a.png", b"not a png"), None); // a wrong extension never counts
        assert_eq!(sniff("a.png", b""), None);
    }
    #[test] fn reads_size_from_header() {
        let mut png = Vec::new(); image::RgbaImage::new(3, 2).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        assert_eq!(sniff("a.png", &png), Some(ImageKind::Png)); assert_eq!(dimensions(&png), Some((3, 2)));
        assert_eq!(dimensions(b"junk"), None); assert_eq!(dimensions(b"<svg/>"), None);
    }
    #[test] fn header_only_picture_has_no_size() {
        // recognised by its magic number but too short to read a size from: the viewer then uses a square placeholder box
        let mut ico = vec![0, 0, 1, 0, 1, 0]; ico.resize(30, 0);
        assert_eq!(sniff("a.ico", &ico), Some(ImageKind::Ico)); assert_eq!(dimensions(&ico), None);
        let mut bmp = b"BM".to_vec(); bmp.resize(30, 0);
        assert_eq!(sniff("a.bmp", &bmp), Some(ImageKind::Bmp)); assert_eq!(dimensions(&bmp), None);
    }
    #[test] fn extension_hint() { assert!(looks_like_image("x/Y.PNG")); assert!(!looks_like_image("x/y.rs")); }
}
