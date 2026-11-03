# Kdiff4 architecture

Kdiff4 is a Rust re-implementation of kdiff3 (`../kdiff3`, HEAD `33e56f0c`, v1.14.70). kdiff3 is the behavioral spec, not a code template: the goal is equivalent functionality, not a line-by-line port. Known kdiff3 bugs are fixed and listed in `DEVIATIONS.md`.

## Workspace layout

```
kdiff4/
  Cargo.toml            workspace root
  crates/
    kdiff4-core/        engine: text loading, diff, alignment, merge model, engine config. Runs the host. No GUI deps.
    kdiff4-protocol/    messages between the engine and front ends, client and host endpoints, transports.
    kdiff4-gui/         egui/eframe front end.
  src/main.rs           `kdiff4` binary: CLI parsing (clap), headless mode, wires the host to a front end.
  tests/data/           test inputs ported from kdiff3 `test/testdata/` plus our own.
  docs/
```

Dependency rules:

```
kdiff4-gui  ──►  kdiff4-protocol  ◄──  kdiff4-core
     ▲                                      ▲
     └───────────── kdiff4 (bin) ───────────┘
```

- `kdiff4-protocol` depends on no other kdiff4 crate and on no GUI or engine crate.
- `kdiff4-core` and `kdiff4-gui` depend on `kdiff4-protocol` and never on each other. Core never depends on egui, eframe or any windowing crate, so everything except rendering is testable headless.
- The binary is the only crate that depends on both core and GUI, and only to wire them: it starts the host, connects a client to it and hands the client to the GUI or to the headless driver. After that it talks to the engine only through the client, like any other front end.

## Modules

| Crate | File | Responsibility |
|---|---|---|
| protocol | `messages.rs` | the message vocabulary: commands, responses, events (snapshot, delta and its changes) and the small types both sides share, such as document, labels, EOL style, revision and protocol error |
| protocol | `transport.rs` | the client and host endpoints and the in-process transport, with the waker that tells a front end a message arrived |
| core | `text.rs` | decoding, line splitting, line-ending and BOM detection, and atomic writes |
| core | `session.rs` | one open comparison: the sources, D and its history, the revision, and load, edit and save |
| core | `undo.rs` | D's undo steps and the saved/modified tracking |
| core | `host.rs` | the host thread: turns each command into a response plus a snapshot or delta |
| core | `error.rs` | core's errors and their conversion to protocol errors |
| gui | `lib.rs` | starts the window with what the binary already knows: pre-filled paths, labels and a pending open request |
| gui | `app.rs` | the application loop: menus, shortcuts, sending commands and handling their results, the status bar, and routing to dialogs and panes |
| gui | `mirror.rs` | the front end's copy of the session, built from snapshots and deltas; applies D edits at once and resyncs on a gap or a rejected edit |
| gui | `panes.rs` | pane layout, pane headers with path fields and Browse, and the splitter above the merge pane |
| gui | `text_view.rs` | the virtualized text widget: draws only visible rows, the caret and selection, and maps pointer input to positions |
| gui | `editing.rs` | caret motion, word and line selection, and turning keys into edits of D, independent of egui |
| gui | `open_dialog.rs` | the Open form, its Run rule, file pickers and the overwrite question |
| gui | `settings_dialog.rs` | the Settings dialog with live preview |
| gui | `config.rs` | `gui.toml`: loading, saving and backing up a broken file |
| gui | `fonts.rs` | discovering monospace font families and installing the chosen one |
| gui | `theme.rs` | palettes and the egui style, including corner radius and pane frames |
| gui | `window_frame.rs` | the native window's corner rounding (Windows only) |
| gui | `paths.rs` | cleaning path text from the command line, the Open form and pane headers |
| bin | `main.rs` | the command line, mapping it to a launch, and wiring the host to the GUI |
| bin | `logging.rs` | the logger: rotating files next to the executable, stdout, the level override and the panic hook |


## Crates

