pub mod settings;

use crate::types::Size;
use alacritty_terminal::event::{
    Event, EventListener, Notify, OnResize, WindowSize,
};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{
    Selection, SelectionRange, SelectionType as AlacrittySelectionType,
};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use alacritty_terminal::term::{
    self, cell::{Cell, Flags as CellFlags}, test::TermSize, viewport_to_point, Term, TermMode,
};
use alacritty_terminal::vte::ansi::{CursorShape, Rgb};
use alacritty_terminal::{tty, Grid};
use egui::Modifiers;
use settings::BackendSettings;
use std::borrow::Cow;
use std::cmp::min;
use std::collections::HashMap;
use std::io::Result;
use std::ops::{Index, RangeInclusive};
use std::sync::mpsc::Sender;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

pub type TerminalMode = TermMode;
pub type PtyEvent = Event;
pub type SelectionType = AlacrittySelectionType;

#[derive(Debug, Clone)]
pub enum BackendCommand {
    Write(Vec<u8>),
    Scroll(i32),
    ScrollToTop,
    ScrollToBottom,
    Resize(Size, Size),
    SelectStart(SelectionType, f32, f32, f32),
    SelectUpdate(f32, f32, f32),
    ClearSelection,
    ProcessLink(LinkAction, Point),
    MouseReport(MouseButton, Modifiers, Point, bool),
}

#[derive(Debug, Clone)]
pub enum MouseMode {
    Sgr,
    Normal(bool),
}

impl From<TermMode> for MouseMode {
    fn from(term_mode: TermMode) -> Self {
        if term_mode.contains(TermMode::SGR_MOUSE) {
            MouseMode::Sgr
        } else if term_mode.contains(TermMode::UTF8_MOUSE) {
            MouseMode::Normal(true)
        } else {
            MouseMode::Normal(false)
        }
    }
}

#[derive(Debug, Clone)]
pub enum MouseButton {
    LeftButton = 0,
    MiddleButton = 1,
    RightButton = 2,
    LeftMove = 32,
    MiddleMove = 33,
    RightMove = 34,
    NoneMove = 35,
    ScrollUp = 64,
    ScrollDown = 65,
    Other = 99,
}

#[derive(Debug, Clone)]
pub enum LinkAction {
    Clear,
    Hover,
    Open,
}

#[derive(Clone, Copy, Debug)]
pub struct TerminalSize {
    pub cell_width: u16,
    pub cell_height: u16,
    num_cols: u16,
    num_lines: u16,
    layout_size: Size,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self {
            cell_width: 1,
            cell_height: 1,
            num_cols: 80,
            num_lines: 50,
            layout_size: Size::default(),
        }
    }
}

impl Dimensions for TerminalSize {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn screen_lines(&self) -> usize {
        self.num_lines as usize
    }

    fn columns(&self) -> usize {
        self.num_cols as usize
    }

    fn last_column(&self) -> Column {
        Column(self.num_cols as usize - 1)
    }

    fn bottommost_line(&self) -> Line {
        Line(self.num_lines as i32 - 1)
    }
}

impl From<TerminalSize> for WindowSize {
    fn from(size: TerminalSize) -> Self {
        Self {
            num_lines: size.num_lines,
            num_cols: size.num_cols,
            cell_width: size.cell_width,
            cell_height: size.cell_height,
        }
    }
}

pub struct TerminalBackend {
    pub id: u64,
    pub url_regex: RegexSearch,
    term: Arc<FairMutex<Term<EventProxy>>>,
    size: TerminalSize,
    shared_size: Arc<Mutex<TerminalSize>>,
    notifier: Notifier,
    last_content: RenderableContent,
    child_pid: u32,
    pub lines_written: AtomicU64,
    /// (prev_total, oldest_row_snapshot) — grouped to avoid nested lock acquisitions.
    capture_state: Mutex<(usize, String)>,
    max_scroll_limit: usize,
}

