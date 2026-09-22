# Codex transcription compatibility

The Codex integration in this Handy fork is based on Microck/handy-codex v0.9.10, commit
448140b500af7d3714ae73ed152e8ba7fb419624. The reference inspected on 2026-09-22
was the installed ChatGPT/Codex desktop 26.911.61220, build 9647, on macOS arm64.
The installed desktop application was read, not modified.

## Request construction

- The batch route remains `https://chatgpt.com/backend-api/transcribe`.
- Audio is encoded as WebM/Opus using FFmpeg. Temporary audio is inside a private
  directory that is deleted after conversion. FFmpeg is a runtime dependency;
  a missing encoder produces an explicit error rather than silently changing
  the audio format.
- Multipart uses `----codex-transcribe-<UUID>`, `codex.webm`,
  `audio/webm;codecs=opus`, and an optional `language` field. Its serialization
  was compared byte-for-byte against the installed desktop's batch serializer
  with synthetic input and a fixed boundary.
- The `originator` is `Codex Desktop`. The user-agent uses the installed desktop
  version and the same platform/architecture names as its desktop header helper.
  Without an identifiable desktop install it identifies Handy Codex. This verifies
  the helper's value; it is not a capture of the native app's final network
  headers. The batch route passes through other native network layers.
- Authorization is read from the selected Codex auth file on each request.
  The account claim in the current bearer token takes precedence over a stale
  cached account ID. Credentials are not copied to another store.
- On HTTP 401, the file is re-read once. Only a changed token for the same
  account is retried. Handy Codex does not rotate Codex's shared refresh token.

## Corrections included

- HTTP 403 is described as access denied, separately from HTTP 401.
- Invalid response bodies are not included in ASR diagnostics; this also
  removes the former Unicode byte-slicing panic in the error formatter.
- Transcript content is no longer written by the normal transcription INFO
  log or the action DEBUG log. User-selected history storage still applies.
- The selected auth file is also used by non-structured Codex post-processing.
- Codex post-processing cannot send the Codex bearer to an arbitrary configured
  server. Its base URL is restricted to the canonical ChatGPT Codex URL.
- Codex HTTP clients refuse redirects and have connection/request timeouts.
- Post-processing requires a successful terminal SSE event and rejects failed,
  incomplete, malformed, or truncated streams instead of returning partial text.

## Verification and limits

For a direct Cargo desktop build, compile the frontend first and include
`--features tauri/custom-protocol`. Without that feature, a release binary can
pass headless transcription tests but try to load the development URL and show
an empty GUI. The Tauri build command normally supplies this feature. The local
release build used two Cargo jobs, LTO disabled, and sixteen codegen units to
bound compiler resource use. Both headless transcription and the packaged GUI
must be checked before replacing the installed app.

Production ASR and LLM modules are imported into the [transport test crate](tests/transport/Cargo.toml). It exercises multipart framing, auth selection, account claims, status
classification, URL restrictions, and SSE completion/error cases. Live tests
with synthetic Russian and English audio exercise the changed production ASR
module using the owner's existing login. Credentials are never printed.

This is protocol compatibility, not a claim of indistinguishability or vendor
approval. Handy Codex still uses reqwest and FFmpeg, while desktop uses Electron's
network stack and Chromium's MediaRecorder. Audio preprocessing, encoder/muxer
metadata, TLS behavior, cookies, optional streaming, and token-refresh ownership
can differ. The desktop can attach server-issued integrity state; this patch
neither copies nor fabricates that state or device attestation.

Existing cancellation behavior also remains: cancelling the UI suppresses the
output but does not abort an already-running blocking ASR request; a recording
saved before cancellation can remain outside the history database. Old logs
and existing recordings are not deleted by this patch.

The local macOS build uses Command Line Tools rather than full Xcode, so the
upstream build script compiles its Apple Intelligence stubs. Apple Intelligence
post-processing is unavailable in this local binary. FFmpeg/libopus was available and exercised during local verification.

The public supported transcription interface is documented at
[OpenAI file transcription](https://developers.openai.com/api/docs/guides/speech-to-text).
No public third-party contract for `/backend-api/transcribe` was found in the
reviewed documentation. Successful requests establish current functionality,
not future endpoint availability or freedom from account restrictions.
