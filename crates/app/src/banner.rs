//! Banner shown when the MR moved on while a review was pinned.
#[derive(Debug, PartialEq)]
pub enum Banner { None, NewCommits { new_head: String }, ForcePushed { new_head: String }, Merged, Closed }

/// `is_ancestor(old, new)` tells whether the pinned head is still in the new head's history.
pub fn banner(state: &str, pinned: &str, host_head: &str, is_ancestor: &dyn Fn(&str, &str) -> bool) -> Banner {
    match state { "merged" => return Banner::Merged, "closed" => return Banner::Closed, _ => {} }
    if pinned == host_head { Banner::None }
    else if is_ancestor(pinned, host_head) { Banner::NewCommits { new_head: host_head.into() } }
    else { Banner::ForcePushed { new_head: host_head.into() } }
}
#[cfg(test)]
mod tests { use super::*;
    #[test] fn states() {
        let anc = |a: &str, _: &str| a == "old";
        assert_eq!(banner("open", "h", "h", &anc), Banner::None);
        assert_eq!(banner("open", "old", "n", &anc), Banner::NewCommits { new_head: "n".into() });
        assert_eq!(banner("open", "x", "n", &anc), Banner::ForcePushed { new_head: "n".into() });
        assert_eq!(banner("merged", "x", "n", &anc), Banner::Merged);
    } }
