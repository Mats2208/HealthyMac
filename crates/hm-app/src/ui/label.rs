//! One line of text that always fits: it is measured and cut at paint time, once its real width is
//! known.
//!
//! GPUI's own `text_ellipsis` cannot be used for this. Its measure closure keeps the runs it shortened
//! on an earlier pass, so measuring twice at different widths slices non-ASCII text mid-character and
//! panics. Without wrapping it dodges that by caching the first measurement, which flex layout often
//! takes with no width at all, and the text stays stuck as "…". This element sidesteps both.

use gpui::{
    div, point, prelude::*, px, AbsoluteLength, App, AvailableSpace, Bounds, DefiniteLength, Div,
    Element, ElementId, GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, Length,
    Pixels, ShapedLine, SharedString, Size, Style, StyleRefinement, TextAlign, TextStyle, Window,
};

use super::f;
use crate::logic::truncate::{fit_end, fit_middle};

#[derive(Clone, Copy)]
enum Cut {
    End,
    Middle,
}

/// A single-line text box: style it like a `div`, and give it text with `.child`.
pub struct Clip {
    container: Div,
    cut: Cut,
    text: Option<SharedString>,
}

/// Cuts from the end, with an ellipsis.
pub fn clip() -> Clip {
    Clip {
        container: div().flex().flex_row().min_w(px(0.)).overflow_hidden(),
        cut: Cut::End,
        text: None,
    }
}

/// Cuts from the middle, for paths: both the root and the file name stay readable.
pub fn clip_middle() -> Clip {
    Clip {
        cut: Cut::Middle,
        ..clip()
    }
}

/// A path in small quiet type, the usual subtitle of a row.
pub fn path_label(text: impl Into<SharedString>, size: f32, color: Hsla) -> Clip {
    use super::TextExt;
    clip_middle()
        .w_full()
        .fs(size)
        .text_color(color)
        .child(text)
}

impl Clip {
    pub fn child(mut self, text: impl Into<SharedString>) -> Self {
        self.text = Some(text.into());
        self
    }
}

impl Styled for Clip {
    fn style(&mut self) -> &mut StyleRefinement {
        self.container.style()
    }
}

impl IntoElement for Clip {
    type Element = Div;

    fn into_element(self) -> Div {
        self.container.child(Line {
            text: self.text.unwrap_or_default(),
            cut: self.cut,
        })
    }
}

struct Line {
    text: SharedString,
    cut: Cut,
}

impl IntoElement for Line {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

fn shape(text: &str, style: &TextStyle, window: &Window) -> ShapedLine {
    let font_size = style.font_size.to_pixels(window.rem_size());
    let run = style.to_run(text.len());
    window.text_system().shape_line(
        SharedString::from(text.to_string()),
        font_size,
        &[run],
        None,
    )
}

fn line_height(style: &TextStyle, window: &Window) -> Pixels {
    let font_size: AbsoluteLength = style.font_size;
    style.line_height.to_pixels(font_size, window.rem_size())
}

impl Element for Line {
    type RequestLayoutState = ();
    type PrepaintState = Option<ShapedLine>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, ()) {
        let text = self.text.clone();
        let text_style = window.text_style();
        let style = Style {
            flex_grow: 1.0,
            flex_shrink: 1.0,
            min_size: Size {
                width: Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(0.)))),
                height: Length::Auto,
            },
            ..Style::default()
        };
        let layout = window.request_measured_layout(style, move |known, available, window, _| {
            let height = line_height(&text_style, window);
            let width = known.width.unwrap_or_else(|| {
                let natural = shape(&text, &text_style, window).width;
                match available.width {
                    AvailableSpace::Definite(room) => natural.min(room),
                    _ => natural,
                }
            });
            Size {
                width: width.ceil(),
                height,
            }
        });
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) -> Option<ShapedLine> {
        let style = window.text_style();
        let room = f(bounds.size.width) + 0.5;
        let full = shape(&self.text, &style, window);
        if f(full.width) <= room {
            return Some(full);
        }
        let measure = |candidate: &str| f(shape(candidate, &style, window).width);
        let fitted = match self.cut {
            Cut::End => fit_end(&self.text, room, measure),
            Cut::Middle => fit_middle(&self.text, room, measure),
        };
        Some(shape(&fitted, &style, window))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        line: &mut Option<ShapedLine>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(line) = line.take() else {
            return;
        };
        let style = window.text_style();
        let spare = (bounds.size.width - line.width).max(px(0.));
        let x = match style.text_align {
            TextAlign::Left => bounds.origin.x,
            TextAlign::Center => bounds.origin.x + spare / 2.0,
            TextAlign::Right => bounds.origin.x + spare,
        };
        line.paint(
            point(x, bounds.origin.y),
            line_height(&style, window),
            window,
            cx,
        )
        .ok();
    }
}
