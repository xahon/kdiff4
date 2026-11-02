# Kdiff4

Rust re-implementation of kdiff3: 2-way/3-way file diff and merge tool with a GUI. License: GPL-2.0-or-later.

## Source of truth

- kdiff3 at `../kdiff3` (HEAD `33e56f0c`, v1.14.70) is the behavioral spec. Read it to answer "what should this do"; never edit it.
- The goal is equivalent functionality, not a line-by-line port. Write idiomatic Rust; don't mirror kdiff3's class structure.
- Behavior matches kdiff3 except where `docs/DEVIATIONS.md` says otherwise. Every intentional difference and every fixed kdiff3 bug goes there.
- Visual match is not required; functional match is.

## Docs

- `docs/ARCHITECTURE.md`: workspace layout, crate dependency rules, data model, protocol, config, GUI design, CLI, glossary.
- `docs/DEVIATIONS.md`: fixed kdiff3 bugs and intentional differences.

Keep these up to date in the same change that makes them stale.

## Workspace layout

The crates, their responsibilities and the allowed dependencies between them are defined in `docs/ARCHITECTURE.md`. Follow those rules; the key ones:

- The engine never depends on GUI or windowing crates.
- Front ends never call the engine directly: all communication goes through the protocol crate.

## Commands

```
cargo build
cargo test
cargo run -- base.txt left.txt right.txt -o out.txt
```

- Don't rely on clippy.
- No `rustfmt.toml` exists, so don't run rustfmt. Match the surrounding style by hand.

## Conventions

- Errors: `thiserror` enums in library crates; `anyhow` with `.context(...)` in the binary.
- No bare `unwrap()` outside tests. Hard failures use `.expect("<Subsystem>: message")`, e.g. `.expect("Eframe: failed to start")`.
- No `unsafe` without the owner's approval. Every `unsafe` block gets a `// SAFETY:` comment.
- Prefer explicit `.clone()`, even for `Copy` types. Avoid `Rc`; design ownership so it isn't needed.
- Iterator pipelines and long chains: one call per line. Short chains stay on one line.
- Exactly two empty lines between definitions (functions, types, impl blocks, modules). Fields and trait method declarations stay adjacent.
- Prefer long lines to wrapping.
- Comments only where the code would otherwise be obscure. A reference to the kdiff3 source (`// kdiff3: diff.cpp, calcDiff3LineListUsingAB`) is fine where it explains non-obvious behavior.
- No new dependencies without approval.
- American English spelling everywhere: code, comments, docs.
- In docs, refer to files by name in backticks, not with markdown links.

## Known kdiff3 bugs

Listed in `docs/DEVIATIONS.md`. When porting an area, check that list first and implement the fixed behavior, not kdiff3's.

## Testing

- Every change comes with tests; write them first where practical. Every functional Rust module should have its own set of tests.
- Every bug fix, and every fixed kdiff3 bug, gets a regression test.
- Port kdiff3's test data (`../kdiff3/test/testdata/`) and alignment tests (`../kdiff3/test/alignmenttest.cpp`, `../kdiff3/src/autotests/`) into `tests/data/` and Rust tests.
- Use `insta` snapshots for alignment and merge output, `proptest` for invariants (every input line appears exactly once per side, in order; result of choosing a side reproduces that side).
- Engine behavior is tested headless, both directly and through the same protocol path the GUI uses; the GUI layer stays thin and dumb.

## GUI rules

- Panes are virtualized: lay out only visible rows. Never build a widget per line for the whole file.
- Never block the UI thread: long work runs in the engine, which reports progress and can be canceled.
- Every user action (menu, shortcut, button) is sent to the engine through the protocol. No diff, alignment or merge logic in GUI code.
- All commands are reachable by keyboard.

## Compatibility contracts

- CLI: drop-in replacement for kdiff3 in `git mergetool` and `git difftool`. Options are specified in `docs/ARCHITECTURE.md`.
- Exit codes follow kdiff3: zero when the output was saved, non-zero when the user quit without saving or on error.
- Saving preserves the input's encoding, BOM and EOL style, and is atomic (temp file + rename).

## Definition of done

1. `cargo build` with no new warnings.
2. `cargo test` passes.
3. New behavior has tests; bug fixes have regression tests.
4. `ARCHITECTURE.md` and `DEVIATIONS.md` updated if affected.
5. New high-level coding entities are updated in `ARCHITECTURE.md` glossary. Removed entites are removed from glossary.
6. Work is left uncommitted in master or commited into a feature branch. Prefer leaving uncommited and unstaged in master.
