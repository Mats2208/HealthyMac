//! The modal sheets. A sheet is a column: fixed header, a body that scrolls if the window is short, and
//! a fixed footer holding the buttons, so the way out is always on screen.

use gpui::{
    deferred, div, prelude::*, px, AnimationExt, AnyElement, Context, ElementId, FontWeight, Window,
};
use hm_core::fmt::human_size;
use hm_core::{t, tf};

use crate::assets::Icon;
use crate::flows::clean::Sheet;
use crate::state::Shell;
use crate::theme::{self, font, radius, sp};
use crate::ui::charts::progress_bar;
use crate::ui::chrome::{notice, NoticeKind};
use crate::ui::controls::{icon_button, spinner, toggle, Btn};
use crate::ui::label::path_label;
use crate::ui::motion;
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
        let closing = self.sheet_closing;
        let opened = self.sheet_opened;
        // The scrim fades on its own layer, so the sheet can move with its own timing above it.
        let scrim = div()
            .absolute()
            .top(px(0.))
            .left(px(0.))
            .size_full()
            .bg(theme::scrim())
            .with_animation(
                ElementId::NamedInteger(
                    if closing { "scrim-out" } else { "scrim-in" }.into(),
                    opened,
                ),
                motion::animation(if closing {
                    motion::pace::LEAVE
                } else {
                    motion::pace::ENTER
                }),
                move |this, t| this.opacity(if closing { 1.0 - t } else { t }),
            );
        // Swapping one sheet for another (review, then cleaning, then the result) cross-fades the
        // content instead of replaying the whole entrance.
        // Not while leaving: the exit wraps the sheet in a different animation, which would
        // restart this fade from zero and blank the sheet as it goes.
        let body = div().flex().flex_col().min_h(px(0.)).child(body);
        let body = if closing {
            body.into_any_element()
        } else {
            body.with_animation(
                ElementId::NamedInteger("sheet-body".into(), serial * 1_000 + opened),
                motion::animation(motion::pace::QUICK),
                |this, t| this.opacity(t),
            )
            .into_any_element()
        };
        let sheet = col()
            .id("sheet")
            .occlude()
            .relative()
            .w(px(SHEET_WIDTH))
            .max_h(px(max_height))
            .rounded(px(radius::LG))
            .bg(theme::sheet_bg())
            .border_1()
            .border_color(theme::hairline_strong())
            .shadow(theme::sheet_shadow())
            .overflow_hidden()
            .child(body);
        let sheet = if closing {
            sheet
                .with_animation(
                    ElementId::NamedInteger("sheet-out".into(), opened),
                    motion::animation(motion::pace::LEAVE),
                    |this, t| this.opacity(1.0 - t).top(px(10.0 * t)),
                )
                .into_any_element()
        } else {
            // Arrives from a little below with a slight settle, and is fully opaque well before it
            // stops moving, so the text is readable right away.
            sheet
                .with_animation(
                    ElementId::NamedInteger("sheet-in".into(), opened),
                    motion::linear(motion::pace::ENTER + 60),
                    |this, t| {
                        let settle = motion::ease_out_back(t);
                        this.opacity((t * 3.0).min(1.0))
                            .top(px(22.0 * (1.0 - settle)))
                    },
                )
                .into_any_element()
        };
        Some(
            deferred(
                div()
                    .id("sheet-layer")
                    .occlude()
                    .absolute()
                    .top(px(0.))
                    .left(px(0.))
                    .size_full()
                    .child(scrim)
                    .child(
                        div()
                            .absolute()
                            .top(px(0.))
                            .left(px(0.))
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(sheet),
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
                    .child(spinner("clean-spin", 16.0, theme::text_2()))
                    .child(
                        div()
                            .fs(font::HEADLINE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if self.mode.is_simulated() {
                                t!("Simulando…", "Simulating…")
                            } else {
                                t!("Limpiando…", "Cleaning…")
                            }),
                    ),
            )
            .child(progress_bar(fraction, theme::primary(), 8.0))
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
                            .child(tf!("{} de {}", "{} of {}", sheet.index + 1, sheet.total)),
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
                            .child(tf!(
                                "{} procesados",
                                "{} processed",
                                human_size(sheet.bytes_done)
                            )),
                    )
                    .child(
                        Btn::secondary(if sheet.cancelling {
                            t!("Terminando el actual…", "Finishing the current one…")
                        } else {
                            t!("Cancelar", "Cancel")
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
                &tf!("¿Terminar {}?", "Quit {}?", port.process_name),
                &tf!(
                    "Escucha en el puerto {} · PID {}",
                    "Listening on port {} · PID {}",
                    port.port,
                    port.pid
                ),
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
                            .child(t!(
                                "Se le pide que termine por las buenas (SIGTERM), para que pueda cerrar y guardar lo que tenga a medias.",
                                "It is asked to quit nicely (SIGTERM), so it can close and save any unfinished work."
                            )),
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
                                    .child(div().fs(font::BODY).child(t!("Forzar (SIGKILL)", "Force (SIGKILL)")))
                                    .child(
                                        div()
                                            .fs(font::CAPTION)
                                            .text_color(theme::text_2())
                                            .child(t!(
                                                "Solo si no responde: lo corta en seco y puede perder lo que no haya guardado.",
                                                "Only if it doesn't respond: it stops at once, and unsaved work may be lost."
                                            )),
                                    ),
                            ),
                    )
                    .when(simulated, |this| {
                        this.child(notice(
                            NoticeKind::Info,
                            t!(
                                "SIMULACIÓN: no se va a enviar ninguna señal",
                                "SIMULATION: no signal will be sent"
                            ),
                            None,
                            None,
                        ))
                    })
                    .when_some(sheet.error.clone(), |this, error| {
                        this.child(notice(
                            NoticeKind::Error,
                            t!("No se ha podido terminar", "Couldn't quit the process"),
                            Some(error.into()),
                            None,
                        ))
                    }),
            )
            .child(
                row()
                    .p(sp(5.))
                    .pt(px(0.))
                    .gap(sp(2.))
                    .justify_end()
                    .child(Btn::ghost(t!("Cancelar", "Cancel")).build(
                        "terminate-cancel",
                        cx.listener(|this, _, _, cx| this.close_sheet(cx)),
                    ))
                    .child(
                        Btn::danger(if sheet.force {
                            t!("Forzar cierre", "Force quit")
                        } else {
                            t!("Terminar", "Quit")
                        })
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
