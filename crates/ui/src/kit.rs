//! Small design kit: palette, fonts, buttons, badges, and the rendered text field.
use crate::icons::Icon;
use gpui::{canvas, div, MouseButton, prelude::*, px, rgb, Animation, AnimationExt, AnyView, App, ClipboardItem, Context, Div, FontWeight, Hsla, Pixels, Render, SharedString, Stateful, Keystroke, Window};
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
            Block::Code { lang, text } if lang.trim() == "suggestion" => div().flex().flex_col().rounded_md().border_1().border_color(p.success).overflow_hidden()
                .child(div().flex().items_center().gap_2().px_3().py(px(4.0)).bg(p.success.opacity(0.16)).text_xs().font_weight(FontWeight::SEMIBOLD).text_color(p.success).child(icon(crate::icons::LIGHTBULB, 15.0, p.success)).child("Suggested change"))
                .child(div().flex().flex_col().px_3().py_2().bg(p.added).font_family(mono.clone()).text_size(px(13.0)).children(text.lines().map(|l| div().whitespace_nowrap().child(if l.is_empty() { " ".to_string() } else { l.to_string() })).collect::<Vec<_>>())),
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

// ---- multi-line field
pub const LINE_H: f32 = 20.0;

/// Renders a multi-line field: one row per line, with selection and caret.
pub fn field_multi(f: &TextField, focused: bool, placeholder: &str, p: &Pal, min_lines: usize) -> Div {
    let lh = px(LINE_H);
    if f.is_empty() {
        return div().flex().flex_col().min_h(px(LINE_H * min_lines as f32)).child(div().h(lh).flex().items_center().when(focused, |d| d.child(div().w(px(1.5)).h(px(15.0)).bg(p.accent).flex_none())).child(div().text_color(p.muted).child(placeholder.to_string())));
    }
    let sel = f.selection(); let mut start = 0usize;
    let mut rows = Vec::new();
    for l in f.text.split('\n') {
        let n = l.chars().count(); let end = start + n;
        let cut = |a: usize, b: usize| -> String { l.chars().skip(a).take(b.saturating_sub(a)).collect() };
        let (s0, s1) = match sel { Some((a, b)) if b > start && a < end + 1 => (a.max(start) - start, b.min(end) - start), _ => (0, 0) };
        let caret_here = focused && sel.is_none() && f.cursor >= start && f.cursor <= end;
        let caret_at = f.cursor.saturating_sub(start).min(n);
        let row = if s1 > s0 {
            div().flex().items_center().h(lh).whitespace_nowrap().child(cut(0, s0)).child(div().bg(p.accent.opacity(0.35)).child(cut(s0, s1))).child(cut(s1, n))
        } else if caret_here {
            div().flex().items_center().h(lh).whitespace_nowrap().child(cut(0, caret_at)).child(div().w(px(1.5)).h(px(15.0)).bg(p.accent).flex_none()).child(cut(caret_at, n))
        } else { div().flex().items_center().h(lh).whitespace_nowrap().child(if l.is_empty() { " ".to_string() } else { l.to_string() }) };
        rows.push(row); start = end + 1;
    }
    div().flex().flex_col().min_h(px(LINE_H * min_lines as f32)).children(rows)
}

/// Char index in a multi-line field at a window position.
pub fn hit_multi(window: &Window, f: &TextField, family: &SharedString, size: Pixels, bounds: &gpui::Bounds<Pixels>, pos: gpui::Point<Pixels>) -> usize {
    let lines = f.lines(); let rel: f32 = (pos.y - bounds.origin.y).into();
    let li = ((rel / LINE_H).floor().max(0.0) as usize).min(lines.len().saturating_sub(1));
    let col = hit_char(window, lines[li], family, size, bounds, pos.x);
    f.index_of(li, col)
}

// ---- small visuals
/// Five small squares showing the ratio of added (green) to removed (red) lines, like GitHub and GitLab.
pub fn diff_squares(adds: u32, dels: u32, p: &Pal, size: f32) -> Div {
    let total = (adds + dels).max(1) as f32; let green = ((adds as f32 / total) * 5.0).round() as usize;
    div().flex().gap(px(2.0)).flex_none().children((0..5usize).map(|i| div().size(px(size)).rounded_sm().bg(if adds + dels == 0 { p.border } else if i < green { p.success } else { p.danger })).collect::<Vec<_>>())
}

/// Dropdown trigger: current value with a chevron. Attach `.on_click(...)`.
pub fn dd_button(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, open: bool, p: &Pal) -> Stateful<Div> {
    let hv = p.hover;
    div().id(id).flex().items_center().justify_between().gap_2().min_w(px(180.0)).px_3().h(px(32.0)).rounded_md().border_1().border_color(if open { p.accent } else { p.border }).bg(p.bg).cursor_pointer().hover(move |s| s.bg(hv))
        .text_sm().child(label.into()).child(icon(if open { crate::icons::EXPAND_LESS } else { crate::icons::EXPAND_MORE }, 18.0, p.muted))
}

