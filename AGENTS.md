# grover — Arbeitsregeln für dieses Repo

`grover` automatisiert meinen Workflow für Projekte mit vielen, teils verbundenen Repos:
pro Ticket ein Ordner, pro Repo ein Worktree darin.

- **Plan:** [PLAN.md](PLAN.md) (Design, Entscheidungen, Status)
- **Beispiel-Config:** [grover.yaml.example](grover.yaml.example)

## Regeln

1. **Maximal 100 Zeichen pro Zeile.** In Rust per `rustfmt.toml` (`max_width = 100`)
   erzwungen; Strings und Kommentare notfalls selbst umbrechen.
2. **Thematisch trennen.** Ein Thema = ein Block; Blöcke werden durch genau eine
   Leerzeile getrennt (Imports, Typen, Hilfsfunktionen, Tests).
3. **Lesbar und maintainable.** Sprechende Namen, frühe Rückkehr statt Verschachtelung,
   keine Abstraktion ohne zweiten Nutzer. Kommentare nur für das *Warum*.
4. **Kleinste vollständige Änderung.** Keine Features, Optionen oder "für später"-Code,
   die niemand angefordert hat. Bewusst Weggelassenes gehört unter *Non-Goals* in
   PLAN.md, nicht in den Code.
5. **Scope vollständig, nicht breiter.** Wer eine Funktion anfasst, prüft alle Aufrufer,
   Tests und die Config-Doku und fixt sie mit.
6. **Grün heißt grün.** `cargo fmt --all --check`,
   `cargo clippy --all-targets -- -D warnings` und `cargo test` müssen durchlaufen.
7. **Kein `unsafe`.** Kein `unwrap`/`expect` außerhalb von Tests.
8. **Shell statt Bibliothek für git.** Wir rufen die `git`-CLI auf (kein `git2`): kein
   C-Build, identisches Verhalten wie im Terminal inkl. Hooks und User-Config.
9. **Trennung der Schichten.** `grover-lib` kennt kein clap und schreibt nichts auf
   stdout. `grover-cli` enthält keine Workflow-Logik: Argumente, Ausgabe, Exit-Code.
10. **Erst Review, dann Rückfrage.** Nach der Umsetzung wird mit Ponytail reviewt und die
    Findings werden gefixt; erst danach kommen Rückfragen an den User.
11. **Plan vor Code.** Änderungen am Verhalten werden zuerst in PLAN.md beschrieben und
    umgesetzt, wenn der User zustimmt.

## Kommandos

```text
cargo test                                   # Unit- und Integrationstests
cargo clippy --all-targets -- -D warnings    # Lints
cargo fmt --all --check                      # Formatierung (100 Zeichen)
cargo run -p grover-cli -- new PROJ-123 -r api -n
make check                                   # alle drei Kommandos oben
make install                                 # Binary nach ~/.local/bin
```

## Layout

```text
Cargo.toml            # virtueller Workspace, gemeinsame Deps und Lints
rustfmt.toml          # max_width = 100
Makefile              # build, install, uninstall, check
grover.yaml.example   # kommentierte Beispiel-Config (YAML)
grover-lib/           # Core: Config, Repo-Discovery, git, Fuzzy, Plan/Apply
grover-cli/           # bin "grover": clap, Logging, Ausgabe
```
