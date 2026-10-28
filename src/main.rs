//! Sofa Remote: control a PC (Windows, Linux X11, macOS) from a phone browser.
//!
//! Run `sofa-remote [--port 8765] [--name "Living room"] [--new-key] [--data DIR]` and open the
//! printed link on the phone. The web app is embedded in the binary; layouts, the access key and
//! layouts.json live next to the executable (or in --data DIR).

mod focus;
mod gamepad;
mod input;
mod keys;
#[cfg(feature = "webrtc")]
mod rtc;
mod session;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::serve::ListenerExt;
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use session::Session;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

const COOKIE_NAME: &str = "remote_token";
const DEFAULT_LAYOUTS: &str = include_str!("../default_layouts.json");
const ASSETS: &[(&str, &str, &str)] = &[
    ("index.html", "text/html; charset=utf-8", include_str!("../web/index.html")),
    ("app.js", "text/javascript; charset=utf-8", include_str!("../web/app.js")),
    ("app.css", "text/css; charset=utf-8", include_str!("../web/app.css")),
];

struct App {
    token: String,
    name: String,
    data_dir: PathBuf,
    clients: Mutex<HashMap<u64, mpsc::UnboundedSender<Message>>>,
    next_id: AtomicU64,
    /// A text field has focus on this PC (see focus.rs)
    text_focus: AtomicBool,
    config_lock: tokio::sync::Mutex<()>,
}

type Shared = Arc<App>;

// ------------------------------------------------------------------ auth

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|c| {
        let (k, v) = c.trim().split_once('=')?;
        (k == COOKIE_NAME).then(|| v.to_string())
    })
}

fn authed(app: &App, headers: &HeaderMap, query: &HashMap<String, String>) -> bool {
    query.get("k").is_some_and(|k| same(k, &app.token))
        || cookie_token(headers).is_some_and(|c| same(&c, &app.token))
}

fn forbidden() -> Response {
    message_page(StatusCode::FORBIDDEN, "Open the link printed in the PC console (it contains the access key).")
}

/// A small HTML page with the mobile meta tags, so plain messages read well on a phone too.
fn message_page(status: StatusCode, text: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1,viewport-fit=cover\">         <meta name=\"theme-color\" content=\"#0e0f13\"><title>Sofa Remote</title>         <style>body{{margin:0;min-height:100vh;display:grid;place-items:center;background:#0e0f13;color:#e8e9ee;         font:17px/1.5 system-ui,sans-serif;padding:24px;box-sizing:border-box;text-align:center}}         a{{color:#7aa2ff}}</style></head><body><div>{text}<br><br><a href=\"/\">Open Sofa Remote</a></div></body></html>"
    );
    (status, [(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], html).into_response()
}

// ------------------------------------------------------------------ HTTP

fn asset(name: &str) -> Option<Response> {
    let (_, mime, embedded) = ASSETS.iter().find(|(n, _, _)| *n == name)?;
    // SOFA_WEB_DIR serves the web app from disk instead of the embedded copy (for development)
    let body = std::env::var_os("SOFA_WEB_DIR")
        .and_then(|dir| std::fs::read_to_string(PathBuf::from(dir).join(name)).ok())
        .unwrap_or_else(|| embedded.to_string());
    Some(([(header::CONTENT_TYPE, *mime), (header::CACHE_CONTROL, "no-cache")], body).into_response())
}

// The page and its scripts hold no secrets, so they load without a key; the app then reads the
// key from the link (?k=, which always wins), from what it saved earlier, or from the cookie.
// Only /api/self and /ws need it.
async fn index() -> Response {
    asset("index.html").unwrap()
}

async fn static_file(Path(file): Path<String>) -> Response {
    asset(&file).unwrap_or_else(|| message_page(StatusCode::NOT_FOUND, "Page not found."))
}

async fn api_self(State(app): State<Shared>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if !authed(&app, &headers, &q) {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": "wrong key" }))).into_response();
    }
    // Lax (not Strict): a link opened from a QR scanner is a cross-site navigation
    let cookie = format!("{COOKIE_NAME}={}; Path=/; Max-Age=31536000; SameSite=Lax; HttpOnly", app.token);
    ([(header::SET_COOKIE, cookie)], Json(json!({ "name": app.name, "key": app.token }))).into_response()
}

async fn ws_upgrade(State(app): State<Shared>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>,
                    ws: WebSocketUpgrade) -> Response {
    if !authed(&app, &headers, &q) {
        return forbidden();
    }
    // c = the phone's own id, so it keeps its virtual gamepad when it reconnects
    let client = q.get("c").filter(|c| !c.is_empty() && c.len() <= 64).cloned();
    ws.on_upgrade(move |socket| ws_conn(app, socket, client))
}

// ------------------------------------------------------------------ layouts config