impl TerminalBackend {
    pub fn new(
        id: u64,
        app_context: egui::Context,
        pty_event_proxy_sender: Sender<(u64, PtyEvent)>,
        settings: BackendSettings,
    ) -> Result<Self> {
        let dynamic_colors = settings.dynamic_colors.clone();
        let pty_config = tty::Options {
            shell: Some(tty::Shell::new(settings.shell, settings.args)),
            working_directory: settings.working_directory,
            env: settings.env,
            ..tty::Options::default()
        };
        let config = term::Config::default();
        let terminal_size = TerminalSize::default();
        let pty = tty::new(&pty_config, terminal_size.into(), id)?;
        let child_pid = pty.child().id();
        let (event_sender, event_receiver) = mpsc::channel();
        let event_proxy = EventProxy(event_sender);
        let mut term = Term::new(config, &terminal_size, event_proxy.clone());
        let initial_content = RenderableContent {
            grid: term.grid().clone(),
            selectable_range: None,
            terminal_mode: *term.mode(),
            terminal_size,
            cursor: term.grid_mut().cursor_cell().clone(),
            hovered_hyperlink: None,
            cursor_visible: term.mode().contains(TermMode::SHOW_CURSOR),
            cursor_shape: term.cursor_style().shape,
        };
        let term = Arc::new(FairMutex::new(term));
        let pty_event_loop =
            EventLoop::new(term.clone(), event_proxy, pty, false, false)?;
        let event_loop_sender = pty_event_loop.channel();
        let notifier = Notifier(event_loop_sender.clone());
        let event_notifier = Notifier(event_loop_sender);
        let url_regex = RegexSearch::new(r#"(ipfs:|ipns:|magnet:|mailto:|gemini://|gopher://|https://|http://|news:|file://|git://|ssh:|ftp://)[^\u{0000}-\u{001F}\u{007F}-\u{009F}<>"\s{-}\^⟨⟩`]+"#).unwrap();
        let shared_size = Arc::new(Mutex::new(terminal_size));
        let _pty_event_loop_thread = pty_event_loop.spawn();
        let event_term = term.clone();
        let shared_size_clone = shared_size.clone();
        let _pty_event_subscription = std::thread::Builder::new()
            .name(format!("pty_event_subscription_{}", id))
            .spawn(move || {
                // `while let Ok` exits cleanly when the sender drops (channel
                // closed on TerminalBackend drop), preventing a busy-loop on
                // a dead channel that would spin a full CPU core at shutdown.
                while let Ok(event) = event_receiver.recv() {
                    if let Event::ColorRequest(index, formatter) = &event {
                        if let Some(color) =
                            resolve_dynamic_color(&event_term, &dynamic_colors, *index)
                        {
                            event_notifier.notify(formatter(color).into_bytes());
                        }
                        continue;
                    }
                    // Write responses directly back to the PTY so programs like fzf
                    // that query cursor position (ESC[6n) or text-area size don't hang.
                    if let Event::PtyWrite(text) = &event {
                        event_notifier.notify(text.clone().into_bytes());
                        continue;
                    }
                    if let Event::TextAreaSizeRequest(formatter) = &event {
                        let size = *shared_size_clone.lock().unwrap();
                        event_notifier.notify(formatter(size.into()).into_bytes());
                        continue;
                    }
                    // If the receiver is gone (app shutting down) just exit.
                    if pty_event_proxy_sender.send((id, event.clone())).is_err() {
                        break;
                    }
                    app_context.clone().request_repaint();
                    if let Event::Exit = event {
                        break;
                    }
                }
            })?;

        Ok(Self {
            id,
            url_regex,
            term: term.clone(),
            size: terminal_size,
            shared_size,
            notifier,
            last_content: initial_content,
            child_pid,
            lines_written: AtomicU64::new(0),
            capture_state: Mutex::new((0usize, String::new())),
            max_scroll_limit: 10_000,
        })
    }

    pub fn process_command(&mut self, cmd: BackendCommand) {
        let term = self.term.clone();
        let mut term = term.lock();
        match cmd {
            BackendCommand::Write(input) => {
                self.write(input);
                term.scroll_display(Scroll::Bottom);
            },
            BackendCommand::Scroll(delta) => {
                self.scroll(&mut term, delta);
            },
            BackendCommand::ScrollToTop => {
                log::info!("terminal scroll: jump to top");
                term.grid_mut().scroll_display(Scroll::Top);
            },
            BackendCommand::ScrollToBottom => {
                log::info!("terminal scroll: jump to bottom");
                term.grid_mut().scroll_display(Scroll::Bottom);
            },
            BackendCommand::Resize(layout_size, font_size) => {
                self.resize(&mut term, layout_size, font_size);
            },
            BackendCommand::SelectStart(selection_type, x, y, ppp) => {
                self.start_selection(&mut term, selection_type, x, y, ppp);
            },
            BackendCommand::SelectUpdate(x, y, ppp) => {
                self.update_selection(&mut term, x, y, ppp);
            },
            BackendCommand::ClearSelection => {
                term.selection = None;
            },
            BackendCommand::ProcessLink(link_action, point) => {
                self.process_link_action(&term, link_action, point);
            },
            BackendCommand::MouseReport(button, modifiers, point, pressed) => {
                self.process_mouse_report(button, modifiers, point, pressed);
            },
        };
    }

    pub fn selection_point(
        x: f32,
        y: f32,
        terminal_size: &TerminalSize,
        display_offset: usize,
        pixels_per_point: f32,
    ) -> Point {
        // Snap cell dimensions to physical pixels so hit-testing matches the
        // renderer, which calls .round_to_pixels() on each cell rect. Without
        // snapping, rounding errors accumulate across columns on high-DPI
        // displays, causing selection to lag one cell behind the visual grid.
        let snapped_w = (terminal_size.cell_width as f32 * pixels_per_point).round() / pixels_per_point;
        let snapped_h = (terminal_size.cell_height as f32 * pixels_per_point).round() / pixels_per_point;
        let col = (x / snapped_w) as usize;
        let col = min(Column(col), Column(terminal_size.num_cols as usize - 1));

        let line = (y / snapped_h) as usize;
        let line = min(line, terminal_size.num_lines as usize - 1);

        viewport_to_point(display_offset, Point::new(line, col))
    }

    pub fn selectable_content(&self) -> String {
        let content = self.last_content();
        let mut result = String::new();
        if let Some(range) = content.selectable_range {
            // iter_from excludes the start point, so grab the first character
            // explicitly before the loop — same pattern used in open_link().
            result.push(content.grid.index(range.start).c);
            let mut prev_line: Option<Line> = Some(range.start.line);
            let last_col = content.grid.last_column();
            // iter_from(range.start) instead of display_iter() — display_iter
            // only covers the current viewport, so a selection that starts in
            // scrollback but ends at the live view would silently drop all cells
            // above the visible top.
            for indexed in content.grid.iter_from(range.start) {
                if indexed.point > range.end {
                    break;
                }
                if range.contains(indexed.point) {
                    if let Some(prev) = prev_line {
                        if prev != indexed.point.line {
                            // Only insert a newline if the previous row was NOT
                            // soft-wrapped (WRAPLINE flag on its last cell).
                            let was_wrapped = content.grid[prev][last_col]
                                .flags
                                .contains(CellFlags::WRAPLINE);
                            // Trim trailing spaces from the completed line
                            let trimmed_len = result.trim_end_matches(' ').len();
                            result.truncate(trimmed_len);
                            if !was_wrapped {
                                result.push('\n');
                            }
                        }
                    }
                    result.push(indexed.c);
                    prev_line = Some(indexed.point.line);
                }
            }
            // Trim trailing spaces from the last line
            let trimmed_len = result.trim_end_matches(' ').len();
            result.truncate(trimmed_len);
        }
        result
    }

    pub fn sync(&mut self) -> &RenderableContent {
        let term = self.term.clone();
        let mut terminal = term.lock();
        let selectable_range = match &terminal.selection {
            Some(s) => s.to_range(&terminal),
            None => None,
        };

        // Full-screen TUIs (e.g. Claude Code) use `ALT_SCREEN`.
        //
        // In that mode we must ensure Plexi's scrollback is empty.
        // If we only clear on ALT_SCREEN exit, any scrollback frames that
        // accumulated *during* the session can still get exposed when the
        // user scrolls up.
        let was_alt_screen = self.last_content.terminal_mode.contains(TermMode::ALT_SCREEN);
        let is_alt_screen = terminal.mode().contains(TermMode::ALT_SCREEN);
        if !was_alt_screen && is_alt_screen {
            // Drop any stale history and reset display offset.
            terminal.grid_mut().clear_history();
        }

        let cursor = terminal.grid_mut().cursor_cell().clone();
        self.last_content.grid = terminal.grid().clone();
        self.last_content.selectable_range = selectable_range;
        self.last_content.cursor = cursor.clone();
        self.last_content.terminal_mode = *terminal.mode();
        self.last_content.terminal_size = self.size;
        self.last_content.cursor_visible = terminal.mode().contains(TermMode::SHOW_CURSOR);
        self.last_content.cursor_shape = terminal.cursor_style().shape;
        self.last_content()
    }

    pub fn last_content(&self) -> &RenderableContent {
        &self.last_content
    }

    pub fn child_pid(&self) -> u32 {
        self.child_pid
    }

    /// Advance `lines_written` to account for newly written terminal lines.
    ///
    /// Always updates `prev_total` — including on terminal resize-down — so
    /// the next call correctly counts lines written after the resize.
    fn advance_cursor(
        &self,
        grid: &alacritty_terminal::Grid<alacritty_terminal::term::cell::Cell>,
        total: usize,
        _screen_lines: usize,
        history_size: usize,
    ) {
        use alacritty_terminal::index::{Column, Line};
        let mut state = self.capture_state.lock().unwrap();
        if total > state.0 {
            let delta = total - state.0;
            self.lines_written.fetch_add(delta as u64, Ordering::Relaxed);
        }
        // Always update prev_total — handles terminal resize-down correctly.
        state.0 = total;
        // Saturation: history at max means oldest rows may scroll off without
        // changing `total`. Detect one scrolled line per call via row comparison.
        if history_size > 0 && history_size == self.max_scroll_limit {
            let oldest: String = {
                let row = &grid[Line(-(history_size as i32))];
                (0..grid.columns()).map(|c| row[Column(c)].c).collect()
            };
            if state.1 != oldest {
                self.lines_written.fetch_add(1, Ordering::Relaxed);
                state.1 = oldest;
            }
        }
    }

    fn capture_lines_from_grid(
        grid: &alacritty_terminal::Grid<alacritty_terminal::term::cell::Cell>,
        n: usize,
    ) -> Vec<String> {
        use alacritty_terminal::index::{Column, Line};
        if n == 0 {
            return Vec::new();
        }
        let screen_lines = grid.screen_lines();
        let history_size = grid.history_size();
        let total = screen_lines + history_size;
        let take = n.min(total);
        let columns = grid.columns();
        let first = screen_lines as i32 - take as i32;
        let last = screen_lines as i32 - 1;
        let mut result = Vec::with_capacity(take);
        for line_idx in first..=last {
            let row = &grid[Line(line_idx)];
            let mut s: String = (0..columns).map(|col_idx| row[Column(col_idx)].c).collect();
            let trimmed_len = s.trim_end().len();
            s.truncate(trimmed_len);
            result.push(s);
        }
        result
    }

    /// Read the last `n` lines from the PTY scrollback buffer.
    ///
    /// Returns a `Vec<String>` of at most `n` entries, ordered oldest → newest.
    /// Each string has trailing whitespace stripped. Read-only — does not affect
    /// PTY state or scrollback position.
    pub fn capture_lines(&self, n: usize) -> Vec<String> {
        if n == 0 {
            return Vec::new();
        }
        let term = self.term.lock();
        Self::capture_lines_from_grid(term.grid(), n)
    }

    /// Read the last `n` lines and the current cursor in one atomic snapshot.
    ///
    /// Avoids the TOCTOU between calling `capture_lines` and reading
    /// `lines_written` separately — both values are derived from the same
    /// term lock.
    pub fn capture_lines_with_cursor(&self, n: usize) -> (Vec<String>, u64) {
        let term = self.term.lock();
        let grid = term.grid();
        let screen_lines = grid.screen_lines();
        let history_size = grid.history_size();
        let total = screen_lines + history_size;
        self.advance_cursor(&grid, total, screen_lines, history_size);
        let lw = self.lines_written.load(Ordering::Relaxed);
        let captured = Self::capture_lines_from_grid(grid, n);
        (captured, lw)
    }

    /// Read lines written after `cursor` from the PTY scrollback buffer.
    ///
    /// Returns `(lines, new_cursor, missed)`:
    /// - `lines`: content since cursor, ordered oldest → newest
    /// - `new_cursor`: opaque value — pass back as `cursor` on next call
    /// - `missed`: true if lines scrolled off between reads (cursor too old)
    ///
    /// When `cursor == 0`, returns all available buffer content.
    pub fn capture_lines_since(&self, cursor: u64) -> (Vec<String>, u64, bool) {
        let term = self.term.lock();
        let grid = term.grid();
        let screen_lines = grid.screen_lines();
        let history_size = grid.history_size();
        let total = screen_lines + history_size;
        self.advance_cursor(&grid, total, screen_lines, history_size);
        let lines_written = self.lines_written.load(Ordering::Relaxed);
        if lines_written == 0 {
            return (Vec::new(), 0, false);
        }

        let oldest_available = lines_written.saturating_sub(total as u64);
        let missed = cursor > 0 && cursor < oldest_available;
        let start = if missed || cursor == 0 {
            0
        } else {
            (cursor.saturating_sub(oldest_available)) as usize
        };
        let lines_to_take = total.saturating_sub(start);

        let captured = if lines_to_take == 0 {
            Vec::new()
        } else {
            Self::capture_lines_from_grid(grid, lines_to_take)
        };

        (captured, lines_written, missed)
    }

    fn process_link_action(
        &mut self,
        terminal: &Term<EventProxy>,
        link_action: LinkAction,
        point: Point,
    ) {
        match link_action {
            LinkAction::Hover => {
                self.last_content.hovered_hyperlink = self.regex_match_at(
                    terminal,
                    point,
                    &mut self.url_regex.clone(),
                );
            },
            LinkAction::Clear => {
                self.last_content.hovered_hyperlink = None;
            },
            LinkAction::Open => {
                self.open_link();
            },
        };
    }

    fn open_link(&self) {
        if let Some(range) = &self.last_content.hovered_hyperlink {
            let start = range.start();
            let end = range.end();

            let mut url = String::from(self.last_content.grid.index(*start).c);
            for indexed in self.last_content.grid.iter_from(*start) {
                url.push(indexed.c);
                if indexed.point == *end {
                    break;
                }
            }

            if let Err(e) = open::that(&url) {
                log::warn!("egui_term: failed to open link {:?}: {}", url, e);
            }
        }
    }

    fn process_mouse_report(
        &self,
        button: MouseButton,
        modifiers: Modifiers,
        point: Point,
        pressed: bool,
    ) {
        let mut mods = 0;
        if modifiers.contains(Modifiers::SHIFT) {
            mods += 4;
        }
        if modifiers.contains(Modifiers::ALT) {
            mods += 8;
        }
        if modifiers.contains(Modifiers::COMMAND) {
            mods += 16;
        }

        match MouseMode::from(self.last_content().terminal_mode) {
            MouseMode::Sgr => {
                self.sgr_mouse_report(point, button as u8 + mods, pressed)
            },
            MouseMode::Normal(is_utf8) => {
                if pressed {
                    self.normal_mouse_report(
                        point,
                        button as u8 + mods,
                        is_utf8,
                    )
                } else {
                    self.normal_mouse_report(point, 3 + mods, is_utf8)
                }
            },
        }
    }

    fn sgr_mouse_report(&self, point: Point, button: u8, pressed: bool) {
        let c = if pressed { 'M' } else { 'm' };

        let msg = format!(
            "\x1b[<{};{};{}{}",
            button,
            point.column + 1,
            point.line + 1,
            c
        );

        self.notifier.notify(msg.as_bytes().to_vec());
    }

    fn normal_mouse_report(&self, point: Point, button: u8, is_utf8: bool) {
        let Point { line, column } = point;
        let max_point = if is_utf8 { 2015 } else { 223 };

        if line >= max_point || column >= max_point {
            return;
        }

        let mut msg = vec![b'\x1b', b'[', b'M', 32 + button];

        let mouse_pos_encode = |pos: usize| -> Vec<u8> {
            let pos = 32 + 1 + pos;
            let first = 0xC0 + pos / 64;
            let second = 0x80 + (pos & 63);
            vec![first as u8, second as u8]
        };

        if is_utf8 && column >= Column(95) {
            msg.append(&mut mouse_pos_encode(column.0));
        } else {
            msg.push(32 + 1 + column.0 as u8);
        }

        if is_utf8 && line >= 95 {
            msg.append(&mut mouse_pos_encode(line.0 as usize));
        } else {
            msg.push(32 + 1 + line.0 as u8);
        }

        self.notifier.notify(msg);
    }

    fn start_selection(
        &mut self,
        terminal: &mut Term<EventProxy>,
        selection_type: SelectionType,
        x: f32,
        y: f32,
        pixels_per_point: f32,
    ) {
        let location = Self::selection_point(
            x,
            y,
            &self.size,
            terminal.grid().display_offset(),
            pixels_per_point,
        );
        terminal.selection = Some(Selection::new(
            selection_type,
            location,
            self.selection_side(x),
        ));
    }

    fn update_selection(
        &mut self,
        terminal: &mut Term<EventProxy>,
        x: f32,
        y: f32,
        pixels_per_point: f32,
    ) {
        let display_offset = terminal.grid().display_offset();
        if let Some(ref mut selection) = terminal.selection {
            let location =
                Self::selection_point(x, y, &self.size, display_offset, pixels_per_point);
            selection.update(location, self.selection_side(x));
        }
    }

    fn selection_side(&self, x: f32) -> Side {
        let cell_x = x as usize % self.size.cell_width as usize;
        let half_cell_width = (self.size.cell_width as f32 / 2.0) as usize;

        if cell_x > half_cell_width {
            Side::Right
        } else {
            Side::Left
        }
    }

    fn resize(
        &mut self,
        terminal: &mut Term<EventProxy>,
        layout_size: Size,
        font_size: Size,
    ) {
        let lines = (layout_size.height / font_size.height.floor()) as u16;
        let cols = (layout_size.width / font_size.width.floor()) as u16;
        if lines == 0 || cols == 0 {
            return;
        }

        // Skip SIGWINCH if row/col count and cell dimensions are unchanged —
        // sub-pixel layout changes don't affect the shell's grid.
        if lines == self.size.num_lines
            && cols == self.size.num_cols
            && font_size.height as u16 == self.size.cell_height
            && font_size.width as u16 == self.size.cell_width
        {
            self.size.layout_size = layout_size;
            return;
        }

        self.size = TerminalSize {
            layout_size,
            cell_height: font_size.height as u16,
            cell_width: font_size.width as u16,
            num_lines: lines,
            num_cols: cols,
        };
        *self.shared_size.lock().unwrap() = self.size;

        self.notifier.on_resize(self.size.into());
        terminal.resize(TermSize::new(cols as usize, lines as usize));
        terminal.scroll_display(Scroll::Bottom);
    }

    fn write<I: Into<Cow<'static, [u8]>>>(&self, input: I) {
        self.notifier.notify(input);
    }

