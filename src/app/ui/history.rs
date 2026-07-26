// src/app/ui/history.rs
//
// A browser over LiNix's history. That history is now git (the
// generation format was deleted — II.1: git IS the history), so the timeline is your commit
// log and each entry shows what that commit changed in your manifests.
//
//   ┌ Commits ─────┐┌ Selected commit ────────────────────┐
//   │ > a1b2c3d     ││ Commit  : a1b2c3d                    │
//   │   9f8e7d6     ││ When    : 2026-07-15                 │
//   │   4c5b6a7     ││ Message : add ripgrep, drop nano     │
//   │              ││ Manifest changes in this commit:      │
//   └──────────────┘└─ + cargo:ripgrep   - apt:nano ───────┘
//   ┌ Shell ───────────────────────────────────────────────┐
//   │ $ _                                                   │
//   └───────────────────────────────────────────────────────┘
//
// Left: the commit timeline (newest first). Right: the selected commit's metadata and the
// manifest lines it added/removed. Bottom: a shell line for running commands without leaving
// the history. Rollback ('r') checks out the selected commit and syncs.
//
// The rendering logic is pure and unit-tested; the ratatui event loop is a thin shell.

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

/// A display-ready view of one git commit (the history's timeline is git history now).
#[derive(Debug, Clone)]
pub struct CommitView {
    /// Short commit hash — the row's identifier.
    pub short: String,
    pub date: String,
    /// Commit subject (the change's message).
    pub subject: String,
    /// Full commit hash — what a rollback checks out.
    pub full_hash: String,
    /// The manifest lines this commit added or removed (`+ apt:curl`, `- apt:nano`).
    pub changes: Vec<String>,
    /// What git says about the commit's signature (II.13), already rendered.
    pub signature: String,
}

/// What the history asks the async caller to do after it exits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryAction {
    Quit,
    /// Roll back to a commit: check out its manifests, then sync the machine to match.
    Rollback {
        reference: String,
    },
}

/// One row in the left-hand timeline: the short hash and the commit subject.
pub fn commit_row(c: &CommitView) -> String {
    format!("{}  {}", c.short, c.subject)
}

/// The right-hand detail lines for a commit: its metadata and the manifest lines it changed.
pub fn detail_lines(current: &CommitView) -> Vec<String> {
    let mut lines = vec![
        format!("Commit  : {}", current.short),
        format!("When    : {}", current.date),
        format!("Message : {}", current.subject),
        format!(
            "Full    : {}",
            &current.full_hash[..current.full_hash.len().min(12)]
        ),
        format!("Signed  : {}", current.signature),
        String::new(),
    ];
    if current.changes.is_empty() {
        lines.push("No manifest changes in this commit.".to_string());
    } else {
        lines.push("Manifest changes in this commit:".to_string());
        for c in &current.changes {
            lines.push(format!("  {}", c));
        }
    }
    lines
}

pub struct HistoryBrowser {
    commits: Vec<CommitView>,
    list_state: ListState,
    input: String,
    /// True while the user is typing a command into the shell line.
    command_mode: bool,
    /// A transient status message (last command result, hints).
    status: String,
}

impl HistoryBrowser {
    pub fn new(commits: Vec<CommitView>) -> Self {
        let mut list_state = ListState::default();
        if !commits.is_empty() {
            list_state.select(Some(0));
        }
        Self {
            commits,
            list_state,
            input: String::new(),
            command_mode: false,
            status: "[j/k] move  [r] rollback (checkout + sync)  [:] shell  [q] quit".into(),
        }
    }

    fn selected(&self) -> Option<&CommitView> {
        self.list_state.selected().and_then(|i| self.commits.get(i))
    }

    fn next(&mut self) {
        if self.commits.is_empty() {
            return;
        }
        let i = self
            .list_state
            .selected()
            .map(|i| (i + 1) % self.commits.len())
            .unwrap_or(0);
        self.list_state.select(Some(i));
    }

    fn previous(&mut self) {
        if self.commits.is_empty() {
            return;
        }
        let i = self
            .list_state
            .selected()
            .map(|i| {
                if i == 0 {
                    self.commits.len() - 1
                } else {
                    i - 1
                }
            })
            .unwrap_or(0);
        self.list_state.select(Some(i));
    }