/// CI status badge for the header. `state` is one of running, pending, passed, failed, canceled; `None` hides it.
pub fn ci_badge(state: &str, p: &Pal) -> Option<Stateful<Div>> {
    let (i, color, label, pulse) = match state {
        "running" => (crate::icons::SYNC, p.warn, "Checks are running", true), "pending" => (crate::icons::SCHEDULE, p.muted, "Checks are queued", true),
        "passed" => (crate::icons::CHECK_CIRCLE, p.success, "Checks passed", false), "failed" => (crate::icons::CANCEL, p.danger, "Checks failed", false),
        "canceled" => (crate::icons::BLOCK, p.muted, "Checks were canceled", false), _ => return None,
    };
    let b = div().id("ci").flex().items_center().justify_center().size(px(30.0)).rounded_md().cursor_pointer().hover(move |s| s.bg(p.hover));
    let ic = icon(i, 20.0, color);
    let b = if pulse { b.child(ic.with_animation("ci-pulse", Animation::new(Duration::from_millis(if state == "running" { 2000 } else { 3500 })).repeat(), |d, t| d.opacity(0.35 + 0.65 * ((t * std::f32::consts::TAU).sin() * 0.5 + 0.5)))) } else { b.child(ic) };
    Some(tip(b, format!("{label}. Click to open the checks page."), p))
}

// ---- reusable input
/// A text input: state, rendered bounds (for mouse hit-testing) and drag state in one place.
pub struct Input { pub f: TextField, pub b: FieldBounds, pub dragging: bool }
impl Input {
    pub fn new(text: &str) -> Self { Input { f: TextField::new(text), b: new_bounds(), dragging: false } }
    pub fn multi(text: &str) -> Self { Input { f: TextField::multi(text), b: new_bounds(), dragging: false } }
}

/// Renders an input's text with caret and selection, ready to place inside a styled box.
pub fn input_text(i: &Input, focused: bool, placeholder: &str, p: &Pal, secret: bool, min_lines: usize) -> Div {
    let inner = if i.f.multiline { field_multi(&i.f, focused, placeholder, p, min_lines) } else if secret {
        let mut m = i.f.clone(); m.text = "•".repeat(i.f.text.chars().count()); field_text(&m, focused, placeholder, p)
    } else { field_text(&i.f, focused, placeholder, p) };
    div().relative().flex_1().min_w_0().cursor_text().text_size(px(14.0)).child(bounds_probe(&i.b)).child(inner)
}

/// Adds mouse caret placement, drag selection, double/triple click and the text cursor to an input's container.
/// `on_focus` runs on mouse down so the owner can mark the input as the active one.
pub fn wire_input<T: 'static>(el: Stateful<Div>, cx: &mut Context<T>, get: fn(&mut T) -> &mut Input, on_focus: fn(&mut T, &mut Context<T>), family: SharedString, secret: bool) -> Stateful<Div> {
    let fam2 = family.clone();
    el.cursor_text()
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &gpui::MouseDownEvent, w, cx| {
            on_focus(this, cx); let i = get(this); i.dragging = true; let b = i.b.get();
            if i.f.multiline {
                let idx = hit_multi(w, &i.f, &family, px(14.0), &b, ev.position);
                match ev.click_count { 2 => i.f.select_word(idx), 3.. => i.f.select_all(), _ => i.f.set_cursor(idx, ev.modifiers.shift) }
            } else if secret {
                let mut m = i.f.clone(); m.text = "•".repeat(i.f.text.chars().count()); field_mouse_down(&mut m, w, &b, &family, px(14.0), ev); i.f.cursor = m.cursor; i.f.anchor = m.anchor;
            } else { field_mouse_down(&mut i.f, w, &b, &family, px(14.0), ev); }
            cx.stop_propagation(); cx.notify();
        }))
        .on_mouse_move(cx.listener(move |this, ev: &gpui::MouseMoveEvent, w, cx| {
            let i = get(this);
            if ev.pressed_button != Some(MouseButton::Left) { i.dragging = false; return; }
            if !i.dragging { return; }
            let b = i.b.get();
            if i.f.multiline { let idx = hit_multi(w, &i.f, &fam2, px(14.0), &b, ev.position); if i.f.anchor.is_none() { i.f.anchor = Some(i.f.cursor); } i.f.cursor = idx; }
            else if secret { let mut m = i.f.clone(); m.text = "•".repeat(i.f.text.chars().count()); field_mouse_drag(&mut m, w, &b, &fam2, px(14.0), ev.position.x); i.f.cursor = m.cursor; i.f.anchor = m.anchor; }
            else { field_mouse_drag(&mut i.f, w, &b, &fam2, px(14.0), ev.position.x); }
            cx.notify();
        }))
}
