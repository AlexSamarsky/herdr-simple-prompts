use crate::agent::AgentStatus;
use crate::app::AppState;
use crate::editor::Editor;
use crate::style::AnsiColor;
use crate::ui::visual_rows::{CellStyle, HistoryDocument, VisualRow, wrap_styled};
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use std::time::Instant;

#[derive(Default)]
pub(crate) struct HistoryRenderCache {
    generation: u64,
    cached_generation: Option<u64>,
    width: Option<u16>,
    document: HistoryDocument,
    #[cfg(test)]
    rebuilds: usize,
}

impl HistoryRenderCache {
    pub(crate) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    fn document_for(&mut self, app: &AppState, width: u16) -> &HistoryDocument {
        if self.cached_generation != Some(self.generation) || self.width != Some(width) {
            self.document = HistoryDocument::from_app(app, width);
            self.cached_generation = Some(self.generation);
            self.width = Some(width);
            #[cfg(test)]
            {
                self.rebuilds += 1;
            }
        }
        &self.document
    }

    #[cfg(test)]
    fn rebuild_count(&self) -> usize {
        self.rebuilds
    }
}

pub(crate) fn render(
    frame: &mut Frame<'_>,
    app: &AppState,
    editor: &Editor,
    history_cache: &mut HistoryRenderCache,
) {
    if app.agent_status == AgentStatus::Blocked {
        render_blocked(frame, app);
        return;
    }
    let area = frame.area();
    let display_text = editor.display_text();
    let working_height = u16::from(app.agent_status == AgentStatus::Working);
    let attachment_rows = attachment_visual_height(app, area.width);
    let editor_rows = wrapped_text_height(display_text, area.width);
    let composer_rows = attachment_rows.saturating_add(editor_rows);
    let composer_height = (composer_rows + 1).clamp(3, (area.height * 2 / 5).max(3));
    let error_height = u16::from(app.visible_error().is_some());
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(error_height),
            Constraint::Length(working_height),
            Constraint::Length(composer_height),
            Constraint::Length(1),
        ])
        .split(area);

    let history = Text::from(
        history_cache
            .document_for(app, areas[0].width)
            .viewport(usize::from(areas[0].height), app.scroll_from_bottom)
            .iter()
            .map(|row| visual_row_line(row, areas[0].width))
            .collect::<Vec<_>>(),
    );
    frame.render_widget(Paragraph::new(history), areas[0]);
    if let Some(error) = app.visible_error() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Error: ", Style::default().fg(Color::Red)),
                Span::raw(error),
            ])),
            areas[1],
        );
    }
    if app.agent_status == AgentStatus::Working {
        let elapsed = app
            .working_since
            .map(|started| Instant::now().saturating_duration_since(started).as_secs())
            .unwrap_or(0);
        frame.render_widget(
            Paragraph::new(format!("Working ({elapsed}s · esc to interrupt)")).style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            areas[2],
        );
    }
    let mut composer_lines = app
        .draft_attachments
        .iter()
        .enumerate()
        .map(|(index, attachment)| {
            Line::styled(
                format!("[Image #{}] {}", index + 1, attachment.display),
                Style::default().fg(Color::Magenta),
            )
        })
        .collect::<Vec<_>>();
    composer_lines.extend(
        app.pending_attachments
            .iter()
            .enumerate()
            .map(|(index, attachment)| {
                Line::styled(
                    format!(
                        "[Image #{}] {} (verifying…)",
                        app.draft_attachments.len() + index + 1,
                        attachment.display
                    ),
                    Style::default().fg(Color::Yellow),
                )
            }),
    );
    if !app.input_enabled {
        composer_lines.push(Line::from(Span::styled(
            "Input disabled · reopen Simple Prompts",
            Style::default().fg(Color::Red),
        )));
    } else if display_text.is_empty() {
        composer_lines.push(Line::from(Span::styled(
            "Write a prompt",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        composer_lines.extend(display_text.lines().map(|line| Line::from(line.to_owned())));
    }
    let composer = Text::from(composer_lines);
    let (editor_row, editor_column) =
        editor_visual_cursor(display_text, editor.display_cursor_byte(), areas[3].width);
    let cursor_content_row = attachment_rows.saturating_add(editor_row);
    let visible_composer_rows = areas[3].height.saturating_sub(1).max(1);
    let composer_scroll = cursor_content_row.saturating_sub(visible_composer_rows - 1);
    frame.render_widget(
        Paragraph::new(composer)
            .block(Block::default().borders(Borders::TOP))
            .wrap(Wrap { trim: false })
            .scroll((composer_scroll, 0)),
        areas[3],
    );
    frame.render_widget(Paragraph::new(footer(app)), areas[4]);

    let (cursor_row, cursor_column) =
        editor_cursor(areas[3], cursor_content_row, editor_column, composer_scroll);
    frame.set_cursor_position((cursor_column, cursor_row));
}

fn render_blocked(frame: &mut Frame<'_>, app: &AppState) {
    let area = frame.area();
    let error_height = u16::from(app.interaction_error.is_some());
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(error_height),
            Constraint::Length(1),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new("INTERACTION REQUIRED").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        areas[0],
    );

    let body = match app.blocked_surface.as_ref() {
        Some(Ok(surface)) => {
            let rows = wrap_styled(surface, usize::from(areas[1].width));
            let start = rows.len().saturating_sub(usize::from(areas[1].height));
            Text::from(
                rows[start..]
                    .iter()
                    .map(|row| visual_row_line(row, areas[1].width))
                    .collect::<Vec<_>>(),
            )
        }
        Some(Err(_)) | None => Text::from("Unable to read native interaction"),
    };
    frame.render_widget(Paragraph::new(body), areas[1]);
    if let Some(error) = &app.interaction_error {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Error: ", Style::default().fg(Color::Red)),
                Span::raw(error.clone()),
            ])),
            areas[2],
        );
    }
    frame.render_widget(
        Paragraph::new("Native Codex/Claude interaction · prefix+m to return"),
        areas[3],
    );
}

