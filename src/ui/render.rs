use crate::agent::AgentStatus;
use crate::app::AppState;
use crate::editor::Editor;
use crate::model::Delivery;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use std::time::Instant;

const PROMPT_BG: Color = Color::DarkGray;
const PROMPT_FG: Color = Color::White;
const ANSWER_FG: Color = Color::Green;
const PROMPT_PREFIX: &str = "YOU  ";
const ANSWER_PREFIX: &str = "ANSWER  ";

#[derive(Clone, Debug)]
struct HistoryRow {
    line: Line<'static>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PromptSection {
    start_row: u16,
    prompt_rows: u16,
    end_row: u16,
}

#[derive(Clone, Debug, Default)]
struct HistoryDocument {
    rows: Vec<HistoryRow>,
    prompts: Vec<PromptSection>,
}

impl HistoryDocument {
    fn text(&self) -> Text<'static> {
        debug_assert!(self.prompts.iter().all(|section| {
            section.prompt_rows > 0
                && section.start_row.saturating_add(section.prompt_rows) <= section.end_row
                && usize::from(section.end_row) <= self.rows.len()
        }));
        Text::from(
            self.rows
                .iter()
                .map(|row| row.line.clone())
                .collect::<Vec<_>>(),
        )
    }
}

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

    let history = build_history_document(app).text();
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

fn build_history_document(app: &AppState) -> HistoryDocument {
    let mut document = HistoryDocument::default();
    for turn in &app.turns {
        let start_row = document_row_count(&document);
        let mut prompt_lines = turn
            .prompt
            .text
            .split('\n')
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let first_attachment_on_prompt_row = turn.prompt.text.is_empty()
            && turn.prompt.attachments.first().is_some_and(|attachment| {
                prompt_lines[0] = format!("[Image #1] {}", attachment.display);
                true
            });
        prompt_lines.extend(
            turn.prompt
                .attachments
                .iter()
                .enumerate()
                .skip(usize::from(first_attachment_on_prompt_row))
                .map(|(index, attachment)| {
                    format!("[Image #{}] {}", index + 1, attachment.display)
                }),
        );
        if let Delivery::Failed { reason } = &turn.delivery {
            prompt_lines.push(format!("not sent: {reason}"));
        }

        let prompt_style = Style::default().fg(PROMPT_FG).bg(PROMPT_BG);
        for (index, content) in prompt_lines.into_iter().enumerate() {
            let prefix = if index == 0 {
                Span::styled(PROMPT_PREFIX, Style::default().add_modifier(Modifier::BOLD))
            } else {
                Span::raw(" ".repeat(PROMPT_PREFIX.len()))
            };
            document.rows.push(HistoryRow {
                line: Line::from(vec![prefix, Span::raw(content)]).style(prompt_style),
            });
        }
        let prompt_rows = document_row_count(&document).saturating_sub(start_row);

        if let Some(answer) = &turn.final_answer {
            push_answer(&mut document.rows, &answer.text);
        }
        document.rows.push(HistoryRow {
            line: Line::default(),
        });
        document.prompts.push(PromptSection {
            start_row,
            prompt_rows,
            end_row: document_row_count(&document),
        });
    }
    document
}

fn push_answer(rows: &mut Vec<HistoryRow>, text: &str) {
    for (index, text_line) in text.split('\n').enumerate() {
        let prefix = if index == 0 {
            Span::styled(
                ANSWER_PREFIX,
                Style::default().fg(ANSWER_FG).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw(" ".repeat(ANSWER_PREFIX.len()))
        };
        rows.push(HistoryRow {
            line: Line::from(vec![prefix, Span::raw(text_line.to_owned())]),
        });
    }
}

fn document_row_count(document: &HistoryDocument) -> u16 {
    u16::try_from(document.rows.len()).unwrap_or(u16::MAX)
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

pub fn render_to_buffer(app: &AppState, editor: &Editor, width: u16, height: u16) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(frame, app, editor)).unwrap();
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
    use super::{editor_visual_cursor, wrapped_text_height};

    #[test]
    fn wrapped_cursor_uses_display_rows_and_columns() {
        assert_eq!(wrapped_text_height("abcdefghij", 4), 3);
        assert_eq!(editor_visual_cursor("abcdefghij", 10, 4), (2, 2));
        assert_eq!(editor_visual_cursor("abcd\n界界a", 12, 4), (2, 1));
    }
}
