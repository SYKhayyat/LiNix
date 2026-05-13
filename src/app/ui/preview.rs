use crate::app::sync::SyncChanges;
use crate::core::{GraphAction, PackageSpec, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};
use std::collections::{HashMap, HashSet};
use std::io;
use petgraph::graph::NodeIndex;

/// An interactive TUI for previewing and filtering the execution DAG.
/// Hardened for Version 3.5.0 to handle Ambiguous Probes (Point 7).
/// 
/// It allows users to:
/// 1. Toggle tasks (Space)
/// 2. Cycle backends for "bare" package names (b)
/// 3. Commit/Cancel the transaction
pub struct TuiPreview<'a> {
    pub changes: &'a SyncChanges,
    /// Indices of nodes that the user has opted to skip.
    pub disabled_nodes: HashSet<NodeIndex>,
    /// Maps a NodeIndex to a user-selected backend override.
    /// Used when multiple backends provide the same package name.
    pub backend_overrides: HashMap<NodeIndex, String>,
    /// List of available backend candidates for specific nodes.
    pub alternatives: HashMap<NodeIndex, Vec<String>>,
    list_state: ListState,
}

impl<'a> TuiPreview<'a> {
    pub fn new(changes: &'a SyncChanges, alternatives: HashMap<NodeIndex, Vec<String>>) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            changes,
            disabled_nodes: HashSet::new(),
            backend_overrides: HashMap::new(),
            alternatives,
            list_state,
        }
    }

    /// Entry point to launch the TUI. 
    /// Returns true if the user confirmed the transaction.
    pub fn run(&mut self) -> Result<bool> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let result = self.main_loop(&mut terminal);

        // Restore terminal state
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        result
    }

    fn main_loop<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<bool> {
        loop {
            terminal.draw(|f| self.ui(f))?;

            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(false),
                    KeyCode::Enter | KeyCode::Char('y') => return Ok(true),
                    KeyCode::Up | KeyCode::Char('k') => self.previous(),
                    KeyCode::Down | KeyCode::Char('j') => self.next(),
                    KeyCode::Char(' ') => self.toggle_selected(),
                    KeyCode::Char('b') => self.cycle_backend(),
                    _ => {}
                }
            }
        }
    }

    fn ui<B: Backend>(&mut self, f: &mut Frame<B>) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints(
                [
                    Constraint::Length(3),
                    Constraint::Min(10),
                    Constraint::Length(4),
                ]
                .as_ref(),
            )
            .split(f.size());

        // 1. Header
        let header = Paragraph::new("LiNix Transaction Preview - Confirm System Changes")
            .block(Block::default().borders(Borders::ALL).title("Status"));
        f.render_widget(header, chunks[0]);

        // 2. Action List
        let items: Vec<ListItem> = self.changes.graph.node_indices()
            .map(|idx| {
                let action = &self.changes.graph[idx];
                let is_disabled = self.disabled_nodes.contains(&idx);
                let user_backend = self.backend_overrides.get(&idx);
                let has_alternatives = self.alternatives.get(&idx);

                let (indicator, mut text, style) = match action {
                    GraphAction::Install(spec) => {
                        let b_name = user_backend.unwrap_or(&spec.backend);
                        let base_style = if is_disabled {
                            Style::default().fg(Color::DarkGray)
                        } else {
                            Style::default().fg(Color::Green)
                        };
                        ("[+]", format!("Install {}:{}", b_name, spec.name), base_style)
                    }
                    GraphAction::Remove { name, backend } => {
                        let base_style = if is_disabled {
                            Style::default().fg(Color::DarkGray)
                        } else {
                            Style::default().fg(Color::Red)
                        };
                        ("[-]", format!("Remove {}:{}", backend, name), base_style)
                    }
                };

                // Visual hint for Ambiguous Probes
                if let Some(alts) = has_alternatives {
                    text = format!("{} (Cycle backends [b]: {:?})", text, alts);
                }

                let checkbox = if is_disabled { "[ ]" } else { "[x]" };
                ListItem::new(format!("{} {} {}", checkbox, indicator, text)).style(style)
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Execution Graph"))
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(40, 40, 40))
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");

        f.render_stateful_widget(list, chunks[1], &mut self.list_state);

        // 3. Footer / Help
        let footer = Paragraph::new(
            " [SPACE] Toggle Task | [b] Cycle Backend (if available) \n [ENTER/Y] Commit Transaction | [ESC/Q] Cancel "
        ).block(Block::default().borders(Borders::ALL).title("Controls"));
        f.render_widget(footer, chunks[2]);
    }

    fn next(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => if i >= self.changes.graph.node_count() - 1 { 0 } else { i + 1 },
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn previous(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => if i == 0 { self.changes.graph.node_count() - 1 } else { i - 1 },
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn toggle_selected(&mut self) {
        if let Some(i) = self.list_state.selected() {
            let node_idx = NodeIndex::new(i);
            if self.disabled_nodes.contains(&node_idx) {
                self.disabled_nodes.remove(&node_idx);
            } else {
                self.disabled_nodes.insert(node_idx);
            }
        }
    }

    /// Point 7: Cycle through available backend candidates for a bare name.
    fn cycle_backend(&mut self) {
        if let Some(i) = self.list_state.selected() {
            let node_idx = NodeIndex::new(i);
            if let Some(alts) = self.alternatives.get(&node_idx) {
                if alts.len() <= 1 { return; }

                // Determine current selected backend for this node
                let current_action = &self.changes.graph[node_idx];
                let current_backend = self.backend_overrides.get(&node_idx)
                    .cloned()
                    .unwrap_or_else(|| {
                        match current_action {
                            GraphAction::Install(s) => s.backend.clone(),
                            _ => String::new(),
                        }
                    });

                // Find next in cycle
                if let Some(pos) = alts.iter().position(|b| b == &current_backend) {
                    let next_pos = (pos + 1) % alts.len();
                    let next_backend = alts[next_pos].clone();
                    self.backend_overrides.insert(node_idx, next_backend);
                }
            }
        }
    }
}