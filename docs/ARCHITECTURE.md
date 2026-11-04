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
  test_data/            test inputs for every crate: kdiff3 alignment cases (`alignment/`) and merge cases (`merge/`).
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
| protocol | `messages.rs` | the message vocabulary: commands, responses, events (snapshot, delta and its changes, current region) and the small types both sides share, such as document, labels, EOL style, revision, navigation and protocol error |
| protocol | `transport.rs` | the client and host endpoints and the in-process transport, with the waker that tells a front end a message arrived |
| protocol | `engine/mod.rs` | the comparison engine interface, its input and result types, the cancel and progress handle, engine errors, and the single entry point the host compares through |
| protocol | `engine/alignment.rs` | validation of manual alignments and the segmentation helper |
| protocol | `engine/check.rs` | the result check run on every engine result |
| protocol | `engine/test_engine.rs` | tests only: a minimal engine and input fixtures |
| core | `text.rs` | decoding, line splitting, line-ending and BOM detection, and atomic writes |
| core | `session.rs` | one open comparison: the sources, their regions and rows, D and its history, the comparison options, the current region, the revision, and load, compare, merge ops (choices and edits), navigation, undo and save |
| core | `merge.rs` | the merge state: regions with their choices, auto-resolve, building D from choices, which regions an edit touches, joining regions, which regions are differences and conflicts |
| core | `undo.rs` | D's undo steps, each with the merge change it made, and the saved/modified tracking |
| core | `host.rs` | the host thread: owns the engine registry and turns each command into a response plus a snapshot or delta |
| core | `normalizer.rs` | turns the lines of all source documents into tokens |
| core | `error.rs` | core's errors and their conversion to protocol errors |
| core | `builtin_engine.rs` | the built-in engine: rows, regions, statuses and character-level diffs on top of `imara-diff` |
| core | `engine_registry.rs` | the engine registry |
| gui | `lib.rs` | starts the window with what the binary already knows: pre-filled paths, labels and a pending open request |
| gui | `app.rs` | the application loop: menus, the toolbar, shortcuts, caret moves, navigation, sending commands and handling their results, the status bar, and routing to dialogs and panes |
| gui | `mirror.rs` | the front end's copy of the session, built from snapshots, deltas and current-region events, with a row map per pane; applies D edits at once and resyncs on a gap or a rejected edit |
| gui | `rows.rs` | row maps: which line, or void, each row of a pane shows, its region and whether that region is boxed, where each line of D came from, padding for void space in D, and the caret on a row |
| gui | `char_diff.rs` | the char-diff rules, which turn a char run into its color and border, and the styled column ranges of a line |
| gui | `panes.rs` | pane layout, pane headers with path fields and Browse, the splitter above the merge pane, shared scrolling, jumps and the caret hidden after a jump |
| gui | `text_view.rs` | the virtualized text widget: draws only visible rows (text, hatching for a void, or the conflict marker), the margin of line numbers and gutter blocks, active for the current region, the region boxes, the jumped-to region's background, the caret and selection, and maps pointer input to positions; reports which rows it shows, so a jump can tell whether it needs to scroll |
| gui | `editing.rs` | caret motion, word and line selection, and turning keys into edits of D, independent of egui |
| gui | `open_dialog.rs` | the Open form, its Run rule, file pickers and the overwrite question |
| gui | `settings_dialog.rs` | the Settings dialog: Font, View and Comparison tabs, with live preview of the view settings |
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
| Session file | `serde_json`, `base64` | core |
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
- **Session file**: a `.kd4` file holding a saved session, including the bytes of every source document, so the merge can continue later or on another computer.
- **Detached source**: a source document whose content comes from a session file instead of the disk. Its path is shown for reference only and cannot be changed or reloaded.

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
- **Character-level diff**: the diff between the lines that share a row. Computed for every row whose lines differ, when the comparison is computed.
- **Char run**: consecutive characters of one line in a character-level diff, with, per source document of the row, whether that document's line has the same characters at the same place.
- **Row map**: the rows of one source pane: for each row its line or a void, its region, and whether that region gets a region box. Built by the mirror from the comparison.
- **Split**: dividing one region into smaller regions at row boundaries.
- **Manual alignment**: a user-defined set of parts, one per source document, that must share rows. See Manual alignment.
- **Alignment part**: one source document's share of a manual alignment: a line range or an anchor.
- **Anchor**: an alignment part that marks a position between two lines, used when the aligned content does not exist in that document.
- **Crossing alignments**: two manual alignments whose order differs between source documents. Rejected.
- **Segment**: a stretch of a source document between two consecutive manual alignments, or one alignment itself. Aligned independently of other segments.
- **Comparison engine**: a pluggable algorithm that turns source documents, comparison options and manual alignments into regions, rows, region statuses and character-level diffs. Holds no session state. Selected by the engine config.
- **Built-in engine**: the default comparison engine, in `kdiff4-core`, built on `imara-diff`.
- **Engine registry**: the comparison engines the host can choose from, by id. The built-in engine is always present and is the default; a second engine with an id already taken is refused.
- **Engine capabilities**: what a comparison engine supports: 2-way and 3-way comparison, native manual alignment, character-level diffs, progress, and which comparison options it honors. Every engine compares 2 source documents and binary files.
- **Result check**: validation of an engine result before the host uses it: coverage, row consistency and manual alignment boundaries.
- **Segmentation helper**: generic code that cuts documents at manual alignments and compares each segment separately, for comparison engines without native manual alignment.

