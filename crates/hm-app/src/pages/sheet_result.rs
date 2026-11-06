//! The result of a clean, honest about the difference between freed and merely moved to the Trash.

use gpui::{div, prelude::*, px, AnyElement, Context, ElementId, Hsla, IntoElement};
use hm_core::fmt::human_size;
use hm_core::CleanStatus;

use super::plural;
use super::sheets::{sheet_header, MAX_LISTED, SHEET_WIDTH};
use crate::assets::Icon;
use crate::flows::clean::ResultSheet;
use crate::logic::errors::{blocked_by_app_management, friendly_error, APP_MANAGEMENT_URL};
use crate::state::Shell;
use crate::theme::{self, font, radius, sp};
use crate::ui::chrome::{big_number, eyebrow, notice, NoticeKind};
use crate::ui::controls::{pill, Btn, BtnSize};
use crate::ui::scrollbar::{scrollbar, ScrollSource};
use crate::ui::{clip, col, row, TextExt};

impl Shell {
    pub(super) fn render_result(&self, result: &ResultSheet, cx: &mut Context<Self>) -> AnyElement {
        let freed = result.report.freed_bytes();
        let trashed = result.report.trashed_bytes();
        let estimated = result.report.has_estimates();
        let was_cut_short = |outcome: &hm_core::CleanOutcome| {
            outcome.errors.iter().any(|error| error == "cancelado")
        };
        let failures = result
            .report
            .outcomes
            .iter()
            .filter(|outcome| {
                matches!(outcome.status, CleanStatus::Failed | CleanStatus::Partial)
                    && !was_cut_short(outcome)
            })
            .count();
        let done = result
            .report
            .outcomes
            .iter()
            .filter(|outcome| matches!(outcome.status, CleanStatus::Done | CleanStatus::WouldClean))
            .count();
        let skipped = result
            .report
            .outcomes
            .iter()
            .filter(|outcome| outcome.status == CleanStatus::Skipped || was_cut_short(outcome))
            .count();
        let simulated = result.simulated;

        let title: String = if simulated && result.cancelled {
            "Simulación cancelada".to_string()
        } else if simulated {
            "Simulación terminada".to_string()
        } else if failures > 0 {
            "Hecho, con algún problema".to_string()
        } else if result.cancelled {
            "Limpieza cancelada".to_string()
        } else if done == 0 {
            "No se ha limpiado nada".to_string()
        } else {
            "Hecho".to_string()
        };
        let subtitle: String = if simulated && result.cancelled {
            "Esto es lo que habría pasado hasta ahí. No se ha tocado nada.".to_string()
        } else if simulated {
            "Esto es lo que habría pasado. No se ha tocado nada.".to_string()
        } else if result.cancelled {
            format!(
                "Se ha parado a mitad: {} de {} estaban hechos.",
                result.report.outcomes.len(),
                result.attempted
            )
        } else {
            format!(
                "{} de {}.",
                plural(done, "elemento listo", "elementos listos"),
                result.attempted
            )
        };

        let needs_app_management = result.report.outcomes.iter().any(|outcome| {
            outcome
                .errors
                .iter()
                .any(|error| blocked_by_app_management(&outcome.finding_id, error))
        });
        let pane = result.scroll.clone();
        let hidden = result.details.len().saturating_sub(MAX_LISTED);

        let figures = col()
                    .w_full()
                    .px(sp(5.))
                    .pb(sp(4.))
                    .gap(sp(3.5))
                    .when(freed > 0 || trashed > 0, |this| {
                        // A figure at zero only earns its place when it is the point: nothing freed
                        // yet is worth saying, but "0 B in the Trash" after a dedupe is just noise.
                        this.child(
                            row()
                                .w(px(SHEET_WIDTH - 42.0))
                                .gap(sp(2.5))
                                .items_start()
                                .child(result_figure(
                                    match (simulated, estimated) {
                                        (true, true) => "Se habría liberado, aprox.",
                                        (true, false) => "Se habría liberado",
                                        (false, true) => "Liberado ahora, aprox.",
                                        (false, false) => "Liberado ahora",
                                    },
                                    freed,
                                    if freed > 0 { theme::green() } else { theme::text_3() },
                                    match (simulated, estimated) {
                                        (_, true) => "Parte es lo que estimó el escaneo: herramientas como Docker o pnpm pueden liberar menos.",
                                        (true, false) => "Espacio libre al momento.",
                                        (false, false) => "Ya es espacio libre en tu disco.",
                                    },
                                ))
                                .when(trashed > 0, |this| {
                                    this.child(result_figure(
                                        if simulated { "Habría ido a la Papelera" } else { "En la Papelera" },
                                        trashed,
                                        theme::amber(),
                                        "Aún ocupa espacio, y se puede recuperar. No queda libre hasta que vacíes la Papelera.",
                                    ))
                                }),
                        )
                    })
                    .when(needs_app_management, |this| {
                        this.child(notice(
                            NoticeKind::Warning,
                            "macOS protege esta app",
                            Some(
                                "Para poder desinstalarla, concede a HealthyMac el permiso «Gestión de apps» en Ajustes del Sistema, dentro de Privacidad y seguridad, y vuelve a intentarlo."
                                    .into(),
                            ),
                            Some(
                                Btn::secondary("Abrir Ajustes")
                                    .size(BtnSize::Sm)
                                    .icon(Icon::External)
                                    .build(
                                        "open-app-management",
                                        cx.listener(|_, _, _, cx| cx.open_url(APP_MANAGEMENT_URL)),
                                    )
                                    .into_any_element(),
                            ),
                        ))
                    })
                    .when(failures > 0 || skipped > 0, |this| {
                        this.child(
                            div()
                                .fs(font::CAPTION)
                                .text_color(theme::text_2())
                                .child(match (failures, skipped) {
                                    (0, 1) => "1 elemento se ha quedado sin tocar:".to_string(),
                                    (0, others) => format!("{others} elementos se han quedado sin tocar:"),
                                    (1, 0) => "1 elemento no ha ido bien:".to_string(),
                                    (bad, 0) => format!("{bad} elementos no han ido bien:"),
                                    (1, 1) => "1 elemento no ha ido bien y otro se ha quedado sin tocar:".to_string(),
                                    (1, others) => format!("1 elemento no ha ido bien y otros {others} se han quedado sin tocar:"),
                                    (bad, 1) => format!("{bad} elementos no han ido bien y otro se ha quedado sin tocar:"),
                                    (bad, others) => format!("{bad} elementos no han ido bien y otros {others} se han quedado sin tocar:"),
                                }),
                        )
                    });

        let details =
            col()
                .w_full()
                .children(result.details.iter().take(MAX_LISTED).enumerate().map(
                    |(index, outcome)| {
                        let title = result
                            .titles
                            .get(&outcome.finding_id)
                            .cloned()
                            .unwrap_or_else(|| outcome.finding_id.clone());
                        outcome_row(index, title, outcome)
                    },
                ))
                .when(hidden > 0, |this| {
                    this.child(
                        div()
                            .px(sp(5.))
                            .py(sp(2.5))
                            .fs(font::CAPTION)
                            .text_color(theme::text_2())
                            .child(format!(
                                "Y {} más.",
                                plural(hidden, "elemento", "elementos")
                            )),
                    )
                });

        col()
            .relative()
            .flex_shrink()
            .min_h(px(0.))
            .child(sheet_header(&title, &subtitle, cx))
            .child(
                div()
                    .id("result-body")
                    .flex_shrink()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(&pane.handle)
                    .child(figures)
                    .child(details),
            )
            .child(scrollbar(
                ScrollSource::Handle(pane.handle.clone()),
                pane.bar.clone(),
            ))
            .child(
                row()
                    .flex_none()
                    .p(sp(5.))
                    .pt(sp(3.5))
                    .gap(sp(2.))
                    .justify_end()
                    .border_t_1()
                    .border_color(theme::hairline())
                    .when(trashed > 0, |this| {
                        this.child(
                            Btn::secondary("Vaciar la Papelera…")
                                .icon(Icon::Trash)
                                .build(
                                    "empty-trash",
                                    cx.listener(move |this, _, _, cx| {
                                        this.review_empty_trash(Some(trashed), cx)
                                    }),
                                ),
                        )
                    })
                    .child(Btn::primary("Listo").build(
                        "result-done",
                        cx.listener(|this, _, _, cx| this.close_sheet(cx)),
                    )),
            )
            .into_any_element()
    }
}

