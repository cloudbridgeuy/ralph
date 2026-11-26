use crate::addr::{check_bind, BindError, BindPolicy};
use crate::admit::{
    admit, counted_peer, counts_toward_peer_cap, Admission, PeerCounts, PeerGuard, MAX_HANDLERS,
    MAX_REJECTED_PER_PEER,
};
use crate::render_api::{plan_body, read_outcome, BodyPlan};
use crate::route::{accepts_body, overloaded, route, ApiConfig, ApiRequest, ApiResponse};
use crate::token::Token;
use ralph_server::warn::{warn, warn_if_err};
use std::io::{Cursor, Read};
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tiny_http::{Header, Request, Response, Server, Timeouts};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub read: Duration,
    pub write: Duration,
    pub header: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            read: Duration::from_secs(10),
            write: Duration::from_secs(10),
            header: Duration::from_secs(10),
        }
    }
}

fn timeouts_for(limits: &Limits) -> Timeouts {
    Timeouts {
        read: Some(limits.read),
        write: Some(limits.write),
        header: Some(limits.header),
    }
}

pub fn bind(addr: SocketAddr, policy: &BindPolicy) -> Result<Server, ApiError> {
    bind_with_limits(addr, policy, &Limits::default())
}

pub fn bind_with_limits(
    addr: SocketAddr,
    policy: &BindPolicy,
    limits: &Limits,
) -> Result<Server, ApiError> {
    check_bind(addr, policy)?;
    let listen_error = |reason: String| ApiError::Listen { addr, reason };
    let listener = TcpListener::bind(addr).map_err(|error| listen_error(error.to_string()))?;
    let server = Server::from_listener_with_timeouts(listener, None, timeouts_for(limits))
        .map_err(|error| listen_error(error.to_string()))?;
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
    let peers = Arc::new(PeerCounts::default());
    while !stop() {
        let received = server
            .recv_timeout(POLL)
            .map_err(|error| ApiError::Recv(error.to_string()))?;
        let Some(request) = received else { continue };
        match admit(active.load(Ordering::SeqCst), MAX_HANDLERS) {
            Admission::Accept => spawn_handler(request, &shared, &active, &peers),
            Admission::Reject => {
                let guard = peer_of(&request).map(|ip| PeerCounts::enter(&peers, ip).0);
                run_off_loop(request, move |request| {
                    reject_overloaded(guard, |after_write| {
                        respond_then(request, overloaded(), after_write);
                    });
                });
            }
        }
    }
    Ok(())
}

fn reject_overloaded(guard: Option<PeerGuard>, respond: impl FnOnce(Box<dyn FnOnce()>)) {
    respond(Box::new(|| ()));
    drop(guard);
}

fn peer_of(request: &Request) -> Option<IpAddr> {
    counted_peer(request.remote_addr().map(SocketAddr::ip))
}

enum Delivery {
    Drain(Option<PeerGuard>),
    Close,
}

fn limit_rejected(
    response: ApiResponse,
    peer: Option<IpAddr>,
    peers: &Arc<PeerCounts>,
) -> (ApiResponse, Delivery) {
    let Some(ip) = peer.filter(|_| counts_toward_peer_cap(response.status)) else {
        return (response, Delivery::Drain(None));
    };
    let (guard, previous) = PeerCounts::enter(peers, ip);
    match admit(previous, MAX_REJECTED_PER_PEER) {
        Admission::Accept => (response, Delivery::Drain(Some(guard))),
        Admission::Reject => (overloaded(), Delivery::Close),
    }
}

fn spawn_handler(
    request: Request,
    shared: &Arc<(ApiConfig, Token)>,
    active: &Arc<AtomicUsize>,
    peers: &Arc<PeerCounts>,
) {
    let slot = Slot::acquire(active);
    let shared = Arc::clone(shared);
    let peers = Arc::clone(peers);
    let peer = peer_of(&request);
    run_off_loop(request, move |request| {
        let (config, token) = &*shared;
        let mut request = request;
        let response = route(&to_api_request(&mut request, config, token), config, token);
        match limit_rejected(response, peer, &peers) {
            (response, Delivery::Drain(guard)) => {
                respond_then(request, response, move || drop(slot));
                drop(guard);
            }
            (response, Delivery::Close) => respond_and_close(request, response, move || drop(slot)),
        }
    });
}

