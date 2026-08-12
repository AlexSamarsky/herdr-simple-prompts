use crate::agent::AgentStatus;
use crate::app::AppState;
use crate::editor::Editor;
use crate::model::Delivery;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use std::time::Instant;

pub fn render(frame: &mut Frame<'_>, app: &AppState, editor: &Editor) {
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

    let history = history_text(app);
    let history_height = wrapped_history_height(&history, areas[0].width);
    let top = history_height
        .saturating_sub(areas[0].height)
        .saturating_sub(app.scroll_from_bottom);
    frame.render_widget(
        Paragraph::new(history)
            .wrap(Wrap { trim: false })
            .scroll((top, 0)),
        areas[0],
    );
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

fn history_text(app: &AppState) -> Text<'static> {
    let mut lines = Vec::new();
    for turn in &app.turns {
        push_prefixed_text(&mut lines, "› ", &turn.prompt.text, Color::Cyan);
        for (index, attachment) in turn.prompt.attachments.iter().enumerate() {
            lines.push(Line::from(format!(
                "  [Image #{}] {}",
                index + 1,
                attachment.display
            )));
        }
        if let Delivery::Failed { reason } = &turn.delivery {
            lines.push(Line::styled(
                format!("  not sent: {reason}"),
                Style::default().fg(Color::Red),
            ));
        }
        if let Some(answer) = &turn.final_answer {
            push_prefixed_text(&mut lines, "• ", &answer.text, Color::Green);
        }
        lines.push(Line::default());
    }
    Text::from(lines)
}

fn push_prefixed_text(lines: &mut Vec<Line<'static>>, prefix: &str, text: &str, color: Color) {
    for (index, text_line) in text.split('\n').enumerate() {
        lines.push(Line::from(vec![
            Span::styled(
                if index == 0 {
                    prefix.to_owned()
                } else {
                    "  ".to_owned()
                },
                Style::default().fg(color),
            ),
            Span::raw(text_line.to_owned()),
        ]));
    }
}

fn wrapped_history_height(history: &Text<'_>, width: u16) -> u16 {
    let width = usize::from(width.max(1));
    history.lines.iter().fold(0_u16, |height, line| {
        let line_width = line
            .spans
            .iter()
            .map(|span| unicode_width::UnicodeWidthStr::width(span.content.as_ref()))
            .sum::<usize>();
        let wrapped = line_width.max(1).div_ceil(width);
        height.saturating_add(u16::try_from(wrapped).unwrap_or(u16::MAX))
    })
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

pub fn render_to_string(app: &AppState, editor: &Editor, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(frame, app, editor)).unwrap();
    let buffer = terminal.backend().buffer();
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
    use super::{editor_visual_cursor, wrapped_text_height};

    #[test]
    fn wrapped_cursor_uses_display_rows_and_columns() {
        assert_eq!(wrapped_text_height("abcdefghij", 4), 3);
        assert_eq!(editor_visual_cursor("abcdefghij", 10, 4), (2, 2));
        assert_eq!(editor_visual_cursor("abcd\n界界a", 12, 4), (2, 1));
    }
}