fn layouts_path(app: &App) -> PathBuf {
    app.data_dir.join("layouts.json")
}

fn load_config(app: &App) -> anyhow::Result<Value> {
    let text = std::fs::read_to_string(layouts_path(app)).unwrap_or_else(|_| DEFAULT_LAYOUTS.to_string());
    Ok(serde_json::from_str(&text)?)
}

fn save_config(app: &App, config: &Value) -> anyhow::Result<()> {
    anyhow::ensure!(config.get("layouts").is_some_and(Value::is_array), "config must be an object with a 'layouts' list");
    let tmp = app.data_dir.join("layouts.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(config)?)?;
    std::fs::rename(tmp, layouts_path(app))?;
    Ok(())
}

fn broadcast(app: &App, except: u64, msg: &Value) {
    let text = msg.to_string();
    for (id, tx) in app.clients.lock().unwrap().iter() {
        if *id != except {
            let _ = tx.send(Message::Text(text.clone().into()));
        }
    }
}

// ------------------------------------------------------------------ clipboard / browser

/// Clipboard lives on its own thread: on X11 the clipboard owner must stay alive to serve pastes.
enum ClipReq {
    Get(tokio::sync::oneshot::Sender<anyhow::Result<String>>),
    Set(String, tokio::sync::oneshot::Sender<anyhow::Result<()>>),
}

fn clipboard() -> &'static std::sync::mpsc::Sender<ClipReq> {
    static TX: std::sync::OnceLock<std::sync::mpsc::Sender<ClipReq>> = std::sync::OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<ClipReq>();
        std::thread::spawn(move || {
            let mut clip = None;
            for req in rx {
                if clip.is_none() {
                    clip = arboard::Clipboard::new().ok();
                }
                let Some(c) = clip.as_mut() else {
                    let err = || anyhow::anyhow!("clipboard unavailable");
                    match req {
                        ClipReq::Get(r) => drop(r.send(Err(err()))),
                        ClipReq::Set(_, r) => drop(r.send(Err(err()))),
                    }
                    continue;
                };
                match req {
                    ClipReq::Get(r) => drop(r.send(c.get_text().map_err(Into::into))),
                    ClipReq::Set(t, r) => drop(r.send(c.set_text(t).map_err(Into::into))),
                }
            }
        });
        tx
    })
}

async fn clip_get() -> anyhow::Result<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    clipboard().send(ClipReq::Get(tx))?;
    rx.await?
}

async fn clip_set(text: String) -> anyhow::Result<()> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    clipboard().send(ClipReq::Set(text, tx))?;
    rx.await?
}

/// Turn a URL, bare domain or search phrase into an http(s) URL.
fn to_url(text: &str) -> String {
    let text = text.trim();
    if (text.starts_with("http://") || text.starts_with("https://")) && text.len() > 8 {
        return text.to_string();
    }
    if !text.is_empty() && !text.contains(char::is_whitespace) && (text.contains('.') || text.starts_with("localhost")) {
        let scheme = if text.starts_with("localhost") { "http://" } else { "https://" };
        return format!("{scheme}{text}");
    }
    let q: String = text.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
        b' ' => "+".into(),
        _ => format!("%{b:02X}"),
    }).collect();
    format!("https://www.google.com/search?q={q}")
}

// ------------------------------------------------------------------ WebSocket

struct Conn {
    app: Shared,
    id: u64,
    tx: mpsc::UnboundedSender<Message>,
    session: Arc<Mutex<Session>>,
    #[cfg(feature = "webrtc")]
    peer: Option<rtc::Peer>,
}

impl Conn {
    fn send_json(&self, v: Value) {
        let _ = self.tx.send(Message::Text(v.to_string().into()));
    }
}

