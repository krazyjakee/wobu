//! The listener, and the four things that have to be true before a request
//! reaches [`Dispatcher`](crate::Dispatcher).
//!
//! ## 1. It is bound to loopback, and that is not configurable
//!
//! [`Server::start`] takes a port. It does not take an address, and there is no
//! overload that does. Wobu's privacy posture is a list of the places bytes can
//! go, and "a port on the user's LAN" is not going to join it because somebody
//! added a text field. A user who genuinely wants to reach this from another
//! machine has SSH port forwarding, which is a decision made outside Wobu by
//! someone who knows they are making it.
//!
//! ## 2. Anything with an `Origin` header is refused
//!
//! Loopback is not a trust boundary on a desktop machine — every process can
//! reach it, and so can a web page, which is the interesting half. A page on
//! `evil.example` can POST to `http://127.0.0.1:9628` and, if it can also guess
//! or steal the token, read the user's entire world. It cannot set an
//! `Authorization` header cross-origin without a preflight this server never
//! answers, but relying on that alone means relying on the whole CORS
//! specification staying the shape it is. So the check is simpler and blunter:
//! a legitimate MCP client is a program, and programs do not send `Origin`. Any
//! request that carries one is refused before the token is looked at, which also
//! closes DNS rebinding.
//!
//! ## 3. The token is compared in constant time
//!
//! One secret stands between a local process and the user's project. An
//! ordinary `==` on strings returns early on the first differing byte, and a
//! caller who can time a few thousand requests over loopback — which is
//! precisely where timing is cleanest — recovers it one byte at a time.
//!
//! ## 4. The body is bounded
//!
//! A `Content-Length` header is a claim, not a fact. Every body goes through
//! [`http_body_util::Limited`], so a request that promises a megabyte and sends
//! a gigabyte is dropped rather than buffered.

use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use bytes::Bytes;
use http::{
    HeaderMap, Method, Request as HttpRequest, Response as HttpResponse, StatusCode, header,
};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::config::Token;
use crate::dispatch::Dispatcher;
use crate::jsonrpc::{INVALID_REQUEST, Incoming as Message, PARSE_ERROR, Request, Response};
use crate::{Error, Result};

/// The path an MCP client is pointed at. `/` is accepted too, because a user
/// who pastes the bare origin into a client's config should not have to debug a
/// 404 to find out about a suffix.
pub const PATH: &str = "/mcp";

/// One megabyte. A `tools/call` for this server is a handful of ids and a patch;
/// nothing legitimate comes close, and the ceiling is what stops a body from
/// being a memory-exhaustion primitive.
const MAX_BODY_BYTES: usize = 1024 * 1024;

pub struct Server;

impl Server {
    /// Bind loopback and start accepting. Nothing before this call opens a
    /// socket, and the caller only reaches it with an explicit `enabled`.
    ///
    /// Port `0` asks the OS for a free one, which is what the tests use and what
    /// a user who does not care can pick; [`Running::port`] reports what was
    /// actually taken, so the settings pane can show the address rather than the
    /// intention.
    pub async fn start(port: u16, token: Token, dispatcher: Arc<Dispatcher>) -> Result<Running> {
        // `Ipv4Addr::LOCALHOST`, written here and nowhere else. See the module
        // header: this is the line that makes the guarantee.
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
        let listener = TcpListener::bind(addr).await.map_err(|source| {
            if source.kind() == std::io::ErrorKind::AddrInUse {
                Error::PortInUse { port }
            } else {
                Error::Bind { addr, source }
            }
        })?;
        let bound = listener.local_addr().map_err(Error::Io)?;

        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        let context = Arc::new(Context { token, dispatcher });

        tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    // Biased so that a stop that arrives at the same moment as a
                    // connection wins. A listener that served one last request
                    // after being switched off is exactly the thing this feature
                    // must not do.
                    biased;
                    _ = &mut stop_rx => break,
                    accepted = listener.accept() => accepted,
                };
                let (stream, peer) = match accepted {
                    Ok(pair) => pair,
                    // A refused or reset connection is not a reason to stop
                    // listening; a broken listener would report the same error
                    // forever, so yield to avoid a hot loop.
                    Err(_) => {
                        tokio::task::yield_now().await;
                        continue;
                    }
                };
                // Belt and braces against a platform that resolves the bind
                // address more generously than asked. Nothing non-loopback can
                // arrive here, and if it somehow did it would be dropped without
                // being read.
                if !peer.ip().is_loopback() {
                    continue;
                }
                let context = Arc::clone(&context);
                tokio::spawn(async move {
                    let service = service_fn(move |request| serve(request, Arc::clone(&context)));
                    let _ =
                        http1::Builder::new().serve_connection(TokioIo::new(stream), service).await;
                });
            }
        });

        Ok(Running { addr: bound, stop: Some(stop_tx) })
    }
}

