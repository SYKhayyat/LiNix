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
    /// Maps UI list positions to actual NodeIndex (fixes issue #3)
    pub ui_index_to_node: Vec<NodeIndex>,
    list_state: ListState,
}

impl<'a> TuiPreview<'a> {
    pub fn new(changes: &'a SyncChanges, alternatives: HashMap<NodeIndex, Vec<String>>) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        
        // Build mapping from UI position (0..N) to actual NodeIndex
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
    
    /// Helper to get NodeIndex from current UI selection
    fn get_selected_node(&self) -> Option<NodeIndex> {
        self.list_state
            .selected()
            .and_then(|i| self.ui_index_to_node.get(i).copied())
    }
    
    /// Helper to get UI position from NodeIndex
    fn get_ui_position(&self, node_idx: NodeIndex) -> Option<usize> {
        self.ui_index_to_node.iter().position(|&idx| idx == node_idx)
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

        // 2. Action List - Build items using the UI index mapping
        let items: Vec<ListItem> = self.ui_index_to_node
            .iter()
            .enumerate()
            .map(|(ui_idx, &node_idx)| {
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
            Some(i) => {
                let total = self.ui_index_to_node.len();
                if i >= total - 1 { 0 } else { i + 1 }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    fn previous(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => {
                let total = self.ui_index_to_node.len();
                if i == 0 { total - 1 } else { i - 1 }
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

    /// Point 7: Cycle through available backend candidates for a bare name.
    fn cycle_backend(&mut self) {
        if let Some(node_idx) = self.get_selected_node() {
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
    
    /// Returns the filtered changes based on user selections.
    pub fn get_filtered_changes(&self) -> SyncChanges {
        let mut filtered = self.changes.clone();
        for idx in &self.disabled_nodes {
            filtered.graph.remove_node(*idx);
        }
        filtered
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use petgraph::stable_graph::StableDiGraph;
    use std::collections::HashMap;

    fn create_test_changes() -> SyncChanges {
        let mut graph = StableDiGraph::new();
        let spec = PackageSpec {
            name: "test".to_string(),
            backend: "apt".to_string(),
            options: HashMap::new(),
            requires: vec![],
        };
        graph.add_node(GraphAction::Install(spec));
        SyncChanges {
            graph,
            node_map: HashMap::new(),
        }
    }

    #[test]
    fn test_ui_index_mapping() {
        let changes = create_test_changes();
        let alternatives = HashMap::new();
        let preview = TuiPreview::new(&changes, alternatives);
        
        assert_eq!(preview.ui_index_to_node.len(), 1);
        assert!(preview.get_ui_position(preview.ui_index_to_node[0]).is_some());
        assert!(preview.get_selected_node().is_some());
    }
    
    #[test]
    fn test_toggle_selected() {
        let changes = create_test_changes();
        let alternatives = HashMap::new();
        let mut preview = TuiPreview::new(&changes, alternatives);
        
        // Select the first item
        preview.list_state.select(Some(0));
        
        let node_idx = preview.get_selected_node().unwrap();
        assert!(!preview.disabled_nodes.contains(&node_idx));
        
        preview.toggle_selected();
        assert!(preview.disabled_nodes.contains(&node_idx));
        
        preview.toggle_selected();
        assert!(!preview.disabled_nodes.contains(&node_idx));
    }
    
    #[test]
    fn test_navigation_wrapping() {
        let changes = create_test_changes();
        let alternatives = HashMap::new();
        let mut preview = TuiPreview::new(&changes, alternatives);
        
        // With only one item, next and previous should keep it at 0
        preview.next();
        assert_eq!(preview.list_state.selected(), Some(0));
        
        preview.previous();
        assert_eq!(preview.list_state.selected(), Some(0));
    }
    
    #[test]
    fn test_cycle_backend() {
        let mut graph = StableDiGraph::new();
        let spec = PackageSpec {
            name: "test".to_string(),
            backend: "apt".to_string(),
            options: HashMap::new(),
            requires: vec![],
        };
        let node_idx = graph.add_node(GraphAction::Install(spec));
        
        let changes = SyncChanges {
            graph,
            node_map: HashMap::new(),
        };
        
        let mut alternatives = HashMap::new();
        alternatives.insert(node_idx, vec!["apt".to_string(), "brew".to_string(), "cargo".to_string()]);
        
        let mut preview = TuiPreview::new(&changes, alternatives);
        preview.list_state.select(Some(0));
        
        // Should cycle through backends
        preview.cycle_backend();
        assert_eq!(preview.backend_overrides.get(&node_idx), Some(&"brew".to_string()));
        
        preview.cycle_backend();
        assert_eq!(preview.backend_overrides.get(&node_idx), Some(&"cargo".to_string()));
        
        preview.cycle_backend();
        assert_eq!(preview.backend_overrides.get(&node_idx), Some(&"apt".to_string()));
    }
    
    #[test]
    fn test_get_filtered_changes() {
        let mut graph = StableDiGraph::new();
        let spec1 = PackageSpec {
            name: "pkg1".to_string(),
            backend: "apt".to_string(),
            options: HashMap::new(),
            requires: vec![],
        };
        let spec2 = PackageSpec {
            name: "pkg2".to_string(),
            backend: "brew".to_string(),
            options: HashMap::new(),
            requires: vec![],
        };
        let idx1 = graph.add_node(GraphAction::Install(spec1));
        let idx2 = graph.add_node(GraphAction::Install(spec2));
        
        let changes = SyncChanges {
            graph,
            node_map: HashMap::new(),
        };
        
        let mut preview = TuiPreview::new(&changes, HashMap::new());
        preview.disabled_nodes.insert(idx1);
        
        let filtered = preview.get_filtered_changes();
        assert_eq!(filtered.graph.node_count(), 1);
        
        // The remaining node should be idx2
        let remaining: Vec<_> = filtered.graph.node_indices().collect();
        assert_eq!(remaining[0], idx2);
    }
}