### Merge

- **Choice**: which hunks of a region D takes its lines from, in order, or unresolved.
- **Auto-resolve**: choosing automatically for every region whose status allows it.
- **Unresolved conflict**: a region without a choice. D cannot be saved while one exists.
- **Edited region**: a region whose lines in D were changed by hand. The edited lines replace the lines from its choice.
- **Merge op**: one mutation of the merge result together with its inverse.
- **Merge update**: what one merge op did to the regions, as sent to front ends: a run of regions replaced by new ones, with their choices, edited flags and the number of D lines each contributes.
- **Placeholder row**: the row D shows for an unresolved conflict that contributes no lines. Boxed with its region.
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
- **Palette**: the colors one theme draws with: window and pane backgrounds, pane border, text, and the document colors of A, B, C and D.
- **Document colors**: a document's pane header color, which also fills its gutter blocks, the block border, and the stronger fill of active blocks.
- **Margin**: the strip left of a pane's text that stays in place when the text scrolls sideways: line numbers, then the gutter.
- **Gutter**: the column of gutter blocks left of a pane's text, one row height wide.
- **Gutter block**: a bordered block in the gutter. In a source pane it spans all rows of one region that differs; in D, a run of lines of one region with the same line origin. Blocks of the current region are active.
- **Line origin**: in D, where a line came from: A, B or C for a line from a chosen hunk, M for a hand edit. D's gutter blocks are colored by it and labeled with its letter.
- **Current region**: the region navigation is at; in the GUI, the **jumped-to region**. Owned by the host and shown with a yellow background in every pane, void space included. A region of D that takes lines from two or three hunks is still one region.
- **Region box**: the bordered box drawn around the rows of a region whose hunks differ, as wide as the pane, with no background and rounded corners. Regions whose hunks are all equal get none.
- **Void space in the output pane**: D lined up with the source panes: every region gets the same rows in every pane, padded with void rows.
- **Char-diff rules**: the one match that turns a char run of a source pane into its style: a char-diff color, and whether a border goes around it.

## Core data model (`kdiff4-core`)

### Source text

- Each source document is decoded once into a single UTF-8 string. Lines are ranges into that string, without their terminators, so loading does no per-line allocation.
- Alongside the text the engine keeps the encoding, whether a BOM was present, the EOL style (for mixed files the majority style is used on save) and whether the last line lacks a terminator.
- Encoding: BOM first, then user override, then `chardetng`, then UTF-8 fallback. Invalid bytes become U+FFFD and the file is flagged so save warns before lossy re-encode.
- Binary detection: NUL bytes in the first chunk mark the file as binary; binary files are compared by bytes only and are not merged.

### Line comparison

Lines are compared through their line keys, produced by a normalizer configured from options (ignore whitespace, ignore case, ignore trailing whitespace, ignore numbers, ignore comments). Line keys are interned into integer tokens before diffing, so the diff runs on integers.

### Comparison engine

The algorithm that compares documents is pluggable. The engine config names the comparison engine to use (`[diff]` key `engine`); the built-in engine is the default. The interface is defined in `kdiff4-protocol` and uses only protocol vocabulary types, so its results go into snapshots and deltas without translation, and a later adapter can run an engine out of process. The host keeps a registry of the available engines.

Responsibilities are split so that every engine gets the same merge behavior:

