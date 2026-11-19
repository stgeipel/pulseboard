# Pulseboard

Pulseboard ist ein lokaler Healthcheck-Monitor für Windows mit einer dunklen, nativen Oberfläche aus Rust und GPUI. Er zeigt den HTTP-Status, die Antwortzeit und die letzten zwölf Ergebnisse pro Endpunkt.

## Windows installieren und aktualisieren

1. Lade unter **Releases** die passende `Pulseboard-…-Windows-x64-Setup.exe` herunter und installiere sie für deinen Windows-Benutzer. Der Installer richtet den Startmenü-Eintrag für Windows-Benachrichtigungen ein.
2. Öffne Pulseboard und klicke auf **Konfiguration öffnen**. Trage deine Endpunkte ein, speichere die Datei und klicke auf **Neu laden**.
3. Für ein Update öffnest du **Updates**, beendest Pulseboard über das Tray-Menü und führst den neueren Installer aus. Der Installationspfad bleibt gleich. Konfiguration und Autostart-Einstellung bleiben bei einem Upgrade erhalten.

Der Installer ist für Windows 10 ab 22H2 und Windows 11 (x64) konfiguriert. Die Installationsdatei ist unsigniert. Der Release-Download enthält zusätzlich eine SHA-256-Datei.

## Konfiguration

Pulseboard legt beim ersten Start eine Vorlage unter `%APPDATA%\Pulseboard\config\config.toml` an. Endpunkte werden erst nach dem Eintragen aktiv. Die App prüft sie beim Start, nach dem Neuladen und anschließend standardmäßig alle 60 Sekunden.

```toml
interval_seconds = 60
timeout_seconds = 10
close_to_tray = true
notifications = true

[[checks]]
id = "api"
name = "Backend API"
url = "https://api.example.com/health"
method = "GET"

[[checks]]
id = "readiness"
name = "Readiness"
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
url = "https://api.example.com/health"
[checks.basic_auth]
username = "monitor"
password_env = "PULSEBOARD_LEGACY_PASSWORD"
```

- Jeder Check braucht eine stabile, eindeutige `id`. Ohne `method` sendet Pulseboard einen GET-Request. Ohne `expected_status` zählt jeder 2xx-Status als Erfolg.
- Pulseboard bewertet den HTTP-Status. Die Latenz endet beim Empfang der Antwort-Header. Weiterleitungen werden als eigener HTTP-Status angezeigt und Zugangsdaten bleiben am konfigurierten Endpunkt.
- `headers` akzeptiert direkte Werte. Bei `headers_from_env` enthält der TOML-Wert den Variablennamen und die Windows-Benutzervariable den vollständigen Headerwert, etwa `Bearer …`. Für Basic Auth stehen `password_env` und alternativ `password` zur Verfügung.
- Für reale Zugangsdaten sind Windows-Benutzervariablen vorgesehen. Starte Pulseboard nach einer Änderung dieser Variablen neu. Die Beispieldatei gehört ins Repository. Die lokale Konfiguration mit Endpunkten und Zugangsdaten bleibt auf dem Rechner.
- Für POST-Checks sollten die konfigurierten Endpunkte wiederholbare, nebenwirkungsfreie Prüfungen anbieten.
- Bei einem Syntaxfehler nach **Neu laden** zeigt die Oberfläche den Fehler und der Monitor verwendet die letzte gültige Konfiguration weiter.
- Der Verlauf bleibt während der laufenden Sitzung erhalten.

## Tray, Autostart und Benachrichtigungen

- Bei `close_to_tray = true` blendet das Schließen des Fensters Pulseboard aus. Die Prüfungen laufen weiter. Ein Linksklick auf das Tray-Symbol öffnet die Oberfläche. Das Kontextmenü bietet **Jetzt prüfen** und **Beenden**.
- **Autostart aus/an** im Dashboard steuert den Start beim Anmelden deines Windows-Benutzers. Der Autostart öffnet Pulseboard im Tray. Zum vollständigen Beenden ohne Tray setze `close_to_tray = false` und lade die Konfiguration neu.
- Windows meldet beim ersten erkannten Ausfall und beim Wechsel zwischen erreichbar und fehlerhaft. Anhaltende Fehler lösen pro Prüfzyklus keine weiteren Meldungen aus. **Nicht stören** und die Windows-Benachrichtigungseinstellungen können die Anzeige beeinflussen.

## Entwickeln und veröffentlichen

Auf Windows brauchst du Rust (MSVC-Toolchain), die Visual Studio C++ Build Tools und das Windows SDK. Für Release-Builds muss GPUI `fxc.exe` finden. Falls nötig, setze `GPUI_FXC_PATH` auf den vollständigen Pfad.

```powershell
cargo run --locked
cargo test --no-default-features --locked
cargo build --release --locked
```

Der Workflow **Windows build and release** baut einen Installer als Actions-Artefakt. Für eine Veröffentlichung erhöhe bei einer neuen Version die Nummer in `Cargo.toml`, aktualisiere `Cargo.lock` und pushe ein passendes Tag (`v0.1.0` für die erste Version). Alternativ kannst du den Workflow auf `main` mit der Option **Publish** manuell starten. Der Windows-Build bezieht den Link zur Update-Seite aus `PULSEBOARD_REPOSITORY` im Format `owner/repo`.
