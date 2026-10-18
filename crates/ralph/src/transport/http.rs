use ralph_core::worker::{basic_auth, server_url, transient_request_error, TRANSIENT_RETRIES};
use std::io::Read;
use std::thread;
use std::time::Duration;
use ureq::Agent;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const STREAM_LIFETIME: Duration = Duration::from_secs(900);
const RETRY_DELAY: Duration = Duration::from_millis(500);

#[derive(Clone)]
pub(crate) struct HttpSse {
    agent: Agent,
    stream: Agent,
    base: String,
    auth: String,
}

impl HttpSse {
    pub(crate) fn new(port: u16, password: &str) -> Self {
        let plain = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(REQUEST_TIMEOUT))
            .build();
        let stream = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_recv_body(Some(STREAM_LIFETIME))
            .build();
        Self {
            agent: plain.into(),
            stream: stream.into(),
            base: server_url(port),
            auth: basic_auth(password),
        }
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub(crate) fn read(mut response: ureq::http::Response<ureq::Body>) -> Result<String, String> {
        let status = response.status();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| e.to_string())?;
        if status.is_success() {
            Ok(body)
        } else {
            Err(format!("HTTP {}: {body}", status.as_u16()))
        }
    }

    pub(crate) fn get(&self, path: &str) -> Result<String, String> {
        let response = self
            .agent
            .get(self.url(path))
            .header("Authorization", &self.auth)
            .call()
            .map_err(|e| e.to_string())?;
        Self::read(response)
    }

    pub(crate) fn post(&self, path: &str, body: &str) -> Result<String, String> {
        let response = self
            .agent
            .post(self.url(path))
            .header("Authorization", &self.auth)
            .header("Content-Type", "application/json")
            .send(body)
            .map_err(|e| e.to_string())?;
        Self::read(response)
    }

    pub(crate) fn post_retrying(&self, path: &str, body: &str) -> Result<String, String> {
        let mut attempt = 0;
        loop {
            match self.post(path, body) {
                Err(message)
                    if attempt < TRANSIENT_RETRIES && transient_request_error(&message) =>
                {
                    attempt += 1;
                    thread::sleep(RETRY_DELAY);
                }
                result => return result,
            }
        }
    }

    pub(crate) fn evict_location(&self, directory: &str) -> Result<String, String> {
        let response = self
            .agent
            .delete(self.url("/api/debug/location"))
            .query("location[directory]", directory)
            .header("Authorization", &self.auth)
            .call()
            .map_err(|e| e.to_string())?;
        Self::read(response)
    }

    pub(crate) fn delete(&self, path: &str) -> Result<String, String> {
        let response = self
            .agent
            .delete(self.url(path))
            .header("Authorization", &self.auth)
            .call()
            .map_err(|e| e.to_string())?;
        Self::read(response)
    }

    pub(crate) fn events(&self, path: &str) -> Result<impl Read + Send, String> {
        let response = self
            .stream
            .get(self.url(path))
            .header("Authorization", &self.auth)
            .header("Accept", "text/event-stream")
            .call()
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status().as_u16()));
        }
        Ok(response.into_body().into_reader())
    }
}
