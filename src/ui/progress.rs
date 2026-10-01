//! Spinners, phase bars, and a download bar, drawn on stderr in the brand green.
//!
//! Every animated line is kept narrower than the terminal: a line that wraps cannot be
//! redrawn in place with `\r`, so messages are shortened with an ellipsis up front. Finished
//! work is cleared and replaced by a static `✔`/`✖` line on stdout, which also lands in logs
//! when stderr is not a terminal and nothing is animated.

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle};
use std::io::{self, IsTerminal, Read};
use std::time::{Duration, Instant};

use super::layout::keep_head;
use super::signal::LinesGuard;
use super::{Theme, ok_line, term};

const UNICODE_TICKS: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ";
const ASCII_TICKS: &str = "|/-\\ ";
const UNICODE_BAR: &str = "━╸─";
const ASCII_BAR: &str = "=> ";
const TICK: Duration = Duration::from_millis(80);

/// Columns a spinner line uses besides its message: glyph, spaces, and `{time}` (≤ 7).
const SPINNER_CHROME: usize = 2 + 1 + 7;
/// Columns a transfer line needs besides its message: glyph, a short bar, and byte counts.
const TRANSFER_CHROME: usize = 2 + 1 + 12 + 1 + 21;

fn style(theme: Theme, template: &str) -> ProgressStyle {
    let (ticks, bar) = if theme.unicode() {
        (UNICODE_TICKS, UNICODE_BAR)
    } else {
        (ASCII_TICKS, ASCII_BAR)
    };
    ProgressStyle::with_template(template)
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .with_key(
            "time",
            |state: &ProgressState, out: &mut dyn std::fmt::Write| {
                let _ = out.write_str(&elapsed_text(state.elapsed()));
            },
        )
        .with_key(
            "left",
            |state: &ProgressState, out: &mut dyn std::fmt::Write| {
                let _ = out.write_str(&elapsed_text(state.eta()));
            },
        )
        .tick_chars(ticks)
        .progress_chars(bar)
}

fn spinner_style(theme: Theme) -> ProgressStyle {
    if theme.color() {
        style(theme, "{spinner:.green.bold} {msg} {time:.dim}")
    } else {
        style(theme, "{spinner} {msg} {time}")
    }
}

fn transfer_style(theme: Theme, known: bool) -> ProgressStyle {
    match (known, theme.color(), theme.unicode()) {
        (true, true, _) => style(
            theme,
            "{spinner:.green.bold} {msg} {wide_bar:.green/dim} {bytes:>9}/{total_bytes:<9}",
        ),
        (true, false, true) => style(
            theme,
            "{spinner} {msg} {wide_bar} {bytes:>9}/{total_bytes:<9}",
        ),
        (true, false, false) => style(
            theme,
            "{spinner} {msg} [{wide_bar}] {bytes:>9}/{total_bytes:<9}",
        ),
        (false, true, _) => style(theme, "{spinner:.green.bold} {msg} {bytes:.dim}"),
        (false, false, _) => style(theme, "{spinner} {msg} {bytes}"),
    }
}

/// Shortens a message so `chrome + message` fits in `total` columns.
pub fn fit_message(theme: Theme, message: &str, chrome: usize, total: usize) -> String {
    let room = total.saturating_sub(chrome + 1).max(8);
    keep_head(message, room, theme.icons().ellipsis)
}

/// `340ms`, `4.2s`, `38s`, `2m 05s`, `1h 02m`: precise while short, coarser as it grows. Live
/// timers and finished-step lines both use it.
pub fn elapsed_text(elapsed: Duration) -> String {
    let millis = elapsed.as_millis();
    if millis < 1_000 {
        format!("{millis}ms")
    } else if millis < 10_000 {
        format!("{:.1}s", (millis / 100) as f64 / 10.0)
    } else if millis < 60_000 {
        format!("{}s", millis / 1_000)
    } else if millis < 3_600_000 {
        format!("{}m {:02}s", millis / 60_000, (millis % 60_000) / 1_000)
    } else {
        format!(
            "{}h {:02}m",
            millis / 3_600_000,
            (millis % 3_600_000) / 60_000
        )
    }
}

