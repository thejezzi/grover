# PLAN — grover

Status: Abschnitte 1–11 und 12.1–12.3 umgesetzt, `make check` grün (81 Tests). Abschnitt 12
ist die priorisierte Liste der nächsten Features; jedes davon bekommt vor dem Code ein Go.
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
    `clone_dir`, `repos`, `copy`; Enter behält den Default, leere Listen landen nicht in
    der Datei. Geschrieben wird erst nach `Config::validate`; eine vorhandene Datei nur
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
gibt, stehen mit ihrer Zielform in Abschnitt 12 (`setup`, 12.4).

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
```

`to` ist relativ zum Ticket-Ordner; ein absoluter Pfad ist ein Config-Fehler.

Unbekannte Felder sind ein Fehler (`deny_unknown_fields`), damit Tippfehler auffallen.
`grover config --init` fragt die Werte ab und schreibt sie, `grover config` ändert sie
interaktiv, `grover config --show` gibt die aufgelöste Config aus, `--path` nur den
Dateipfad (Details in Abschnitt 11).

## 5. CLI

```text
grover new [<ticket>] [-r|--repo <name>]... [-a|--all] [-n|--dry-run]
                      [-c|--config <pfad>]
grover list [-c|--config <pfad>]
grover rm [<ticket>] [-f|--force] [-n|--dry-run] [-c|--config <pfad>]
grover path [<ticket>] [-c|--config <pfad>]
grover status [<ticket>] [-c|--config <pfad>]
grover config [-c|--config <pfad>] [-p|--path] [-s|--show]
                     [-i|--init [--force]]
grover completions <bash|zsh|fish|...>
```

- `new` gibt den Ticket-Ordner auf stdout aus, Fortschritt/Warnungen nach stderr.
- `--repo` ist fuzzy, mehrfach oder kommagetrennt; `--all` nimmt alle.
- Fehlen Ticket, `--repo` und `--all`, fragt `new` nach — aber nur mit TTY (Abschnitt 11.3).
  Ohne TTY bleibt es beim Fehler, es wird nichts angelegt.
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
   aus `branch_template`, Ticket-Ordner aus `work_dir` + Slug.
5. Auswahl per Fuzzy auflösen; Plan bauen (`worktrees`, `copies`).
6. `apply`: Ticket-Ordner anlegen, pro Repo Worktree anlegen (Basis ist der erste dort
   existierende Ref aus `base`), Dateien kopieren. Vorhandene Pfade werden übersprungen,
   nicht überschrieben. Alle Pfade sind vor dem git-Aufruf absolut: ein relatives Ziel
   würde `git -C <repo> worktree add` im Repo anlegen, nicht im Projekt.

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
- Für Abschnitt 12 kommen die Fälle aus 12.1 bis 12.4 dazu (dort je Feature gelistet).

## 9. Non-Goals

- Kein `--json`, keine Fullscreen-TUI (`ratatui`), keine Maus, kein Undo/History.
- Keine Änderung an `git config`, keine git-Hooks. Setup-Befehle nach dem Anlegen sind
  geplant (12.4), aber als Config-Liste, nicht als Hook-System.
- Keine `copy`-Einträge pro Repo (nur Ticket-Root).
- Kein Mergen und kein Aufräumen ohne Aufforderung; Löschen ist `rm` (12.1), nie implizit.
  Kein `fetch` im Normallauf, höchstens opt-in (12.5).
- Keine Doku-Generierung (Handbuch, Manpage); `README.md` und `docs/` werden von Hand
  gepflegt.
- Keine Kommentar-Erhaltung beim interaktiven Schreiben der Config.
- Kein Bearbeiten einzelner Listeneinträge (nur hinzufügen und entfernen).

## 10. Status

- [x] Workspace: virtuelles Manifest, `grover-lib` + `grover-cli`, Lints, `rustfmt.toml`
- [x] Projektname `grover`: Binary, Crates, Config-Pfade, Tests und Doku
- [x] AGENTS.md (Regeln) und PLAN.md (dieser Plan)
- [x] `grover-lib`: `config`, `repo`, `git`, `fuzzy`, `plan`, `error`
- [x] `grover-cli`: `new`, `list`, `config`, `completions`
- [x] `Makefile`: `build`, `install`, `uninstall`, `check`
- [x] `grover.yaml.example` und Tests (19 Unit-, 8 Integrationstests)
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
- [ ] Durchlauf mit deiner echten Config

## 11. Mini-TUI (`config`, `new`)

Drei Eingaben, ein gemeinsamer Unterbau: `grover-cli/src/tui.rs` kapselt `inquire`;
alle Listen filtern beim Tippen, `↑`/`↓` bewegen, `→` wählt alle, `←` keine.
Vor jedem Prompt steht der TTY-Wächter (`std::io::IsTerminal`), sonst passiert genau das,
was heute passiert. Prompts nach stderr, Fortschritt weiterhin über `tracing`.

Ein gemeinsames `RenderConfig` macht die Prompts optisch konsistent: `?` für offene und `✓`
für bestätigte Eingaben, `❯` für die aktive Option, `[✓]` für Mehrfachauswahlen. Cyan markiert
Fokus und Antworten, Grün Bestätigungen; Hilfen und inaktive Checkboxen sind dunkelgrau.
Bis zu zwölf Optionen stehen ohne Blättern untereinander, Werte und Repo-Status sind bündig.
`NO_COLOR` behält Marker und Layout bei, entfernt aber sämtliche Farben und Attribute.

### 11.1 `grover config --init`

- Ziel: `--config <pfad>`, sonst derselbe Fundort wie beim Lesen
  (`./grover.yaml`, sonst `$XDG_CONFIG_HOME/grover/config.yaml`).
- Keine Vorlage mehr: das Binary steht für sich, es schreibt nur die Werte, die du
  eingibst. Leere Listen (`repo_dirs`, `repos`, `copy`) erscheinen nicht im YAML.
- Ein Durchlauf, Feld für Feld: `work_dir` (Pflicht, wird bis zur Eingabe wiederholt),
  `branch_template`, `base`, `repo_dirs`, `clone_dir`, `repos`, `copy`. Enter behält den
  angezeigten Default, Listen überspringt man über `← back`.
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
  beenden
```

