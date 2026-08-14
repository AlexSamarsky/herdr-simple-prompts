pub mod agent;
pub mod ansi;
pub mod app;
pub mod composer;
pub mod editor;
mod error;
pub mod herdr;
pub mod history;
pub mod local_time;
pub mod markdown;
pub mod model;
pub mod paste;
pub mod state;
pub mod status;
pub mod style;
pub mod toggle;
pub mod transport;
pub mod ui;

pub use error::AppError;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Toggle,
    Ui,
}

impl Mode {
    pub fn parse(argument: Option<&str>) -> AppResult<Self> {
        match argument {
            Some("toggle") => Ok(Self::Toggle),
            Some("ui") => Ok(Self::Ui),
            _ => Err(AppError::new(
                "startup",
                "usage: herdr-simple-prompts <toggle|ui>",
            )),
        }
    }
}

pub fn run_toggle() -> AppResult<()> {
    toggle::run_from_env()
}

pub fn run_ui() -> AppResult<()> {
    ui::run_from_env()
}
