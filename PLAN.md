# PLAN — grover

Status: Abschnitte 1–11 und 12.1–12.7 umgesetzt, `make check` grün (111 Tests). Abschnitt 12
ist die priorisierte Liste der nächsten Features; 12.8 bekommt vor dem Code ein Go.
Regeln für die Umsetzung stehen in [AGENTS.md](AGENTS.md).

## 1. Ziel

Ein Ticket braucht heute viele manuelle Schritte: Ordner anlegen, in jedes betroffene Repo
gehen, Worktree mit Ticket-Branch anlegen, Agenten-Dateien dorthin kopieren, `cd`. `grover`
macht daraus einen Befehl.

```text
$ cd "$(grover new PROJ-123 --repo api,web)"
~/work/tickets/PROJ-123
├── AGENTS.md          # aus der Config kopiert
├── api/               # git worktree, Branch ticket/PROJ-123
└── web/               # git worktree, Branch ticket/PROJ-123
```

## 2. Entscheidungen (umgesetzt)

1. **Ausgabe = Pfad.** Ein Kindprozess kann die Shell des Users nicht wechseln. Deshalb ist
   der Ticket-Ordner die einzige Zeile auf stdout, Logs laufen über stderr. Nutzung:
   `cd "$(grover new PROJ-123 -a)"`. Alternative wäre ein Shell-Wrapper (`eval`) — mehr
   Magie, mehr Fehlerquellen.
2. **Worktrees flach im Ticket-Ordner** (`<ticket>/<repo>`), nicht gruppiert unter
   `<ticket>/repos/`. Kurze Pfade, direkt sichtbar in Editoren.
3. **Ticket wird normalisiert.** Aus `"PROJ 123/foo"` wird der Slug `PROJ-123-foo`; dieser
   Slug geht in Ordner *und* Branch (`ticket/PROJ-123-foo`). Roh-Ticket nur, wenn du
   Sonderzeichen im Branch willst — dann bricht git mit unklarer Meldung ab.
4. **Kein interaktiver Auswahldialog in Skript-Läufen.** Repos kommen über `--repo`
   (fuzzy, mehrfach oder kommagetrennt) oder `--all`. Ein Prompt wäre in Agenten- und
   Skript-Läufen ein Hänger. *Ergänzt durch 9:* fehlen Angaben und stdin ist ein TTY,
   fragt die CLI nach.
5. **`copy` kopiert ins Ticket-Root**, nicht in jedes Repo. Pro-Repo-Vorbereitung ist ein
   eigener Bedarf, siehe 12.4.
6. **Klone nur bei Bedarf.** Repos aus `repos` werden erst geklont, wenn sie gewählt
   werden; lokale Repos aus `repo_dirs` haben Vorrang.
7. **Fuzzy mit Sicherheitsnetz.** Exakter Treffer gewinnt; ist der beste Treffer nicht
   eindeutig, bricht `grover` mit Kandidatenliste ab, statt zu raten. Kein Treffer →
   Fehler mit den drei ähnlichsten Namen.
8. **Basis-Refs mit Fallback.** `base` darf eine Kette sein (`main|master|HEAD`); pro Repo
   gewinnt der erste Ref, der dort existiert. Existiert keiner, bricht `grover` ab.
9. **Mini-TUI nur im Terminal.** Prompts laufen nur, wenn stdin ein TTY ist
   (`std::io::IsTerminal`, ohne Extra-Dependency). Ohne TTY bleibt der bisherige Fehler —
   kein Hänger in Agenten und Skripten. Prompts rendern nach stderr, stdout bleibt allein
   der Ticket-Pfad, `cd "$(grover new)"` funktioniert also weiter.
10. **`inquire` als TUI-Bibliothek.** Inline-Prompts (`Text`, `Select`, `MultiSelect`) mit
    einem gemeinsamen Theme; Listen sind durchsuchbar, indem man tippt (Fuzzy-Filter,
    Seiten, `→` alle, `←` keine). Kein eigener `/`-Key: der wäre in `inquire` nur mit
    einem handgebauten Widget auf `console` zu haben (verworfen, zu viel Terminal-Code).
    Rendert auf stderr. Nur `grover-cli` kennt die Lib; `grover-lib` bleibt prompt-frei
    (Schichttrennung). Keine Fullscreen-TUI, kein `ratatui`.
11. **`config --init` fragt alles ab.** Kein `init`-Subcommand und keine eingebettete
    Vorlage: das Binary steht für sich und schreibt nur Werte, die du eingibst. Ein Wizard
    fragt `work_dir` (Pflicht, kein Default), `branch_template`, `base`, `repo_dirs`,
    `clone_dir`, `repos`, `copy` und `setup`. Enter behält den Default; leere Listen landen
    nicht in der Datei. Geschrieben wird erst nach `Config::validate`; eine vorhandene Datei nur
    mit `--force`. Feintuning macht danach `grover config`.
12. **`config` schreibt sofort.** Jede Änderung geht direkt in die Datei; kein Dirty-State,
    kein Speichern-Schritt. Geschrieben wird über eine Temp-Datei plus `rename`, damit ein
    Absturz nie eine halbe Config hinterlässt. Preis: Kommentare gehen beim ersten Speichern
    verloren. Alternative wäre Text-Editing mit Kommentar-Erhaltung — viel Code für
    seltenen Nutzen.
13. **Projektname `grover`.** Binary, Crates und Config-Namen lauten einheitlich `grover`:
    `grover-lib`/`grover-cli`, `./grover.yaml`, `$XDG_CONFIG_HOME/grover/config.yaml`,
    `~/.grover/repos` und `GROVER_LOG`. Keine Legacy-Aliase oder Fallbacks auf alte Pfade;
    vorhandene Configs und Klone werden einmalig manuell umbenannt.

## 3. Workspace

```text
Cargo.toml             # virtueller Workspace (resolver 3), gemeinsame Deps + Lints
rustfmt.toml           # max_width = 100
Makefile               # make install (PREFIX, Default ~/.local), build, check
grover.yaml.example    # Format-Referenz (Doku, nicht eingebettet)
grover-lib/            # Core, keine Ausgabe, kein clap
grover-cli/            # bin "grover"
```

