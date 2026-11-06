//! The first thing you see: how your Mac is doing and what you could do about it.

use gpui::{
    div, prelude::*, px, AnyElement, Background, Context, Div, FontWeight, SharedString, Stateful,
    Window,
};
use hm_core::fmt::human_size;
use hm_core::Area;
use hm_system::types::{DiskStats, Severity, SystemSnapshot};

use super::resumen_tiles::AREAS;
use super::{content_width, plural, scroll_pane};
use crate::assets::Icon;
use crate::logic::selection::{is_actionable, Summary};
use crate::state::Shell;
use crate::theme::{self, font, radius, sp, GUTTER};
use crate::ui::charts::{health_ring, indeterminate_bar, progress_bar, stack_bar};
use crate::ui::chrome::{action_bar, big_number, card, eyebrow, notice, NoticeKind};
use crate::ui::controls::{dot, Btn, BtnSize};
use crate::ui::{clip, col, icon, row, TextExt};

impl Shell {
    pub fn render_resumen(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let pane = self.resumen_scroll.clone();
        let summary = self.selection.summarize(&self.findings);
        col()
            .size_full()
            .child(scroll_pane(
                "resumen-scroll",
                &pane,
                col()
                    .w_full()
                    .px(px(GUTTER))
                    .pt(sp(10.))
                    .pb(sp(6.))
                    .gap(sp(7.))
                    .child(self.render_hero(window, cx))
                    .child(self.render_disk_usage())
                    .child(self.render_resumen_body(cx)),
            ))
            .when(summary.count > 0, |this| {
                this.child(self.render_selection_bar(None, summary, cx))
            })
            .into_any_element()
    }

    fn render_hero(&mut self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let wide = content_width(window) > 880.0;
        let diameter = if wide { 184.0 } else { 160.0 };
        // Until a scan says what can be recovered, the score leaves that out, and scanning would
        // seem to make it worse. Saying so up front keeps the drop from being a surprise.
        let provisional = !self.scan.finished_once;
        let (ring, headline, sub): (AnyElement, SharedString, SharedString) =
            match &self.live.health {
                Some(report) => (
                    health_ring(
                        self.live.ring_key,
                        self.live.ring_from,
                        self.live.ring_to,
                        if provisional {
                            "Provisional".into()
                        } else {
                            report.label.clone().into()
                        },
                        diameter,
                    )
                    .into_any_element(),
                    headline_for(report.score).into(),
                    sub_for(
                        report.score,
                        report
                            .factors
                            .iter()
                            .filter(|factor| factor.penalty > 0)
                            .count(),
                        provisional,
                    )
                    .into(),
                ),
                None => (
                    ring_placeholder(diameter),
                    "Tomándole el pulso a tu Mac".into(),
                    "Un momento, estamos leyendo cómo va todo.".into(),
                ),
            };

        let factors: Vec<AnyElement> = self
            .live
            .health
            .iter()
            .flat_map(|report| report.factors.iter())
            .take(if wide { 4 } else { 3 })
            .map(|factor| {
                let color = severity_color(factor.severity);
                row()
                    .gap(sp(2.5))
                    .items_start()
                    .child(div().mt(sp(1.5)).child(dot(color, 7.0)))
                    .child(
                        col()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .fs(font::BODY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::text_1())
                                    .child(factor.title.clone()),
                            )
                            .child(
                                div()
                                    .fs(font::CAPTION)
                                    .text_color(theme::text_2())
                                    .child(factor.detail.clone()),
                            ),
                    )
                    .when(factor.penalty > 0, |this| {
                        this.child(
                            div()
                                .flex_none()
                                .fs(font::CAPTION)
                                .tabular()
                                .text_color(theme::text_3())
                                .child(format!("−{}", factor.penalty)),
                        )
                    })
                    .into_any_element()
            })
            .collect();