| Purpose | Crate | Where |
|---|---|---|
| Errors | `thiserror`, `anyhow` | all |
| Logging | `log` | all |
| Protocol messages | `serde` | protocol |
| Line diff | `imara-diff` (Histogram default, Myers optional) | core |
| Encoding decode/encode | `encoding_rs` | core |
| Encoding detection | `chardetng` | core |
| Directory compare (v1.1) | `walkdir`, `ignore` | core |
| Config | `serde`, `toml`, `dirs` | core, gui |
| GUI | `egui`, `eframe`, `egui_extras` | gui |
| File dialogs | `rfd` | gui |
| Font discovery and monospace detection | `fontique`, `skrifa` | gui |
| CLI | `clap` | bin |
| Log output | `flexi_logger` | bin |
| Snapshot tests | `insta` | dev |
| Property tests | `proptest` | dev |


## Glossary

### Documents

- **A**: placeholder for Base file (common ancestor version of the file). Readonly.
- **B**: placeholder for Left file. Readonly.
- **C**: placeholder for Right file. Readonly.
- **D**: placeholder for Merged output file. Read-Write.
- **Document**: the generalized name of A, B, C, or D, which could mean any of them.
- **Source Document**: either A, B, or C.
- **Session**: one open comparison: the source documents, their regions and rows, manual alignments, D, undo history and the engine config in effect. Owned by the host.

### Text

- **Source text**: decoded content of one source document, together with its encoding, BOM flag, EOL style and whether the last line has a terminator.
- **EOL style**: line terminator of a document: LF, CRLF, CR or mixed.
- **Binary file**: a file with NUL bytes in its first chunk. Compared by bytes only, never merged.
- **Column**: position inside a line, counted in Unicode characters, not bytes or UTF-16 units.

### Comparison

- **Normalizer**: turns a line into its line key according to the active ignore options.
- **Line key**: normalized form of a line; two lines are equal when their keys are equal.
- **Token**: integer id of an interned line key. Diffs run on tokens, not text.
- **Hunk**: the lines of one source document that belong to one region: a range of consecutive lines, possibly empty. In a changed region it is the chunk of code that was added, removed or modified. "Hunk A", "hunk B" and "hunk C" mean the hunks of A, B and C in the region being discussed.
- **Void**: an empty hunk. The document has no lines in that region, because the text was added in another document or removed from this one.
- **Hunk change**: how hunk B or hunk C relates to hunk A:
  - **Unchanged**: equal to hunk A.
  - **Added**: hunk A is a void, this hunk is not.
  - **Removed**: this hunk is a void, hunk A is not.
  - **Modified**: both have lines, and they differ.
- **Region**: a group of hunks, one per source document, that cover the same part of the text and are diffed or merged as a unit. The regions of a session cover every line of every source document exactly once, in document order. Unchanged text forms regions too, so D is built entirely from regions.
- **Diff Region**: a region of two hunks, A and B. Used in 2-way mode.
- **Merge Region**: a region of three hunks, A, B and C. Used in 3-way mode.
- **Region status**: what the hunks of a region have in common.
  - Diff Region: the hunk change of hunk B (Unchanged, Added, Removed or Modified).
  - Merge Region: one of the following, which together cover every possible case:
    - **Unchanged**: hunks A, B and C are equal.
    - **Changed in B**: hunk B differs from hunk A; hunk C equals hunk A.
    - **Changed in C**: hunk C differs from hunk A; hunk B equals hunk A.
    - **Changed alike**: hunks B and C are equal and differ from hunk A.
    - **Conflict**: hunks A, B and C all differ from each other.
    - **Whitespace conflict**: a conflict where hunks B and C differ from each other only in whitespace.
- **Row**: one horizontal line of the synchronized view. For each source document a row holds either one of its lines or nothing. All panes show the same row at the same height, which is how they scroll together. A region is a run of consecutive rows. Inside a region, similar lines of different documents are put in the same row, so their character-level differences can be shown side by side.
- **Void Region**: the rows in a pane that have no source text: the rows of a void, and the padding below a hunk that is shorter than another hunk of the same region. Drawn hatched.
- **Character-level diff**: the diff between the lines that share a row. Computed only for visible rows.
- **Split**: dividing one region into smaller regions at row boundaries.
- **Manual alignment**: a user-defined set of parts, one per source document, that must share rows. See Manual alignment.
- **Alignment part**: one source document's share of a manual alignment: a line range or an anchor.
- **Anchor**: an alignment part that marks a position between two lines, used when the aligned content does not exist in that document.
- **Crossing alignments**: two manual alignments whose order differs between source documents. Rejected.
- **Segment**: a stretch of a source document between two consecutive manual alignments, or one alignment itself. Aligned independently of other segments.

