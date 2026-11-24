//! Small design kit: palette, fonts, buttons, badges, and the rendered text field.
use crate::icons::Icon;
use gpui::{canvas, div, prelude::*, px, rgb, Animation, AnimationExt, AnyView, App, ClipboardItem, Context, Div, FontWeight, Hsla, Pixels, Render, SharedString, Stateful, Keystroke, Window};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;
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

/// Bounds of a rendered text field, recorded each paint so mouse positions can be mapped to characters.
pub type FieldBounds = Rc<Cell<gpui::Bounds<Pixels>>>;
pub fn new_bounds() -> FieldBounds { Rc::new(Cell::new(gpui::Bounds::default())) }

/// Char index in `text` at window x position `x` (the text starts at the left edge of `bounds`).
pub fn hit_char(window: &Window, text: &str, family: &SharedString, size: Pixels, bounds: &gpui::Bounds<Pixels>, x: Pixels) -> usize {
    if text.is_empty() || x <= bounds.origin.x { return 0; }
    let run = gpui::TextRun { len: text.len(), font: gpui::font(family.clone()), color: gpui::white(), background_color: None, underline: None, strikethrough: None };
    let line = window.text_system().shape_line(text.to_string().into(), size, &[run], None);
    let byte = line.index_for_x(x - bounds.origin.x).unwrap_or(text.len()).min(text.len());
    let mut b = byte; while b > 0 && !text.is_char_boundary(b) { b -= 1; }
    text[..b].chars().count()
}

/// Mouse down in a field: caret placement, Shift extends, double-click selects a word, triple-click everything.
pub fn field_mouse_down(f: &mut TextField, window: &Window, bounds: &gpui::Bounds<Pixels>, family: &SharedString, size: Pixels, ev: &gpui::MouseDownEvent) {
    let i = hit_char(window, &f.text, family, size, bounds, ev.position.x);
    match ev.click_count { 2 => f.select_word(i), 3.. => f.select_all(), _ => f.set_cursor(i, ev.modifiers.shift) }
}
/// Mouse drag in a field: extends the selection from where it started.
pub fn field_mouse_drag(f: &mut TextField, window: &Window, bounds: &gpui::Bounds<Pixels>, family: &SharedString, size: Pixels, x: Pixels) {
    let i = hit_char(window, &f.text, family, size, bounds, x);
    if f.anchor.is_none() { f.anchor = Some(f.cursor); }
    f.cursor = i;
}

// ---- icons, tooltips, loading indicator
pub fn icon(i: Icon, size: f32, color: Hsla) -> Div {
    div().font_family("Material Icons").text_size(px(size)).line_height(px(size)).size(px(size)).flex().items_center().justify_center().flex_none().text_color(color).child(i.0.to_string())
}

pub struct TipView { text: SharedString, p: Pal }
impl Render for TipView {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().px_2().py(px(4.0)).rounded_md().bg(self.p.fg).text_color(self.p.bg).text_xs().font_weight(FontWeight::MEDIUM).child(self.text.clone())
    }
}
pub fn tip(el: Stateful<Div>, text: impl Into<SharedString>, p: &Pal) -> Stateful<Div> {
    let (text, p) = (text.into(), *p);
    el.tooltip(move |_w, cx| -> AnyView { cx.new(|_| TipView { text: text.clone(), p }).into() })
}

/// A compact icon-only button with a tooltip. Attach `.on_click(...)` to the result.
pub fn icon_btn(id: impl Into<gpui::ElementId>, i: Icon, tooltip: &str, kind: Btn, p: &Pal) -> Stateful<Div> {
    let (fg, bg, hv, border) = match kind {
        Btn::Primary => (gpui::white(), { let mut g = p.success; if g.l > 0.4 { g.l *= 0.62; } g }, p.success, gpui::transparent_black()),
        Btn::Secondary => (p.fg, p.surface, p.hover, p.border),
        Btn::Ghost => (p.muted, gpui::transparent_black(), p.hover, gpui::transparent_black()),
    };
    let b = div().id(id).size(px(30.0)).flex().items_center().justify_center().flex_none().rounded_md().border_1().border_color(border).bg(bg).cursor_pointer().hover(move |s| s.bg(hv)).child(icon(i, 18.0, fg));
    tip(b, tooltip.to_string(), p)
}

/// Icon plus text button.
pub fn icon_text_btn(id: impl Into<gpui::ElementId>, i: Icon, label: impl Into<SharedString>, kind: Btn, p: &Pal) -> Stateful<Div> {
    let (fg, bg, hv, border) = match kind {
        Btn::Primary => (gpui::white(), { let mut g = p.success; if g.l > 0.4 { g.l *= 0.62; } g }, p.success, gpui::transparent_black()),
        Btn::Secondary => (p.fg, p.surface, p.hover, p.border),
        Btn::Ghost => (p.muted, gpui::transparent_black(), p.hover, gpui::transparent_black()),
    };
    div().id(id).flex().items_center().gap_2().px_3().h(px(30.0)).flex_none().rounded_md().border_1().border_color(border).bg(bg).text_color(fg).text_sm().font_weight(FontWeight::MEDIUM).cursor_pointer().hover(move |s| s.bg(hv))
        .child(icon(i, 17.0, fg)).child(label.into())
}

/// Segmented control cell showing only an icon.
pub fn seg_icon(id: impl Into<gpui::ElementId>, i: Icon, tooltip: &str, on: bool, p: &Pal) -> Stateful<Div> {
    let (fg, hv) = (p.fg, p.hover);
    let c = div().id(id).px_2().h(px(28.0)).flex().items_center().cursor_pointer().bg(if on { p.hover } else { gpui::transparent_black() }).hover(move |s| s.bg(hv)).child(icon(i, 17.0, if on { fg } else { p.muted }));
    tip(c, tooltip.to_string(), p)
}

/// Three pulsing dots: an agent is working.
pub fn loading_dots(p: &Pal) -> Div {
    let c = p.accent;
    div().flex().items_center().gap(px(4.0)).children((0..3usize).map(|i| {
        div().size(px(7.0)).rounded_full().bg(c).with_animation(("dot", i), Animation::new(Duration::from_millis(1200)).repeat(), move |d, t| {
            let phase = ((t * std::f32::consts::TAU) - i as f32 * 0.9).sin() * 0.5 + 0.5; d.opacity(0.2 + 0.8 * phase)
        })
    }).collect::<Vec<_>>())
}

/// Records the bounds of the element it is placed in (absolute, fills the parent).
pub fn bounds_probe(b: &FieldBounds) -> impl IntoElement {
    let b = b.clone(); canvas(move |bounds, _, _| b.set(bounds), |_, _, _, _| {}).absolute().size_full()
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