    fn scroll(&mut self, terminal: &mut Term<EventProxy>, delta_value: i32) {
        if delta_value != 0 {
            let scroll = Scroll::Delta(delta_value);
            if terminal
                .mode()
                .contains(TermMode::ALTERNATE_SCROLL | TermMode::ALT_SCREEN)
            {
                let line_cmd = if delta_value > 0 { b'A' } else { b'B' };
                let mut content = vec![];

                for _ in 0..delta_value.abs() {
                    content.push(0x1b);
                    content.push(b'O');
                    content.push(line_cmd);
                }

                self.notifier.notify(content);
            } else {
                terminal.grid_mut().scroll_display(scroll);
            }
        }
    }

    /// Based on alacritty/src/display/hint.rs > regex_match_at
    /// Retrieve the match, if the specified point is inside the content matching the regex.
    ///
    /// If the match terminates at the right edge of a row that is NOT marked
    /// `WRAPLINE` (i.e. a client-wrapped row produced by tools like Claude Code,
    /// git, bat, etc. that emit a literal `\n` at their own `$COLUMNS` boundary),
    /// extend the match across the visual wrap so URLs stay clickable as a
    /// single unit. See `extend_url_match_across_client_wraps`.
    fn regex_match_at(
        &self,
        terminal: &Term<EventProxy>,
        point: Point,
        regex: &mut RegexSearch,
    ) -> Option<Match> {
        let m = visible_regex_match_iter(terminal, regex)
            .find(|rm| rm.contains(&point))?;
        Some(extend_url_match_across_client_wraps(terminal, m))
    }
}

