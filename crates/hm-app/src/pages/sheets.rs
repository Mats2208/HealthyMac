//! The modal sheets. A sheet is a column: fixed header, a body that scrolls if the window is short, and
//! a fixed footer holding the buttons, so the way out is always on screen.

use std::time::Duration;

use gpui::{
    deferred, div, ease_in_out, prelude::*, px, Animation, AnimationExt, AnyElement, Context,
    ElementId, FontWeight, Window,
};
use hm_core::fmt::human_size;

use crate::assets::Icon;
use crate::flows::clean::Sheet;
use crate::state::Shell;
use crate::theme::{self, font, radius, sp};
use crate::ui::charts::progress_bar;
use crate::ui::chrome::{notice, NoticeKind};
use crate::ui::controls::{icon_button, spinner, toggle, Btn};
use crate::ui::label::path_label;
use crate::ui::{clip, col, f, row, TextExt};

pub(super) const SHEET_WIDTH: f32 = 580.0;
/// Rows to lay out in a sheet. Anything past this is summed up in one line instead.
pub(super) const MAX_LISTED: usize = 150;

impl Shell {
    pub fn render_sheet(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let body: AnyElement = match self.sheet.as_ref()? {
            Sheet::Review(review) => self.render_review(review, window, cx),
            Sheet::Cleaning(_) => self.render_cleaning(cx),
            Sheet::Result(result) => self.render_result(result, cx),
            Sheet::Terminate(_) => self.render_terminate(cx),
        };
        let serial = match self.sheet.as_ref()? {
            Sheet::Review(_) => 1,
            Sheet::Cleaning(_) => 2,
            Sheet::Result(_) => 3,
            Sheet::Terminate(_) => 4,
        };
        let max_height = (f(window.viewport_size().height) - 64.0).max(320.0);
        Some(
            deferred(
                div()
                    .id("sheet-scrim")
                    .occlude()
                    .absolute()
                    .top(px(0.))
                    .left(px(0.))
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme::scrim())
                    .child(
                        col()
                            .id("sheet")
                            .occlude()
                            .w(px(SHEET_WIDTH))
                            .max_h(px(max_height))
                            .rounded(px(radius::XL))
                            .bg(theme::sheet_bg())
                            .border_1()
                            .border_color(theme::hairline_strong())
                            .shadow(theme::sheet_shadow())
                            .overflow_hidden()
                            .child(body)
                            .with_animation(
                                ElementId::NamedInteger("sheet-in".into(), serial),
                                Animation::new(Duration::from_millis(170)).with_easing(ease_in_out),
                                |this, t| this.opacity(t).mt(px(10.0 * (1.0 - t))),
                            ),
                    ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }

    fn render_cleaning(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(Sheet::Cleaning(sheet)) = &self.sheet else {
            return div().into_any_element();
        };
        let fraction = if sheet.total > 0 {
            sheet.index as f32 / sheet.total as f32
        } else {
            0.0
        };
        col()
            .p(sp(6.))
            .gap(sp(4.))
            .child(
                row()
                    .gap(sp(2.5))
                    .child(spinner("clean-spin", 16.0, theme::teal()))
                    .child(
                        div()
                            .fs(font::HEADLINE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if self.mode.is_simulated() {
                                "Simulando…"
                            } else {
                                "Limpiando…"
                            }),
                    ),
            )
            .child(progress_bar(fraction, theme::brand_stops(), 8.0))
            .child(
                row()
                    .justify_between()
                    .gap(sp(3.))
                    .child(
                        clip()
                            .flex_1()
                            .fs(font::BODY)
                            .text_color(theme::text_2())
                            .child(sheet.current.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .fs(font::BODY)
                            .tabular()
                            .text_color(theme::text_2())
                            .child(format!("{} de {}", sheet.index + 1, sheet.total)),
                    ),
            )
            .child(
                row()
                    .justify_between()
                    .child(
                        div()
                            .fs(font::CAPTION)
                            .tabular()
                            .text_color(theme::text_3())
                            .child(format!("{} procesados", human_size(sheet.bytes_done))),
                    )
                    .child(
                        Btn::secondary(if sheet.cancelling {
                            "Terminando el actual…"
                        } else {
                            "Cancelar"
                        })
                        .disabled(sheet.cancelling)
                        .build(
                            "clean-cancel",
                            cx.listener(|this, _, _, cx| this.cancel_clean(cx)),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn render_terminate(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(Sheet::Terminate(sheet)) = &self.sheet else {
            return div().into_any_element();
        };
        let port = &sheet.port;
        let simulated = !self.mode.may_signal_processes();
        col()
            .child(sheet_header(
                &format!("¿Terminar {}?", port.process_name),
                &format!("Escucha en el puerto {} · PID {}", port.port, port.pid),
                cx,
            ))
            .child(
                col()
                    .px(sp(5.))
                    .pb(sp(4.))
                    .gap(sp(3.5))
                    .when_some(port.cwd.clone().filter(|cwd| cwd.as_os_str() != "/"), |this, cwd| {
                        this.child(path_label(cwd.display().to_string(), font::CAPTION, theme::text_2()))
                    })
                    .child(
                        div()
                            .fs(font::BODY)
                            .text_color(theme::text_2())
                            .child("Se le pide que termine por las buenas (SIGTERM), para que pueda cerrar y guardar lo que tenga a medias."),
                    )
                    .child(
                        row()
                            .gap(sp(2.5))
                            .items_start()
                            .child(toggle(
                                "force-kill",
                                sheet.force,
                                theme::rose(),
                                cx.listener(|this, _, _, cx| this.toggle_terminate_force(cx)),
                            ))
                            .child(
                                col()
                                    .flex_1()
                                    .child(div().fs(font::BODY).child("Forzar (SIGKILL)"))
                                    .child(
                                        div()
                                            .fs(font::CAPTION)
                                            .text_color(theme::text_2())
                                            .child("Solo si no responde: lo corta en seco y puede perder lo que no haya guardado."),
                                    ),
                            ),
                    )
                    .when(simulated, |this| {
                        this.child(notice(
                            NoticeKind::Info,
                            "SIMULACIÓN: no se va a enviar ninguna señal",
                            None,
                            None,
                        ))
                    })
                    .when_some(sheet.error.clone(), |this, error| {
                        this.child(notice(NoticeKind::Error, "No se ha podido terminar", Some(error.into()), None))
                    }),
            )
            .child(
                row()
                    .p(sp(5.))
                    .pt(px(0.))
                    .gap(sp(2.))
                    .justify_end()
                    .child(Btn::ghost("Cancelar").build(
                        "terminate-cancel",
                        cx.listener(|this, _, _, cx| this.close_sheet(cx)),
                    ))
                    .child(
                        Btn::danger(if sheet.force { "Forzar cierre" } else { "Terminar" })
                            .disabled(sheet.in_flight)
                            .build(
                                "terminate-confirm",
                                cx.listener(|this, _, _, cx| this.confirm_terminate(cx)),
                            ),
                    ),
            )
            .into_any_element()
    }
}

pub(super) fn sheet_header(
    title: &str,
    subtitle: &str,
    cx: &mut Context<Shell>,
) -> impl IntoElement {
    row()
        .flex_none()
        .px(sp(5.))
        .pt(sp(4.5))
        .pb(sp(3.))
        .gap(sp(3.))
        .items_start()
        .child(
            col()
                .flex_1()
                .min_w(px(0.))
                .gap(sp(0.5))
                .child(
                    div()
                        .fs(font::HEADLINE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .fs(font::BODY)
                        .text_color(theme::text_2())
                        .child(subtitle.to_string()),
                ),
        )
        .child(icon_button(
            "sheet-close",
            Icon::X,
            theme::text_3(),
            cx.listener(|this, _, _, cx| this.close_sheet(cx)),
        ))
}
