#![allow(dead_code)]

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, JoinHandle};

static NEXT_DIR: AtomicU64 = AtomicU64::new(1);

pub struct FakeHerdr {
    directory: PathBuf,
    socket: PathBuf,
    worker: Option<JoinHandle<()>>,
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