        let summary = row()
            .w_full()
            .gap(px(if wide { 36.0 } else { 24.0 }))
            .items_start()
            .child(ring)
            .child(
                col()
                    .flex_1()
                    .min_w(px(0.))
                    .gap(sp(3.5))
                    .pt(sp(1.))
                    .child(
                        col()
                            .gap(sp(1.))
                            .child(
                                div()
                                    .fs(font::TITLE)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(headline),
                            )
                            .child(div().fs(font::BODY).text_color(theme::text_2()).child(sub)),
                    )
                    .child(col().gap(sp(2.5)).children(factors)),
            );
        // Narrow, the card would squeeze the reasons into a sliver, so it moves below and lies flat.
        if wide {
            summary.child(self.render_scan_card(false, cx))
        } else {
            col()
                .w_full()
                .gap(sp(5.))
                .child(summary)
                .child(self.render_scan_card(true, cx))
        }
    }

    /// The card that invites a scan, shows it running, and then shows what it found. `flat` lays it
    /// out as one wide row, for windows too narrow to give it a column of its own.
    fn render_scan_card(&mut self, flat: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let (info, action): (AnyElement, Stateful<Div>) = if self.scan.running {
            let (done, total) = self.scan.progress();
            let fraction = if total > 0 {
                done as f32 / total as f32
            } else {
                0.0
            };
            (
                col()
                    .gap(sp(3.))
                    .child(eyebrow("Escaneando"))
                    .child(big_number(
                        format!("{done} de {total}"),
                        font::DISPLAY - 8.0,
                        theme::text_1(),
                    ))
                    .child(if total == 0 {
                        indeterminate_bar("scan-indeterminate", theme::primary(), 6.0)
                    } else {
                        progress_bar(fraction, theme::primary(), 6.0)
                    })
                    .child(
                        div()
                            .fs(font::CAPTION)
                            .text_color(theme::text_2())
                            .child("Solo mira, no toca nada."),
                    )
                    .into_any_element(),
                Btn::secondary(if self.scan.cancelling {
                    "Cancelando…"
                } else {
                    "Cancelar"
                })
                .icon(Icon::Stop)
                .disabled(self.scan.cancelling)
                .build(
                    "scan-cancel",
                    cx.listener(|this, _, _, cx| this.cancel_scan(cx)),
                ),
            )
        } else if self.scan.finished_once {
            // What needs a human decision is not "reclaimable" until its owner says so.
            let certain = hm_system::health::reclaimable_bytes(&self.findings);
            let to_review = self.reclaimable_total().saturating_sub(certain);
            let chosen = self.selection.summarize(&self.findings);
            (
                col()
                    .gap(sp(1.))
                    .child(eyebrow("Puedes recuperar"))
                    .child(big_number(
                        human_size(certain),
                        font::DISPLAY,
                        theme::text_1(),
                    ))
                    // Ties this number to the one on the clean button, which counts only what is
                    // chosen and so is usually smaller.
                    .when(certain > 0, |this| {
                        this.child(
                            col()
                                .mt(sp(1.))
                                .gap(sp(1.5))
                                .child(progress_bar(
                                    chosen.bytes as f32 / certain as f32,
                                    theme::primary(),
                                    4.0,
                                ))
                                .child(
                                    div()
                                        .fs(font::CAPTION)
                                        .tabular()
                                        .text_color(theme::text_2())
                                        .child(format!(
                                            "{} elegidos para limpiar",
                                            human_size(chosen.bytes)
                                        )),
                                ),
                        )
                    })
                    .child(
                        div()
                            .mt(sp(1.))
                            .fs(font::CAPTION)
                            .text_color(theme::text_3())
                            .child(found_sentence(&self.findings, to_review)),
                    )
                    .into_any_element(),
                Btn::secondary("Escanear de nuevo")
                    .icon(Icon::Refresh)
                    .build(
                        "scan-again",
                        cx.listener(|this, _, _, cx| this.start_scan(cx)),
                    ),
            )
        } else {
            (
                col()
                    .gap(sp(1.))
                    .child(
                        div()
                            .fs(font::SUBHEAD)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("¿Cuánto puedes recuperar?"),
                    )
                    .child(
                        div()
                            .fs(font::CAPTION)
                            .text_color(theme::text_2())
                            .child("Escanear tarda un momento y no cambia nada en tu Mac."),
                    )
                    .into_any_element(),
                Btn::primary("Escanear")
                    .size(BtnSize::Lg)
                    .icon(Icon::Scan)
                    .build(
                        "scan-start",
                        cx.listener(|this, _, _, cx| this.start_scan(cx)),
                    ),
            )
        };
        let card = card().flex().p(sp(4.));
        if flat {
            card.flex_row()
                .items_center()
                .w_full()
                .gap(sp(6.))
                .child(div().flex_1().min_w(px(0.)).child(info))
                .child(action.flex_none().w(px(220.)))
        } else {
            card.flex_col()
                .flex_none()
                .w(px(236.))
                .min_h(px(176.))
                .gap(sp(3.))
                .child(info)
                .child(action.w_full())
        }
    }

    fn render_disk_usage(&self) -> impl IntoElement {
        let disk = self.live.snapshot.as_ref().and_then(main_disk);
        let Some(disk) = disk else {
            return col()
                .gap(sp(2.5))
                .child(section_heading("Disco", "Leyendo el disco…".to_string()))
                .child(indeterminate_bar("disk-loading", theme::surface_4(), 12.0));
        };
        let total = disk.total.max(1);
        let used = disk.total.saturating_sub(disk.available);
        let by_area = crate::logic::selection::reclaimable_by_area(&self.findings);
        let mut segments: Vec<(f32, Background)> = Vec::new();
        let mut legend: Vec<AnyElement> = Vec::new();
        let mut accounted = 0u64;
        for area in AREAS {
            let bytes = by_area.get(&area).copied().unwrap_or(0).min(used);
            if bytes == 0 {
                continue;
            }
            accounted = accounted.saturating_add(bytes);
            segments.push((bytes as f32 / total as f32, theme::area_color(area).into()));
            legend.push(legend_item(
                theme::area_color(area),
                area.label(),
                human_size(bytes),
            ));
        }
        let other = used.saturating_sub(accounted);
        segments.push((other as f32 / total as f32, theme::surface_4().into()));
        legend.push(legend_item(
            theme::surface_4(),
            if accounted > 0 { "Lo demás" } else { "En uso" },
            human_size(other),
        ));
        segments.push((
            disk.available as f32 / total as f32,
            theme::surface_2().into(),
        ));
        legend.push(legend_item(
            theme::surface_2(),
            "Libre",
            human_size(disk.available),
        ));

        col()
            .gap(sp(2.5))
            .child(section_heading(
                "Disco",
                format!(
                    "{} libres de {}",
                    human_size(disk.available),
                    human_size(disk.total)
                ),
            ))
            .child(stack_bar(&segments, 12.0))
            .child(
                row()
                    .flex_wrap()
                    .gap_x(sp(4.))
                    .gap_y(sp(1.))
                    .children(legend),
            )
    }

    fn render_resumen_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.scan.running
            || (self.scan.finished_once && self.scan.was_cancelled && self.findings.is_empty())
        {
            return self.render_live_scan().into_any_element();
        }
        if !self.scan.finished_once {
            return col()
                .gap(sp(3.))
                .child(section_title_row("Lo que vamos a mirar"))
                .child(
                    row()
                        .w_full()
                        .gap(sp(2.5))
                        .items_start()
                        .children(AREAS.map(|area| self.area_tile(area, cx))),
                )
                .child(div().mt(sp(4.)).child(section_title_row("Cómo funciona")))
                .child(how_it_works())
                .into_any_element();
        }
        let failures = self.scan.failures();
        col()
            .gap(sp(3.))
            .when(self.scan.was_cancelled, |this| {
                this.child(notice(
                    NoticeKind::Info,
                    "Has cancelado el escaneo",
                    Some("Esto es lo que dio tiempo a encontrar. Puede faltar algo.".into()),
                    None,
                ))
            })
            .when(!failures.is_empty(), |this| {
                let names: Vec<String> = failures
                    .iter()
                    .map(|(id, _)| self.scanner_name(id))
                    .collect();
                this.child(notice(
                    NoticeKind::Warning,
                    format!(
                        "{} no se pudo revisar",
                        plural(failures.len(), "parte", "partes")
                    ),
                    Some(format!("{}. El resto del escaneo es correcto.", names.join(", ")).into()),
                    None,
                ))
            })
            .child(section_title_row("Por área"))
            .child(
                row()
                    .w_full()
                    .gap(sp(2.5))
                    .items_start()
                    .children(AREAS.map(|area| self.area_tile(area, cx))),
            )
            .into_any_element()
    }

    pub fn render_selection_bar(
        &self,
        area: Option<Area>,
        summary: Summary,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let label = format!("Limpiar {}", summary.total_label());
        let at_least = summary.is_lower_bound();
        action_bar(
            row()
                .gap(sp(2.))
                .when(at_least, |this| {
                    this.child(
                        div()
                            .fs(font::CAPTION)
                            .text_color(theme::text_2())
                            .child("Al menos"),
                    )
                })
                .child(big_number(
                    human_size(summary.bytes),
                    font::HEADLINE + 2.0,
                    theme::text_1(),
                ))
                .child(
                    clip()
                        .ml(sp(1.))
                        .fs(font::CAPTION)
                        .text_color(theme::text_2())
                        .child(format!(
                            "en {}",
                            plural(summary.count, "seleccionado", "seleccionados")
                        )),
                ),
            Btn::primary(label).icon(Icon::Sparkle).build(
                "clean-selection",
                cx.listener(move |this, _, _, cx| this.review_selection(area, cx)),
            ),
        )
    }
}