/// A listener that is up. Dropping it takes the port down.
///
/// The `Drop` is the whole design. Settings holds one of these in an `Option`;
/// turning the server off is `= None`, and there is no path where the flag says
/// "off" and a socket is still open — which is the failure mode that would make
/// the disclosure a lie.
pub struct Running {
    addr: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
}

impl Running {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// `http://127.0.0.1:<port>/mcp`, for pasting into a client's config.
    pub fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}{PATH}", self.addr.port())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

struct Context {
    token: Token,
    dispatcher: Arc<Dispatcher>,
}

async fn serve(
    request: HttpRequest<Incoming>,
    context: Arc<Context>,
) -> std::result::Result<HttpResponse<Full<Bytes>>, Infallible> {
    Ok(match route(request, context).await {
        Ok(response) => response,
        Err(response) => response,
    })
}

/// `Err` is a refusal that is already a complete response — written this way so
/// that every gate below is one `?` and no path can fall through a check by
/// forgetting an `else`.
async fn route(
    request: HttpRequest<Incoming>,
    context: Arc<Context>,
) -> std::result::Result<HttpResponse<Full<Bytes>>, HttpResponse<Full<Bytes>>> {
    let path = request.uri().path().to_owned();
    if path != PATH && path != "/" {
        return Err(plain(StatusCode::NOT_FOUND, format!("Wobu's MCP endpoint is at {PATH}.")));
    }

    // Before authentication, deliberately: a page that is probing for a local
    // MCP server should not be able to tell a wrong token from a right one.
    if request.headers().contains_key(header::ORIGIN) {
        return Err(plain(
            StatusCode::FORBIDDEN,
            "This endpoint does not serve web pages. MCP clients are programs and do not send \
             an Origin header.",
        ));
    }

    if request.method() != Method::POST {
        // No GET, which also means no SSE stream. See `lib.rs` on what this
        // crate deliberately does not implement.
        let mut response = plain(
            StatusCode::METHOD_NOT_ALLOWED,
            "Wobu's MCP endpoint takes a JSON-RPC POST. There is no event stream.",
        );
        response.headers_mut().insert(header::ALLOW, "POST".parse().expect("a static token"));
        return Err(response);
    }

    if !authorised(request.headers(), &context.token) {
        let mut response = plain(
            StatusCode::UNAUTHORIZED,
            "Send Wobu's MCP token as `Authorization: Bearer <token>`. It is shown in \
             Settings → Agent access.",
        );
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, "Bearer".parse().expect("a static token"));
        return Err(response);
    }

    let body = Limited::new(request.into_body(), MAX_BODY_BYTES)
        .collect()
        .await
        .map(|collected| collected.to_bytes())
        .map_err(|_| {
            plain(
                StatusCode::PAYLOAD_TOO_LARGE,
                "That request body is larger than Wobu's MCP endpoint accepts.",
            )
        })?;

    let message: Message = match serde_json::from_slice(&body) {
        Ok(message) => message,
        Err(error) => {
            return Ok(json_response(
                StatusCode::OK,
                &serde_json::to_value(Response::error(
                    Value::Null,
                    PARSE_ERROR,
                    format!("that is not a JSON-RPC message: {error}"),
                ))
                .unwrap_or_else(|_| json!({})),
            ));
        }
    };

    let requests = match message {
        Message::Single(request) => vec![request],
        Message::Batch(requests) if requests.is_empty() => {
            return Ok(json_response(
                StatusCode::OK,
                &serde_json::to_value(Response::error(
                    Value::Null,
                    INVALID_REQUEST,
                    "an empty batch is not a request",
                ))
                .unwrap_or_else(|_| json!({})),
            ));
        }
        Message::Batch(requests) => requests,
    };

    // The world behind the dispatcher is a `parking_lot::Mutex` around a SQLite
    // index — synchronous by construction, and the same lock every Tauri command
    // takes. Running it on a blocking thread is what keeps one slow tool call
    // from parking the runtime that the app's own jobs are on.
    let dispatcher = Arc::clone(&context.dispatcher);
    let answers = tokio::task::spawn_blocking(move || dispatch_all(&dispatcher, &requests))
        .await
        .unwrap_or_default();

    // Every message in the batch was a notification: nothing to say, and saying
    // `{}` would be a response to something that asked for none.
    if answers.is_empty() {
        return Ok(empty(StatusCode::ACCEPTED));
    }
    let payload = if answers.len() == 1 {
        answers.into_iter().next().unwrap_or(Value::Null)
    } else {
        Value::Array(answers)
    };
    Ok(json_response(StatusCode::OK, &payload))
}

