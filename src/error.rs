use std::fmt;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AppError {
    context: &'static str,
    message: String,
}

impl AppError {
    pub fn new(context: &'static str, message: impl Into<String>) -> Self {
        Self {
            context,
            message: message.into(),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.context, self.message)
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        Self::new("I/O", error.to_string())
    }
}

impl From<&'static str> for AppError {
    fn from(message: &'static str) -> Self {
        Self::new("startup", message)
    }
}