/// Three steps that say, before anything is touched, why it is safe to press the button.
fn how_it_works() -> impl IntoElement {
    let step = |which: Icon, title: &'static str, detail: &'static str| {
        row()
            .flex_1()
            .min_w(px(0.))
            .gap(sp(3.))
            .items_start()
            .child(
                row()
                    .flex_none()
                    .size(px(32.))
                    .justify_center()
                    .rounded(px(radius::MD))
                    .bg(theme::surface_2())
                    .child(icon(which, 16.0, theme::text_2())),
            )
            .child(
                col()
                    .flex_1()
                    .min_w(px(0.))
                    .gap(sp(0.5))
                    .child(
                        div()
                            .fs(font::BODY)
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .child(
                        div()
                            .fs(font::CAPTION)
                            .text_color(theme::text_2())
                            .child(detail),
                    ),
            )
    };
    row()
        .w_full()
        .gap(sp(6.))
        .items_start()
        .child(step(
            Icon::Scan,
            "Escanea",
            "Solo mira: no mueve ni borra nada.",
        ))
        .child(step(
            Icon::Shield,
            "Revisa",
            "Lo que no se regenera solo nunca viene marcado de antemano.",
        ))
        .child(step(
            Icon::Trash,
            "Limpia",
            "Por defecto va a la Papelera, así que puedes recuperarlo.",
        ))
}

pub(super) fn section_title_row(text: &'static str) -> impl IntoElement {
    div()
        .fs(font::SUBHEAD)
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

fn section_heading(title: &'static str, detail: String) -> impl IntoElement {
    row()
        .justify_between()
        .child(
            div()
                .fs(font::SUBHEAD)
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .fs(font::CAPTION)
                .tabular()
                .text_color(theme::text_2())
                .child(detail),
        )
}

fn legend_item(color: gpui::Hsla, label: &str, value: String) -> AnyElement {
    row()
        .gap(sp(1.5))
        .h(px(20.))
        .child(dot(color, 8.0))
        .child(
            div()
                .fs(font::CAPTION)
                .text_color(theme::text_2())
                .child(label.to_string()),
        )
        .child(
            div()
                .fs(font::CAPTION)
                .tabular()
                .text_color(theme::text_1())
                .child(value),
        )
        .into_any_element()
}

fn ring_placeholder(diameter: f32) -> AnyElement {
    div()
        .flex_none()
        .size(px(diameter))
        .rounded_full()
        .border(px((diameter * 0.078).round()))
        .border_color(theme::surface_2())
        .into_any_element()
}

fn headline_for(score: u8) -> String {
    match score {
        85..=100 => "Tu Mac está en plena forma",
        65..=84 => "Tu Mac va bien",
        40..=64 => "A tu Mac le vendría bien un repaso",
        _ => "Tu Mac necesita atención",
    }
    .to_string()
}

fn sub_for(score: u8, factors: usize, provisional: bool) -> String {
    if provisional {
        return if factors == 0 {
            "Puntuación provisional hasta que escanees. De momento, nada te debe preocupar."
        } else {
            "Puntuación provisional hasta que escanees. De momento, esto es lo que más pesa:"
        }
        .to_string();
    }
    match (score, factors) {
        (_, 0) => "No hay nada que te deba preocupar ahora mismo.".to_string(),
        (85..=100, _) => "Solo unos detalles, por si quieres afinar:".to_string(),
        (65..=84, _) => "Hay algo de margen. Esto es lo que más pesa:".to_string(),
        _ => "Esto es lo que más está pesando:".to_string(),
    }
}

fn found_sentence(findings: &[hm_core::Finding], to_review: u64) -> String {
    let actionable = findings.iter().filter(|f| is_actionable(f)).count();
    if actionable == 0 {
        "No hemos encontrado nada que limpiar.".to_string()
    } else if to_review > 0 {
        format!(
            "Y otros {} si los revisas: pueden ser cosas tuyas.",
            human_size(to_review)
        )
    } else {
        format!("en {}", plural(actionable, "hallazgo", "hallazgos"))
    }
}

fn severity_color(severity: Severity) -> gpui::Hsla {
    match severity {
        Severity::Good => theme::green(),
        Severity::Info => theme::blue(),
        Severity::Warning => theme::amber(),
        Severity::Critical => theme::rose(),
    }
}

/// The boot volume, or failing that the biggest fixed disk.
pub fn main_disk(snapshot: &SystemSnapshot) -> Option<DiskStats> {
    snapshot
        .disks
        .iter()
        .find(|disk| disk.mount_point == std::path::Path::new("/"))
        .or_else(|| {
            snapshot
                .disks
                .iter()
                .filter(|disk| !disk.is_removable)
                .max_by_key(|disk| disk.total)
        })
        .cloned()
}
