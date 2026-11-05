//! The review sheet: everything that is about to happen, before anything does.

use gpui::{
    div, prelude::*, px, AnyElement, Background, Context, ElementId, Hsla, IntoElement,
    SharedString, Window,
};
use hm_core::fmt::human_size;
use hm_core::{Finding, RemovalMode, Risk};

use super::sheets::{sheet_header, MAX_LISTED, SHEET_WIDTH};
use super::{fmt_count, plural};
use crate::assets::Icon;
use crate::flows::clean::ReviewSheet;
use crate::logic::selection::{destination, risk_index, stays_inside, Destination, GroupState};
use crate::state::Shell;
use crate::theme::{self, font, sp};
use crate::ui::charts::stack_bar;
use crate::ui::chrome::big_number;
use crate::ui::controls::{checkbox, dot, pill, toggle, Btn};
use crate::ui::label::path_label;
use crate::ui::scrollbar::{scrollbar, ScrollSource};
use crate::ui::{clip, col, icon, row, TextExt};

impl Shell {
    pub(super) fn render_review(
        &self,
        review: &ReviewSheet,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let summary = &review.summary;
        let needs_ack = summary.needs_acknowledgement();
        let can_confirm = !needs_ack || review.acknowledged;
        let simulated = self.mode.is_simulated();
        let mode = self.ctx.config.removal_mode;
        let review_count = summary.by_risk[risk_index(Risk::Review)].count;

        // Risk breakdown.
        let total = summary.bytes.max(1) as f32;
        let segments: Vec<(f32, Background)> = [Risk::Safe, Risk::Moderate, Risk::Review]
            .iter()
            .map(|risk| {
                (
                    summary.by_risk[risk_index(*risk)].bytes as f32 / total,
                    theme::risk_color(*risk).into(),
                )
            })
            .collect();
        let breakdown = [Risk::Safe, Risk::Moderate, Risk::Review]
            .into_iter()
            .filter(|risk| summary.by_risk[risk_index(*risk)].count > 0)
            .map(|risk| {
                let totals = summary.by_risk[risk_index(risk)];
                row()
                    .gap(sp(1.5))
                    .child(dot(theme::risk_color(risk), 7.0))
                    .child(
                        div()
                            .fs(font::CAPTION)
                            .text_color(theme::text_2())
                            .child(risk.label()),
                    )
                    .child(
                        div()
                            .fs(font::CAPTION)
                            .tabular()
                            .text_color(theme::text_1())
                            .child(format!(
                                "{} · {}",
                                fmt_count(totals.count as u64),
                                human_size(totals.bytes)
                            )),
                    )
            });

        // Where everything ends up, said plainly.
        let lines = destination_lines(&review.items, mode);

        // What cannot be cleaned right now.
        let blocked: Vec<(String, usize)> = review
            .running
            .iter()
            .flatten()
            .map(|process| {
                let count = review
                    .items
                    .iter()
                    .filter(|finding| finding.blocked_by.contains(process))
                    .count();
                (process.clone(), count)
            })
            .filter(|(_, count)| *count > 0)
            .collect();

        let outside_test_home = self.mode.home_override.as_ref().map_or(0, |home| {
            review
                .items
                .iter()
                .filter(|finding| !stays_inside(finding, home))
                .count()
        });

        let pane = review.scroll.clone();
        let hidden = review.items.len().saturating_sub(MAX_LISTED);
        let hidden_bytes: u64 = review
            .items
            .iter()
            .skip(MAX_LISTED)
            .map(|finding| finding.size)
            .sum();

        let summary_block = col()
                    .w_full()
                    .px(sp(5.))
                    .pb(sp(3.5))
                    .gap(sp(3.))
                    .child(
                        row()
                            .gap(sp(3.))
                            .items_end()
                            .when(summary.is_lower_bound(), |this| {
                                this.child(
                                    div().pb(px(5.)).fs(font::BODY).text_color(theme::text_2()).child("Al menos"),
                                )
                            })
                            .child(big_number(human_size(summary.bytes), font::DISPLAY, theme::text_1()))
                            .child(
                                div()
                                    .pb(px(5.))
                                    .fs(font::BODY)
                                    .text_color(theme::text_2())
                                    .child(plural(summary.count, "elemento", "elementos")),
                            ),
                    )
                    .child(div().w(px(SHEET_WIDTH - 42.0)).child(stack_bar(&segments, 8.0)))
                    .child(row().flex_wrap().gap_x(sp(4.)).gap_y(sp(0.5)).children(breakdown))
                    .child(col().gap(sp(1.5)).children(lines))
                    .when(summary.is_lower_bound(), |this| {
                        this.child(note(
                            Icon::Layers,
                            theme::text_3(),
                            format!(
                                "Otros {} están compartidos con otras carpetas, como la caché de uv o el almacén de pnpm. Esos bloques siguen en el disco, y por eso la cifra es un mínimo.",
                                human_size(summary.shared_bytes)
                            ),
                            false,
                        ))
                    })
                    .children(blocked.iter().map(|(process, count)| {
                        note(
                            Icon::Alert,
                            theme::amber(),
                            format!(
                                "{process} está abierto, así que {} se omitirá. Ciérralo antes si quieres incluirlo.",
                                plural(*count, "elemento", "elementos")
                            ),
                            true,
                        )
                    }))
                    .when(outside_test_home > 0, |this| {
                        this.child(note(
                            Icon::Shield,
                            theme::blue(),
                            format!(
                                "{} queda fuera de la home de prueba, así que no se tocará.",
                                plural(outside_test_home, "elemento", "elementos")
                            ),
                            true,
                        ))
                    })
                    .when_some(review.uninstall.as_ref(), |this, options| {
                        let leftovers: u64 = options.app.leftovers.iter().map(|l| l.size).sum();
                        this.child(
                            row()
                                .gap(sp(2.5))
                                .child(toggle(
                                    "include-leftovers",
                                    options.include_leftovers,
                                    theme::lime(),
                                    cx.listener(|this, _, _, cx| this.toggle_uninstall_leftovers(cx)),
                                ))
                                .child(
                                    div()
                                        .fs(font::BODY)
                                        .child(format!(
                                            "Incluir también sus restos ({}, {})",
                                            plural(options.app.leftovers.len(), "carpeta", "carpetas"),
                                            human_size(leftovers)
                                        )),
                                ),
                        )
                    });

        let items = col()
            .w_full()
            .pt(sp(1.))
            .pb(sp(1.))
            .border_t_1()
            .border_color(theme::hairline())
            .children(
                review
                    .items
                    .iter()
                    .take(MAX_LISTED)
                    .enumerate()
                    .map(|(index, item)| review_item(index, item, mode)),
            )
            .when(hidden > 0, |this| {
                this.child(
                    div()
                        .px(sp(5.))
                        .py(sp(2.5))
                        .fs(font::CAPTION)
                        .text_color(theme::text_2())
                        .child(format!(
                            "Y {} más, con {}. Los has elegido en las páginas de cada área.",
                            plural(hidden, "elemento", "elementos"),
                            human_size(hidden_bytes)
                        )),
                )
            });

        col()
            .relative()
            .flex_shrink()
            .min_h(px(0.))
            .child(sheet_header(&review.title, "Nada se toca hasta que confirmes.", cx))
            .when(simulated, |this| {
                this.child(
                    row()
                        .flex_none()
                        .mx(sp(5.))
                        .mb(sp(3.))
                        .px(sp(2.5))
                        .py(px(7.))
                        .gap(px(7.))
                        .rounded(px(theme::radius::MD))
                        .bg(theme::amber().alpha(0.1))
                        .border_1()
                        .border_color(theme::amber().alpha(0.22))
                        .child(icon(Icon::Flask, 14.0, theme::amber()))
                        .child(
                            div()
                                .fs(font::CAPTION)
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(theme::amber())
                                .child("SIMULACIÓN"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .fs(font::CAPTION)
                                .text_color(theme::text_2())
                                .child("Verás lo que pasaría, pero no se va a borrar nada."),
                        ),
                )
            })
            .child(
                div()
                    .id("review-body")
                    .flex_shrink()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(&pane.handle)
                    .child(summary_block)
                    .child(items),
            )
            .child(scrollbar(ScrollSource::Handle(pane.handle.clone()), pane.bar.clone()))
            .child(
                col()
                    .flex_none()
                    .p(sp(5.))
                    .pt(sp(3.5))
                    .gap(sp(3.5))
                    .border_t_1()
                    .border_color(theme::hairline())
                    .when(needs_ack, |this| {
                        this.child(
                            row()
                                .gap(sp(2.))
                                .items_start()
                                .child(checkbox(
                                    "review-ack",
                                    if review.acknowledged { GroupState::All } else { GroupState::None },
                                    theme::rose(),
                                    cx.listener(|this, _, _, cx| this.toggle_acknowledged(cx)),
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .mt(px(3.))
                                        .fs(font::BODY)
                                        .text_color(theme::text_1())
                                        .child(if let Some(text) = &review.acknowledgement {
                                            text.clone()
                                        } else if review_count == 1 {
                                            "Lo he mirado. Entiendo que lo marcado «Revisar» puede contener algo que me importa.".to_string()
                                        } else {
                                            format!("Lo he mirado. Entiendo que estos {review_count} elementos marcados «Revisar» pueden contener cosas que me importan.")
                                        }),
                                ),
                        )
                    })
                    .child(
                        row()
                            .justify_end()
                            .gap(sp(2.))
                            .child(Btn::ghost("Cancelar").build(
                                "review-cancel",
                                cx.listener(|this, _, _, cx| this.close_sheet(cx)),
                            ))
                            .child(
                                Btn::primary(
                                    if simulated {
                                        "Simular".to_string()
                                    } else {
                                        format!("{} {}", review.confirm_label, summary.total_label())
                                    },
                                    if needs_ack {
                                        (theme::hex(0xFDA4AF), theme::rose())
                                    } else {
                                        theme::brand_stops()
                                    },
                                )
                                .disabled(!can_confirm)
                                .build(
                                    "review-confirm",
                                    cx.listener(|this, _, _, cx| this.confirm_review(cx)),
                                ),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn destination_lines(items: &[Finding], mode: RemovalMode) -> Vec<AnyElement> {
    let mut totals: Vec<(Destination, usize, u64)> = Vec::new();
    for finding in items {
        let target = destination(finding, mode);
        match totals.iter_mut().find(|(kind, _, _)| *kind == target) {
            Some((_, count, bytes)) => {
                *count += 1;
                *bytes = bytes.saturating_add(finding.size);
            }
            None => totals.push((target, 1, finding.size)),
        }
    }
    totals.sort_by_key(|(_, _, bytes)| std::cmp::Reverse(*bytes));
    totals
        .into_iter()
        .map(|(kind, _count, bytes)| {
            let size = human_size(bytes);
            let (which, color, text): (Icon, Hsla, String) = match kind {
                Destination::Trash => (
                    Icon::Trash,
                    theme::amber(),
                    format!("{size} irán a la Papelera. Podrás recuperarlos, pero el espacio no se libera hasta que la vacíes."),
                ),
                Destination::Deleted => (
                    Icon::Bolt,
                    theme::teal(),
                    format!("{size} se borran al momento, porque se regeneran solos."),
                ),
                Destination::Tool => (
                    Icon::Code,
                    theme::violet(),
                    format!("{size} los limpia la propia herramienta con su comando. Esa cifra es una estimación."),
                ),
                Destination::Clone => (
                    Icon::Copy,
                    theme::green(),
                    format!("{size} se liberan cambiando copias por clones APFS. Todo sigue en su sitio: no se pierde nada."),
                ),
                Destination::EmptyTrash => (
                    Icon::Alert,
                    theme::rose(),
                    "Borrado definitivo de todo lo que hay en la Papelera. Esto no se puede deshacer.".to_string(),
                ),
                Destination::Nothing => (Icon::Info, theme::text_3(), "Sin efecto.".to_string()),
            };
            row()
                .gap(sp(2.))
                .items_start()
                .child(div().mt(px(1.)).child(icon(which, 15.0, color)))
                .child(div().flex_1().min_w(px(0.)).fs(font::CAPTION).text_color(theme::text_1()).child(text))
                .into_any_element()
        })
        .collect()
}

fn review_item(index: usize, finding: &Finding, mode: RemovalMode) -> AnyElement {
    let target = destination(finding, mode);
    let which = match target {
        Destination::Trash => Icon::Trash,
        Destination::Deleted => Icon::Bolt,
        Destination::Tool => Icon::Code,
        Destination::Clone => Icon::Copy,
        Destination::EmptyTrash => Icon::Alert,
        Destination::Nothing => Icon::Info,
    };
    let subtitle: SharedString = finding.subtitle.clone().unwrap_or_default().into();
    row()
        .id(ElementId::NamedInteger("review-item".into(), index as u64))
        .h(px(46.))
        .px(sp(5.))
        .gap(sp(2.5))
        .child(icon(which, 14.0, theme::text_3()))
        .child(
            col()
                .flex_1()
                .min_w(px(0.))
                .child(clip().fs(font::CAPTION).child(finding.title.clone()))
                .when(!subtitle.is_empty(), |this| {
                    this.child(path_label(subtitle.clone(), font::MICRO, theme::text_3()))
                }),
        )
        .child(pill(finding.risk.label(), theme::risk_color(finding.risk)))
        .child(
            div()
                .flex_none()
                .w(px(64.))
                .text_right()
                .fs(font::CAPTION)
                .tabular()
                .child(human_size(finding.size)),
        )
        .into_any_element()
}

/// A line of plain information with its icon, quieter than a notice.
fn note(which: Icon, color: Hsla, text: impl Into<SharedString>, strong: bool) -> impl IntoElement {
    row()
        .w_full()
        .gap(sp(2.))
        .items_start()
        .child(div().mt(px(1.)).child(icon(which, 15.0, color)))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .fs(font::CAPTION)
                .text_color(if strong {
                    theme::text_1()
                } else {
                    theme::text_2()
                })
                .child(text.into()),
        )
}
