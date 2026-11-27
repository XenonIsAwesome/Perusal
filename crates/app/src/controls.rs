//! Catalog of review-screen buttons the user may hide ("Customize interface" in settings).
//! The UI wraps each hideable button in a check against `Settings::hidden_controls`; the settings preview is drawn from `CONTROLS`.

/// Where a control lives on the review screen; the settings preview places it from this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone { TopBar, FilterRow, FilesPanel, RightTabs, FloatingBar, SelectionBar }

/// One hideable control: stable id (stored in settings.toml), label for tooltips and the preview, and its zone.
#[derive(Debug, Clone, Copy)]
pub struct Control { pub id: &'static str, pub label: &'static str, pub zone: Zone }

/// Every hideable control. Settings, home and the window buttons are deliberately absent: they must always be reachable.
pub const CONTROLS: &[Control] = &[
    Control { id: "rename", label: "Rename review", zone: Zone::TopBar },
    Control { id: "ci", label: "CI status", zone: Zone::TopBar },
    Control { id: "refresh", label: "Refresh", zone: Zone::TopBar },
    Control { id: "ai_review", label: "AI review", zone: Zone::TopBar },
    Control { id: "request_review", label: "Request a review", zone: Zone::TopBar },
    Control { id: "gen_filter", label: "Generate a filter with AI", zone: Zone::FilterRow },
    Control { id: "whitespace", label: "Hide whitespace-only changes", zone: Zone::FilterRow },
    Control { id: "sort", label: "Sort", zone: Zone::FilterRow },
    Control { id: "presets", label: "Filter and sort presets", zone: Zone::FilterRow },
    Control { id: "fold_all", label: "Expand / collapse folders", zone: Zone::FilesPanel },
    Control { id: "files_view", label: "Flat list / tree", zone: Zone::FilesPanel },
    Control { id: "ask_ai_tab", label: "Ask AI tab", zone: Zone::RightTabs },
    Control { id: "nav", label: "Previous / next batch", zone: Zone::FloatingBar },
    Control { id: "viewed", label: "Mark as viewed", zone: Zone::FloatingBar },
    Control { id: "global_comment", label: "Comment on the whole review", zone: Zone::FloatingBar },
    Control { id: "diff_mode", label: "Unified / side by side", zone: Zone::FloatingBar },
    Control { id: "md_mode", label: "Markdown source / rendered", zone: Zone::FloatingBar },
    Control { id: "batch_view", label: "Batch layout", zone: Zone::FloatingBar },
    Control { id: "fullscreen", label: "Full screen", zone: Zone::FloatingBar },
    Control { id: "finish", label: "Finish review", zone: Zone::FloatingBar },
    Control { id: "sel_comment", label: "Comment on selection", zone: Zone::SelectionBar },
    Control { id: "sel_ask_ai", label: "Ask AI about selection", zone: Zone::SelectionBar },
];

pub fn is_known(id: &str) -> bool { CONTROLS.iter().any(|c| c.id == id) }

/// Drops ids that no longer exist (renamed or removed in a later version) and duplicates, keeping order.
pub fn normalize(ids: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for i in ids { if is_known(&i) && !out.contains(&i) { out.push(i); } }
    out
}

#[cfg(test)]
mod tests { use super::*;
    #[test] fn ids_are_unique() { for (i, c) in CONTROLS.iter().enumerate() { assert!(!CONTROLS[..i].iter().any(|d| d.id == c.id), "{}", c.id); } }
    #[test] fn normalize_drops_unknown_and_duplicates() {
        assert_eq!(normalize(vec!["sort".into(), "bogus".into(), "sort".into(), "finish".into()]), vec!["sort", "finish"]);
    }
}