async fn ws_conn(app: Shared, socket: WebSocket, client: Option<String>) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if sink.send(m).await.is_err() {
                break;
            }
        }
    });
    let id = app.next_id.fetch_add(1, Ordering::Relaxed);
    app.clients.lock().unwrap().insert(id, tx.clone());
    let mut conn = Conn {
        app: app.clone(), id, tx,
        session: Arc::new(Mutex::new(Session::new(client.unwrap_or_else(|| format!("conn-{id}"))))),
        #[cfg(feature = "webrtc")]
        peer: None,
    };
    conn.send_json(json!({ "t": "hello", "name": app.name, "keys": keys::table(), "rtc": cfg!(feature = "webrtc") }));
    if app.text_focus.load(Ordering::Relaxed) {
        conn.send_json(json!({ "t": "focus", "text": true }));
    }

    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Binary(data) => {
                let reply = conn.session.lock().unwrap().binary(&data);
                if let Some(reply) = reply {
                    let _ = conn.tx.send(Message::Binary(reply.into()));
                }
            }
            Message::Text(text) => {
                let Ok(m) = serde_json::from_str::<Value>(&text) else { continue };
                let rid = m.get("id").cloned().filter(|v| !v.is_null());
                match handle_json(&mut conn, &m).await {
                    Ok(Some(mut reply)) => {
                        if let Some(rid) = rid {
                            reply["t"] = "reply".into();
                            reply["id"] = rid;
                            conn.send_json(reply);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => conn.send_json(match rid {
                        Some(rid) => json!({ "t": "reply", "id": rid, "error": e.to_string() }),
                        None => json!({ "t": "error", "msg": e.to_string() }),
                    }),
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    conn.session.lock().unwrap().release_all();
    app.clients.lock().unwrap().remove(&id);
    #[cfg(feature = "webrtc")]
    if let Some(peer) = conn.peer.take() {
        rtc::close(peer).await;
    }
    writer.abort();
}

fn str_field<'a>(m: &'a Value, k: &str) -> &'a str {
    m.get(k).and_then(Value::as_str).unwrap_or("")
}

fn key_list(m: &Value) -> anyhow::Result<Vec<u16>> {
    let list = match m.get("keys") {
        Some(Value::Array(a)) => a.clone(),
        Some(v) => vec![v.clone()],
        None => vec![],
    };
    list.iter().map(|k| {
        let name = match k { Value::String(s) => s.clone(), v => v.to_string() };
        keys::code_of(&name).ok_or_else(|| anyhow::anyhow!("unknown key: {name}"))
    }).collect()
}

fn clamp_i32(m: &Value, k: &str) -> i32 {
    m.get(k).and_then(Value::as_f64).unwrap_or(0.0).clamp(-4000.0, 4000.0) as i32
}

/// Handle one JSON message. `Ok(Some(fields))` is sent back as the reply when the message had an id.
async fn handle_json(conn: &mut Conn, m: &Value) -> anyhow::Result<Option<Value>> {
    use input::{Cmd, send};
    let app = &conn.app;
    let mut reply = json!({});
    match str_field(m, "t") {
        "ping" => {
            conn.send_json(json!({ "t": "pong", "id": m.get("id") }));
            return Ok(None);
        }
        "key" => {
            let vk = keys::code_of(str_field(m, "k")).ok_or_else(|| anyhow::anyhow!("unknown key: {}", str_field(m, "k")))?;
            send(Cmd::Combo(vec![vk]));
        }
        "combo" => send(Cmd::Combo(key_list(m)?)),
        t @ ("down" | "up") => {
            let vks = key_list(m)?;
            let mut s = conn.session.lock().unwrap();
            if t == "down" {
                vks.iter().for_each(|&vk| s.key(vk, true));
            } else {
                vks.iter().rev().for_each(|&vk| s.key(vk, false));
            }
        }
        "release" => conn.session.lock().unwrap().release_all(),
        "type" => send(Cmd::Text {
            text: str_field(m, "text").to_string(),
            backspaces: m.get("backspaces").and_then(Value::as_u64).unwrap_or(0).min(1000) as u32,
        }),
        "mouse" => send(match str_field(m, "kind") {
            "move" => Cmd::Move { dx: clamp_i32(m, "dx"), dy: clamp_i32(m, "dy") },
            "scroll" => Cmd::Scroll { dy: clamp_i32(m, "dy"), dx: clamp_i32(m, "dx") },
            "click" => Cmd::Click(input::Click::Left),
            "dblclick" => Cmd::Click(input::Click::Double),
            "rclick" => Cmd::Click(input::Click::Right),
            "mclick" => Cmd::Click(input::Click::Middle),
            other => anyhow::bail!("unknown mouse action: {other}"),
        }),
        "mbtn" => {
            let b = input::Button::from_name(m.get("b").and_then(Value::as_str).unwrap_or("left"))
                .ok_or_else(|| anyhow::anyhow!("unknown mouse button"))?;
            let down = m.get("down").and_then(Value::as_bool).unwrap_or(false);
            conn.session.lock().unwrap().button(b, down);
        }
        "clip_get" => reply["text"] = clip_get().await?.into(),
        "clip_set" => {
            clip_set(str_field(m, "text").to_string()).await?;
            if m.get("paste").and_then(Value::as_bool).unwrap_or(false) {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                send(Cmd::Combo(vec![keys::VK_LCONTROL, keys::VK_V]));
            }
        }
        "open" => {
            let url = to_url(str_field(m, "url"));
            open::that_detached(&url)?;
            reply["url"] = url.into();
        }
        "config_get" => {
            let _g = app.config_lock.lock().await;
            reply["config"] = load_config(app)?;
        }
        "config_put" => {
            let config = m.get("config").cloned().unwrap_or(Value::Null);
            {
                let _g = app.config_lock.lock().await;
                save_config(app, &config)?;
            }
            broadcast(app, conn.id, &json!({ "t": "config", "config": config }));
        }
        "config_reset" => {
            let config = {
                let _g = app.config_lock.lock().await;
                let _ = std::fs::remove_file(layouts_path(app));
                load_config(app)?
            };
            broadcast(app, conn.id, &json!({ "t": "config", "config": config }));
            reply["config"] = config;
        }
        "pad_open" => {
            let client = conn.session.lock().unwrap().client().to_string();
            let player = tokio::task::spawn_blocking(move || gamepad::open(&client)).await?
                .map_err(|e| anyhow::anyhow!(e))?;
            reply["player"] = player.into();
        }
        #[cfg(feature = "webrtc")]
        "rtc_offer" => {
            if let Some(old) = conn.peer.take() {
                rtc::close(old).await;
            }
            let (sdp, peer) = rtc::answer(conn.session.clone(), str_field(m, "sdp").to_string()).await?;
            conn.peer = Some(peer);
            reply["sdp"] = sdp.into();
        }
        other => anyhow::bail!("unknown message type: {other}"),
    }
    Ok(Some(reply))
}

// ------------------------------------------------------------------ startup

fn random_token() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes).expect("no OS random source");
    bytes.iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect()
}

