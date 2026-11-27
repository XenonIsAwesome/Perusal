//! "Customize interface": a miniature skeleton of the review screen where every hideable button can be clicked to hide or show it.
//! Hidden buttons are grayed out with a dashed border. The list of controls lives in `perusal_app::controls`.
use super::SettingsView;
use crate::icons::{self as ic, Icon};
use crate::kit::{self, icon, icon_btn, icon_text_btn, tip, Btn};
use gpui::{deferred, div, prelude::*, px, relative, AnyElement, Context, FontWeight, MouseButton, SharedString};

impl SettingsView {
    /// Easter egg: shown the first time the AI features are switched off.
    pub(super) fn egg_overlay(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.pal;
        deferred(div().absolute().top_0().left_0().size_full().flex().items_center().justify_center().bg(gpui::black().opacity(0.6)).occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| { this.egg = false; cx.notify(); }))
            .child(div().id("egg-card").relative().p_2().rounded_lg().border_1().border_color(p.border).bg(p.surface).shadow_lg().occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(kit::easter_egg(420.0))
                .child(div().absolute().top(px(14.0)).right(px(14.0)).child(icon_btn("egg-close", ic::CLOSE, "Close", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.egg = false; cx.notify(); })))))).with_priority(30).into_any_element()
    }

    pub(super) fn controls_editor(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.pal;
        let hidden_n = self.settings.hidden_controls.len();
        // ---- building blocks (all static visuals; the slot around them takes the clicks)
        let bone = move |w: f32| div().h(px(7.0)).w(px(w)).rounded_full().bg(p.border).flex_none();
        let bone_f = move || div().h(px(7.0)).flex_1().min_w_0().rounded_full().bg(p.border);
        let ibtn = move |i: Icon, k: Btn| {
            let (fg, bg, bd) = match k { Btn::Primary => (gpui::white(), { let mut g = p.success; if g.l > 0.4 { g.l *= 0.62; } g }, gpui::transparent_black()), Btn::Secondary => (p.fg, p.surface, p.border), Btn::Ghost => (p.muted, gpui::transparent_black(), gpui::transparent_black()) };
            div().size(px(26.0)).flex().items_center().justify_center().flex_none().rounded_md().border_1().border_color(bd).bg(bg).child(icon(i, 16.0, fg))
        };
        let tbtn = move |i: Icon, label: &'static str, k: Btn| {
            let (fg, bg, bd) = match k { Btn::Primary => (gpui::white(), { let mut g = p.success; if g.l > 0.4 { g.l *= 0.62; } g }, gpui::transparent_black()), Btn::Secondary => (p.fg, p.surface, p.border), Btn::Ghost => (p.muted, gpui::transparent_black(), gpui::transparent_black()) };
            div().flex().items_center().gap_1().px_2().h(px(26.0)).flex_none().rounded_md().border_1().border_color(bd).bg(bg).text_color(fg).text_xs().font_weight(FontWeight::MEDIUM).child(icon(i, 15.0, fg)).child(label)
        };
        let seg = move |icons: &[Icon]| div().flex().flex_none().rounded_md().border_1().border_color(p.border).overflow_hidden().children(icons.iter().enumerate().map(|(n, i)| div().px_2().h(px(24.0)).flex().items_center().bg(if n == 0 { p.hover } else { gpui::transparent_black() }).child(icon(*i, 15.0, if n == 0 { p.fg } else { p.muted }))).collect::<Vec<_>>());
        let locked = move |v: gpui::Div| v.opacity(0.9);
        // A clickable, hide/show-able control. Hidden: grayed out, dashed border.
        let slot = |id: &'static str, visual: AnyElement, cx: &mut Context<Self>| {
            // AI controls are forced hidden by the "Disable AI features" switch: shown as hidden and locked, clicks ignored.
            let ai_locked = !self.settings.ai_on() && perusal_app::controls::is_ai(id);
            let hidden = ai_locked || self.settings.hidden_controls.iter().any(|h| h == id);
            let label = perusal_app::controls::CONTROLS.iter().find(|c| c.id == id).map(|c| c.label).unwrap_or(id);
            let el = div().id(SharedString::from(format!("ctl-{id}"))).flex().items_center().p(px(2.0)).rounded_md().border_1().flex_none()
                .border_color(if hidden { p.muted } else { gpui::transparent_black() })
                .when(hidden, |d| d.border_dashed().opacity(0.4))
                .when(!ai_locked, |d| d.cursor_pointer().hover(move |d| if hidden { d.opacity(0.7) } else { d.border_color(p.accent) })
                    .on_click(cx.listener(move |this, _, _, cx| { let now = this.settings.hidden_controls.iter().any(|h| h == id); this.settings.set_hidden(id, !now); this.changed(cx); })))
                .child(visual);
            let note = if ai_locked { "hidden because AI features are disabled (AI tab)" } else if hidden { "hidden. Click to show it" } else { "shown. Click to hide it" };
            tip(el, format!("{label}: {note}"), &p)
        };

        // ---- tab strip
        let tabs = div().flex().items_center().gap_2().px_2().h(px(30.0)).bg(p.surface).border_b_1().border_color(p.border)
            .child(div().flex().items_center().gap_2().px_3().h(px(24.0)).rounded_md().bg(p.bg).border_1().border_color(p.border).child(bone(90.0)).child(icon(ic::CLOSE, 12.0, p.muted)))
            .child(icon(ic::ADD, 14.0, p.muted));
        // ---- top bar
        let top = div().flex().items_center().gap_2().px_3().h(px(42.0)).bg(p.surface).border_b_1().border_color(p.border)
            .child(locked(ibtn(ic::HOME, Btn::Ghost))).child(bone(150.0))
            .child(slot("rename", ibtn(ic::EDIT, Btn::Ghost).into_any_element(), cx))
            .child(div().px_2().h(px(18.0)).flex().items_center().rounded_full().border_1().border_color(p.border).child(bone(60.0)))
            .child(slot("ci", icon(ic::CHECK_CIRCLE, 18.0, p.success).into_any_element(), cx))
            .child(div().flex_1())
            .child(slot("refresh", ibtn(ic::REFRESH, Btn::Secondary).into_any_element(), cx))
            .child(slot("ai_review", tbtn(ic::AUTO_AWESOME, "AI review", Btn::Secondary).into_any_element(), cx))
            .child(slot("request_review", ibtn(ic::PERSON_ADD, Btn::Secondary).into_any_element(), cx))
            .child(tip(locked(ibtn(ic::SETTINGS, Btn::Secondary)).id("ctl-settings"), "Settings can't be hidden", &p));
        // ---- progress and filter row
        let progress = div().flex().items_center().gap_3().px_3().h(px(22.0)).bg(p.bg).border_b_1().border_color(p.border)
            .child(bone(80.0)).child(div().flex_1().h(px(6.0)).rounded_full().bg(p.border).child(div().h_full().w(relative(0.4)).rounded_full().bg(p.success))).child(bone(40.0));
        let filter = div().flex().items_center().gap_1().px_3().py(px(5.0)).bg(p.bg).border_b_1().border_color(p.border)
            .child(div().flex().items_center().gap_2().px_2().h(px(26.0)).flex_1().min_w_0().rounded_md().border_1().border_color(p.border).bg(p.surface).child(icon(ic::SEARCH, 15.0, p.muted)).child(bone(160.0)))
            .child(slot("gen_filter", ibtn(ic::AUTO_AWESOME, Btn::Ghost).into_any_element(), cx))
            .child(slot("whitespace", ibtn(ic::SPACE_BAR, Btn::Ghost).into_any_element(), cx))
            .child(slot("sort", tbtn(ic::SORT, "Sort", Btn::Ghost).into_any_element(), cx))
            .child(slot("presets", ibtn(ic::BOOKMARK, Btn::Ghost).into_any_element(), cx));
        // ---- left panel
        let file_row = move |w: f32, sel: bool| div().flex().items_center().gap_2().px_3().h(px(24.0)).when(sel, |d| d.bg(p.accent.opacity(0.12)).border_l_2().border_color(p.accent)).child(div().size(px(12.0)).rounded_sm().bg(p.border)).child(bone(w)).child(div().flex_1()).child(bone(22.0));
        let left = div().flex().flex_col().w(px(210.0)).flex_none().h_full().bg(p.surface).border_r_1().border_color(p.border)
            .child(div().flex().items_center().gap_1().px_2().h(px(34.0)).border_b_1().border_color(p.border).child(div().text_xs().font_weight(FontWeight::SEMIBOLD).child("Files")).child(div().flex_1())
                .child(slot("fold_all", div().flex().child(ibtn(ic::UNFOLD_MORE, Btn::Ghost)).child(ibtn(ic::UNFOLD_LESS, Btn::Ghost)).into_any_element(), cx))
                .child(slot("files_view", seg(&[ic::VIEW_LIST, ic::ACCOUNT_TREE]).into_any_element(), cx)))
            .children([(90.0, true), (110.0, false), (70.0, false), (100.0, false), (80.0, false), (120.0, false), (60.0, false), (95.0, false)].map(|(w, s)| file_row(w, s)));
        // ---- diff area
        let diff_row = move |n: usize| {
            let tint = match n % 7 { 2 | 3 => Some(p.success), 5 => Some(p.danger), _ => None };
            div().flex().items_center().gap_3().px_3().h(px(22.0)).when_some(tint, |d, c| d.bg(c.opacity(0.13))).child(bone(18.0)).child(bone(18.0)).child(bone(40.0 + ((n * 53) % 200) as f32))
        };
        let sel_bar = div().absolute().top(px(88.0)).left(px(90.0)).flex().items_center().gap_1().px_2().py_1().rounded_lg().bg(p.surface).border_1().border_color(p.accent).shadow_lg()
            .child(bone(50.0))
            .child(slot("sel_comment", tbtn(ic::ADD_COMMENT, "Comment", Btn::Secondary).into_any_element(), cx))
            .child(slot("sel_ask_ai", tbtn(ic::AUTO_AWESOME, "Ask AI", Btn::Secondary).into_any_element(), cx))
            .child(locked(ibtn(ic::CLOSE, Btn::Ghost)));
        let sep = move || div().w(px(1.0)).h(px(20.0)).bg(p.border).flex_none();
        let bar = div().absolute().bottom(px(10.0)).left_0().right_0().flex().justify_center().child(
            div().flex().items_center().gap_1().px_2().py_1().rounded_lg().bg(p.surface).border_1().border_color(p.border).shadow_lg()
                .child(slot("nav", div().flex().gap_1().child(ibtn(ic::CHEVRON_LEFT, Btn::Secondary)).child(ibtn(ic::CHEVRON_RIGHT, Btn::Primary)).into_any_element(), cx))
                .child(slot("viewed", ibtn(ic::VISIBILITY_OFF, Btn::Secondary).into_any_element(), cx))
                .child(slot("global_comment", ibtn(ic::ADD_COMMENT, Btn::Secondary).into_any_element(), cx))
                .child(sep())
                .child(slot("diff_mode", seg(&[ic::VIEW_AGENDA, ic::VERTICAL_SPLIT]).into_any_element(), cx))
                .child(slot("md_mode", seg(&[ic::CODE, ic::VISIBILITY]).into_any_element(), cx))
                .child(slot("batch_view", seg(&[ic::VIEW_QUILT, ic::VIEW_STREAM]).into_any_element(), cx))
                .child(slot("fullscreen", ibtn(ic::FULLSCREEN, Btn::Secondary).into_any_element(), cx))
                .child(sep())
                .child(slot("finish", tbtn(ic::RATE_REVIEW, "Finish", Btn::Primary).into_any_element(), cx)));
        let center = div().relative().flex_1().min_w_0().h_full().flex().flex_col().overflow_hidden()
            .child(div().flex().items_center().gap_2().px_3().h(px(30.0)).border_b_1().border_color(p.border).child(icon(ic::EXPAND_MORE, 14.0, p.muted)).child(bone(130.0)).child(div().flex_1()).child(bone(36.0)))
            .children((0..14).map(diff_row)).child(sel_bar).child(bar);
        // ---- right panel
        let card = move || div().flex().flex_col().gap_2().p_2().mx_2().mt_2().rounded_md().border_1().border_color(p.border).child(bone(80.0)).child(bone_f()).child(bone(120.0));
        let right = div().flex().flex_col().w(px(200.0)).flex_none().h_full().bg(p.surface).border_l_1().border_color(p.border)
            .child(div().flex().items_center().gap_1().px_2().h(px(34.0)).border_b_1().border_color(p.border)
                .child(div().flex().items_center().gap_1().px_2().h(px(26.0)).border_b_2().border_color(p.accent).child(icon(ic::COMMIT, 14.0, p.fg)).child(bone(36.0)))
                .child(slot("ask_ai_tab", div().flex().items_center().gap_1().px_2().h(px(26.0)).child(icon(ic::AUTO_AWESOME, 14.0, p.muted)).child(bone(40.0)).into_any_element(), cx)))
            .child(card()).child(card()).child(card());
        let window = div().flex().flex_col().w_full().h(px(560.0)).flex_none().rounded_lg().border_1().border_color(p.border).bg(p.bg).overflow_hidden()
            .child(tabs).child(top).child(progress).child(filter)
            .child(div().flex().flex_1().min_h_0().child(left).child(center).child(right));

        // ---- the card around it
        let title = div().flex().items_center().gap_3().px_5().py_3().border_b_1().border_color(p.border)
            .child(div().flex_1().flex().flex_col().child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Customize interface"))
                .child(div().text_xs().text_color(p.muted).child("Click a button to hide or show it where it appears in a review. Hidden buttons are grayed out with a dashed border. Keyboard shortcuts keep working.")));
        let footer = div().flex().items_center().justify_end().gap_2().px_5().py_3().border_t_1().border_color(p.border).bg(p.surface)
            .child(div().flex_1().text_xs().text_color(p.muted).child(format!("{hidden_n} hidden. Changes are saved as you make them.")))
            .child(icon_text_btn("ctl-reset", ic::UNDO, "Show all", Btn::Secondary, &p).on_click(cx.listener(|this, _, _, cx| { this.settings.hidden_controls.clear(); this.changed(cx); })))
            .child(icon_text_btn("ctl-done", ic::CHECK, "Done", Btn::Primary, &p).on_click(cx.listener(|this, _, _, cx| { this.controls_open = false; cx.notify(); })));
        let card = div().id("controls-card").text_color(p.fg).text_size(px(13.0)).flex().flex_col().w(px(1000.0)).max_w(relative(0.96)).max_h(relative(0.96)).rounded_lg().border_1().border_color(p.border).bg(p.surface).shadow_lg().occlude().overflow_hidden()
            .child(title).child(div().id("controls-scroll").flex_1().min_h_0().overflow_y_scroll().p_4().child(window)).child(footer);
        deferred(div().absolute().top_0().left_0().size_full().flex().items_center().justify_center().bg(gpui::black().opacity(0.55)).occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| { this.controls_open = false; cx.notify(); }))
            .child(card)).with_priority(25).into_any_element()
    }
}