`grover-lib` (crate `grover_lib`):

| Modul    | Inhalt                                                           |
| -------- | ---------------------------------------------------------------- |
| `config` | `Config` lesen (serde/yaml), `~` und relative Pfade auflösen,    |
|          | validieren, als YAML ausgeben                                    |
| `repo`   | Repos finden (`repo_dirs`, `repos`), `--repo` auflösen           |
| `git`    | `is_repo`, `worktree_add`, `clone` via git-CLI                   |
| `fuzzy`  | Auswahl: exakt, Präfix, Teilstring, ein Tippfehler, Jaro-Winkler |
| `plan`   | `Plan::build`/`apply`, vorhandene Ticket-Namen auflisten         |
| `error`  | `Error` per thiserror, `Result<T>`                               |

Öffentliche API:

```rust
let config = Config::load(optional_path)?;           // lesen + validieren
let config = config.save(&path)?;                    // schreiben (config --init)
let plan = Plan::build(&config, &Request { .. })?;   // auflösen, nichts schreiben
let report = plan.apply(dry_run)?;                   // anlegen, melden was passiert ist
let tickets = plan::existing_tickets(&config)?;      // Ticket-Namen, sortiert
let statuses = plan::statuses(&ticket_dir)?;         // Worktree-Zustand, sortiert
```

`grover-cli` (bin `grover`) enthält `clap`-Definitionen, Logging-Setup, Ausgabe, den
Exit-Code und die Prompts:

| Datei         | Inhalt                                            |
| ------------- | ------------------------------------------------- |
| `src/main.rs` | Argumente, Ablauf, Ausgabe                        |
| `src/tui.rs`  | Prompts über `inquire`; einziger Ort mit TUI-Code |

Abhängigkeiten: `clap` (derive), `clap_complete`, `anyhow`, `inquire`, `tracing`,
`tracing-subscriber` (`env-filter`, Logs Ziel stderr, Level über `-v`/`-q`/`GROVER_LOG`).
Lib-Abhängigkeiten: `serde`, `serde_yaml_ng`, `thiserror`, `strsim`, `dirs`, `tracing`;
Tests: `tempfile`.

## 4. Config

Format ist YAML. Fundort: `--config <pfad>`, sonst `./grover.yaml`, sonst
`$XDG_CONFIG_HOME/grover/config.yaml`. Relative Pfade in der Datei gelten relativ zur
Datei, `~` ist das Home-Verzeichnis, alles wird intern absolut. Felder, die es noch nicht
gibt, stehen mit ihrer Zielform in Abschnitt 12 (`ai` 12.8).

```yaml
work_dir: "~/work/tickets"           # Pflicht: hier entstehen die Ticket-Ordner
branch_template: "ticket/{ticket}"   # Pflicht-Token {ticket}, Default s. o.
base: "main|master|HEAD"             # erster existierender Ref je Repo
repo_dirs:                           # Ordner, deren Kind-Repos gefunden werden
  - "~/code"
clone_dir: "~/.grover/repos"         # Ziel für Klone aus `repos`

repos:                               # Repos, die bei Bedarf geklont werden
  - url: "git@github.com:org/api.git"
    name: api                        # optional, sonst aus der URL abgeleitet

copy:                                # Dateien in den Ticket-Ordner
  - from: "~/templates/AGENTS.md"
    to: AGENTS.md                    # optional, sonst Dateiname

setup:                               # Befehle nach jedem neuen Worktree
  - "cp .env.example .env"
```

`to` ist relativ zum Ticket-Ordner; ein absoluter Pfad ist ein Config-Fehler.

Unbekannte Felder sind ein Fehler (`deny_unknown_fields`), damit Tippfehler auffallen.
`grover config --init` fragt die Werte ab und schreibt sie, `grover config` ändert sie
interaktiv, `grover config --show` gibt die aufgelöste Config aus, `--path` nur den
Dateipfad (Details in Abschnitt 11).

## 5. CLI

```text
grover new [<ticket>] [-r|--repo <name>]... [-a|--all] [-n|--dry-run]
                      [-t|--title <text>] [-c|--config <pfad>]
grover list [-c|--config <pfad>]
grover rm [<ticket>] [-f|--force] [-n|--dry-run] [-c|--config <pfad>]
grover path [<ticket>] [-c|--config <pfad>]
grover status [<ticket>] [-c|--config <pfad>]
grover config [-c|--config <pfad>] [-p|--path] [-s|--show]
                     [-i|--init [--force] [--defaults]]
                     [--set <feld>[=<wert>]]
grover completions <bash|zsh|fish|...>
```

- `new` gibt den Ticket-Ordner auf stdout aus, Fortschritt/Warnungen nach stderr.
- `--repo` ist fuzzy, mehrfach oder kommagetrennt; `--all` nimmt alle.
- Fehlen Ticket, `--repo` und `--all`, fragt `new` nach — aber nur mit TTY (Abschnitt 11.3).
  Ohne TTY bleibt es beim Fehler, es wird nichts angelegt.
- `--title` beschreibt das Ticket für `ai.branch_name` (12.8); ohne Titel fragt `new` auf
  einem TTY danach, wenn `ai` konfiguriert ist.
- `--dry-run` zeigt Plan und Aktionen, ohne zu schreiben.
- `list` zeigt nur vorhandene Work Items: die Namen der direkten Ticket-Unterordner von
  `work_dir`, alphabetisch sortiert und ohne Zusatzspalten. Dateien werden ignoriert;
  ein fehlendes `work_dir` ergibt eine leere Liste. Repos werden dafür nicht gesucht.
- `config` ohne Flags ist der interaktive Editor, `--show` die aufgelöste Config,
  `--init` der Wizard für eine neue Datei (braucht ein Terminal, `--force` überschreibt).