fn dispatch_all(dispatcher: &Dispatcher, requests: &[Request]) -> Vec<Value> {
    requests
        .iter()
        .filter_map(|request| dispatcher.handle(request))
        .filter_map(|response| serde_json::to_value(response).ok())
        .collect()
}

/// Constant time, and `false` for a missing or malformed header rather than a
/// separate code path — a caller learns "no" and nothing about why.
fn authorised(headers: &HeaderMap, token: &Token) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            let (scheme, credential) = value.split_once(' ')?;
            scheme.eq_ignore_ascii_case("bearer").then(|| credential.trim())
        })
        .is_some_and(|candidate| token.matches(candidate))
}

fn json_response(status: StatusCode, payload: &Value) -> HttpResponse<Full<Bytes>> {
    let body = serde_json::to_vec(payload).unwrap_or_else(|_| b"{}".to_vec());
    HttpResponse::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        // No `Access-Control-Allow-Origin`, ever. Its absence is what stops a
        // browser reading an answer even if one were somehow produced for it.
        .header(header::CACHE_CONTROL, "no-store")
        .body(Full::new(Bytes::from(body)))
        .expect("a response with static headers builds")
}

fn plain(status: StatusCode, message: impl Into<String>) -> HttpResponse<Full<Bytes>> {
    HttpResponse::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Full::new(Bytes::from(message.into())))
        .expect("a response with static headers builds")
}

fn empty(status: StatusCode) -> HttpResponse<Full<Bytes>> {
    HttpResponse::builder()
        .status(status)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Full::new(Bytes::new()))
        .expect("a response with static headers builds")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bearer_token_is_accepted_in_any_case_and_nothing_else_is() {
        let token = Token::from_raw("abcdef0123456789");
        let header = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::AUTHORIZATION, value.parse().unwrap());
            headers
        };

        assert!(authorised(&header("Bearer abcdef0123456789"), &token));
        assert!(authorised(&header("bearer abcdef0123456789"), &token));
        assert!(!authorised(&header("Bearer abcdef0123456780"), &token));
        assert!(!authorised(&header("Basic abcdef0123456789"), &token));
        assert!(!authorised(&header("abcdef0123456789"), &token));
        // A prefix of the real token must not pass. It would if the comparison
        // were on the shorter length.
        assert!(!authorised(&header("Bearer abcdef"), &token));
        assert!(!authorised(&HeaderMap::new(), &token));
    }
}
