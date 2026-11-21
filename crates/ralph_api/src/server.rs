use crate::addr::{check_bind, BindError, BindPolicy};
use crate::admit::{admit, Admission, MAX_HANDLERS};
use crate::route::{overloaded, route, ApiConfig, ApiRequest, ApiResponse};
use crate::token::Token;
use ralph_server::warn::{warn, warn_if_err};
use std::io::Cursor;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tiny_http::{Header, Request, Response, Server};

const POLL: Duration = Duration::from_millis(100);

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error(transparent)]
    Bind(#[from] BindError),
    #[error("could not bind {addr}: {reason}")]
    Listen { addr: SocketAddr, reason: String },
    #[error("the server is not listening on an IP address")]
    NotIp,
    #[error("could not read a request: {0}")]
    Recv(String),
}

pub fn bind(addr: SocketAddr, policy: &BindPolicy) -> Result<Server, ApiError> {
    check_bind(addr, policy)?;
    let server = Server::http(addr).map_err(|error| ApiError::Listen {
        addr,
        reason: error.to_string(),
    })?;
    let bound = server.server_addr().to_ip().ok_or(ApiError::NotIp)?;
    check_bind(bound, policy)?;
    Ok(server)
}

pub fn serve(
    server: Server,
    config: &ApiConfig,
    token: &Token,
    stop: &dyn Fn() -> bool,
) -> Result<(), ApiError> {
    let shared = Arc::new((config.clone(), token.clone()));
    let active = Arc::new(AtomicUsize::new(0));
    while !stop() {
        let received = server
            .recv_timeout(POLL)
            .map_err(|error| ApiError::Recv(error.to_string()))?;
        let Some(request) = received else { continue };
        match admit(active.load(Ordering::SeqCst), MAX_HANDLERS) {
            Admission::Accept => spawn_handler(request, &shared, &active),
            Admission::Reject => respond(request, overloaded()),
        }
    }
    Ok(())
}

fn spawn_handler(request: Request, shared: &Arc<(ApiConfig, Token)>, active: &Arc<AtomicUsize>) {
    active.fetch_add(1, Ordering::SeqCst);
    let (shared, active) = (Arc::clone(shared), Arc::clone(active));
    let spawned = std::thread::Builder::new().spawn(move || {
        let _slot = Slot(active);
        let (config, token) = &*shared;
        let response = route(&to_api_request(&request), config, token);
        respond(request, response);
    });
    if let Err(error) = spawned {
        warn(format!("ralph web: could not start a handler: {error}"));
    }
}

struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn respond(request: Request, response: ApiResponse) {
    warn_if_err(
        request.respond(to_http_response(response)),
        "ralph web: could not write a response",
    );
}

pub fn split_url(url: &str) -> (&str, &str) {
    url.split_once('?').unwrap_or((url, ""))
}

fn header_values(request: &Request, name: &'static str) -> Vec<String> {
    request
        .headers()
        .iter()
        .filter(|header| header.field.equiv(name))
        .map(|header| header.value.to_string())
        .collect()
}

fn header_value(request: &Request, name: &'static str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv(name))
        .map(|header| header.value.to_string())
}

fn to_api_request(request: &Request) -> ApiRequest {
    let (path, query) = split_url(request.url());
    ApiRequest {
        method: request.method().as_str().to_string(),
        path: path.to_string(),
        query: query.to_string(),
        host: header_value(request, "Host"),
        authorization: header_value(request, "Authorization"),
        cookies: header_values(request, "Cookie"),
    }
}

fn to_http_response(response: ApiResponse) -> Response<Cursor<Vec<u8>>> {
    let headers = std::iter::once(("Content-Type", response.content_type))
        .chain(response.headers)
        .filter_map(|(name, value)| Header::from_bytes(name, value).ok());
    headers.fold(
        Response::from_string(response.body).with_status_code(response.status),
        |built, header| built.with_header(header),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_url_separates_path_and_query() {
        assert_eq!(
            split_url("/api/health?token=a&b=c"),
            ("/api/health", "token=a&b=c")
        );
    }

    #[test]
    fn split_url_without_query_is_empty_query() {
        assert_eq!(split_url("/api/health"), ("/api/health", ""));
    }

    #[test]
    fn split_url_keeps_later_question_marks_in_the_query() {
        assert_eq!(split_url("/p?a=1?b"), ("/p", "a=1?b"));
    }
}
