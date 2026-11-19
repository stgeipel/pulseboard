# Pulseboard

Pulseboard ist ein lokaler Healthcheck-Monitor für Windows mit einer dunklen, nativen Oberfläche aus Rust und GPUI. Er zeigt den HTTP-Status, die Antwortzeit und die letzten zwölf Ergebnisse pro Endpunkt.

## Windows installieren und aktualisieren

1. Lade unter **Releases** die passende `Pulseboard-…-Windows-x64-Setup.exe` herunter und installiere sie für deinen Windows-Benutzer. Der Installer richtet den Startmenü-Eintrag für Windows-Benachrichtigungen ein.
2. Öffne Pulseboard und klicke auf **Konfiguration bearbeiten**. Trage deine Endpunkte ein und wähle **Speichern und anwenden**.
3. Für ein Update öffnest du **Updates**, beendest Pulseboard über das Tray-Menü und führst den neueren Installer aus. Der Installationspfad bleibt gleich. Konfiguration und Autostart-Einstellung bleiben bei einem Upgrade erhalten.

Der Installer ist für Windows 10 ab 22H2 und Windows 11 (x64) konfiguriert. Unter **Assets** findest du auch `pulseboard.exe` zum direkten Start ohne Installation. Die EXE-Dateien sind unsigniert. Für beide gibt es eine SHA-256-Datei.

## Konfiguration

Der integrierte TOML-Editor zeigt Syntaxfarben und Zeilennummern. Fehler mit einer bekannten Position werden im Text markiert. Weitere Validierungsfehler erscheinen unter dem Editor. **Speichern und anwenden** (Strg+S) prüft die Datei vor dem Speichern und lädt sie für den Monitor neu. Bei Änderungen an der Datei durch einen externen Editor bleibt der Entwurf geöffnet. **Neu laden** steht weiterhin für extern gespeicherte Änderungen zur Verfügung.

Über **Design** in der Kopfzeile wählst du zwischen **Porzellan & Kobaltblau**, **Graphit & Bernstein**, **Indigo & Magenta** und **Tiefblau & Cyan**. Die letzten beiden Paletten ergänzen einen dezenten Glow an der Statusleiste und der aktiven Auswahl. Pulseboard speichert die Auswahl in `appearance.toml` neben `config.toml` und lädt sie beim nächsten Start wieder. Die Standardpalette ist Graphit & Bernstein.

Pulseboard legt beim ersten Start eine Vorlage unter `%APPDATA%\Pulseboard\config\config.toml` an. Endpunkte werden erst nach dem Eintragen aktiv. Die App prüft sie beim Start, nach dem Neuladen und anschließend standardmäßig alle 60 Sekunden.

### Servergruppen

Lege mit `[[servers]]` eine Gruppe an und ergänze darunter beliebig viele `[[servers.checks]]`. Die **Matrix** ist die Standardansicht. Sie zeigt je Server eine Zeile und je Check-ID eine Spalte. Verwende auf mehreren Servern dieselbe Check-ID für vergleichbare Endpunkte. Ein Punkt bedeutet, dass der Check für diesen Server nicht konfiguriert ist. Ausstehende Checks erscheinen mit „…“.

Mit **Kacheln** wechselst du zur Ansicht mit kompaktem Status pro Server. Beide Ansichten zeigen betroffene Server zuerst und bieten eine Serversuche sowie den Filter **Nur Fehler**. Ein Klick auf den Namen, eine Kachel oder eine Statuszelle öffnet die Details mit den Kopieraktionen. Pulseboard speichert die gewählte Ansicht neben der Farbpalette in `appearance.toml`.

```toml
[[servers]]
id = "produktion"
name = "Produktion"
base_url = "https://api.example.com"

[[servers.checks]]
id = "live"
name = "API Live"
path = "/health/live"
method = "GET"
expected_status = [200, 204]

[[servers.checks]]
id = "ready"
name = "API Ready"
path = "/health/ready"
method = "GET"
```

- `path` beginnt mit `/` und bezieht sich auf die Wurzel der `base_url`. Verwende für einen Check mit eigener vollständiger Adresse `url` anstelle von `path`. Eine Gruppe mit vollständigen URLs kann `base_url` weglassen.
- **Server kopieren** legt den gesamten TOML-Block mit einer neuen Server-ID in die Zwischenablage. Füge ihn am Ende der Datei ein und lade die Konfiguration neu. Checks mit `path` verwenden die `base_url` der Kopie, sobald du sie änderst.
- **Kopieren** in einer Zeile kopiert den einzelnen Check mit einer neuen ID. Füge ihn am Ende der gewünschten Servergruppe ein, vor dem nächsten `[[servers]]`. Bei Checks im Format `[[checks]]` kannst du die Kopie als eigenen Block ans Dateiende setzen. Wörtlich eingetragene Header, Passwörter und Anfrageinhalte werden ebenfalls kopiert.
- Eine Check-ID muss innerhalb ihrer Servergruppe eindeutig sein. Bereits vorhandene `[[checks]]` funktionieren weiter und erscheinen unter **Einzelne Endpunkte**.

### Einzelne Endpunkte

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

