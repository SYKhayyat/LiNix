use crate::app::sync::SyncChanges;
use crate::core::{GraphAction, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use petgraph::graph::NodeIndex;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};
use std::collections::{HashMap, HashSet};
use std::io;

/// An interactive TUI for previewing and filtering the execution DAG.
pub struct TuiPreview<'a> {
    pub changes: &'a SyncChanges,
    /// Indices of nodes that the user has opted to skip.
    pub disabled_nodes: HashSet<NodeIndex>,
    /// Maps a NodeIndex to a user-selected backend override.
    pub backend_overrides: HashMap<NodeIndex, String>,
    /// List of available backend candidates for specific nodes.
    pub alternatives: HashMap<NodeIndex, Vec<String>>,
    pub ui_index_to_node: Vec<NodeIndex>,
    list_state: ListState,
}

impl<'a> TuiPreview<'a> {
    pub fn new(changes: &'a SyncChanges, alternatives: HashMap<NodeIndex, Vec<String>>) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));

        // NodeIndex is not the list position: the graph is sparse, so the UI keeps its own
        // dense index and must map back through this table before touching the graph.
        let ui_index_to_node: Vec<NodeIndex> = changes.graph.node_indices().collect();

        Self {
            changes,
            disabled_nodes: HashSet::new(),
            backend_overrides: HashMap::new(),
            alternatives,
            ui_index_to_node,
            list_state,
        }
    }

    fn get_selected_node(&self) -> Option<NodeIndex> {
        self.list_state
            .selected()
            .and_then(|i| self.ui_index_to_node.get(i).copied())
    }

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

    fn main_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<bool> {
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

    fn ui(&mut self, f: &mut Frame) {
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

        let header = Paragraph::new("LiNix Transaction Preview - Confirm System Changes")
            .block(Block::default().borders(Borders::ALL).title("Status"));
        f.render_widget(header, chunks[0]);

        let items: Vec<ListItem> = self
            .ui_index_to_node
            .iter()
            .map(|&node_idx| {
                let action = &self.changes.graph[node_idx];
                let is_disabled = self.disabled_nodes.contains(&node_idx);
                let user_backend = self.backend_overrides.get(&node_idx);
                let has_alternatives = self.alternatives.get(&node_idx);

                let (indicator, mut text, style) = match action {
                    GraphAction::Install(spec) => {
                        let b_name = user_backend.unwrap_or(&spec.backend);
                        let base_style = if is_disabled {
                            Style::default().fg(Color::DarkGray)
                        } else {
                            Style::default().fg(Color::Green)
                        };
                        (
                            "[+]",
                            format!("Install {}:{}", b_name, spec.name),
                            base_style,
                        )
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

                if let Some(alts) = has_alternatives {
                    text = format!("{} (Cycle backends [b]: {:?})", text, alts);
                }

                let checkbox = if is_disabled { "[ ]" } else { "[x]" };
                ListItem::new(format!("{} {} {}", checkbox, indicator, text)).style(style)
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Execution Graph"),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(40, 40, 40))
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");

        f.render_stateful_widget(list, chunks[1], &mut self.list_state);

        let footer = Paragraph::new(
            " [SPACE] Toggle Task | [b] Cycle Backend (if available) \n [ENTER/Y] Commit Transaction | [ESC/Q] Cancel "
        ).block(Block::default().borders(Borders::ALL).title("Controls"));
        f.render_widget(footer, chunks[2]);
    }

    fn next(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => {
                let total = self.ui_index_to_node.len();
                if i >= total - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn previous(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => {
                let total = self.ui_index_to_node.len();
                if i == 0 {
                    total - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn toggle_selected(&mut self) {
        if let Some(node_idx) = self.get_selected_node() {
            if self.disabled_nodes.contains(&node_idx) {
                self.disabled_nodes.remove(&node_idx);
            } else {
                self.disabled_nodes.insert(node_idx);
            }
        }
    }

    fn cycle_backend(&mut self) {
        if let Some(node_idx) = self.get_selected_node() {
            if let Some(alts) = self.alternatives.get(&node_idx) {
                if alts.len() <= 1 {
                    return;
                }

                let current_action = &self.changes.graph[node_idx];
                let current_backend = self
                    .backend_overrides
                    .get(&node_idx)
                    .cloned()
                    .unwrap_or_else(|| match current_action {
                        GraphAction::Install(s) => s.backend.clone(),
                        _ => String::new(),
                    });

                if let Some(pos) = alts.iter().position(|b| b == &current_backend) {
                    let next_pos = (pos + 1) % alts.len();
                    let next_backend = alts[next_pos].clone();
                    self.backend_overrides.insert(node_idx, next_backend);
                }
            }
        }
    }

    pub fn get_filtered_changes(&self) -> SyncChanges {
        let mut filtered = self.changes.clone();
        for idx in &self.disabled_nodes {
            filtered.graph.remove_node(*idx);
        }
        filtered
    }
}
