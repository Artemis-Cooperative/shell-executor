//! mprocs-style TUI for running parallel commands interactively.
//!
//! Activated by `-i --parallel`. Each child gets its own PTY; the sidebar
//! lists every child and the main pane shows the focused child's terminal.
//! `Enter` enters input mode (keys go to the child's PTY); `Ctrl-Q` returns
//! to nav mode. `q` quits after confirmation. The TUI auto-exits once every
//! child has finished and then prints the standard `--parallel` summary
//! block on the main screen.

#![expect(
    clippy::print_stdout,
    reason = "CLI tool intentionally writes to stdout for user-facing output"
)]

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseEventKind,
};
use crossterm::execute;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::Stylize;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Widget,
};
use ratatui::Frame;
use vt100::Parser;

use crate::outcome::{self, Outcome, OutputCapture};
use crate::parallel::{print_final_block, write_log_entry};
use crate::{derive_display_message, CommandOutput, RunReport, RunStatus};

const SCROLLBACK_LINES: usize = 10_000;
const TICK_MS: u64 = 50;
const SIDEBAR_MIN: u16 = 15;
const SIDEBAR_PREF: u16 = 24;
const SIDEBAR_MAX: u16 = 40;
const STATUS_BAR_HEIGHT: u16 = 1;
const KILL_GRACE_MS: u64 = 200;

struct Pane {
    label: String,
    parser: Arc<Mutex<Parser>>,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Box<dyn Child + Send + Sync>,
    status: PaneStatus,
    started_at: Instant,
    finished_at: Option<Instant>,
    scrollback: usize,
}

enum PaneStatus {
    Running,
    Done(RunStatus, i32),
}

impl Pane {
    fn is_running(&self) -> bool {
        matches!(self.status, PaneStatus::Running)
    }