### Merge

- **Choice**: which hunks of a region D takes its lines from, in order, or unresolved.
- **Auto-resolve**: choosing automatically for every region whose status allows it.
- **Unresolved conflict**: a region without a choice. D cannot be saved while one exists.
- **Edited region**: a region whose lines in D were changed by hand. The edited lines replace the lines from its choice.
- **Merge op**: one mutation of the merge result together with its inverse.
- **Undo step**: a group of merge ops undone and redone together.
- **Output reference**: the source document whose encoding, BOM and EOL style D is saved with.

### Protocol

- **Engine**: `kdiff4-core`: everything that computes diffs, alignment and merge results.
- **Front end**: anything that talks to the engine: the GUI, the headless CLI driver, integration tests, a future remote UI.
- **Host**: the engine-side endpoint. Owns the session and processes requests.
- **Client**: the front-end-side endpoint. Sends requests, receives responses and events.
- **Transport**: carries messages between one client and one host. In-process in v1.
- **Request**: a message from client to host, carrying a command or a query.
- **Command**: a request that changes the session.
- **Query**: a request that reads data the client mirror does not hold.
- **Response**: the host's answer to one request: a result or a protocol error.
- **Event**: an unsolicited message from host to client: progress, snapshot, delta, notice.
- **Revision**: a counter that the host increments on every change to the session.
- **Snapshot**: the full session state the client needs to render, sent after open and on resync.
- **Delta**: the changes made by one revision.
- **Mirror**: the client's copy of the session state, built from the snapshot and deltas. Front ends render from it.

### Config

- **Engine config**: options that change diff, alignment, merge or save results. Stored in `config.toml`, owned by the engine.
- **GUI config**: presentation preferences. Stored in `gui.toml`, owned by the GUI.
- **Override**: a `--cs` setting that applies for one run and is never written back.

### GUI

- **Pane**: the view of one document.
- **Merge pane**: the pane showing D. A custom editor widget.
- **Overview strip**: the 4-line lens row between the source panes and the merge pane.
- **Caret**: the one text cursor on screen, with its selection. It lives in one pane at a time; pressing the mouse button in another pane moves it there.
- **Splitter**: the draggable border between the source panes and the merge pane.
- **Open dialog**: the window where A, B, C and D are chosen before a session opens.
- **Settings dialog**: the window where GUI config is edited, with a live preview.
- **Theme**: light or dark appearance of the GUI.
- **Palette**: the colors one theme draws with: window and pane backgrounds, pane border, pane header, text.

## Core data model (`kdiff4-core`)

### Source text

- Each source document is decoded once into a single UTF-8 string. Lines are ranges into that string, without their terminators, so loading does no per-line allocation.
- Alongside the text the engine keeps the encoding, whether a BOM was present, the EOL style (for mixed files the majority style is used on save) and whether the last line lacks a terminator.
- Encoding: BOM first, then user override, then `chardetng`, then UTF-8 fallback. Invalid bytes become U+FFFD and the file is flagged so save warns before lossy re-encode.
- Binary detection: NUL bytes in the first chunk mark the file as binary; binary files are compared by bytes only and are not merged.

### Line comparison

Lines are compared through their line keys, produced by a normalizer configured from options (ignore whitespace, ignore case, ignore trailing whitespace, ignore numbers, ignore comments). Line keys are interned into integer tokens before diffing, so the diff runs on integers.

### 2-way diff

A thin wrapper over `imara-diff`: it diffs the token sequences of two documents with the configured algorithm and returns the changed ranges, which become Diff Regions. The unchanged stretches between them become Unchanged Diff Regions.

### Regions and rows

3-way mode builds Merge Regions from two 2-way diffs:

1. Diff A with B and A with C. Each diff gives a list of changed ranges of A.
2. Walk both lists along A. A change that only B or only C made becomes a Merge Region of its own. Changes from B and C that overlap, or meet at the same position of A, join into one Merge Region spanning their union. Stretches of A that neither side changed become Unchanged Merge Regions.
3. The status of each region follows from comparing its hunks.
4. The lines of each region are paired into rows. Equal lines share a row. In regions where both B and C changed, B and C are diffed with each other, so lines equal between them share a row instead of being stacked. Lines left unpaired get rows of their own, padded with void regions in the other panes.

