use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InteractionInput {
    Text(String),
    Key(&'static str),
}

pub fn map_interaction_key(key: KeyEvent) -> Option<InteractionInput> {
    let named = match (key.code, key.modifiers) {
        (KeyCode::Up, KeyModifiers::NONE) => "up",
        (KeyCode::Down, KeyModifiers::NONE) => "down",
        (KeyCode::Left, KeyModifiers::NONE) => "left",
        (KeyCode::Right, KeyModifiers::NONE) => "right",
        (KeyCode::Tab, KeyModifiers::NONE) => "tab",
        (KeyCode::BackTab, KeyModifiers::NONE | KeyModifiers::SHIFT) => "shift+tab",
        (KeyCode::Char(' '), KeyModifiers::NONE | KeyModifiers::SHIFT) => "space",
        (KeyCode::Enter, KeyModifiers::NONE) => "enter",
        (KeyCode::Backspace, KeyModifiers::NONE) => "backspace",
        (KeyCode::Delete, KeyModifiers::NONE) => "delete",
        (KeyCode::Esc, KeyModifiers::NONE) => "esc",
        (KeyCode::Char(character), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            return Some(InteractionInput::Text(character.to_string()));
        }
        _ => return None,
    };
    Some(InteractionInput::Key(named))
}

pub fn map_interaction_paste(content: &str) -> InteractionInput {
    InteractionInput::Text(content.to_owned())
}
