//! Light and dark. The theme follows macOS unless the user picked a side in Settings.

use gpui::{Context, Window, WindowAppearance};
use hm_core::Appearance;

use crate::state::Shell;
use crate::theme;

impl Shell {
    /// Applies the current choice and keeps following the system while it says to.
    pub fn watch_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_appearance(window);
        cx.observe_window_appearance(window, |this, window, cx| {
            this.apply_appearance(window);
            cx.notify();
        })
        .detach();
    }

    pub fn set_appearance(
        &mut self,
        value: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_config(cx, |config| config.appearance = value);
        self.apply_appearance(window);
        window.refresh();
    }

    fn apply_appearance(&self, window: &Window) {
        let dark = match self.ctx.config.appearance {
            Appearance::System => matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
            Appearance::Light => false,
            Appearance::Dark => true,
        };
        theme::set_dark(dark);
    }
}