2-way mode is the same process with only A and B.

Character-level diffs are computed lazily for visible rows and cached; above a line length limit the whole line is marked as different.

kdiff3 reference: `diff.cpp`, `pdiff.cpp`, `test/alignmenttest.cpp`. We reuse its test data, not its algorithm.

### Splitting regions

The user can split a region into smaller ones at row boundaries, like staging single lines in `git gui`. Each part gets its own hunks, status and choice, so a large conflict can be resolved piece by piece; parts that turn out to be one-sided auto-resolve.

- The choice and edits of the original region are dropped; the parts start unresolved unless their status auto-resolves.
- Splitting is one undo step.
- A recomputation of the affected segment (options change, reload, manual alignment change) drops its splits.

### Manual alignment

The user can override the automatic alignment by telling the engine which parts of the source documents belong together.

**Why.** Automatic alignment pairs lines by content and position, which goes wrong when unrelated code lands in the same place. Typical case: B introduces function "Foo" after "Bar" and before "Baz"; C introduces a different function "Qux" at the same spot. Both changes touch the same place in A, so the engine builds one conflict region and pairs look-alike lines of the two unrelated functions (braces, `return`, blank lines) into rows. The user wants both functions in D, one after the other. Manual alignments let them say so.

**Alignment.** A manual alignment has one part per source document (A, B and C in 3-way mode; A and B in 2-way mode). Each part is either:

- a line range: one or more lines the user selected in that document, or
- an anchor: a position between two lines (or at the start or end of the file), used when the aligned content does not exist in that document.

All parts of one alignment occupy the same rows. Anchors, and ranges shorter than the longest part, are padded with void regions. The session keeps its manual alignments in one list, in document order.

**Ordering rules.**

- Alignments must not cross: if alignment X comes before Y in one source document, X comes before Y in every source document.
- Line ranges of different alignments must not overlap within a document.
- Several anchors may share one position. When a new anchor lands on a position that already holds anchors, the user specifies its order among them. This order decides how the aligned contents are ordered in the rows and in D, and it must agree with the order of the same alignments in the other documents; otherwise the alignments cross.

Example: Foo and Qux from above are separated with two alignments.

| | A | B | C |
|---|---|---|---|
| 1 | anchor between Bar and Baz, first | Foo | anchor between Bar and Qux |
| 2 | anchor between Bar and Baz, second | anchor between Foo and Baz | Qux |

Alignment 1 turns Foo into a Merge Region with voids in A and C, status Changed in B; alignment 2 does the same for Qux, status Changed in C. Both auto-resolve, so D gets Foo followed by Qux with no conflict. To get Qux first, alignment 2 goes first: its anchor in B moves between Bar and Foo, alignment 1's anchor in C moves between Qux and Baz, and the anchor order in A flips.

**Effect on the engine.**

- Alignments cut every source document into segments: the alignments themselves and the stretches between consecutive alignments. Each segment gets its regions and rows on its own with the process above; rows and regions never span a segment boundary.
- Both ends of an alignment are forced region boundaries, so its contents never merge with anything outside it. Inside an alignment, lines pair up and regions form as usual.
- Adding or removing an alignment is one undo step. Regions and rows are recomputed only for the affected segments; regions there lose their choices, edits and splits, regions elsewhere keep them.

**Invalid alignments.** Crossing or overlapping alignments are rejected: the GUI shows an error message box, the attempt is logged, and the alignment list stays unchanged. Supporting crossing alignments is a possible later feature: it would allow merging three files even when functions were reordered relative to each other in all three source documents.

kdiff3 reference: manual diff help in `diff.cpp`. Differences are listed in `DEVIATIONS.md`.

### Merge model

- D is built from the regions in order. Each region contributes the lines of the hunks in its choice, in the chosen order, or its edited lines.
- Auto-resolve for Merge Regions:
  - Unchanged: hunk A.
  - Changed in B: hunk B.
  - Changed in C: hunk C.
  - Changed alike: hunk B.
  - Whitespace conflict: the configured default side, if one is set; otherwise unresolved.
  - Conflict: unresolved.