fn visual_row_line(row: &VisualRow, width: u16) -> Line<'static> {
    let fill = row.fill.unwrap_or_default();
    let mut spans = row
        .spans
        .iter()
        .map(|span| {
            Span::styled(
                span.text.clone(),
                ratatui_style(merge_styles(fill, span.style)),
            )
        })
        .collect::<Vec<_>>();
    let padding = usize::from(width).saturating_sub(row.cell_width());
    if padding > 0 && row.fill.is_some() {
        spans.push(Span::styled(" ".repeat(padding), ratatui_style(fill)));
    }
    Line::from(spans)
}

fn merge_styles(base: CellStyle, overlay: CellStyle) -> CellStyle {
    CellStyle {
        foreground: overlay.foreground.or(base.foreground),
        background: overlay.background.or(base.background),
        modifiers: crate::style::StyleModifiers {
            bold: base.modifiers.bold || overlay.modifiers.bold,
            dim: base.modifiers.dim || overlay.modifiers.dim,
            italic: base.modifiers.italic || overlay.modifiers.italic,
            underline: base.modifiers.underline || overlay.modifiers.underline,
        },
    }
}

fn ratatui_style(style: CellStyle) -> Style {
    let mut rendered = Style::default();
    if let Some(foreground) = style.foreground {
        rendered = rendered.fg(ratatui_color(foreground));
    }
    if let Some(background) = style.background {
        rendered = rendered.bg(ratatui_color(background));
    }
    if style.modifiers.bold {
        rendered = rendered.add_modifier(Modifier::BOLD);
    }
    if style.modifiers.dim {
        rendered = rendered.add_modifier(Modifier::DIM);
    }
    if style.modifiers.italic {
        rendered = rendered.add_modifier(Modifier::ITALIC);
    }
    if style.modifiers.underline {
        rendered = rendered.add_modifier(Modifier::UNDERLINED);
    }
    rendered
}

fn ratatui_color(color: AnsiColor) -> Color {
    match color {
        AnsiColor::Black => Color::Black,
        AnsiColor::Red => Color::Red,
        AnsiColor::Green => Color::Green,
        AnsiColor::Yellow => Color::Yellow,
        AnsiColor::Blue => Color::Blue,
        AnsiColor::Magenta => Color::Magenta,
        AnsiColor::Cyan => Color::Cyan,
        AnsiColor::White => Color::Gray,
        AnsiColor::BrightBlack => Color::DarkGray,
        AnsiColor::BrightRed => Color::LightRed,
        AnsiColor::BrightGreen => Color::LightGreen,
        AnsiColor::BrightYellow => Color::LightYellow,
        AnsiColor::BrightBlue => Color::LightBlue,
        AnsiColor::BrightMagenta => Color::LightMagenta,
        AnsiColor::BrightCyan => Color::LightCyan,
        AnsiColor::BrightWhite => Color::White,
        AnsiColor::Indexed(index) => Color::Indexed(index),
        AnsiColor::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
    }
}