- `rm` entfernt ein Ticket wieder (Abschnitt 12.1); stdout bleibt dabei leer.
- `path` gibt den Ticket-Ordner als einzige Zeile aus (Abschnitt 12.2); ohne Argument
  entscheidet der cwd, ein cwd außerhalb von `work_dir` ist ein Fehler.
- `status` zeigt je Worktree Branch, Abstand zum Upstream und dirty-Dateien; ohne
  `<ticket>` entscheidet der cwd wie bei `path` (Abschnitt 12.3).
- Exit-Code 0 bei Erfolg, 1 bei Fehlern und Abbruch, 2 bei Clap-Nutzungsfehlern.

## 6. Ablauf `grover new`

1. Config laden und validieren (`branch_template` enthält `{ticket}`, `base` hat mindestens
   einen Ref, mindestens eine Repo-Quelle, URLs nicht leer).
2. Repos sammeln: Kind-Ordner von `repo_dirs` mit `.git`, plus `repos` (Name aus
   `name` oder URL-Ende, Pfad `clone_dir/<name>`). Doppelte Namen: lokaler Eintrag gewinnt.
3. Fehlende Angaben erfragen, nur wenn stdin ein TTY ist: Ticket und/oder Repo-Auswahl
   (Abschnitt 11.3). Ohne TTY: Fehler wie bisher.
4. Ticket zu Slug normalisieren (nur `[A-Za-z0-9._-]`, keine führenden `-`/`.`), Branch
   aus `branch_template` — oder aus `ai.branch_name`, wenn konfiguriert (12.8); bei
   Fehlschlag bleibt es beim Template. Ticket-Ordner aus `work_dir` + Slug.
5. Auswahl per Fuzzy auflösen; Plan bauen (`worktrees`, `copies`).
6. `apply`: Ticket-Ordner anlegen, pro Repo einen Worktree auf dem ersten dort
   existierenden Ref aus `base` anlegen und Setup ausführen; danach Dateien kopieren.
   Vorhandene Pfade werden übersprungen und nicht überschrieben.
   Alle Pfade sind vor dem git-Aufruf absolut: Ein relatives Ziel würde
   `git -C <repo> worktree add` im Repo anlegen, nicht im Projekt.

## 7. Fehlerfälle und Verhalten

| Fall                                  | Verhalten                                           |
| ------------------------------------- | --------------------------------------------------- |
| Worktree-/Dateipfad existiert schon   | übersprungen, Warnung, Lauf gilt als Erfolg         |
| Branch existiert schon lokal          | wird wiederverwendet (`worktree add <pfad> <br>`)   |
| Branch ist schon woanders ausgecheckt | git-Fehler wird durchgereicht                       |
| Repo hat noch keinen Commit           | git-Fehler wird durchgereicht                       |
| Kein Basis-Ref existiert              | Fehler mit der Liste der versuchten Refs            |
| Remote-Repo fehlt lokal               | wird vorher geklont, Klon landet in `clone_dir`     |
| Klon braucht Passwort                 | `GIT_TERMINAL_PROMPT=0`, also Fehler statt Hänger   |
| Unbekannter Repo-Name                 | Fehler mit den drei ähnlichsten Namen               |
| Mehrdeutiger Fuzzy-Treffer            | Fehler mit Kandidatenliste                          |
| `repo_dirs`-Ordner fehlt              | Warnung, Rest läuft weiter                          |
| Verzeichniseintrag nicht lesbar       | Warnung pro Eintrag, der Rest läuft weiter          |
| Relative Pfade in Config oder Aufruf  | vor dem git-Aufruf absolut machen, sonst bezieht    |
|                                       | `git -C` sie auf das Repo statt auf das Projekt     |
| Absoluter `copy.to`                   | Fehler bei `Config::validate`, nichts geschrieben   |
| Zwei Repos mit gleichem Namen         | erster Treffer gewinnt (lokal vor remote)           |
| Kein TTY und Ticket/Repos fehlen      | Fehler wie bisher, kein Prompt                      |
| `config --init` ohne TTY              | Fehler mit dem Zielpfad, nichts geschrieben         |
| `config --init` trifft vorhandene     | Fehler, außer `--force`; Abbruch hinterlässt keine  |
|                                       | Datei                                               |
| `work_dir` fehlt oder ist leer        | Fehler, sonst landen Tickets im Config-Ordner       |
| Abbruch im Prompt (Ctrl-C/Esc)        | nichts geschrieben, Exit 1                          |

## 8. Tests

- Unit-Tests je Modul: Slug/Branch-Template, Config-Defaults und `deny_unknown_fields`,
  `~`-Expansion, URL→Name, Fuzzy (exakt, Präfix, Vertauscher, unbekannt, mehrdeutig),
  Auswahl (`all`, Reihenfolge, Duplikate), `base` ohne Ref.
- Integrationstests mit echten git-Repos in `TempDir`: Worktree landet im Ticket-Ordner,
  Branch heißt wie konfiguriert, `copy`-Datei liegt im Ticket-Root, zweiter Lauf ist
  idempotent, `--dry-run` schreibt nichts, Remote-Repo wird geklont, unbekannter Name
  scheitert vor dem ersten Schreibzugriff, `base`-Fallback (`missing|stable|HEAD` checkt aus
  `stable` aus) und Fehler mit den Ref-Namen, wenn keiner existiert.
- Relativer Config-Pfad: alle Pfade bleiben absolut.
- `Config::save`: schreiben und wieder laden ergibt dieselbe Config (Round-Trip), und es
  bleibt keine Temp-Datei liegen. `work_dir: ""` ist ein Fehler statt des Config-Ordners.
  Die Beispiel-Config im Repo (reine Doku, nicht mehr eingebettet) muss `Config::load`
  bestehen, damit die Format-Referenz nicht verrottet.
- CLI-Integration über `CARGO_BIN_EXE_grover` (kein Extra-Crate): ohne TTY und ohne
  Angaben → Exit 1 statt Hänger; `config --init` ohne TTY schreibt nichts und nennt den
  Zielpfad, eine vorhandene Datei scheitert ohne `--force`; `config --show` und `--path`
  schreiben nur das Erwartete auf stdout. `list` zeigt nur die Namen angelegter Tickets,
  sortiert und ohne Repo-Informationen; ohne `work_dir` ist die Ausgabe leer.
