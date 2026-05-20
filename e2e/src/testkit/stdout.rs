use std::{future::Future, io::Read};

use gag::BufferRedirect;
use serde_json::{Map, Value};

pub struct StdoutLogCapture {
    buf: BufferRedirect,
}

impl Default for StdoutLogCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl StdoutLogCapture {
    pub fn new() -> Self {
        Self {
            buf: BufferRedirect::stdout().unwrap(),
        }
    }

    pub fn lines(mut self) -> Vec<String> {
        let mut output = String::new();
        self.buf.read_to_string(&mut output).unwrap();

        let lines = output.lines();
        lines.map(|v| v.to_string()).collect()
    }
}

pub trait CaptureStdoutExt: Sized {
    #[allow(async_fn_in_trait)]
    async fn capture_stdout_lines(self) -> Vec<String>;
    #[allow(async_fn_in_trait)]
    async fn capture_stdout_json(self) -> StdOutCaptureBridge;
}

impl<F> CaptureStdoutExt for F
where
    F: Future,
{
    async fn capture_stdout_lines(self) -> Vec<String> {
        let stdout_log = StdoutLogCapture::new();
        let _result = self.await;
        stdout_log.lines()
    }

    async fn capture_stdout_json(self) -> StdOutCaptureBridge {
        let stdout_log = StdoutLogCapture::new();
        let _result = self.await;
        StdOutCaptureBridge::new(stdout_log.lines())
    }
}

pub struct StdOutCaptureBridge {
    pub lines: Vec<String>,
    pub lines_json: Vec<Map<String, Value>>,
}

impl StdOutCaptureBridge {
    pub fn new(lines: Vec<String>) -> Self {
        Self {
            lines_json: lines
                .iter()
                .map(|s| serde_json::from_str(s).expect("failed to parse log line as json"))
                .collect(),
            lines,
        }
    }

    pub fn by_message(&self, msg: &str) -> Option<&Map<String, Value>> {
        self.lines_json
            .iter()
            .find(|v| v.get("message").is_some_and(|v| v == msg))
    }
}