/// A finished-step line: `✔ message  1.2s` or `✖ message  1.2s`.
pub fn done_line(theme: Theme, ok: bool, message: &str, elapsed: Duration) -> String {
    let timing = theme.dim(elapsed_text(elapsed));
    if ok {
        format!("{}  {timing}", ok_line(theme, message))
    } else {
        format!(
            "{} {message}  {timing}",
            theme.paint(theme.icons().cross, owo_colors::Style::new().bold().red())
        )
    }
}

fn animated(quiet: bool) -> bool {
    !quiet && io::stderr().is_terminal()
}

pub struct Spinner {
    theme: Theme,
    width: usize,
    started: Instant,
    bar: Option<ProgressBar>,
    _lines: Option<LinesGuard>,
}

impl Spinner {
    fn on(theme: Theme, message: &str, target: ProgressDrawTarget, width: usize) -> Self {
        let bar = ProgressBar::with_draw_target(None, target);
        bar.set_style(spinner_style(theme));
        bar.set_message(fit_message(theme, message, SPINNER_CHROME, width));
        bar.enable_steady_tick(TICK);
        Self {
            theme,
            width,
            started: Instant::now(),
            bar: Some(bar),
            _lines: Some(LinesGuard::new(1)),
        }
    }

    fn hidden(theme: Theme) -> Self {
        Self {
            theme,
            width: term::FALLBACK_WIDTH,
            started: Instant::now(),
            bar: None,
            _lines: None,
        }
    }

    pub fn set_message(&self, message: &str) {
        if let Some(bar) = &self.bar {
            bar.set_message(fit_message(self.theme, message, SPINNER_CHROME, self.width));
        }
    }