- TUI-Pfad von Hand (echtes Terminal oder `script`), weil Prompts ohne TTY nicht sinnvoll
  testbar sind: Ticket eingeben, Repos mehrfach wählen, abbrechen.
- Für Abschnitt 12 kommen die Fälle aus 12.1 bis 12.8 dazu (dort je Feature gelistet).

## 9. Non-Goals

- Kein `--json`, keine Fullscreen-TUI (`ratatui`), keine Maus, kein Undo/History.
- Keine Änderung an `git config` oder generische git-Hooks. Setup-Befehle nach dem
  Anlegen sind in 12.4 als Config-Liste umgesetzt, nicht als Hook-System.
- Keine `copy`-Einträge pro Repo (nur Ticket-Root).
- Kein Mergen und kein Aufräumen ohne Aufforderung; Löschen ist `rm` (12.1), nie implizit.
  Kein `fetch` im Normallauf, höchstens opt-in (12.9).
- Keine Doku-Generierung (Handbuch, Manpage); `README.md` und `docs/` werden von Hand
  gepflegt.
- Keine eingebaute LLM-Anbindung: Branch-Namen per AI laufen über ein konfiguriertes
  Kommando (12.8), nicht über einen HTTP-Client im Binary.
- Keine Kommentar-Erhaltung beim interaktiven Schreiben der Config.
- Kein Bearbeiten einzelner Listeneinträge (nur hinzufügen und entfernen).

## 10. Status

- [x] Workspace: virtuelles Manifest, `grover-lib` + `grover-cli`, Lints, `rustfmt.toml`
- [x] Projektname `grover`: Binary, Crates, Config-Pfade, Tests und Doku
- [x] AGENTS.md (Regeln) und PLAN.md (dieser Plan)
- [x] `grover-lib`: `config`, `repo`, `git`, `fuzzy`, `plan`, `error`
- [x] `grover-cli`: `new`, `list`, `config`, `completions`
- [x] `Makefile`: `build`, `install`, `uninstall`, `check`
- [x] `grover.yaml.example` und Tests (85 Tests)
- [x] Ponytail-Review: keine offenen Findings (`fmt`, `clippy -D warnings`, `test` grün)
- [x] `README.md` und `docs/` (Einstieg, Konzepte, Config, Kommandos, Fehlerbehebung)
- [x] `inquire` + `grover-cli/src/tui.rs` (Prompts, TTY-Wächter, Suchfilter)
- [x] `grover config --init` (Wizard, `--force`)
- [x] `grover config` interaktiv, dazu `--show`/`--path`
- [x] `grover new` ohne Argumente (Ticket-Prompt + Repo-MultiSelect)
- [x] Tests und Gate für Abschnitt 11 (43 Tests), TUI-Pfad im PTY geprüft
- [x] 12.1 `grover rm`: Worktrees, Branches, `copy`-Dateien, Ticket-Ordner (69 Tests)
- [x] 12.2 `grover path`: Argument, Fuzzy, cwd (73 Tests)
- [x] 12.3 `grover status`: Branch, Upstream, dirty, Tabelle (81 Tests)
- [x] 12.4 Setup-Befehle nach Worktree-Erstellung (85 Tests)
- [x] 12.6 `--set`, Feldhilfe in Editor und Wizard (92 Tests)
- [x] 12.7 Esc als „zurück“, Defaults, Schritt-Zähler, Schlussfrage (100 Tests)
- [x] 12.5 Pfad-Autovervollständigung per Tab (111 Tests)
- [ ] Durchlauf mit deiner echten Config

## 11. Mini-TUI (`config`, `new`)

Drei Eingaben, ein gemeinsamer Unterbau: `grover-cli/src/tui.rs` kapselt `inquire`;
alle Listen filtern beim Tippen, `↑`/`↓` bewegen, `→` wählt alle, `←` keine.
Vor jedem Prompt steht der TTY-Wächter (`std::io::IsTerminal`), sonst passiert genau das,
was heute passiert. Prompts nach stderr, Fortschritt weiterhin über `tracing`.

Ein gemeinsames `RenderConfig` macht die Prompts optisch konsistent: `?` für offene und `✓`
für bestätigte Eingaben, `❯` für die aktive Option, `[✓]` für Mehrfachauswahlen. Cyan
markiert Fokus und Antworten, Grün Bestätigungen; Hilfen und inaktive Checkboxen sind
dunkelgrau.
Bis zu zwölf Optionen stehen ohne Blättern untereinander, Werte und Repo-Status sind bündig.
`NO_COLOR` behält Marker und Layout bei, entfernt aber sämtliche Farben und Attribute.

### 11.1 `grover config --init`

- Ziel: `--config <pfad>`, sonst derselbe Fundort wie beim Lesen
  (`./grover.yaml`, sonst `$XDG_CONFIG_HOME/grover/config.yaml`).
- Keine Vorlage mehr: das Binary steht für sich, es schreibt nur die Werte, die du
  eingibst. Leere Listen (`repo_dirs`, `repos`, `copy`, `setup`) erscheinen nicht im
  YAML.
- Ein Durchlauf, Feld für Feld: `work_dir` (Pflicht, wird bis zur Eingabe wiederholt),
  `branch_template`, `base`, `repo_dirs`, `clone_dir`, `repos`, `copy`, `setup`.
  Enter behält den angezeigten Default, Listen überspringt man über `← back`.
- Vorhandene Datei: Fehler mit Pfad, außer `--force`.
- Ohne TTY: Fehler mit dem Zielpfad, nichts geschrieben — kein Hänger in Skripten.
- Geschrieben wird erst nach `Config::validate`; Abbruch oder Validierungsfehler
  hinterlassen keine Datei.

### 11.2 `grover config` (interaktiv)

