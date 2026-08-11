# Runtime Theme System: frei konfigurierbare Farben und statische Verläufe

Stand: 2026-07-27 · Branch: `feature/theme-system` (gestapelt auf
`feature/session-switcher` bei `71bfc94`, ursprünglich von
`upstream/development`)

> Lokales Entwicklungsdokument in Deutsch. Diese Spec wird nicht committed und
> gehört nicht in den späteren Upstream-PR. Produktdokumentation, Beispiel-Themes,
> Code, Kommentare und Doc-Comments werden für den PR auf Englisch geschrieben.

## Problem

SSHub verwendet derzeit eine fest kompilierte Palette in `src/tui/theme.rs`.
16 RGB-Konstanten und davon abgeleitete `Style`-Hilfsfunktionen werden von allen
Screens und Widgets gemeinsam verwendet. Anwender können weder das gesamte Theme
wechseln noch einzelne Bereiche wie Rahmen, Trennlinien, Session-Tabs, Footer-Keys
oder Auswahlflächen unabhängig gestalten.

Ratatui und Crossterm können True Color über `Color::Rgb` ausgeben, sofern der
Terminal-Emulator dies unterstützt. Terminalzellen besitzen jedoch keinen
Alpha-Kanal. Echte Transparenz oder Opacity pro Zelle ist deshalb nicht möglich.

SSHub soll ein zur Laufzeit austauschbares Theme-System erhalten, das einerseits
einfach über gemeinsame Palettenwerte bedienbar ist und andererseits gezielte
Overrides für jeden relevanten SSHub-Bereich erlaubt. Statische Farbverläufe
gehören ausdrücklich zu Version 1.

## Ziele

- Beliebig viele benutzerdefinierte Themes als TOML-Dateien
- Theme-Auswahl zur Laufzeit über einen eigenen Picker
- Live-Vorschau ohne sofortige Speicherung
- Frei benannte Palette, fester semantischer Kern und komponentenspezifische Overrides
- Vererbung von eingebauten und benutzerdefinierten Themes
- Hex- und RGB-Farbwerte
- Helligkeitsanpassung und simulierte Opacity
- Wiederverwendbare statische Multi-Stop-Verläufe
- Individuelle Gestaltung von Rahmen, Titeln, Texten, Flächen, Trennlinien,
  Footer-Hinweisen, Session-Tabs, Statuswerten und Auswahlzuständen
- Strikter eingebauter Validator über `sshub theme check <file>`
- Sichere Fallbacks: Ein fehlerhaftes Theme darf die TUI nicht unbenutzbar machen
- Rückwärtskompatibles `default`-Theme mit zellgenauer Parität für alle
  bisherigen `theme.rs`-Pfade und dokumentierter semantischer Normalisierung
  verstreuter direkter ANSI-Stile
- Dokumentierte Beispiel-Themes, die das Schema praktisch erklären

## Nicht-Ziele für Version 1

- Animierte oder wandernde Farbverläufe
- Ein visueller Theme-Editor innerhalb der TUI
- Automatisches Beobachten von Dateien über einen Filesystem-Watcher
- Echte Alpha-Transparenz einzelner Terminalzellen
- Umfärben des Inhalts einer eingebetteten SSH/PTTY-Session
- Mausbedienung im Theme-Picker
- Import fremder Theme-Formate
- Freies Styling undifferenzierter Terminalkoordinaten nach Art von CSS-Selektoren
- Automatisches Herunterladen oder Installieren von Themes aus dem Internet
- Konfigurierbare Glyphen, Rahmenformen, Sparkline-Zeichen oder Statussymbole

## Festgelegte Produktentscheidungen

### Eigene Theme-Dateien

Benutzer-Themes liegen unter:

```text
~/.config/sshub/themes/*.toml
```

Bei gesetztem `SSHUB_CONFIG_DIR` wird entsprechend
`$SSHUB_CONFIG_DIR/themes/*.toml` verwendet. Die bestehende Legacy-Fallback-Logik
für `SSH_LAUNCHER_CONFIG_DIR` bleibt wirksam.

Die Hauptkonfiguration speichert nur die aktive Theme-ID:

```toml
[appearance]
active_theme = "aqua"
```

Das bewahrt `config.toml` als kompakte Anwendungskonfiguration. Themes können
separat kopiert, geteilt und versioniert werden.

### Palette, semantischer Kern und Komponenten-Overrides

Ein Theme besitzt drei Styling-Ebenen:

1. `[palette]` enthält beliebig benannte Hilfsfarben des Theme-Autors.
2. `[semantic]` enthält einen kleinen, festen Kern aus ungefähr 20
   SSHub-Bedeutungen wie Hintergrund, Text, Fokus, Akzent und Status.
3. `[components]` überschreibt bei Bedarf konkrete SSHub-Bereiche.

Jede Komponentenrolle besitzt im Rust-Code einen dokumentierten Fallback auf
eine semantische Rolle. Deshalb muss ein Theme nicht jede UI-Komponente von Hand
verdrahten: Wer beispielsweise nur `semantic.accent` ändert, verändert alle
geerbten Komponenten, die den Akzent verwenden. Gleichzeitig kann ein
vollständiges Theme jede publizierte Komponentenrolle unabhängig überschreiben.

Benannte statische Verläufe liegen unter `[gradients]` und können von
gradientenfähigen Komponentenrollen referenziert werden.

### Theme-Vererbung

`extends` verweist auf die ID eines eingebauten oder benutzerdefinierten Themes:

```toml
extends = "default"
```

Für alle Benutzer-Themes ist `extends = "default"` der implizite Standard.
Ein explizites `extends` wählt stattdessen ein anderes Eltern-Theme. Nur das
eingebaute Wurzel-Theme `default` besitzt keinen Parent.

Vererbung ist ein Deep-Merge auf `ThemeDefinition`-Ebene:

- Metadaten des Kindes ersetzen gleichnamige Metadaten des Eltern-Themes.
- `palette`, `semantic` und `gradients` werden nach Eintragsnamen gemergt;
  ein Kind-Eintrag ersetzt den gleichnamigen Eintrag vollständig.
- `components` werden zuerst nach vollständigem Rollenpfad und bei `Style`
  anschließend nach Style-Feld gemergt; Kind-Felder gewinnen.
- Der reservierte Wert `"auto"` entfernt einen geerbten `Color`-, `Paint`-
  oder `Tint`-Override oder ein einzelnes `Style`-Feld und stellt den
  Rust-seitigen Rollen-Fallback wieder her. `{ auto = true }` setzt einen
  gesamten `Style`-Override zurück.
- Eine gesetzte `modifiers`-Liste ersetzt die Elternliste vollständig;
  `modifiers = []` entfernt alle Modifikatoren, `modifiers = "auto"` stellt
  den semantischen Rollen-Fallback wieder her.

Farb- und Gradient-Referenzen werden **erst nach dem vollständigen Deep-Merge**
aufgelöst. Überschreibt ein Kind `semantic.accent`, wirken deshalb auch alle
geerbten Komponentenreferenzen auf `accent` mit der neuen Farbe. Ein Kind darf
Namen referenzieren, die nur im Eltern-Theme definiert sind.

Der Resolver erkennt:

- unbekannte Eltern-Themes
- direkte und indirekte Zyklen
- ungültige oder mehrdeutige Theme-IDs
- Referenzen auf nicht vorhandene Farben oder Verläufe

Nach Merge, semantischen Rust-Fallbacks und Auflösung enthält `ResolvedTheme`
alle Rollen als Typinvariante. Komponentenrollen bleiben in Theme-Dateien
immer optional. Neue UI-Rollen erhalten im Code einen semantischen Fallback und
machen bestehende Themes dadurch nicht ungültig.

### Statische Verläufe

Version 1 unterstützt ausschließlich statische Verläufe. Die Architektur trennt
Definition, Auflösung, Sampling und Rendering, enthält aber keine
Animationsparameter. Animationen sind ein mögliches späteres Feature und werden
nicht vorab in das öffentliche V1-Schema eingebaut.

## Theme-Dateiformat

### Metadaten

Minimales Benutzer-Theme:

```toml
schema_version = 1
name = "My Theme"
```

Unterstützte Top-Level-Felder:

| Feld | Pflicht | Bedeutung |
| --- | --- | --- |
| `schema_version` | ja | Muss in V1 exakt `1` sein |
| `name` | ja | Anzeigename im Theme-Picker |
| `extends` | nein | Eltern-Theme; Standard für Benutzer-Themes ist `default` |
| `description` | nein | Kurze Beschreibung für die Vorschau |
| `author` | nein | Freie Theme-Metadaten, keine Code-Author-Angabe |
| `palette` | nein | Benannte Farben |
| `semantic` | nein | Overrides des festen semantischen Kerns |
| `gradients` | nein | Benannte statische Verläufe |
| `components` | nein | Komponenten-Overrides |

Die technische Theme-ID ist bei Benutzer-Themes der Dateiname ohne `.toml`.
Sie verwendet nur ASCII-Kleinbuchstaben, Ziffern, `-` und `_`. Der sichtbare
`name` darf frei gewählt werden. Die IDs eingebauter Themes sind reserviert und
können nicht durch Benutzerdateien überschrieben werden.

### Farbwerte

Ein Farbwert akzeptiert genau eine der folgenden Formen:

```toml
[palette]
deep_sea = "#08202a"
accent = "#52d273"
warning = { rgb = [245, 180, 60] }
surface = { color = "palette.deep_sea", brightness = 0.12 }
soft_accent = { color = "palette.accent", opacity = 0.35, over = "palette.surface" }

[semantic]
background = "terminal"
text = "#d6e1d4"
accent = "palette.accent"
```

Regeln:

- Ein blanker String ist genau dann ein Hex-Literal, wenn er dem Muster
  `^#[0-9a-fA-F]{6}$` entspricht. Referenzen sind immer qualifiziert:
  `"palette.<name>"` oder `"semantic.<name>"`.
- `"terminal"` ist ein reservierter Sentinel für Ratatuis `Color::Reset`,
  also den Standard des Terminal-Emulators. Er ist keine Palettenreferenz.
- Hex verwendet exakt `#RRGGBB`.
- `rgb` enthält exakt drei Ganzzahlen im Bereich `0..255`.
- `color` referenziert einen qualifizierten Eintrag aus Palette oder
  semantischem Kern.
- `rgb` und `color` schließen sich gegenseitig aus.
- `brightness` ist optional und liegt in `-1.0..1.0`.
- `opacity` ist optional und liegt in `0.0..1.0`.
- `over` ist nur zusammen mit `opacity` erlaubt und wählt den opaken
  RGB-Mischgrund. Standard ist `semantic.background`.
- `brightness`, `opacity` und `over` sind mit `"terminal"` unzulässig.
- Weitere Felder sind Fehler.
- Rekursive und zyklische Farbreferenzen sind Fehler.
- Unqualifizierte oder nicht auflösbare Strings werden niemals als Ratatui-
  oder ANSI-Farbnamen interpretiert.

Positive Helligkeit mischt die aufgelöste Farbe in Richtung Weiß, negative
Helligkeit in Richtung Schwarz. Danach wird Opacity simuliert, indem die Farbe
mit `over` beziehungsweise `semantic.background` gemischt wird:

```text
result = color * opacity + over * (1 - opacity)
```

`semantic.background` darf `"terminal"` sein, kann dann aber nicht als
Opacity-Mischgrund dienen. Ein Theme, das Opacity verwendet, muss dafür einen
expliziten opaken `over`-Wert setzen oder einen opaken semantischen Hintergrund
besitzen.

