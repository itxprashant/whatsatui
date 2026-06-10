# whatsatui

A terminal UI for WhatsApp, built in Rust with [Ratatui](https://ratatui.rs).
It talks to a running [`go-whatsapp-web-multidevice`](https://github.com/aldinokemal/go-whatsapp-web-multidevice)
gateway over its REST API.

```
┌ whatsatui · <device> ─────────────────────────── ● online ┐
│ Chats            │  <chat name>                            │
│ ▎ A  Alice   2m  │   incoming bubble                       │
│   B  Bob     1h  │                          outgoing ✓     │
│   C  Carol   Mon │   [inline image preview]                │
│                  ├─ Message ──────────────────────────────┤
│                  │ Type a message…                        │
└──────────────────┴─────────────────────────────────────────┘
 j/k move   enter open   i compose   r refresh   q quit
```

## Prerequisites

- **Rust** (stable) and Cargo — [rustup.rs](https://rustup.rs)
- **Docker** and **Docker Compose** — to run the bundled WhatsApp gateway
- A terminal (TTY) for the interactive TUI

The bundled `docker-compose.yml` uses `network_mode: host`, so the gateway
listens directly on port **56310** on the host. This setup is straightforward
on Linux; on macOS/Windows Docker Desktop, host networking may differ — run
the gateway on Linux or point whatsatui at a remote gateway instead.

## Complete setup

### 1. Clone the repository

```bash
git clone <repository-url>
cd whatsatui
```

### 2. Create the gateway `.env` file

The gateway reads its configuration from `.env` (gitignored — never commit it).
Create `.env` in the repo root with at least:

```dotenv
# Application Settings
APP_PORT=56310
APP_HOST=0.0.0.0
APP_DEBUG=false
APP_OS=Chrome
APP_BASIC_AUTH=admin:changeme
APP_BASE_PATH=
APP_TRUSTED_PROXIES=0.0.0.0/0

# Database Settings
DB_URI=file:storages/whatsapp.db?_foreign_keys=on
DB_KEYS_URI=

# WhatsApp Settings
WHATSAPP_AUTO_MARK_READ=false
WHATSAPP_AUTO_REJECT_CALL=false
WHATSAPP_AUTO_DOWNLOAD_MEDIA=true
WHATSAPP_CHAT_STORAGE=true

# Live updates → whatsatui's local webhook receiver
WHATSAPP_WEBHOOK=http://localhost:56311/webhook
WHATSAPP_WEBHOOK_SECRET=secret
WHATSAPP_WEBHOOK_EVENTS=message,message.reaction,message.revoked,message.edited,message.ack,message.deleted,group.participants
```

`WHATSAPP_CHAT_STORAGE=true` is required for image decryption keys.
`WHATSAPP_WEBHOOK_SECRET` must match whatsatui's `WHATSATUI_WEBHOOK_SECRET`
(default `secret`).

Change `APP_BASIC_AUTH` (and matching `WHATSATUI_USER` / `WHATSATUI_PASS`) if
the gateway is reachable from other machines.

### 3. Start the gateway container

```bash
docker compose up -d
```

This pulls `aldinokemal2104/go-whatsapp-web-multidevice:latest`, creates
`storages/` and `statics/` on the host (session data and media), and starts
the container as `go-whatsapp-web-multidevice`.

Check that it is running:

```bash
docker compose ps
docker compose logs -f whatsapp_go   # Ctrl-C to stop following
```

Stop or restart later:

```bash
docker compose stop
docker compose start
docker compose down                  # stops container; keeps volumes
```

After changing `.env` (especially webhook settings), recreate the container
without wiping data:

```bash
docker compose up -d --force-recreate
```

### 4. Link your WhatsApp account

1. Open **http://localhost:56310** in a browser.
2. Sign in with basic auth (`admin` / `changeme`, or whatever you set in
   `APP_BASIC_AUTH`).
3. Scan the QR code with WhatsApp on your phone:
   **Settings → Linked devices → Link a device**.

Wait until the gateway shows the device as connected. Session state persists
in `./storages/` across container restarts.

### 5. Verify the API (read-only)

From the repo root, with the gateway still running:

```bash
cargo run --example probe
```

This performs GET requests only (no messages sent). You should see device,
contact, chat, and message counts. Optional: probe a specific chat:

```bash
WHATSATUI_PROBE_JID=1234567890@s.whatsapp.net cargo run --example probe
```

### 6. Build and run the TUI

```bash
cargo run              # debug build (good for development)
cargo run --release    # optimized build
```

whatsatui defaults match the Docker deployment above (`http://localhost:56310`,
`admin` / `changeme`). On startup it:

- binds a local webhook receiver on `127.0.0.1:56311/webhook` for live updates;
- reads archived-chat state from `storages/whatsapp.db`;
- reads media decryption keys from `storages/chatstorage.db`.

If the webhook port is busy, the UI still works — press `r` to refresh manually.

## Configuration

whatsatui reads **environment variables** (defaults match the bundled gateway):

| Variable                   | Default                  | Description                                   |
|----------------------------|--------------------------|-----------------------------------------------|
| `WHATSATUI_BASE_URL`       | `http://localhost:56310` | Gateway base URL                              |
| `WHATSATUI_USER`           | `admin`                  | Basic-auth username (`APP_BASIC_AUTH` user)   |
| `WHATSATUI_PASS`           | `changeme`               | Basic-auth password                           |
| `WHATSATUI_DEVICE_ID`      | _(unset)_                | Optional device JID to scope to               |
| `WHATSATUI_WEBHOOK_ADDR`   | `127.0.0.1:56311`        | Local webhook receiver bind address           |
| `WHATSATUI_WEBHOOK_PATH`   | `/webhook`               | Path the gateway POSTs live events to         |
| `WHATSATUI_WEBHOOK_SECRET` | `secret`                 | Must equal `WHATSAPP_WEBHOOK_SECRET`          |
| `WHATSATUI_WHATSMEOW_DB`   | `storages/whatsapp.db`   | Gateway whatsmeow DB (archived chats)         |
| `WHATSATUI_CHATSTORAGE_DB` | `storages/chatstorage.db`| Gateway chatstorage DB (media keys)           |
| `WHATSATUI_CACHE_DIR`      | `~/.cache/whatsatui`     | On-disk cache for group subjects              |

If only one device is linked on the gateway, `WHATSATUI_DEVICE_ID` can stay unset.

Example with explicit settings:

```bash
WHATSATUI_BASE_URL=http://localhost:56310 \
WHATSATUI_USER=admin \
WHATSATUI_PASS=changeme \
cargo run
```

### Live updates (webhook)

The gateway's `/ws` endpoint only carries device/login status — not messages.
whatsatui receives message events via the local HTTP webhook receiver. Each POST
is verified with `X-Hub-Signature-256` (HMAC-SHA256) against the shared secret.

If live push stops working after you change webhook settings, run:

```bash
docker compose up -d --force-recreate
```

Optional offline check of the webhook verifier (no gateway required):

```bash
cargo run --example webhook_check
```

## Keybindings

### Chat list (default focus)

| Key            | Action                          |
|----------------|---------------------------------|
| `j` / `↓`      | Move selection down             |
| `k` / `↑`      | Move selection up               |
| `Enter`        | Open the selected chat          |
| `/`            | Search/filter chats             |
| `a`            | Switch active ↔ archived chat list |
| `PgUp` / `Home`| Scroll up / load older messages |
| `PgDn` / `End` | Scroll down / jump to latest    |
| `[` / `]`      | Previous / next image in chat   |
| `v`            | View selected image full-screen |
| `i` / `Tab`    | Focus the compose box           |
| `r`            | Refresh chats and messages      |
| `q`            | Quit                            |
| `Ctrl-C`       | Quit (from anywhere)            |

### Search mode (`/`)

| Key       | Action                       |
|-----------|------------------------------|
| _typing_  | Filter chats by name or jid  |
| `j` / `k` | Move selection             |
| `Enter`   | Open selected chat         |
| `Esc`     | Clear filter and exit      |

### Image viewer (`v`)

| Key       | Action             |
|-----------|--------------------|
| `Esc` / `v` / `q` | Close viewer |

### Compose box

| Key       | Action                       |
|-----------|------------------------------|
| _typing_  | Edit the message             |
| `Enter`   | Send the message             |
| `Esc` / `Tab` | Return focus to chat list |

## Features

- Chat list with avatar-style badges, relative timestamps, and last-message previews.
- Search/filter chats (`/`), toggle archived (`a`), scroll history (`PgUp`/`PgDn`) and load older messages.
- Media labels (`[image]`, `[video]`, `[file: …]`, captions) with **inline image thumbnails** in bubbles.
- Full-screen in-terminal image viewer (`v`) using braille rendering; images decrypted locally from gateway media keys.
- Contact and group names resolved from the gateway (address book and
  `/user/my/groups`; chat-list `Group <id>` placeholders are ignored).
- Conversation view with chat-bubble styling (outgoing right / incoming left).
- Send text messages to the open chat.
- Live updates: incoming messages refresh the open conversation and reorder the
  chat list via the local webhook receiver (selection preserved).
- Non-blocking async networking: the UI stays responsive while data loads,
  with a loading spinner in the header.

## Project layout

| Path              | Responsibility                                        |
|-------------------|-------------------------------------------------------|
| `src/main.rs`     | Terminal setup, tokio runtime, input + event loop     |
| `src/api.rs`      | Async REST client and response models                 |
| `src/app.rs`      | Application state and events                          |
| `src/ui.rs`       | Ratatui rendering (header, chats, messages, compose)|
| `src/theme.rs`    | Central color palette and style helpers               |
| `src/config.rs`   | Environment-based configuration                       |
| `src/archive.rs`  | Archived chats from whatsmeow DB                      |
| `src/media.rs`    | WhatsApp CDN download + decrypt                       |
| `src/termimg.rs`  | Braille terminal image rendering                      |
| `src/cache.rs`    | On-disk group-name cache                              |
| `src/webhook.rs`  | Local HTTP receiver + HMAC-SHA256 verifier            |
| `docker-compose.yml` | Bundled `go-whatsapp-web-multidevice` gateway      |
| `examples/probe.rs` | Read-only API connectivity check                    |

## Troubleshooting

| Problem | Things to check |
|---------|-----------------|
| `devices parsed: 0` or probe errors | Gateway running? `docker compose ps`. Linked WhatsApp at http://localhost:56310? |
| 401 / auth errors | `WHATSATUI_USER`/`PASS` match `APP_BASIC_AUTH` in `.env`. |
| No live updates | `WHATSAPP_WEBHOOK` points to `http://localhost:56311/webhook`; secrets match; recreate container. Port 56311 free? |
| Archived list wrong | `WHATSATUI_WHATSMEOW_DB` points at `./storages/whatsapp.db` (not chatstorage). |
| Images don't load | `WHATSAPP_CHAT_STORAGE=true` in `.env`; `storages/chatstorage.db` exists after linking. |
| TUI won't start in CI | Needs a real TTY; use `cargo run --example probe` instead. |

**Do not delete** `storages/` or `statics/` unless you intend to wipe the linked
WhatsApp session and downloaded media.

## License

MIT
