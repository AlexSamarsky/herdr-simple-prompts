pub mod agent;
pub mod app;
pub mod editor;
mod error;
pub mod herdr;
pub mod model;

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
    Err(AppError::new(
        "toggle",
        "the controller is not available in this build stage",
    ))
}

pub fn run_ui() -> AppResult<()> {
    Err(AppError::new(
        "ui",
        "the overlay is not available in this build stage",
    ))
}