- Jeder Check braucht eine stabile `id`. Ohne `method` sendet Pulseboard einen GET-Request. Ohne `expected_status` zählt jeder 2xx-Status als Erfolg.
- Pulseboard bewertet den HTTP-Status. Die Latenz endet beim Empfang der Antwort-Header. Weiterleitungen werden als eigener HTTP-Status angezeigt und Zugangsdaten bleiben am konfigurierten Endpunkt.
- `headers` akzeptiert direkte Werte. Bei `headers_from_env` enthält der TOML-Wert den Variablennamen und die Windows-Benutzervariable den vollständigen Headerwert, etwa `Bearer …`. Für Basic Auth stehen `password_env` und alternativ `password` zur Verfügung.
- Für reale Zugangsdaten sind Windows-Benutzervariablen vorgesehen. Starte Pulseboard nach einer Änderung dieser Variablen neu. Die Beispieldatei gehört ins Repository. Die lokale Konfiguration mit Endpunkten und Zugangsdaten bleibt auf dem Rechner.
- Für POST-Checks sollten die konfigurierten Endpunkte wiederholbare, nebenwirkungsfreie Prüfungen anbieten.
- Bei einem Syntaxfehler nach **Neu laden** zeigt die Oberfläche den Fehler und der Monitor verwendet die letzte gültige Konfiguration weiter.
- Der Verlauf bleibt während der laufenden Sitzung erhalten.

### Absturzprotokoll unter Windows

Über **Absturzprotokoll** unten im Fenster öffnest du `%APPDATA%\Pulseboard\config\crash.log`. Die Datei hält App-Start, Fenster- und Tray-Aktionen sowie das reguläre Beenden fest. Bei einem Rust-Panic schreibt Pulseboard einen Stacktrace. Erreicht eine unbehandelte native Windows-Ausnahme den Absturzhandler, protokolliert die App den Fehlercode, den Thread und bei einem Speicherzugriffsfehler die Art des Zugriffs. Wenn Windows das geladene Modul ermitteln kann, stehen auch sein Dateiname und der Offset im Protokoll. Die Datei enthält keine Konfigurationswerte, URLs, Zugangsdaten oder Antwortinhalte.

Wenn der Prozess plötzlich endet, starte Pulseboard erneut. Der nächste Start markiert die vorige Sitzung als unerwartet beendet. Das kann auch nach einem erzwungenen Beenden oder einem Neustart des Rechners passieren. Beim Start wird ein Log ab 512 KiB nach `crash.previous.log` verschoben. Für Windows-Fehler, die den Absturzhandler umgehen, kannst du zusätzlich in der **Ereignisanzeige → Windows-Protokolle → Anwendung** nach `pulseboard.exe` suchen.

## Tray, Autostart und Benachrichtigungen

- Bei `close_to_tray = true` blendet das Schließen des Fensters Pulseboard aus. Die Prüfungen laufen weiter. Ein Linksklick auf das Tray-Symbol öffnet die Oberfläche. Das Kontextmenü bietet **Jetzt prüfen** und **Beenden**.
- **Autostart aus/an** im Dashboard steuert den Start beim Anmelden deines Windows-Benutzers. Der Autostart öffnet Pulseboard im Tray. Zum vollständigen Beenden ohne Tray setze `close_to_tray = false` und lade die Konfiguration neu.
- Windows meldet beim ersten erkannten Ausfall und beim Wechsel zwischen erreichbar und fehlerhaft. Anhaltende Fehler lösen pro Prüfzyklus keine weiteren Meldungen aus. **Nicht stören** und die Windows-Benachrichtigungseinstellungen können die Anzeige beeinflussen.

## Entwickeln und veröffentlichen

Für weitere Designs ergänze eine Palette in `src/appearance.rs` und trage sie in `THEMES` ein. Die Felder benennen die jeweiligen Rollen, etwa Akzent, Flächen, Text und Statusfarben. Verwende eine eindeutige, stabile `id`. Das Dropdown übernimmt neue Paletten aus `THEMES` automatisch. Mit `glow: Some(farbe)` kannst du optional einen Lichtsaum für das Design aktivieren.

Auf Windows brauchst du Rust (MSVC-Toolchain), die Visual Studio C++ Build Tools und das Windows SDK. Für Release-Builds muss GPUI `fxc.exe` finden. Falls nötig, setze `GPUI_FXC_PATH` auf den vollständigen Pfad.

`rust-toolchain.toml` legt Rust 1.99.1 fest, um einen Windows-Absturz mit Rust 1.100.0 und `vertdll.dll` zu umgehen. Der Windows-Workflow prüft zusätzlich die DLL-Importe der fertigen EXE. [Rust-Issue #165915](https://github.com/rust-lang/rust/issues/165915)

```powershell
cargo run --locked
cargo test --no-default-features --locked
cargo build --release --locked
```

Der Workflow **Windows build and release** baut `pulseboard.exe` und den Installer für Windows x64. Beide Dateien und ihre SHA-256-Prüfsummen stehen im Actions-Artefakt und bei einer Veröffentlichung unter **Releases → Assets** bereit. Für eine neue Version erhöhe die Nummer in `Cargo.toml`, aktualisiere `Cargo.lock` und pushe ein passendes Tag wie `v0.1.1`. Alternativ kannst du den Workflow auf `main` mit der Option **Publish** manuell starten. Der Windows-Build bezieht den Link zur Update-Seite aus `PULSEBOARD_REPOSITORY` im Format `owner/repo`.