- The comparison engine computes: regions with their hunks, the rows of each region, region statuses (including whitespace conflicts), the status of a given set of hunks (used after a split), character-level diffs for the lines of one row, and binary compare. It honors the comparison options and the manual alignments it is given: the parts of each alignment share rows, and both ends of an alignment are region boundaries. This holds in 2-way and 3-way mode.
- The session keeps the merge state: the manual alignment list and its validation (crossing and overlapping alignments are rejected before the engine sees them), deciding which part of the documents to recompute, splits, choices, auto-resolve, edited regions, undo, D, save, session files and navigation.
- Line normalization stays in the session: it builds line keys and tokens and hands them to the engine together with the text, so ignore options behave the same with every engine.

An engine reports its engine capabilities, which go into the snapshot so front ends can disable what it does not support. An engine without native manual alignment is driven through the segmentation helper, which gives it manual alignment for free. Binary compare has a byte-by-byte default that engines may replace.

The host compares only through one entry point, which validates the input (2 or 3 source documents, 3 only if the engine supports it; one token per line; valid manual alignments), hands the alignments to the engine or to the segmentation helper, and runs the result check. The segmentation helper also checks each segment's result on its own, so an engine cannot spill a region across a segment boundary.

Comparison runs on host worker threads with the cancel flag and progress reporting described under Threading and cancellation; the engine itself is synchronous. The host checks every engine result before using it: regions must cover every line of every source document exactly once, in order, and rows must be consistent with the regions and the manual alignments. A result that fails the check, or an engine failure, becomes a protocol error and a notice, never a corrupt session. Engine errors cross as their own error kinds: canceled, invalid request (bad input or manual alignments), or engine (unsupported feature, failure, or a result that failed the check).

Implemented so far: the interface, the entry point, manual alignment validation, the segmentation helper, the result check, the built-in engine and the engine registry. The session compares with the registry's default engine when it opens and when a source document is reloaded, and keeps the result; the snapshot carries the regions, rows and engine capabilities. Line keys are the exact line text. Not yet: the `engine` config key and choosing an engine, ignore options in the normalizer, and running comparisons as cancelable worker jobs: for now they run to completion on the host thread, like loading.

Every engine runs the same conformance tests: coverage invariants, statuses, manual alignment constraints in 2-way and 3-way mode, and the kdiff3 alignment test data. Session tests use a stub engine with fixed results, so merge state is tested without a diff algorithm.

The merge cases in `test_data/merge/` run the built-in engine through the host the way the GUI drives it. Each case folder holds the base, left and right documents (left and right alone for a 2-way merge) and the expected result: the auto-merge, with B chosen for every conflict left. The cases cover plain merges without conflicts, conflicting and alike changes, a 2-way merge, a long file, functions moved with and without changes on the other side, whitespace-only changes, and edge cases (an empty base, a last line without a line break, CRLF line ends). Each test checks the number of differences, conflicts and unresolved regions, the expected result, and that choosing one side everywhere reproduces that side.

The sections below, from 2-way diff through Regions and rows, describe the built-in engine.

### 2-way diff

A thin wrapper over `imara-diff`: it diffs the token sequences of two documents with the configured algorithm and returns the changed ranges. Histogram is the default; Myers with try hard is the minimal Myers variant, and try hard does nothing for Histogram.

The diff of A and B gives its rows directly: equal lines share a row, the lines of one changed range are paired by position, and the rest of the longer side gets rows of its own.

### Regions and rows

3-way mode builds rows from two 2-way diffs, then regions from the rows:

1. Diff A with B and A with C, and pair their lines as in 2-way mode. Every line of A now has a partner in B and in C, or none.
2. Each line of A gets one row with both partners. Lines of B and C without a line of A, which come between the same two lines of A, are diffed with each other and paired the same way, so equal lines of B and C share a row instead of being stacked. They are placed before the next line of A.
3. A row whose lines are equal in every document is stable. Runs of stable rows become Unchanged regions; the runs between them become changed regions. So changes from B and C that overlap, or meet at the same position of A, join into one region.
4. The status of each region follows from comparing its hunks. A conflict is a whitespace conflict when hunks B and C are equal once all whitespace, line breaks included, is removed.

2-way mode builds its regions from its rows the same way.

Pairing B and C across rows that also hold a line of A (kdiff3 `Diff3AlignBC`) is not implemented yet; the built-in engine reports that option as not honored.