/// Return true if `c` is in the URL character set used by `url_regex` —
/// i.e. it would be accepted by `[^\u{0000}-\u{001F}\u{007F}-\u{009F}<>"\s{-}\^⟨⟩\`]`.
/// Keep this in exact sync with the regex in `TerminalBackend::new`.
///
/// Note: inside a regex character class, `{-}` parses as the inclusive range
/// `{` (U+007B) .. `}` (U+007D), which covers `{`, `|`, `}` — NOT a literal
/// `-`. That is why plain hyphens ARE valid URL chars (e.g. `foo-bar.com`).
fn is_url_char(c: char) -> bool {
    // C0 / DEL / C1 control ranges.
    if ('\u{0000}'..='\u{001F}').contains(&c)
        || ('\u{007F}'..='\u{009F}').contains(&c)
    {
        return false;
    }
    // Explicit terminators from the regex char class.
    // `{-}` in the class is the range `{`..=`}` = `{`, `|`, `}`.
    if matches!(c, '<' | '>' | '"' | '{' | '|' | '}' | '^' | '⟨' | '⟩' | '`') {
        return false;
    }
    // All whitespace (regex `\s`).
    if c.is_whitespace() {
        return false;
    }
    true
}

/// Extend a URL match across a "client-wrapped" visual boundary.
///
/// Many CLI tools (Claude Code, git, bat, etc.) wrap long lines themselves at
/// `$COLUMNS` and emit a literal `\n`. Our backend correctly ingests that as a
/// hard newline, so the cell at the right edge of the wrapped row does NOT
/// carry `CellFlags::WRAPLINE`. Alacritty's `RegexIter` therefore stops the
/// match at the newline and the URL is truncated.
///
/// This helper detects that specific pattern and walks the match end rightward
/// across subsequent rows, appending cells as long as:
///   * the row on which the match currently ends is at the terminal's right
///     edge (`last_column`) AND does NOT have `WRAPLINE` set (so we're looking
///     at a client wrap, not a normal soft wrap — alacritty already handles
///     soft wraps internally via `line_search_right`),
///   * the next row begins with a char accepted by the URL char class,
///   * and we haven't yet hit a whitespace or URL-terminating character.
///
/// The heuristic is deliberately conservative: if any condition fails we stop
/// extending. A false negative is just "link not clickable across the break",
/// which is identical to today's behavior; a false positive would attach
/// unrelated adjacent text to the URL.
fn extend_url_match_across_client_wraps(
    terminal: &Term<EventProxy>,
    m: Match,
) -> Match {
    let grid = terminal.grid();
    let last_col = terminal.last_column();
    let bottom = terminal.bottommost_line();

    let start = *m.start();
    let mut end = *m.end();

    loop {
        // The match must currently end at the right edge of its row.
        if end.column != last_col {
            break;
        }
        // And the row's last cell must NOT be alacritty-soft-wrapped (if it
        // were, alacritty's own `line_search_right` would have already
        // followed it and the match would already span the join).
        let end_cell: &Cell = &grid[end];
        if end_cell.flags.contains(CellFlags::WRAPLINE) {
            break;
        }
        // Don't walk past the last visible/scrollback row we can index.
        let next_line = end.line + 1;
        if next_line > bottom {
            break;
        }

        // Peek the first cell of the next row. Only bridge if it looks like a
        // direct continuation of the URL — i.e. a non-whitespace URL-valid
        // character sits at column 0.
        let first_point = Point::new(next_line, Column(0));
        let first_cell: &Cell = &grid[first_point];
        if !is_url_char(first_cell.c) {
            break;
        }

        // Commit the bridge: advance end to (next_line, 0), then walk right
        // along the new row until the URL char class breaks or we hit the
        // right edge.
        end = first_point;
        let mut col = Column(0);
        while col < last_col {
            let peek = Column(col.0 + 1);
            let peek_cell: &Cell = &grid[Point::new(next_line, peek)];
            if !is_url_char(peek_cell.c) {
                break;
            }
            col = peek;
        }
        end.column = col;

        // If we stopped before the right edge, the URL has genuinely ended.
        if col != last_col {
            break;
        }
        // Otherwise loop: another client-wrap may continue the URL further.
    }

    start..=end
}