fn lan_ip() -> String {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("10.255.255.255:1")?; // no packet is sent
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".into())
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "PC".into())
}

fn print_qr(url: &str) {
    if let Ok(code) = qrcode::QrCode::new(url) {
        let art = code.render::<qrcode::render::unicode::Dense1x2>()
            .dark_color(qrcode::render::unicode::Dense1x2::Light)
            .light_color(qrcode::render::unicode::Dense1x2::Dark)
            .quiet_zone(true)
            .build();
        println!("{art}");
    }
}

struct Args {
    port: u16,
    name: String,
    new_key: bool,
    no_focus: bool,
    data: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut a = Args { port: 8765, name: hostname(), new_key: false, no_focus: false, data: None };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--port" => a.port = it.next().and_then(|v| v.parse().ok()).unwrap_or(a.port),
            "--name" => a.name = it.next().unwrap_or(a.name),
            "--data" => a.data = it.next().map(PathBuf::from),
            "--new-key" => a.new_key = true,
            "--no-focus" => a.no_focus = true,
            "-h" | "--help" => {
                println!("sofa-remote [--port 8765] [--name NAME] [--data DIR] [--new-key] [--no-focus]");
                println!("  --no-focus  don't watch for text fields on the PC (the phone keyboard won't open by itself)");
                std::process::exit(0);
            }
            other => eprintln!("ignoring unknown argument: {other}"),
        }
    }
    a
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = parse_args();
    let data_dir = match args.data {
        Some(d) => d,
        None => std::env::current_exe()?.parent().map(PathBuf::from).unwrap_or_else(|| ".".into()),
    };
    std::fs::create_dir_all(&data_dir)?;
    let token_file = data_dir.join(".remote_token");
    if args.new_key {
        let _ = std::fs::remove_file(&token_file);
    }
    let token = match std::fs::read_to_string(&token_file) {
        Ok(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => {
            let t = random_token();
            std::fs::write(&token_file, &t)?;
            t
        }
    };

    if let Err(e) = input::start() {
        eprintln!("Input injection unavailable: {e}");
    }

    let app = Arc::new(App {
        token, name: args.name, data_dir, clients: Mutex::new(HashMap::new()),
        next_id: AtomicU64::new(1), text_focus: AtomicBool::new(false), config_lock: tokio::sync::Mutex::new(()),
    });
    if !args.no_focus {
        let a = app.clone();
        focus::watch(move |text| {
            a.text_focus.store(text, Ordering::Relaxed);
            broadcast(&a, 0, &json!({ "t": "focus", "text": text }));
        });
    }
    let router = Router::new()
        .route("/", get(index))
        .route("/ws", get(ws_upgrade))
        .route("/api/self", get(api_self))
        .route("/{file}", get(static_file))
        .with_state(app.clone());

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", args.port)).await?
        .tap_io(|tcp| { let _ = tcp.set_nodelay(true); });
    let url = format!("http://{}:{}/?k={}", lan_ip(), args.port, app.token);
    println!("\nSofa Remote '{}' running. Open on your phone (same Wi-Fi),", app.name);
    println!("or add it under the device menu on a phone already using another PC:\n\n  {url}\n");
    print_qr(&url);
    let source = if layouts_path(&app).exists() { "layouts.json" } else { "built-in default" };
    println!("Layouts: {source} (in {}). Ctrl+C to stop.", app.data_dir.display());
    axum::serve(listener, router).await?;
    Ok(())
}