    fn elapsed(&self) -> Duration {
        match self.finished_at {
            Some(end) => end.duration_since(self.started_at),
            None => self.started_at.elapsed(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Nav,
    Input,
    ConfirmQuit,
}

pub(crate) fn run_report(
    commands: &[String],
    parent_message: Option<&str>,
    quiet: bool,
    log: Option<&PathBuf>,
    show_time: bool,
) -> RunReport {
    let n = commands.len();
    let parent_label = match parent_message {
        Some(m) => m.to_string(),
        None => format!("{n} parallel commands"),
    };

    if n == 0 {
        println!("[ \x1b[32m✓\x1b[0m ] {parent_label}");
        return RunReport {
            status: RunStatus::Success,
            exit_code: 0,
        };
    }

    let (term_cols, term_rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let sidebar_w = sidebar_width(term_cols);
    let pane_size = pane_pty_size(term_cols, term_rows, sidebar_w);

    let mut panes: Vec<Pane> = Vec::with_capacity(n);
    for cmd in commands {
        match spawn_pane(cmd, pane_size) {
            Ok(p) => panes.push(p),
            Err(e) => {
                // Insert a synthetic already-failed pane so we can still finish the run.
                let label = derive_display_message(None, cmd);
                let now = Instant::now();
                let mut parser = Parser::new(pane_size.rows, pane_size.cols, SCROLLBACK_LINES);
                let msg = format!("Failed to spawn: {e}\r\n");
                parser.process(msg.as_bytes());
                panes.push(synthetic_failed_pane(label, parser, now));
            }
        }
    }

    let group_start = Instant::now();
    let outcome = match event_loop(&mut panes, &parent_label, show_time) {
        Ok(o) => o,
        Err(_e) => LoopOutcome::Errored,
    };

    // Tear down still-running children if user quit early.
    if matches!(outcome, LoopOutcome::QuitByUser) {
        kill_all(&mut panes);
    }
    finalize_pane_statuses(&mut panes);

    let group_elapsed = group_start.elapsed();

    let child_outcomes: Vec<Outcome> = panes.iter().map(pane_to_outcome).collect();
    let parent_status = outcome::aggregate(&child_outcomes);
    let parent_outcome = Outcome {
        status: parent_status,
        output: OutputCapture::Inherited,
        elapsed: group_elapsed,
        label: parent_label.clone(),
        signal_num: None,
    };

    print_final_block(&parent_outcome, &child_outcomes, show_time, quiet);

    if let Some(log_path) = log {
        write_log_entry(
            log_path,
            &parent_outcome,
            &child_outcomes,
            quiet,
            /* include_bodies = */ true,
        );
    }

    RunReport {
        status: parent_status,
        exit_code: outcome::exit_code(&parent_outcome),
    }
}

pub(crate) fn run_succinct_report(
    commands: &[String],
    parent_message: Option<&str>,
    quiet: bool,
    log: Option<&PathBuf>,
) -> RunReport {
    // Succinct mode skips the TUI entirely — no value in opening a UI you
    // then refuse to render. Fall through to the existing parallel succinct
    // path semantically by running the same teardown without a UI loop.
    let n = commands.len();
    let parent_label = match parent_message {
        Some(m) => m.to_string(),
        None => format!("{n} parallel commands"),
    };

    if n == 0 {
        return RunReport {
            status: RunStatus::Success,
            exit_code: 0,
        };
    }

    let (term_cols, term_rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let pane_size = pane_pty_size(term_cols, term_rows, sidebar_width(term_cols));

    let mut panes: Vec<Pane> = Vec::with_capacity(n);
    for cmd in commands {
        if let Ok(p) = spawn_pane(cmd, pane_size) {
            panes.push(p);
        } else {
            let label = derive_display_message(None, cmd);
            let parser = Parser::new(pane_size.rows, pane_size.cols, SCROLLBACK_LINES);
            panes.push(synthetic_failed_pane(label, parser, Instant::now()));
        }
    }

    let group_start = Instant::now();
    // Wait silently for all children.
    loop {
        let mut all_done = true;
        for p in &mut panes {
            if matches!(p.status, PaneStatus::Running) {
                match p.child.try_wait() {
                    Ok(Some(status)) => {
                        #[expect(
                            clippy::cast_possible_wrap,
                            clippy::as_conversions,
                            reason = "exit codes are at most 255 in practice; wide cast preserves the value"
                        )]
                        let code = status.exit_code() as i32;
                        let run_status = if code == 0 {
                            RunStatus::Success
                        } else {
                            RunStatus::Failure
                        };
                        p.status = PaneStatus::Done(run_status, code);
                        p.finished_at = Some(Instant::now());
                    }
                    _ => all_done = false,
                }
            }
        }
        if all_done {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let group_elapsed = group_start.elapsed();

    let child_outcomes: Vec<Outcome> = panes.iter().map(pane_to_outcome).collect();
    let parent_status = outcome::aggregate(&child_outcomes);
    let parent_outcome = Outcome {
        status: parent_status,
        output: OutputCapture::Inherited,
        elapsed: group_elapsed,
        label: parent_label.clone(),
        signal_num: None,
    };

    if let Some(log_path) = log {
        write_log_entry(
            log_path,
            &parent_outcome,
            &child_outcomes,
            quiet,
            /* include_bodies = */ false,
        );
    }

    RunReport {
        status: parent_status,
        exit_code: outcome::exit_code(&parent_outcome),
    }
}

fn sidebar_width(term_cols: u16) -> u16 {
    let third = (term_cols / 3).max(SIDEBAR_MIN);
    SIDEBAR_PREF.clamp(SIDEBAR_MIN, third.min(SIDEBAR_MAX))
}

fn pane_pty_size(term_cols: u16, term_rows: u16, sidebar_w: u16) -> PtySize {
    let inner_cols = term_cols.saturating_sub(sidebar_w).saturating_sub(2); // 2 = borders
    let inner_rows = term_rows
        .saturating_sub(STATUS_BAR_HEIGHT)
        .saturating_sub(2);
    PtySize {
        rows: inner_rows.max(3),
        cols: inner_cols.max(10),
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn spawn_pane(cmd: &str, size: PtySize) -> io::Result<Pane> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(size)
        .map_err(|e| io::Error::other(format!("openpty: {e}")))?;

    let mut builder = CommandBuilder::new("sh");
    builder.arg("-c");
    builder.arg(cmd);
    if let Ok(cwd) = std::env::current_dir() {
        builder.cwd(cwd);
    }
    for (k, v) in std::env::vars_os() {
        builder.env(k, v);
    }

    let child = pair
        .slave
        .spawn_command(builder)
        .map_err(|e| io::Error::other(format!("spawn: {e}")))?;
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| io::Error::other(format!("clone_reader: {e}")))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| io::Error::other(format!("take_writer: {e}")))?;

    let parser = Arc::new(Mutex::new(Parser::new(
        size.rows,
        size.cols,
        SCROLLBACK_LINES,
    )));
    let parser_for_reader = Arc::clone(&parser);

    thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if let Ok(mut p) = parser_for_reader.lock() {
                        p.process(&buf[..n]);
                    }
                }
            }
        }
    });

    Ok(Pane {
        label: derive_display_message(None, cmd),
        parser,
        master: Arc::new(Mutex::new(pair.master)),
        writer: Arc::new(Mutex::new(writer)),
        child,
        status: PaneStatus::Running,
        started_at: Instant::now(),
        finished_at: None,
        scrollback: 0,
    })
}