/// Copied from alacritty/src/display/hint.rs:
/// Iterate over all visible regex matches.
fn visible_regex_match_iter<'a>(
    term: &'a Term<EventProxy>,
    regex: &'a mut RegexSearch,
) -> impl Iterator<Item = Match> + 'a {
    let viewport_start = Line(-(term.grid().display_offset() as i32));
    let viewport_end = viewport_start + term.bottommost_line();
    let mut start =
        term.line_search_left(Point::new(viewport_start, Column(0)));
    let mut end = term.line_search_right(Point::new(viewport_end, Column(0)));
    start.line = start.line.max(viewport_start - 100);
    end.line = end.line.min(viewport_end + 100);

    RegexIter::new(start, end, Direction::Right, term, regex)
        .skip_while(move |rm| rm.end().line < viewport_start)
        .take_while(move |rm| rm.start().line <= viewport_end)
}

fn resolve_dynamic_color(
    term: &Arc<FairMutex<Term<EventProxy>>>,
    dynamic_colors: &HashMap<usize, [u8; 3]>,
    index: usize,
) -> Option<Rgb> {
    if let Some(color) = term.lock().colors()[index] {
        return Some(color);
    }

    dynamic_colors
        .get(&index)
        .map(|[r, g, b]| Rgb {
            r: *r,
            g: *g,
            b: *b,
        })
}