`semantic.background` darf selbst kein `opacity` verwenden. Jeder explizite
oder implizite `over`-Wert muss zu einer opaken RGB-Farbe auflösen; `"terminal"`
als Mischgrund ist ein Validierungsfehler. Damit kann keine implizite
Selbstreferenz entstehen.

Mischung und Gradient-Interpolation arbeiten in V1 kanalweise im sRGB-Raum.
Jeder Zwischenkanal wird in `f32` berechnet, auf `0..255` geclamped und mit
`round()` auf den nächsten `u8` gerundet. Helligkeit wird vor Opacity angewandt.
Die Simulation ist damit zellengenau deterministisch, aber keine echte
Terminaltransparenz.

### Semantischer Kern

Der feste semantische Kern enthält in `schema_version = 1` exakt:

```text
background, canvas, surface, surface_raised,
border, border_focus, border_popup,
text, text_bright, text_highlight, text_muted, text_dim, text_inverse,
accent, selection_bg, selection_fg,
success, warning, error, info, connecting, exited, unknown
```

Alle Felder sind in Benutzer-Themes optional und erben letztlich aus `default`.
Im Wurzel-Theme sind sie vollständig. Komponenten-Fallbacks referenzieren nur
diese stabilen semantischen Namen, niemals frei erfundene Palettennamen.

Die Default-Belegung ist gegen die bisherigen Konstanten in
`src/tui/theme.rs` festgeschrieben:

| Semantik | Default |
| --- | --- |
| `background` | `"terminal"` |
| `canvas` | `#0b0d10` (`BG`) |
| `surface`, `surface_raised` | `"terminal"` |
| `border` | `#1f2a24` (`BORDER`) |
| `border_focus` | `#6fb3b8` (`CYAN`) |
| `border_popup` | `#6a7a72` (`MUTE`) |
| `text` | `#d6e1d4` (`TEXT`) |
| `text_bright`, `selection_fg` | `#c7e8c9` (`BRIGHT`/`SEL_FG`) |
| `text_highlight` | `#f4f8f3` (`WHITE`) |
| `text_muted` | `#6a7a72` (`MUTE`) |
| `text_dim`, `unknown` | `#3d4a44` (`DIM`) |
| `text_inverse` | `#06080a` (`BG_DEEP`) |
| `accent` | `#9ec99b` (`ACCENT`) |
| `selection_bg` | `#182b22` (`SEL_BG`) |
| `success` | `#7cb992` (`GREEN`) |
| `warning`, `connecting` | `#d6a76b` (`AMBER`) |
| `error`, `exited` | `#c97a7a` (`RED`) |
| `info` | `#6fb3b8` (`CYAN`) |

`canvas` ist der opake Kompatibilitäts- und Mischwert für ein ansonsten
transparentes Theme; `background` steuert, ob der App-Hintergrund tatsächlich
gemalt wird.

Das eingebaute `default.toml` verwendet unter `[components]` ausschließlich
`semantic.*`-Referenzen sowie die Sentinel-Werte `"terminal"`, `"auto"` und
`"native"` – niemals Hex-Literale oder `palette.*`-Referenzen. Dadurch bleibt
jede Default-Zuordnung über den semantischen Kern überschreibbar.
Bestehende direkte ANSI-Farben außerhalb von `theme.rs` werden bewusst auf
ihre semantischen Rollen normalisiert; diese begrenzte Vereinheitlichung wird
in den Release Notes dokumentiert.

Von der zellgenauen Default-Parität gibt es damit **drei** dokumentierte
Abweichungsklassen, nicht eine:

1. die eben beschriebene **ANSI-Normalisierung** — die Zelle hatte eine direkte
   ANSI-Farbe, für die es keine `theme.rs`-Vorlage gibt, an die man sich halten
   könnte;