- 2-way merge (two files and an output): Unchanged Diff Regions take hunk A; every other Diff Region is unresolved, except whitespace-only differences when a 2-way default side is configured.
- Editing D inside a region turns it into an edited region.
- Undo/redo: every mutation is a merge op with a stored inverse. Groups of ops (e.g. "choose B everywhere") are one undo step.
- Save writes D with the encoding, BOM and EOL style of the output reference (A by default, configurable), atomically via temp file + rename.

### Session

The session holds everything about one open comparison (see Glossary). The host applies each command to the session and turns the result into a response plus a delta. Unit tests apply commands to a session directly; integration tests go through a client, like the GUI.

## Protocol (`kdiff4-protocol`)

All communication between the engine and anything else goes through this crate: it defines the messages, the two endpoints and the transports between them. Replacing the in-process transport with a network one must not require changes in core or GUI.

### Roles

- **Host**: lives in `kdiff4-core` and runs on its own thread. Owns the session, handles requests one at a time in arrival order, and emits events.
- **Client**: the front end's endpoint. Non-blocking: the GUI drains it once per frame and never waits on it.
- **Transport**: v1 has one, an in-process pair of channels; messages are moved, not serialized. A socket transport that serializes messages can be added later without touching core or GUI.

### Messages

| Direction | Message | Content |
|---|---|---|
| client → host | Request | an id plus a command or a query |
| host → client | Response | the id of the request it answers plus a result or a protocol error; exactly one per request |
| host → client | Event | progress, snapshot, delta or notice |

- Commands: open, reload, cancel, choose hunks for a region, choose a side for all unresolved regions, split a region, edit D, add or remove a manual alignment, undo, redo, save, set engine config. Every user action in any front end ends up as a command.
- Implemented so far: open; reload one source document from a (possibly new) path, keeping D; edit D (replace a byte range, tagged with the revision it was made on; a stale revision or a range off character boundaries is rejected); set D's path without writing it (or clear it, making D unnamed); save; save as; undo and redo of D; resync (the host re-sends a snapshot). Deltas so far: a source document replaced, D edited (with whether D now differs from its saved text, which undo can bring back), D's path changed, D saved.
- D's undo history is kept by the host. Typing on one line is one undo step, and so is a run of Backspace or of Delete; a line break, a paste, replacing a selection, a word deletion, save, undo and redo start a new step. Undoing back to the saved text clears the modified flag.
- D's text crosses the protocol with `\n` line ends. It is saved as UTF-8 with the EOL style and BOM of A, the output reference.
- Queries: reads the mirror does not hold: character-level diffs for a row range, navigation (next or previous difference, conflict or unresolved conflict from a row), config help text.
- Progress events report long work (job, amount done, total); notices carry warnings such as a pending lossy re-encode.
- The first exchange is a handshake with the protocol version; a mismatch is an error.

### State mirroring

- After open, the host sends a snapshot: document texts and metadata (labels, encoding, EOL style), regions, rows, D, manual alignments, engine config and the revision.
- Every change increments the revision and is broadcast as a delta (replaced ranges of regions, rows and D lines). The client applies deltas in order; on a gap in revisions it requests a new snapshot.
- Front ends render from the mirror only and never compute diffs, alignment or merge results themselves. View state (scroll position, cursor, selection, focus) stays in the front end.
- Typing in D: the client applies the edit to its mirror at once and sends it as a command tagged with the revision it was based on. The host is the authority. The delta caused by an accepted edit carries the request id, so the client does not apply it twice; a rejected edit makes the client drop its local change and resync. One editing client per session in v1.
- Character-level diffs are queried for the visible rows plus a margin and cached in the mirror; a delta touching those rows evicts them.

### Threading and cancellation

- The host's request loop never blocks on long work: load, diff and align run on host worker threads with a cancel flag and report progress. A cancel command stops the running job. Requests that depend on its result wait in order until it finishes or is canceled.
- The in-process transport takes a wake-up callback from the front end; the GUI passes one that requests a repaint, so it sleeps while idle and wakes when a message arrives. The callback keeps the protocol crate free of GUI types.

### Message rules

