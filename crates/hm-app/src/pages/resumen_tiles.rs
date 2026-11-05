//! The dashboard's lower half: a tile per area, and every scanner's progress while one runs.

use gpui::{div, prelude::*, px, relative, AnyElement, Context, SharedString};
use hm_core::fmt::human_size;
use hm_core::Area;

use super::resumen::section_title_row;
use crate::assets::Icon;
use crate::logic::selection::is_actionable;
use crate::state::{AppsTab, Page, ScannerStatus, Shell};
use crate::theme::{self, font, radius, sp};
use crate::ui::chrome::big_number;
use crate::ui::controls::{dot, spinner};
use crate::ui::{clip, col, icon, row, TextExt};

/// The areas in the order they appear on the dashboard.
pub const AREAS: [Area; 5] = [
    Area::System,
    Area::Developer,
    Area::LocalAi,
    Area::Apps,
    Area::Files,
];

impl Shell {
    pub(super) fn area_tile(&self, area: Area, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = theme::area_color(area);
        let scanned = self.scan.finished_once;
        let members: Vec<_> = self
            .findings
            .iter()
            .filter(|f| f.area == area && is_actionable(f))
            .collect();
        let total: u64 = members.iter().map(|f| f.size).fold(0, u64::saturating_add);
        let selected = self.selection.summarize(members.iter().copied());
        let which = match area {
            Area::System => Icon::Sparkle,
            Area::Developer => Icon::Code,
            Area::LocalAi => Icon::Nodes,
            Area::Apps => Icon::Grid,
            Area::Files => Icon::File,
        };
        col()
            .id(SharedString::from(format!("tile-{area:?}")))
            .flex_1()
            .min_w(px(0.))
            .gap(sp(2.5))
            .p(sp(3.5))
            .rounded(px(radius::LG))
            .bg(theme::surface_1())
            .border_1()
            .border_color(theme::hairline())
            .cursor_pointer()
            .hover(|style| {
                style
                    .bg(theme::surface_2())
                    .border_color(theme::hairline_strong())
            })
            .active(|style| style.opacity(0.8))
            .on_click(cx.listener(move |this, _, _, cx| {
                if area == Area::Apps {
                    this.apps.tab = AppsTab::Leftovers;
                    this.navigate(Page::Apps, cx);
                } else {
                    this.navigate(Page::Area(area), cx);
                }
            }))
            .child(
                row().gap(px(7.)).child(icon(which, 15.0, accent)).child(
                    clip()
                        .fs(font::CAPTION)
                        .text_color(theme::text_2())
                        .child(area.label()),
                ),
            )
            .when(!scanned, |this| {
                this.child(
                    div()
                        .fs(font::CAPTION)
                        .text_color(theme::text_3())
                        .child(tile_blurb(area)),
                )
            })
            .when(scanned, |this| {
                this.child(
                    col()
                        .gap(sp(0.5))
                        .child(big_number(
                            if total == 0 {
                                "Nada".to_string()
                            } else {
                                human_size(total)
                            },
                            font::HEADLINE + 2.0,
                            if total == 0 {
                                theme::text_3()
                            } else {
                                theme::text_1()
                            },
                        ))
                        .child(
                            clip()
                                .fs(font::MICRO)
                                .tabular()
                                .text_color(theme::text_3())
                                .child(if total == 0 {
                                    "Todo limpio".to_string()
                                } else if selected.count > 0 {
                                    format!("{} elegidos", selected.total_label())
                                } else {
                                    "Sin elegir".to_string()
                                }),
                        ),
                )
                .child(
                    div()
                        .h(px(3.))
                        .w_full()
                        .rounded(px(2.))
                        .bg(theme::surface_2())
                        .when(total > 0 && selected.bytes > 0, |this| {
                            this.child(
                                div()
                                    .h_full()
                                    .w(relative((selected.bytes as f32 / total as f32).min(1.0)))
                                    .rounded(px(2.))
                                    .bg(accent),
                            )
                        }),
                )
            })
    }

    /// Every scanner and what it is doing right now, by area.
    pub fn render_live_scan(&self) -> impl IntoElement {
        let mut scanners = self.scanners.clone();
        scanners.sort_by_key(|meta| AREAS.iter().position(|area| *area == meta.area));
        let (done, total) = self.scan.progress();
        col()
            .gap(sp(3.))
            .child(section_title_row(if self.scan.running {
                "Revisando tu Mac"
            } else {
                "Lo que dio tiempo a revisar"
            }))
            .when(total > 0 && done == total && self.scan.running, |this| {
                this.child(
                    div()
                        .fs(font::CAPTION)
                        .text_color(theme::text_2())
                        .child("Atando cabos…"),
                )
            })
            .child(
                div()
                    .grid()
                    .grid_cols(3)
                    .w_full()
                    .gap_x(sp(5.))
                    .gap_y(sp(0.5))
                    .children(scanners.into_iter().map(|meta| {
                        let status = self
                            .scan
                            .statuses
                            .get(meta.id)
                            .cloned()
                            .unwrap_or(ScannerStatus::Waiting);
                        scanner_line(meta.id, meta.name, meta.area, status)
                    })),
            )
    }
}

fn tile_blurb(area: Area) -> &'static str {
    match area {
        Area::System => "Cachés, registros y restos del sistema",
        Area::Developer => "Compilaciones, dependencias y Docker",
        Area::LocalAi => "Modelos repetidos y cachés de IA",
        Area::Apps => "Restos de apps que ya no tienes",
        Area::Files => "Archivos grandes y descargas viejas",
    }
}

fn scanner_line(
    id: &'static str,
    name: &'static str,
    area: Area,
    status: ScannerStatus,
) -> impl IntoElement {
    let status_icon: AnyElement = match &status {
        ScannerStatus::Waiting => dot(theme::surface_4(), 6.0).mx(sp(1.)).into_any_element(),
        ScannerStatus::Running => spinner(
            SharedString::from(format!("spin-{id}")),
            14.0,
            theme::area_color(area),
        )
        .into_any_element(),
        ScannerStatus::Done { .. } => {
            icon(Icon::Check, 14.0, theme::area_color(area)).into_any_element()
        }
        ScannerStatus::Failed(_) => icon(Icon::Alert, 14.0, theme::amber()).into_any_element(),
    };
    let trailing = match &status {
        ScannerStatus::Done { bytes, .. } if *bytes > 0 => human_size(*bytes),
        ScannerStatus::Done { .. } => "Nada".to_string(),
        ScannerStatus::Failed(_) => "Falló".to_string(),
        _ => String::new(),
    };
    let waiting = matches!(status, ScannerStatus::Waiting);
    row()
        .min_w(px(0.))
        .h(px(28.))
        .gap(sp(2.))
        .child(row().w(px(16.)).justify_center().child(status_icon))
        .child(
            clip()
                .flex_1()
                .fs(font::CAPTION)
                .text_color(if waiting {
                    theme::text_3()
                } else {
                    theme::text_1()
                })
                .child(name),
        )
        .child(
            div()
                .flex_none()
                .fs(font::CAPTION)
                .tabular()
                .text_color(theme::text_2())
                .child(trailing),
        )
}
