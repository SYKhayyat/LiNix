// src/app/ui/cockpit.rs
//
// The generation cockpit: a time-travel dashboard for LiNix's history.
//
//   ┌ Generations ─┐┌ Selected generation ───────────────┐
//   │ > g3  (now)  ││ 42 packages · git a1b2c3d            │
//   │   g2         ││ apt:curl 8.4.0                       │
//   │   g1         ││ cargo:ripgrep 14.1                   │
//   │              ││ … changes vs previous generation …  │
//   └──────────────┘└─────────────────────────────────────┘
//   ┌ Shell ───────────────────────────────────────────────┐
//   │ $ _                                                   │
//   └───────────────────────────────────────────────────────┘
//
// Left: the generation timeline. Right: the selected generation's realized package set, its
// stamped git commit, and a diff against the previous generation. Bottom: a shell line for
// running commands (linix or anything) without leaving the cockpit.
//
// The rendering/diff logic is pure and unit-tested; the ratatui event loop is a thin shell.

use crate::core::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io;

/// A decoupled, display-ready view of one generation (so the TUI never depends on the store).
#[derive(Debug, Clone)]
pub struct GenView {
    pub id: String,
    pub timestamp: String,
    pub label: String,
    pub pinned: bool,
    /// Rendered package identifiers, e.g. "apt:curl 8.4.0".
    pub packages: Vec<String>,
    pub git_commit: Option<String>,
}

/// What the cockpit asks the async caller to do after it exits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CockpitAction {
    Quit,
    /// Roll back to a generation; `with_config` also checks out its git commit.
    Rollback { id: String, with_config: bool },
}

/// One row in the left-hand timeline.
pub fn gen_row(g: &GenView) -> String {
    let pin = if g.pinned { "📌" } else { "  " };
    let label = if g.label.is_empty() {
        String::new()
    } else {
        format!("  {}", g.label)
    };
    format!("{} {}  ({} pkgs){}", pin, g.id, g.packages.len(), label)
}

/// The right-hand detail lines for a generation, plus a diff against the previous one.
pub fn detail_lines(current: &GenView, previous: Option<&GenView>) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!("Generation : {}", current.id));
    lines.push(format!("When       : {}", current.timestamp));
    if !current.label.is_empty() {
        lines.push(format!("Label      : {}", current.label));
    }
    match &current.git_commit {
        Some(c) => lines.push(format!("Git commit : {}", &c[..c.len().min(12)])),
        None => lines.push("Git commit : (config not under git)".to_string()),
    }
    lines.push(format!("Packages   : {}", current.packages.len()));
    lines.push(String::new());

    if let Some(prev) = previous {
        let (added, removed) = pkg_set_diff(&prev.packages, &current.packages);
        if added.is_empty() && removed.is_empty() {
            lines.push("No package changes vs previous generation.".to_string());
        } else {
            lines.push(format!(
                "Changes vs {} : +{} / -{}",
                prev.id,
                added.len(),
                removed.len()
            ));
            for a in &added {
                lines.push(format!("  + {}", a));
            }
            for r in &removed {
                lines.push(format!("  - {}", r));
            }
        }
        lines.push(String::new());
    }

    lines.push("Package set:".to_string());
    for p in &current.packages {
        lines.push(format!("  {}", p));
    }
    lines
}

/// Diff two package-identifier lists into (added, removed), preserving order.
pub fn pkg_set_diff(older: &[String], newer: &[String]) -> (Vec<String>, Vec<String>) {
    use std::collections::HashSet;
    let old_set: HashSet<&String> = older.iter().collect();
    let new_set: HashSet<&String> = newer.iter().collect();
    let added = newer.iter().filter(|p| !old_set.contains(*p)).cloned().collect();
    let removed = older.iter().filter(|p| !new_set.contains(*p)).cloned().collect();
    (added, removed)
}

/// Cockpit UI state.
pub struct Cockpit {
    gens: Vec<GenView>,
    list_state: ListState,
    /// The shell input buffer.
    input: String,
    /// True while the user is typing a command into the shell line.
    command_mode: bool,
    /// A transient status message (last command result, hints).
    status: String,
}