pub struct RenderableContent {
    pub grid: Grid<Cell>,
    pub hovered_hyperlink: Option<RangeInclusive<Point>>,
    pub selectable_range: Option<SelectionRange>,
    pub cursor: Cell,
    pub terminal_mode: TermMode,
    pub terminal_size: TerminalSize,
    pub cursor_visible: bool,
    pub cursor_shape: CursorShape,
}

impl Default for RenderableContent {
    fn default() -> Self {
        Self {
            grid: Grid::new(0, 0, 0),
            hovered_hyperlink: None,
            selectable_range: None,
            cursor: Cell::default(),
            terminal_mode: TermMode::empty(),
            terminal_size: TerminalSize::default(),
            cursor_visible: true,
            cursor_shape: CursorShape::default(),
        }
    }
}

impl Drop for TerminalBackend {
    fn drop(&mut self) {
        let _ = self.notifier.0.send(Msg::Shutdown);
        // Reap the PTY child on a background thread. reap_child polls for up
        // to 200 ms then issues SIGKILL + blocking waitpid — running that on
        // the caller's thread (the render thread during quit) froze the UI.
        #[cfg(unix)]
        {
            let pid = self.child_pid;
            let _ = std::thread::Builder::new()
                .name(format!("pty-reap-{pid}"))
                .spawn(move || reap_child(pid));
        }
    }
}

