use owo_colors::Style;
use std::fmt;
use std::io::{self, IsTerminal};

use super::layout::keep_head;
use super::signal::PromptGuard;
use super::{Theme, hinted, term};

/// Columns dialoguer draws in front of a menu item (`❯ ` or `> `), plus one spare so the
/// widest item never touches the last column and wraps.
const ITEM_CHROME: usize = 3;

struct ConfirmTheme(Theme);

impl dialoguer::theme::Theme for ConfirmTheme {
    fn format_confirm_prompt(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        default: Option<bool>,
    ) -> fmt::Result {
        let theme = self.0;
        let choices = match default {
            Some(true) => format!("[{}/n]", theme.bold("Y")),
            Some(false) => format!("[y/{}]", theme.bold("N")),
            None => "[y/n]".to_string(),
        };
        write!(
            f,
            "{} {} {} ",
            theme.paint("?", Style::new().bold().green()),
            theme.bold(prompt),
            theme.dim(choices)
        )
    }

    fn format_confirm_prompt_selection(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        selection: Option<bool>,
    ) -> fmt::Result {
        let theme = self.0;
        let icons = theme.icons();
        match selection {
            Some(true) => write!(
                f,
                "{} {}{}{}",
                theme.good(icons.check),
                theme.bold(prompt),
                theme.sep(),
                theme.good("yes")
            ),
            Some(false) => write!(
                f,
                "{} {}{}{}",
                theme.bad(icons.cross),
                theme.bold(prompt),
                theme.sep(),
                theme.bad("no")
            ),
            None => write!(f, "{} {}", theme.dim("?"), theme.bold(prompt)),
        }
    }
}

/// dialoguer's colorful theme in the ShellOps palette: green accents, the same `✔`/`✖`
/// glyphs as the rest of the output.
fn colorful() -> dialoguer::theme::ColorfulTheme {
    use console::{Style as ConsoleStyle, style};
    let base = dialoguer::theme::ColorfulTheme::default();
    dialoguer::theme::ColorfulTheme {
        defaults_style: ConsoleStyle::new().for_stderr().green(),
        prompt_prefix: style("?".to_string()).for_stderr().green().bold(),
        error_prefix: style("✖".to_string()).for_stderr().red().bold(),
        active_item_style: ConsoleStyle::new().for_stderr().green().bold(),
        unchecked_item_prefix: style("○".to_string()).for_stderr().dim(),
        ..base
    }
}

/// Runs a prompt with the themed renderer while Ctrl-C knows to restore the cursor and echo.
fn with_prompt_theme<R>(run: impl FnOnce(&dyn dialoguer::theme::Theme) -> R) -> R {
    let _guard = PromptGuard::new();
    let theme = Theme::stderr();
    if theme.color() && theme.unicode() {
        run(&colorful())
    } else {
        run(&dialoguer::theme::SimpleTheme)
    }
}

/// Shortens menu items so none is wider than the terminal: a wrapped item breaks
/// dialoguer's redraw and leaves stale copies of the menu on screen.
pub fn fit_items<S: AsRef<str>>(theme: Theme, items: &[S], total: usize) -> Vec<String> {
    let room = total.saturating_sub(ITEM_CHROME).max(8);
    items
        .iter()
        .map(|item| keep_head(item.as_ref(), room, theme.icons().ellipsis))
        .collect()
}

fn fitted<S: AsRef<str>>(items: &[S]) -> Vec<String> {
    fit_items(Theme::stderr(), items, term::stderr_width())
}

pub fn confirm_default(prompt: &str, default_yes: bool) -> anyhow::Result<bool> {
    if !io::stdin().is_terminal() {
        return Err(hinted(
            prompt,
            "Run this command in a terminal so it can ask.",
        ));
    }
    let _guard = PromptGuard::new();
    let theme = ConfirmTheme(Theme::stderr());
    let choice = dialoguer::Confirm::with_theme(&theme)
        .with_prompt(prompt)
        .default(default_yes)
        .interact()?;
    Ok(choice)
}

pub fn confirm(prompt: &str, yes: bool) -> anyhow::Result<bool> {
    if yes {
        return Ok(true);
    }
    confirm_default(prompt, false)
}

pub fn select(prompt: &str, items: &[&str]) -> anyhow::Result<usize> {
    if !io::stdin().is_terminal() {
        anyhow::bail!("pass arguments, or run in a terminal");
    }
    let items = fitted(items);
    Ok(with_prompt_theme(|theme| {
        dialoguer::Select::with_theme(theme)
            .with_prompt(prompt)
            .items(&items)
            .default(0)
            .interact()
    })?)
}

pub fn input(prompt: &str, default: Option<&str>) -> anyhow::Result<String> {
    if !io::stdin().is_terminal() {
        anyhow::bail!("run this command in a terminal");
    }
    Ok(with_prompt_theme(|theme| {
        let mut input = dialoguer::Input::<String>::with_theme(theme).with_prompt(prompt);
        if let Some(default) = default {
            input = input.default(default.to_string());
        }
        input.interact_text()
    })?)
}

pub fn password(prompt: &str) -> anyhow::Result<String> {
    if !io::stdin().is_terminal() {
        anyhow::bail!("run this command in a terminal");
    }
    Ok(with_prompt_theme(|theme| {
        dialoguer::Password::with_theme(theme)
            .with_prompt(prompt)
            .interact()
    })?)
}

pub fn fuzzy(prompt: &str, items: &[String], default: usize) -> anyhow::Result<usize> {
    if !io::stdin().is_terminal() {
        anyhow::bail!("run this command in a terminal");
    }
    let items = fitted(items);
    Ok(with_prompt_theme(|theme| {
        dialoguer::FuzzySelect::with_theme(theme)
            .with_prompt(prompt)
            .items(&items)
            .default(default)
            .interact()
    })?)
}

pub fn multi_select(prompt: &str, items: &[String]) -> anyhow::Result<Vec<usize>> {
    if !io::stdin().is_terminal() {
        anyhow::bail!("pass arguments, or run {prompt} in a terminal");
    }
    let items = fitted(items);
    let picked = with_prompt_theme(|theme| {
        dialoguer::MultiSelect::with_theme(theme)
            .with_prompt(prompt)
            .items(&items)
            .interact()
    })?;
    Ok(picked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::layout::width;

    #[test]
    fn menu_items_never_wrap() {
        let items = [
            "SHA256:Zm9vYmFyYmF6cXV4cXV1eGNvcmdlZ3JhdWx0Z2FycGx5  ~/.ssh/id_ed25519_work_laptop.pub",
            "Use gh auth token",
        ];
        for total in [40, 60, 80] {
            for theme in [Theme::plain(), Theme::ascii()] {
                let fitted = fit_items(theme, &items, total);
                for item in &fitted {
                    assert!(width(item) + ITEM_CHROME <= total, "{item:?} at {total}");
                }
                assert_eq!(fitted[1], "Use gh auth token");
            }
        }
    }

    #[test]
    fn confirm_prompt_uses_the_shared_glyphs() {
        let theme = ConfirmTheme(Theme::plain());
        let mut out = String::new();
        dialoguer::theme::Theme::format_confirm_prompt(&theme, &mut out, "Continue?", Some(true))
            .unwrap();
        assert_eq!(out, "? Continue? [Y/n] ");
        let mut done = String::new();
        dialoguer::theme::Theme::format_confirm_prompt_selection(
            &theme,
            &mut done,
            "Continue?",
            Some(false),
        )
        .unwrap();
        assert_eq!(done, "✖ Continue? · no");
        let colorful = colorful();
        assert!(colorful.error_prefix.to_string().contains('✖'));
    }
}