- Messages are plain owned data that can move between threads and be serialized with `serde`; no references, shared pointers, file handles or callbacks.
- File paths in messages refer to the host's file system.
- Errors cross the boundary as a protocol error (kind plus message), never as core's own error types.
- Small vocabulary types shared by both sides (document, hunk change, region status, choice, EOL style, engine config) are defined in the protocol crate and used by core directly. Core internals (source text, tokens, the undo stack) never appear in messages.

## Config

### Files

Config lives in two TOML files in the per-user config directory reported by `dirs`, under `kdiff4/`:

| Platform | Directory |
|---|---|
| Windows | `%APPDATA%\kdiff4\` |
| Linux | `$XDG_CONFIG_HOME/kdiff4/`, default `~/.config/kdiff4/` |
| macOS | `~/Library/Application Support/kdiff4/` |

| File | Owner | Contents |
|---|---|---|
| `config.toml` | engine | engine config: options that change diff, alignment, merge or save results |
| `gui.toml` | GUI | GUI config: presentation preferences |

The split follows the protocol boundary: the engine and the GUI may run in different processes or on different machines, so each side reads and writes only its own file. `--config FILE` replaces the path of `config.toml` for one run (kdiff3 compatible); `gui.toml` has no override.

### Engine config (`config.toml`)

Sections with representative keys; the config definitions in code are the authoritative list. kdiff3 option names in parentheses.

- `[diff]`: algorithm (Histogram or Myers), ignore whitespace, ignore trailing whitespace, ignore case (`IgnoreCase`), ignore numbers (`IgnoreNumbers`), ignore comments (`IgnoreComments`), try hard (`TryHard`), pair B and C lines in conflict regions (`Diff3AlignBC`), line length limit for character-level diffs.
- `[merge]`: auto-resolve on open, default side for whitespace-only differences in 2-way and 3-way merges (`WhiteSpace2FileMergeDefault`, `WhiteSpace3FileMergeDefault`), save and quit when no conflicts remain (`AutoSaveAndQuitOnMergeWithoutConflicts`).
- `[encoding]`: default encoding per source document, Unicode auto-detection, same encoding for all inputs (`SameEncoding`), output encoding (`AutoSelectOutEncoding` or explicit).
- `[save]`: output reference, EOL style of D, create `.orig` backup files (`CreateBakFiles`).

### GUI config (`gui.toml`)

- `[view]`: font family (`font_family`, monospace only; absent means the built-in font), font size (`font_size`, 6 to 48), font hinting (`font_hinting`), subpixel glyph positioning (`subpixel_positioning`), corner radius of widgets, dialogs, panes and the window frame (`corner_radius`, 0 to 12), tab size (`TabSize`), replace tabs (`ReplaceTabs`), show line numbers (`ShowLineNumbers`), show whitespace (`ShowWhiteSpace`, `ShowWhiteSpaceCharacters`), auto-advance after choosing (`AutoAdvance`, `AutoAdvanceDelay`), Escape quits (`EscapeKeyQuits`).
- `[colors]`: foreground, background, removed, added, modified, conflict, manual alignment ranges, void regions.
- `[window]`: size, position, maximized, full screen.
- `[recent]`: recent A/B/C/output files and encodings, directory of the last file confirmed in a file dialog (`last_dir`).

### Precedence

1. Built-in defaults: every key has one.
2. The owner's config file.
3. Overrides from `--cs section.key=value`: applied for this run only and never written back.

Each owner keeps two copies: the file config and the effective config (file plus overrides). Edits in the settings dialog change both and save the file config, so an override never leaks into the file unless the user changes that key in the dialog.

The binary routes each override by its section to the engine (in the open command) or to the GUI. An unknown section or key, or a value of the wrong type, is a config error: the message is shown and the program exits with code 1, as kdiff3 does. `--confighelp` prints every key of both files with its effective value and exits.

### Loading and saving

- Missing file: defaults are used; the file is created on the first save from the settings dialog.
- Missing keys take their defaults, so files from older versions keep loading.
- Unknown keys are logged at warn level and ignored, so files from newer versions load too; they are dropped on the next save.
- A top-level `version` key allows migrations when the format changes: older versions are upgraded on load.
- A file that fails to parse is logged and reported (a GUI notice, or stderr when headless), defaults are used for that run, and the file is left untouched. If the user then saves settings, the broken file is first renamed to `<name>.bak`.
- Writes are atomic (temp file + rename), like saving D.

### Over the protocol

- The engine config belongs to the session: it is part of the snapshot, and front ends change it with a set-engine-config command that says whether to persist it. If comparison options changed, the host recomputes regions and rows as a cancelable job and sends a new snapshot.
- The GUI config never crosses the protocol.
- Importing `kdiff3rc` and accepting kdiff3 key names in `--cs` are v1.1 nice-to-haves.

## GUI (`kdiff4-gui`)

- Layout:
  - Two (2-way diff) panes side by side (A/B), Three (3-way merge) panes side by side (A/B/C) and a horizontal pane full width below for output merge result (D).
  - A session is a merge when it has C or an output file. Merges show the merge pane below the source panes, behind a splitter. It starts at half the height and keeps its share when the window is resized.
  - Each pane has a header with the document letter (with `*` while D has unsaved changes), its label, an editable path field and a Browse button. Enter in the field, or a file picked with Browse, reloads A, B or C from that path and keeps D's edits; for D it only changes where D will be saved.
  - Panes show a caret and its selection; there is one on screen. Pressing the mouse button in any pane moves the caret there at once, and dragging selects (Shift+click extends). Clicking below the text goes to its end, and right of a line to the line's end. Double-click selects a word (or a run of whitespace, or one punctuation character), triple-click a line. The keys are arrows, Ctrl+Left/Right by words, Home/End, PageUp/PageDown and Ctrl+Home/End, with Shift to select, plus Ctrl+A and Ctrl+C. A, B and C are read-only. D is editable: typing, Enter (also with Ctrl held), Backspace, Delete, Ctrl+Backspace and Ctrl+Delete by words, Ctrl+V and Ctrl+X, which replace or remove a selection; Edit → Undo (Ctrl+Z) and Redo (Ctrl+Y, Ctrl+Shift+Z) work on D wherever the caret is. Each edit is applied to the mirror at once and sent to the engine.
  - File → Save writes D to its path, or asks for one like Save As when D has none. Save As picks a new path with the native Save dialog, which asks before overwriting, and D moves there. The outcome of saving, reloading and path changes shows in the status bar under the panes. The status bar is always there, one line high, so the panes never move when a message comes or goes; long messages are cut, with the full text in a tooltip.
  - Overview strip: a 4-line lens row between (A/B/C) and (D), shows current lines from all A, B, C, and D. D is allowed to be edited in this widget, and it automatically reflected in the D text. Shows char-diffs within each A, B, and C lines, so the contents could be judged by user and edited as the user wants.
- Void Region: a special graphical entity that spans the rows of a region where a document has no text, and it represents absence of text in the specified document. Graphically represented as hatched block of space with no text in it.
- Scrolling of the three panes is synchronized through row indices, not pixels. Void Regions take rows like text does, so synced scrolling stays aligned.
- The merge pane is a custom widget, not `TextEdit`: own cursor, selection, clipboard, IME, and region coloring.
- The GUI talks to the engine only through its protocol client and renders from the client mirror.
- Long work (load, diff, align) runs in the host; the GUI shows its progress events, keeps repainting and can cancel.
- Dialogs: open files, save-as, settings, find, go to line, "unsaved changes" confirm.
- Open dialog: shown at start and from File → Open. Each document has a path field and a Browse button that opens a native file dialog; D gets a save dialog, since it may not exist yet. Every file dialog starts in `last_dir`, which is updated whenever a file is confirmed. Run needs A and B; C makes a 3-way merge, and D names the output file (a merge without D keeps D in memory until it is saved). If D is an existing file, Run asks before overwriting it. Until the merge engine exists, D starts with the existing output file's contents, or empty.
- Corner radius: applied to widgets, menus, dialogs and panes. On Windows 11 the native window frame follows it too, through the DWM corner preference: 0 is square, 1 to 4 is small rounding, more is normal rounding.
- Theme: light for now. Both themes have a palette and are styled at startup, so switching to dark later needs no other change. Panes are bordered boxes with a header strip; their text uses the palette's text color, which has at least 7:1 contrast with the pane background.
- Settings dialog: Settings → Configure Kdiff4... (Ctrl+,). It edits the `[view]` font keys and the corner radius. The font list shows only installed monospace families, found on a background thread, plus the built-in font. Changes preview live; OK saves `gui.toml`, Cancel reverts.
- Colors:
  - Red (removed hunks, chars)
  - Green (added hunks, chars)
  - Magenta (modified hunks, chars)

## Binary and CLI

Usage: `kdiff4 [OPTIONS] FILE1 [FILE2] [FILE3]`

| Option | Meaning |
|---|---|
| `FILE1 FILE2 FILE3` | 3-way: A, B, C. With two files: 2-way, A and B. |
| `-b`, `--base FILE` | explicit A; the positional files become B and C |
| `-o`, `--output FILE`, `--out FILE` | output file D; implies `--merge` |
| `-m`, `--merge` | merge mode; without `-o` the output defaults to `unnamed.txt` |
| `--auto` | if every conflict auto-resolves, save to `-o` and exit without a window. Without `-o`: warning on stderr, GUI opens. |
| `--noauto` | cancels `--auto` |
| `--L1`, `--L2`, `--L3 LABEL` | display label of A, B, C |
| `-L`, `--fname LABEL` | label for the next input without an `--Ln` label; repeatable |
| `--cs KEY=VALUE` | config override for this run; repeatable; see Config |
| `--config FILE` | use another engine config file |
| `--confighelp` | print all config keys with their effective values and exit |
| `-u`, `--query`, `--html`, `--abort` | accepted and ignored |
| `-h`, `--help`, `-v`, `--version` | usage and version |

- Compatible with `git mergetool` / `git difftool` invocations used for kdiff3.
- Implemented so far: the positional files, `-b`/`--base`, `-o`/`--output`/`--out`, `--L1`/`--L2`/`--L3` and `-L`/`--fname`. A set with A and B opens directly, with no Open window and no overwrite question; fewer files open the Open window with what was given filled in. Paths from the command line, the Open window and the pane headers are cleaned the same way: surrounding whitespace, line breaks and other control characters are dropped, and on Windows surrounding quotes too, so a pasted "Copy as path" value or an IDE run configuration that adds a trailing line break still names the file.
- Wiring: the binary starts the host, connects a client over the in-process transport and sends the open command. With `--auto` it drives the session headless; if conflicts remain, the GUI takes over the same client and session, so nothing is loaded twice.
- Exit codes follow kdiff3: `0` when the output was saved, `1` when the user quit without saving, on error, and on invalid usage (clap's default of `2` is overridden).
- Any option not in the table is rejected with a usage error. Differences from kdiff3's command line are listed in `DEVIATIONS.md`.

## Error handling and logging

- `kdiff4-core`, `kdiff4-protocol` and `kdiff4-gui` expose `thiserror` enums; the binary wraps them with `anyhow` and `.context(...)`.
- No bare `unwrap()` outside tests. Startup failures use `.expect("<Subsystem>: message")`.
- Logging everything. Library crates only call the `log` macros; the binary installs the logger, after the command line is parsed, so `--help` and usage errors stay on the console.
- Log records go to rotating files in a `logs` folder next to the executable and are duplicated to stdout, in the same format, with local timestamps. The current file is `kdiff4_rCURRENT.log`; at 5 MB it is renamed to `kdiff4_r00000.log`, `kdiff4_r00001.log` and so on (higher is newer), and the 5 newest rotated files are kept. A new run appends to the current file. If the folder can't be created or written, logging goes to stdout only, with a warning.
- Level: kdiff4's crates log at info and above, dependencies at warn and above. The `KDIFF4_LOG` environment variable (off, error, warn, info, debug, trace) changes kdiff4's level for one run; an unknown value is reported and info is used.
- Logged at info: start (with the log folder), the launch arguments, opened files, reloads, output path changes, saves, and exit. Requests and D edits are logged at debug, failed requests at warn, and panics at error.

## Directory compare (v1.1, optional, pending decision)

Separate module in core built on `walkdir` + `ignore`: tree of entries with per-side metadata, comparison by size/date/content, merge operations as a plan applied in one pass. GUI: tree view plus the existing file diff for a selected entry. Not designed in detail until v1.0 ships.

## Out of scope

Explorer/Dolphin/KIO integration, translations, word wrap, preprocessor commands, auto-merge of VCS history/keyword blocks.
