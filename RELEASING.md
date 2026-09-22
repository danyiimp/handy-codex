# Building Handy releases from handy-codex

The app display name is Handy and the macOS bundle is `Handy.app`. The fork's
repository and release destination remain `danyiimp/handy-codex`; its bundle ID
and data directory remain `io.handycodex.desktop`.

`main` is the default branch and release source. Run the Release workflow from
`main` after verification; the workflow rejects other branches. Use temporary
`fix/`, `feat/`, or `chore/` branches when needed and delete them after integration.

The initial release targets macOS Apple Silicon. Source builds for other platforms
inherit Handy's requirements in [BUILD.md](BUILD.md), but are not validated by the
initial release. GitHub's cross-platform build workflow remains manually available;
the release workflow defaults to macOS arm64 and creates a draft release.

## macOS build

Install Rust, Bun, Command Line Tools (or full Xcode), and the native prerequisites
listed in BUILD.md. Ensure `src-tauri/resources/models/silero_vad_v4.onnx` exists;
BUILD.md documents its upstream download URL. FFmpeg with libopus is an external
runtime dependency for Codex ASR, not bundled with the application.

```sh
bun install --frozen-lockfile
bunx tsc --noEmit
bunx vite build
cargo build --manifest-path src-tauri/Cargo.toml --locked --release --bin handy \
  --features tauri/custom-protocol --jobs 2 \
  --config 'profile.release.lto=false' \
  --config 'profile.release.codegen-units=16'
bunx tauri bundle --bundles app --ci
```

The `tauri/custom-protocol` feature is mandatory for a direct Cargo desktop build;
a binary without it can pass headless tests yet display an empty window because it
loads the development URL. The full `bun run tauri build` command supplies it.

Command Line Tools builds use the upstream Apple Intelligence stubs. Explicitly
state this limitation in release notes when applicable. These instructions produce
an ad-hoc signed app; they do not provide Apple notarization.

## Verification and packaging

1. Run the transport regression suite with
   `cargo test --manifest-path tests/transport/Cargo.toml --locked --jobs 2`.
2. Run TypeScript, lint, translation consistency, formatting, and the existing
   updater tests. Check that source and update links point to this repository.
3. Check the packaged app's name, version, bundle ID, architecture, and signature.
   Open the **packaged GUI** and verify settings render. Test the packaged binary's
   `--transcribe-file <16kHz-mono-PCM16.wav> --model codex-chatgpt-asr --json` path
   with an authorized local login. Do not place that auth or audio in the repo.
4. Include LICENSE, NOTICE, and a build-info JSON with source commit, target,
   compiler versions, build features, and known limitations. After modifying bundle
   contents, sign again with the configured entitlements and verify the signature.
5. Create a ZIP with macOS `ditto -c -k --keepParent`, generate SHA256SUMS, and test
   extraction. Upload only the app archive and checksums. Review both for accidental
   credentials, settings, user recordings, source extraction, and local test artifacts.
6. Publish against the verified source commit. Keep release notes explicit about
   FFmpeg, ad-hoc signed, non-notarized distribution, Apple Intelligence, cloud privacy,
   and the unsupported internal API. Do not claim immunity to account restrictions.

The transport test crate imports the actual production modules. Its settings stub
contains only the provider fields consumed by `llm_client`; no HTTP code is copied.
CI runs these tests without credentials or live service requests.