    /// Launch the history; returns the action the caller should perform.
    pub fn run(&mut self) -> Result<HistoryAction> {
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
    ) -> Result<HistoryAction> {
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
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(HistoryAction::Quit),
                    KeyCode::Down | KeyCode::Char('j') => self.next(),
                    KeyCode::Up | KeyCode::Char('k') => self.previous(),
                    KeyCode::Char(':') | KeyCode::Char('/') => {
                        self.command_mode = true;
                        self.input.clear();
                    }
                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        if let Some(c) = self.selected() {
                            return Ok(HistoryAction::Rollback {
                                reference: c.full_hash.clone(),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Run a command from the shell line: drop out of the alternate screen, execute it via the
    /// system shell, wait for a keypress, then restore the history.
    fn run_shell(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        cmd: &str,
    ) -> Result<()> {
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;

        println!("$ {}\n", cmd);
        #[cfg(windows)]
        let status = std::process::Command::new("cmd").args(["/C", cmd]).status();
        #[cfg(not(windows))]
        let status = std::process::Command::new("sh").args(["-c", cmd]).status();
        match status {
            Ok(s) => self.status = format!("`{}` exited with {}", cmd, s),
            Err(e) => self.status = format!("`{}` failed to run: {}", cmd, e),
        }
        println!("\n[press Enter to return to the history]");
        let _ = io::stdin().read_line(&mut String::new());

        enable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            EnterAlternateScreen,
            EnableMouseCapture
        )?;
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

        // Left: the commit timeline.
        let items: Vec<ListItem> = self
            .commits
            .iter()
            .map(|c| ListItem::new(commit_row(c)))
            .collect();
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" Commits "))
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(40, 40, 40))
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");
        f.render_stateful_widget(list, cols[0], &mut self.list_state);

        // Right: the selected commit's detail.
        let detail = match self.selected() {
            Some(c) => detail_lines(c).join("\n"),
            None => "No commits yet. Run `linix git init`, then `sync` commits your history."
                .to_string(),
        };
        let detail_widget = Paragraph::new(detail)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Selected commit "),
            )
            .wrap(Wrap { trim: false });
        f.render_widget(detail_widget, cols[1]);

        // Bottom: shell line / status.
        let bottom = if self.command_mode {
            format!("$ {}\u{2588}", self.input)
        } else {
            self.status.clone()
        };
        let title = if self.command_mode {
            " Shell (Enter to run, Esc to cancel) "
        } else {
            " Shell "
        };
        let shell =
            Paragraph::new(bottom).block(Block::default().borders(Borders::ALL).title(title));
        f.render_widget(shell, rows[1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cv(short: &str, subject: &str, changes: &[&str]) -> CommitView {
        CommitView {
            short: short.into(),
            date: "2026-07-15".into(),
            subject: subject.into(),
            full_hash: format!("{}0000000000000000000000000000000000000", short),
            changes: changes.iter().map(|s| s.to_string()).collect(),
            signature: "unsigned".to_string(),
        }
    }

    #[test]
    fn commit_row_shows_hash_and_subject() {
        let row = commit_row(&cv("a1b2c3d", "add ripgrep", &["+ cargo:rg"]));
        assert!(row.contains("a1b2c3d"));
        assert!(row.contains("add ripgrep"));
    }

    #[test]
    fn detail_lines_show_metadata_and_changes() {
        let c = cv(
            "a1b2c3d",
            "swap nano for ripgrep",
            &["+ cargo:rg", "- apt:nano"],
        );
        let joined = detail_lines(&c).join("\n");
        assert!(joined.contains("Commit  : a1b2c3d"));
        assert!(joined.contains("Message : swap nano for ripgrep"));
        assert!(joined.contains("Manifest changes in this commit:"));
        assert!(joined.contains("+ cargo:rg"));
        assert!(joined.contains("- apt:nano"));
    }

    #[test]
    fn detail_lines_note_a_commit_with_no_manifest_changes() {
        let c = cv("a1b2c3d", "docs only", &[]);
        assert!(detail_lines(&c)
            .iter()
            .any(|l| l.contains("No manifest changes")));
    }

    #[test]
    fn a_rollback_targets_the_full_hash() {
        // The row shows the short hash, but a rollback must check out the full commit.
        let mut c = HistoryBrowser::new(vec![cv("a1b2c3d", "x", &[])]);
        c.next(); // stays on 0
        let sel = c.selected().unwrap();
        assert!(sel.full_hash.starts_with("a1b2c3d"));
        assert!(sel.full_hash.len() > 7);
    }

    #[test]
    fn navigation_wraps() {
        let mut c = HistoryBrowser::new(vec![
            cv("c3", "", &[]),
            cv("c2", "", &[]),
            cv("c1", "", &[]),
        ]);
        assert_eq!(c.selected().unwrap().short, "c3");
        c.previous(); // from 0 wraps to last
        assert_eq!(c.selected().unwrap().short, "c1");
        c.next(); // wraps back to 0
        assert_eq!(c.selected().unwrap().short, "c3");
    }

    #[test]
    fn empty_history_has_no_selection() {
        let c = HistoryBrowser::new(vec![]);
        assert!(c.selected().is_none());
    }
}
