//! Placeholder avatars: two letters on a flat colour, both derived deterministically from the name.
//! Used whenever the real picture cannot be fetched or does not apply (for example a local git committer).

/// Two-letter initials. Words split on separators and camelCase humps: `XenonIsAwesome` -> `XA`, `john doe` -> `JD`,
/// `bob-smith` -> `BS`; a single word gives its first two letters (`alice` -> `AL`). An email uses its local part.
pub fn initials(name: &str) -> String {
    let name = name.trim(); let name = name.split('@').next().unwrap_or(name);
    let mut words: Vec<String> = vec![]; let mut cur = String::new(); let mut prev_lower = false;
    for c in name.chars() {
        if !c.is_alphanumeric() { if !cur.is_empty() { words.push(std::mem::take(&mut cur)); } prev_lower = false; continue; }
        if c.is_uppercase() && prev_lower && !cur.is_empty() { words.push(std::mem::take(&mut cur)); }
        prev_lower = c.is_lowercase(); cur.push(c);
    }
    if !cur.is_empty() { words.push(cur); }
    let up = |s: &str, n: usize| s.chars().take(n).flat_map(|c| c.to_uppercase()).collect::<String>();
    match words.as_slice() {
        [] => "?".into(),
        [one] => up(one, 2),
        [first, .., last] => format!("{}{}", up(first, 1), up(last, 1)),
    }
}

/// Background colours that sit well next to both light and dark themes.
pub const PALETTE: [[u8; 3]; 12] = [
    [0xE5, 0x73, 0x73], [0xF0, 0x8A, 0x4B], [0xD9, 0xA4, 0x41], [0x7C, 0xB3, 0x42], [0x3F, 0xA7, 0x7A], [0x2E, 0xA5, 0xA5],
    [0x42, 0x9B, 0xD9], [0x5C, 0x7C, 0xE0], [0x8E, 0x6C, 0xD8], [0xB5, 0x62, 0xC9], [0xD9, 0x5F, 0x9E], [0x78, 0x8A, 0x99],
];

/// FNV-1a over the lower-cased name, so the colour is stable across runs and machines.
pub fn hash(name: &str) -> u32 { name.to_lowercase().bytes().fold(0x811c9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x01000193)) }

fn luminance(c: [u8; 3]) -> f32 {
    let f = |v: u8| { let v = v as f32 / 255.0; if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) } };
    0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
}
fn mix(c: [u8; 3], t: [u8; 3], k: f32) -> [u8; 3] { let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k).round() as u8; [m(c[0], t[0]), m(c[1], t[1]), m(c[2], t[2])] }

fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 { let (x, y) = (luminance(a), luminance(b)); (x.max(y) + 0.05) / (x.min(y) + 0.05) }

/// Foreground for a background: a lighter tint of it or a darker shade of it, whichever reads better on that background.
pub fn foreground(bg: [u8; 3]) -> [u8; 3] {
    let (light, dark) = (mix(bg, [255, 255, 255], 0.88), mix(bg, [0, 0, 0], 0.78));
    if contrast(bg, light) >= contrast(bg, dark) { light } else { dark }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Swatch { pub bg: [u8; 3], pub fg: [u8; 3] }

pub fn swatch(name: &str) -> Swatch { let bg = PALETTE[hash(name) as usize % PALETTE.len()]; Swatch { bg, fg: foreground(bg) } }

/// GitHub noreply commit emails (`123+login@users.noreply.github.com`) name the account; anything else is `None`.
pub fn login_from_noreply(email: &str) -> Option<String> {
    let local = email.strip_suffix("@users.noreply.github.com")?;
    Some(local.split_once('+').map(|x| x.1).unwrap_or(local).to_string()).filter(|l| !l.is_empty())
}

#[cfg(test)]
mod tests { use super::*;
    #[test] fn initials_follow_the_name() {
        assert_eq!(initials("XenonIsAwesome"), "XA"); assert_eq!(initials("john doe"), "JD"); assert_eq!(initials("bob-smith"), "BS");
        assert_eq!(initials("alice"), "AL"); assert_eq!(initials("x"), "X"); assert_eq!(initials("Ada Byron King"), "AK");
        assert_eq!(initials("dev@corp.com"), "DE"); assert_eq!(initials("  "), "?"); assert_eq!(initials("Émile Zola"), "ÉZ");
    }
    #[test] fn colour_is_stable_and_case_blind() {
        assert_eq!(swatch("Alice"), swatch("alice")); assert_eq!(swatch("alice"), swatch("alice"));
        let distinct: std::collections::HashSet<[u8; 3]> = (0..200).map(|i| swatch(&format!("user{i}")).bg).collect(); assert!(distinct.len() >= 8);
    }
    #[test] fn foreground_contrasts_with_background() {
        for c in PALETTE { let f = foreground(c); let ratio = contrast(c, f); assert!(ratio >= 3.0, "{c:?} -> {f:?} ratio {ratio}"); }
    }
    #[test] fn noreply_emails_name_the_account() {
        assert_eq!(login_from_noreply("123+octo@users.noreply.github.com").as_deref(), Some("octo")); assert_eq!(login_from_noreply("octo@users.noreply.github.com").as_deref(), Some("octo"));
        assert_eq!(login_from_noreply("a@b.com"), None);
    }
}