Menü mit allen Top-Level-Feldern und ihrem aktuellen Wert:

```text
? Config bearbeiten  ~/.config/grover/config.yaml
❯ work_dir          ~/work/tickets
  branch_template   ticket/{ticket}
  base              main|master|HEAD
  repo_dirs         1 Eintrag
  clone_dir         ~/.grover/repos
  repos             2 Einträge
  copy              1 Eintrag
  setup             1 Eintrag
  beenden
```

- Eingabe je Feld über `Input`; leere Eingabe behält den alten Wert. Validiert wird durch
  Neuladen (`Config::load`): schlägt es fehl, wird die Datei auf den Stand davor
  zurückgeschrieben und der Fehler als Warnung gemeldet. Keine doppelte Logik.
- Listen (`repo_dirs`, `repos`, `copy`, `setup`) laufen über einen gemeinsamen
  Untermenü-Helfer: Einträge anzeigen, hinzufügen, entfernen.
  Eine Hilfsfunktion für vier Felder, kein Zustandsautomat für
  „bearbeiten“.
- `repos` fragt `url` und `name` (leer = aus der URL), `copy` fragt `from` und
  `to` (leer = Dateiname), `repo_dirs` einen Pfad und `setup` einen Befehl je
  Eintrag.
- Geschrieben wird nach jeder Menüaktion; ein Listeneintrag landet beim Verlassen
  des Untermenüs in der Datei. Geschrieben werden absolute Pfade; Kommentare gehen
  beim Speichern verloren (Entscheidung 12).
- `--path` bleibt (Fundort), neu `--show` (aufgelöste Config, bisheriges Verhalten des
  nackten `config`). Ohne TTY und ohne Flag: Fehler mit Hinweis auf `--show`.
- Abbruch: `Ctrl-C` bricht jeden Prompt ab, Exit 1, nichts geschrieben; „exit“ im Menü ist
  Exit 0. Prompt-Texte sind englisch wie der Rest der CLI. `Esc` heißt seit 12.7 „eine
  Ebene zurück“, der alte Wert steht als Default statt als Vorbelegung.

### 11.3 `grover new` ohne Argumente

```text
$ grover new
? Ticket  PROJ-123
? Repos (Leertaste = wählen)
❯ [✓] api   local
  [ ] web   remote (wird geklont)
```

- `<ticket>` wird optional. Fehlt es und stdin ist ein TTY, kommt ein `Input` mit derselben
  Prüfung wie danach im Slug (nicht leer, ergibt einen Ordnernamen).
- Fehlen `--repo` und `--all`, kommt ein `MultiSelect` über die gefundenen Repos, in
  Discovery-Reihenfolge (alphabetisch), remote gekennzeichnet.
- `--repo`, `--all` und ein Ticket-Argument überspringen jeweils nur ihren Prompt.
- Leere Mehrfachauswahl: Warnung und nochmal fragen. Abbruch: Exit 1, nichts geschrieben.
- Danach unverändert: Plan bauen, `apply`, Pfad auf stdout.

## 12. Roadmap (in dieser Reihenfolge)

Die Reihenfolge ist Nutzen pro Aufwand, nicht Wunschliste. 12.1–12.7 sind umgesetzt; 12.8
bekommt vor dem Code sein Go. Die Tests aus Abschnitt 8 gelten mit. 12.9 ist Backlog, 12.10
ist die Absage.

12.5–12.7 sind ein Paket (Bequemlichkeit des Wizards) und fassen dieselbe `Prompter`-API
an; gebaut wurden sie in der Reihenfolge 12.6 → 12.7 → 12.5, damit jede Stufe auf der
vorigen aufsetzt. Alle drei sind umgesetzt.

### 12.1 `grover rm [<ticket>]` — aufräumen (umgesetzt)

Heute ist der Zyklus einbahnig: `new` legt an, nichts baut ab. Jedes Ticket hinterlässt pro
Repo einen Worktree und einen Branch; Aufräumen heißt N-mal drei git-Befehle plus `rm -rf`.
Das ist die einzige Stelle, an der `grover` Daten löscht — deshalb ist sie ausdrücklich,
nie implizit.

```text
grover rm [<ticket>] [-f|--force] [-n|--dry-run]
```

- Ticket: Argument, sonst auf einem TTY ein `Select` über `existing_tickets` (Tippen
  filtert), ohne TTY ein Fehler mit Nutzung. Bei einer Löschoperation wird nicht aus dem
  cwd geraten. Unbekanntes Ticket → Fehler mit Kandidaten, nichts angefasst.
- Reihenfolge: Worktrees, dann die `copy`-Dateien, dann der Ticket-Ordner.
- Je Kind mit `.git`-Datei (= Worktree), vor dem Entfernen im Worktree:
  `HEAD`-Branch (`rev-parse --abbrev-ref HEAD`) und Common-Dir
  (`rev-parse --git-common-dir`, relativ zum Worktree auflösen) merken. Danach
  `git worktree remove <pfad>` und `git worktree prune` im Common-Dir.
  Grund für diesen Umweg: Repo und Branch kommen aus dem Worktree selbst, nicht aus der
  Config — `rm` funktioniert also auch, wenn `repo_dirs` sich seither geändert hat.
- Die Kinder werden erst gesammelt, dann entfernt: Löschen während `read_dir` läuft, kann
  auf Dateisystemen mit gehashten Verzeichnissen einen Eintrag überspringen.
- Branch danach über das Common-Dir: `git branch -d <branch>`. Nicht gemergt → Warnung,
  Branch bleibt (`-d`, nie `-D` ohne `--force`). `--force` entfernt Worktree und Branch
  hart (`worktree remove --force`, `branch -D`). Ein detached HEAD meldet `HEAD`; dann
  bleibt der Branch stehen, es wird keiner gelöscht.
- Ticket-Ordner: `fs::remove_dir`, nicht `remove_dir_all`. Liegt noch etwas darin
  (Notizen, fremde Ordner), bleibt er mit Warnung stehen; gelöscht wird nur, was `grover`
  selbst angelegt hat.