impl Cockpit {
    pub fn new(gens: Vec<GenView>) -> Self {
        let mut list_state = ListState::default();
        if !gens.is_empty() {
            list_state.select(Some(0));
        }
        Self {
            gens,
            list_state,
            input: String::new(),
            command_mode: false,
            status: "[j/k] move  [r] rollback  [R] rollback+config  [:] shell  [q] quit".into(),
        }
    }

    fn selected(&self) -> Option<&GenView> {
        self.list_state.selected().and_then(|i| self.gens.get(i))
    }

    fn selected_previous(&self) -> Option<&GenView> {
        // Generations are newest-first, so the "previous" (older) one is the next index.
        self.list_state
            .selected()
            .and_then(|i| self.gens.get(i + 1))
    }

    fn next(&mut self) {
        if self.gens.is_empty() {
            return;
        }
        let i = self.list_state.selected().map(|i| (i + 1) % self.gens.len()).unwrap_or(0);
        self.list_state.select(Some(i));
    }

    fn previous(&mut self) {
        if self.gens.is_empty() {
            return;
        }
        let i = self
            .list_state
            .selected()
            .map(|i| if i == 0 { self.gens.len() - 1 } else { i - 1 })
            .unwrap_or(0);
        self.list_state.select(Some(i));
    }

