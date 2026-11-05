//! Page-level furniture: headers, cards, notices and empty states.

use gpui::{div, prelude::*, px, AnyElement, Div, FontWeight, Hsla, SharedString};

use super::{col, icon, row, TextExt};
use crate::assets::Icon;
use crate::theme::{self, font, radius, sp, GUTTER, TITLEBAR_H};

/// A soft card. Most things do not need one: reach for it only to group something as a unit.
pub fn card() -> Div {
    div()
        .rounded(px(radius::LG))
        .bg(theme::surface_1())
        .border_1()
        .border_color(theme::hairline())
}

/// The page title, with room for a subtitle and actions on the right.
pub fn page_header(
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    actions: impl IntoIterator<Item = AnyElement>,
) -> Div {
    row()
        .flex_none()
        .w_full()
        .pt(px(TITLEBAR_H - 16.0))
        .pb(sp(4.))
        .px(px(GUTTER))
        .gap(sp(4.))
        .items_start()
        .child(
            col()
                .flex_1()
                .min_w(px(0.))
                .gap(sp(0.5))
                .child(
                    div()
                        .fs(font::TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text_1())
                        .child(title.into()),
                )
                .child(
                    div()
                        .fs(font::BODY)
                        .text_color(theme::text_2())
                        .child(subtitle.into()),
                ),
        )
        .child(row().flex_none().gap(sp(2.)).mt(sp(0.5)).children(actions))
}

pub fn section_title(text: impl Into<SharedString>) -> Div {
    div()
        .fs(font::SUBHEAD)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::text_1())
        .child(text.into())
}

/// A big number with its unit, sized by the caller.
pub fn big_number(text: impl Into<SharedString>, size: f32, color: Hsla) -> Div {
    div()
        .fs(size)
        .font_weight(FontWeight::SEMIBOLD)
        .tabular()
        .text_color(color)
        .whitespace_nowrap()
        .child(text.into())
}

/// The little label above a number.
pub fn eyebrow(text: impl Into<SharedString>) -> Div {
    div()
        .fs(font::CAPTION)
        .text_color(theme::text_3())
        .whitespace_nowrap()
        .child(text.into())
}

/// Centred placeholder for pages with nothing to show, written like a person wrote it.
pub fn empty_state(
    which: Icon,
    accent: Hsla,
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
    action: Option<AnyElement>,
) -> Div {
    col()
        .flex_1()
        .size_full()
        .items_center()
        .justify_center()
        .gap(sp(3.5))
        .p(sp(8.))
        .child(
            row()
                .size(px(60.))
                .justify_center()
                .rounded_full()
                .bg(accent.alpha(0.10))
                .border_1()
                .border_color(accent.alpha(0.16))
                .child(icon(which, 26.0, accent)),
        )
        .child(
            col()
                .items_center()
                .gap(sp(1.5))
                .max_w(px(440.))
                .child(
                    div()
                        .fs(font::HEADLINE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text_1())
                        .text_center()
                        .child(title.into()),
                )
                .child(
                    div()
                        .fs(font::BODY)
                        .text_color(theme::text_2())
                        .text_center()
                        .child(message.into()),
                ),
        )
        .children(action.map(|action| div().mt(sp(1.)).child(action)))
}

#[derive(Clone, Copy)]
pub enum NoticeKind {
    Info,
    Warning,
    Error,
}

impl NoticeKind {
    fn look(self) -> (Icon, Hsla) {
        match self {
            NoticeKind::Info => (Icon::Info, theme::blue()),
            NoticeKind::Warning => (Icon::Alert, theme::amber()),
            NoticeKind::Error => (Icon::Alert, theme::rose()),
        }
    }
}

/// An inline message that explains something, with an optional action on the right.
pub fn notice(
    kind: NoticeKind,
    title: impl Into<SharedString>,
    detail: Option<SharedString>,
    action: Option<AnyElement>,
) -> Div {
    let (which, color) = kind.look();
    row()
        .w_full()
        .gap(sp(2.5))
        .items_start()
        .p(sp(3.))
        .rounded(px(radius::MD + 2.0))
        .bg(color.alpha(0.08))
        .border_1()
        .border_color(color.alpha(0.16))
        .child(div().mt(px(1.)).child(icon(which, 16.0, color)))
        .child(
            col()
                .flex_1()
                .min_w(px(0.))
                .gap(sp(0.5))
                .child(
                    div()
                        .fs(font::BODY)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::text_1())
                        .child(title.into()),
                )
                .children(detail.map(|detail| {
                    div()
                        .fs(font::CAPTION)
                        .text_color(theme::text_2())
                        .child(detail)
                })),
        )
        .children(action)
}

/// Sticky bar at the bottom of a page: the selected total and what to do with it.
pub fn action_bar(left: impl IntoElement, right: impl IntoElement) -> Div {
    row()
        .flex_none()
        .w_full()
        .h(px(60.))
        .px(px(GUTTER))
        .gap(sp(4.))
        .justify_between()
        .border_t_1()
        .border_color(theme::hairline())
        .bg(theme::surface_1())
        .child(div().flex_1().min_w(px(0.)).child(left))
        .child(div().flex_none().child(right))
}