Character-level diffs are computed by the session with the comparison (on open, reload and new comparison options) for every row whose lines are not all present and equal, and sent in the snapshot, per source document and line. Rows depend only on the comparison, so edits of D, which only join regions and keep their rows, never make them stale. Each line of a row is diffed by characters with every other line in the row (Myers), and the result is a list of char runs covering the line: each run says in which documents of the row its characters are the same. A void is never the same. Above the line length limit two different lines are different as a whole. An engine without character-level diffs, or one that fails, leaves every line unmarked.

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
- These rules bind every comparison engine. The built-in engine meets them through the segmentation helper.
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
- Implemented so far:
  - D is built at open by auto-resolving every region; an existing output file is not read. Its text is always the regions' texts in order, and each region contributes whole lines, each with a line break.
  - Merge ops: setting a region's choice (an ordered list of hunks, possibly empty, or unresolved), which drops its hand edits; choosing one side for every unresolved region; hand edits. Every op replaces a run of regions and the matching part of D, and is one undo step that restores both. Typing inside one region still joins into one step.
  - A hand edit belongs to the regions whose text it touches. An insertion goes to the region holding its position, so typing at the start of the line after an empty unresolved conflict goes to that line's region, not into the conflict: to write a conflict by hand, choose a hunk first and edit it. If the edit leaves the touched regions without a final line break while text follows, the next region is taken in too. When several regions are touched they are joined into one edited region with no choice, spanning all their hunks and rows, and the engine classifies it again; undo brings the original regions back.
  - An unresolved conflict is a region with no choice and no hand edits. Save and Save As are refused while one exists.
  - Reloading a source document recomputes the comparison and rebuilds D the same way as at open: choices and hand edits are dropped and D's undo history starts over, with D marked modified.
  - Not yet: the whitespace default sides, the auto-resolve and save-and-quit options, keeping a last line without a line break, and splitting.

### Session

The session holds everything about one open comparison (see Glossary). The host applies each command to the session and turns the result into a response plus a delta. Unit tests apply commands to a session directly; integration tests go through a client, like the GUI.

The session is a core entity and never crosses the protocol: front ends see it only through the snapshot and deltas.

### Session file (`.kd4`)

A session can be saved to a `.kd4` file and opened again, to continue a merge later or to hand a partially resolved merge to someone else, whose computer may have different files at different paths.

- Contents: a `version` key for migrations; for each source document its label, original path and raw bytes (base64), so encoding, BOM and EOL style are reproduced exactly; D's text, path and output reference; the manual alignments, splits, choices and edited regions; the regions and rows as computed; the undo history; the engine config with the comparison engine's id and version.
- Format: JSON via `serde_json`, written atomically (temp file + rename) like D.
- Every source document of an opened session file is a detached source: the engine works from the embedded bytes, and the disk is not consulted. Its path is kept for reference; it cannot be changed or reloaded, so the merge state is never recomputed from a changed source. D's path stays editable, so the receiver chooses where to save the result.
- Opening restores the saved regions and rows without recomputing, so the result does not depend on the engines available. If the comparison engine's id or version differs from the saved one, a notice says so; recomputing is the user's choice and drops choices, edits and splits like an options change.
- Saving a session does not save D, and saving D does not save the session.
- The file is read and written by the host only; the format is owned by core.

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