fn run_off_loop(request: Request, job: impl FnOnce(Request) + Send + 'static) {
    let cell = Arc::new(Mutex::new(Some(request)));
    let owned = Arc::clone(&cell);
    let spawned = std::thread::Builder::new().spawn(move || {
        let taken = owned.lock().ok().and_then(|mut guard| guard.take());
        if let Some(request) = taken {
            job(request);
        }
    });
    if let Err(error) = spawned {
        warn(format!("ralph web: could not start a handler: {error}"));
        if let Some(request) = cell.lock().ok().and_then(|mut guard| guard.take()) {
            #[allow(
                clippy::mem_forget,
                reason = "dropping a request drains its body and would block the loop; forgetting it also leaves this connection's tiny_http thread and socket stuck until the process exits, which is acceptable only when the OS cannot start a thread"
            )]
            std::mem::forget(request);
        }
    }
}

struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn acquire(active: &Arc<AtomicUsize>) -> Slot {
        active.fetch_add(1, Ordering::SeqCst);
        Slot(Arc::clone(active))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn respond_then(request: Request, response: ApiResponse, after_write: impl FnOnce()) {
    warn_if_err(
        request.respond_then(to_http_response(response), after_write),
        "ralph web: could not write a response",
    );
}

fn respond_and_close(request: Request, response: ApiResponse, after_write: impl FnOnce()) {
    warn_if_err(
        request.respond_and_close(to_http_response(response), after_write),
        "ralph web: could not write a response",
    );
}

pub(crate) fn split_url(url: &str) -> (&str, &str) {
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

fn read_body(request: &mut Request, plan: BodyPlan) -> (Vec<u8>, bool) {
    let limit = match plan {
        BodyPlan::Skip => return (Vec::new(), false),
        BodyPlan::TooLarge => return (Vec::new(), true),
        BodyPlan::Read(limit) => limit,
    };
    let mut body = Vec::new();
    let read = request
        .as_reader()
        .take(limit as u64)
        .read_to_end(&mut body);
    read_outcome(body, read.is_err())
}

fn to_api_request(request: &mut Request, config: &ApiConfig, token: &Token) -> ApiRequest {
    let (path, query) = split_url(request.url());
    let mut api = ApiRequest {
        method: request.method().as_str().to_string(),
        path: path.to_string(),
        query: query.to_string(),
        host: header_value(request, "Host"),
        authorization: header_value(request, "Authorization"),
        cookies: header_values(request, "Cookie"),
        body: Vec::new(),
        content_type: header_value(request, "Content-Type"),
        body_too_large: false,
    };
    let plan = plan_body(request.body_length(), accepts_body(&api, config, token));
    (api.body, api.body_too_large) = read_body(request, plan);
    api
}

fn to_http_response(response: ApiResponse) -> Response<Cursor<Vec<u8>>> {
    let cookie = response
        .set_cookie
        .as_deref()
        .map(|value| ("Set-Cookie", value));
    let headers = std::iter::once(("Content-Type", response.content_type))
        .chain(response.headers)
        .chain(cookie)
        .filter_map(|(name, value)| Header::from_bytes(name, value).ok());
    headers.fold(
        Response::from_data(response.body).with_status_code(response.status),
        |built, header| built.with_header(header),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::{error, json_ok};
    use serde_json::json;

    fn peer() -> Option<IpAddr> {
        Some(IpAddr::from([100, 64, 0, 9]))
    }

    #[test]
    fn limit_rejected_closes_the_third_rejection_without_counting_it() {
        let peers = Arc::new(PeerCounts::default());
        let mut held = Vec::new();
        for _ in 0..MAX_REJECTED_PER_PEER {
            let (response, delivery) = limit_rejected(error(401, "unauthorized"), peer(), &peers);
            assert_eq!(response.status, 401);
            assert!(matches!(delivery, Delivery::Drain(Some(_))));
            held.push(delivery);
        }
        let (response, delivery) = limit_rejected(error(403, "forbidden host"), peer(), &peers);
        assert_eq!(response.status, 503);
        assert!(matches!(delivery, Delivery::Close));
        let (_probe, previous) = PeerCounts::enter(&peers, peer().unwrap());
        assert_eq!(previous, MAX_REJECTED_PER_PEER);
    }

    #[test]
    fn limit_rejected_never_counts_or_limits_accepted_responses() {
        let peers = Arc::new(PeerCounts::default());
        let held: Vec<_> = (0..MAX_REJECTED_PER_PEER)
            .map(|_| limit_rejected(error(401, "unauthorized"), peer(), &peers))
            .collect();
        let (response, delivery) = limit_rejected(json_ok(json!({})), peer(), &peers);
        assert_eq!(response.status, 200);
        assert!(matches!(delivery, Delivery::Drain(None)));
        drop(held);
    }

    #[test]
    fn reject_overloaded_holds_the_guard_through_the_drain() {
        let peers = Arc::new(PeerCounts::default());
        let ip = IpAddr::from([100, 64, 0, 9]);
        let (guard, _) = PeerCounts::enter(&peers, ip);
        let mut held_during_drain = 0;
        reject_overloaded(Some(guard), |after_write| {
            after_write();
            let (probe, previous) = PeerCounts::enter(&peers, ip);
            held_during_drain = previous;
            drop(probe);
        });
        assert_eq!(held_during_drain, 1);
        let (_probe, after) = PeerCounts::enter(&peers, ip);
        assert_eq!(after, 0);
    }

    #[test]
    fn limit_rejected_ignores_exempt_peers() {
        let peers = Arc::new(PeerCounts::default());
        for _ in 0..5 {
            let (response, delivery) = limit_rejected(error(401, "unauthorized"), None, &peers);
            assert_eq!(response.status, 401);
            assert!(matches!(delivery, Delivery::Drain(None)));
        }
    }
    use std::io::Read;

    #[test]
    fn timeouts_for_maps_each_limit_to_its_direction() {
        let limits = Limits {
            read: Duration::from_millis(1234),
            write: Duration::from_millis(2345),
            header: Duration::from_millis(3456),
        };
        let timeouts = timeouts_for(&limits);
        assert_eq!(timeouts.read, Some(limits.read));
        assert_eq!(timeouts.write, Some(limits.write));
        assert_eq!(timeouts.header, Some(limits.header));
    }

    #[test]
    fn default_limits_are_ten_seconds_each() {
        let limits = Limits::default();
        assert_eq!(limits.read, Duration::from_secs(10));
        assert_eq!(limits.write, Duration::from_secs(10));
    }

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

    #[test]
    fn slot_counts_while_held_and_frees_on_drop() {
        let active = Arc::new(AtomicUsize::new(0));
        let first = Slot::acquire(&active);
        let second = Slot::acquire(&active);
        assert_eq!(active.load(Ordering::SeqCst), 2);
        drop(first);
        assert_eq!(active.load(Ordering::SeqCst), 1);
        drop(second);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_binary_body_is_written_byte_for_byte() {
        let response = ApiResponse {
            status: 200,
            content_type: "font/woff2",
            headers: vec![],
            body: vec![0x77, 0x4f, 0x46, 0x32, 0xff, 0x00, 0xfe],
            set_cookie: None,
        };
        let mut written = Vec::new();
        to_http_response(response)
            .into_reader()
            .read_to_end(&mut written)
            .unwrap();
        assert_eq!(written, vec![0x77, 0x4f, 0x46, 0x32, 0xff, 0x00, 0xfe]);
    }

    #[test]
    fn a_cookie_is_written_as_a_set_cookie_header() {
        let response = ApiResponse {
            status: 302,
            content_type: "text/plain; charset=utf-8",
            headers: vec![("Location", "/")],
            body: Vec::new(),
            set_cookie: Some("n=v; Path=/".to_string()),
        };
        let built = to_http_response(response);
        let cookie = built.headers().iter().find(|h| h.field.equiv("Set-Cookie"));
        assert_eq!(cookie.map(|h| h.value.as_str()), Some("n=v; Path=/"));
    }
}