fn wrapped_text_height(text: &str, width: u16) -> u16 {
    let width = usize::from(width.max(1));
    text.split('\n').fold(0_u16, |height, line| {
        let line_width = unicode_width::UnicodeWidthStr::width(line);
        let wrapped = line_width.max(1).div_ceil(width);
        height.saturating_add(u16::try_from(wrapped).unwrap_or(u16::MAX))
    })
}

fn editor_visual_cursor(text: &str, cursor: usize, width: u16) -> (u16, u16) {
    let width = usize::from(width.max(1));
    let before = &text[..cursor];
    let mut rows = 0_u16;
    let mut lines = before.split('\n').peekable();
    while let Some(line) = lines.next() {
        let line_width = unicode_width::UnicodeWidthStr::width(line);
        if lines.peek().is_some() {
            rows = rows.saturating_add(
                u16::try_from(line_width.max(1).div_ceil(width)).unwrap_or(u16::MAX),
            );
        } else {
            rows = rows.saturating_add(u16::try_from(line_width / width).unwrap_or(u16::MAX));
            return (rows, u16::try_from(line_width % width).unwrap_or(u16::MAX));
        }
    }
    (rows, 0)
}

fn attachment_visual_height(app: &AppState, width: u16) -> u16 {
    let confirmed = app
        .draft_attachments
        .iter()
        .enumerate()
        .map(|(index, attachment)| format!("[Image #{}] {}", index + 1, attachment.display));
    let pending = app
        .pending_attachments
        .iter()
        .enumerate()
        .map(|(index, attachment)| {
            format!(
                "[Image #{}] {} (verifying…)",
                app.draft_attachments.len() + index + 1,
                attachment.display
            )
        });
    confirmed.chain(pending).fold(0_u16, |height, line| {
        height.saturating_add(wrapped_text_height(&line, width))
    })
}

fn footer(app: &AppState) -> String {
    let Some(status) = &app.status_line else {
        return "Simple Prompts · prefix+m to return".to_owned();
    };
    let mut fields = vec![status.agent.to_string()];
    if let Some(model) = &status.model {
        fields.push(model.clone());
    }
    fields.push(status.cwd.display().to_string());
    if let Some(branch) = &status.branch {
        fields.push(branch.clone());
    }
    if let Some(usage) = &status.usage {
        fields.push(usage.clone());
    }
    fields.join(" · ")
}

fn editor_cursor(area: Rect, content_row: u16, column: u16, scroll: u16) -> (u16, u16) {
    (
        (area.y + 1 + content_row.saturating_sub(scroll)).min(area.bottom().saturating_sub(1)),
        (area.x + column).min(area.right().saturating_sub(1)),
    )
}

pub fn render_to_buffer(app: &AppState, editor: &Editor, width: u16, height: u16) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut history_cache = HistoryRenderCache::default();
    terminal
        .draw(|frame| render(frame, app, editor, &mut history_cache))
        .unwrap();
    terminal.backend().buffer().clone()
}

pub fn render_to_string(app: &AppState, editor: &Editor, width: u16, height: u16) -> String {
    let buffer = render_to_buffer(app, editor, width, height);
    let mut output = String::new();
    for y in 0..height {
        for x in 0..width {
            output.push_str(buffer[(x, y)].symbol());
        }
        output.push('\n');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{HistoryRenderCache, editor_visual_cursor, wrapped_text_height};
    use crate::app::{AppEvent, AppState};
    use crate::model::Message;

    #[test]
    fn wrapped_cursor_uses_display_rows_and_columns() {
        assert_eq!(wrapped_text_height("abcdefghij", 4), 3);
        assert_eq!(editor_visual_cursor("abcdefghij", 10, 4), (2, 2));
        assert_eq!(editor_visual_cursor("abcd\n界界a", 12, 4), (2, 1));
    }

    #[test]
    fn history_cache_reuses_until_invalidated_or_resized() {
        let mut cache = HistoryRenderCache::default();
        let mut app = AppState::default();

        cache.document_for(&app, 20);
        cache.document_for(&app, 20);
        assert_eq!(cache.rebuild_count(), 1);

        app.apply(AppEvent::NativeUser(Message::text(
            "u1",
            "changed",
            Some(1),
        )));
        cache.invalidate();
        cache.document_for(&app, 20);
        assert_eq!(cache.rebuild_count(), 2);

        cache.document_for(&app, 21);
        assert_eq!(cache.rebuild_count(), 3);
    }
}
