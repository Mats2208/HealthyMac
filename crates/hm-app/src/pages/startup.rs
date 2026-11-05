//! What starts by itself. Read-only for now: seeing it is half the battle.

use gpui::{div, prelude::*, px, uniform_list, AnyElement, Context, ElementId, FontWeight, Window};
use hm_system::types::{StartupItem, StartupKind};

use super::{gutter_row, plural};
use crate::assets::Icon;
use crate::state::Shell;
use crate::theme::{self, font, radius, sp, GUTTER};
use crate::ui::chrome::{empty_state, notice, page_header, NoticeKind};
use crate::ui::controls::{pill, tag};
use crate::ui::label::path_label;
use crate::ui::scrollbar::{scrollbar, ScrollSource};
use crate::ui::{clip, col, row, TextExt};

fn kind_label(kind: StartupKind) -> &'static str {
    match kind {
        StartupKind::UserAgent => "Tu usuario",
        StartupKind::GlobalAgent => "Todos los usuarios",
        StartupKind::GlobalDaemon => "Sistema",
    }
}

impl Shell {
    pub fn render_startup(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let items = self.startup.items.data.as_deref();
        let count = items.map_or(0, |items| items.len());
        let orphans = items.map_or(0, |items| items.iter().filter(|item| item.orphaned).count());
        let subtitle = if items.is_none() {
            "Lo que se pone en marcha sin que se lo pidas.".to_string()
        } else if orphans > 0 {
            format!(
                "{}. {} apunta a un programa que ya no existe.",
                plural(count, "elemento", "elementos"),
                plural(orphans, "huérfano", "huérfanos")
            )
        } else {
            format!(
                "{}. Solo lectura: de momento aquí se mira, no se toca.",
                plural(count, "elemento", "elementos")
            )
        };
        let header = page_header("Arranque", subtitle, Vec::new());

        let body: AnyElement = if let (None, Some(error)) =
            (&self.startup.items.data, &self.startup.items.error)
        {
            div()
                .px(px(GUTTER))
                .child(notice(
                    NoticeKind::Error,
                    "No hemos podido leer lo que arranca",
                    Some(error.clone().into()),
                    None,
                ))
                .into_any_element()
        } else if items.is_none() {
            empty_state(
                Icon::Power,
                theme::text_2(),
                "Mirando qué arranca",
                "Un segundo.",
                None,
            )
            .into_any_element()
        } else if count == 0 {
            empty_state(
                Icon::Power,
                theme::green(),
                "Nada arranca por su cuenta",
                "No hay agentes ni daemons de terceros. Tu Mac arranca ligero.",
                None,
            )
            .into_any_element()
        } else {
            let handle = self.startup.scroll.clone();
            let bar = self.startup.bar.clone();
            div()
                .relative()
                .flex_1()
                .min_h(px(0.))
                .border_t_1()
                .border_color(theme::hairline())
                .child(
                    uniform_list(
                        "startup-list",
                        count,
                        cx.processor(
                            |this: &mut Shell, range: std::ops::Range<usize>, _window, _cx| {
                                range
                                    .filter_map(|index| {
                                        let item = this.startup.items.data.as_ref()?.get(index)?;
                                        Some(render_item(index, item))
                                    })
                                    .collect::<Vec<_>>()
                            },
                        ),
                    )
                    .track_scroll(handle.clone())
                    .size_full(),
                )
                .child(scrollbar(ScrollSource::Uniform(handle), bar))
                .into_any_element()
        };
        col()
            .size_full()
            .child(header)
            .child(body)
            .into_any_element()
    }
}

fn render_item(index: usize, item: &StartupItem) -> AnyElement {
    let program = item
        .program
        .clone()
        .unwrap_or_else(|| item.plist_path.display().to_string());
    gutter_row()
        .child(
            row()
                .id(ElementId::NamedInteger("startup".into(), index as u64))
                .h(px(52.))
                .w_full()
                .px(sp(2.))
                .gap(sp(3.))
                .rounded(px(radius::MD))
                .hover(|style| style.bg(theme::surface_1()))
                .child(
                    col()
                        .flex_1()
                        .min_w(px(0.))
                        .child(
                            clip()
                                .fs(font::BODY)
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(if item.disabled {
                                    theme::text_3()
                                } else {
                                    theme::text_1()
                                })
                                .child(item.label.clone()),
                        )
                        .child(path_label(
                            program,
                            font::MICRO,
                            if item.orphaned {
                                theme::rose().alpha(0.8)
                            } else {
                                theme::text_3()
                            },
                        )),
                )
                .child(
                    row()
                        .flex_none()
                        .gap(sp(1.5))
                        .when(item.malformed, |this| {
                            this.child(pill("No se puede leer", theme::amber()))
                        })
                        .when(item.orphaned, |this| {
                            this.child(pill("Huérfano", theme::rose()))
                        })
                        .when(item.disabled, |this| this.child(tag("Desactivado")))
                        .when(item.keep_alive, |this| this.child(tag("Siempre activo")))
                        .when(item.run_at_load && !item.keep_alive, |this| {
                            this.child(tag("Al iniciar"))
                        })
                        .child(
                            div()
                                .w(px(112.))
                                .text_right()
                                .fs(font::CAPTION)
                                .text_color(theme::text_3())
                                .child(kind_label(item.kind)),
                        ),
                ),
        )
        .into_any_element()
}
