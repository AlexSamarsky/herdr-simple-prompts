use super::protocol::{ApiError, Request, Response};
use serde_json::{Value, json};
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug)]
pub enum HerdrError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Api(ApiError),
    Protocol(String),
}

impl HerdrError {
    pub fn api_code(&self) -> Option<&str> {
        match self {
            Self::Api(error) => Some(&error.code),
            _ => None,
        }
    }
}

impl fmt::Display for HerdrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "Herdr socket I/O: {error}"),
            Self::Json(error) => write!(formatter, "invalid Herdr JSON: {error}"),
            Self::Api(error) => write!(formatter, "Herdr {}: {}", error.code, error.message),
            Self::Protocol(message) => write!(formatter, "Herdr protocol: {message}"),
        }
    }
}

impl std::error::Error for HerdrError {}

impl From<std::io::Error> for HerdrError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for HerdrError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub struct HerdrClient {
    socket_path: PathBuf,
    next_id: AtomicU64,
    timeout: Duration,
}

impl HerdrClient {
    pub fn connect(socket_path: impl AsRef<Path>) -> Result<Self, HerdrError> {
        let socket_path = socket_path.as_ref().to_owned();
        if !socket_path.exists() {
            return Err(HerdrError::Protocol(format!(
                "socket does not exist: {}",
                socket_path.display()
            )));
        }
        Ok(Self {
            socket_path,
            next_id: AtomicU64::new(1),
            timeout: DEFAULT_TIMEOUT,
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value, HerdrError> {
        let mut stream = UnixStream::connect(&self.socket_path)?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        let id = format!(
            "herdr-simple-prompts-{}",
            self.next_id.fetch_add(1, Ordering::Relaxed)
        );
        serde_json::to_writer(
            &mut stream,
            &Request {
                id: &id,
                method,
                params,
            },
        )?;
        stream.write_all(b"\n")?;
        stream.flush()?;

        let mut line = Vec::new();
        let bytes = BufReader::new(stream)
            .take(MAX_RESPONSE_BYTES + 1)
            .read_until(b'\n', &mut line)?;
        if bytes == 0 {
            return Err(HerdrError::Protocol("unexpected EOF".to_owned()));
        }
        if line.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(HerdrError::Protocol(format!(
                "response exceeds {MAX_RESPONSE_BYTES} bytes"
            )));
        }
        if line.last() != Some(&b'\n') {
            return Err(HerdrError::Protocol(
                "response ended before newline".to_owned(),
            ));
        }
        let response: Response = serde_json::from_slice(&line)?;
        if response.id != id {
            return Err(HerdrError::Protocol(format!(
                "response id mismatch: expected {id}, received {}",
                response.id
            )));
        }
        if let Some(error) = response.error {
            return Err(HerdrError::Api(error));
        }
        response
            .result
            .ok_or_else(|| HerdrError::Protocol("response has no result or error".to_owned()))
    }

    pub fn agent_prompt(&self, target: &str, text: &str) -> Result<Value, HerdrError> {
        self.call("agent.prompt", json!({"target": target, "text": text}))
    }

    pub fn agent_send_keys(&self, target: &str, keys: &[&str]) -> Result<Value, HerdrError> {
        self.call("agent.send_keys", json!({"target": target, "keys": keys}))
    }

    pub fn agent_get(&self, target: &str) -> Result<Value, HerdrError> {
        self.call("agent.get", json!({"target": target}))
    }

    pub fn pane_get(&self, pane_id: &str) -> Result<Value, HerdrError> {
        self.call("pane.get", json!({"pane_id": pane_id}))
    }

    pub fn pane_focus(&self, pane_id: &str) -> Result<Value, HerdrError> {
        self.call("pane.focus", json!({"pane_id": pane_id}))
    }

    pub fn pane_read_visible(&self, pane_id: &str, lines: u16) -> Result<Value, HerdrError> {
        self.call(
            "pane.read",
            json!({
                "pane_id": pane_id,
                "source": "visible",
                "lines": lines,
                "format": "text",
                "strip_ansi": true
            }),
        )
    }

    pub fn pane_send_input(
        &self,
        pane_id: &str,
        text: Option<&str>,
        keys: &[&str],
    ) -> Result<Value, HerdrError> {
        let mut params = json!({"pane_id": pane_id, "keys": keys});
        if let Some(text) = text {
            params["text"] = Value::String(text.to_owned());
        }
        self.call("pane.send_input", params)
    }

    pub fn plugin_pane_open(&self, source: &str) -> Result<String, HerdrError> {
        let result = self.call(
            "plugin.pane.open",
            json!({
                "plugin_id": "herdr.simple-prompts",
                "entrypoint": "simple-prompts",
                "placement": "overlay",
                "target_pane_id": source,
                "env": {"HERDR_SIMPLE_PROMPTS_SOURCE_PANE": source},
                "focus": true
            }),
        )?;
        result
            .pointer("/plugin_pane/pane/pane_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| HerdrError::Protocol("plugin pane response has no pane id".to_owned()))
    }

    pub fn plugin_pane_focus(&self, pane_id: &str) -> Result<(), HerdrError> {
        self.call("plugin.pane.focus", json!({"pane_id": pane_id}))?;
        Ok(())
    }

    pub fn plugin_pane_close(&self, pane_id: &str) -> Result<(), HerdrError> {
        self.call("plugin.pane.close", json!({"pane_id": pane_id}))?;
        Ok(())
    }
}