    /// Launch the cockpit; returns the action the caller should perform.
    pub fn run(&mut self) -> Result<CockpitAction> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let action = self.event_loop(&mut terminal);

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;
        action
    }

    fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<CockpitAction> {
        loop {
            terminal.draw(|f| self.draw(f))?;
            if let Event::Key(key) = event::read()? {
                if self.command_mode {
                    match key.code {
                        KeyCode::Esc => {
                            self.command_mode = false;
                            self.input.clear();
                        }
                        KeyCode::Enter => {
                            let cmd = std::mem::take(&mut self.input);
                            self.command_mode = false;
                            if !cmd.trim().is_empty() {
                                self.run_shell(terminal, &cmd)?;
                            }
                        }
                        KeyCode::Backspace => {
                            self.input.pop();
                        }
                        KeyCode::Char(c) => self.input.push(c),
                        _ => {}
                    }
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(CockpitAction::Quit),
                    KeyCode::Down | KeyCode::Char('j') => self.next(),
                    KeyCode::Up | KeyCode::Char('k') => self.previous(),
                    KeyCode::Char(':') | KeyCode::Char('/') => {
                        self.command_mode = true;
                        self.input.clear();
                    }
                    KeyCode::Char('r') => {
                        if let Some(g) = self.selected() {
                            return Ok(CockpitAction::Rollback {
                                id: g.id.clone(),
                                with_config: false,
                            });
                        }
                    }
                    KeyCode::Char('R') => {
                        if let Some(g) = self.selected() {
                            return Ok(CockpitAction::Rollback {
                                id: g.id.clone(),
                                with_config: true,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Run a command from the shell line: drop out of the alternate screen, execute it via the
    /// system shell, wait for a keypress, then restore the cockpit.
    fn run_shell(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        cmd: &str,
    ) -> Result<()> {
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;

        println!("$ {}\n", cmd);
        #[cfg(windows)]
        let status = std::process::Command::new("cmd").args(["/C", cmd]).status();
        #[cfg(not(windows))]
        let status = std::process::Command::new("sh").args(["-c", cmd]).status();
        match status {
            Ok(s) => self.status = format!("`{}` exited with {}", cmd, s),
            Err(e) => self.status = format!("`{}` failed to run: {}", cmd, e),
        }
        println!("\n[press Enter to return to the cockpit]");
        let _ = io::stdin().read_line(&mut String::new());

        enable_raw_mode()?;
        execute!(terminal.backend_mut(), EnterAlternateScreen, EnableMouseCapture)?;
        terminal.clear()?;
        Ok(())
    }

    fn draw(&mut self, f: &mut Frame) {
        // Top row (left list + right detail), then bottom shell line.
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(5), Constraint::Length(3)].as_ref())
            .split(f.size());
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(38), Constraint::Percentage(62)].as_ref())
            .split(rows[0]);

        // Left: generations timeline.
        let items: Vec<ListItem> = self
            .gens
            .iter()
            .map(|g| ListItem::new(gen_row(g)))
            .collect();
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" Generations "))
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(40, 40, 40))
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");
        f.render_stateful_widget(list, cols[0], &mut self.list_state);

        // Right: detail + diff.
        let detail = match self.selected() {
            Some(g) => detail_lines(g, self.selected_previous()).join("\n"),
            None => "No generations yet. They are created after each `sync`.".to_string(),
        };
        let detail_widget = Paragraph::new(detail)
            .block(Block::default().borders(Borders::ALL).title(" Selected generation "))
            .wrap(Wrap { trim: false });
        f.render_widget(detail_widget, cols[1]);

        // Bottom: shell line / status.
        let bottom = if self.command_mode {
            format!("$ {}\u{2588}", self.input)
        } else {
            self.status.clone()
        };
        let title = if self.command_mode { " Shell (Enter to run, Esc to cancel) " } else { " Shell " };
        let shell = Paragraph::new(bottom)
            .block(Block::default().borders(Borders::ALL).title(title));
        f.render_widget(shell, rows[1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gv(id: &str, pkgs: &[&str]) -> GenView {
        GenView {
            id: id.into(),
            timestamp: "2026-07-15T00:00:00Z".into(),
            label: String::new(),
            pinned: false,
            packages: pkgs.iter().map(|s| s.to_string()).collect(),
            git_commit: None,
        }
    }

    #[test]
    fn gen_row_shows_id_and_count() {
        let row = gen_row(&gv("g3", &["apt:curl", "cargo:rg"]));
        assert!(row.contains("g3"));
        assert!(row.contains("(2 pkgs)"));
    }

    #[test]
    fn gen_row_marks_pinned() {
        let mut g = gv("g1", &[]);
        g.pinned = true;
        assert!(gen_row(&g).contains("📌"));
    }

    #[test]
    fn pkg_set_diff_reports_added_and_removed() {
        let older = vec!["apt:curl".to_string(), "apt:nano".to_string()];
        let newer = vec!["apt:curl".to_string(), "cargo:rg".to_string()];
        let (added, removed) = pkg_set_diff(&older, &newer);
        assert_eq!(added, vec!["cargo:rg"]);
        assert_eq!(removed, vec!["apt:nano"]);
    }

    #[test]
    fn detail_lines_include_git_and_diff() {
        let mut cur = gv("g2", &["apt:curl", "cargo:rg"]);
        cur.git_commit = Some("a1b2c3d4e5f6a7b8".into());
        let prev = gv("g1", &["apt:curl", "apt:nano"]);
        let lines = detail_lines(&cur, Some(&prev));
        let joined = lines.join("\n");
        assert!(joined.contains("Git commit : a1b2c3d4e5f6")); // truncated to 12
        assert!(joined.contains("+1 / -1"));
        assert!(joined.contains("+ cargo:rg"));
        assert!(joined.contains("- apt:nano"));
    }

    #[test]
    fn detail_lines_without_git_says_so() {
        let cur = gv("g1", &["apt:curl"]);
        let lines = detail_lines(&cur, None);
        assert!(lines.iter().any(|l| l.contains("config not under git")));
    }

    #[test]
    fn navigation_wraps_and_tracks_previous() {
        let c = Cockpit::new(vec![gv("g2", &["a"]), gv("g1", &["b"])]);
        // Newest-first: index 0 is g2, its "previous" (older) is g1 at index 1.
        assert_eq!(c.selected().unwrap().id, "g2");
        assert_eq!(c.selected_previous().unwrap().id, "g1");
    }

    #[test]
    fn empty_cockpit_has_no_selection() {
        let c = Cockpit::new(vec![]);
        assert!(c.selected().is_none());
        assert!(c.selected_previous().is_none());
    }

    #[test]
    fn next_previous_wrap_around() {
        let mut c = Cockpit::new(vec![gv("g3", &[]), gv("g2", &[]), gv("g1", &[])]);
        c.previous(); // from 0 wraps to last
        assert_eq!(c.selected().unwrap().id, "g1");
        c.next(); // wraps back to 0
        assert_eq!(c.selected().unwrap().id, "g3");
    }
}