    pub fn suspend<R>(&self, work: impl FnOnce() -> R) -> R {
        match &self.bar {
            Some(bar) => bar.suspend(work),
            None => work(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    fn clear(&mut self) {
        if let Some(bar) = self.bar.take() {
            bar.finish_and_clear();
        }
        self._lines = None;
    }

    /// Clears the spinner and prints `✔ message  1.2s` on stdout.
    pub fn success(mut self, message: &str) {
        self.clear();
        println!(
            "{}",
            done_line(Theme::stdout(), true, message, self.elapsed())
        );
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.clear();
    }
}

/// A one-line spinner on stderr. Hidden when `quiet` or when stderr is not a terminal.
pub fn spinner(message: &str, quiet: bool) -> Spinner {
    let theme = Theme::stderr();
    if !animated(quiet) {
        return Spinner::hidden(theme);
    }
    Spinner::on(
        theme,
        message,
        ProgressDrawTarget::stderr(),
        term::stderr_width(),
    )
}

/// One spinner line per parallel task. Each line turns into `✔ label  1.2s` (or `✖`) as its
/// task finishes; [`Bars::summary`] prints the same lines statically afterwards.
pub struct Bars {
    theme: Theme,
    labels: Vec<String>,
    started: Instant,
    outcomes: std::sync::Mutex<Vec<Option<(bool, Duration)>>>,
    bars: Vec<ProgressBar>,
    multi: Option<MultiProgress>,
    _lines: Option<LinesGuard>,
}

impl Bars {
    pub fn new(labels: &[&str]) -> Self {
        let theme = Theme::stderr();
        if !animated(false) {
            return Self::build(theme, labels, None, term::FALLBACK_WIDTH);
        }
        Self::build(
            theme,
            labels,
            Some(ProgressDrawTarget::stderr()),
            term::stderr_width(),
        )
    }

    fn build(
        theme: Theme,
        labels: &[&str],
        target: Option<ProgressDrawTarget>,
        width: usize,
    ) -> Self {
        let labels: Vec<String> = labels
            .iter()
            .map(|label| fit_message(theme, label, SPINNER_CHROME, width))
            .collect();
        let outcomes = std::sync::Mutex::new(vec![None; labels.len()]);
        let Some(target) = target else {
            return Self {
                theme,
                bars: labels.iter().map(|_| ProgressBar::hidden()).collect(),
                labels,
                started: Instant::now(),
                outcomes,
                multi: None,
                _lines: None,
            };
        };
        let multi = MultiProgress::with_draw_target(target);
        let bars = labels
            .iter()
            .map(|label| {
                let bar = multi.add(ProgressBar::new_spinner());
                bar.set_style(spinner_style(theme));
                bar.set_message(label.clone());
                bar.enable_steady_tick(TICK);
                bar
            })
            .collect();
        Self {
            theme,
            _lines: Some(LinesGuard::new(labels.len())),
            labels,
            started: Instant::now(),
            outcomes,
            bars,
            multi: Some(multi),
        }
    }

    /// Marks task `index` as finished; its line stops spinning and shows the outcome.
    pub fn finish(&self, index: usize, ok: bool) {
        let elapsed = self.started.elapsed();
        if let Ok(mut outcomes) = self.outcomes.lock()
            && let Some(slot) = outcomes.get_mut(index)
        {
            *slot = Some((ok, elapsed));
        }
        if let (Some(bar), Some(label)) = (self.bars.get(index), self.labels.get(index)) {
            bar.set_style(style(self.theme, "{msg}"));
            bar.finish_with_message(done_line(self.theme, ok, label, elapsed));
        }
    }

    /// Clears every line, leaving the screen as it was before [`Bars::new`].
    pub fn clear(&self) {
        for bar in &self.bars {
            bar.finish_and_clear();
        }
        if let Some(multi) = &self.multi {
            let _ = multi.clear();
        }
    }

    /// Static `✔ label  1.2s` lines for every finished task, for stdout.
    pub fn summary(&self, theme: Theme) -> Vec<String> {
        let outcomes = self
            .outcomes
            .lock()
            .map(|outcomes| outcomes.clone())
            .unwrap_or_default();
        self.labels
            .iter()
            .zip(outcomes)
            .filter_map(|(label, outcome)| {
                outcome.map(|(ok, elapsed)| done_line(theme, ok, label, elapsed))
            })
            .collect()
    }
}

impl Drop for Bars {
    fn drop(&mut self) {
        self.clear();
    }
}

/// A byte-counting download bar. Falls back to a spinner with a byte count when the size is
/// unknown, and draws nothing when stderr is not a terminal.
pub struct Transfer {
    started: Instant,
    bar: Option<ProgressBar>,
    _lines: Option<LinesGuard>,
}

impl Transfer {
    fn on(
        theme: Theme,
        label: &str,
        total: Option<u64>,
        target: ProgressDrawTarget,
        width: usize,
    ) -> Self {
        let bar = ProgressBar::with_draw_target(total, target);
        bar.set_style(transfer_style(theme, total.is_some()));
        bar.set_message(fit_message(theme, label, TRANSFER_CHROME, width));
        bar.enable_steady_tick(TICK);
        Self {
            started: Instant::now(),
            bar: Some(bar),
            _lines: Some(LinesGuard::new(1)),
        }
    }

    pub fn inc(&self, bytes: u64) {
        if let Some(bar) = &self.bar {
            bar.inc(bytes);
        }
    }

    /// Wraps a reader so every read advances the bar.
    pub fn wrap<'a, R: Read + 'a>(&self, read: R) -> Box<dyn Read + 'a> {
        match &self.bar {
            Some(bar) => Box::new(bar.wrap_read(read)),
            None => Box::new(read),
        }
    }

    /// Clears the bar and prints `✔ message  1.2s` on stdout.
    pub fn success(mut self, message: &str) {
        if let Some(bar) = self.bar.take() {
            bar.finish_and_clear();
        }
        self._lines = None;
        println!(
            "{}",
            done_line(Theme::stdout(), true, message, self.started.elapsed())
        );
    }
}

impl Drop for Transfer {
    fn drop(&mut self) {
        if let Some(bar) = self.bar.take() {
            bar.finish_and_clear();
        }
    }
}

pub fn transfer(label: &str, total: Option<u64>) -> Transfer {
    let theme = Theme::stderr();
    if !animated(false) {
        return Transfer {
            started: Instant::now(),
            bar: None,
            _lines: None,
        };
    }
    Transfer::on(
        theme,
        label,
        total,
        ProgressDrawTarget::stderr(),
        term::stderr_width(),
    )
}

fn dual_styles(theme: Theme) -> (ProgressStyle, ProgressStyle) {
    if theme.color() {
        (
            style(
                theme,
                "{spinner:.green.bold} {prefix:<6.bold.green} {bar:32.green/dim} {pos:>4}/{len:<4} {time:>6.dim} {wide_msg}",
            ),
            style(
                theme,
                "  {spinner:.green} {prefix:<4.dim} {bar:32.green/dim} {pos:>4}/{len:<4} eta {left:.yellow} {wide_msg:.dim}",
            ),
        )
    } else {
        (
            style(
                theme,
                "{spinner} {prefix:<6} [{bar:32}] {pos:>4}/{len:<4} {time:>6} {wide_msg}",
            ),
            style(
                theme,
                "  {spinner} {prefix:<4} [{bar:32}] {pos:>4}/{len:<4} eta {left} {wide_msg}",
            ),
        )
    }
}

pub struct DualProgress {
    theme: Theme,
    steps: ProgressBar,
    rows: ProgressBar,
    _multi: Option<MultiProgress>,
    _lines: Option<LinesGuard>,
}

impl DualProgress {
    pub fn new(label: &str, steps: u64, quiet: bool) -> Self {
        let theme = Theme::stderr();
        if !animated(quiet) {
            let steps =
                ProgressBar::with_draw_target(Some(steps.max(1)), ProgressDrawTarget::hidden());
            let rows = ProgressBar::with_draw_target(Some(1), ProgressDrawTarget::hidden());
            return Self {
                theme,
                steps,
                rows,
                _multi: None,
                _lines: None,
            };
        }
        let (style_steps, style_rows) = dual_styles(theme);
        let multi = MultiProgress::with_draw_target(ProgressDrawTarget::stderr());
        let steps_bar = multi.add(ProgressBar::new(steps.max(1)));
        let rows_bar = multi.add(ProgressBar::new(1));
        steps_bar.set_style(style_steps);
        steps_bar.set_prefix(label.to_string());
        rows_bar.set_style(style_rows);
        rows_bar.set_prefix("rows");
        steps_bar.enable_steady_tick(Duration::from_millis(100));
        rows_bar.enable_steady_tick(Duration::from_millis(100));
        Self {
            theme,
            steps: steps_bar,
            rows: rows_bar,
            _multi: Some(multi),
            _lines: Some(LinesGuard::new(2)),
        }
    }

    pub fn step(&self, index: u64, message: &str) {
        self.steps.set_position(index);
        let styled = message
            .split(' ')
            .map(|word| self.theme.token(word))
            .collect::<Vec<_>>()
            .join(" ");
        self.steps.set_message(styled);
    }

    pub fn rows(&self, done: u64, total: u64, message: &str) {
        self.rows.set_length(total.max(1));
        self.rows.set_position(done.min(total.max(1)));
        self.rows.set_message(message.to_string());
    }

    pub fn finish(&self) {
        self.steps.finish_and_clear();
        self.rows.finish_and_clear();
    }
}

#[cfg(test)]
pub(crate) mod screen {
    //! A tiny virtual terminal with autowrap: enough of a VT to replay what indicatif draws.
    //! indicatif pads every line to the full width and lets the terminal wrap to the next
    //! row, so a line wider than the terminal shows up here as an extra row.

    use indicatif::TermLike;
    use std::io;
    use std::sync::{Arc, Mutex};

    use crate::ui::layout::strip_ansi;
    use unicode_width::UnicodeWidthChar;

    #[derive(Debug, Default)]
    pub struct State {
        pub rows: Vec<Vec<char>>,
        pub row: usize,
        pub col: usize,
    }

    #[derive(Debug, Clone)]
    pub struct Screen {
        pub cols: u16,
        pub state: Arc<Mutex<State>>,
    }

    impl Screen {
        pub fn new(cols: u16) -> Self {
            Self {
                cols,
                state: Arc::new(Mutex::new(State {
                    rows: vec![Vec::new()],
                    ..State::default()
                })),
            }
        }

        /// Non-blank rows, trailing spaces trimmed.
        pub fn visible(&self) -> Vec<String> {
            let state = self.state.lock().unwrap();
            state
                .rows
                .iter()
                .map(|row| row.iter().collect::<String>().trim_end().to_string())
                .filter(|row| !row.is_empty())
                .collect()
        }

        fn with<R>(&self, work: impl FnOnce(&mut State) -> R) -> R {
            work(&mut self.state.lock().unwrap())
        }
    }

    impl State {
        fn ensure(&mut self, row: usize) {
            while self.rows.len() <= row {
                self.rows.push(Vec::new());
            }
        }

        fn put(&mut self, ch: char, cols: usize) {
            match ch {
                '\r' => self.col = 0,
                '\n' => {
                    self.row += 1;
                    self.col = 0;
                    self.ensure(self.row);
                }
                _ => {
                    let wide = ch.width().unwrap_or(0);
                    if self.col + wide > cols {
                        self.row += 1;
                        self.col = 0;
                        self.ensure(self.row);
                    }
                    let row = &mut self.rows[self.row];
                    while row.len() < self.col {
                        row.push(' ');
                    }
                    if row.len() == self.col {
                        row.push(ch);
                    } else {
                        row[self.col] = ch;
                    }
                    self.col += wide.max(1);
                }
            }
        }
    }

    impl TermLike for Screen {
        fn width(&self) -> u16 {
            self.cols
        }

        fn move_cursor_up(&self, n: usize) -> io::Result<()> {
            self.with(|state| state.row = state.row.saturating_sub(n));
            Ok(())
        }

        fn move_cursor_down(&self, n: usize) -> io::Result<()> {
            self.with(|state| {
                state.row += n;
                state.ensure(state.row);
            });
            Ok(())
        }

        fn move_cursor_right(&self, n: usize) -> io::Result<()> {
            self.with(|state| state.col += n);
            Ok(())
        }

        fn move_cursor_left(&self, n: usize) -> io::Result<()> {
            self.with(|state| state.col = state.col.saturating_sub(n));
            Ok(())
        }

        fn write_line(&self, s: &str) -> io::Result<()> {
            self.write_str(s)?;
            self.write_str("\n")
        }

        fn write_str(&self, s: &str) -> io::Result<()> {
            let cols = self.cols as usize;
            self.with(|state| {
                for ch in strip_ansi(s).chars() {
                    state.put(ch, cols);
                }
            });
            Ok(())
        }

        fn clear_line(&self) -> io::Result<()> {
            self.with(|state| {
                let row = state.row;
                state.rows[row].clear();
                state.col = 0;
            });
            Ok(())
        }

        fn flush(&self) -> io::Result<()> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::screen::Screen;
    use super::*;
    use crate::ui::layout::strip_ansi;

    fn target(screen: &Screen) -> ProgressDrawTarget {
        ProgressDrawTarget::term_like_with_hz(Box::new(screen.clone()), 100)
    }

    fn themes() -> [Theme; 3] {
        [Theme::plain(), Theme::colored(), Theme::ascii()]
    }

    #[test]
    fn templates_parse_in_every_theme() {
        for theme in themes() {
            let (steps, rows) = dual_styles(theme);
            let bar = ProgressBar::hidden();
            bar.set_style(steps);
            bar.set_style(rows);
            bar.set_style(spinner_style(theme));
            bar.set_style(transfer_style(theme, true));
            bar.set_style(transfer_style(theme, false));
        }
        let hidden = DualProgress::new("build", 3, true);
        hidden.step(1, "php /tmp/app");
        hidden.rows(5, 10, "rows");
        hidden.finish();
        let bars = Bars::new(&["cache", "redis"]);
        bars.finish(0, true);
        bars.clear();
    }

    #[test]
    fn spinner_lines_fit_the_terminal_and_leave_no_residue() {
        let long = "Downloading so-aarch64-apple-darwin.tar.gz from the latest GitHub release of shellops-cli";
        for theme in themes() {
            for cols in [30_u16, 60, 80] {
                let screen = Screen::new(cols);
                let spinner = Spinner::on(theme, long, target(&screen), cols as usize);
                assert!(spinner._lines.is_some(), "Ctrl-C knows about the line");
                spinner.bar.as_ref().unwrap().force_draw();
                spinner.set_message(long);
                spinner.bar.as_ref().unwrap().force_draw();
                let drawn = screen.visible();
                assert_eq!(drawn.len(), 1, "one row, no wrap at {cols}: {drawn:?}");
                assert!(drawn[0].contains(theme.icons().ellipsis), "{drawn:?}");
                drop(spinner);
                assert!(
                    screen.visible().is_empty(),
                    "residue: {:?}",
                    screen.visible()
                );
            }
        }
    }

    #[test]
    fn transfer_bar_fits_the_terminal() {
        for theme in themes() {
            for cols in [40_u16, 60, 80, 120] {
                for total in [Some(9_000_000), None] {
                    let screen = Screen::new(cols);
                    let transfer = Transfer::on(
                        theme,
                        "so 2.0.0 (so-aarch64-apple-darwin.tar.gz)",
                        total,
                        target(&screen),
                        cols as usize,
                    );
                    transfer.inc(3_500_000);
                    transfer.bar.as_ref().unwrap().force_draw();
                    let drawn = screen.visible();
                    assert_eq!(drawn.len(), 1, "one row, no wrap at {cols}: {drawn:?}");
                    drop(transfer);
                    assert!(screen.visible().is_empty());
                }
            }
        }
    }

    #[test]
    fn phase_bars_finish_in_place_then_clear() {
        for theme in themes() {
            let screen = Screen::new(60);
            let bars = Bars::build(
                theme,
                &["cache", "redis", "runtime"],
                Some(target(&screen)),
                60,
            );
            for bar in &bars.bars {
                bar.force_draw();
            }
            bars.finish(1, true);
            bars.finish(0, false);
            for bar in &bars.bars {
                bar.force_draw();
            }
            let drawn = screen.visible();
            assert_eq!(drawn.len(), 3, "one row per phase: {drawn:?}");
            assert!(
                drawn
                    .iter()
                    .any(|line| line.starts_with(&format!("{} redis", theme.icons().check))),
                "{drawn:?}"
            );
            assert!(
                drawn
                    .iter()
                    .any(|line| line.starts_with(&format!("{} cache", theme.icons().cross))),
                "{drawn:?}"
            );
            let summary: Vec<String> = bars
                .summary(Theme::plain())
                .iter()
                .map(|line| strip_ansi(line))
                .collect();
            assert_eq!(summary.len(), 2, "runtime never finished: {summary:?}");
            assert!(summary[0].starts_with("✖ cache  "));
            assert!(summary[1].starts_with("✔ redis  "));
            bars.clear();
            assert!(
                screen.visible().is_empty(),
                "residue: {:?}",
                screen.visible()
            );
        }
    }

    #[test]
    fn elapsed_is_short_and_human() {
        assert_eq!(elapsed_text(Duration::from_millis(420)), "420ms");
        assert_eq!(elapsed_text(Duration::from_millis(1_250)), "1.2s");
        assert_eq!(elapsed_text(Duration::from_millis(9_999)), "9.9s");
        assert_eq!(elapsed_text(Duration::from_secs(14)), "14s");
        assert_eq!(elapsed_text(Duration::from_secs(125)), "2m 05s");
        assert_eq!(elapsed_text(Duration::from_secs(3_725)), "1h 02m");
        assert_eq!(
            strip_ansi(&done_line(
                Theme::colored(),
                true,
                "Fetched origin",
                Duration::from_millis(1_200)
            )),
            "✔ Fetched origin  1.2s"
        );
        assert_eq!(
            done_line(Theme::ascii(), false, "cache", Duration::from_secs(3)),
            "x cache  3.0s"
        );
    }

    #[test]
    fn fitted_messages_respect_the_width() {
        let theme = Theme::plain();
        let text = fit_message(theme, &"x".repeat(200), SPINNER_CHROME, 40);
        assert!(crate::ui::layout::width(&text) + SPINNER_CHROME < 40);
        assert_eq!(fit_message(theme, "short", SPINNER_CHROME, 80), "short");
    }
}