2. die weiter unten einzeln festgehaltenen **farblosen Zellen** (Abschnitt
   „Bewusste Grenzen und direkte Farbmigration"): Zellen, die überhaupt keine
   Farbe trugen und mit der Migration bewusst eine bekommen. Sie fallen nicht
   unter (1) und sind jede für sich mit Begründung protokolliert;
3. **bewusste Verbesserungen pro Oberfläche**: Stellen, an denen das
   eingefrorene Aussehen nachweislich unbrauchbar war und `default` deshalb
   absichtlich etwas anderes zeichnet. Genau zwei Fälle, beide im Rollenkatalog
   unter „Tabs, Tunnel und SFTP" einzeln begründet — nicht im Abschnitt zu (2):
   - `components.selection.inactive` — die ausgewählte Zeile des nicht
     fokussierten SFTP-Panels war überhaupt nicht hervorgehoben. Expliziter
     Override, deshalb in `DEFAULT_PARITY_OVERRIDES` und in
     `assets/themes/default.toml` verankert;
   - `components.sftp.notice` — der Queue-Warnhinweis steckte bei gefüllter
     Queue in der Kopfzeile und wird jetzt getrennt gezeichnet. Das ist eine
     Renderer-Änderung, kein Rollen-Override: in `DEFAULT_PARITY_OVERRIDES`
     taucht der Fall deshalb bewusst **nicht** auf.

Alle drei Klassen gehören in die Release Notes. Sie sind abschließend nur
zusammen mit den Einzelbegründungen weiter unten zu lesen; eine Abweichung, die
in keiner der drei steht und dort nicht begründet ist, wäre ein Fehler.

### Verläufe

Ein Verlauf besitzt eine Richtung und mindestens zwei frei positionierte
Farbstopps:

```toml
[gradients.panel_border]
direction = "horizontal"
stops = [
  { at = 0.0, color = "semantic.accent" },
  { at = 0.55, color = "#a0ffe0" },
  { at = 1.0, color = { rgb = [8, 40, 50], brightness = 0.05 } },
]
```

Unterstützte Richtungen:

- `horizontal`
- `vertical`
- `diagonal_down`
- `diagonal_up`
- `perimeter`

Regeln:

- `stops` enthält mindestens zwei Einträge.
- `at` liegt in `0.0..1.0`.
- Positionen sind streng aufsteigend.
- Der erste Stopp liegt bei `0.0`, der letzte bei `1.0`.
- Jeder Stopp verwendet die gleiche Farbsyntax wie die Palette.
- Komponenten referenzieren Verläufe ausschließlich qualifiziert als
  `"gradients.<name>"`.
- `perimeter` ist nur für geschlossene Rahmen gültig.
- Bei `perimeter` müssen der erste und letzte Stopp zur gleichen RGB-Farbe
  auflösen. Dadurch schließt sich der Verlauf ohne sichtbare Naht.
- Ein Komponentenfeld darf nur dann einen Verlauf referenzieren, wenn diese
  Rolle Verläufe unterstützt.

Zwischen zwei Stopps werden die RGB-Kanäle nach den festgelegten
sRGB-/Rundungsregeln deterministisch interpoliert.
Die Interpolation liegt hinter einer internen `GradientSampler`-Abstraktion;
das öffentliche Dateiformat hängt nicht von einer bestimmten Fremd-Crate ab.

### Komponentenwerte

Ein normaler Text- oder Flächenstil kann Vordergrund, Hintergrund und
Modifikatoren definieren:

```toml
[components.footer.key]
foreground = "#f8fff9"
background = "semantic.accent"
modifiers = ["bold"]
```

Unterstützte Modifikatoren:

- `bold`
- `dim`
- `italic`
- `underlined`
- `reversed`
- `crossed_out`

Paint-Felder wie Rahmen oder Trennlinien akzeptieren eine Farbe oder einen
Verlauf:

```toml
[components.dashboard.host_list]
border = { gradient = "gradients.panel_border" }
background = { color = "semantic.surface", opacity = 0.85, over = "palette.deep_sea" }

[components.dashboard.details]
border = "semantic.border"
```

Inline definierte Multi-Stop-Verläufe in Komponenten sind in V1 nicht erlaubt.
Verläufe werden unter `[gradients]` benannt und anschließend referenziert. Das
hält komplexe Dateien lesbar und ermöglicht Wiederverwendung.

Jede Zuweisung unter `[components]` adressiert den vollständigen Rollenpfad.
Die Tabellenform ist nur TOML-Kurzschreibweise: Das obige `border` entspricht
`components.dashboard.host_list.border`. Ein Kind kann geerbte Overrides
gezielt zurücksetzen:

```toml
[components.dashboard.host_list]
border = "auto"

[components.footer.key]
background = "auto"
modifiers = []
```

`"auto"` ist ausschließlich unter `[components]` ein Reset-Sentinel und in
Palette, Semantik und Gradient-Stopps ungültig. `"native"` ist ausschließlich
für `Tint`-Rollen zulässig.

Jede öffentliche Komponentenrolle besitzt genau einen Werttyp:

| Typ | Erlaubte Werte |
| --- | --- |
| `Color` | Farbe, Referenz, `"terminal"` oder `"auto"` |
| `Style` | `foreground`, `background`, `modifiers`, je Feld optional `"auto"`; gesamte Rolle optional `{ auto = true }` |
| `Paint` | Farbe/Referenz, `"terminal"`, `"auto"` oder `{ gradient = "…" }` |
| `Tint` | Farbe/Referenz, `"native"` für unveränderte Logo-Farben oder `"auto"` |

`foreground` und `background` innerhalb eines `Style` akzeptieren dieselbe
Farbsyntax einschließlich `"terminal"` wie `Color`. Gradient-Referenzen sind
analog zu Farben vollständig als `"gradients.<name>"` qualifiziert.

Die folgende Role-to-Type-Matrix ist nach Inventur sämtlicher Laufzeitnutzungen
von `theme::*`, direkten `Color::*`-Werten und `Color::Reset` der eingefrorene
V1-Vertrag. Validator, `ResolvedTheme` und README werden aus derselben
typisierten Rust-Definition abgeleitet; es gibt keine zweite handgepflegte
Implementierungsliste.

## Öffentlicher V1-Rollenkatalog

Der Checker behandelt unbekannte Rollen als Fehler. Die Runtime zeigt bei
unbekannten Komponentenrollen aus einer möglicherweise neueren V1-Datei eine
nicht fatale Diagnose, ignoriert nur diese Rolle und verwendet ihren
semantischen Fallback. Unbekannte Sections, Style-Felder oder Wertformen bleiben
auch zur Laufzeit Fehler. Umbenannte Rollen erhalten bei Bedarf eine
Alias-/Deprecation-Tabelle.

Die Kurzschreibweise `{a,b,c}` in der Dokumentation bezeichnet mehrere
eigenständige, wörtliche Rollen; im TOML werden keine geschweiften
Klammerausdrücke verwendet.

### Global

| Rollenpfad | Typ | Semantischer Fallback |
| --- | --- | --- |
| `components.app.background` | `Paint` | `background` |
| `components.text.{primary,bright,muted,dim}` | `Style` | `text`, `text_bright`, `text_muted`, `text_dim` |
| `components.selection.active` | `Style` | `selection_fg` auf `selection_bg` |
| `components.selection.inactive` | `Style` | `text` auf `surface_raised` |
| `components.focus.indicator` | `Style` | `accent` |
| `components.separator.primary` | `Paint` | `border` |
| `components.separator.secondary` | `Paint` | `text_dim` |
| `components.status.{success,warning,error,info,unknown}` | `Color` | jeweils gleichnamige semantische Rolle |

Die String-zu-Status-Zuordnung lautet:
`ok|launched|online|up → success`,
`slow|idle|retry|warning → warning`,
`down|fail|error|unreachable → error`;
alle übrigen Werte verwenden `status.unknown`.

`components.status` besitzt bewusst **keine** `connecting`- und `exited`-Rolle.

`components.focus.indicator` ist die **globale** Markierung des aktiven Feldes
und gilt nur dort, wo der Marker vor der Migration keine eigene `theme.rs`-Zelle
besaß, sondern an einem direkt in ANSI eingefärbten Label klebte: Host-Formular
und Identity-Formular. Überall sonst trug der Marker den Stil seiner Zeile
beziehungsweise seines Labels — und behält ihn über eine eigene `marker`-Rolle
seiner Familie: `components.picker.marker` (Field-Picker, Tag-Filter),
`components.group_form.marker`, `components.keybind.marker` und
`components.settings.marker` (Tunnel-Reconnect). So bleibt der Marker überall
unabhängig themebar, ohne dass `default` seine frühere Zelle verliert.
Der einzige produktive Leser ist die Audit-Statusspalte, deren erlaubte Werte
`ok`, `fail` und `retry` sind — eine Verbindungs- oder Beendigungsphase kommt
dort nicht vor. Lebenszyklus-Zustände gehören den Domänen, die sie wirklich
besitzen: `components.session.{connecting,exited}` und
`components.tunnel.{connecting,stopped,retrying}`.

### Header und lokales Session-Chrome

| Rollenpfad | Typ | Semantischer Fallback |
| --- | --- | --- |
| `components.header.background` | `Paint` | `surface_raised` |
| `components.header.brand` | `Style` | `text_inverse` auf `text_bright` |
| `components.header.stats_label` | `Style` | `text_muted` |
| `components.header.stats_value` | `Style` | `text` |
| `components.header.separator` | `Paint` | `text_dim` |
| `components.header.session_active` | `Style` | `text_inverse` auf `text_bright` |
| `components.header.session_inactive` | `Style` | `text_muted` |
| `components.header.session_more` | `Style` | `text_muted` |
| `components.header.session_{success,warning,error}` | `Color` | `success`, `warning`, `error` |
| `components.session.background` | `Paint` | `background` |
| `components.session.border` | `Paint` | `border_popup` |
| `components.session.title` | `Style` | `text_inverse` auf `text_bright` |
| `components.session.scrollback` | `Style` | `warning` |
| `components.session.connecting` | `Color` | `connecting` |
| `components.session.exited` | `Color` | `exited` |
| `components.session.debug_tail` | `Style` | `text_dim` |

Diese Rollen betreffen ausschließlich von SSHub gerendertes Chrome,
Connecting-/Failure-Screens, Hinweise und Debug-Tail – niemals Remote-PTY-Zellen.
Inventarorte: `src/session/render.rs`, `src/tui/widgets/header.rs`,
`src/tui/widgets/tab_bar.rs`.

### Dashboard

| Rollenpfad | Typ | Semantischer Fallback |
| --- | --- | --- |
| `components.dashboard.host_list.border` | `Paint` | `border` |
| `components.dashboard.host_list.border_focused` | `Paint` | `border_focus` |
| `components.dashboard.host_list.title` | `Style` | `text_bright` |
| `components.dashboard.host_list.count` | `Style` | `text_dim` |
| `components.dashboard.host_list.background` | `Paint` | `surface` |
| `components.dashboard.host_list.group` | `Style` | `info` |
| `components.dashboard.host_list.host` | `Style` | `text` |
| `components.dashboard.host_list.host_selected` | `Style` | `text_highlight` auf `selection_bg` |
| `components.dashboard.host_list.match` | `Style` | `warning` |
| `components.dashboard.details.border` | `Paint` | `border` |
| `components.dashboard.details.border_focused` | `Paint` | `border_focus` |
| `components.dashboard.details.title` | `Style` | `text_bright` |
| `components.dashboard.details.background` | `Paint` | `surface` |
| `components.dashboard.details.label` | `Style` | `info` |
| `components.dashboard.details.value` | `Style` | `text` |
| `components.dashboard.details.metadata` | `Style` | `text_muted` |
| `components.dashboard.details.field_marker` | `Style` | `accent` |
| `components.dashboard.metrics.sparkline_{low,medium,high}` | `Color` | `success`, `warning`, `error` |
| `components.dashboard.{ssh_log,agent,latency,recent,auth,ping}.border` | `Paint` | `border` |
| `components.dashboard.{ssh_log,agent,latency,recent,auth,ping}.border_focused` | `Paint` | `border_focus` |
| `components.dashboard.{ssh_log,agent,latency,recent,auth,ping}.title` | `Style` | `text_bright` |
| `components.dashboard.{ssh_log,agent,latency,recent,auth,ping}.background` | `Paint` | `surface` |

Inventarorte: `src/tui/widgets/hosts_panel.rs`, `detail_panel.rs`,
`middle_stack.rs`, `right_stack.rs`, `panel_box.rs` sowie der ältere
`src/tui/screens/hosts.rs`-Pfad mit direkten ANSI-Farben.

**`components.dashboard.metrics` ist kein Panel, sondern die panelübergreifende
Sparkline-Rampe.** Die Familie besitzt ausschließlich
`sparkline_{low,medium,high}` und wird von jedem Panel benutzt, das Messwerte
zeichnet (derzeit `latency`). Sie hat bewusst **keine** `border`-,
`border_focused`-, `title`-, `count`- oder `background`-Rolle: sie rahmt nichts
ein. Panels mit eigenem Rahmen — `latency` eingeschlossen — bringen ihre eigene
Panel-Familie mit: `border`, `border_focused`, `title`, `background` und, nur wo
der Aufrufer wirklich ein Badge übergibt, `count`.

**Eine `count`-Rolle existiert nur dort, wo der produktive Aufrufer wirklich ein
Badge übergibt.** Das sind `dashboard.host_list`, `sftp.panel` und
`broadcast.panel`. Die übrigen Panels rufen `render_panel_box` immer mit
`count = None` auf; eine veröffentlichte `count`-Rolle könnte dort in keinem
Zustand eine Zelle erreichen. Ein Aufruf von `render_panel_box`
übergibt deshalb einen `PanelFrame`, der Rollenbündel **und** Badge zusammen
trägt und nur über `PanelRoles::plain()` bzw. `PanelRoles::with_badge(text)`
entstehen kann. `PanelRoles`, `PanelBadge` und `PanelFrame` haben **alle** private
Felder, sodass weder ein Badge mit der `count`-Rolle einer fremden Familie, noch
ein Panel in den Farben der einen und mit dem Badge einer anderen Familie, noch
ein aus Teilen zweier Familien zusammengesetztes Rollenbündel ausdrückbar ist. Eine Familie ohne
`count`-Rolle liefert einen Frame ohne Badge — es wird kein Platz reserviert und
nichts gezeichnet.

### Footer und Statusleiste

| Rollenpfad | Typ | Semantischer Fallback |
| --- | --- | --- |
| `components.footer.background` | `Paint` | `surface_raised` |
| `components.footer.key` | `Style` | `text_bright` |
| `components.footer.label` | `Style` | `text_muted` |
| `components.footer.separator` | `Paint` | `text_dim` |
| `components.status_bar.toast` | `Style` | `info`, in `default` mit `reversed` |

### Popups, Picker, Formulare und Tabellen

| Rollenpfad | Typ | Semantischer Fallback |
| --- | --- | --- |
| `components.popup.background` | `Paint` | `surface` |
| `components.popup.border` | `Paint` | `border_popup` |
| `components.popup.title` | `Style` | `text_bright` |
| `components.popup.hint` | `Style` | `text_dim` |
| `components.popup.legend` | `Style` | `text_muted` |
| `components.popup.error` | `Style` | `error` |
| `components.popup.warning` | `Style` | `warning` |
| `components.picker.border` | `Paint` | `accent` |
| `components.picker.query` | `Style` | `text_bright` |
| `components.picker.match` | `Style` | `accent` |
| `components.picker.row` | `Style` | `text` |
| `components.picker.row_selected` | `Style` | `selection_fg` auf `selection_bg` |
| `components.picker.marker` | `Style` | `selection_fg` auf `selection_bg` |
| `components.picker.badge_{success,warning,error}` | `Color` | `success`, `warning`, `error` |
| `components.command_palette.query` | `Style` | `text_highlight` |
| `components.command_palette.row_selected` | `Style` | `text_highlight` auf `selection_bg` |
| `components.settings.row_selected` | `Style` | `text_highlight` auf `selection_bg` |
| `components.settings.marker` | `Style` | `text_highlight` auf `selection_bg` |
| `components.group_form.label` | `Style` | `text_muted` |
| `components.group_form.label_focused` | `Style` | `text_bright` mit Bold |
| `components.group_form.value` | `Style` | `text` |
| `components.group_form.value_focused` | `Style` | `text_bright` mit Bold |
| `components.group_form.marker` | `Style` | `text_bright` mit Bold |
| `components.form.label` | `Style` | `text_dim` |
| `components.form.label_focused` | `Style` | `info` |
| `components.form.label_editing` | `Style` | `warning` |
| `components.form.value` | `Style` | `text` |
| `components.form.input` | `Style` | `text_bright` |
| `components.form.input_focused` | `Style` | `text_bright` |
| `components.form.input_editing` | `Style` | `text_bright` mit Underline/Bold |
| `components.form.help` | `Style` | `text_dim` |
| `components.form.error` | `Style` | `error` |
| `components.tunnel_form.title` | `Style` | `accent` |
| `components.tunnel_form.label` | `Style` | `text_muted` |
| `components.tunnel_form.label_focused` | `Style` | `text_bright` |
| `components.tunnel_form.value` | `Style` | `text` |
| `components.tunnel_form.value_focused` | `Style` | `text_bright` |
| `components.tunnel_form.value_editing` | `Style` | `text_highlight` mit Underline |
| `components.tunnel_form.marker` | `Style` | `success` |
| `components.tunnel_form.help` | `Style` | `text_dim` |
| `components.tunnel_form.border` | `Paint` | `accent` |
| `components.table.row` | `Style` | `text` |
| `components.table.row_selected` | `Style` | `selection_fg` auf `selection_bg` |

Der auf diesem Feature-Stack bereits vorhandene
`src/tui/screens/session_picker.rs` verwendet für Titel, Trennlinie, Hinweis und
`current`-Marker die passenden `popup.*`-Rollen. Sein **Rahmen** ist dagegen seit
jeher der Akzent und nicht der gedämpfte Popup-Rand, deshalb besitzt er mit
`components.picker.border` eine eigene Paint-Rolle. Query, normale und
ausgewählte Zeile sowie die Zustands-Badges verwenden `picker.*`;
`Up`/`Connecting`/`Exited` werden auf `badge_success`/`badge_warning`/
`badge_error` abgebildet. Die drei Picker-Zwecke und ihre Geometrie bleiben
unverändert.

Inventarorte: generische Popup-Pfade in `src/tui/mod.rs`, alle Form-/Picker-
Screens sowie die direkten Cyan/Yellow/White-Stile in `host_form.rs` und
`keychain.rs`.

**Drei Familien sind bewusst getrennt, weil ihre Altwerte es sind.** Die
Fuzzy-Palette tippt in `text_highlight`, der Session-Picker und der Tag-Filter
tippen eine Nuance dunkler in `text_bright` — eine einzige `query`-Rolle könnte
nur eine der beiden Zellen reproduzieren, also hat die Palette mit
`components.command_palette.query` eine eigene. Ebenso markiert das
Gruppen-Formular sein aktuelles Feld, indem es Label **und** Wert aufhellt und
fettet, während das Host-Formular mit Cyan/Gelb-Akzenten arbeitet; beide teilen
sich deshalb keine `form.*`-Fokusrollen, sondern das Gruppen-Formular bringt
`components.group_form.*` mit. Und `components.table.row_selected` hebt mit
`selection_fg` hervor, nicht mit `text_highlight`: die einzige Fläche, die die
generische Tabellenfamilie rahmt, ist die Gruppenliste, und die hat immer
`theme::selected()` benutzt.

### Help und Keybindings

| Rollenpfad | Typ | Semantischer Fallback |
| --- | --- | --- |
| `components.help.section` | `Style` | `text_bright` |
| `components.help.key` | `Style` | `text_bright` |
| `components.help.description` | `Style` | `text` |
| `components.keybind.row` | `Style` | `text` |
| `components.keybind.row_selected` | `Style` | `text_highlight` auf `selection_bg` |
| `components.keybind.marker` | `Style` | `text_highlight` auf `selection_bg` |
| `components.keybind.value` | `Style` | `text_muted` |
| `components.keybind.value_bound` | `Style` | `success` |
| `components.keybind.value_capturing` | `Style` | `warning` |

### Tabs, Tunnel und SFTP

| Rollenpfad | Typ | Semantischer Fallback |
| --- | --- | --- |
| `components.tabs.active` | `Style` | `text_inverse` auf `text_bright` |
| `components.tabs.inactive` | `Style` | `text_muted` |
| `components.tabs.separator` | `Paint` | `text_dim` |
| `components.tunnel.running` | `Color` | `success` |
| `components.tunnel.stopped` | `Color` | `error` |
| `components.tunnel.retrying` | `Color` | `warning` |
| `components.tunnel.connecting` | `Color` | `connecting` |
| `components.tunnel.unknown` | `Color` | `unknown` |
| `components.tunnels.summary` | `Style` | `text_muted` |
| `components.tunnels.table_header` | `Style` | `text_bright` mit Bold |
| `components.tunnels.separator` | `Paint` | `text_dim` |
| `components.tunnels.row` | `Style` | `text` |
| `components.tunnels.row_selected` | `Style` | `selection_fg` auf `selection_bg` |
| `components.tunnels.direction` | `Style` | `info` |
| `components.tunnels.remote` | `Style` | `text_muted` |
| `components.tunnels.metadata` | `Style` | `text_dim` |
| `components.tunnels.notice` | `Style` | `warning` |
| `components.tunnels.error` | `Style` | `error` |
| `components.tunnels.empty` | `Style` | `text_dim` |
| `components.sftp.local` | `Style` | `info` |
| `components.sftp.remote` | `Style` | `info` |
| `components.sftp.selection` | `Style` | `selection_fg` auf `selection_bg` |
| `components.sftp.search` | `Style` | `text_inverse` auf `warning` |
| `components.sftp.queue_download` | `Style` | `success` |
| `components.sftp.queue_upload` | `Style` | `warning` |
| `components.sftp.progress` | `Style` | `warning` |
| `components.sftp.progress_complete` | `Style` | `success` |
| `components.sftp.progress_remaining` | `Style` | `text_dim` |
| `components.sftp.notice` | `Style` | `warning` |
| `components.sftp.queue_header` | `Style` | `text_bright` mit Bold |
| `components.sftp.panel.border` | `Paint` | `border` |
| `components.sftp.panel.border_focused` | `Paint` | `border_focus` |
| `components.sftp.panel.title` | `Style` | `text_bright` |
| `components.sftp.panel.count` | `Style` | `text_dim` |
| `components.sftp.panel.background` | `Paint` | `surface` |

Der Zoom-Toast — der schwebende Chip, der die Notiz-Fläche der Statuszeile
ersetzt, solange ein Panel gezoomt ist — besitzt mit
`components.status_bar.toast` eine eigene Rolle. Er war seit jeher
`theme::cyan()` **plus** `REVERSED`; die Farbe liefert der `info`-Fallback, die
Invertierung steht als dokumentierter `default`-Override in
`assets/themes/default.toml`, damit ein Theme sie abschalten kann, statt sie im
Renderer festzuschweißen.

`components.sftp.queue_header` reproduziert den `theme::heading()`-Aufruf der
Queue-Kopfzeile; ohne eigene Rolle hätte diese Zelle ihre Fettung verloren,
genau wie es bei `popup.title` und `help.section` bereits passiert war.
`components.audit.table_header` bekam aus demselben Grund `text_bright` **mit
Bold** — die Audit- und die Tunnel-Spaltenköpfe stammen aus demselben
`theme::heading()`-Aufruf und dürfen nicht auseinanderlaufen.
`components.broadcast.pending` fällt auf `text_muted` zurück statt auf
`unknown`: eine noch nicht erreichte Zielmaschine war immer `theme::mute()`,
nicht der gedimmte Unbekannt-Ton.

`components.selection.inactive` markiert die ausgewählte Zeile des **nicht**
fokussierten SFTP-Panels. Vor der Migration war diese Zeile gar nicht
hervorgehoben (`active = is_sel && focused`), sodass die Cursorposition genau in
dem Panel unsichtbar war, in das der Nutzer als Nächstes zurückspringt. Der
semantische Fallback (`text` auf `surface_raised`) kann das **nicht** heilen,
weil `surface_raised` in SSHub der Terminalgrund selbst ist — die Zeile sähe
weiterhin aus wie jede andere. `default` schreibt die Rolle deshalb explizit
aus: derselbe Selektionsbalken wie im fokussierten Panel, aber mit
`text_muted` statt `selection_fg`. Das ist die **erste** der beiden bewussten
Verbesserungen gegenüber dem eingefrorenen Aussehen (Abweichungsklasse 3 oben)
und steht als solche in `DEFAULT_PARITY_OVERRIDES`. Der Pfeilmarker bleibt dem fokussierten Panel
vorbehalten, damit eindeutig bleibt, wohin Tastendrücke gehen.

Der SFTP-Queue-Hinweis trägt `components.sftp.notice` in **beiden**
Queue-Zuständen. Bei gefüllter Queue steckte derselbe Warntext bisher in der
`theme::heading()`-Kopfzeile, war also genau dann Chrome, wenn er am wichtigsten
war; Kopfzeile und Hinweis werden jetzt getrennt gezeichnet. Das ist die
**zweite** bewusste Verbesserung derselben Klasse, ebenfalls dokumentiert. `components.separator.secondary` zeichnet die inneren
Trennlinien: den Teiler zwischen Hostliste und Ausgabe im Broadcast-Zoom sowie
die Linie unter den Audit-Spaltenköpfen.

Die Statusfarben des Audit-Tabs stammen aus `components.audit.*`, die Note-Zeile
ebenfalls: sie war immer nach dem Status des Ereignisses eingefärbt, deshalb
liefert `components.audit.note` alles außer dem Vordergrund, und der kommt aus
`components.audit.{success,warning,error,unknown}`.

`components.tunnel.*` beschreibt einzelne Tunnelzustände.
`components.tunnels.*` beschreibt dagegen das Chrome des vollständigen
Tunnel-Tabs in `src/tui/screens/tunnels.rs`; insbesondere bleibt dessen
ausgewählte Zeile beim bisherigen `selection_fg` statt `text_highlight`.

### Identity-Karten

| Rollenpfad | Typ | Semantischer Fallback |
| --- | --- | --- |
| `components.identities.empty` | `Style` | `text_dim` |
| `components.identities.card.border` | `Paint` | `border` |
| `components.identities.card.border_selected` | `Paint` | `accent` |
| `components.identities.card.selection` | `Style` | `selection_fg` auf `selection_bg` |
| `components.identities.card.name` | `Style` | `text_bright` mit Bold |
| `components.identities.card.text` | `Style` | `text` |
| `components.identities.card.metadata` | `Style` | `text_dim` |
| `components.identities.card.key_type` | `Style` | `text_muted` |
| `components.identities.card.loaded` | `Color` | `success` |
| `components.identities.card.missing` | `Color` | `unknown` |
| `components.identities.card.credential` | `Color` | `warning` |
| `components.identities.agent.separator` | `Paint` | `text_dim` |
| `components.identities.agent.label` | `Style` | `text_muted` |
| `components.identities.agent.value` | `Style` | `text` |
| `components.identities.agent.count` | `Style` | `text_bright` |
| `components.identities.notice` | `Style` | `warning` |

Diese Rollen gehören zum Kartenraster des Identities-Tabs in
`src/tui/screens/keys.rs`. Das zeilenbasierte Identity-Form-Popup in
`src/tui/screens/keychain.rs` ist dagegen ein Formular und verwendet
`components.form.*`, `components.focus.indicator` und `components.popup.*`;
beide Oberflächen teilen keine impliziten Tabellenrollen. Eine eigene
`components.keychain.*`-Familie gibt es nicht: die Identitätsliste und die
Notice-Zeile, die sie gelesen hätten, wurden vom Kartenraster ersetzt, und
Rollen ohne produktiven Leser gehören nicht in den Katalog. Aus demselben Grund
besitzt `components.table.*` nur `row` und `row_selected` — `header` hing allein
am ersetzten Gruppenbaum, `border` hatte nie einen Leser.

### Audit und Broadcast

| Rollenpfad | Typ | Semantischer Fallback |
| --- | --- | --- |
| `components.audit.{success,warning,error,unknown}` | `Color` | `success`, `warning`, `error`, `unknown` |
| `components.audit.filter_active` | `Style` | `text_inverse` auf `text_bright` |
| `components.audit.filter_inactive` | `Style` | `text_dim` |
| `components.audit.note` | `Style` | `text_muted` |
| `components.audit.table_header` | `Style` | `text_bright` mit Bold |
| `components.audit.row` | `Style` | `text` |
| `components.audit.row_selected` | `Style` | `selection_fg` auf `selection_bg` |
| `components.broadcast.pending` | `Color` | `text_muted` |
| `components.broadcast.running` | `Color` | `warning` |
| `components.broadcast.success` | `Color` | `success` |
| `components.broadcast.error` | `Color` | `error` |
| `components.broadcast.stdout` | `Style` | `text_muted` |
| `components.broadcast.stderr` | `Style` | `error` |
| `components.broadcast.detail` | `Style` | `text_dim` |
| `components.broadcast.countdown` | `Style` | `info` |
| `components.broadcast.panel.border` | `Paint` | `border` |
| `components.broadcast.panel.border_focused` | `Paint` | `border_focus` |
| `components.broadcast.panel.title` | `Style` | `text_bright` |
| `components.broadcast.panel.count` | `Style` | `text_dim` |
| `components.broadcast.panel.background` | `Paint` | `surface` |

### Startanimation

| Rollenpfad | Typ | Semantischer Fallback |
| --- | --- | --- |
| `components.animation.background` | `Paint` | `background` |
| `components.animation.node` | `Style` | `success` |
| `components.animation.node_label` | `Style` | `text` |
| `components.animation.spoke` | `Style` | `text_dim` |
| `components.animation.hub_early` | `Style` | `success` |
| `components.animation.hub_ready` | `Style` | `text_bright` mit Bold |
| `components.animation.hub_label` | `Style` | `text_muted` |
| `components.animation.halo` | `Paint` | `selection_bg` |
| `components.animation.hub_flash` | `Style` | `warning` |
| `components.animation.wordmark` | `Style` | `text_bright` mit Bold |
| `components.animation.wordmark_accent` | `Style` | `warning` |
| `components.animation.tagline` | `Style` | `text_muted` |
| `components.animation.tagline_accent` | `Style` | `warning` |
| `components.animation.quip` | `Style` | `text_dim` |
| `components.animation.prompt_key` | `Style` | `text_bright` |
| `components.animation.prompt_text` | `Style` | `text_muted` |
| `components.animation.cursor` | `Style` | `success` |

`src/tui/animation.rs` ist die maßgebliche Inventarquelle. Blit und Tween
besitzen keine eigenen sichtbaren Rollen, müssen aber den aufgelösten
Hintergrund der animierten Zielrolle verwenden.

`hub_early` trägt die beiden frühen Hub-Glyphen (`·`, `+`), `hub_ready` die
beiden fertigen (`◆`, `◉`). `hub_flash` ist **nicht** die vierte Glyph-Stufe,
sondern das pulsierende Wort `hub` nach Ende der Animation; solange der Hub noch
entsteht, trägt dasselbe Wort `hub_label`. Diese vier Zellen waren vorher
`GREEN`, `BRIGHT+BOLD`, `AMBER+BOLD` und `MUTE` — ohne eigene `hub_label`-Rolle
hätte das ruhige Wort die Farbe des pulsierenden übernommen.

`hub_ready` und `wordmark` bekommen ihr Gewicht aus dem Katalogrezept
`text_bright mit Bold`; für `hub_flash` und `wordmark_accent` gibt es kein
„warning mit Bold"-Rezept, deshalb steht dort nur der Modifier als
`default`-Override in `assets/themes/default.toml` — dieselbe Form wie bei
`popup.title`.

### OS-Logos

| Rollenpfad | Typ | Semantischer Fallback |
| --- | --- | --- |
| `components.os_logo.tint` | `Tint` | `"native"` |

Die RGB- und ANSI-16-Farben aus den eingebetteten Distro-Logo-Assets bleiben
bei `"native"` absichtlich unverändert. Ein gesetzter Tint darf sie einfarbig
überschreiben.

### Bewusste Grenzen und direkte Farbmigration

Folgende direkte Farben werden in die genannten Rollen migriert:

- Host-/Keychain-/Detail-/Statusbar-ANSI-Farben in `host_form.rs`,
  `keychain.rs`, `widgets/detail_panel.rs` und `widgets/status_bar.rs`
  (`screens/hosts.rs` wurde stattdessen entfernt: der Gruppenbaum darin hatte
  keinen Aufrufer mehr)
- rote/gelbe Popup- und Confirm-Stile in `src/tui/mod.rs`
- schwarzer SFTP-Suchtext in `src/tui/screens/sftp.rs`
- feste `theme::BG`-/`BRIGHT`-Annahmen in Blit, Opaque-Background und
  Ping-Flash

**Drei Zellen der beiden Formular-Popups hatten gar keine Farbe** und bekommen
mit der Migration bewusst eine. Sie fallen *nicht* unter die ANSI-Ausnahme oben,
sondern sind hier einzeln als beschlossene Abweichung festgehalten:

- der **Titel** von Host- und Identity-Formular war ein ungestylter
  `Block::title(...)`, also die Vordergrundfarbe des Terminals. Er verwendet
  jetzt wie jedes andere Overlay `components.popup.title`; ein ungestylter Titel
  auf einem gethemten Popup-Grund wäre weder lesbar garantiert noch überhaupt
  themebar.
- der **Wert eines nicht aktiven Feldes** war `Style::default()`. Er verwendet
  jetzt `components.form.value`, also dieselbe Fließtextrolle, die der Rest der
  Anwendung für genau diese Art Text schon benutzt.
- die **Tastenhinweise** trugen nur `Modifier::DIM` ohne Farbe. Der reine
  DIM-Modifier ist terminalabhängig und wird von mehreren Emulatoren ignoriert;
  `components.form.help` drückt dieselbe Absicht als Farbe aus, die das Theme
  kontrolliert.

Das Tunnel-Formular (`render_tunnel_form` in `src/tui/screens/tunnels.rs`)
bringt mit `components.tunnel_form.*` eine **eigene** Familie mit. Es ist
weder ein zweiter Leser von `components.form.*` (dessen Fokus-Idiom Cyan/Gelb
ist) noch von `components.group_form.*` (dessen fokussierte Zellen fett sind):
das Tunnel-Formular hellt sein aktives Label nur auf (`theme::bright()`, ohne
Bold) und unterstreicht den Wert in `theme::WHITE` (ohne Bold). Eine der beiden
bestehenden Familien zu verwenden wäre eine stille Regression genau der Art,
die die Overlay-Runden gekostet haben. Der eingebettete Host-Picker ist dagegen
eine gewöhnliche `picker.*`-Oberfläche und verwendet
`components.picker.{border,query,row,row_selected}`, `components.popup.title`,
`components.popup.legend`, `components.separator.secondary` und
`components.text.muted`.

Der **Titel** des Tunnel-Formulars verwendet bewusst `components.tunnel_form.title`
mit Fallback `accent` und **nicht** `components.popup.title`. Er war ein
ungestyltes `Block::title(..)` über einem `theme::ACCENT`-Rahmen, und ratatui
zeichnet einen ungestylten Titel im Rahmenstil — die Zelle war also `ACCENT`
ohne Modifier, nicht `theme::heading()`. Er fällt damit *nicht* unter die
Ausnahme für den Host- und Identity-Formulartitel weiter oben: dort war die
Zelle wirklich farblos, hier hatte sie eine Farbe, nur eine geerbte. Eine
eigene Rolle statt weiterhin implizit zu erben, weil genau diese implizite
Vererbung die Abweichung unsichtbar gemacht hat.

`components.dashboard.details.field_marker` ist der `> `-Cursor vor dem gerade
bearbeiteten Feld des Detail-Panels. Er fällt **nicht** unter
`components.focus.indicator`: diese globale Rolle gilt laut Katalog nur dort, wo
der Marker an einem direkt in ANSI eingefärbten Label klebte (Host- und
Identity-Formular). Die ganze Editierzeile des Detail-Panels war ein
ungestyltes `Line::from(String)`, also **gar keine Farbe** — dritte bewusst
festgehaltene Abweichung: die eine Zelle, die sagt, wo Tastendrücke landen, ist
jetzt themebar und unter `default` sichtbar (`accent` gegen den ungefärbten
Rest der Zeile).

Der Theme-Picker selbst (`src/tui/screens/theme_picker.rs`) zeichnet sein
Chrome aus `components.popup.{title,hint,legend,error,background,border}`,
`components.picker.{row,row_selected,badge_success,badge_warning,badge_error}`
und `components.text.muted` — und zwar aus dem **live vorschauten** Theme, weil
die Navigation dieses Theme ohnehin schon auf die ganze TUI anwendet.

Absichtlich nicht vom Theme überschrieben werden:

- sämtliche ANSI-/RGB-Farben des echten Remote-PTY-Inhalts
- native Distro-Logo-Farben, solange `os_logo.tint = "native"` gilt
- reine Test-Fixtures
- unbemalte temporäre `Color::Reset`-Bufferzellen

`"terminal"` im Theme und eine unbemalte interne `Color::Reset`-Zelle sind
verschiedene Zustände. Highlight-Wipes, Fades und Blits stellen bei internen
Reset-Zellen den aufgelösten Hintergrund der Zielrolle wieder her, statt
blind `app.background` zu verwenden.

## Eingebaute Themes

Die eingebauten Themes liegen als echte TOML-Assets im Repository und werden mit
`include_str!` in das Binary eingebettet. Dadurch verwenden Built-ins und
Benutzer-Themes exakt denselben Parser, Resolver und Validator.

V1 enthält:

### `default`

Exakte visuelle Abbildung der heutigen Konstanten aus `src/tui/theme.rs`.
Keine überraschende Erscheinungsänderung nach dem Upgrade.

### `summer`

Helles, warmes Theme mit Cremeflächen, Sonnengelb und Weiß. Sanfte gelb-weiße
Rahmen- und Trennlinienverläufe demonstrieren helle Themes, Multi-Stop-Verläufe
und komponentenspezifische Overrides.

### `aqua`

Tiefes Meeresblau als Grundfläche, Türkis und Cyan als Akzente. Horizontale und
umlaufende Blau-Wasser-Verläufe demonstrieren dunkle Themes und
`perimeter`-Rahmen.

### `fire`

Dunkle Kohleflächen mit Rot-, Orange- und Goldverläufen. Semantische
Statusfarben bleiben trotzdem unterscheidbar; Dekoration darf Statusbedeutung
nicht ersetzen.

### `high-contrast`

Sehr klare Vorder-/Hintergrundkombinationen ohne dekorative Verläufe. Dieses
Theme demonstriert, dass alle Gradienten optional sind.

`summer`, `aqua` und `fire` werden bewusst ausführlich kommentiert und dienen
zusammen mit der README-Dokumentation als kopierbare Referenzen. Alle Built-ins
außer `default` erben von `default` und setzen nur ihre tatsächlichen
Abweichungen. Über `sshub theme show <id>` kann ihr eingebetteter TOML-Quelltext
ohne Repository-Checkout ausgegeben und unter einer neuen ID gespeichert werden.

## Runtime-Architektur

### Datenmodelle

Das Theme-System trennt vier Zustände:

1. `ThemeDefinition`: positionsbehaftet deserialisierte TOML-Struktur mit
   optionalen Werten und gesondert erfassten unbekannten Komponentenrollen
2. `ResolvedTheme`: vollständig vererbtes und validiertes Runtime-Theme
3. `ThemeRegistry`: eingebaute und gefundene Benutzer-Themes samt Diagnosen
4. `ThemeManager`: aktives, gespeichertes und temporär vorgemerktes Theme

`ResolvedTheme` ist ein typisierter Rust-Struct, keine String-Map. Sein
semantischer Kern und jede Komponentenrolle sind nach Merge und Fallback
non-optional. Es enthält keine ungeklärten Strings mehr. Farben sind
`Color::Rgb` oder bewusst `Color::Reset`, Modifikatoren sind Ratatui-Werte und
Verläufe enthalten vollständig aufgelöste RGB-Stopps.

Parser und Resolver sind in CLI und Runtime identisch; nur die
Validierungsrichtlinie unterscheidet sich. Der Checker läuft im Modus `Strict`
und macht jede unbekannte Rolle zum Fehler. Die Runtime läuft im Modus
`Compatible` und stuft ausschließlich unbekannte Komponentenrollen zu
nicht fatalen Diagnosen herab. Unbekannte Top-Level-Felder, Sections,
semantische Rollen, Style-Felder und Wertformen bleiben in beiden Modi Fehler.

### Kein veränderlicher globaler Theme-Zustand

Das aktive `ResolvedTheme` liegt als `Rc<ResolvedTheme>` im `App`-Zustand.
Renderer greifen über `app.theme()` darauf zu; nur die wenigen App-losen
Render-Helfer erhalten `&ResolvedTheme` explizit. Die bisherigen parameterlosen
`theme::text()`- und Konstanten-Zugriffe werden schrittweise durch Methoden des
Runtime-Themes ersetzt. `const`-Style-Tabellen wie `HUB_STAGES` in
`src/tui/animation.rs` werden dabei zu Theme-abhängigen Funktionen.

Das vermeidet ein globales `RwLock` und bietet:

- deterministische parallele Tests
- keine versteckten globalen Seiteneffekte
- atomaren Austausch eines vollständig validierten Themes
- klare Abhängigkeiten im Render-Code

Ein Theme-Wechsel – auch eine temporäre Vorschau – invalidiert alle gehaltenen
Buffer-Snapshots und beendet laufende Blit-/Slide-Übergänge. Dadurch werden
keine Zellen des alten Themes über einen neuen Frame kopiert.

### Registry und Auflösung

Beim Start:

1. Eingebaute Assets sind bereits als Build-/Test-Invariante validiert und
   werden aus den eingebetteten Definitionen registriert.
2. `themes/*.toml` lexikografisch einlesen.
3. Jede Datei syntaktisch und strukturell validieren.
4. Vererbung und Referenzen auflösen.
5. Das konfigurierte `active_theme` aktivieren.

Ist das aktive Theme nicht vorhanden oder ungültig, startet SSHub mit `default`
und zeigt einen nicht fatalen Hinweis. `config.toml` wird dabei nicht automatisch
überschrieben; nach einer Reparatur kann der Anwender das gewünschte Theme wieder
laden.

Eine Benutzerdatei mit reservierter Built-in-ID wird als ungültig gelistet und
überschreibt niemals das eingebaute Theme. Die Diagnose nennt eine konkrete
neue ID wie `aqua-custom`.

## Gradient-Rendering

`tui-gradient-block` wird nicht direkt eingebunden. Die veröffentlichte Version
`0.1.3` basiert auf Ratatui `0.29`, während SSHub Ratatui `0.30` verwendet.
Direkte Nutzung würde inkompatible doppelte Ratatui-Typen in den Build bringen.
Außerdem soll SSHub nicht nur `Block`, sondern auch beliebige Trennlinien,
Footer-Elemente und Session-Bereiche mit Verläufen darstellen können.

Statt eigener Widget-Nachbauten erhält SSHub eine kleine native
Buffer-Nachbearbeitung auf Basis seiner aktuellen Ratatui-Version:

- `GradientSampler` löst eine relative Position in eine RGB-Farbe auf.
- `paint_gradient_ring` färbt den äußeren Zellring eines bereits gerenderten
  Blocks.
- `paint_gradient_line` färbt Trennlinien, Titel oder Footer-Segmente.
- `paint_gradient_area` färbt Vorder- oder Hintergrund einer Fläche.
- Standard-Widgets rendern zunächst mit einer soliden Fallback-Farbe; nur eine
  konfigurierte Gradient-Rolle löst anschließend eine Nachbearbeitung aus.

Damit bleiben Rahmenglyphen und der gesamte Solid-Color-Pfad exakt bei
Ratatui. Die Helfer arbeiten auf geclippten Rechtecken und färben nur Zellen,
die zur jeweiligen Rolle gehören.

`paint_gradient_area` für `components.app.background` verändert nur bislang
unbemalte `Color::Reset`-Zellen in SSHub-eigenen Flächen und erhält eine
Exclusion-Region für den Remote-PTY-Viewport. Ein naiver Vollbild-Pass über
bereits gerenderte PTY-Zellen ist unzulässig.

### Richtungssemantik

Alle Koordinaten beziehen sich auf das Rechteck der jeweiligen Komponente, nicht
auf den gesamten Bildschirm. Es gilt:

```text
norm(v, n) = 0.0, wenn n <= 1
norm(v, n) = v / (n - 1), sonst

horizontal    = norm(x, width)
vertical      = norm(y, height)
diagonal_down =
  0.0                                           wenn width <= 1 und height <= 1
  norm(x, width)                                wenn height <= 1
  norm(y, height)                               wenn width <= 1
  (norm(x, width) + norm(y, height)) / 2        sonst
diagonal_up =
  0.0                                           wenn width <= 1 und height <= 1
  norm(x, width)                                wenn height <= 1
  1 - norm(y, height)                           wenn width <= 1
  (norm(x, width) + (1 - norm(y, height))) / 2  sonst
```

Eine Diagonale auf einem `N×1`-Rechteck degradiert damit über die vollständige
Farbskala zu einem horizontalen Verlauf. Auf `1×N` wird `diagonal_down`
vollständig vertikal und `diagonal_up` vollständig umgekehrt vertikal
abgebildet; es gibt keine Division durch null.

`perimeter` läuft den äußeren Ring im Uhrzeigersinn ab:

1. obere Reihe links nach rechts, inklusive beider Ecken
2. rechte Spalte ab der zweiten Zelle nach unten, inklusive rechter unterer Ecke
3. untere Reihe ab der vorletzten Zelle nach links, inklusive linker unterer Ecke
4. linke Spalte ab der vorletzten Zelle nach oben, ohne beide bereits gezählten Ecken

Für `width >= 2` und `height >= 2` ist die Länge
`2 * width + 2 * height - 4`; `t = index / (length - 1)`. Da erster und letzter
Stopp gleich auflösen müssen, schließt sich der Ring ohne Naht. `1×N` und `N×1`
werden als einfache Linie in ihrer natürlichen Richtung behandelt; `1×1`
verwendet `t = 0`.

### Performance

V1 besitzt bewusst keinen Gradient-Cache. Der Sampler arbeitet in einem
linearen Durchlauf, ohne Heap-Allokation pro Zelle und ohne mit der Frame-Historie
wachsenden Zustand. Ein Cache wird erst nach Profiling eingeführt.

Vor Abschluss wird ein Release-Benchmark mit einem `200×60`-`TestBackend`
dokumentiert. Die zusätzliche mediane Renderzeit durch Verläufe darf `2 ms` pro
Frame nicht überschreiten.

Der Nachweis erfolgt bewusst **nicht** über eine isolierte A/B-Messung desselben
Layouts mit Solid Colors — dafür müsste derselbe App-Zustand zweimal gemessen
werden, einmal mit Gradient-, einmal mit äquivalenten Solid-Paints, in
alternierender Reihenfolge; das ist für V1 nicht vorgesehen. Nachgewiesen wird
stattdessen die **obere Schranke aus der Gesamt-Frame-Zeit** des
Gradient-Themes: der Gradient-Pass läuft seriell innerhalb des Frames und kann
deshalb nicht mehr kosten als der ganze Frame. Liegt die mediane Frame-Zeit des
Gradient-Themes deutlich unter `2 ms`, ist das Kriterium erfüllt.

Zusätzlich wird als nicht isolierte Smoke-Beobachtung die Differenz zu einem
Theme ohne Verläufe protokolliert. Sie ist ausdrücklich **keine** Schranke: die
verglichenen Themes unterscheiden sich in allen Werten, die Messreihenfolge ist
fest, und eine negative Beobachtung wird auf null geklemmt. Der bestehende
50-ms-Event-Poll bleibt unverändert.

## Theme-Picker

### Einstieg

Der bestehende Settings-Dialog erhält eine Aktionszeile:

```text
Theme…    default
```

`Enter` auf dieser Zeile öffnet `AppMode::ThemePicker`. Die bisherige Annahme,
dass jede Settings-Zeile ein Boolean-Toggle ist, wird durch typisierte
Settings-Einträge ersetzt. Bestehende Toggle-Reihenfolge und Tastaturbedienung
bleiben erhalten.

### Layout

Der Theme-Picker ist ein zentriertes, responsives Overlay:

- links: Theme-Liste mit Name, Built-in/User-Kennzeichen und Zustand
  `valid`, `warning` oder `invalid`
- rechts: kompakte Vorschau aus zwei gerahmten Boxen
- unten: Beschreibung oder Validierungsfehler und Tastenlegende

Built-ins erscheinen in der festgelegten Reihenfolge `default`, `summer`,
`aqua`, `fire`, `high-contrast`; danach folgen Benutzer-Themes alphabetisch
nach Anzeigename und ID. Der Picker zeigt den Benutzer-Theme-Pfad an, damit
direkt erkennbar ist, wo neue Dateien angelegt werden.

Die Vorschau zeigt mindestens:

- normalen, hellen, gedimmten und markierten Text
- einen fokussierten und einen inaktiven Rahmen
- Box-Titel und Trennlinie
- aktive und inaktive Session-Tabs
- `up`, `warning` und `error` als Text plus Farbe
- eine ausgewählte Host-Zeile
- Footer-Key mit heller Taste und dunklerer Beschriftung
- Hintergrund-, Füll- und Padding-Farben
- konfigurierte Rahmen-/Linienverläufe

Auf schmalen Terminals werden Liste und Vorschau untereinander angeordnet. Ist
selbst dafür nicht genug Platz vorhanden, bleibt die Liste bedienbar und die
Vorschau wird mit einem klaren Größenhinweis ausgeblendet. Sämtliche
Buffer-Zugriffe werden gegen `frame.area()` geclippt.

### Interaktion

| Taste | Verhalten |
| --- | --- |
| `↑` / `↓` | Theme auswählen, umlaufend |
| `PageUp` / `PageDown` | Seitenweise navigieren |
| `Home` / `End` | Ersten/letzten Eintrag wählen |
| `Enter` | Ausgewähltes gültiges Theme speichern und schließen |
| `r` | Built-ins und Benutzerdateien neu laden |
| `Esc` | Vorheriges Theme wiederherstellen und schließen |

Beim Bewegen der Auswahl wird das gültige Theme sofort temporär auf die gesamte
SSHub-Oberfläche und die Vorschau angewandt. Das macht den tatsächlichen Eindruck
sichtbar, ohne `config.toml` zu verändern.

Ungültige Themes bleiben in der Liste sichtbar, erhalten einen Fehlerindikator
und zeigen ihre Diagnosen im unteren Bereich. Sie werden niemals als Vorschau
aktiviert und `Enter` ist für sie wirkungslos.

Themes mit unbekannten Komponentenrollen sind im Runtime-Modus gültig mit
Warnungen: Sie dürfen als Vorschau aktiviert und gespeichert werden, tragen
aber einen sichtbaren Warnindikator und zeigen alle ignorierten Rollen im
Diagnosebereich. Beim Start wird dieselbe Kompatibilitätsdiagnose über den
nicht fatalen Notice-Kanal angezeigt; ein Tippfehler bleibt damit nicht still.

Beim Reload mit `r` bleibt das zuletzt gültige Theme aktiv. Ist die gerade
bearbeitete Datei nun gültig, aktualisieren sich Vorschau und Diagnose sofort.
Automatisches File-Watching ist nicht Bestandteil von V1.

Der Picker verwaltet drei IDs getrennt:

- `saved_id`: aktuell in `config.toml` gespeicherte ID
- `original_id`: beim Öffnen tatsächlich aktives, gültiges Theme
- `preview_id`: aktuell ausgewähltes und temporär dargestelltes Theme

Zustandsregeln:

- Navigation ändert ausschließlich `preview_id` und den Runtime-`Rc`.
- `Esc` aktiviert `original_id`; fehlt dieses nach einem Reload, wird das beim
  Öffnen aufgelöste `Rc<ResolvedTheme>` wieder eingesetzt.
- `Enter` schreibt nur dann `active_theme`, wenn `preview_id` gültig ist.
- Scheitert das Speichern, bleibt der Picker offen, `saved_id` unverändert und
  die Fehlermeldung sichtbar.
- Wird die Preview-Datei bei `r` gelöscht oder ungültig, bleibt das zuletzt
  gültige Runtime-Theme aktiv und die Auswahl springt auf den nun ungültigen
  beziehungsweise entfernten Eintragsplatz, soweit dieser noch darstellbar ist;
  andernfalls auf `original_id`.
- Weder Öffnen, Navigation, `r` noch `Esc` schreiben `config.toml` oder irgendeine
  Theme-Datei. Nur erfolgreiches `Enter` schreibt die aktive Theme-ID.

## Theme-CLI und Validator

### Aufruf

```text
sshub theme check <file> [--format plain|json]
sshub theme list [--format plain|json]
sshub theme show <id> [--resolved] [--format toml|json]
```

Alle Theme-Befehle laufen headless und starten weder TUI noch Datenbank. Sie
werden in `main.rs` vor dem normalen `is_subcommand`-/`CliContext::bootstrap`-
Pfad dispatcht; andernfalls würden Metadaten- und Launcher-Datenbank unnötig
geöffnet. CLI-Hilfe, Shell-Completions und Smoke-Tests werden mit aktualisiert.

`theme list` zeigt IDs, Anzeigenamen, Quelle und Validierungsstatus.
`theme show` gibt den eingebetteten beziehungsweise installierten TOML-Quelltext
aus. `--resolved` erzeugt ein vollständiges, wieder einlesbares TOML ohne
explizites `extends` oder ungelöste Referenzen. Beim Wiedereinlesen einer
Benutzerdatei bleibt der implizite `default`-Parent wirkungsgleich, weil der
Export alle Werte explizit ausschreibt. `--format json` liefert dieselben Felder
strukturiert. Der aufgelöste TOML-Output muss den Theme-Parser im Round-Trip
bestehen. Damit ist der dokumentierte Kopier-Workflow ausführbar:

```text
sshub theme show aqua > ~/.config/sshub/themes/aqua-custom.toml
sshub theme check ~/.config/sshub/themes/aqua-custom.toml
```

Der Quelltext-Export beginnt mit einem TOML-Kommentar, der die Ursprungs-ID
nennt und beim Kopieren daran erinnert, den sichtbaren `name` anzupassen. Der
Kommentar beeinflusst den Round-Trip nicht und verhindert zwei gleich benannte
Einträge im Picker nicht technisch, macht den notwendigen Schritt aber direkt
im erzeugten Ausgangsdokument sichtbar.

`theme check` und `theme list` folgen der bestehenden CLI-Konvention und
unterstützen zusätzlich `--format plain|json`. Der Validator verwendet denselben
Parser und Resolver wie die Anwendung.

Beim Runtime-Start existiert genau eine Registry aus:

1. reservierten eingebauten Themes
2. Benutzerdateien im einen konfigurierten Theme-Verzeichnis

Eine Benutzer-ID, die mit einer Built-in-ID kollidiert, ist immer ein Fehler;
es gibt kein First-wins-Verhalten. Da die ID dem Dateinamen entspricht, kann
eine ID innerhalb desselben Verzeichnisses nur einmal vorkommen.

Für `theme check <file>` bildet das Verzeichnis der geprüften Datei anstelle des
installierten Benutzerverzeichnisses die Registry. Dadurch können portable
Theme-Pakete mit Geschwister-Parents gemeinsam geprüft werden. Wird ein Parent
aus einer Geschwisterdatei aufgelöst, weist eine Warnung darauf hin, dass diese
Datei zusammen mit dem Kind installiert werden muss.

`extends` ist immer eine Theme-ID, niemals ein relativer oder absoluter Pfad.
Damit kann Vererbung das Theme-Verzeichnis nicht verlassen.

### Validierungsstufen

1. Datei lesbar und UTF-8
2. gültige TOML-Syntax
3. unterstützte `schema_version`
4. ausschließlich bekannte Top-Level-Felder und Sections
5. ausschließlich bekannte semantische Rollen, Komponentenrollen und Style-Felder
6. gültige Farbformate, Zahlenbereiche und Modifikatoren
7. gültige Gradient-Richtungen und Stopps
8. auflösbare Palette-, Gradient- und Elternreferenzen
9. zyklenfreie Theme- und Farbreferenzen
10. nach Definition-Merge und semantischen Fallbacks vollständig aufgelöstes Runtime-Theme

Im Checker sind unbekannte Keys und Rollen Fehler, keine Warnungen. Bei
hinreichend ähnlichen bekannten Keys erscheint ein Vorschlag:

```text
themes/ocean.toml:28:5 error: unknown key `bordr`
  help: did you mean `border`?
```

Dieselbe Vorschlagslogik kennt die reservierten Sentinel-Werte `"auto"`,
`"terminal"` und `"native"`. Tippfehler wie `"atuo"` oder `"termnial"` erhalten
deshalb einen konkreten `did you mean`-Hinweis statt nur einer generischen
Meldung über einen unqualifizierten String.

Wenn der TOML-Parser eine Position liefert, enthalten Meldungen Datei, Zeile und
Spalte. Semantische Diagnosen enthalten mindestens den vollständigen Feldpfad.
Entsteht die Ursache über Vererbung oder Referenzauflösung, nennt die Diagnose
zusätzlich Quelldatei, Theme-ID und aufgelösten Ursprungswert, beispielsweise:
`semantic.surface resolves via default to terminal; opacity requires opaque RGB`.
Mehrere voneinander unabhängige Fehler werden in einem Lauf gesammelt, statt nur
den ersten Fehler auszugeben.

V1 sammelt unabhängige strukturelle und semantische Fehler, enthält aber noch
keine automatische Kontrastbewertung. Kontrastanalyse kann später als
optionaler Prüfschritt ergänzt werden.

### Exit-Codes

| Code | Bedeutung |
| --- | --- |
| `0` | Theme gültig, gegebenenfalls mit Warnungen |
| `1` | Validierungs-/Dateifehler oder unbekannte Theme-ID bei `show` |
| `2` | falsche CLI-Verwendung |

`theme list` liefert bei erfolgreich gelesener Registry immer `0`, auch wenn
einzelne gelistete Themes `warning` oder `invalid` sind; deren Zustand steht in
der Ausgabe. I/O-Fehler der Registry liefern `1`. Unbekannte Optionen,
fehlende Pflichtargumente und inkompatible Formatkombinationen liefern `2`.

Erfolgsbeispiel:

```text
OK: aqua-custom (extends aqua), 23 colors, 4 gradients, 61 overrides
```

## Fehler- und Fallback-Verhalten

- Ein ungültiges aktives Benutzer-Theme verhindert niemals den SSHub-Start.
- `default` ist im Binary eingebettet und steht ohne Dateisystemzugriff bereit.
- Ein Theme wird erst atomar aktiviert, nachdem es vollständig aufgelöst ist.
- Reload-Fehler ersetzen weder aktives Theme noch Preview mit Teilzuständen.
- Ein fehlgeschlagenes Speichern von `active_theme` wird als nicht fataler
  Hinweis angezeigt; die Runtime-Auswahl bleibt bis zum Beenden sichtbar.
- Fehler in einem Theme machen nicht alle anderen Benutzer-Themes unbrauchbar.
- Unbekannte Komponentenrollen werden zur Laufzeit einzeln ignoriert und als
  Kompatibilitätsdiagnose angezeigt; der strikte Checker bleibt rot.
- Unbekannte neue Schema-Versionen werden nicht bestmöglich geraten, sondern
  verständlich abgelehnt.
- Ein Theme darf durch extrem große Dateien oder Stopplisten nicht unkontrolliert
  Speicher belegen; Parser und Validator erhalten sinnvolle Obergrenzen.

Vorgesehene Obergrenzen für V1:

- maximal 256 Paletteneinträge
- maximal 128 Gradienten
- maximal 32 Stopps pro Verlauf
- maximal 1 MiB pro Theme-Datei
- maximale Vererbungstiefe 16
- maximal 256 Theme-Dateien im Benutzerverzeichnis
- maximale Farbreferenz-Tiefe 16

## Rückwärtskompatibilität

- Fehlt `appearance.active_theme`, wird `default` verwendet.
- Alle bisherigen `theme.rs`-basierten Rollen bleiben im `default`-Theme
  zellengenau erhalten; direkte verstreute ANSI-Stile werden wie dokumentiert
  auf semantische Rollen vereinheitlicht.
- Löst `components.app.background` zu einer echten Farbe oder einem Verlauf
  auf, bemalt das Theme alle noch `Color::Reset`-farbenen **SSHub-eigenen**
  Hintergrundzellen. Löst es zu `"terminal"` auf, findet keine Theme-Bemalung
  statt.
- `opaque_background` bleibt als Kompatibilitätsschalter: Ist er aktiv und
  `app.background` ist `"terminal"`, werden Reset-Zellen wie bisher mit
  `semantic.canvas` gefüllt. Er kann eine vom Theme ausdrücklich gesetzte
  App-Fläche nicht unterdrücken.
- Der Theme-getriebene App-Hintergrund schließt den Remote-PTY-Viewport
  ausdrücklich aus, auch wenn dessen Zellen `Color::Reset` tragen. Nur der vom
  Benutzer aktivierte Legacy-Schalter `opaque_background` darf dort weiterhin
  das bisherige Solid-Backdrop-Verhalten anwenden. Theme-Verläufe färben niemals
  Remote-PTY-Zellen.
- `"terminal"` löst zu `Color::Reset` auf. Bestehende Render-Passes dürfen
  `Color::Reset` jedoch nicht mehr blind als Panel-Hintergrund einsetzen:
  Highlight-Wipes, Fades und Blits stellen den aufgelösten Hintergrund der
  betroffenen Rolle wieder her.
- SSHub gibt RGB-Werte aus. Erkennt oder unterstützt ein Terminal True Color
  nicht, entscheidet dessen Farbreduktion; SSHub verspricht dort keine
  farbgetreue Darstellung, muss aber ohne Panic und mit lesbaren Statuswörtern
  funktionieren.
- Bestehende `config.toml`-Kommentare und unbekannte Einstellungen bleiben beim
  Speichern durch die vorhandene `toml_edit`-Merge-Logik erhalten.
- Bestehende Keybindings ändern sich nicht.
- Theme-Assets werden nicht aus dem Benutzerverzeichnis gelöscht oder
  automatisch umgeschrieben.

## Vorgesehene Modulgrenzen

Neu:

- `src/theme/mod.rs` — öffentliche Theme-Schnittstelle
- `src/theme/model.rs` — positionsbehaftete Definitionen und bekannte Rollen
- `src/theme/validate.rs` — strukturelle und semantische Diagnosen
- `src/theme/resolve.rs` — Vererbung, Referenzen und vollständige Auflösung
- `src/theme/registry.rs` — Built-ins und Benutzerdateien
- `src/theme/gradient.rs` — allokationsarmes Sampling und Buffer-Nachbearbeitung
- `src/cli/theme.rs` — `theme check`, `list` und `show`
- `src/tui/screens/theme_picker.rs` — Auswahl und Vorschau
- `assets/themes/default.toml`
- `assets/themes/summer.toml`
- `assets/themes/aqua.toml`
- `assets/themes/fire.toml`
- `assets/themes/high-contrast.toml`

Geändert:

- `src/config.rs` — `appearance.active_theme`
- `src/main.rs` und `src/cli/*` — Theme-Dispatch vor `CliContext::bootstrap`,
  Hilfe, JSON-Ausgabe und Completions
- `src/app/mod.rs` und `src/app/types.rs` — `ThemeManager` und Picker-Zustand
- `src/app/keys.rs` — Settings-Aktion und Theme-Picker-Tasten
- `src/tui/theme.rs` — Übergang von Konstanten zu Runtime-Zugriffen oder
  Aufgehen im neuen `src/theme`-Modul
- `src/tui/animation.rs` — konstante Style-Tabellen in Theme-abhängige
  Funktionen überführen
- `src/tui/blit.rs`, `src/tui/tween.rs` und Highlight-Wipes — Theme-Hintergrund
  und Snapshot-Invalidierung berücksichtigen
- `src/tui/widgets/panel_box.rs` — typisiertes Rollenbündel für normalen und
  fokussierten Rahmen, Titel, Count und Fläche statt fest kodierter Styles
- `src/tui/widgets/middle_stack.rs`, `right_stack.rs` sowie SFTP/Broadcast —
  jeden tatsächlichen Panel-Aufruf mit seinem eingefrorenen Rollenbündel
  verbinden
- sämtliche SSHub-Renderer mit Theme-Abhängigkeit
- `src/tui/mod.rs` und `src/tui/screens/mod.rs` — Render-Dispatch
- `README.md` und CLI-Hilfe — Benutzer- und Schema-Dokumentation

Die endgültige Dateiaufteilung darf während der Implementierungsplanung
zusammengezogen werden, sofern Datenmodell, Validator und Renderer getrennte
Verantwortlichkeiten behalten.

## Teststrategie

### Parser und Validator

- Minimales Theme ohne explizites `extends` erbt implizit von `default`.
- Jede unbekannte Section, semantische Rolle, Komponentenrolle und jedes
  Style-Feld ist im Checker ein Fehler.
- Typischer Tippfehler liefert einen passenden Vorschlag.
- Tippfehler bei `"auto"`, `"terminal"` und `"native"` schlagen den passenden
  reservierten Sentinel vor.
- Hex, RGB und `"terminal"` akzeptieren gültige Werte und lehnen ungültige ab.
- `brightness` und `opacity` akzeptieren nur ihre definierten Bereiche.
- `"terminal"` lehnt Helligkeit, Opacity und `over` ab.
- `semantic.background` lehnt `opacity` ab.
- Explizite und implizite `over`-Werte müssen zu einer opaken RGB-Farbe
  auflösen; `"terminal"` als Mischgrund wird abgelehnt.
- Farb-, Theme- und Vererbungszyklen werden erkannt.
- Fehlende Eltern, Farben und Gradienten werden mit Feldpfad gemeldet.
- Gradient-Stopps müssen vollständig, geordnet und begrenzt sein.
- `perimeter` wird an nicht geschlossenen Rollen abgelehnt.
- `perimeter` verlangt identische aufgelöste Start-/Endfarben.
- Die Role-to-Type-Matrix lehnt Gradienten auf `Color`/`Style`-Feldern ab.
- Obergrenzen werden getestet.
- Alle fünf eingebauten Assets bestehen denselben öffentlichen Validator.

### Auflösung

- Mehrstufige Vererbung überschreibt ausschließlich gesetzte Felder.
- Definitionen werden vor Referenzauflösung gemergt: Überschreibt das Kind
  `semantic.accent`, übernehmen geerbte Komponentenreferenzen die Kind-Farbe.
- `"auto"` stellt geerbte Rollen oder einzelne Style-Felder auf ihren
  semantischen Rust-Fallback zurück; `modifiers = []` leert die Liste.
- `"terminal"` ist in `foreground` und `background` eines `Style` gültig;
  `"auto"` setzt auch `Tint`-Rollen auf ihren Rust-Fallback zurück.
- Komponenten-Overrides verdrängen keine nicht genannten Elternwerte.
- Helligkeit wird vor simulierter Opacity angewandt.
- RGB-Mischung und Rundung sind an `0.0`, Zwischenwerten und `1.0` deterministisch.
- Jede Rollen-Matrixzeile wird gegen ihre bisherige Ist-Farbe beziehungsweise
  ihr Ist-Farbpaar geprüft. Eigene Tests sichern insbesondere Inverse-Stile,
  beide Selection-Varianten, Fokus- und Popup-Rahmen sowie bislang unbemalte
  Flächen ab.
- Das aufgelöste `default` entspricht den bisherigen `theme.rs`-Konstanten;
  die dokumentierten direkten ANSI-Ausnahmen entsprechen ihrer festgelegten
  semantischen Normalisierung.

### Gradient-Rendering

- Alle fünf Richtungen werden auf kleinen bekannten Rechtecken zellengenau geprüft.
- Der definierte `perimeter`-Pfad zählt jede Ecke genau einmal und schließt ohne Naht.
- `0×N`, `1×1`, `1×N` und `N×1` verursachen weder Panic noch Division durch null.
- Diagonalen nutzen auf `N×1`- und `1×N`-Rechtecken die vollständige Farbskala
  in der festgelegten horizontalen beziehungsweise vertikalen Richtung.
- Buffer-Nachbearbeitung verändert nur Zellen der Zielrolle.
- Solid-Color-Komponenten benutzen weiterhin den normalen schnellen Pfad.
- Strukturtests belegen: keine Heap-Allokation pro Zelle, linearer Durchlauf
  und kein mit der Frame-Historie wachsender Zustand.
- Der `2 ms`-Benchmark ist eine dokumentierte lokale Release-Messung und
  ausdrücklich kein flackernder Timing-Gate auf geteilten CI-Runnern.

### Theme-Picker

- Öffnen über Settings bewahrt das ursprünglich aktive Theme.
- Navigation wendet nur gültige Vorschauen an.
- `Esc` rollt auf das Ausgangs-Theme zurück.
- `Enter` persistiert exakt die gewählte ID.
- `r` übernimmt eine reparierte Datei.
- `r` mit Fehler bewahrt das letzte gültige Theme.
- Gelöschte oder ungültig gewordene Preview-Dateien folgen der definierten
  `saved_id`/`original_id`/`preview_id`-Zustandsmatrix.
- Ungültige Dateien sind sichtbar, aber nicht aktivierbar.
- Gültige Themes mit unbekannten Komponentenrollen erscheinen als `warning`,
  bleiben vorschau- und speicherbar und zeigen die ignorierten Rollen.
- Ein beim Start aktives Compatible-Theme bleibt aktiv; unbekannte
  Komponentenrollen werden über den nicht fatalen Notice-Kanal gemeldet.
- Öffnen, Navigation, Reload und `Esc` schreiben weder Theme-Dateien noch
  `config.toml`; nur erfolgreiches `Enter` persistiert.
- Theme-Wechsel invalidiert alte Buffer-Snapshots und beendet Blit-Übergänge.
- Breite und schmale Layouts schreiben nie außerhalb des Buffers.
- Die Vorschau enthält die vereinbarten Rollen und Gradienten.

### CLI

- `theme check` startet keine TUI und benötigt keine Datenbank.
- `theme list` und `theme show` starten ebenfalls ohne `CliContext::bootstrap`.
- Gültige Datei liefert Exit-Code `0`.
- Ungültige Datei liefert Exit-Code `1` und verständliche Diagnosen.
- Fehlendes Argument oder unbekannte Option liefert Exit-Code `2`.
- Plain- und JSON-Ausgabe sind maschinenlesbar und semantisch gleich.
- Die Command×Format-Matrix erlaubt nur `check/list × plain|json` sowie
  `show × toml|json`; jede inkompatible Kombination liefert Exit-Code `2`.
- `theme list` liefert trotz einzelner `warning`-/`invalid`-Einträge `0`, bei
  Registry-I/O-Fehlern `1`; `theme show <unbekannt>` liefert `1`.
- Built-in- und Sibling-Vererbung werden geprüft; Sibling-Parents erzeugen den
  Installationshinweis.
- `theme show aqua` liefert den eingebetteten Quelltext, `--resolved` ein
  vollständig aufgelöstes, wieder einlesbares Theme; TOML-Ausgabe besteht den
  Parser-Round-Trip.

### Regression

- Bestehende Unit-, E2E-, Config-, Smoke- und Dokumentationstests bleiben grün.
- Der Start ohne Theme-Verzeichnis erzeugt keine Fehlermeldung.
- Alle bisherigen `theme.rs`-basierten Default-Pfade bleiben semantisch und
  farblich unverändert; direkte ANSI-Ausnahmen folgen der dokumentierten
  Normalisierung.
- Ein Golden-Buffer-Test vergleicht eine repräsentative Dashboard-Ansicht vor
  und nach Runtime-Migration für alle bisherigen `theme.rs`-Pfade zellengenau;
  direkte ANSI-Ausnahmen werden separat gegen ihre dokumentierte semantische
  Normalisierung geprüft.
- Eingebettete SSH/PTTY-Ausgabe wird nicht umgefärbt.
- Ein expliziter Theme-Hintergrund wirkt ohne `opaque_background`, während
  `"terminal"` ohne den Legacy-Schalter unbemalt bleibt.
- Der Legacy-Schalter füllt bei `"terminal"` mit `semantic.canvas`, kann einen
  expliziten Theme-Hintergrund nicht unterdrücken und Theme-Verläufe erreichen
  niemals Remote-PTY-Zellen.
- Theme-Wechsel beeinflusst keine Host-, Tunnel-, SFTP- oder Session-Daten.

## Produktdokumentation

Der finale PR dokumentiert:

- Speicherort und Auswahl von Themes
- Aufbau einer minimalen Theme-Datei
- Palette, semantischer Kern, Komponenten-Fallbacks und `"auto"`-Reset
- Hex-, RGB-, Helligkeits- und Opacity-Syntax
- `"terminal"` und Grenzen echter Terminaltransparenz
- Vererbung und Referenzen
- Gradient-Richtungen und Multi-Stop-Beispiele
- vollständigen öffentlichen Rollenkatalog
- `sshub theme check <file>`, `theme list` und `theme show`
- Hinweis auf True-Color-Terminals und simulierte statt echter Opacity
- Kopieren und Anpassen der eingebauten Referenz-Themes

Die lokale Spec und spätere lokale Implementierungspläne werden nicht in den PR
aufgenommen. Produktrelevante README-Änderungen und die eingebetteten
Beispiel-Themes gehören dagegen ausdrücklich zum finalen Feature.

## Abnahmekriterien

Das Feature ist fertig, wenn:

1. SSHub ohne Konfigurationsänderung mit `default` startet und die definierte
   Default-Parität einhält — einschließlich aller drei dokumentierter
   Abweichungsklassen: der ANSI-Normalisierung, der einzeln protokollierten
   Zellen, die zuvor gar keine Farbe trugen, und der zwei bewussten
   Verbesserungen pro Oberfläche (`components.selection.inactive`,
   `components.sftp.notice`).
2. Der Theme-Picker alle eingebauten und benutzerdefinierten Themes auflistet.
3. Navigation eine Live-Vorschau zeigt, `Esc` zurückrollt und `Enter` speichert.
4. Ein Benutzer-Theme semantische Werte oder einzelne Rollen überschreiben und
   den Rest implizit von `default` erben kann; Referenzen werden nach dem Merge
   aufgelöst.
5. Hex, RGB, `"terminal"`, Helligkeit und simulierte Opacity mit explizitem
   Mischgrund korrekt aufgelöst werden.
6. Statische Multi-Stop-Verläufe auf Rahmen und Linien sichtbar funktionieren.
7. Jeder inventarisierte SSHub-eigene UI-Stil einer konfigurierbaren Rolle
   zugeordnet ist oder ausdrücklich als nicht themebar dokumentiert wurde.
8. `sshub theme check <file>` Syntax, Keys, Sections, Farben, Referenzen,
   Vererbung, Rollentypen und Gradienten semantisch prüft; `theme list/show`
   machen eingebaute Beispiele zugänglich.
9. Ein fehlerhaftes Theme weder Start noch laufende TUI unbenutzbar macht.
10. `default`, `summer`, `aqua`, `fire` und `high-contrast` validieren und ihre
    jeweils beabsichtigte Vorschau darstellen.
11. Gradienten per Buffer-Nachbearbeitung ohne per-cell Heap-Allokation
    funktionieren und die festgelegte lokale Release-Messung die `2 ms`-Grenze
    über die obere Schranke aus der Gesamt-Frame-Zeit des Gradient-Themes
    nachweist.
12. Sämtliche bestehenden und neuen Tests grün sind.

## Spätere Erweiterungen

Ausdrücklich nicht Teil dieser Spec, aber durch die Trennung der Module möglich:

- animierte Gradienten
- automatischer File-Watcher
- visueller Theme-Editor
- Theme-Scaffolding oder Installation per CLI
- Community-Theme-Katalog
- alternative Farbräume für die Interpolation
- Kontrast- und Farbwahrnehmungsanalyse
- Gradient-Cache nach nachgewiesenem Profiling-Bedarf
- Provenienz-Ausgabe wie `theme check --explain <rolle>`
- `theme check` ohne Datei zum Prüfen aller installierten Themes