fn result_figure(
    label: &'static str,
    bytes: u64,
    color: Hsla,
    detail: &'static str,
) -> impl IntoElement {
    col()
        .flex_1()
        .min_w(px(0.))
        .gap(sp(0.5))
        .p(sp(3.5))
        .rounded(px(radius::LG))
        .bg(theme::surface_1())
        .border_1()
        .border_color(theme::hairline())
        .child(eyebrow(label))
        .child(big_number(human_size(bytes), font::TITLE, color))
        .child(
            div()
                .mt(sp(0.5))
                .fs(font::CAPTION)
                .text_color(theme::text_2())
                .child(detail),
        )
}

fn outcome_row(index: usize, title: String, outcome: &hm_core::CleanOutcome) -> AnyElement {
    let cut_short = outcome.errors.iter().any(|error| error == "cancelado");
    let (label, color) = match outcome.status {
        _ if cut_short => ("Cancelado", theme::text_3()),
        CleanStatus::Failed => ("Falló", theme::rose()),
        CleanStatus::Partial => ("A medias", theme::amber()),
        _ => ("Omitido", theme::text_3()),
    };
    let reason = outcome
        .errors
        .first()
        .filter(|_| !cut_short)
        .map(|error| {
            if blocked_by_app_management(&outcome.finding_id, error) {
                "Falta el permiso «Gestión de apps».".to_string()
            } else {
                friendly_error(error)
            }
        })
        .or_else(|| {
            outcome.message.as_ref().map(|message| {
                if message.ends_with("está abierto") {
                    format!("{message}. Ciérralo y vuelve a intentarlo.")
                } else {
                    message.clone()
                }
            })
        })
        .unwrap_or_else(|| {
            if cut_short {
                "Lo paraste antes de llegar a este.".to_string()
            } else {
                String::new()
            }
        });
    row()
        .id(ElementId::NamedInteger("outcome".into(), index as u64))
        .h(px(46.))
        .px(sp(5.))
        .gap(sp(2.5))
        .child(
            col()
                .flex_1()
                .min_w(px(0.))
                .child(clip().fs(font::CAPTION).child(title))
                .child(
                    clip()
                        .fs(font::MICRO)
                        .text_color(theme::text_3())
                        .child(reason),
                ),
        )
        .child(pill(label, color))
        .into_any_element()
}
