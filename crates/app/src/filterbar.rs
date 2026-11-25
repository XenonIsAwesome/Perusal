//! Filter bar view model: inline errors, autocomplete, presets with default auto-apply.
use perusal_core::presets::{Preset, Presets};

#[derive(Default)]
pub struct FilterBar { pub text: String, pub sort: String, pub error: Option<String>, pub suggestions: Vec<&'static str>, pub override_for_this_mr: bool }

impl FilterBar {
    pub fn type_text(&mut self, t: &str) {
        self.text = t.into();
        self.error = perusal_filter::compile(t).err().filter(|_| !t.trim().is_empty());
        self.suggestions = perusal_filter::complete(t);
    }
    /// Applies the project (else global) default when an MR opens, unless the user overrides.
    pub fn on_open(&mut self, presets: &Presets, remote: &str) {
        if self.override_for_this_mr { return; }
        if let Some(p) = presets.effective_default(remote) { self.type_text(&p.filter.clone()); self.sort = p.sort.clone(); }
    }
    pub fn clear(&mut self) { self.type_text(""); self.sort.clear(); self.override_for_this_mr = true; }
    pub fn as_preset(&self) -> Preset { Preset { filter: self.text.clone(), sort: self.sort.clone() } }
}
#[cfg(test)]
mod tests { use super::*;
    #[test] fn errors_complete_defaults() {
        let mut b = FilterBar::default(); b.type_text("additions >"); assert!(b.error.is_some()); b.type_text("add"); assert!(b.suggestions.contains(&"additions")); b.type_text("added > 3"); assert!(b.error.is_none());
        let mut p = Presets::default(); p.save(Some("https://h/a/b"), "p", Preset { filter: "!viewed".into(), sort: "name".into() }); p.set_default(Some("https://h/a/b"), Some("p"));
        let mut b = FilterBar::default(); b.on_open(&p, "git@h:a/b.git"); assert_eq!((b.text.as_str(), b.sort.as_str()), ("!viewed", "name"));
        b.clear(); b.on_open(&p, "git@h:a/b.git"); assert_eq!(b.text, ""); assert_eq!(b.as_preset(), Preset::default());
    } }