/// Kill and reap a PTY child process. Sends SIGKILL if the process has not
/// exited within ~200 ms of the caller sending Msg::Shutdown.
#[cfg(unix)]
fn reap_child(pid: u32) {
    use std::os::raw::c_int;
    const SIGKILL: c_int = 9;
    const WNOHANG: c_int = 1;
    const POLL_MS: u64 = 25;
    const POLLS: u32 = 8; // 8 × 25 ms = 200 ms grace period

    extern "C" {
        fn waitpid(pid: libc::pid_t, status: *mut c_int, options: c_int) -> libc::pid_t;
        fn kill(pid: libc::pid_t, sig: c_int) -> c_int;
    }

    let ipid = pid as libc::pid_t;
    for _ in 0..POLLS {
        let result = unsafe { waitpid(ipid, std::ptr::null_mut(), WNOHANG) };
        if result != 0 {
            // Reaped (result == pid) or error (result == -1); either way done.
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
    }
    // Child still alive — escalate to SIGKILL and block until reaped.
    log::warn!("egui_term: PTY child {pid} did not exit after Shutdown — sending SIGKILL");
    unsafe { kill(ipid, SIGKILL) };
    unsafe { waitpid(ipid, std::ptr::null_mut(), 0) };
}

#[derive(Clone)]
pub struct EventProxy(mpsc::Sender<Event>);

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        let _ = self.0.send(event.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::is_url_char;

    /// The URL char predicate must mirror the negated class in `url_regex`:
    /// `[^\u{0000}-\u{001F}\u{007F}-\u{009F}<>"\s{-}\^⟨⟩\`]`.
    /// If these diverge, the cross-wrap extension will either stop too early
    /// or swallow non-URL text — regression-guard the obvious cases here.
    #[test]
    fn is_url_char_accepts_typical_url_bytes() {
        // `-` is a valid URL char (e.g. `foo-bar.com`); see the note in
        // `is_url_char` about `{-}` actually meaning the range `{`..=`}`.
        for c in [
            'a', 'Z', '0', '/', ':', '.', '?', '=', '&', '%', '#', '_', '+',
            '[', ']', '(', ')', '~', '-',
        ] {
            assert!(is_url_char(c), "expected {c:?} to be a URL char");
        }
    }

    #[test]
    fn is_url_char_rejects_terminators_and_whitespace() {
        for c in [
            ' ', '\t', '\n', '\r', '<', '>', '"', '{', '|', '}', '^', '⟨',
            '⟩', '`', '\u{0000}', '\u{001F}', '\u{007F}', '\u{0085}',
        ] {
            assert!(!is_url_char(c), "expected {c:?} to NOT be a URL char");
        }
    }

    /// `reap_child` must kill and reap a process that does not exit on its
    /// own. After it returns, the PID must be gone (kill -0 returns ESRCH).
    #[cfg(unix)]
    #[test]
    fn reap_child_kills_and_reaps_stubborn_process() {
        use std::process::Command;

        let child = Command::new("/bin/sleep")
            .arg("3600")
            .spawn()
            .expect("failed to spawn sleep 3600");
        let pid = child.id();

        // Forget the handle — we want reap_child to do the cleanup.
        std::mem::forget(child);

        // Confirm the child is alive before we reap it.
        let alive_before = unsafe { libc::kill(pid as libc::pid_t, 0) };
        assert_eq!(alive_before, 0, "child {pid} should be alive before reap");

        super::reap_child(pid);

        // Give the OS a tick to update process state.
        std::thread::sleep(std::time::Duration::from_millis(50));

        let alive_after = unsafe { libc::kill(pid as libc::pid_t, 0) };
        assert_eq!(
            alive_after, -1,
            "child {pid} must be gone after reap_child, kill returned {alive_after}"
        );
    }
}
