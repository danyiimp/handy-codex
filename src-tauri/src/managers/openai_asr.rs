//! OpenAI Platform API transcription: batch (`gpt-transcribe`) file uploads
//! and live (`gpt-live-transcribe`) realtime streaming sessions. Unlike the
//! Codex path, these use a documented public API surface billed to the user's
//! API key, so no client impersonation or undocumented endpoints are involved.

use anyhow::{Context, Result};
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::io::Cursor;
use std::sync::mpsc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::http;

use crate::settings::AppSettings;

pub const OPENAI_TRANSCRIPTIONS_ENDPOINT: &str = "https://api.openai.com/v1/audio/transcriptions";
pub const OPENAI_FILE_MODEL: &str = "gpt-transcribe";
pub const OPENAI_LIVE_MODEL: &str = "gpt-live-transcribe";

/// GA transcription session connect URL: `intent=transcription` selects a
/// transcription session on the generic realtime endpoint. No model in the
/// query and no OpenAI-Beta header — both are rejected server-side.
const OPENAI_REALTIME_URLS: [&str; 1] = ["wss://api.openai.com/v1/realtime?intent=transcription"];

const INPUT_SAMPLE_RATE: u32 = 16_000;
const REALTIME_SAMPLE_RATE: u32 = 24_000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(30);

/// Resolve the API key: the app setting wins, the `OPENAI_API_KEY`
/// environment variable is the fallback for terminal-launched use.
pub fn resolve_api_key(settings: &AppSettings) -> Result<String> {
    if let Some(key) = settings
        .openai_api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        return Ok(key.to_string());
    }
    if let Some(key) = std::env::var_os("OPENAI_API_KEY")
        .map(|key| key.to_string_lossy().trim().to_string())
        .filter(|key| !key.is_empty())
    {
        return Ok(key);
    }
    anyhow::bail!("OpenAI API key is not configured; add it in Settings or set OPENAI_API_KEY")
}

/// Batch file transcription through the documented `/v1/audio/transcriptions`
/// endpoint. Audio goes up as 16 kHz PCM WAV — the API accepts it directly, so
/// no FFmpeg dependency is needed on this path.
#[derive(Clone, Debug)]
pub struct OpenAiAsrClient {
    api_key: String,
}

impl OpenAiAsrClient {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    pub fn transcribe(&self, audio: &[f32], language: &str) -> Result<String> {
        if audio.is_empty() {
            return Ok(String::new());
        }
        let wav = encode_wav(audio)?;
        // gpt-transcribe takes the plural `languages` field instead of the
        // legacy singular `language`. If the server rejects the hint shape,
        // retry once without any hint rather than failing the dictation.
        let response = self.send(&wav, language).or_else(|first_error| {
            if language != "auto" && !language.trim().is_empty() {
                self.send(&wav, "auto").map_err(|_| first_error)
            } else {
                Err(first_error)
            }
        })?;
        Ok(response.trim().to_string())
    }

    fn send(&self, wav: &[u8], language: &str) -> Result<String> {
        let boundary = format!("----openai-transcribe-{}", uuid::Uuid::new_v4());
        let body = encode_multipart(wav, language, &boundary);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(300))
            .connect_timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .https_only(true)
            .build()?;
        let mut authorization =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", self.api_key))?;
        authorization.set_sensitive(true);
        let response = client
            .post(OPENAI_TRANSCRIPTIONS_ENDPOINT)
            .header(reqwest::header::AUTHORIZATION, authorization)
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body)
            .send()?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!(http_error(status.as_u16()));
        }
        let parsed: TranscriptionResponse = response.json().map_err(|_| {
            anyhow::anyhow!("OpenAI API transcription returned an invalid JSON response")
        })?;
        Ok(parsed.text)
    }
}

fn http_error(status: u16) -> String {
    match status {
        401 => "OpenAI API key rejected (HTTP 401); check the key in Settings".into(),
        403 => "OpenAI API access denied (HTTP 403); check the key's project permissions".into(),
        404 => "OpenAI API transcription model not found (HTTP 404); the model id may have changed"
            .into(),
        429 => "OpenAI API rate limit reached (HTTP 429); try again later".into(),
        code => format!("OpenAI API transcription failed with HTTP {code}"),
    }
}