- Commands: open, reload, cancel, choose hunks for a region, choose a side for all unresolved regions, split a region, edit D, add or remove a manual alignment, undo, redo, save, set engine config, navigate, set the current region. Every user action in any front end ends up as a command.
- Planned: save the session to a session file; open a session file.
- Implemented so far: open; reload one source document from a (possibly new) path, answered with a new snapshot; set the choice of a region and choose one side for every unresolved region (both tagged with the revision their region indices refer to); edit D (replace a byte range, tagged with the revision it was made on; a stale revision or a range off character boundaries is rejected); set D's path without writing it (or clear it, making D unnamed); save; save as; undo and redo of D; resync (the host re-sends a snapshot); set the comparison options, which compares again and rebuilds D like a reload and is answered with a new snapshot; navigate to the next or previous difference or conflict from a region (the caret's, or the current one); set the current region. Navigation and setting the current region are answered with a current-region event. Deltas so far: D changed by a merge op, undo or redo (the replaced range of D's text, the merge update, and whether D now differs from its saved text, which undo can bring back), D's path changed, D saved. The snapshot carries each region's merge state.
- D's undo history is kept by the host. Typing on one line is one undo step, and so is a run of Backspace or of Delete; a line break, a paste, replacing a selection, a word deletion, save, undo and redo start a new step. Undoing back to the saved text clears the modified flag.
- D's text crosses the protocol with `\n` line ends. It is saved as UTF-8 with the EOL style and BOM of A, the output reference.
- Queries: reads the mirror does not hold: config help text.
- Progress events report long work (job, amount done, total); notices carry warnings such as a pending lossy re-encode.
- The first exchange is a handshake with the protocol version; a mismatch is an error.

### State mirroring

- After open, the host sends a snapshot: document texts and metadata (labels, encoding, EOL style, whether the document is a detached source), regions, rows, D, manual alignments, engine config, comparison options, engine capabilities, the current region and the revision.
- Every change increments the revision and is broadcast as a delta (replaced ranges of regions, rows and D lines). The client applies deltas in order; on a gap in revisions it requests a new snapshot.
- The current region belongs to the session, so navigation is the same for every client and can be driven from outside, as automation would. Navigation commands move it, and the host broadcasts it in a current-region event, which names the revision its region index refers to but does not change it, so an edit in flight stays valid. A change of D makes its first region current without an event, so undoing a choice makes that region current again. Navigation is not an undo step. Every client follows the event: the GUI jumps there whoever moved it.
- Front ends render from the mirror only and never compute diffs, alignment or merge results themselves. View state (scroll position, cursor, selection, focus) stays in the front end.
- Typing in D: the client applies the edit to its mirror at once and sends it as a command tagged with the revision it was based on. The host is the authority. The delta caused by an accepted edit carries the request id, so the client does not apply it twice; a rejected edit makes the client drop its local change and resync. One editing client per session in v1.
- Character-level diffs come with the snapshot and stay in the mirror until the next one.

### Threading and cancellation

- The host's request loop never blocks on long work: load, diff and align run on host worker threads with a cancel flag and report progress. A cancel command stops the running job. Requests that depend on its result wait in order until it finishes or is canceled.
- The in-process transport takes a wake-up callback from the front end; the GUI passes one that requests a repaint, so it sleeps while idle and wakes when a message arrives. The callback keeps the protocol crate free of GUI types.

### Message rules

- Messages are plain owned data that can move between threads and be serialized with `serde`; no references, shared pointers, file handles or callbacks.
- File paths in messages refer to the host's file system.
- Errors cross the boundary as a protocol error (kind plus message), never as core's own error types.
- Small vocabulary types shared by both sides (document, hunk, hunk change, region, region status, row, choice, manual alignment, EOL style, engine config, engine capabilities) are defined in the protocol crate and used by core directly. The comparison engine interface and the segmentation helper live there too, in an `engine` module. Core internals (source text, the undo stack) never appear in messages. Tokens are passed to comparison engines but never to front ends.

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

- `[view]`: font family (`font_family`, monospace only; absent means the built-in font), font size (`font_size`, 6 to 48), font hinting (`font_hinting`), subpixel glyph positioning (`subpixel_positioning`), corner radius of widgets, dialogs, panes and the window frame (`corner_radius`, 0 to 12), line numbers (`show_line_numbers`, on by default), gutters of the source panes (`show_gutters`, on by default), void space in the output pane (`align_output`, off by default), lines kept above a region navigation jumps to (`jump_context_lines`, 0 to 20, default 12), the background of the jumped-to region (`current_region_color`, red, green, blue and alpha, 0 to 255 each, a translucent yellow by default), the character-diff colors (`char_diff_red`, `char_diff_blue`, `char_diff_green`, red, green and blue, 0 to 255 each) and the corner radius of their borders (`char_diff_corner_radius`, 0 to 12, default 2), the space between adjacent region boxes (`region_gap`, 0 to 8 pixels, default 3), the corner radius of region boxes (`region_corner_radius`, 0 to 8 pixels, default 2), tab size (`TabSize`), replace tabs (`ReplaceTabs`), show whitespace (`ShowWhiteSpace`, `ShowWhiteSpaceCharacters`), auto-advance after choosing (`AutoAdvance`, `AutoAdvanceDelay`), Escape quits (`EscapeKeyQuits`).
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
  - Each pane has a header with the document letter (with `*` while D has unsaved changes), its label, an editable path field and a Browse button. Enter in the field, or a file picked with Browse, reloads A, B or C from that path and rebuilds D from the new comparison; for D it only changes where D will be saved. For a detached source the path field and Browse are disabled.
  - File → Save Session and File → Open Session write and read `.kd4` files.
  - Panes show a caret and its selection; there is one on screen. Pressing the mouse button in any pane moves the caret there at once, and dragging selects (Shift+click extends). Clicking below the text goes to its end, and right of a line to the line's end. Double-click selects a word (or a run of whitespace, or one punctuation character), triple-click a line. The keys are arrows, Ctrl+Left/Right by words, Home/End, PageUp/PageDown and Ctrl+Home/End, with Shift to select, plus Ctrl+A and Ctrl+C. Up, Down, PageUp and PageDown move row by row, so the caret can stop in void space: it is drawn at the void row's left edge, and its position is the start of the line after the void, which is where typing and selections go. A, B and C are read-only. D is editable: typing, Enter (also with Ctrl held), Backspace, Delete, Ctrl+Backspace and Ctrl+Delete by words, Ctrl+V and Ctrl+X, which replace or remove a selection; Edit → Undo (Ctrl+Z) and Redo (Ctrl+Y, Ctrl+Shift+Z) work on D wherever the caret is. Each edit is applied to the mirror at once and sent to the engine.
  - Margin: left of the text every pane has line numbers (`show_line_numbers`), blank on void rows. Source panes then have a gutter (`show_gutters`): one bordered block per region that differs, in the document's color, spanning all its rows, void rows included; regions whose hunks are all equal get none. D always has a gutter: within each such region, one block per run of lines with the same origin, in that source's color (D's neutral color for a hand-edited region, an unresolved conflict's placeholder or an unknown origin), labeled once with A, B, C or M. The gutter is one row height wide, so a one-row block is a square. Clicking a block makes its region current: its blocks in every pane turn active, and the caret goes to the region's start in the clicked pane; the source panes don't scroll, and D scrolls only when it doesn't line up with them and the region is out of view. The margin stays in place when the text scrolls sideways.
  - Region boxes: in every pane, D included, each region whose hunks differ is drawn as a bordered box across the full pane width, margin included, spanning all its rows, void rows included. Boxes have no background and rounded corners (`region_corner_radius`), and adjacent boxes are `region_gap` pixels apart. Rows have no background shade by change; the boxes mark the differences.
  - Toolbar under the menu bar: a small hatched square toggles void space in the output pane (`align_output`). On, every region gets the same rows in every pane, padded with void rows, and D scrolls with the source panes; off, D has just its own rows and scrolls on its own.
  - File → Save writes D to its path, or asks for one like Save As when D has none. Save and Save As are disabled while unresolved conflicts remain; Ctrl+S then reports how many are left. Save As picks a new path with the native Save dialog, which asks before overwriting, and D moves there. The outcome of saving, reloading and path changes shows in the status bar under the panes. After open and after a reload it shows how many differences and conflicts the comparison found, and how many of the conflicts are whitespace only. The status bar is always there, one line high, so the panes never move when a message comes or goes; long messages are cut, with the full text in a tooltip.
  - Overview strip: a 4-line lens row between (A/B/C) and (D), shows current lines from all A, B, C, and D. D is allowed to be edited in this widget, and it automatically reflected in the D text. Shows char-diffs within each A, B, and C lines, so the contents could be judged by user and edited as the user wants.
- Void Region: a special graphical entity that spans the rows of a region where a document has no text, and it represents absence of text in the specified document. Graphically represented as hatched block of space with no text in it.
- Scrolling of the three panes is synchronized through row indices, not pixels. Void Regions take rows like text does, so synced scrolling stays aligned. Every source pane has the same rows at the same height, so the panes share one vertical scroll offset: the pane the user scrolls, or the one that reveals the caret, moves all of them. Horizontal scrolling stays per pane. D scrolls on its own unless void space in the output pane is on, which gives it the source panes' rows.
- Source panes draw rows, not lines: each row shows its line or the hatching of a void, inside its region's box when the region differs. The caret and selection stay in line positions; a click on a void puts the caret on that void row, at the start of the next line. If the comparison does not fit a document, the pane falls back to one row per line and the mismatch is logged.
- The merge pane is a custom widget, not `TextEdit`: own cursor, selection, clipboard, IME, and region coloring.
- Merge menu: Choose A, B or C Here (Ctrl+1, Ctrl+2, Ctrl+3) toggles that hunk in the region of the row with the caret, in any pane: it is appended to the region's choice, or removed if it is already there; an unresolved or hand-edited region starts from no choice, and removing the last chosen hunk makes the region unresolved again. Each toggle is one undo step. Choose A, B or C for All Unresolved resolves every unresolved region at once. C is offered only in 3-way merges. While a typed edit is unconfirmed, choices wait, since its delta may join regions. D shows a placeholder row with a red `<Merge Conflict>` marker for each unresolved conflict, and the status bar counts unresolved conflicts after open, reload and every choice.
- Navigation: Merge → Previous and Next Difference (Ctrl+Up, Ctrl+Down) and Previous and Next Conflict (Ctrl+PageUp, Ctrl+PageDown) ask the engine to move the current region from the region of the caret's row. Differences skip unchanged regions; conflicts are the regions auto-resolve leaves unresolved, whether or not D resolves them by now. If every pane already shows all rows of the region, nothing scrolls; otherwise every pane scrolls so the region starts `jump_context_lines` rows below the top. The caret goes to the region's start in the pane that has it, or in A, but is hidden until it is moved again by a click, a caret key or an edit; choosing for the caret's region still works on the jumped-to region. With nowhere to go, the status bar says so. The jumped-to region gets the `current_region_color` background in every pane, void space included.
- The GUI talks to the engine only through its protocol client and renders from the client mirror.
- Long work (load, diff, align) runs in the host; the GUI shows its progress events, keeps repainting and can cancel.
- Dialogs: open files, save-as, settings, find, go to line, "unsaved changes" confirm.
- Open dialog: shown at start and from File → Open. Each document has a path field and a Browse button that opens a native file dialog; D gets a save dialog, since it may not exist yet. Every file dialog starts in `last_dir`, which is updated whenever a file is confirmed. Run needs A and B; C makes a 3-way merge, and D names the output file (a merge without D keeps D in memory until it is saved). If D is an existing file, Run asks before overwriting it. D starts as the auto-resolved merge, whatever the output file holds.
- Corner radius: applied to widgets, menus, dialogs and panes. On Windows 11 the native window frame follows it too, through the DWM corner preference: 0 is square, 1 to 4 is small rounding, more is normal rounding.
- Theme: light for now. Both themes have a palette and are styled at startup, so switching to dark later needs no other change. Panes are bordered boxes with a header strip; their text uses the palette's text color, which has at least 7:1 contrast with the pane background.
- Settings dialog: Settings → Configure Kdiff4... (Ctrl+,), in three tabs. Editor: font family, size, hinting and subpixel positioning; the font list shows only installed monospace families, found on a background thread, plus the built-in font; line numbers, gutters, void space in the output pane, lines above jumps, the jumped-to region's color, the gap and corner radius of region boxes, and the character-diff colors and border corner radius. View: corner radius of widgets, dialogs, panes and the window frame. Comparison, while files are open: the diff algorithm, minimal diff (Myers only) and the character-diff line limit. View changes preview live; OK saves `gui.toml`, Cancel reverts. Comparison changes are sent on OK as one set-comparison-options command, which compares again and rebuilds D, dropping its choices, edits and undo history; they are not saved yet, since the engine config file doesn't exist yet. Aligning B with C is left out, as the built-in engine doesn't support it.
- Colors:
  - Red (removed chars)
  - Green (added chars)
  - Magenta (modified chars)
  - Amber (conflicts)
  - Implemented so far: void hatching and the border of region boxes, as fixed colors of the palette; the document colors (headers and gutters: A blue, B green, C violet, D neutral; for now each source's gutter blocks use its header color) and the red of the conflict marker, also fixed; the jumped-to region's background, from `current_region_color`, whose default keeps text at 7:1 contrast. Character-level colors come from the `char_diff_*` keys of `[view]`. The `[colors]` section of `gui.toml` is not read yet.
- Character-level diff: drawn in every visible row of A, B and C that has one, in any region; D has none. The char-diff rules, one match in the GUI, map each char run to a style from its document and where its characters are the same. In B and C: changed from A and not the same in the other of B and C is red with a border; changed from A and the same in the other is blue with a border; the same as in A is green, without a border. In A: characters missing from B or C are red with a border, the rest is plain. A 2-way diff follows the same rules with C missing. Neighboring runs with the same style share one border, which has the text's color and `char_diff_corner_radius`.

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
