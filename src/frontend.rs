use std::io::Stdout;
use std::time::Duration;

use act_zero::timer::Tick;
use async_trait::async_trait;
use crossterm::execute;
use crossterm::terminal::{enable_raw_mode, EnterAlternateScreen};

use std::io;
use tokio::time::Instant;
use tui::backend::CrosstermBackend;
use tui::layout::{Constraint, Layout};
use tui::style::{Color, Style};
use tui::widgets::{Block, Borders, Cell, Row, Table, TableState};
use tui::Terminal;

use uuid::Uuid;

use act_zero::runtimes::tokio::Timer;
use act_zero::*;

pub struct StreamInfo {
    pub id: Uuid,
    pub method: String,
    pub remote: String,
    pub upstream: String,
    pub opened_at: Instant,
    pub closed_at: Option<Instant>,
}
pub struct Tui {
    state: TableState,
    terminal: Terminal<CrosstermBackend<Stdout>>,
    items: Vec<StreamInfo>,
    timer: Timer,
}

impl Default for Tui {
    fn default() -> Self {
        enable_raw_mode().expect("failed to enable raw mode");
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen).expect("failed to enable raw mode");

        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).expect("failed to start TUI");
        Self {
            state: Default::default(),
            items: Default::default(),
            timer: Timer::default(),
            terminal,
        }
    }
}

#[async_trait]
impl Actor for Tui {
    async fn started(&mut self, addr: Addr<Self>) -> ActorResult<()> {
        self.timer
            .set_interval_weak(addr.downgrade().clone(), Duration::from_millis(250));
        Tui::ui(&mut self.terminal, &self.items, &mut self.state);
        Produces::ok(())
    }
}

#[async_trait]
impl Tick for Tui {
    async fn tick(&mut self) -> ActorResult<()> {
        if self.timer.tick() {
            self.items
                .retain(|v| v.closed_at.is_none() || v.closed_at.unwrap().elapsed().as_secs() < 4);
            Tui::ui(&mut self.terminal, &self.items, &mut self.state);
        }
        Produces::ok(())
    }
}

impl Tui {
    pub async fn push(&mut self, info: StreamInfo) -> ActorResult<()> {
        self.items.push(info);
        Tui::ui(&mut self.terminal, &self.items, &mut self.state);
        Produces::ok(())
    }
    pub async fn remove(&mut self, id: Uuid) -> ActorResult<()> {
        match self.items.iter().position(|v| v.id == id) {
            Some(pos) => {
                self.items[pos].closed_at = Some(Instant::now());
            }
            None => {}
        }
        Tui::ui(&mut self.terminal, &self.items, &mut self.state);
        Produces::ok(())
    }
    fn ui(
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        items: &Vec<StreamInfo>,
        state: &mut TableState,
    ) {
        terminal
            .draw(|f| {
                let rects = Layout::default()
                    .constraints([Constraint::Percentage(100)].as_ref())
                    .split(f.size());

                let normal_style = Style::default().bg(Color::Blue);
                let header_cells = ["Method", "Host", "Proxy", "State"]
                    .iter()
                    .map(|h| Cell::from(*h).style(Style::default().fg(Color::Red)));
                let header = Row::new(header_cells).style(normal_style);
                let rows = items.iter().map(|item| {
                    let mut cells = vec![
                        Cell::from(item.method.clone()),
                        Cell::from(item.remote.clone()),
                        Cell::from(item.upstream.clone()),
                    ];
                    match item.closed_at {
                        Some(v) => {
                            cells
                                .push(Cell::from(format!("Closed {}s ago", v.elapsed().as_secs())));
                        }
                        None => {
                            cells.push(Cell::from(format!(
                                "Streaming for {}s",
                                item.opened_at.elapsed().as_secs()
                            )));
                        }
                    }
                    Row::new(cells)
                });
                let t = Table::new(rows)
                    .header(header)
                    .block(Block::default().borders(Borders::ALL).title("Connections"))
                    .widths(&[
                        Constraint::Percentage(10),
                        Constraint::Percentage(40),
                        Constraint::Percentage(20),
                        Constraint::Percentage(20),
                    ]);
                f.render_stateful_widget(t, rects[0], state);
            })
            .expect("failed to draw TUI");
    }
}
