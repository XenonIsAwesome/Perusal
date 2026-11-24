//! Small design kit: palette, fonts, buttons, badges, and the rendered text field.
use gpui::{div, prelude::*, px, rgb, App, ClipboardItem, Div, FontWeight, Hsla, SharedString, Stateful, Keystroke};
use perusal_app::textfield::{is_paste, TextField};
use perusal_app::theme::Theme;
use std::collections::BTreeMap;

pub fn col(t: &BTreeMap<String, String>, k: &str) -> Hsla {
    let h = t.get(k).map(|s| s.trim_start_matches('#')).unwrap_or("ff00ff");
    rgb(u32::from_str_radix(h, 16).unwrap_or(0xff00ff)).into()
}

#[derive(Clone, Copy)]
pub struct Pal { pub bg: Hsla, pub surface: Hsla, pub border: Hsla, pub fg: Hsla, pub muted: Hsla, pub accent: Hsla, pub hover: Hsla,
    pub success: Hsla, pub danger: Hsla, pub warn: Hsla, pub ai: Hsla, pub added: Hsla, pub removed: Hsla, pub added_word: Hsla, pub removed_word: Hsla }

impl Pal {
    pub fn new(t: &Theme) -> Self {
        Pal { bg: col(&t.ui, "background"), surface: col(&t.ui, "surface"), border: col(&t.ui, "border"), fg: col(&t.ui, "foreground"), muted: col(&t.ui, "muted"),
            accent: col(&t.ui, "accent"), hover: col(&t.ui, "hover"), success: col(&t.status, "ci_passed"), danger: col(&t.status, "ci_failed"), warn: col(&t.status, "ci_running"),
            ai: col(&t.status, "ai"), added: col(&t.diff, "added"), removed: col(&t.diff, "removed"), added_word: col(&t.diff, "added_word"), removed_word: col(&t.diff, "removed_word") }
    }
}

#[derive(Clone)]
pub struct Fonts { pub ui: SharedString, pub mono: SharedString }

/// Picks installed fonts: a pleasant UI font if present, else the platform UI font; a good monospace if present.
pub fn pick_fonts(cx: &App) -> Fonts {
    let names = cx.text_system().all_font_names();
    let has = |n: &str| names.iter().any(|x| x == n);
    let pick = |c: &[&str], fallback: &str| c.iter().find(|n| has(n)).map(|s| s.to_string()).unwrap_or_else(|| fallback.to_string());
    Fonts { ui: pick(&["Inter", "Segoe UI", "SF Pro Text", "Cantarell", "Noto Sans", "Roboto"], ".SystemUIFont").into(),
        mono: pick(&["JetBrains Mono", "Fira Code", "Cascadia Mono", "SF Mono", "Menlo", "DejaVu Sans Mono", "Liberation Mono", "Noto Sans Mono"], "monospace").into() }
}