#[expect(
    clippy::expect_used,
    reason = "dummy PTY fallback; allocation failures here are fatal anyway"
)]
fn synthetic_failed_pane(label: String, parser: Parser, now: Instant) -> Pane {
    // Used when spawn fails: a pane that is already Done(Failure).
    // We can't easily fabricate a real Child/MasterPty/Writer, so this
    // constructor is only used in the error path before the event loop.
    // Use a dummy PTY pair to satisfy the type.
    let pty_system = native_pty_system();
    let dummy = pty_system
        .openpty(PtySize {
            rows: 1,
            cols: 1,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("dummy pty for failed pane");
    let writer = dummy.master.take_writer().expect("dummy writer");
    // Spawn a no-op child on the dummy slave so we have a Child to hold.
    let child = dummy
        .slave
        .spawn_command(CommandBuilder::new("true"))
        .expect("dummy child");
    drop(dummy.slave);
    Pane {
        label,
        parser: Arc::new(Mutex::new(parser)),
        master: Arc::new(Mutex::new(dummy.master)),
        writer: Arc::new(Mutex::new(writer)),
        child,
        status: PaneStatus::Done(RunStatus::Failure, 1),
        started_at: now,
        finished_at: Some(now),
        scrollback: 0,
    }
}

enum LoopOutcome {
    AllDone,
    QuitByUser,
    Errored,
}

#[allow(clippy::too_many_lines, reason = "event loop is a single cohesive state machine; splitting it would obscure control flow without improving clarity")]
fn event_loop(panes: &mut [Pane], parent_label: &str, show_time: bool) -> io::Result<LoopOutcome> {
    let mut terminal = ratatui::init();
    let mut stdout = io::stdout();
    let _ = execute!(stdout, EnableMouseCapture);

    let result = (|| -> io::Result<LoopOutcome> {
        let mut selected: usize = 0;
        let mut mode = Mode::Nav;
        let mut list_state = ListState::default();
        list_state.select(Some(selected));

        loop {
            // Reap finished children.
            for p in panes.iter_mut() {
                if let PaneStatus::Running = p.status {
                    if let Ok(Some(status)) = p.child.try_wait() {
                        #[expect(
                            clippy::cast_possible_wrap,
                            clippy::as_conversions,
                            reason = "exit codes are at most 255 in practice; wide cast preserves the value"
                        )]
                        let code = status.exit_code() as i32;
                        let run_status = if code == 0 {
                            RunStatus::Success
                        } else {
                            RunStatus::Failure
                        };
                        p.status = PaneStatus::Done(run_status, code);
                        p.finished_at = Some(Instant::now());
                    }
                }
            }

            let all_done = panes.iter().all(|p| !p.is_running());

            // Render.
            terminal.draw(|frame| {
                draw_frame(
                    frame,
                    panes,
                    selected,
                    mode,
                    &mut list_state,
                    parent_label,
                    show_time,
                );
            })?;

            if all_done && mode != Mode::ConfirmQuit {
                return Ok(LoopOutcome::AllDone);
            }

            // Poll for input.
            if event::poll(Duration::from_millis(TICK_MS))? {
                let evt = event::read()?;
                match evt {
                    Event::Resize(cols, rows) => {
                        let sidebar_w = sidebar_width(cols);
                        let new_size = pane_pty_size(cols, rows, sidebar_w);
                        for p in panes.iter() {
                            if let Ok(m) = p.master.lock() {
                                let _ = m.resize(new_size);
                            }
                            if let Ok(mut parser) = p.parser.lock() {
                                parser.screen_mut().set_size(new_size.rows, new_size.cols);
                            }
                        }
                    }
                    Event::Key(key) => {
                        if key.kind == KeyEventKind::Release {
                            continue;
                        }
                        match mode {
                            Mode::Nav => {
                                if let Some(action) = nav_key(key, panes.len()) {
                                    match action {
                                        NavAction::Quit => mode = Mode::ConfirmQuit,
                                        NavAction::Enter => {
                                            if let Some(p) = panes.get(selected) {
                                                if p.is_running() {
                                                    mode = Mode::Input;
                                                }
                                            }
                                        }
                                        NavAction::Next => {
                                            if !panes.is_empty() {
                                                selected = (selected + 1).min(panes.len() - 1);
                                                list_state.select(Some(selected));
                                                if let Some(p) = panes.get_mut(selected) {
                                                    p.scrollback = 0;
                                                }
                                            }
                                        }
                                        NavAction::Prev => {
                                            selected = selected.saturating_sub(1);
                                            list_state.select(Some(selected));
                                            if let Some(p) = panes.get_mut(selected) {
                                                p.scrollback = 0;
                                            }
                                        }
                                        NavAction::Select(i) => {
                                            if i < panes.len() {
                                                selected = i;
                                                list_state.select(Some(i));
                                                if let Some(p) = panes.get_mut(selected) {
                                                    p.scrollback = 0;
                                                }
                                            }
                                        }
                                        NavAction::ScrollUp => {
                                            if let Some(p) = panes.get_mut(selected) {
                                                p.scrollback = p
                                                    .scrollback
                                                    .saturating_add(5)
                                                    .min(SCROLLBACK_LINES);
                                                if let Ok(mut parser) = p.parser.lock() {
                                                    parser
                                                        .screen_mut()
                                                        .set_scrollback(p.scrollback);
                                                }
                                            }
                                        }
                                        NavAction::ScrollDown => {
                                            if let Some(p) = panes.get_mut(selected) {
                                                p.scrollback = p.scrollback.saturating_sub(5);
                                                if let Ok(mut parser) = p.parser.lock() {
                                                    parser
                                                        .screen_mut()
                                                        .set_scrollback(p.scrollback);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Mode::Input => {
                                // Ctrl-Q exits input mode without forwarding.
                                if key.code == KeyCode::Char('q')
                                    && key.modifiers.contains(KeyModifiers::CONTROL)
                                {
                                    mode = Mode::Nav;
                                } else if let Some(p) = panes.get(selected) {
                                    if let Some(bytes) = key_to_bytes(key) {
                                        if let Ok(mut w) = p.writer.lock() {
                                            let _ = w.write_all(&bytes);
                                            let _ = w.flush();
                                        }
                                    }
                                }
                            }
                            Mode::ConfirmQuit => match key.code {
                                KeyCode::Char('y' | 'Y') => {
                                    return Ok(LoopOutcome::QuitByUser);
                                }
                                _ => {
                                    mode = Mode::Nav;
                                }
                            },
                        }
                    }
                    Event::Mouse(m) => match m.kind {
                        MouseEventKind::ScrollUp if mode == Mode::Nav => {
                            if let Some(p) = panes.get_mut(selected) {
                                p.scrollback = p.scrollback.saturating_add(3).min(SCROLLBACK_LINES);
                                if let Ok(mut parser) = p.parser.lock() {
                                    parser.screen_mut().set_scrollback(p.scrollback);
                                }
                            }
                        }
                        MouseEventKind::ScrollDown if mode == Mode::Nav => {
                            if let Some(p) = panes.get_mut(selected) {
                                p.scrollback = p.scrollback.saturating_sub(3);
                                if let Ok(mut parser) = p.parser.lock() {
                                    parser.screen_mut().set_scrollback(p.scrollback);
                                }
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
    })();

    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

enum NavAction {
    Quit,
    Enter,
    Next,
    Prev,
    Select(usize),
    ScrollUp,
    ScrollDown,
}

fn nav_key(key: KeyEvent, n: usize) -> Option<NavAction> {
    match key.code {
        KeyCode::Char('q') => Some(NavAction::Quit),
        KeyCode::Enter => Some(NavAction::Enter),
        KeyCode::Char('j') | KeyCode::Down => Some(NavAction::Next),
        KeyCode::Char('k') | KeyCode::Up => Some(NavAction::Prev),
        KeyCode::Char('g') => Some(NavAction::Select(0)),
        KeyCode::Char('G') => Some(NavAction::Select(n.saturating_sub(1))),
        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
            #[expect(
                clippy::as_conversions,
                reason = "guard restricts c to ASCII digits 1-9; cast is lossless"
            )]
            let digit = c as u8;
            let idx = usize::from(digit - b'1');
            Some(NavAction::Select(idx))
        }
        KeyCode::PageUp => Some(NavAction::ScrollUp),
        KeyCode::PageDown => Some(NavAction::ScrollDown),
        _ => None,
    }
}

#[allow(clippy::too_many_lines, reason = "single-pass TUI render; splitting would require threading state through multiple small helpers with no clarity gain")]
fn draw_frame(
    frame: &mut Frame,
    panes: &[Pane],
    selected: usize,
    mode: Mode,
    list_state: &mut ListState,
    parent_label: &str,
    show_time: bool,
) {
    let area = frame.area();
    let sidebar_w = sidebar_width(area.width);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(STATUS_BAR_HEIGHT)])
        .split(area);
    let body = outer[0];
    let status_area = outer[1];

    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(sidebar_w), Constraint::Min(10)])
        .split(body);
    let sidebar_area = split[0];
    let main_area = split[1];

    // Sidebar list.
    let items: Vec<ListItem> = panes
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let icon = match p.status {
                PaneStatus::Running => Span::raw("●").yellow(),
                PaneStatus::Done(RunStatus::Success, _) => Span::raw("✓").green(),
                PaneStatus::Done(RunStatus::Failure | RunStatus::Timeout, _) => {
                    Span::raw("✘").red()
                }
                PaneStatus::Done(RunStatus::Interrupted, _) => Span::raw("!").yellow(),
            };
            let elapsed = format_mmss(p.elapsed());
            let label = truncate_label(&p.label, usize::from(sidebar_w.saturating_sub(10)));
            let line = Line::from(vec![
                icon,
                Span::raw(" "),
                Span::raw(format!("{:1}", digit_for(i))),
                Span::raw(" "),
                Span::raw(label),
                Span::raw(" "),
                Span::raw(elapsed).dim(),
            ]);
            ListItem::new(line)
        })
        .collect();

    let sidebar = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .title(format!(
                    " {} ",
                    truncate_label(parent_label, usize::from(sidebar_w.saturating_sub(4)))
                )),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    frame.render_stateful_widget(sidebar, sidebar_area, list_state);

    // Main pane: render focused pane's vt100 screen.
    let main_border_style = if matches!(mode, Mode::Input) {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let main_title = panes
        .get(selected)
        .map(|p| {
            format!(
                " {} ",
                truncate_label(&p.label, usize::from(main_area.width.saturating_sub(4)))
            )
        })
        .unwrap_or_default();
    let main_block = Block::default()
        .borders(Borders::ALL)
        .border_style(main_border_style)
        .title(main_title);
    let inner = main_block.inner(main_area);
    frame.render_widget(main_block, main_area);

    if let Some(p) = panes.get(selected) {
        let widget = Vt100Widget {
            parser: Arc::clone(&p.parser),
            show_cursor: matches!(mode, Mode::Input) && p.is_running(),
        };
        frame.render_widget(widget, inner);
    }

    // Status bar.
    let status_text = match mode {
        Mode::Nav => "[NAV] Enter: type into pane  •  j/k ↑/↓: navigate  •  1-9: jump  •  PgUp/PgDn: scroll  •  q: quit".to_string(),
        Mode::Input => "[INPUT] Ctrl-Q: leave input mode".to_string(),
        Mode::ConfirmQuit => {
            let running = panes.iter().filter(|p| p.is_running()).count();
            format!("Kill {running} running process(es)? [y/N]")
        }
    };
    let status_style = match mode {
        Mode::Input => Style::default().bg(Color::Blue).fg(Color::White),
        Mode::ConfirmQuit => Style::default().bg(Color::Red).fg(Color::White),
        Mode::Nav => Style::default().bg(Color::DarkGray).fg(Color::White),
    };
    let status = Paragraph::new(status_text).style(status_style);
    frame.render_widget(status, status_area);

    // Show elapsed time in status bar if requested by --time.
    let _ = show_time; // sidebar already shows elapsed; --time only affects post-exit dump.

    if matches!(mode, Mode::ConfirmQuit) {
        let popup_area = centered_rect(40, 5, area);
        frame.render_widget(Clear, popup_area);
        let running = panes.iter().filter(|p| p.is_running()).count();
        let popup = Paragraph::new(vec![
            Line::from(""),
            Line::from(format!("Kill {running} running process(es)?")),
            Line::from(""),
            Line::from(Span::raw("[y/N]").bold()),
        ])
        .alignment(ratatui::layout::Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .title(" Confirm quit "),
        );
        frame.render_widget(popup, popup_area);
    }
}

fn digit_for(i: usize) -> String {
    if i < 9 {
        format!("{}", i + 1)
    } else {
        " ".to_string()
    }
}

fn truncate_label(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max <= 1 {
        return s.chars().take(max).collect();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

fn format_mmss(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}", s / 60, s % 60)
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

struct Vt100Widget {
    parser: Arc<Mutex<Parser>>,
    show_cursor: bool,
}

impl Widget for Vt100Widget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let Ok(parser) = self.parser.lock() else {
            return;
        };
        let screen = parser.screen();
        let (vt_rows, vt_cols) = screen.size();
        let max_rows = area.height.min(vt_rows);
        let max_cols = area.width.min(vt_cols);

        for row in 0..max_rows {
            for col in 0..max_cols {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                let contents = cell.contents();
                let ch = if contents.is_empty() { " " } else { contents };
                let mut style = Style::default()
                    .fg(map_color(cell.fgcolor(), Color::Reset))
                    .bg(map_color(cell.bgcolor(), Color::Reset));
                if cell.bold() {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if cell.italic() {
                    style = style.add_modifier(Modifier::ITALIC);
                }
                if cell.underline() {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                if cell.inverse() {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                let buf_cell = &mut buf[(area.x + col, area.y + row)];
                buf_cell.set_symbol(ch);
                buf_cell.set_style(style);
            }
        }

        if self.show_cursor && !screen.hide_cursor() {
            let (crow, ccol) = screen.cursor_position();
            if crow < max_rows && ccol < max_cols {
                let buf_cell = &mut buf[(area.x + ccol, area.y + crow)];
                buf_cell.set_style(
                    Style::default()
                        .add_modifier(Modifier::REVERSED)
                        .add_modifier(Modifier::SLOW_BLINK),
                );
            }
        }
    }
}

fn map_color(c: vt100::Color, default: Color) -> Color {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn key_to_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    use KeyCode::{
        BackTab, Backspace, Char, Delete, Down, End, Enter, Esc, Home, Insert, Left, Null,
        PageDown, PageUp, Right, Tab, Up, F,
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let mut out: Vec<u8> = Vec::new();

    let body: Vec<u8> = match key.code {
        Char(c) => {
            if ctrl {
                #[expect(
                    clippy::as_conversions,
                    reason = "char->u8 truncation; non-ASCII falls through the ascii_alphabetic check below"
                )]
                let b = c.to_ascii_lowercase() as u8;
                if b.is_ascii_alphabetic() {
                    vec![b & 0x1f]
                } else {
                    match c {
                        ' ' | '@' => vec![0],
                        '[' => vec![0x1b],
                        '\\' => vec![0x1c],
                        ']' => vec![0x1d],
                        '^' => vec![0x1e],
                        '_' => vec![0x1f],
                        '?' => vec![0x7f],
                        _ => c.to_string().into_bytes(),
                    }
                }
            } else {
                c.to_string().into_bytes()
            }
        }
        Enter => vec![b'\r'],
        Backspace => vec![0x7f],
        Tab => vec![b'\t'],
        BackTab => b"\x1b[Z".to_vec(),
        Esc => vec![0x1b],
        Left => b"\x1b[D".to_vec(),
        Right => b"\x1b[C".to_vec(),
        Up => b"\x1b[A".to_vec(),
        Down => b"\x1b[B".to_vec(),
        Home => b"\x1b[H".to_vec(),
        End => b"\x1b[F".to_vec(),
        PageUp => b"\x1b[5~".to_vec(),
        PageDown => b"\x1b[6~".to_vec(),
        Insert => b"\x1b[2~".to_vec(),
        Delete => b"\x1b[3~".to_vec(),
        F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => b"\x1b[15~".to_vec(),
            6 => b"\x1b[17~".to_vec(),
            7 => b"\x1b[18~".to_vec(),
            8 => b"\x1b[19~".to_vec(),
            9 => b"\x1b[20~".to_vec(),
            10 => b"\x1b[21~".to_vec(),
            11 => b"\x1b[23~".to_vec(),
            12 => b"\x1b[24~".to_vec(),
            _ => return None,
        },
        Null => vec![0],
        _ => return None,
    };

    if alt {
        out.push(0x1b);
    }
    out.extend(body);
    Some(out)
}

fn kill_all(panes: &mut [Pane]) {
    for p in panes.iter_mut() {
        if matches!(p.status, PaneStatus::Running) {
            let _ = p.child.kill();
        }
    }
    let deadline = Instant::now() + Duration::from_millis(KILL_GRACE_MS);
    while Instant::now() < deadline {
        let any_alive = panes.iter_mut().any(|p| {
            matches!(p.status, PaneStatus::Running) && !matches!(p.child.try_wait(), Ok(Some(_)))
        });
        if !any_alive {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    // Second pass — anything still alive gets killed again (portable-pty's
    // kill() on Unix sends SIGHUP; that's usually enough).
    for p in panes.iter_mut() {
        if matches!(p.status, PaneStatus::Running) {
            let _ = p.child.kill();
        }
    }
}

fn finalize_pane_statuses(panes: &mut [Pane]) {
    for p in panes.iter_mut() {
        if matches!(p.status, PaneStatus::Running) {
            // Wait briefly for the child to actually exit so we get a real status.
            let deadline = Instant::now() + Duration::from_millis(200);
            loop {
                if let Ok(Some(status)) = p.child.try_wait() {
                    #[expect(
                        clippy::cast_possible_wrap,
                        clippy::as_conversions,
                        reason = "exit codes are at most 255 in practice; wide cast preserves the value"
                    )]
                    let code = status.exit_code() as i32;
                    // Killed children typically come back as Interrupted.
                    let run_status = if code == 0 {
                        RunStatus::Success
                    } else {
                        RunStatus::Interrupted
                    };
                    p.status = PaneStatus::Done(run_status, code);
                    p.finished_at = Some(Instant::now());
                    break;
                }
                if Instant::now() >= deadline {
                    p.status = PaneStatus::Done(RunStatus::Interrupted, 1);
                    p.finished_at = Some(Instant::now());
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

fn pane_to_outcome(p: &Pane) -> Outcome {
    let (status, exit_code) = match p.status {
        PaneStatus::Done(s, c) => (s, c),
        PaneStatus::Running => (RunStatus::Interrupted, 1),
    };
    let elapsed = p.elapsed();
    let body = pane_text_dump(&p.parser);
    Outcome {
        status,
        output: OutputCapture::Captured(CommandOutput {
            stdout: body,
            stderr: String::new(),
            exit_code,
        }),
        elapsed,
        label: p.label.clone(),
        signal_num: None,
    }
}

fn pane_text_dump(parser: &Arc<Mutex<Parser>>) -> String {
    let Ok(parser) = parser.lock() else {
        return String::new();
    };
    let screen = parser.screen();
    // Pull visible rows + any scrollback. vt100 exposes contents() for the
    // visible screen; scrollback rows need separate handling. For v1 we dump
    // contents() which gives the visible state — sufficient for short outputs
    // and matches what the user sees on screen at exit.
    screen.contents()
}