- stdout bleibt leer (nichts cd't dorthin); was entfernt und was übersprungen wurde, geht
  über `tracing` nach stderr. `--dry-run` zeigt dasselbe, schreibt nichts.
- Ein dirty Worktree oder ein fremder Ordner bricht den Lauf nicht ab: Warnung, weiter mit
  dem nächsten Repo, Exit 0. Fehler nur, wenn gar nichts aufgelöst werden kann.
- `git.rs`: `worktree_remove`, `worktree_prune`, `branch_delete`, `head_branch` (liefert
  `None` bei detached `HEAD`), `common_dir`. `plan.rs`: `remove` plus `resolve_ticket`
  (geteilt mit 12.2 und 12.3).
- `resolve_ticket` löst nur exakt auf; Fuzzy dient allein der Kandidatenliste im Fehler.
  Dafür heißen die Fuzzy-Fehler seit 12.1 neutral `UnknownName`/`AmbiguousName` („unknown
  name") und `fuzzy::similar` liefert die Vorschläge. Auch Repo-Abfragen melden dadurch
  „unknown name" — mit `-r` im Aufruf reicht das, eine eigene Repo-Formulierung im Aufrufer
  lohnt den Code nicht.
- Tests: Worktree + gemergter Branch + Ordner sind weg; dirty Worktree bleibt ohne
  `--force` stehen und der Rest wird trotzdem entfernt; ungemergter Branch bleibt ohne
  `--force`; `--force` räumt dirty und unmerged ab; zusätzliche Datei im Ticket-Ordner
  verhindert das Ordner-Löschen; detached HEAD geht ohne Branch-Löschung; die Config darf
  das Repo inzwischen vergessen haben; unbekanntes Ticket ändert nichts (kein Fuzzy-Treffer);
  `--dry-run` ändert nichts.

### 12.2 `grover path [<ticket>]` — Pfad ausgeben (umgesetzt)

`new` gibt den Pfad nur beim Anlegen aus; für ein bestehendes Ticket fehlt der Weg dorthin.

```text
grover path [<ticket>]
```

- Mit Argument: Slug + vorhandener Ordner, sonst Fuzzy über `existing_tickets`; mehrdeutig
  → Kandidaten, Exit 1. Genau eine Zeile auf stdout, sonst nichts.
- Ohne Argument: cwd aufwärts bis unter `work_dir`, das erste Segment darunter ist das
  Ticket. Liegt der cwd nicht darunter: Fehler.
- `plan::find_ticket` (exakt, sonst der eine Fuzzy-Treffer) steht neben `resolve_ticket`,
  das exakt bleibt, weil `rm` löscht; `plan::ticket_above` plus `Error::NotInWorkDir` deckt
  den cwd-Fall ab. Kein Prompt: ohne Argument entscheidet allein der cwd.
- Nutzung: `cd "$(grover path PROJ-1)"`.

### 12.3 `grover status [<ticket>]` — Zustand aller Repos (umgesetzt)

Der Grund, warum viele Repos weh tun: man fragt sie einzeln. Eine Tabelle pro Ticket.

```text
$ grover status PROJ-1
repo   branch             ahead/behind  dirty
api    ticket/PROJ-1      +2/-0         3 files
web    ticket/PROJ-1      -             clean
```

- Je Kind des Ticket-Ordners, das ein Worktree ist; andere Einträge werden ignoriert.
- Je Repo drei git-Aufrufe: `rev-parse --abbrev-ref HEAD`, `status --porcelain` (Zeilen
  zählen), `rev-list --left-right --count @{u}...@`. Kein Upstream (frischer Ticket-Branch)
  → `-`.
- Sequenziell. Parallel erst, wenn ein echtes Ticket messbar zu langsam ist — dann mit
  einem `shortcut:`-Kommentar im Code, nicht vorher.
- Tabelle auf stdout (das ist der Zweck des Befehls), Warnungen nach stderr. Kein `--json`,
  keine Spaltenoptionen.
- Ohne `<ticket>` wie 12.2.
- Tests: Ausgabeformat mit einem sauberen und einem dirty Repo, Repo ohne Upstream, fremde
  Datei im Ticket-Ordner wird ignoriert, unbekanntes Ticket ist ein Fehler.

### 12.4 Setup-Befehle nach dem Anlegen (umgesetzt)

Nach jedem Worktree folgt derselbe Handgriff: `.env` kopieren, `npm install`,
`direnv allow`. Heute macht das die Hand oder ein Shell-Skript danach.

```yaml
setup:                               # nach jedem neuen Worktree, cwd = Worktree
  - "cp .env.example .env"
```

- Eine Top-Level-Liste, kein Feld pro Repo: die Befehle sehen `GROVER_REPO` und
  `GROVER_TICKET`, ein `case "$GROVER_REPO"` reicht für den Sonderfall. Eine Map
  Name→Befehle wäre die nächste Stufe, keine zweite Config-Fläche zum Start.
- Ausführung über `sh -c`, sequenziell, cwd = neuer Worktree.
  Setup-stdout geht nach stderr, stderr wird geerbt; Ausgabe bleibt sichtbar.
  stdout enthält dadurch weiter nur den Ticket-Pfad.
- Nur für Worktrees, die in diesem Lauf neu entstehen — nicht für übersprungene, nicht für
  `copy`-Ziele.
- Fehlschlag: Warnung mit Exit-Code, der Worktree bleibt, der Lauf gilt als Erfolg. Kein
  Abbruch, sonst halb fertige Tickets.
- `--dry-run` loggt die Befehle und führt nichts aus.
- Config: `#[serde(default, skip_serializing_if = "Vec::is_empty")] setup: Vec<String>`,
  gleiche Form wie `repo_dirs` in `config --init` (leer = nicht in der Datei) und im
  Editor (Listen-Untermenü mit einem Befehl je Eintrag).
- Tests: Befehl legt eine Datei im Worktree an, Fehlschlag ist eine Warnung ohne Abbruch,
  `--dry-run` führt nichts aus, übersprungener Worktree bekommt kein Setup.

### 12.5 Pfad-Autovervollständigung (umgesetzt)

Pfad-Prompts (`work_dir`, `repo_dirs`, `clone_dir`, `copy.from`) tippt man sonst von Hand.
`inquire` bringt Tab-Completion mit, es fehlte nur ein `Autocomplete`.

- `PathCompleter { cwd: PathBuf }` in `tui.rs` implementiert `inquire::Autocomplete` (muss
  `Clone` sein): `get_suggestions` listet die Einträge des Verzeichnisses aus der Eingabe,
  deren Name mit dem letzten Pfadsegment beginnt (Ordner immer, Dateien nur bei
  Datei-Prompts); `get_completion` ergänzt auf das längste gemeinsame Präfix. Tab übernimmt,
  `↑`/`↓` gehen durch die Vorschläge (`Text::with_page_size`).
- `~` wird fürs Nachschlagen expandiert und in den Vorschlägen genau so zurückgegeben, wie
  es getippt wurde — Tab darf die Home-Abkürzung nicht durch einen absoluten Pfad ersetzen.
- Relative Eingaben gelten ab dem Arbeitsverzeichnis, `../` und `/` funktionieren. Mehr als
  1000 Vorschläge werden nicht gelistet, damit die Liste bedienbar bleibt.
- `Prompter::input_path` ist `input` (12.7) plus Completer; nur Pfad-Felder nutzen ihn.
  `copy.to` bleibt ohne Completion: das Ziel liegt im Ticket-Ordner, den es noch nicht gibt.
- Tests: Vorschläge zu einem Präfix, längstes gemeinsames Präfix, `~`-Rundlauf, ein Präfix
  ohne Treffer liefert keine Vorschläge.

### 12.6 Mehr Flags und Feldhilfe (umgesetzt)

Der Wizard kennt die Felder, erklärt sie aber nicht: was `base` mit `main|master|HEAD`
macht, steht nur in dieser Datei. Und wer einen Wert ändern will, klickt sich durchs Menü.

- Eine Tabelle `FIELDS` trägt je bestehendem skalarem Feld (`work_dir`, `branch_template`,
  `base`, `clone_dir`) Name, Setter `fn(&mut Config, &str)` und einen Satz Hilfe
  (Aufgabe, Beispiel, was Enter tut). Sie speist `--set`, Editor und `--init`:
  `edit_string`, `edit_path` und `edit_optional_path` werden zu einem Weg, `required`
  zu „der Setter lehnt leer ab, also nochmal fragen“. `ai.branch_name` kommt erst mit
  12.8 in die Tabelle.
- Die Hilfe steht dauerhaft als `help_message` unter dem Prompt und als zweite Spalte in
  der Menüzeile (`work_dir  ~/work/tickets  where tickets go`); Listenfelder bekommen ihren
  Satz in `menu`. Kein `?`-Key: `inquire` 0.9.4 kennt keine Help-Action, `?` landet im
  Textfeld als Zeichen und in Listen im Filter. Damit gibt es genau einen Hilfe-Text je
  Feld.
- `grover config --set <feld>[=<wert>]` für die Felder aus `FIELDS`; Listen bleiben im
  Editor, sonst bräuchte es eine zweite Syntax. Ein Argument mit `=`, weil clap bei zwei
  Werten kein optionales zweites kennt. Ohne `=` wird auf einem TTY geprompted, ein leerer
  Wert löscht `clone_dir`. Geschrieben wird über `save_and_reload` wie im
  Editor; die Datei muss existieren.
- Tests: `--set base` ändert nur `base`; unbekanntes Feld ist ein Fehler mit Kandidaten;
  `--set` ohne Datei ist ein Fehler; jeder Name in `FIELDS` ist ein echtes Config-Feld.

### 12.7 Besserer Wizard für `--init` und den Editor (umgesetzt)

Zwei echte Defekte und fehlende Bequemlichkeit. Was `inquire` 0.9.4 hergibt (im Quelltext
geprüft): Esc kommt als `OperationCanceled` an, Ctrl-C als `OperationInterrupted`;
`prompt_skippable` macht nur aus dem ersten `Ok(None)`.

1. **Default statt Vorbelegung.** `Text::with_initial_value(current)` legt den alten Wert in
   den Puffer; wer ändern will, muss ihn erst löschen. Üblich ist: Default blass neben dem
   Prompt zeigen, Enter übernimmt, Tippen ersetzt — `with_default(current)`, dazu
   `theme.default_value` auf Dunkelgrau (heute leer, also unsichtbar abgesetzt). Kein
   zusätzliches `with_placeholder`, sonst stünde der Wert zweimal da. Das ist der
   Hauptgrund, warum der Wizard unausgereift wirkt.
2. **Esc ist heute alles oder nichts.** Es kann „eine Ebene zurück“ heißen, ohne eigenes
   Widget: der Feld-Prompt liefert `Ok(None)`, der Aufrufer entscheidet; Ctrl-C bleibt der
   Abbruch. Dafür darf `cancelled()` `OperationInterrupted` nicht mehr verschlucken.
3. **`--init` hat keinen Weg zurück** und schreibt ohne letzte Rückfrage.

- `Prompter::input(prompt, default, hint)` ersetzt `input(prompt, initial)`: leer plus
  Enter gibt den Default zurück, damit entfällt `answer_or_current`. Die `hint`-Texte
  kommen aus `FIELDS` (12.6).
- `cancelled()` behandelt nur Esc als Prompt-Abbruch; Ctrl-C wird ein Fehler → Exit 1,
  nichts geschrieben (der Test `cancel_and_interrupt_are_aborts` wird angepasst).
- Editor: Esc in einem Feld-Prompt führt zurück ins Menü, der alte Wert bleibt; Esc im Menü
  beendet den Editor mit Exit 0 (jedes Feld schreibt weiterhin sofort, Entscheidung 12).
  Listen-Untermenüs behalten `← back`.
- `--init` wird eine numerierte Kette (`[2/8] base`): Esc im Feld geht zum vorigen Feld und
  behält den getippten Wert, Esc im ersten Feld bricht ab. Den Schritt-Zähler liefert der
  Index, den der Navigator ohnehin braucht.
- Letzter Schritt ist ein `Confirm` „write <pfad>?“. Bis dahin ist nichts geschrieben; ein
  „nein“ beendet mit Exit 1 ohne Datei.
- `--init --force` auf einer ladbaren Datei füllt alle Defaults daraus vor; ist sie nicht
  ladbar, kommt eine Warnung und der Lauf startet leer. Ohne `--force` bleibt die
  vorhandene Datei ein Fehler.
- Tests: Esc im Feld behält den Wert und kehrt ins Menü zurück; Ctrl-C bricht mit Fehler ab;
  Esc in `--init` geht ein Feld zurück und behält die Eingabe; „nein“ in der Rückfrage
  schreibt nichts; Enter ohne Eingabe nimmt den Default; vorhandene Werte erscheinen als
  Default.

### 12.8 Branch-Name aus einem lokalen Modell

`branch_template` macht aus dem Ticket einen Namen. Ein lokales Modell kann aus dem Titel
einen sprechenden machen (`ticket/PROJ-123-login-redirect-loop`).

Analyse (auf dieser Maschine geprüft, 09.11.2026):

- `ollama` ist installiert (llama3.1:8b, gemma4:31b, phi4, deepseek-r1:14b); `llm`,
  `llama-cli` und `lms` sind es nicht. Ein 8B-Modell antwortet auf einen Einzeiler in rund
  4–6 s, wenn es geladen ist.
- `ollama run` schreibt eine Vorrede („Here is a kebab-case branch name: …“) nach stdout
  und einen Spinner nach stderr. Der Parser braucht deshalb die letzte nicht-leere Zeile,
  nicht die ganze Ausgabe; `--hidethinking` unterdrückt den Denk-Block von R1-Modellen.
- Ergebnis: kein HTTP-Client, kein Modell-API, keine neue Dependency. Das Kommando ist die
  Schnittstelle, `grover` normalisiert und prüft nur die Antwort (Abschnitt 9 bleibt).

```yaml
ai:
  branch_name: >-
    ollama run llama3.1:8b --hidethinking
    "kebab-case branch name, max 5 words, prefix ticket/: $GROVER_TICKET $GROVER_TITLE"
```

- Ein optionales Top-Level-Feld `ai` mit genau einem Kommando, ausgeführt über `sh -c`.
  stdin ist `null` (sonst greift das Modell-CLI das Terminal ab), stdout wird gefangen,
  stderr geht als Debug-Log nach stderr. Env: `GROVER_TICKET` (Slug), `GROVER_TITLE`.
- Die Antwort *ist* der Branch-Name; es gibt kein `{title}`-Token in `branch_template`.
  `plan::branch_slug` schickt jedes `/`-Segment durch `slug`, damit `ticket/PROJ-123-foo`
  seinen Slash behält; Backticks, Anführungszeichen und Leerzeilen fallen weg.
- Der Titel kommt aus `new --title <text>`. Fehlt er und `ai` ist gesetzt, fragt `new` auf
  einem TTY danach (überspringbar). Ohne Titel läuft das Kommando trotzdem — es kann den
  Titel selbst holen (`curl` gegen Jira), das ist seine Sache.
- Deadline 20 s (`spawn` plus `try_wait`-Polling; die Frist ist Parameter der Hilfsfunktion,
  damit ein Test 50 ms nutzen kann). Fehler, Frist oder leere Antwort: Warnung und
  `branch_template` wie bisher. 20 s decken 8B geladen ab; ein 31B-Modell lädt beim ersten
  Aufruf länger und fällt dann auf den bisherigen Namen zurück. Bewusst kein Config-Feld
  dafür, bis es wirklich stört.
- `--dry-run` führt das Kommando nicht aus und sagt das; ein Prompt für den Branch-Namen
  gibt es nicht (nicht reproduzierbar).
- Tests: Antwort → Branch wie geliefert (normalisiert); Vorrede und Backticks fallen weg;
  `ticket/PROJ-1-A` behält den Slash; Fehler und Frist → bisheriger Name mit Warnung; ohne
  `ai`-Feld bleibt alles unverändert; `--dry-run` führt nichts aus.

### 12.9 Kleinkram (Backlog, erst mit Bedarf)

Klein, aber ohne Bedarf kein Code; der Auslöser steht jeweils dabei.

- **`new --fetch`** — `git fetch --quiet` vor `first_existing`, opt-in. Auslöser: die
  Basis ist sichtbar veraltet (Netz und Latenz, deshalb nicht der Default).
- **`new --base <ref>`** — überschreibt `base` für einen Lauf. Auslöser: ein Ticket geht
  gegen einen Release-Branch statt gegen `main`.
- **`copy` als Symlink** — Feld `link: true` in `CopySpec`, `std::os::unix::fs::symlink`.
  Auslöser: AGENTS.md-Änderungen sollen in alten Tickets ankommen (nur Unix).
- **`copy` von Verzeichnissen** — rekursiv kopieren. Auslöser: eine Vorlage ist ein
  Ordner.

Bereits vorhanden, nur nirgends als Feature genannt: existiert der Ticket-Branch schon im
Repo, nutzt `worktree_add` ihn (`branch_exists`) statt neu zu basen.

### 12.10 Nicht bauen

- `--json`: niemand konsumiert es, `list` und `status` sind schon maschinenlesbar genug.
- Fullscreen-TUI, Maus, History/Undo.
- Plugins, Template-Engine, Hook-System.
- gh-/PR-Automatik (gemergt → automatisch `rm`): zweite Abhängigkeit und ein Rate-Spiel;
  `rm` bleibt die bewusste Aktion.
- `sync`/`merge` über die Repos eines Tickets.
- `list --long`: dafür gibt es 12.3.
- Setup-Map pro Repo: 12.4 deckt den Bedarf mit einer Env-Var.
