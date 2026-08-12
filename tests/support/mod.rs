#![allow(dead_code)]

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

static NEXT_DIR: AtomicU64 = AtomicU64::new(1);

pub struct FakeHerdr {
    directory: PathBuf,
    socket: PathBuf,
    worker: Option<JoinHandle<()>>,
}

pub struct GrowingFile {
    directory: PathBuf,
    path: PathBuf,
}

impl GrowingFile {
    pub fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "herdr-simple-prompts-growing-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("session.jsonl");
        std::fs::write(&path, []).unwrap();
        Self { directory, path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, content: &str) {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.path)
            .unwrap();
        file.write_all(content.as_bytes()).unwrap();
    }

    pub fn replace(&self, content: &str) {
        let replacement = self.directory.join("replacement.jsonl");
        std::fs::write(&replacement, content).unwrap();
        std::fs::rename(replacement, &self.path).unwrap();
    }

    pub fn truncate_and_regrow(&self, content: &str) {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)
            .unwrap();
        file.write_all(content.as_bytes()).unwrap();
    }
}

impl Drop for GrowingFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

pub struct ScriptedHerdr {
    directory: PathBuf,
    socket: PathBuf,
    requests: Arc<Mutex<Vec<Value>>>,
    worker: Option<JoinHandle<()>>,
}

impl ScriptedHerdr {
    pub fn start(results: Vec<Value>) -> Self {
        Self::start_responses(results.into_iter().map(Ok).collect())
    }

    pub fn start_responses(results: Vec<Result<Value, Value>>) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "herdr-simple-prompts-scripted-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let socket = directory.join("herdr.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let worker_requests = Arc::clone(&requests);
        let worker = thread::spawn(move || {
            for result in results {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                let request: Value = serde_json::from_str(&request).unwrap();
                worker_requests.lock().unwrap().push(request.clone());
                let response = match result {
                    Ok(result) => {
                        serde_json::json!({"id": request["id"], "result": result})
                    }
                    Err(error) => {
                        serde_json::json!({"id": request["id"], "error": error})
                    }
                };
                serde_json::to_writer(&mut stream, &response).unwrap();
                stream.write_all(b"\n").unwrap();
            }
        });
        Self {
            directory,
            socket,
            requests,
            worker: Some(worker),
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    pub fn requests(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for ScriptedHerdr {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

impl FakeHerdr {
    pub fn start(handler: impl FnOnce(Value) -> Value + Send + 'static) -> Self {
        Self::start_raw(move |request, stream| {
            let response = handler(request);
            serde_json::to_writer(&mut *stream, &response).unwrap();
            stream.write_all(b"\n").unwrap();
        })
    }

    pub fn start_raw(handler: impl FnOnce(Value, &mut UnixStream) + Send + 'static) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "herdr-simple-prompts-test-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let socket = directory.join("herdr.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            let request = serde_json::from_str(&request).unwrap();
            handler(request, &mut stream);
        });
        Self {
            directory,
            socket,
            worker: Some(worker),
        }
    }

    pub fn error(code: &'static str, message: &'static str) -> Self {
        Self::start(move |request| {
            serde_json::json!({
                "id": request["id"],
                "error": {"code": code, "message": message}
            })
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }
}

impl Drop for FakeHerdr {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
