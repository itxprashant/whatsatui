# AGENTS.md

Guidance for AI coding agents working in this repository.

## Project overview

`whatsatui` is a terminal UI (TUI) for WhatsApp, written in Rust with
[Ratatui](https://ratatui.rs). It talks to a running
[`go-whatsapp-web-multidevice`](https://github.com/aldinokemal/go-whatsapp-web-multidevice)
gateway over its REST API.

The repository contains two things:

1. The Rust TUI app (`src/`, `Cargo.toml`).
2. A `docker-compose.yml` that runs the WhatsApp gateway the TUI connects to.

This is intended to be a **public** repository. Do not commit secrets, session
data, or downloaded media (see `.gitignore`).

## Tech stack

- Rust (stable, edition 2021).
- `ratatui` + `crossterm` for the terminal UI.
- `tokio` async runtime (multi-thread).
- `reqwest` (rustls) for HTTP.
- `tiny_http` for the local webhook receiver; `hmac` + `sha2` + `hex` for
  signature verification.
- `serde` / `serde_json` for models, `chrono` for timestamps, `anyhow` for errors.
- `image` for decode/resize; `aes` + `cbc` + manual HKDF in `media.rs` for
  WhatsApp CDN decrypt.

## Layout

| Path                      | Responsibility                                                |
|---------------------------|---------------------------------------------------------------|
| `src/main.rs`             | Binary: terminal setup/teardown, event loop, input, task spawning |
| `src/lib.rs`              | Re-exports modules so the binary and `examples/` share code   |
| `src/api.rs`              | Async REST client (`ApiClient`), serde models, `Message::body_for_display` |
| `src/app.rs`              | `App` state, `AppEvent`, focus, selection, name/display logic   |
| `src/ui.rs`               | Ratatui rendering (header, chats, bubbles, compose, footer)   |
| `src/theme.rs`            | Central color palette and `Style` helpers                     |
| `src/config.rs`           | Env-based configuration with deployment defaults              |
| `src/archive.rs`          | Archived jids from gateway `whatsmeow_chat_settings`          |
| `src/media.rs`            | WhatsApp media download/decrypt (chatstorage keys)            |
| `src/termimg.rs`          | Braille terminal image rendering + inline thumbnail helpers   |
| `src/cache.rs`            | On-disk cache for group subjects (`groups.json`)              |
| `src/webhook.rs`          | Local `tiny_http` receiver + `verify_signature` (HMAC-SHA256) |
| `examples/probe.rs`       | Read-only API connectivity probe (no messages sent)           |
| `examples/webhook_check.rs` | Offline webhook receiver + signature check (no messages sent) |

## Architecture notes

### Event loop and concurrency

- The UI never blocks on network I/O. Input is read on a dedicated OS thread;
  network calls run in spawned `tokio` tasks. Both communicate with the main
  loop via a `tokio::mpsc` channel of `AppEvent` values (`src/app.rs`).
- Live updates use webhooks, not the WebSocket. The gateway's `/ws` only
  broadcasts device/login status; messages are delivered as signed HTTP POSTs.
  `src/webhook.rs` runs a `tiny_http` server on its own thread (mirroring the
  input thread), verifies the `X-Hub-Signature-256` HMAC against
  `WHATSATUI_WEBHOOK_SECRET` in constant time, and forwards `AppEvent::Webhook`.
  The main loop refetches the open chat and (debounced) the chat list.

### Stale-response guards (preserve when touching fetch flows)

- **Messages**: fetches are tagged with `App::next_messages_generation`; the
  main loop drops responses where `generation < active_messages_generation`.
- **Chats**: same pattern via `next_chats_generation` / `active_chats_generation`.
  `AppEvent::Chats` also carries `show_archived`; `set_chats` ignores responses
  whose archived mode does not match `App::show_archived`. This prevents a slow
  archived fetch from wiping the active list after the user toggles back.

### Sending and messages

- Sending uses optimistic updates: the outgoing message is appended locally
  (with a `pending:` id, see `Message::is_pending`) and reconciled when the
  gateway returns the real message. On failure it is removed via `SendFailed`.
- `get_messages(jid, limit, offset)` returns `MessagesPage` with pagination
  metadata. Initial load uses `MESSAGES_PAGE` (50) at `offset=0`; older pages
  are prepended via `AppEvent::OlderMessages` when the user scrolls up.
- Media messages use `Message::body_for_display()` / `preview_text()` for labels
  (`[image]`, `[video]`, `[file: …]`, captions). Keep display logic in `api.rs`;
  `ui.rs` calls these helpers.

### Images and media

- The gateway's `GET /message/:id/download` is unreliable for this deployment;
  images are fetched from the WhatsApp CDN and decrypted locally using keys from
  `chatstorage.db` (`src/media.rs`, whatsmeow-compatible HKDF + AES-CBC).
- **Inline previews**: after messages load, `maybe_spawn_thumbnails` in
  `main.rs` fetches up to 8 thumbnails (most recent images first). Decoded
  images are downscaled via `termimg::thumbnail_image` and cached in
  `App::message_thumbnails`. `ui.rs` renders them inside bubbles at
  `INLINE_PREVIEW_COLS` × `INLINE_PREVIEW_ROWS` (22×5 cells) using braille
  pixels. Captions show below the preview; `[image]` is a fallback only when
  no preview is cached yet.
- **Full-screen viewer** (`v` on a picked image): `spawn_image_view` downloads
  the full image, `Focus::ImageViewer` overlay via `draw_image_overlay`.
  `[/]` steps `App::media_pick` among viewable images in the open chat.
- Rendering uses Unicode braille (2×4 px per cell), not half-blocks — see
  `termimg::render_image`. Preserve sharpness when changing the renderer.

### Chat list

- Archived state comes from the gateway's **whatsmeow** sqlite DB
  (`whatsmeow_chat_settings.archived`), not `GET /chats?archived=true` (that
  reads `chatstorage.db` and is often wrong). `src/archive.rs` loads archived
  jids; `spawn_chats` filters paginated API results client-side.
- Active mode: first page (`limit=100`) minus archived jids. Archived mode:
  `list_all_chats()` then keep only archived jids.
- `App::set_chats` preserves selection by jid across reorders.
- Last-message previews are fetched lazily (`spawn_chat_previews`) for the first
  visible chats and on webhook events; stored in `App::chat_previews`.
- Search (`Focus::Search`, `/` key) filters the current list client-side via
  `App::chat_filter` and `chat_visible()`.

### Contact and group names

- Contact names: `App::contacts` (jid -> name), loaded via `list_contacts()`.
- Group subjects: `App::groups`, loaded via `list_groups()` (`GET /user/my/groups`).
  That endpoint is large (~10 MB), so subjects are persisted in
  `~/.cache/whatsatui/groups.json` (override with `WHATSATUI_CACHE_DIR`) and
  loaded synchronously in `App::new()` for instant display; the async fetch
  refreshes and rewrites the cache via `src/cache.rs`.
- The chat list often returns placeholder `Group <id>` names for `@g.us` jids.
  Resolution order — groups: group subject -> non-placeholder chat name -> bare
  jid; individuals: contact name -> chat name -> bare jid (`App::chat_display_name`
  / `jid_display_name`). Do name resolution in `app.rs`; `ui.rs` only calls
  these helpers.

### UI

- All colors come from `src/theme.rs`. Do not hardcode `Color::*` in `ui.rs`;
  add or reuse a palette entry instead.

### Keybindings (preserve when changing input handling)

| Context | Keys | Action |
|---------|------|--------|
| Chats | `j`/`k`, `Enter`, `/`, `a`, `r`, `q` | Navigate, open, search, archived, refresh, quit |
| Open chat | `PgUp`/`Home`, `PgDn`/`End` | Scroll / load older messages |
| Open chat | `[`/`]`, `v` | Pick image, view full-screen |
| Search | type, `Esc` | Filter, exit |
| Compose | `Enter`, `Esc`/`Tab` | Send, back to chats |
| Image viewer | `Esc`/`v`/`q` | Close viewer |
| Global | `Ctrl-C` | Quit |

## Configuration

Read from environment variables (defaults match the bundled docker-compose):

| Variable                   | Default                  |
|----------------------------|--------------------------|
| `WHATSATUI_BASE_URL`       | `http://localhost:56310` |
| `WHATSATUI_USER`           | `admin`                  |
| `WHATSATUI_PASS`           | `changeme`               |
| `WHATSATUI_DEVICE_ID`      | _(unset; single device used automatically)_ |
| `WHATSATUI_WEBHOOK_ADDR`   | `127.0.0.1:56311`        |
| `WHATSATUI_WEBHOOK_PATH`   | `/webhook`               |
| `WHATSATUI_WEBHOOK_SECRET` | `secret` (must equal the gateway's `WHATSAPP_WEBHOOK_SECRET`) |
| `WHATSATUI_WHATSMEOW_DB`   | `storages/whatsapp.db`   |
| `WHATSATUI_CHATSTORAGE_DB` | `storages/chatstorage.db` |
| `WHATSATUI_CACHE_DIR`      | `~/.cache/whatsatui`     |

For live push the gateway must be pointed at the receiver: set
`WHATSAPP_WEBHOOK=http://localhost:56311/webhook` in `.env` and recreate the
container (`docker compose up -d --force-recreate`; preserves volumes).

## Public repository hygiene

**Never commit** (already in `.gitignore`):

- `.env` — gateway credentials, webhook secret, and other local overrides.
- `storages/` — whatsmeow/chatstorage sqlite DBs (linked WhatsApp session).
- `statics/` — gateway media files and static assets tied to the session.
- `/target/` — Rust build artifacts.

Safe to commit: source, `Cargo.toml`, `Cargo.lock`, `docker-compose.yml`,
`README.md`, `AGENTS.md`, and `examples/`.

If adding sample env documentation, use a committed `.env.example` with
placeholder values only — never copy a real `.env`.

## Build, run, verify

```bash
cargo build              # debug build
cargo test --lib         # unit tests (webhook verifier, cache, display logic)
cargo run                # run the TUI (debug; preferred during development)
cargo build --release    # optimized build (thin LTO)
cargo run --example probe         # read-only API check (no messages sent)
cargo run --example webhook_check # offline webhook path check (no messages sent)
```

- The interactive TUI requires a real terminal (TTY). It cannot be exercised
  from a non-interactive shell; use `examples/probe.rs` to verify the API path
  instead.
- `release` uses `lto = "thin"`. Do NOT switch to `lto = true` (full LTO):
  it makes the final link step appear hung for minutes.

## Conventions

- Keep modules small and focused per the table above. Rendering logic stays in
  `ui.rs`; state lives in `app.rs`; HTTP lives in `api.rs`.
- After edits, run `cargo build` and fix all warnings (the tree should build
  clean).
- Comments should explain intent/constraints, not narrate the code.

## Critical rules

- This project connects to a live WhatsApp account. NEVER send messages as a
  test. Verify changes with `cargo build` and the read-only `probe` / `webhook_check`
  examples.
- Do not delete or wipe gateway data (`storages/`, `statics/`) or any database.