pub fn badge(text: impl Into<SharedString>, fg: Hsla, bg: Hsla) -> Div { div().px_2().rounded_full().text_xs().font_weight(FontWeight::MEDIUM).text_color(fg).bg(bg).child(text.into()) }
pub fn pill(text: impl Into<SharedString>, p: &Pal) -> Div { div().px_2().py(px(1.0)).rounded_full().text_xs().border_1().border_color(p.border).text_color(p.muted).child(text.into()) }
pub fn kbd(text: &'static str, p: &Pal) -> Div { div().px(px(5.0)).rounded_sm().border_1().border_color(p.border).bg(p.surface).text_xs().text_color(p.fg).child(text) }

#[derive(Clone, Copy, PartialEq)]
pub enum Btn { Primary, Secondary, Ghost }

pub fn button(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, kind: Btn, p: &Pal) -> Stateful<Div> {
    let (bg, fg, hover_bg, border) = match kind {
        Btn::Primary => { let mut g = p.success; if g.l > 0.4 { g.l *= 0.62; } let mut h = g; h.l = (h.l * 1.15).min(1.0); (g, gpui::white(), h, g) }
        Btn::Secondary => (p.surface, p.fg, p.hover, p.border),
        Btn::Ghost => (gpui::transparent_black(), p.muted, p.hover, gpui::transparent_black()),
    };
    div().id(id).px_3().py(px(5.0)).rounded_md().border_1().border_color(border).bg(bg).text_color(fg).text_sm().font_weight(FontWeight::MEDIUM).cursor_pointer()
        .hover(move |s| s.bg(hover_bg)).child(label.into())
}

/// Feeds a key to a field, doing clipboard I/O. Returns true if the key was used.
pub fn field_key(f: &mut TextField, ks: &Keystroke, cx: &mut App) -> bool {
    let m = &ks.modifiers;
    let ctrl = m.control || m.platform;
    let clip = if is_paste(&ks.key, ctrl, m.shift) { cx.read_from_clipboard().and_then(|c| c.text()) } else { None };
    let o = f.apply(&ks.key, ctrl, m.shift, m.alt, ks.key_char.as_deref(), clip.as_deref());
    if let Some(t) = o.copy { cx.write_to_clipboard(ClipboardItem::new_string(t)); }
    o.consumed
}

/// Renders a text field's text with selection and caret.
pub fn field_text(f: &TextField, focused: bool, placeholder: &str, p: &Pal) -> Div {
    let (before, sel, after) = f.parts();
    let caret = || div().w(px(1.5)).h(px(15.0)).bg(p.accent).flex_none();
    if f.is_empty() {
        return div().flex().items_center().when(focused, |d| d.child(caret())).child(div().text_color(p.muted).child(placeholder.to_string()));
    }
    div().flex().items_center().whitespace_nowrap().overflow_hidden()
        .child(before)
        .when(!sel.is_empty(), |d| d.child(div().bg(p.accent.opacity(0.35)).child(sel.clone())))
        .when(focused && sel.is_empty(), |d| d.child(caret()))
        .child(after)
}

/// A bordered input box wrapping `field_text`, with a focus ring.
pub fn input_box(f: &TextField, focused: bool, placeholder: &str, p: &Pal, secret: bool) -> Div {
    let shown;
    let f = if secret { shown = { let mut t = f.clone(); t.text = "•".repeat(f.text.chars().count()); t }; &shown } else { f };
    div().px_3().py(px(6.0)).rounded_md().border_1().border_color(if focused { p.accent } else { p.border }).bg(p.bg).text_color(p.fg).min_h(px(32.0)).flex().items_center().cursor_text()
        .child(field_text(f, focused, placeholder, p))
}

/// On/off switch visual (wrap in an element with an on_click).
pub fn switch(on: bool, p: &Pal) -> Div {
    div().w(px(34.0)).h(px(18.0)).rounded_full().p(px(2.0)).flex().when(on, |d| d.justify_end()).bg(if on { p.success } else { p.border }).flex_none()
        .child(div().size(px(14.0)).rounded_full().bg(gpui::white()))
}

/// Segmented control cell.
pub fn seg_cell(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, on: bool, p: &Pal) -> Stateful<Div> {
    let (fg, hv) = (p.fg, p.hover);
    div().id(id).px_3().py(px(3.0)).text_xs().font_weight(FontWeight::MEDIUM).cursor_pointer().text_color(if on { p.fg } else { p.muted }).bg(if on { p.hover } else { gpui::transparent_black() })
        .hover(move |s| s.text_color(fg).bg(hv)).child(label.into())
}
pub fn seg_box(p: &Pal) -> Div { div().flex().rounded_md().border_1().border_color(p.border).overflow_hidden() }

/// Renders parsed markdown as a document.
pub fn markdown_view(blocks: &[perusal_app::markdown::Block], p: &Pal, mono: &SharedString) -> Div {
    use perusal_app::markdown::Block;
    let mut col = div().flex().flex_col().gap_3();
    for b in blocks {
        col = col.child(match b {
            Block::Heading(l, spans) => {
                let size = match l { 1 => 28.0, 2 => 22.0, 3 => 18.0, 4 => 16.0, _ => 14.0 };
                div().pt_2().pb_1().when(*l <= 2, |d| d.border_b_1().border_color(p.border)).text_size(px(size)).font_weight(FontWeight::BOLD).child(inline(spans, p, mono))
            }
            Block::Para(spans) => div().text_color(p.fg).line_height(px(22.0)).child(inline(spans, p, mono)),
            Block::Code { text, .. } => div().flex().flex_col().p_3().rounded_md().border_1().border_color(p.border).bg(p.surface).font_family(mono.clone()).text_size(px(13.0)).overflow_hidden()
                .children(text.lines().map(|l| div().whitespace_nowrap().child(if l.is_empty() { " ".to_string() } else { l.to_string() })).collect::<Vec<_>>()),
            Block::List { ordered, start, items } => div().flex().flex_col().gap_1().pl_4().children(items.iter().enumerate().map(|(i, it)| {
                div().flex().gap_2().child(div().w(px(22.0)).flex_none().text_color(p.muted).text_right().child(if *ordered { format!("{}.", start + i as u64) } else { "•".to_string() }))
                    .child(markdown_view(it, p, mono).flex_1().min_w_0().gap_1())
            }).collect::<Vec<_>>()),
            Block::Quote(inner) => div().pl_3().border_l_4().border_color(p.border).text_color(p.muted).child(markdown_view(inner, p, mono)),
            Block::Rule => div().h(px(1.0)).w_full().bg(p.border).my_2(),
            Block::Table { head, rows } => div().flex().flex_col().rounded_md().border_1().border_color(p.border).overflow_hidden()
                .child(div().flex().bg(p.surface).font_weight(FontWeight::SEMIBOLD).children(head.iter().map(|c| div().flex_1().min_w_0().px_3().py_1().border_r_1().border_color(p.border).child(inline(c, p, mono))).collect::<Vec<_>>()))
                .children(rows.iter().map(|r| div().flex().border_t_1().border_color(p.border).children(r.iter().map(|c| div().flex_1().min_w_0().px_3().py_1().border_r_1().border_color(p.border).child(inline(c, p, mono))).collect::<Vec<_>>())).collect::<Vec<_>>()),
        });
    }
    col
}

fn inline(spans: &[perusal_app::markdown::Span], p: &Pal, _mono: &SharedString) -> gpui::StyledText {
    let mut text = String::new(); let mut runs: Vec<(std::ops::Range<usize>, gpui::HighlightStyle)> = vec![];
    for s in spans {
        let start = text.len(); text.push_str(&s.text);
        let mut st = gpui::HighlightStyle::default(); let mut any = false;
        if s.bold { st.font_weight = Some(FontWeight::BOLD); any = true; }
        if s.italic { st.font_style = Some(gpui::FontStyle::Italic); any = true; }
        if s.code { st.background_color = Some(p.hover); st.color = Some(p.fg); any = true; }
        if s.link.is_some() { st.color = Some(p.accent); any = true; }
        if any && text.len() > start { runs.push((start..text.len(), st)); }
    }
    gpui::StyledText::new(text).with_highlights(runs)
}
