use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::io::{self, IsTerminal};
use std::time::Duration;

use super::Theme;

const UNICODE_TICKS: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ";
const ASCII_TICKS: &str = "|/-\\ ";

fn style(theme: Theme, template: &str) -> ProgressStyle {
    let ticks = if theme.unicode() {
        UNICODE_TICKS
    } else {
        ASCII_TICKS
    };
    ProgressStyle::with_template(template)
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .tick_chars(ticks)
}

pub struct Spinner(Option<ProgressBar>);

impl Spinner {
    pub fn set_message(&self, message: &str) {
        if let Some(bar) = &self.0 {
            bar.set_message(message.to_string());
        }
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        if let Some(bar) = self.0.take() {
            bar.finish_and_clear();
        }
    }
}

pub fn spinner(message: &str, quiet: bool) -> Spinner {
    if quiet || !io::stderr().is_terminal() {
        return Spinner(None);
    }
    let theme = Theme::stderr();
    let template = if theme.color() {
        "{spinner:.cyan.bold} {msg} {elapsed:.dim}"
    } else {
        "{spinner} {msg} {elapsed}"
    };
    let bar = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr());
    bar.set_style(style(theme, template));
    bar.set_message(message.to_string());
    bar.enable_steady_tick(Duration::from_millis(80));
    Spinner(Some(bar))
}
