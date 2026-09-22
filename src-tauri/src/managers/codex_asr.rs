use anyhow::{Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde::Deserialize;
use serde_json::Value;
use std::io::Cursor;
use std::path::{Path, PathBuf};

pub const CODEX_ASR_ENDPOINT: &str = "https://chatgpt.com/backend-api/transcribe";

const SAMPLE_RATE: u32 = 16_000;

#[derive(Clone, Debug)]
pub struct CodexAsrClient {
    auth_file: PathBuf,
    endpoint: String,
}

impl CodexAsrClient {
    pub fn new() -> Self {
        Self {
            auth_file: default_auth_file(),
            endpoint: CODEX_ASR_ENDPOINT.to_string(),
        }
    }

    pub fn with_auth_file(auth_file: impl Into<PathBuf>) -> Self {
        Self {
            auth_file: auth_file.into(),
            endpoint: CODEX_ASR_ENDPOINT.to_string(),
        }
    }

    pub fn transcribe(&self, audio: &[f32], language: &str) -> Result<String> {
        if audio.is_empty() {
            return Ok(String::new());
        }
        let auth = load_auth(&self.auth_file)?;
        let audio = encode_webm(audio)?;
        let boundary = format!("----codex-transcribe-{}", uuid::Uuid::new_v4());
        let body = encode_multipart(&audio, language, &boundary);
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .connect_timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .https_only(true)
            .build()?;
        let send = |auth: &CodexAuth| -> Result<reqwest::blocking::Response> {
            let mut authorization =
                HeaderValue::from_str(&format!("Bearer {}", auth.access_token))?;
            authorization.set_sensitive(true);
            let mut request = client
                .post(&self.endpoint)
                .header(AUTHORIZATION, authorization)
                .header("originator", "Codex Desktop")
                .header(USER_AGENT, desktop_user_agent())
                .header(
                    CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(body.clone());
            if let Some(account_id) = &auth.account_id {
                request = request.header("ChatGPT-Account-Id", account_id);
            }
            Ok(request.send()?)
        };
        let mut response = send(&auth)?;
        // Codex owns token refresh. Re-read once if it refreshed the same account
        // while the request was in flight; never rotate its shared refresh token.
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            if let Ok(updated) = load_auth(&self.auth_file) {
                if can_retry_with_auth(&auth, &updated) {
                    response = send(&updated)?;
                }
            }
        }
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!(http_error(status.as_u16()));
        }
        // Error pages can contain private data. Do not copy the body into logs.
        let response: TranscriptionResponse = response.json().map_err(|_| {
            anyhow::anyhow!("Codex transcription returned an invalid JSON response")
        })?;
        Ok(response.text.trim().to_string())
    }
}

fn can_retry_with_auth(previous: &CodexAuth, updated: &CodexAuth) -> bool {
    previous.access_token != updated.access_token
        && matches!(
            (&previous.account_id, &updated.account_id),
            (Some(previous), Some(updated)) if previous == updated
        )
}

