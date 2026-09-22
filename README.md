# Handy Codex

Desktop dictation with global shortcuts, transcription history, local speech models,
and optional ChatGPT / Codex transcription.

This is an independent fork of [Handy](https://github.com/cjpais/Handy), with the
Codex integration inherited from [Handy Codex](https://github.com/Microck/handy-codex)
v0.9.10 and additional transport, credential-handling, and error-handling fixes.
The source also incorporates Handy main through `8f9cf53` (2026-09-19), including
recording fixes, shortcut activation modes, and the updated local inference engine.
It is not affiliated with or endorsed by Handy, OpenAI, or the Handy Codex maintainer.

## Download

Get the macOS Apple Silicon build from
[Releases](https://github.com/danyiimp/handy-codex/releases).
The initial release is experimental. Other platforms retain upstream source support
but do not have verified binaries in this release.

1. Install **FFmpeg with libopus** for Codex transcription. On a Homebrew installation:
   `brew install ffmpeg`. Local speech models do not require this extra encoder.
2. Extract `Handy Codex.app` from the release ZIP and move it to Applications.
3. Open the app and grant microphone and Accessibility permissions when requested.
   This release is ad-hoc signed, not Apple-notarized; macOS may require the usual
   **Privacy & Security → Open Anyway** approval.
4. For cloud transcription, sign in using Codex, then select **Codex / ChatGPT** in
   Models. The app reads your existing `~/.codex/auth.json`; a custom file can be
   selected in settings. No credentials are included in the download.
5. Configure the keyboard shortcut and record a short test. Stop other dictation
   apps using the same shortcut first.

## What changed

- WebM/Opus uploads and multipart framing aligned with the inspected desktop batch
  transcription path.
- Desktop version discovery instead of a hardcoded Windows user-agent.
- Auth is re-read for each request, with account checks on the one permitted retry
  after a changed token. Codex retains ownership of token refresh.
- Redirects are rejected, requests have timeouts, and Codex post-processing accepts
  only its canonical service URL.
- Failed, incomplete, or truncated post-processing streams return an error.
- Normal transcription logs omit full transcript text. History retention remains
  controlled by the app settings.
- Independent app branding, data directory, and release links.

Read [CODEX_TRANSPORT.md](CODEX_TRANSPORT.md) for the exact scope and verification.

## Troubleshooting

For inherited device, shortcut, clipboard, and platform behavior, see the
[upstream troubleshooting notes](https://github.com/cjpais/Handy/blob/8f9cf53/README.md#troubleshooting).
Those notes use upstream app paths; this fork uses the separate paths below.
Existing hold/toggle settings migrate to the corresponding shortcut mode; new
installations default to tap-or-hold behavior.

## Privacy and service limitations

Local models process audio locally. **Codex mode sends audio to OpenAI**, using an
existing Codex login and the internal `/backend-api/transcribe` endpoint. This is
not the documented public transcription API, and it can change or become unavailable.
A successful request does not establish permission for third-party use or guarantee
that an account will never face restrictions.

The app does **not** promise indistinguishability from Codex Desktop. Its HTTP/TLS
stack, encoder metadata, audio preprocessing, and refresh behavior differ. It does
not copy or fabricate server integrity state, cookies, or device attestation.

The initial local macOS binary has **Apple Intelligence post-processing disabled**
because it was built with Command Line Tools. Codex transcription remains available.
Cancelling suppresses output but does not abort an already running blocking cloud
request; a cancelled recording may remain outside the history database.

## Existing Handy settings

Handy Codex uses bundle ID `io.handycodex.desktop` and an independent data folder.
It does not overwrite Handy or automatically import private data.

To migrate on macOS, quit both apps and back up your data first. Before the first
Handy Codex launch, copy
`~/Library/Application Support/com.pais.handy/` to
`~/Library/Application Support/io.handycodex.desktop/` using Finder. If the target
already exists, move it to a backup location rather than merging or overwriting it.
This copies models, settings, history, and recordings, including any saved provider
credentials. Keep the copy local. Permissions must be granted separately for the new app.

## Development and verification

See [BUILD.md](BUILD.md) for inherited platform prerequisites and
[RELEASING.md](RELEASING.md) for this fork's build and packaging procedure.
Transport regression tests can run without the native audio engines:

```sh
cargo test --manifest-path tests/transport/Cargo.toml --locked --jobs 2
bun install --frozen-lockfile
bunx tsc --noEmit
bun test src/components/update-checker/portableInstaller.test.ts
```

Report fork-specific problems in
[this repository](https://github.com/danyiimp/handy-codex/issues).
Do not include auth files, access tokens, private recordings, or transcripts in reports.

## License and attribution

Source code is distributed under the [MIT license](LICENSE). Original copyright
notices are retained. Handy's name, logo, and brand assets are separate from its
source-code license. This unofficial fork retains the blue hand icon inherited
from Microck/handy-codex, adds fork artwork, and preserves Handy attribution;
no endorsement is implied. See [NOTICE](NOTICE).