- Eingabe je Feld über `Input`; leere Eingabe behält den alten Wert. Validiert wird durch
  Neuladen (`Config::load`): schlägt es fehl, wird die Datei auf den Stand davor
  zurückgeschrieben und der Fehler als Warnung gemeldet. Keine doppelte Logik.
- Listen (`repo_dirs`, `repos`, `copy`) laufen über einen gemeinsamen Untermenü-Helfer:
  Einträge anzeigen, hinzufügen, entfernen. Eine Hilfsfunktion für drei Felder, kein
  Zustandsautomat für „bearbeiten“.
- `repos` fragt `url` und `name` (leer = aus der URL), `copy` fragt `from` und `to`
  (leer = Dateiname), `repo_dirs` einen Pfad je Eintrag.
- Geschrieben wird nach jeder Menüaktion; ein Listeneintrag landet also beim Verlassen des
  Untermenüs in der Datei. Geschrieben werden absolute Pfade, die Kommentare der Datei
  gehen dabei verloren (Entscheidung 12).
- `--path` bleibt (Fundort), neu `--show` (aufgelöste Config, bisheriges Verhalten des
  nackten `config`). Ohne TTY und ohne Flag: Fehler mit Hinweis auf `--show`.
- Abbruch: `Esc` und `Ctrl-C` brechen jeden Prompt ab, Exit 1, nichts geschrieben;
  „exit“ im Menü ist Exit 0. Prompt-Texte sind englisch wie der Rest der CLI.

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

Die Reihenfolge ist Nutzen pro Aufwand, nicht Wunschliste. 12.4 ist offen und bekommt vor
dem Code sein Go; die Tests aus Abschnitt 8 gelten mit. 12.5 ist Backlog, 12.6 ist die
Absage.

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

### 12.4 Setup-Befehle nach dem Anlegen

Nach jedem Worktree folgt derselbe Handgriff: `.env` kopieren, `npm install`,
`direnv allow`. Heute macht das die Hand oder ein Shell-Skript danach.

```yaml
setup:                               # nach jedem neuen Worktree, cwd = Worktree
  - "cp .env.example .env"
```

- Eine Top-Level-Liste, kein Feld pro Repo: die Befehle sehen `GROVER_REPO` und
  `GROVER_TICKET`, ein `case "$GROVER_REPO"` reicht für den Sonderfall. Eine Map
  Name→Befehle wäre die nächste Stufe, keine zweite Config-Fläche zum Start.
- Ausführung über `sh -c`, sequenziell, cwd = neuer Worktree, stdout/stderr geerbt (die
  Install-Ausgabe soll sichtbar sein).
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

### 12.5 Kleinkram (Backlog, erst mit Bedarf)

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

### 12.6 Nicht bauen

- `--json`: niemand konsumiert es, `list` und `status` sind schon maschinenlesbar genug.
- Fullscreen-TUI, Maus, History/Undo.
- Plugins, Template-Engine, Hook-System.
- gh-/PR-Automatik (gemergt → automatisch `rm`): zweite Abhängigkeit und ein Rate-Spiel;
  `rm` bleibt die bewusste Aktion.
- `sync`/`merge` über die Repos eines Tickets.
- `list --long`: dafür gibt es 12.3.
- Setup-Map pro Repo: 12.4 deckt den Bedarf mit einer Env-Var.