impl Default for CodexAsrClient {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub(crate) struct CodexAuth {
    pub(crate) access_token: String,
    pub(crate) account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TranscriptionResponse {
    text: String,
}

pub(crate) fn default_auth_file() -> PathBuf {
    let manual_path = std::env::var_os("CODEX_ASR_AUTH_FILE").map(PathBuf::from);
    let codex_home = std::env::var_os("CODEX_HOME").map(PathBuf::from);
    let user_profile = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_auth_file(
        manual_path.as_deref(),
        codex_home.as_deref(),
        user_profile.as_deref(),
        home.as_deref(),
    )
}

fn resolve_auth_file(
    manual_path: Option<&Path>,
    codex_home: Option<&Path>,
    user_profile: Option<&Path>,
    home: Option<&Path>,
) -> PathBuf {
    manual_path
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .or_else(|| codex_home.map(|path| path.join("auth.json")))
        .or_else(|| user_profile.map(|path| path.join(".codex").join("auth.json")))
        .or_else(|| home.map(|path| path.join(".codex").join("auth.json")))
        .unwrap_or_else(|| PathBuf::from(".codex").join("auth.json"))
}

pub(crate) fn load_auth(path: &Path) -> Result<CodexAuth> {
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read Codex login file: {}", path.display()))?;
    let root: Value = serde_json::from_str(&body).context("Codex login file is not valid JSON")?;
    let tokens = root
        .get("tokens")
        .context("Codex login file has no tokens object")?;
    let access_token = tokens
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .context("Codex login file has no access token")?
        .to_string();
    // Desktop derives the account from the bearer token; a stale cached
    // account_id must not route a current token to a different workspace.
    let account_id = account_id_from_jwt(&access_token).or_else(|| {
        tokens
            .get("account_id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .map(str::to_string)
    });
    Ok(CodexAuth {
        access_token,
        account_id,
    })
}

fn account_id_from_jwt(access_token: &str) -> Option<String> {
    let payload = access_token.split('.').nth(1)?;
    let mut encoded = payload.replace('-', "+").replace('_', "/");
    encoded.push_str(&"=".repeat((4 - encoded.len() % 4) % 4));
    let bytes = base64_decode(&encoded)?;
    let payload: Value = serde_json::from_slice(&bytes).ok()?;
    payload
        .get("https://api.openai.com/auth")?
        .get("chatgpt_account_id")?
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string)
}

fn base64_decode(value: &str) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(value.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for byte in value.bytes() {
        let six = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(six);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(output)
}

fn encode_wav(audio: &[f32]) -> Result<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
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

fn http_error(status: u16) -> String {
    match status {
        401 => "Codex authentication rejected (HTTP 401); open Codex to renew the login".into(),
        403 => "Codex transcription access denied (HTTP 403); check account and workspace access"
            .into(),
        429 => "Codex transcription rate limit reached (HTTP 429); try again later".into(),
        code => format!("Codex transcription failed with HTTP {code}"),
    }
}

/// Match the installed desktop version and Electron platform names. If no
/// desktop installation is available, identify Handy Codex instead of inventing one.
pub(crate) fn desktop_user_agent() -> String {
    let version = installed_desktop_version();
    user_agent(
        version.as_deref(),
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

fn user_agent(version: Option<&str>, os: &str, arch: &str) -> String {
    let platform = match os {
        "macos" => "Mac OS",
        "windows" => "Windows NT 10.0",
        "linux" => "X11; Linux",
        other => other,
    };
    let arch = match arch {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    };
    match version {
        Some(version) => format!("Codex Desktop/{version} ({platform}; {arch})"),
        None => format!(
            "HandyCodex/{} ({platform}; {arch})",
            env!("CARGO_PKG_VERSION")
        ),
    }
}

fn valid_version(value: &str) -> bool {
    !value.is_empty()
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit()))
}

fn installed_desktop_version() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications"));
        }
        for root in roots {
            for app in ["ChatGPT.app", "Codex.app"] {
                let bundle = root.join(app).join("Contents");
                // ChatGPT Classic does not contain the Codex runtime.
                if !bundle.join("Resources/codex").is_file() {
                    continue;
                }
                if let Ok(info) = plist::Value::from_file(bundle.join("Info.plist")) {
                    if let Some(version) = info
                        .as_dictionary()
                        .and_then(|d| d.get("CFBundleShortVersionString"))
                        .and_then(plist::Value::as_string)
                    {
                        if valid_version(version) {
                            return Some(version.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

fn encode_multipart(audio: &[u8], language: &str, boundary: &str) -> Vec<u8> {
    let mut body = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"codex.webm\"\r\nContent-Type: audio/webm;codecs=opus\r\n\r\n").into_bytes();
    body.extend_from_slice(audio);
    body.extend_from_slice(b"\r\n");
    if language != "auto" && !language.trim().is_empty() {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\n{language}\r\n").as_bytes());
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

fn encode_webm(audio: &[f32]) -> Result<Vec<u8>> {
    // Desktop records WebM/Opus with Chromium MediaRecorder. FFmpeg gives us
    // the same media format, but is a different encoder/muxer implementation.
    // Keep temporary audio private and delete it on every exit path.
    let temp = tempfile::tempdir()?;
    let input = temp.path().join("recording.wav");
    let output = temp.path().join("recording.webm");
    std::fs::write(&input, encode_wav(audio)?)?;
    let ffmpeg = [
        "/opt/homebrew/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "/usr/bin/ffmpeg",
    ]
    .into_iter()
    .find(|path| Path::new(path).is_file())
    .unwrap_or("ffmpeg");
    let result = std::process::Command::new(ffmpeg)
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&input)
        .args([
            "-vn", "-c:a", "libopus", "-ar", "48000", "-ac", "1", "-b:a", "128k", "-f", "webm",
        ])
        .arg(&output)
        .stdin(std::process::Stdio::null())
        .output()
        .context("Codex WebM encoding requires FFmpeg with libopus installed")?;
    anyhow::ensure!(result.status.success(), "Codex WebM audio encoding failed");
    std::fs::read(output).context("Cannot read encoded Codex audio")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn jwt_account_id_fallback_reads_chatgpt_claim() {
        let header = "eyJhbGciOiJub25lIn0";
        let payload = json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "acct_test" }
        });
        let encoded = base64_json(&payload);
        assert_eq!(
            account_id_from_jwt(&format!("{header}.{encoded}.sig")),
            Some("acct_test".into())
        );
    }

    #[test]
    fn wav_encoder_writes_pcm_wave_header() {
        let wav = encode_wav(&[0.0, 1.0, -1.0]).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(
            u32::from_le_bytes(wav[24..28].try_into().unwrap()),
            SAMPLE_RATE
        );
        assert_eq!(u16::from_le_bytes(wav[34..36].try_into().unwrap()), 16);
    }

    #[test]
    fn empty_audio_encodes_to_header_only() {
        assert_eq!(encode_wav(&[]).unwrap().len(), 44);
    }

    #[test]
    fn auth_path_uses_windows_userprofile_when_home_is_missing() {
        assert_eq!(
            resolve_auth_file(None, None, Some(Path::new(r"C:\Users\Microck")), None,),
            PathBuf::from(r"C:\Users\Microck")
                .join(".codex")
                .join("auth.json")
        );
    }

    #[test]
    fn manual_auth_path_takes_precedence_over_environment_paths() {
        assert_eq!(
            resolve_auth_file(
                Some(Path::new(r"D:\Shared\auth.json")),
                Some(Path::new(r"C:\Users\Microck\.codex")),
                Some(Path::new(r"C:\Users\Microck")),
                Some(Path::new(r"/home/microck")),
            ),
            PathBuf::from(r"D:\Shared\auth.json")
        );
    }

    #[test]
    fn desktop_profile_matches_electron_platform_names() {
        assert_eq!(
            user_agent(Some("26.911.61220"), "macos", "aarch64"),
            "Codex Desktop/26.911.61220 (Mac OS; arm64)"
        );
        assert_eq!(
            user_agent(Some("26.911.61220"), "windows", "x86_64"),
            "Codex Desktop/26.911.61220 (Windows NT 10.0; x64)"
        );
        assert!(user_agent(None, "linux", "x86_64").starts_with("HandyCodex/"));
        assert!(!valid_version("26.1\r\nx-header: value"));
    }

    #[test]
    fn multipart_matches_desktop_framing() {
        let body = encode_multipart(b"audio", "ru", "boundary");
        assert_eq!(body, b"--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"codex.webm\"\r\nContent-Type: audio/webm;codecs=opus\r\n\r\naudio\r\n--boundary\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\nru\r\n--boundary--\r\n");
        let automatic = String::from_utf8(encode_multipart(b"audio", "auto", "boundary")).unwrap();
        assert!(!automatic.contains("name=\"language\""));
    }

    #[test]
    fn denied_access_is_not_reported_as_expired_login() {
        assert!(http_error(403).contains("access denied"));
        assert!(http_error(401).contains("authentication rejected"));
        assert!(http_error(429).contains("rate limit"));
    }

    #[test]
    fn current_token_account_wins_over_stale_cached_account() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let token = format!(
            "header.{}.sig",
            base64_json(&json!({"https://api.openai.com/auth": {"chatgpt_account_id": "current"}}))
        );
        std::fs::write(
            file.path(),
            json!({"tokens": {"access_token": token, "account_id": "stale"}}).to_string(),
        )
        .unwrap();
        assert_eq!(
            load_auth(file.path()).unwrap().account_id.as_deref(),
            Some("current")
        );
    }

    #[test]
    fn retry_requires_a_known_unchanged_account_and_new_token() {
        let auth = |token: &str, account: Option<&str>| CodexAuth {
            access_token: token.into(),
            account_id: account.map(str::to_string),
        };
        assert!(can_retry_with_auth(
            &auth("old", Some("a")),
            &auth("new", Some("a"))
        ));
        assert!(!can_retry_with_auth(&auth("old", None), &auth("new", None)));
        assert!(!can_retry_with_auth(
            &auth("old", Some("a")),
            &auth("new", Some("b"))
        ));
        assert!(!can_retry_with_auth(
            &auth("old", Some("a")),
            &auth("old", Some("a"))
        ));
    }

    fn base64_json(value: &Value) -> String {
        let bytes = serde_json::to_vec(value).unwrap();
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let a = chunk[0] as u32;
            let b = chunk.get(1).copied().unwrap_or(0) as u32;
            let c = chunk.get(2).copied().unwrap_or(0) as u32;
            out.push(alphabet[((a >> 2) & 63) as usize] as char);
            out.push(alphabet[(((a & 3) << 4) | (b >> 4)) as usize] as char);
            out.push(if chunk.len() > 1 {
                alphabet[(((b & 15) << 2) | (c >> 6)) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                alphabet[(c & 63) as usize] as char
            } else {
                '='
            });
        }
        out
    }
}