#[derive(Debug, serde::Deserialize)]
struct TranscriptionResponse {
    text: String,
}

pub(crate) fn encode_wav(audio: &[f32]) -> Result<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: INPUT_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::new(&mut output, spec)?;
    for sample in audio {
        writer.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
    }
    writer.finalize()?;
    Ok(output.into_inner())
}

fn encode_multipart(wav: &[u8], language: &str, boundary: &str) -> Vec<u8> {
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n{}\r\n",
        OPENAI_FILE_MODEL
    )
    .into_bytes();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(wav);
    body.extend_from_slice(b"\r\n");
    if language != "auto" && !language.trim().is_empty() {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"languages\"\r\n\r\n{language}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

/// Incremental text from the live transcription session.
pub enum LiveEvent {
    Delta(String),
}

/// Commands into the live session worker. Uses a tokio channel so the async
/// session loop can select over it while sync callers use `blocking_send`.
enum LiveCmd {
    /// 24 kHz little-endian PCM16 audio to append.
    Audio(Vec<u8>),
    /// Commit the buffer and reply with the final transcript.
    Finalize(mpsc::Sender<Result<String, String>>),
    Cancel,
}

/// A live `gpt-live-transcribe` realtime session. The WebSocket runs as an
/// async task on Tauri's tokio runtime; the recording thread feeds frames
/// through a channel and pumps delta events between feeds, mirroring the
/// local streaming engines' feed/finalize shape.
pub struct OpenAiLiveStream {
    cmd_tx: tokio::sync::mpsc::Sender<LiveCmd>,
    event_rx: mpsc::Receiver<LiveEvent>,
}

impl OpenAiLiveStream {
    /// Connect and configure a transcription session. Blocks until the
    /// handshake and `session.update` complete (or fail), so callers can fall
    /// back to batch transcription on error.
    pub fn connect(api_key: &str, language: Option<&str>) -> Result<Self> {
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<LiveCmd>(64);
        let (event_tx, event_rx) = mpsc::channel::<LiveEvent>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let api_key = api_key.to_string();
        let language = language.map(str::to_string);
        std::thread::Builder::new()
            .name("openai-live-asr".into())
            .spawn(move || {
                tauri::async_runtime::block_on(run_session(
                    api_key, language, cmd_rx, event_tx, ready_tx,
                ))
            })
            .context("cannot spawn OpenAI live session thread")?;
        match ready_rx.recv_timeout(CONNECT_TIMEOUT) {
            Ok(Ok(())) => Ok(Self { cmd_tx, event_rx }),
            Ok(Err(message)) => anyhow::bail!("OpenAI live session failed to start: {message}"),
            Err(_) => anyhow::bail!("OpenAI live session timed out during connect"),
        }
    }

    /// Queue a 16 kHz f32 frame. Cheap: resamples to 24 kHz PCM16 and hands
    /// the bytes to the async session task.
    pub fn feed(&self, pcm: &[f32]) -> Result<()> {
        let bytes = resample_to_pcm16_24k(pcm);
        self.cmd_tx
            .blocking_send(LiveCmd::Audio(bytes))
            .map_err(|_| anyhow::anyhow!("OpenAI live session task exited"))
    }

    /// Drain pending incremental text.
    pub fn drain_events(&mut self) -> Vec<LiveEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Commit the audio buffer and wait (bounded) for the final transcript.
    pub fn finalize(&mut self) -> Result<String> {
        let (reply_tx, reply_rx) = mpsc::channel::<Result<String, String>>();
        self.cmd_tx
            .blocking_send(LiveCmd::Finalize(reply_tx))
            .map_err(|_| anyhow::anyhow!("OpenAI live session task exited"))?;
        match reply_rx.recv_timeout(FINALIZE_TIMEOUT) {
            Ok(Ok(text)) => Ok(text),
            Ok(Err(message)) => anyhow::bail!("OpenAI live transcription failed: {message}"),
            Err(_) => anyhow::bail!(
                "OpenAI live transcription timed out waiting for the final transcript"
            ),
        }
    }

    pub fn cancel(&self) {
        let _ = self.cmd_tx.blocking_send(LiveCmd::Cancel);
    }
}

impl Drop for OpenAiLiveStream {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// The async session task. `ready` receives the handshake outcome before the
/// feed loop starts, so `connect()` can surface connect failures synchronously.
async fn run_session(
    api_key: String,
    language: Option<String>,
    mut cmd_rx: tokio::sync::mpsc::Receiver<LiveCmd>,
    event_tx: mpsc::Sender<LiveEvent>,
    ready: mpsc::Sender<Result<(), String>>,
) {
    match open_session(&api_key, language.as_deref(), &mut cmd_rx, &event_tx).await {
        Ok((mut sink, mut stream)) => {
            let _ = ready.send(Ok(()));
            feed_loop(&mut sink, &mut stream, &mut cmd_rx, &event_tx).await;
        }
        Err(message) => {
            let _ = ready.send(Err(message));
            // Drain commands so early feeds/finalize don't block the caller.
            while let Some(cmd) = cmd_rx.recv().await {
                if let LiveCmd::Finalize(reply) = cmd {
                    let _ = reply.send(Err("OpenAI live session was not established".into()));
                }
            }
        }
    }
}

/// Connect to the first reachable realtime endpoint and configure the
/// transcription session.
async fn open_session(
    api_key: &str,
    language: Option<&str>,
    cmd_rx: &mut tokio::sync::mpsc::Receiver<LiveCmd>,
    _event_tx: &mpsc::Sender<LiveEvent>,
) -> Result<
    (
        futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            tokio_tungstenite::tungstenite::Message,
        >,
        futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
    ),
    String,
> {
    let mut last_error = "no realtime endpoints configured".to_string();
    // Verified working connect shape (probed end-to-end through the local
    // proxy): `/v1/realtime?intent=transcription` opens a GA transcription
    // session with no model in the query and no OpenAI-Beta header. The plain
    // /v1/realtime URL without intent rejects transcription sessions, and the
    // dedicated /v1/realtime/transcription_sessions path is 403'd at the edge
    // for API-key WebSocket clients.
    let connect_urls = [OPENAI_REALTIME_URLS[0].to_string()];
    for url in connect_urls {
        // A prebuilt http::Request is passed through verbatim, so the five
        // handshake headers tungstenite requires must be set here: Host,
        // Connection, Upgrade, Sec-WebSocket-Version, Sec-WebSocket-Key.
        let host = url
            .trim_start_matches("wss://")
            .split('/')
            .next()
            .unwrap_or("api.openai.com");
        let sec_websocket_key = base64::engine::general_purpose::STANDARD
            .encode(uuid::Uuid::new_v4().as_bytes()[..16].to_vec());
        let request = match http::Request::builder()
            .method("GET")
            .uri(&url)
            .header("Host", host)
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", sec_websocket_key)
            .header("Authorization", format!("Bearer {api_key}"))
            // No OpenAI-Beta header: it opts into the retired beta API shape
            // ("beta_api_shape_disabled"); the GA Realtime API needs nothing
            // beyond the bearer token.
            .body(())
        {
            Ok(request) => request,
            Err(error) => {
                last_error = format!("cannot build request for {url}: {error}");
                continue;
            }
        };
        let connected =
            tokio::time::timeout(CONNECT_TIMEOUT, connect_websocket(request, host)).await;
        let (websocket, _response) = match connected {
            Ok(Ok(pair)) => pair,
            Ok(Err(error)) => {
                last_error = format!("{url}: {error}");
                log::debug!("OpenAI realtime endpoint attempt failed: {}", last_error);
                continue;
            }
            Err(_) => {
                last_error = format!("{url}: connect timeout");
                log::debug!("OpenAI realtime endpoint attempt failed: {}", last_error);
                continue;
            }
        };
        let (mut sink, mut stream) = websocket.split();

        // GA realtime session shape: audio input config is nested under
        // `audio.input`, and language hints are the plural `languages` array.
        let mut transcription = serde_json::json!({
            "model": OPENAI_LIVE_MODEL,
            // Low delay trades a little final accuracy for snappier partials,
            // which is what a live dictation preview wants.
            "delay": "low",
        });
        if let Some(language) = language.filter(|lang| *lang != "auto" && !lang.is_empty()) {
            transcription["languages"] = Value::Array(vec![Value::String(language.to_string())]);
        }
        let session = serde_json::json!({
            "type": "transcription",
            "audio": {
                "input": {
                    "format": { "type": "audio/pcm", "rate": 24000 },
                    "transcription": transcription,
                    "turn_detection": null,
                }
            }
        });
        let update = serde_json::json!({ "type": "session.update", "session": session });
        if let Err(error) = sink.send(tungstenite_message(update)).await {
            last_error = format!("session.update failed: {error}");
            continue;
        }
        // Wait for session.created so configuration errors surface at connect.
        loop {
            match tokio::time::timeout(CONNECT_TIMEOUT, stream.next()).await {
                Ok(Some(Ok(message))) => {
                    if let Ok(text) = message.into_text() {
                        if let Some(event_type) =
                            serde_json::from_str::<Value>(&text).ok().and_then(|value| {
                                value
                                    .get("type")
                                    .and_then(Value::as_str)
                                    .map(str::to_string)
                            })
                        {
                            match event_type.as_str() {
                                "session.created" | "session.updated" => {
                                    let _ = cmd_rx; // receiver parked for the feed loop
                                    return Ok((sink, stream));
                                }
                                "error" => {
                                    last_error = format!("session rejected: {}", text);
                                    log::debug!(
                                        "OpenAI realtime endpoint rejected: {}",
                                        last_error
                                    );
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Ok(Some(Err(error))) => {
                    last_error = format!("session setup read failed: {error}");
                    break;
                }
                Ok(None) => {
                    last_error = "session closed during setup".into();
                    break;
                }
                Err(_) => {
                    last_error = "session setup timed out".into();
                    break;
                }
            }
        }
    }
    let _ = _event_tx;
    Err(last_error)
}

/// Establish the WebSocket. When the environment configures an HTTP proxy
/// (this machine reaches OpenAI only through the local v2rayN/xray tunnel,
/// and direct connects are Cloudflare-403'd), always tunnel through it;
/// tokio-tungstenite does not read proxy env vars itself. Direct connect is
/// used only when no proxy is configured.
async fn connect_websocket(
    request: http::Request<()>,
    host: &str,
) -> Result<
    (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    ),
    String,
> {
    if let Some(proxy_addr) = http_proxy_from_env() {
        return connect_via_proxy(&proxy_addr, host, request).await;
    }
    let (websocket, response) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|error| format!("direct connect failed: {error}"))?;
    Ok((websocket, response))
}

/// The `host:port` of an http:// proxy from HTTPS_PROXY/ALL_PROXY (either
/// case), or None. socks5:// proxies are not supported for the WebSocket.
fn http_proxy_from_env() -> Option<String> {
    const KEYS: [&str; 4] = ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"];
    for key in KEYS {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim().to_string();
            if value.is_empty() {
                continue;
            }
            if value.starts_with("http://") {
                return Some(
                    value
                        .trim_start_matches("http://")
                        .trim_end_matches('/')
                        .to_string(),
                );
            }
            if value.starts_with("socks") {
                log::debug!(
                    "Ignoring {} proxy for the OpenAI realtime WebSocket: {}",
                    key,
                    value
                );
                return None;
            }
        }
    }
    None
}

/// Tunnel: TCP to the proxy → CONNECT host:443 → TLS → WebSocket handshake.
async fn connect_via_proxy(
    proxy_addr: &str,
    host: &str,
    request: http::Request<()>,
) -> Result<
    (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    ),
    String,
> {
    let mut tcp = tokio::net::TcpStream::connect(proxy_addr)
        .await
        .map_err(|error| format!("proxy tcp connect: {error}"))?;
    tcp.write_all(format!("CONNECT {host}:443 HTTP/1.1\r\nHost: {host}:443\r\n\r\n").as_bytes())
        .await
        .map_err(|error| format!("proxy connect write: {error}"))?;
    let mut header = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    loop {
        tcp.read(&mut byte)
            .await
            .map_err(|error| format!("proxy connect read: {error}"))?;
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") || header.len() > 8192 {
            break;
        }
    }
    let status = String::from_utf8_lossy(&header);
    if !status.starts_with("HTTP/1.") || !status.contains(" 200") {
        return Err(format!(
            "proxy CONNECT refused: {}",
            status.lines().next().unwrap_or("")
        ));
    }
    let tls_connector = tokio_native_tls::TlsConnector::from(
        native_tls::TlsConnector::new().map_err(|error| format!("tls connector: {error}"))?,
    );
    let tls = tls_connector
        .connect(host, tcp)
        .await
        .map_err(|error| format!("proxy tls handshake: {error}"))?;
    let stream = tokio_tungstenite::MaybeTlsStream::NativeTls(tls);
    let (websocket, response) = tokio_tungstenite::client_async(request, stream)
        .await
        .map_err(|error| format!("proxy websocket handshake: {error}"))?;
    Ok((websocket, response))
}

/// Feed loop: forward appended audio, surface deltas, and answer finalize by
/// committing and awaiting the completed transcript.
async fn feed_loop(
    sink: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::Message,
    >,
    stream: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    cmd_rx: &mut tokio::sync::mpsc::Receiver<LiveCmd>,
    event_tx: &mpsc::Sender<LiveEvent>,
) {
    let mut pending_finalize: Option<mpsc::Sender<Result<String, String>>> = None;
    loop {
        tokio::select! {
            command = cmd_rx.recv() => {
                match command {
                    Some(LiveCmd::Audio(bytes)) => {
                        if bytes.is_empty() {
                            continue;
                        }
                        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        let append = serde_json::json!({
                            "type": "input_audio_buffer.append",
                            "audio": encoded,
                        });
                        if sink.send(tungstenite_message(append)).await.is_err() {
                            reply_error(&mut pending_finalize, "OpenAI live session closed");
                            return;
                        }
                    }
                    Some(LiveCmd::Finalize(reply)) => {
                        let commit = serde_json::json!({ "type": "input_audio_buffer.commit" });
                        if sink.send(tungstenite_message(commit)).await.is_err() {
                            let _ = reply.send(Err("OpenAI live session closed".into()));
                            return;
                        }
                        pending_finalize = Some(reply);
                    }
                    Some(LiveCmd::Cancel) | None => {
                        reply_error(&mut pending_finalize, "OpenAI live session cancelled");
                        return;
                    }
                }
            }
            message = stream.next() => {
                match message {
                    Some(Ok(text_message)) => {
                        let Some(text) = text_message.into_text().ok() else {
                            continue;
                        };
                        match parse_server_event(&text) {
                            ServerEvent::Delta(delta) => {
                                let _ = event_tx.send(LiveEvent::Delta(delta));
                            }
                            ServerEvent::Completed(transcript) => {
                                if let Some(reply) = pending_finalize.take() {
                                    let _ = reply.send(Ok(transcript));
                                }
                            }
                            ServerEvent::Error(message) => {
                                log::warn!("OpenAI live session server error: {}", message);
                                if let Some(reply) = pending_finalize.take() {
                                    let _ = reply.send(Err(message));
                                    return;
                                }
                            }
                            ServerEvent::Ignored => {}
                        }
                    }
                    Some(Err(error)) => {
                        reply_error(&mut pending_finalize, &format!("session read failed: {error}"));
                        return;
                    }
                    None => {
                        reply_error(&mut pending_finalize, "session closed");
                        return;
                    }
                }
            }
        }
    }
}

fn reply_error(pending: &mut Option<mpsc::Sender<Result<String, String>>>, message: &str) {
    if let Some(reply) = pending.take() {
        let _ = reply.send(Err(message.to_string()));
    }
}

fn tungstenite_message(value: Value) -> tokio_tungstenite::tungstenite::Message {
    tokio_tungstenite::tungstenite::Message::Text(value.to_string().into())
}

enum ServerEvent {
    Delta(String),
    Completed(String),
    Error(String),
    Ignored,
}

fn parse_server_event(text: &str) -> ServerEvent {
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => return ServerEvent::Ignored,
    };
    match value.get("type").and_then(Value::as_str) {
        Some("conversation.item.input_audio_transcription.delta") => ServerEvent::Delta(
            value
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ),
        Some("conversation.item.input_audio_transcription.completed") => ServerEvent::Completed(
            value
                .get("transcript")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ),
        Some("error") => ServerEvent::Error(
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("unknown server error")
                .to_string(),
        ),
        _ => ServerEvent::Ignored,
    }
}

/// Linear resample 16 kHz f32 mono → 24 kHz PCM16 little-endian. Speech ASR
/// is insensitive to the interpolation filter shape at this ratio; a sinc
/// kernel would add cost without a measurable accuracy change.
fn resample_to_pcm16_24k(input: &[f32]) -> Vec<u8> {
    if input.is_empty() {
        return Vec::new();
    }
    let ratio = f64::from(REALTIME_SAMPLE_RATE) / f64::from(INPUT_SAMPLE_RATE); // 1.5
    let output_len = ((input.len() as f64) * ratio).floor() as usize;
    let step = 1.0 / ratio; // input samples advanced per output sample (2/3)
    let mut pcm = Vec::with_capacity(output_len * 2);
    let mut position = 0.0f64;
    for _ in 0..output_len {
        let left = position.floor() as usize;
        let right = (left + 1).min(input.len() - 1);
        let fraction = (position - left as f64) as f32;
        let sample = input[left] * (1.0 - fraction) + input[right] * fraction;
        let quantized = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        pcm.extend_from_slice(&quantized.to_le_bytes());
        position += step;
    }
    pcm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multipart_includes_model_file_and_language_hint() {
        let body = encode_multipart(b"wav", "ru", "boundary");
        assert!(body.starts_with(
            b"--boundary\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\ngpt-transcribe\r\n"
        ));
        assert!(body.windows(b"audio.wav".len()).any(|w| w == b"audio.wav"));
        assert!(body.windows(b"languages".len()).any(|w| w == b"languages"));
        assert!(body.ends_with(b"--boundary--\r\n"));
    }

    #[test]
    fn multipart_omits_language_for_auto() {
        let body = encode_multipart(b"wav", "auto", "boundary");
        assert!(!body.windows(b"languages".len()).any(|w| w == b"languages"));
    }

    #[test]
    fn wav_encoder_writes_pcm_wave_header() {
        let wav = encode_wav(&[0.0, 1.0, -1.0]).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(wav[24..28].try_into().unwrap()),
            INPUT_SAMPLE_RATE
        );
    }

    #[test]
    fn resampler_grows_length_by_three_halves() {
        let pcm = resample_to_pcm16_24k(&[0.0; 1600]);
        assert_eq!(pcm.len(), 2400 * 2);
    }

    #[test]
    fn resampler_handles_empty_input() {
        assert!(resample_to_pcm16_24k(&[]).is_empty());
    }

    #[test]
    fn server_event_parsing_extracts_delta_completed_and_error() {
        match parse_server_event(
            r#"{"type":"conversation.item.input_audio_transcription.delta","delta":"при"}"#,
        ) {
            ServerEvent::Delta(text) => assert_eq!(text, "при"),
            _ => panic!("expected delta"),
        }
        match parse_server_event(
            r#"{"type":"conversation.item.input_audio_transcription.completed","transcript":"привет"}"#,
        ) {
            ServerEvent::Completed(text) => assert_eq!(text, "привет"),
            _ => panic!("expected completed"),
        }
        match parse_server_event(r#"{"type":"error","error":{"message":"bad key"}}"#) {
            ServerEvent::Error(message) => assert_eq!(message, "bad key"),
            _ => panic!("expected error"),
        }
        assert!(matches!(
            parse_server_event(r#"{"type":"session.created"}"#),
            ServerEvent::Ignored
        ));
    }

    #[test]
    fn http_errors_name_the_cause() {
        assert!(http_error(401).contains("key rejected"));
        assert!(http_error(429).contains("rate limit"));
    }
}
