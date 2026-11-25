//! Single source of truth for product naming. A rename is a change here
//! (plus workspace metadata).
pub const PRODUCT_NAME: &str = "Perusal";
pub const BINARY_NAME: &str = "perusal";
pub const CONFIG_DIR: &str = "perusal";
pub const DEEP_LINK_SCHEME: &str = "perusal";

/// Builds a deep link that opens the given MR/PR URL.
pub fn deep_link_for(mr_url: &str) -> String {
    let enc: String = mr_url
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("{DEEP_LINK_SCHEME}://open?url={enc}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deep_link_encodes() {
        assert_eq!(deep_link_for("https://a/b?c=1"), "perusal://open?url=https%3A%2F%2Fa%2Fb%3Fc%3D1");
    }
}
