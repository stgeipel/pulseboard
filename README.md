# Pulseboard

Pulseboard is a Windows desktop app for monitoring HTTP endpoints. Group checks by server, spot failures at a glance, and keep monitoring from the system tray. It runs locally and stores its configuration in TOML. The desktop UI is currently in German.

## Get started

1. Download `Pulseboard-<version>-Windows-x64-Setup.exe` from [GitHub Releases](https://github.com/stgeipel/pulseboard/releases) and run it. The installer supports Windows 10 22H2 and Windows 11 on x64.
2. Start Pulseboard and choose **Konfiguration bearbeiten** (Edit configuration).
3. Paste the example below, replace the sample URL with your own, and choose **Speichern und anwenden** (Save and apply, Ctrl+S).

The installer adds the Start menu shortcut that Windows uses for Pulseboard notifications. The release also includes a portable `pulseboard.exe` and SHA-256 checksums for both downloads. The executables are currently unsigned.

## Dashboard views

- **Serverliste** (Server list) is the default. The sidebar groups servers by status. Select a server to see its checks, HTTP methods, status codes, and response times on the right. Expand a check to see its path and result. You can copy a server or a check as TOML.
- **Matrix** shows one row per server and one column per check ID. Reuse the same check ID on different servers to compare equivalent endpoints. A dot marks a check that is absent from that server. An ellipsis marks a check waiting for its first result.
- **Kacheln** (Cards) shows a compact summary for each server. Select a card to open its details.

You can search for servers and show only failures with **Nur Fehler**. Use **Design** to switch color themes. Pulseboard saves your view and theme in `appearance.toml`.

## Configuration

Pulseboard creates `%APPDATA%\Pulseboard\config\config.toml` on first launch. The editor highlights TOML syntax and marks validation errors when their location is available. To apply changes made outside the app, choose **Neu laden** (Reload). If the new file is invalid, monitoring continues with the last valid configuration.

### Group checks by server

```toml
interval_seconds = 60
timeout_seconds = 10
close_to_tray = true
notifications = true

[[servers]]
id = "production"
name = "Production"
base_url = "https://api.example.com"

[[servers.checks]]
id = "live"
name = "API liveness"
path = "/health/live"
method = "GET"
expected_status = [200, 204]

[[servers.checks]]
id = "ready"
name = "API readiness"
path = "/health/ready"
```

A `path` starts with `/` and is resolved from the root of `base_url`. Use an absolute `url` for a check on another host. A group whose checks all use absolute URLs can omit `base_url`. Server IDs must be unique, and check IDs must be unique within each server.

To duplicate a configuration block, use **Server kopieren** or **Check kopieren**. Paste a copied server at the end of the file. Paste a copied grouped check at the end of its target group, before the next `[[servers]]`. These commands copy literal headers, passwords, and request bodies along with the rest of the block.

### Standalone checks and authentication

Use `[[checks]]` for endpoints that do not belong to a server group. They appear under **Einzelne Endpunkte** (Standalone endpoints).

```toml
[[checks]]
id = "readiness"
name = "Database readiness"
url = "https://api.example.com/readiness"
method = "POST"
expected_status = [200, 204]
timeout_seconds = 5
body = '{"component":"database"}'

[checks.headers]
Content-Type = "application/json"

[checks.headers_from_env]
Authorization = "PULSEBOARD_READINESS_AUTH"

[[checks]]
id = "legacy"
name = "Legacy API"
url = "https://legacy.example.com/health"

[checks.basic_auth]
username = "monitor"
password_env = "PULSEBOARD_LEGACY_PASSWORD"
```

- **Methods and status codes.** Pulseboard supports GET and POST. GET is the default. Any HTTP 2xx response counts as success if you omit `expected_status`. Use POST endpoints that are safe to call repeatedly.
- **Timing.** Checks run at launch, after reloading, and then every 60 seconds by default. The default timeout is 10 seconds and can be overridden per check. Response time measures the wait for response headers. Redirects are reported as their own HTTP status. History is kept for the current session.
- **Secrets.** Set `PULSEBOARD_READINESS_AUTH` to the complete header value, such as `Bearer YOUR_TOKEN`. Set `PULSEBOARD_LEGACY_PASSWORD` to the Basic Auth password. Restart Pulseboard after changing Windows user environment variables. Keep real credentials out of committed TOML files.

See [config.example.toml](config.example.toml) for the template created on first launch.

## Tray, notifications, and updates

- With `close_to_tray = true`, closing the window keeps monitoring in the tray. Click the tray icon to reopen the dashboard. The tray menu lets you check immediately or exit.
- With `notifications = true`, Pulseboard sends a Windows notification when it first detects an outage and when an endpoint's status changes. An ongoing outage generates one alert. If a banner is missing, check Windows notification settings and **Do not disturb**.
- Use **Autostart** to launch Pulseboard in the tray when you sign in to Windows.
- Official builds check GitHub for a newer stable release at startup. **Update verfügbar** (Update available) opens the release page. Download the installer there, exit Pulseboard from the tray, then run setup. Upgrades keep your local configuration and autostart setting.

## Troubleshooting

Choose **Absturzprotokoll** (Crash log) to open `%APPDATA%\Pulseboard\config\crash.log`. The log records startup, shutdown, and crash diagnostics. Pulseboard keeps the previous log as `crash.previous.log` when rotating the active log. For crashes that bypass the app's handler, check **Event Viewer → Windows Logs → Application** for `pulseboard.exe`.

## Build from source

On Windows, install Rust through rustup, the Visual Studio C++ Build Tools, and the Windows SDK. The repository pins Rust in [rust-toolchain.toml](rust-toolchain.toml). GPUI needs the SDK's `fxc.exe` for release builds. Set `GPUI_FXC_PATH` to its full path if GPUI cannot find it.

```powershell
cargo run --locked
cargo fmt --all --check
cargo test --no-default-features --locked
cargo build --release --locked
```

To add a color theme, define a palette in [src/appearance.rs](src/appearance.rs) and register it in `THEMES`. Use a unique, stable theme ID. An optional `glow` color adds a highlight around active controls.

### Publish a release

Update the package version in `Cargo.toml` and refresh `Cargo.lock` with `cargo check --no-default-features`. Commit both files. Once the change is on `main`, push a matching Git tag such as `v0.2.2`, or run **Windows build and release** on `main` with **Publish** enabled. The [release workflow](.github/workflows/windows.yml) builds the Windows x64 executable and installer and uploads both with their checksums. It sets `PULSEBOARD_REPOSITORY` to the GitHub repository used for update checks and release links.

## Report an issue

Open an issue on [GitHub](https://github.com/stgeipel/pulseboard/issues) with the Pulseboard version, Windows version, and steps to reproduce. If relevant, include a redacted configuration snippet or crash-log excerpt.
