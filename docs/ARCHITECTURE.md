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
| protocol | `engine/inset.rs` | the order of the parts of each document, which parts a document shows away from their place, the layout of the documents in display order with the parts exchanged between their places, comparing it, and bringing the regions back to the documents |
| protocol | `engine/check.rs` | the result check run on every engine result |
| protocol | `engine/test_engine.rs` | tests only: a minimal engine and input fixtures |
| core | `text.rs` | decoding, line splitting, line-ending and BOM detection, and atomic writes |
| core | `session.rs` | one open comparison: the sources, their regions and rows, D and its history, the comparison options, the alignment marks, the current region, the revision, and load, compare, apply alignments, merge ops (choices and edits), navigation, undo and save |
| core | `alignment_marks.rs` | the alignment marks: numbering, the checks on marking, renumbering with swap, which numbers give alignments and the errors that name the user's numbers, the display order of the alignments, where an inset could be shown and which documents move its hunk, each mark's status, and the alignments the comparison was last computed with |
| core | `heuristics/mod.rs` | Heuristics Alignment: finding the entities of every source document, limiting them to the inside of an alignment for a scoped run, pairing them and naming the pairs |
| core | `heuristics/extract.rs` | the extractor: parsing a document with tree-sitter (C and C++ for now; C falls back to the C++ grammar for headers that hold C++), and turning functions, methods, types and (when asked) the blocks inside functions into a tree of entities with their name, signature, lines with the comments right above them, body and children |
| core | `heuristics/matching.rs` | pairing entities across documents in stages (equal signature, unique name, similar signature and body), keeping overloads apart, pairing 3 documents through A and anchoring an entity one document lacks between its neighbors, then pairing the children of every pair the same way, with the blocks kept in order |
| core | `merge.rs` | the merge state: regions with their choices, auto-resolve, building D from choices, which regions an edit touches, joining regions, which regions are differences and conflicts |
| core | `undo.rs` | D's undo steps, each with the merge change it made, and the saved/modified tracking |
| core | `host.rs` | the host thread: owns the engine registry and turns each command into a response plus a snapshot or delta |
| core | `normalizer.rs` | turns the lines of all source documents into tokens |
| core | `error.rs` | core's errors and their conversion to protocol errors |
| core | `builtin_engine.rs` | the built-in engine: rows, regions and statuses on top of `imara-diff`, and the character-level diffs of a row |
| core | `similar_lines.rs` | pairing the lines of a changed chunk by how alike they are (character pairs of the trimmed lines), with the lines between similar pairs paired by position, and the score of a row that decides whether a region keeps those rows |
| core | `fine_diff.rs` | the changed columns of two lines, found the way kdiff3 does (its own match search and the merging of short matches; unlike kdiff3, a change starts at its first differing character), for the built-in engine's character-level diffs |
| core | `engine_registry.rs` | the engine registry |
| gui | `lib.rs` | starts the window with what the binary already knows: pre-filled paths, labels and a pending open request |
| gui | `app.rs` | the application loop: menus, the toolbar and its buttons, shortcuts, caret moves, navigation, sending commands and handling their results, the status bar, and routing to dialogs and panes |
| gui | `mirror.rs` | the front end's copy of the session, built from snapshots, deltas and current-region events, with a row map per pane; applies D edits at once and resyncs on a gap or a rejected edit |
| gui | `rows.rs` | row maps: which line, or void, each row of a pane shows, its region and whether that region is boxed, where each line of D came from, padding for void space in D, and the caret on a row |
| gui | `alignment_gutter.rs` | the alignment column of a source pane: where each mark sits in the rows, what the pointer is on, the tooltip, the numbers a mark can be renumbered to, the places an inset can be shown at, the arrows of moved hunks and their lanes, and painting |
| gui | `inset_dialog.rs` | the window that asks where to show an inset alignment |
| gui | `eol.rs` | the names of the EOL styles, the status bar drop-down for the Output EOL, and the window that asks for a style when saving in a conflict |
| gui | `heuristics_dialog.rs` | the Heuristics Alignment window: its options, the table of proposed pairs with a checkbox each, and whether the table is stale |
| gui | `char_diff.rs` | the char-diff rules, which turn a char run or a region's gutter block into its style, and the styled column ranges of a line |
| gui | `panes.rs` | pane layout with the overview strip's place, pane headers with path fields and Browse, the splitter above the merge pane, the scroll groups and their shared scrolling, wheel input, jumps and the caret hidden after a jump |
| gui | `minimap.rs` | the minimap: the column of difference blocks and the view rectangle right of the panes, and turning a press or drag in it into scrolling |
| gui | `overview.rs` | the overview strip: which row of each document lines up with the caret, drawing those lines with their character diff, and turning the mouse over them into caret moves |
| gui | `scroll.rs` | smooth scrolling: a scroll offset that moves toward its target over a set duration, or at once |
| gui | `find.rs` | the find bar: searching the lines of one pane (plain text or regular expression, case, whole word), stepping to the next and previous match, and the bar widget |
| gui | `text_view.rs` | the virtualized text widget: draws only visible rows (text, hatching for a void, or the conflict marker), the margin of line numbers, alignment column and gutter blocks, active for the current region, the context menu of source panes, the region boxes, the jumped-to region's background, the caret and selection, and maps pointer input to positions; reports which rows it shows, so a jump can tell whether it needs to scroll, and the vertical wheel scrolling over it, which it leaves to the caller |
| gui | `editing.rs` | caret motion, word and line selection, and turning keys into edits of D, independent of egui |
| gui | `open_dialog.rs` | the Open form, its Run rule, file pickers and the overwrite question |
| gui | `settings_dialog.rs` | the Settings dialog: Panes, Appearance and Comparison tabs, with live preview of the view settings |
| gui | `config.rs` | `gui.toml`: loading, saving and backing up a broken file |
| gui | `update_check.rs` | finding out whether a newer release exists: the GitHub latest-release request on a worker thread, parsing the release (version, Markdown notes, page, files), choosing the file for this platform, the once-a-day rule and the skipped-version rule |
| gui | `self_update.rs` | installing a release: downloading the platform's executable to `<exe>.new` with progress and cancel, checking it is complete and looks like an executable, moving the running executable to `<exe>.old`, putting the download in its place, restarting, and removing the leftovers at the next start |
| gui | `update_dialog.rs` | the update windows: Update Available with the release notes rendered as Markdown, the download progress, and plain notices |
| gui | `fonts.rs` | discovering monospace font families and installing the chosen one |
| gui | `language.rs` | the languages the status bar offers with their logos, file names, extensions and shebang programs, detecting a session's language from its paths and first lines, keeping a picked language while the source files stay, and the drop-down |
| gui | `theme.rs` | palettes with the user's color overrides, the theme choice and the egui style, including corner radius and pane frames |
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
| Config | `serde`, `toml`, `dirs` | core, gui |
| GUI | `egui`, `eframe`, `egui_extras` | gui |
| File dialogs | `rfd` | gui |
| Update check and download | `ureq` (rustls), `semver`, `serde_json` | gui |
| Release notes display | `egui_commonmark` | gui |
| Code structure for Heuristics Alignment | `tree-sitter`, `tree-sitter-c`, `tree-sitter-cpp` | core |
| Font discovery and monospace detection | `fontique`, `skrifa` | gui |
| CLI | `clap` | bin |
| Log output | `flexi_logger` | bin |
| Snapshot tests | `insta` | dev |
| Property tests | `proptest` | dev |
| GUI interaction tests | `egui_kittest` | dev |


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
- **EOL style**: line terminator of a document: LF, CRLF, CR or mixed. A document without any terminator has none.
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
- **Void Region**: the rows in a pane that have no source text: the rows of a void, and the padding below a hunk that is shorter than another hunk of the same region. Drawn hatched, in the color of the pane's source document (in D, the color of the document whose empty hunk was accepted; neutral otherwise).
- **Character-level diff**: the diff between the lines that share a row. Computed for every row whose lines differ, when the comparison is computed.
- **Char run**: consecutive characters of one line in a character-level diff, with, per source document of the row, whether that document's line has the same characters at the same place.
- **Row map**: the rows of one source pane: for each row its line or a void, its region, and whether that region gets a region box. Built by the mirror from the comparison.
- **Split**: dividing one region into smaller regions at row boundaries.
- **Manual alignment**: what the alignment marks with one number give once applied: a set of parts, one per source document, that must share rows. See Manual alignment.
- **Alignment part**: one source document's share of a manual alignment: the mark with that number in the document, a line range or an anchor.
- **Anchor**: an alignment part that marks a position between two lines, used when the aligned content does not exist in that document.
- **Alignment mark**: what the user marked in one source document for alignment: a line range or an anchor, with an alignment number. Marking changes nothing in the comparison.
- **Alignment number**: the number of a mark. Marks with the same number in different documents belong to the same alignment. A new mark takes the lowest number its document has not used.
- **Applied, pending and unmatched marks**: a mark is applied when the comparison was computed with its alignment, pending when applying would put its alignment into effect, and unmatched when its number is not marked in every source document, so applying ignores it.
- **Apply**: comparing all documents again with the alignments the marks give. It rebuilds D like a reload and can't be undone.
- **Inset alignment**: a manual alignment that the source documents disagree on the order of: its place among the other alignments differs between documents. It is shown in one place, and the documents that have it elsewhere exchange its hunk with the one that has its place there. See Inset alignment.
- **Nested alignment**: a manual alignment whose parts lie inside the parts of another alignment in every document. It is cut out of its parent's part by the segmentation helper and moves with it. See Nested alignment.
- **Sibling group**: the alignments with the same parent (the smallest alignment that encloses them), or the top-level alignments. The display order, insets, placements and moved parts are decided within a sibling group. See Nested alignment.
- **Moved part**: a part of an alignment that a document shows away from its place in the document. The parts of a document that already have the display order stay (a longest run of them, and of several the one with the alignments listed first), the others are moved; the parts in between only follow along when the places are exchanged. The old place of a moved part shows the hunk of another alignment.
- **Display order**: the order of the alignments in the rows. In the layout every document has its alignments in this order.
- **Layout**: the source documents in display order: in every document the text between the places of the parts stays, and the place of the `n`-th part shows the part of the `n`-th alignment of the display order. The comparison runs on it like on documents without insets, and its regions are then brought back to the documents.
- **Seam**: a place in a layout where the next line is not the line after the previous one in the document. Seams are always edges of alignment parts, so no region reaches over one.
- **Placement**: where the user shows an inset alignment among the others, chosen when the alignment is marked or later from the menu of the alignment column. The inset choice lists the placements an alignment has.
- **Inset choice**: the places an inset alignment can be shown at, one per set of documents that keep their order there, and which one it is shown at now. Part of the alignments in snapshots and deltas.
- **Inset arrow**: an arrow in the strip left of the line numbers of a source pane, from where a moved hunk was to the rows it is shown on, drawn like a branch in a graph. Arrows that overlap in height take different lanes.
- **Heuristics Alignment**: finding the functions, methods and types of the source documents, pairing them across the documents and marking each pair as an alignment, so moved code is exchanged like inset alignments. See Heuristics Alignment.
- **Entity**: a function, method, type or block an extractor found in one document, with its qualified name (a block: its header), its signature, its lines from the comments right above it to its closing brace, its body and its children, the entities inside it.
- **Block**: an entity kind for a statement with a body of at least 3 lines inside a function: condition, loop, switch, case, try, lambda or nested function. Blocks pair in order only.
- **Scoped heuristics run**: Heuristics Alignment limited to what lies inside one applied alignment (`within`), started with Ctrl+Alt+H. Its blocks are always on, and applying it replaces only the heuristic marks nested in that alignment.
- **Extractor**: the parser that finds entities. Selected in the settings; tree-sitter is the only one so far.
- **Heuristic mark**: an alignment mark Heuristics Alignment made. Running it again replaces the heuristic marks and keeps the user's own.
- **Match stage**: how the entities of a heuristic alignment were paired: by equal signature, by name, or by similar signature and body, with their similarity.
- **Heuristic proposal**: the pairs Heuristics Alignment found, each with its kind, the name and lines of each document's entity (or an anchor), its match stage, the pair that holds it and whether a document moves it, plus how many entities each document has and how many pairs collide with the user's marks.
- **Segment**: a stretch of a source document between two consecutive manual alignments, or one alignment itself. Aligned independently of other segments.
- **Similar line pairing**: pairing the lines inside a changed region by how alike they are instead of by position; the comparison option `pair_similar_lines`. See Regions and rows.
- **Comparison engine**: a pluggable algorithm that turns source documents, comparison options and manual alignments into regions, rows, region statuses and character-level diffs. Holds no session state. Selected by the engine config.
- **Built-in engine**: the default comparison engine, in `kdiff4-core`, built on `imara-diff`.
- **Engine registry**: the comparison engines the host can choose from, by id. The built-in engine is always present and is the default; a second engine with an id already taken is refused.
- **Engine capabilities**: what a comparison engine supports: 2-way and 3-way comparison, native manual alignment, character-level diffs, progress, and which comparison options it honors. Every engine compares 2 source documents and binary files.
- **Result check**: validation of an engine result before the host uses it: coverage, row consistency and manual alignment boundaries.
- **Segmentation helper**: generic code that cuts documents at manual alignments and compares each segment separately, for comparison engines without native manual alignment. It also compares the segments of a layout, for every engine.

### Merge

- **Choice**: which hunks of a region D takes its lines from, in order, or unresolved.
- **Auto-resolve**: choosing automatically for every region whose status allows it.
- **Unresolved conflict**: a region without a choice. D cannot be saved while one exists.
- **Edited region**: a region whose lines in D were changed by hand. The edited lines replace the lines from its choice.
- **Merge op**: one mutation of the merge result together with its inverse.
- **Merge update**: what one merge op did to the regions, as sent to front ends: a run of regions replaced by new ones, with their choices, edited flags and the number of D lines each contributes.
- **Placeholder row**: the row D shows for an unresolved conflict that contributes no lines. Boxed with its region.
- **Empty region row**: the void row D shows for a region that differs, is not an unresolved conflict and contributes no lines, such as a deletion that was chosen. Boxed with its region, so D has a box for every region the source panes have one for. Typing on it gives the region those lines and makes it edited.
- **Undo step**: a group of merge ops undone and redone together.
- **Output reference**: the source document whose encoding, BOM and final line break D is saved with.
- **Output EOL**: the EOL style D is saved with. It is what the sources with an EOL style share, LF when none has one, and a conflict when two differ, until the user chooses a style. D can't be saved while it is in conflict.

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
- **Overview strip**: the lens between the source panes and the merge pane: one line per document, A, B and C, then D, on the row that lines up with the caret.
- **Caret**: the one text cursor on screen, with its selection. It lives in one pane at a time; pressing the mouse button in another pane moves it there.
- **Splitter**: the draggable border between the source panes and the merge pane.
- **Find bar**: the floating bar at the top right of the panes, VS Code style: a field, toggles for match case, whole word and regular expression, the result count, previous and next buttons and close. It searches the pane that has the caret.
- **Find match**: one occurrence of the find bar text in a line of the searched pane. All of them are highlighted, the one the selection covers stronger.
- **Open dialog**: the window where A, B, C and D are chosen before a session opens.
- **Settings dialog**: the window where GUI config is edited, with a live preview.
- **Update check**: asking GitHub for the latest published release, at most once a day on start, or at once from Help → Check for Updates... Runs on its own thread; a failure is logged, and shown only when the user asked.
- **Update window**: the modal that follows a check: Update Available with the release notes and the choices Update and Restart, Remind Me Later and Skip This Version; the download progress; or a notice. While one is open the rest of the window takes no input.
- **Update install**: downloading the release's executable for this platform, replacing the running executable in place and restarting it with the same command line.
- **Theme**: light or dark appearance of the GUI; the user picks light, dark, or following the system.
- **Palette**: the colors one theme draws with: window and pane backgrounds, pane border, UI text, pane text, the document colors of A, B, C and D, and the red of what differs from every other document. The user can override the UI text and pane text colors, the colors of A, B and C, and the red, per theme.
- **Theme colors**: the colors the user picked for one theme; a color left unset keeps the palette's.
- **Document colors**: a document's pane header color, which also fills the gutter blocks and chars marked in its color, the block border, and the stronger fill of active blocks and char-diff backgrounds. For A, B, C and the red they are derived from one picked color, the border.
- **Margin**: the strip left of a pane's text that stays in place when the text scrolls sideways: line numbers, then the gutter.
- **Gutter**: the column of gutter blocks left of a pane's text, one row height wide.
- **Gutter block**: a bordered block in the gutter spanning all rows of one region that differs, in every pane. Blocks of the current region are active. A block of a source pane is marked by the char-diff rules applied to its region.
- **Alignment column**: the strip between the line numbers and the gutter in a source pane: a cyan bar over each marked range, with its number, and a chip with the number on the edge of each anchor. The inset arrows are in a strip of their own left of the line numbers.
- **Line origin**: in D, where a line came from: A, B or C for a line from a chosen hunk, M for a hand edit. Each line of D in a gutter block is filled with its origin's color and labeled with its letter.
- **Current region**: the region navigation is at; in the GUI, the **jumped-to region**. Owned by the host and shown with a yellow background in every pane, void space included. A region of D that takes lines from two or three hunks is still one region.
- **Region box**: the bordered box drawn around the rows of a region whose hunks differ, as wide as the pane, with no background and rounded corners. Regions whose hunks are all equal get none.
- **Whitespace markers**: shapes drawn over whitespace in the panes: a dot for a space, an arrow for a tab, arrows after the text for the line feed and carriage return ending a line, a ring for other Unicode spaces.
- **Toolbar button**: a bordered button in the toolbar with its icon centered above its label, filled while its toggle is on.
- **Void space in the output pane**: D lined up with the source panes: every region gets the same rows in every pane, padded with void rows.
- **Scroll group**: panes that share one vertical scroll offset: the source panes, with D when void space in the output pane is on, and D alone otherwise.
- **Smooth scrolling**: wheel notches and jumps moving a scroll group toward where they go over a set duration instead of at once.
- **Char-diff rules**: the one match that turns a char run, or a region's gutter block, of a source pane into its style: red, or the color of the one other document it differs from.
- **Language**: the kind of text the documents of a session hold: a programming, markup or configuration language, or plain text. One per session, detected when the session opens and shown in the status bar. For now it is only a label: panes are not highlighted by it.
- **Language selector**: the drop-down at the right end of the status bar that shows the session's language with its logo and lets the user pick another.
- **Heuristics Alignment window**: the modal Ctrl+H (or, for a scoped run, Ctrl+Alt+H) opens: the options of Heuristics Alignment, Find Matches, and the table of proposed pairs, each with a checkbox and indented by how many pairs hold it, that Align marks and applies.

## Core data model (`kdiff4-core`)

### Source text

- Each source document is decoded once into a single UTF-8 string. Lines are ranges into that string, without their terminators, so loading does no per-line allocation.
- Alongside the text the engine keeps the encoding, whether a BOM was present, the EOL style (the majority style for mixed files; none when the text has no terminator) and whether the last line lacks a terminator.
- Encoding: BOM first, then user override, then `chardetng`, then UTF-8 fallback. Invalid bytes become U+FFFD and the file is flagged so save warns before lossy re-encode.
- Binary detection: NUL bytes in the first chunk mark the file as binary; binary files are compared by bytes only and are not merged.

### Line comparison

Lines are compared through their line keys, produced by a normalizer configured from options (ignore whitespace, ignore case, ignore trailing whitespace, ignore numbers, ignore comments). Line keys are interned into integer tokens before diffing, so the diff runs on integers.

### Comparison engine

The algorithm that compares documents is pluggable. The engine config names the comparison engine to use (`[diff]` key `engine`); the built-in engine is the default. The interface is defined in `kdiff4-protocol` and uses only protocol vocabulary types, so its results go into snapshots and deltas without translation, and a later adapter can run an engine out of process. The host keeps a registry of the available engines.

Responsibilities are split so that every engine gets the same merge behavior:

- The comparison engine computes: regions with their hunks, the rows of each region, region statuses (including whitespace conflicts), the status of a given set of hunks (used after a split), character-level diffs for the lines of one row, and binary compare. It honors the comparison options and the manual alignments it is given: the parts of each alignment share rows, and both ends of an alignment are region boundaries. This holds in 2-way and 3-way mode.
- The session keeps the merge state: the manual alignment list and its validation (alignments that cross or contradict each other are rejected before the engine sees them), deciding which part of the documents to recompute, splits, choices, auto-resolve, edited regions, undo, D, save, session files and navigation.
- Line normalization stays in the session: it builds line keys and tokens and hands them to the engine together with the text, so ignore options behave the same with every engine.

An engine reports its engine capabilities, which go into the snapshot so front ends can disable what it does not support. An engine without native manual alignment is driven through the segmentation helper, which gives it manual alignment for free. Nested alignments always go through the helper, whatever the engine claims. Binary compare has a byte-by-byte default that engines may replace.

The host compares only through one entry point, which validates the input (2 or 3 source documents, 3 only if the engine supports it; one token per line; valid manual alignments), hands the alignments to the engine or to the segmentation helper, and runs the result check. Alignments that a document holds in another order than the list are laid out in display order first, so the engine sees documents and alignments that keep one order. The segmentation helper also checks each segment's result on its own, so an engine cannot spill a region across a segment boundary.

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

With `pair_similar_lines` (on by default), every changed region is paired again from its hunks alone, the same way, except that the lines of a changed chunk are paired by how alike they are: in order, so that the pairs at least 50% alike (twice the common pairs of adjacent characters over all of them, leading and trailing whitespace left out) are as alike as possible in total. The lines between such pairs are paired by position, so a chunk without similar lines keeps its rows. The region takes the new rows only when they score higher than its rows by position, counting 1000 for each two equal lines in a row and the similarity for each two lines at least 50% alike, and when none of them is equal in every document. Hunks and statuses never change, only the rows inside a region: a line that changed a little keeps its partner even when lines were added before it, and equal lines of B and C still share a row before any similar ones do, as in kdiff3. Chunks of more than 250,000 line pairs are paired by position.

Pairing B and C across rows that also hold a line of A (kdiff3 `Diff3AlignBC`) is not implemented yet; the built-in engine reports that option as not honored.

Character-level diffs are computed by the session with the comparison (on open, reload and new comparison options) for every row whose lines are not all present and equal, and sent in the snapshot, per source document and line. Rows depend only on the comparison, so edits of D, which only join regions and keep their rows, never make them stale. Each line of a row is diffed by characters with every other line in the row, the way kdiff3 does it (`fine_diff.rs`: its own match search, not an LCS, short matches merged into the change next to them, and, unlike kdiff3, each change starting at its first differing character when the lines have a long match; the pairs are A with B, B with C and C with A, because the result depends on the direction), and the result is a list of char runs covering the line: each run says in which documents of the row its characters are the same. A void is never the same. Above the line length limit two different lines are different as a whole. An engine without character-level diffs, or one that fails, leaves every line unmarked.

kdiff3 reference: `diff.cpp`, `pdiff.cpp`, `test/alignmenttest.cpp`. We reuse its test data, not its algorithm, except for the character-level diff of a row (`fine_diff.rs`, `DiffList::calcDiff` and `DiffList::optimize`).

### Splitting regions

The user can split a region into smaller ones at row boundaries, like staging single lines in `git gui`. Each part gets its own hunks, status and choice, so a large conflict can be resolved piece by piece; parts that turn out to be one-sided auto-resolve.

- The choice and edits of the original region are dropped; the parts start unresolved unless their status auto-resolves.
- Splitting is one undo step.
- A recomputation (options change, reload, applying alignments) drops splits.

### Manual alignment

The user can override the automatic alignment by marking which lines of the source documents belong together, and then applying the marks.

**Why.** Automatic alignment pairs lines by content and position, which goes wrong when unrelated code lands in the same place. Typical case: B introduces function "Foo" after "Bar" and before "Baz"; C introduces a different function "Qux" at the same spot. Both changes touch the same place in A, so the engine builds one conflict region and pairs look-alike lines of the two unrelated functions (braces, `return`, blank lines) into rows. The user wants both functions in D, one after the other. Manual alignments let them say so.

**Marks and numbers.** A mark belongs to one source document and is either:

- a line range: one or more adjacent lines the user selected, the smallest unit being a line, or
- an anchor: a position between two lines (or at the start or end of the document), used when the aligned content does not exist in that document.

Every mark has a number. A new mark takes the lowest number its document has not used yet, so the first mark of each document is 1, the second is 2, and marks pair up by order across documents. A mark can be renumbered; when another mark of the same document has that number, the two swap. A document has at most one mark per number.

Marking, renumbering, removing and clearing change nothing in the regions, rows or D. The marks are kept as they are until the user applies them.

**Alignments.** The marks with one number form an alignment only when every source document has one (A and B in 2-way mode; A, B and C in 3-way mode). Marks of any other number are kept and ignored when applying; they show in the alignment column as unmatched. All parts of one alignment occupy the same rows. Anchors, and ranges shorter than the longest part, are padded with void regions. The alignments are ordered by position, not by number: the numbers only pair marks across documents, so 2 above 1 is fine when it is so in every document. Where the documents disagree on the order, see Inset alignment. Where an alignment lies inside another one, see Nested alignment.

**Rules for marks.**

- Marked ranges of one document must not cross: two ranges may be equal, touch or contain each other, and an anchor may lie anywhere, inside a range too. This is checked when marking, and the mark is refused with a message. Whether the marks of all documents fit together is checked when applying: see Nested alignment.
- Several anchors may share one position. Their order is decided by where the same alignments lie in the other documents; alignments at the same positions in every document are ordered by number.
- An alignment whose parts are all anchors is refused.
- Alignments that cross, because X comes before Y in one source document and after it in another, are insets. They are accepted and shown in one order; see Inset alignment.
- An alignment whose parts lie inside the parts of another one, in every document, is nested in it; see Nested alignment.

Example: Foo and Qux from above are separated with two alignments.

| Number | A | B | C |
|---|---|---|---|
| 1 | anchor before Baz | Foo | anchor before Qux |
| 2 | anchor before Baz | anchor after Foo | Qux |

The two anchors in A share a position and are ordered by number. Alignment 1 turns Foo into a Merge Region with voids in A and C, status Changed in B; alignment 2 does the same for Qux, status Changed in C. Both auto-resolve, so D gets Foo followed by Qux with no conflict. To get Qux first, give Qux's alignment the lower number in every document.

**Applying.** Apply Alignments hands the alignments the marks give to the engine and compares all documents again from scratch. Like a reload, it rebuilds rows, regions, character-level diffs and D: choices, hand edits, splits and the undo history are dropped, and it is not an undo step. So the host refuses to apply while D holds an edited region or a choice that differs from what auto-resolve makes, unless the command says the user confirmed; the GUI then shows a warning with Proceed and Cancel. When the marks give the alignments already applied, applying does nothing and warns nothing. If the engine or the check fails, the session is unchanged.

### Inset alignment

An inset alignment is a manual alignment that the source documents disagree on the order of. Example, with the numbers of the marks in the order they stand in each document:

| A | B | C |
|---|---|---|
| 1 | 2 | 1 |
| 2 | 1 | 2 |

Function 2 stands above function 1 in B, and below it in A and C. An inset alignment is a manual alignment like any other, except that the alignments don't keep one order in all documents. Rows can't show both orders, so the alignments are shown in one **display order** (1 above 2, or 2 above 1), each as one row of matching parts. A document that has its own order exchanges its hunks: the text between the hunks stays where it is, and the place of each hunk shows the hunk of the alignment that has that place in the display order. With 2 shown above 1, B keeps its text; in A and C the place of function 1 shows function 2 and the place of function 2 shows function 1, with the blank lines between them where they were, so they read like B character by character. D follows the display order, so choosing the order of B also puts B's order into D.

Display order is decided among siblings only (see Nested alignment): an inset alignment is shown among the alignments inside the same alignment, or among the top-level ones.

**Where it is shown.** When a mark completes an alignment that crosses others, the alignment is shown where the fewest documents keep their order, which is where the document that differs from the others has it: that document most likely moved the code on purpose, and following its order puts the change into D. In the example, 2 is shown above 1, B keeps its order, and the hunks of A and C are moved. Ties go to the first document, so a 2-way comparison keeps A's order and moves B's hunk. The answer to the mark carries the places the alignment could be shown at (the inset choice) and asks the user, who picks another place in a window or keeps the suggestion, which is in effect already. Later the menu of the alignment column of any alignment the documents disagree on lists the other places: "Show Elsewhere". A place is "above", "below" or "between" other alignments, with the documents whose hunk it moves. Placing, like marking, changes nothing in the comparison until the user applies.

A place is kept until the marks of its alignment change; then the alignment is placed again by the rule above, without asking. Renumbering can therefore change places silently, and the menu can put it back.

**Exchanging hunks.** Take the parts of a document in the order they have in it: those are its places. The place of the `n`-th part shows the part of the `n`-th alignment of the display order, and everything between the places stays. Parts at one position, such as anchors, keep the order of the display order, so they never swap among themselves. So every line of the document is shown once and none is added or lost between the alignments: the blank lines that separated two functions stay between them wherever the functions go. Only the parts outside a longest run that already has the display order count as moved. If the first of ten functions of a document moves to the end, the others follow along one place up, and only that function is moved.

**The comparison.** The engine never sees an exchange. The documents are laid out in display order, compared like documents with alignments, and the regions are brought back to the documents. A place of the layout where the next line is not the next line of the document is a seam. Seams are edges of alignment parts, which are region boundaries, so every region is one stretch of lines in every document. The check reads the regions in display order: in every document the regions cover each line once, and the regions of moved hunks may stand out of the document's order.

**Anchors and ranges.** Anchors take part like ranges: an alignment can be an inset by its anchors in some documents. An anchor takes the place of the part it is exchanged with, and that part takes the place of the anchor. The chip of an anchor that is exchanged is still drawn at the anchor's own position in the document.

**Arrows.** After the alignments are applied, every moved hunk of an applied alignment has an arrow in the strip left of the line numbers of its source pane, drawn like a branch in a graph: a dot next to the line numbers on the first row of the hunk that is shown where it was, a line down its lane, and a tip pointing at the first row of the hunk. Arrows that overlap in height take different lanes, next to each other with `inset_arrow_gap` pixels between them (1 by default). The strip is as wide as its lanes need and is not there without arrows. Anchors have no arrow, and neither do hunks that only follow along.

Clearing removes every mark but keeps the rows as they are; applying afterwards compares without alignments. Reloading a source document drops the marks and the applied alignments, since their lines moved. Changing the comparison options compares again with the applied alignments and keeps the marks.

**Effect on the engine.**

- Alignments cut every source document into segments: the alignments themselves and the stretches between consecutive alignments. An alignment with nested alignments is cut further inside its parts, the same way. Each segment gets its regions and rows on its own with the process above; rows and regions never span a segment boundary.
- Both ends of an alignment are forced region boundaries, so its contents never merge with anything outside it. Inside an alignment, lines pair up and regions form as usual, so an alignment can hold several regions.
- These rules bind every comparison engine. The built-in engine meets them through the segmentation helper.

**Navigation.** With the setting on, Previous and Next Difference also stop at the first region of every applied alignment, whether or not its hunks differ. The stop is the region that has the alignment's first lines, found from the regions' hunks, so it survives regions joined by edits.

Supporting crossing alignments is a possible later feature: it would allow merging three files even when functions were reordered relative to each other in all three source documents.

kdiff3 reference: manual diff help in `diff.cpp`. Differences are listed in `DEVIATIONS.md`.

### Nested alignment

A nested alignment is a manual alignment that lies inside another one in every source document: each of its parts is inside the part of the other alignment in the same document. Example: the alignment of a class (1) and, inside it, the alignment of one of its methods (2).

| A | B | C |
|---|---|---|
| 1: lines 10 to 40 | 1: lines 12 to 44 | 1: lines 10 to 41 |
| 2: lines 20 to 25 | 2: lines 22 to 27 | 2: lines 20 to 26 |

**The relation of two alignments.** In each document two parts are apart (they don't overlap, but may touch, and anchors may share a position), one is inside the other (an anchor may lie anywhere in a range, edges included, and equal parts are inside each other), or they cross. For two alignments:

- if their parts are apart in every document, they are **siblings**; this wins over the next rule, so every set of alignments without nesting keeps its meaning;
- else if one's parts are inside the other's in every document, the first is nested in the second;
- equal alignments, which are inside each other everywhere, are refused ("alignments X and Y are the same");
- anything else is refused: parts that cross in some document, or documents that disagree, such as 2 inside 1 in A and outside it in B ("alignments X and Y overlap in A", naming the first document where the parts cross, or else the first where they are not apart).

**Sibling groups.** The smallest alignment that encloses an alignment is its parent. The alignments with one parent are a **sibling group**, and so are the top-level alignments, those without a parent. Everything that is said about the order of alignments holds per sibling group: the order of the list is the display order within a group, a document that orders the members of a group differently exchanges their hunks (see Inset alignment), and the parts that count as moved are decided within a group. A hunk carries its whole content with it, nested alignments and the text between them included, and inside it the nested alignments are laid out the same way. So a nested alignment moves with its parent, and an inset among nested siblings moves only within the parent and leaves the parent's own siblings where they are. The list of alignments may hold the groups in any interleaving; only the order within each group counts. A new alignment is placed among its siblings, and "Show Elsewhere" lists places among them.

**The comparison.** An alignment with nested alignments is cut up inside its parts: the text before the first child, the child, the text between children, and so on. Each child is a segment of its own, or cut further the same way. Both ends of every alignment, nested ones included, are region boundaries. Nesting is not left to engines: the segmentation helper cuts nested alignments even for engines with native manual alignment.

**The alignment column.** Marks that lie inside other marks of the same pane are drawn one bar further right per level of nesting, and the column grows by one bar for each level. The pointer finds the innermost mark.

Heuristics Alignment creates nested alignments for the entities inside paired entities, and keeps to the marks of the user: a match that lies inside or around the alignments of the user is accepted, one that crosses one or contradicts one is left out.

### Heuristics Alignment

Marking every function of three documents by hand to merge moved code is slow. Heuristics Alignment does it: it finds the entities of every source document, pairs them across the documents, then the entities inside each pair, and marks each pair as one alignment, nested in the alignment of the pair that holds it. Where a document moved an entity, its alignment is an inset and the hunks are exchanged as described above.

**Entities.** The extractor parses each document with the grammar of the session's language, the one the language selector shows (C, C++ and CUDA for now; a C document that does not parse cleanly as C is parsed as C++, since `.h` files often hold C++). It finds:

- functions: function definitions with a body, outside types;
- methods: function definitions inside a type, or with a qualified name such as `Widget::draw`;
- types: classes, structs, unions and enums with a body. With types on, a class or struct with members to align is an entity whose children are those members (methods, nested types); an enum or a plain struct is an entity without children. With types off, the members of a class stand at the level of the class and the class itself is not an entity;
- blocks, with blocks on (or in a scoped run): inside the body of a function or method, and recursively inside blocks, the `if` statement with its whole `else if` and `else` chain, `for`, range `for`, `while`, `do`, `switch`, every `case` or `default` of a switch, `try` with its handlers, a lambda (as the statement that holds it, when that statement starts its line), a local class or struct (a type, with the rules above) and a nested function. A block spans at least 3 lines, from its header to its closing line: smaller ones are not entities, and the blocks inside them move up to the block around. A block is named, and its signature is, its header: the text before its body, in the same normal form as a signature (a `case`: the label up to the colon; a `do`: `do while` and its condition). The blocks inside the branches of an `if` chain are children of the `if`; the cases of a `switch` are children of the `switch`.

Declarations without a body are not entities. Namespaces and types qualify the names of what they hold. An entity's lines run from the comments right above it (each starting its line, with no blank line between them and the entity) to its closing brace; the blank lines after it stay outside, so exchanging hunks keeps the spacing. The entities inside an entity are its children, in the order of the document and inside its lines. The signature is the text before the body (a constructor's initializer list left out) without comments, with whitespace kept only between two words, so formatting changes don't count. tree-sitter recovers from what it can't parse, and the extractor takes the entities it still finds; preprocessor branches that open a block twice (`#ifdef` with two function headers and one body) confuse it.

**Pairing.** Two documents are paired in stages. Each stage pairs only what is unambiguous, and what one stage paired is out of the next:

1. Equal signatures, unique in both documents.
2. Equal qualified names, when the option is on and the name is left once in each document. Overloads, names that a document has more than once, are kept out, so a changed overload is never paired with the wrong one; an option pairs the one overload left over after the equal signatures.
3. Similar entities, when the option is on: the same kind, a body with at least the set share of lines in common (twice the common lines over all lines, default 50%), and a signature at least the set share alike (100% less the edit distance over the longer length, default 80%). A pair is taken when each is the other's best and no other candidate has the same score. Overloads are kept out like in stage 2. The body check keeps `getX` from pairing with `getY`.

Blocks are the exception to the overload rule: several blocks of one function often share a header (`for(inti=0;i<n;++i)`), so blocks with equal names are still paired in stage 3 by their bodies.

In 3 documents, A is paired with B and with C; entities paired with the same entity of A form one alignment, with the weaker stage of the two. Entities of B and C left unpaired are then paired with each other. An entity that only two documents have is aligned, when the option is on, at an anchor in the third: between the complete alignments right above and below it, when those two are next to each other, in that order, in the third document too (or after the last one, or before the first one); otherwise it is left out.

**Children.** Once the entities of a level are paired, the children of every pair that has an entity in each document are paired the same way, with the same stages and options, and so on down. A pair with an anchor has no children. A missing child is anchored among its siblings. The pairs come as a list, depth-first by position in the first document, each naming the pair that holds it (`parent`, the index in the list; parents come first). Members of a type pair freely and may be reordered, which makes insets among their siblings. Blocks pair in order only: of the paired blocks of one parent, the largest set that every document has in the same order stays (the longest chain by dominance of the lines in every document), and the others, with the blocks inside them, are left out, so blocks never become insets.

**The window.** Ctrl+H (Merge → Heuristics Alignment…) opens a window with the options it was last closed with and asks the engine for its pairs at once. The table lists each pair with a checkbox, its kind (function, method, type or block), the name and lines in every document (or the anchor), how it was paired and whether a document moves it; the names are indented by how many pairs hold the pair. Unchecking a pair unchecks the pairs inside it, and checking one checks the pairs around it, since a pair inside an unchecked one would mean something else. The Blocks inside functions option is off by default. Changing an option makes the table stale until Find Matches runs again. Align marks the checked pairs and applies all marks, warning like Apply Alignments when that would discard choices or edits in D.

**Scoped run.** Ctrl+Alt+H (Merge → Heuristics Alignment Inside…) aligns what lies inside one applied alignment, for example a function the user marked by hand. The alignment is the innermost applied one that holds the caret line in the focused pane (the smallest part there; of equal parts the one nested in the most alignments); without a caret in A, B or C inside an applied alignment it reports an error. The window is the same, titled Inside alignment *n*, with the same options (blocks are always on). The engine parses every document as usual and takes, per document, the outermost entities inside that document's part of the alignment; while each document has exactly one (a single block without children stays), their children are taken instead, so a marked function aligns its insides. Applying a scoped run replaces only the heuristic marks nested inside that alignment in every document; the other heuristic marks and the user's marks stay.

**Marks.** Heuristic marks are alignment marks like the user's, with the lowest numbers no document uses, in the order of A, and they can be renumbered and removed the same way. Running Heuristics Alignment again replaces the heuristic marks and keeps the user's own; pairs that cross the user's marks, take exactly the lines of one, or contradict the user's alignments are left out and counted in the proposal; pairs inside or around them are kept. Insets are shown where the rule of Inset alignment puts them, never asking: the document whose order differs from the others keeps its order, and A's order wins a tie. The menu of the alignment column can show one elsewhere afterwards.

**Over the protocol.** `ProposeHeuristicAlignments` (language, options, `within`) is answered with a `HeuristicProposal` event before its response and changes nothing; a language the extractor can't parse is refused, and so is a `within` that is not the number of an applied, complete alignment. Pairs that collide with the user's marks are left out together with the pairs inside them. `ApplyHeuristicAlignments` carries the checked pairs, one part per document each, and the same `within`, so the host needs no state between the two commands.

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
- Editing D inside a region turns it into an edited region. Inside an unedited region whose hunks are equal, the region is split instead: each line the edit touches becomes an edited region of its own, spanning that line in every source, so the region count stays the same across documents. New lines beyond the touched ones get regions with no lines in the sources; touched lines left without new text get regions with no text in D. The lines around the edit stay as they were. Such a line stands out like a difference: it is boxed with a gutter block in every pane, and Previous and Next Difference stop at it. Choosing a single hunk for such a line drops the edit and joins it back with the unedited regions around it that it continues.
- Undo/redo: every mutation is a merge op with a stored inverse. Groups of ops (e.g. "choose B everywhere") are one undo step.
- Save writes D with the Output EOL, and the encoding, BOM and final line break of the output reference (A by default, configurable), atomically via temp file + rename. D itself always ends every line with `\n`; the last one loses it on save when the output reference's last line has none. Save is refused with `EolConflict` while the Output EOL is in conflict. Choosing a style settles it; it stays until a source document is loaded again, which computes it anew.
- Implemented so far:
  - D is built at open by auto-resolving every region; an existing output file is not read. Its text is always the regions' texts in order, and each region contributes whole lines, each with a line break.
  - Merge ops: setting a region's choice (an ordered list of hunks, possibly empty, or unresolved), which drops its hand edits; choosing one side for every unresolved region; hand edits. Every op replaces a run of regions and the matching part of D, and is one undo step that restores both. Typing inside one region still joins into one step.
  - A hand edit belongs to the regions whose text it touches. An insertion goes to the region holding its position, so typing at the start of the line after an empty unresolved conflict goes to that line's region, not into the conflict: to write a conflict by hand, choose a hunk first and edit it. An edit can instead name a region that has no lines in D; it must then insert whole lines at that region's place, and goes to that region alone, which keeps its choice and becomes edited. This is how typing on an empty region row works, since a position alone can't tell such a region from its neighbors. If the edit leaves the touched regions without a final line break while text follows, the next region is taken in too. When several regions are touched they are joined into one edited region with no choice, spanning all their hunks and rows, and the engine classifies it again; undo brings the original regions back.
  - An unresolved conflict is a region with no choice and no hand edits. Save and Save As are refused while one exists.
  - Reloading one or all source documents, or applying alignments, recomputes the comparison and rebuilds D the same way as at open: choices and hand edits are dropped and D's undo history starts over, with D marked modified.
  - Not yet: the whitespace default sides, the auto-resolve and save-and-quit options, and splitting.

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

- Commands: open, reload, cancel, choose hunks for a region, choose a side for all unresolved regions, split a region, edit D, mark, remove, renumber and clear alignment marks, place an inset alignment, apply alignments, propose and apply heuristic alignments (optionally inside one alignment), undo, redo, save, set engine config, navigate, set the current region. Every user action in any front end ends up as a command.
- Planned: save the session to a session file; open a session file.
- Implemented so far: open; reload one source document from a (possibly new) path, answered with a new snapshot; set the choice of a region and choose one side for every unresolved region (both tagged with the revision their region indices refer to); edit D (replace a byte range, tagged with the revision it was made on; a stale revision or a range off character boundaries is rejected); set D's path without writing it (or clear it, making D unnamed); save; save as; set the Output EOL; undo and redo of D; resync (the host re-sends a snapshot); set the comparison options, which compares again and rebuilds D like a reload and is answered with a new snapshot; navigate to the next or previous difference or conflict from a region (the caret's, or the current one); set the current region. Navigation and setting the current region are answered with a current-region event. Navigating can ask to stop at applied alignments too. Marking, removing, renumbering and clearing alignment marks are answered with a delta carrying every mark with its status (applied, pending or unmatched) and whether applying would change the comparison. Applying alignments is answered with a new snapshot, does nothing when the marks give the alignments already applied, and fails with a needs-confirmation error when D holds choices or edits and the command does not say the user confirmed. Deltas so far: D changed by a merge op, undo or redo (the replaced range of D's text, the merge update, and whether D now differs from its saved text, which undo can bring back), D's path changed, D saved, the alignment marks changed. The snapshot carries each region's merge state and the alignment marks.
- D's undo history is kept by the host. Typing on one line is one undo step, and so is a run of Backspace or of Delete; a line break, a paste, replacing a selection, a word deletion, save, undo and redo start a new step. Undoing back to the saved text clears the modified flag.
- D's text crosses the protocol with `\n` line ends. It is saved as UTF-8 with the Output EOL, and the BOM and final line break of A, the output reference. The snapshot and the `OutputEolChanged` change carry the Output EOL as a style or a conflict.
- Queries: reads the mirror does not hold: config help text.
- Progress events report long work (job, amount done, total); notices carry warnings such as a pending lossy re-encode.
- The first exchange is a handshake with the protocol version; a mismatch is an error.

### State mirroring

- After open, the host sends a snapshot: document texts and metadata (labels, encoding, EOL style or none, whether the document is a detached source), regions, rows, D, alignment marks, engine config, comparison options, engine capabilities, the current region and the revision.
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

- `[view]`: font family (`font_family`, monospace only; absent means the built-in font), font size (`font_size`, 6 to 48), font hinting (`font_hinting`), subpixel glyph positioning (`subpixel_positioning`), corner radius of widgets, dialogs, panes and the window frame (`corner_radius`, 0 to 12), line numbers (`show_line_numbers`, on by default), gutters of the source panes (`show_gutters`, on by default), void space in the output pane (`align_output`, off by default), the overview strip (`show_overview_strip`, on by default), the minimap (`show_minimap`, on by default), lines kept above a region navigation jumps to (`jump_context_lines`, 0 to 20, default 12), smooth scrolling (`smooth_scrolling`, on by default) and its duration (`smooth_scroll_ms`, 20 to 1000 milliseconds, default 150), lines one mouse wheel notch scrolls (`scroll_step_lines`, 1 to 20, default 3), the background of the jumped-to region (`current_region_color`, red, green, blue and alpha, 0 to 255 each, a translucent yellow by default), the color of the alignment marks and of the arrows of moved hunks (`alignment_color`, red, green, blue and alpha, cyan by default), the space between the lines of those arrows (`inset_arrow_gap`, 0 to 8 pixels, default 1), whether Previous and Next Difference also stop at applied alignments (`alignment_stops`, on by default), the extractor of Heuristics Alignment (`heuristic_extractor`: `TreeSitter`, the only one so far, set under Settings → Comparison), the theme (`theme`: `light`, the default, `dark` or `system`), whitespace markers (`[view.whitespace]`: `render`, on by default, and per kind `spaces`, `tabs`, `line_feeds`, `carriage_returns`, `other`, all on by default), the corner radius of region boxes (`region_corner_radius`, 0 to 8 pixels, default 2), the opacity of the borders of the regions other than the jumped-to one (`other_region_border_percent`, 0 to 100, default 45), tab size (`TabSize`), replace tabs (`ReplaceTabs`), auto-advance after choosing (`AutoAdvance`, `AutoAdvanceDelay`), Escape quits (`EscapeKeyQuits`).
- `[view.light_colors]`, `[view.dark_colors]`: per-theme overrides of the UI text color (`ui_text`), the pane text color (`pane_text`), the document colors of A, B and C (`document_a`, `document_b`, `document_c`) and the red of whatever differs from every other document (`red`), red, green and blue, 0 to 255 each; absent keeps the theme's color. Light text is black by default. A document color or the red is one color: the border of its gutter blocks; the pane header, the block fill and the stronger fill of active blocks and char-diff backgrounds are derived from it by blending it into the pane background. The old `char_diff_red` key of `[view]` is ignored.
- `[colors]`: foreground, background, removed, added, modified, conflict, manual alignment ranges, void regions.
- `[window]`: size, position, maximized, full screen.
- `[updates]`: look for a newer release when the program starts (`check_automatically`, on by default), Unix time of the last successful automatic check (`last_check_unix`, 0 when none), and the release the user chose to skip (`skipped_version`, absent by default).
- `[heuristics]`: the options the Heuristics Alignment window was last closed with: which entities it looks for (`functions` and `methods` on, `types` and `blocks` off by default), name pairing (`match_names`, on), similar pairing (`match_similar`, on) with its thresholds (`signature_similarity`, default 80, and `body_similarity`, default 50, in percent), the leftover overload (`overload_leftovers`, off) and anchors between neighbors (`missing_by_neighbors`, on).
- `[recent]`: recent A/B/C/output files and encodings, directory of the last file confirmed in a file dialog (`last_dir`).

### Precedence

1. Built-in defaults: every key has one.
2. The owner's config file.
3. Overrides from `--cs section.key=value`: applied for this run only and never written back.

Each owner keeps two copies: the file config and the effective config (file plus overrides). Edits in the settings dialog change both and save the file config, so an override never leaks into the file unless the user changes that key in the dialog.

The binary routes each override by its section to the engine (in the open command) or to the GUI. An unknown section or key, or a value of the wrong type, is a config error: the message is shown and the program exits with code 1, as kdiff3 does. `--confighelp` prints every key of both files with its effective value and exits.

### Loading and saving

- Missing file: defaults are used; the file is created on the first save from the settings dialog. The dialog's "Open in editor" button opens the file with the system's default handler, creating it first if it is missing.
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
  - Each pane has a header with the document letter (with `*` while D has unsaved changes), its label, an editable path field and a Browse button. In A, B and C the EOL style of the document (LF, CR or CRLF) is written left of Browse; a document without a line terminator shows none. Enter in the field, or a file picked with Browse, reloads A, B or C from that path and rebuilds D from the new comparison; for D it only changes where D will be saved. For a detached source the path field and Browse are disabled.
  - File → Save Session and File → Open Session write and read `.kd4` files.
  - Panes show a caret and its selection; there is one on screen. Pressing the mouse button in any pane moves the caret there at once, and dragging selects (Shift+click extends). Clicking below the text goes to its end, and right of a line to the line's end. Double-click selects a word (or a run of whitespace, or one punctuation character), triple-click a line. The keys are arrows, Ctrl+Left/Right by words, Home/End, PageUp/PageDown and Ctrl+Home/End, with Shift to select, plus Ctrl+A and Ctrl+C. Up, Down, PageUp and PageDown move row by row, so the caret can stop in void space: it is drawn at the void row's left edge, and its position is the start of the line after the void, which is where typing and selections go. A, B and C are read-only. D is editable: typing, Enter (also with Ctrl held), Backspace, Delete, Ctrl+Backspace and Ctrl+Delete by words, Ctrl+V and Ctrl+X, which replace or remove a selection; Edit → Undo (Ctrl+Z) and Redo (Ctrl+Shift+Z) work on D wherever the caret is. Each edit is applied to the mirror at once and sent to the engine.
  - Margin: left of the text every pane has line numbers (`show_line_numbers`), blank on void rows. Source panes then have an alignment column (see Alignment marks) and a gutter (`show_gutters`): one bordered block per region that differs or that D holds a hand edit of, spanning all its rows, void rows included; other regions whose hunks are all equal get none. A block follows the char-diff rules, applied to its region: it is in the color of the one other document the region differs from, red when the region differs from every other document, and a 2-way diff marks A's blocks in B's color and B's in A's (a region changed only in B is B-colored in A and C and red in B; one changed alike in B and C is A-colored in B and C and red in A; a conflict is red in every pane). A hand-edited region whose hunks are all equal keeps the pane's own document color. D always has a gutter, with the same blocks as the source panes: one per such region, bordered in D's neutral color. Each line in it is filled with its origin's color and labeled with A, B, C or M; an unresolved conflict's placeholder, an empty region row and lines of unknown origin stay neutral and unlabeled. The gutter is one row height wide, so a one-row block is a square. Clicking a block makes its region current: its blocks in every pane turn active, the caret goes to the region's start in the clicked pane and stays visible, and every pane scrolls as for a jump (see Navigation), whichever pane was clicked, so the source panes and D always meet at the same region. The margin stays in place when the text scrolls sideways.
  - Alignment marks: every source pane has an alignment column between the line numbers and the gutter, one row wide and always there, so panes never shift when the first mark is made. D has none. A marked range is a bar over its rows, void rows between its lines included, with its number on the first row; an anchor is a chip with its number on the edge above its line, or below the last line for the end of the text; anchors sharing a position share a chip that lists their numbers. Marks nested in other marks of the pane are drawn one bar further right per level (see Nested alignment). Everything is drawn in `alignment_color`, which is cyan by default. The arrows of moved hunks, in the strip of their own left of the line numbers, are drawn in it too (see Inset alignment). An applied mark is filled strongly, a pending one lightly, and an unmatched one is only outlined faintly; the tooltip says which and why. Ctrl+Space (Merge → Mark Selected Lines for Alignment) marks the lines the selection touches in A, B or C, or the caret's line without a selection; a selection ending at the start of a line leaves that line out. Ctrl+Shift+Space marks an anchor before the caret's line. A right click in a source pane opens a menu to mark the selected lines (when the caret is in that pane) or an anchor before or after the line under the pointer; on a mark in the alignment column it also offers the mark's Change Number, listing every number up to one past the highest in use, and Remove. This menu is the only way to remove one mark or renumber it. Dragging the top or bottom edge of a range bar (the pointer shows a vertical resize cursor within a few pixels of the edge) stretches or shrinks the range, and dragging an anchor's chip moves the anchor; an outline shows where the mark would stand, the edge snaps to the nearest line boundary (a range keeps at least one line), Esc cancels, and releasing sends `SetAlignmentMarkLines`. Heuristic marks can be dragged too and become the user's own. Ctrl+Y (Merge → Apply Alignments) applies, warning with Proceed and Cancel when that would discard choices or edits in D; Ctrl+Shift+Y (Merge → Clear All Alignments) removes every mark. Ctrl+H (Merge → Heuristics Alignment…) opens the Heuristics Alignment window and Ctrl+Alt+H (Merge → Heuristics Alignment Inside…) opens it for the alignment around the caret (see Heuristics Alignment). The status bar says when marks wait for Apply, and a refused mark or apply shows its reason there.
  - Region boxes: in every pane, D included, each region whose hunks differ is drawn as a bordered box across the full pane width, margin included, spanning all its rows, void rows included. Boxes have no background and rounded corners (`region_corner_radius`); the border of the jumped-to region is drawn at full opacity, the others at reduced opacity. Rows have no background shade by change; the boxes mark the differences.
  - Toolbar under the menu bar, shown while a session is open (not over the Open window): bordered buttons with their icon centered above the label. Void Space, with a hatched square icon, toggles void space in the output pane (`align_output`); Merge → Void Space, after a separator at the bottom of the menu, does the same. On, every region gets the same rows in every pane, padded with void rows, and D scrolls with the source panes; off, D has just its own rows and scrolls on its own. Overview Strip, with an icon of lines in a box, shows or hides the overview strip (`show_overview_strip`), in merges and 2-way diffs alike. A press in the strip gives it the keyboard: Up and Down then move the caret to the line above or below in the strip (A, B, C, D), at the same column, skipping lines with no row there and stopping at the first and last. Clicking a pane or gutter block, or jumping to a difference, gives the keyboard back to the panes, where Up and Down move by rows. While the strip has the keyboard its rows are fixed: nothing moves the caret off its row. Left, Right and the word motions stop at the line's ends instead of wrapping, on a void or a `<Merge Conflict>` row no horizontal motion moves, PageUp, PageDown and Ctrl+Home/End do nothing, Ctrl+A and a triple-click select the line's text without its line break, and edits that would split, join, add or remove lines (Enter, Backspace at the start, Delete at the end, pasted line breaks, selections over a line break, Delete or Backspace on a void) are ignored. Both toggles are saved at once and followed by an open Settings dialog.
  - File → Reload (F5) reads A, B and C again from their paths and compares them anew (`Command::Reload`), warning with Proceed and Cancel when that would discard choices or edits in D.
  - File → Save writes D to its path, or asks for one like Save As when D has none. Save and Save As are disabled while unresolved conflicts remain; Ctrl+S then reports how many are left. Save As picks a new path with the native Save dialog, which asks before overwriting, and D moves there. The outcome of saving, reloading and path changes shows in the status bar under the panes. After open and after a reload it shows how many differences and conflicts the comparison found, and how many of the conflicts are whitespace only. The status bar is always there, one line high, so the panes never move when a message comes or goes; long messages are cut, with the full text in a tooltip. When the Output EOL is in conflict, saving (Save, Ctrl+S or Save As, after the path is picked) first opens a window with the buttons LF, CR, CRLF and Cancel; a style sets the Output EOL and then saves, Cancel (or Esc) saves nothing.
  - Output EOL selector: in merges, the status bar holds a drop-down with the Output EOL just left of the language selector, listing LF, CR and CRLF. In a conflict it reads "Conflict line ending" in red, and picking a style settles it.
  - Language selector: the right end of the status bar holds a drop-down with the session's language, one for A, B, C and D together: Plain text first, then programming, markup and configuration languages by name. It is detected whenever a snapshot arrives, that is when the session opens or reloads: document by document, A first, then B, C and D. A is the source of truth: when the documents are named for different languages, A's language wins, and a later document counts only when every one before it names none. A document names its language by its path, by file name and then by extension, ignoring a backup ending such as `.orig` or `.bak` (so the temporary names of merge tools, like `main_BASE_1234.rs`, work); failing that by its first line: a shebang, `<?xml`, `<?php` or an HTML doctype. Plain text is the fallback. A language picked by hand stays while the session has the same source files and is replaced by detection when one is loaded from another path or another session opens. It is not saved. Each language has its logo in front of its name, in the selector and in its list: an SVG from the vscode-icons set (MIT; the files and their license text are in `crates/kdiff4-gui/assets/language_icons`), drawn through the SVG loader of `egui_extras`. Makefile and Properties share the set's generic configuration icon, as it has none of their own. The language is only shown for now: the panes are not highlighted by it.
  - Minimap: a narrow column (28 pixels) at the right edge of the session, from the top of the source panes to the bottom of the output pane, standing for the whole document in the rows of the source panes. Every region that differs is a block in the color of the document it differs in (B or C; B in a 2-way diff), red for a conflict, and the neutral color for a hand-edited region or a change B and C share; the jumped-to region has an outline. A rectangle shows what the source panes show; pressing or dragging in the column scrolls them, centered on the pointer, at once, like a scroll bar. D scrolls with them when it lines up and on its own otherwise. Ctrl+G, the View → Minimap item, the Minimap toolbar button and the Settings dialog toggle it (`show_minimap`); all are saved at once.
  - Overview strip: a lens row between (A/B/C) and the splitter above (D), one line per document labeled with its letter in the document's color, with its line number when line numbers are on; in a 2-way diff without D it sits below the source panes with just A and B. It shows the row that lines up with the caret: the caret's row in the source panes, and in D the row the same distance into the same region, which is exactly the row beside it with void space in the output pane on; with the caret in D the source rows follow the same way back. A document with no row there, and a void, is hatched; D's placeholder shows the conflict marker. Without a caret it shows the start of the current region, and after a jump the region jumped to. A, B and C lines carry the same character diff as their panes, drawn by the same rules; D has none. All lines scroll sideways together, and the strip scrolls the caret into view when it moves. Pressing, dragging, double- and triple-clicking on a line work as in that document's pane and move the one caret there, so D is edited in the strip through the caret like in its pane, and the edit shows in both at once; a press on a void or placeholder of D puts the caret on that row, where typing gives the region lines. A press on a document with no row there does nothing.
- Find: Edit → Find... (Ctrl+F) shows the find bar at the top right of the panes with the keyboard; if the selection is on one line, its text becomes the search text. The search runs in the pane that has the caret, so pressing the mouse button in a pane moves the search there, D included, and it is purely a view feature: the GUI searches the text it already mirrors, and nothing goes through the protocol. Matches are found per line, never across a line break, as plain text or, with the regular expression toggle, as a pattern (an invalid pattern shows its error in place of the count; empty matches are skipped). Toggles: match case (off by default, so the search ignores case), whole word, regular expression. Every match in the pane is highlighted, and the one the selection covers is stronger; the count reads `N of M`, `? of M` when the selection covers none, or `No results`. Typing selects the first match at or after the caret and scrolls to it; Enter, the down button and F3 (Edit → Find Next) select the next match, Shift+Enter, the up button and Shift+F3 (Edit → Find Previous) the previous one, both wrapping around. The selected match is the caret, so the pane can be copied from or edited at once. F3 and Shift+F3 work with the bar closed, continuing the last search, and open the bar when there is nothing to search for. Escape or the close button hides the bar and its highlights and keeps the text, options and selection.
- Void Region: a special graphical entity that spans the rows of a region where a document has no text, and it represents absence of text in the specified document. Graphically represented as hatched block of space with no text in it, hatched in the color of the pane's source document (in D, the color of the document whose empty hunk was accepted; neutral gray otherwise).
- Scrolling of the three panes is synchronized through row indices, not pixels. Void Regions take rows like text does, so synced scrolling stays aligned. Every source pane has the same rows at the same height, so the panes share one vertical scroll offset: the pane the user scrolls, or the one that reveals the caret, moves all of them. Horizontal scrolling stays per pane. D scrolls on its own unless void space in the output pane is on, which gives it the source panes' rows. Each scroll group's offset is kept by the panes code, not by the panes' scroll areas: a pane reports the vertical wheel scrolling over it and the group scrolls by it, so panes of a group never drift apart; dragging a pane's scroll bar or revealing the caret moves its group to where that pane went. A mouse wheel notch scrolls `scroll_step_lines` rows; with `smooth_scrolling` on, notches and jumps glide over `smooth_scroll_ms`, and notches that come quickly add up. Trackpad scrolling follows the fingers at once. Shift with the wheel scrolls sideways and Ctrl with the wheel zooms, as egui does.
- Source panes draw rows, not lines: each row shows its line or the hatching of a void, inside its region's box when the region differs. The caret and selection stay in line positions; a click on a void puts the caret on that void row, at the start of the next line. If the comparison does not fit a document, the pane falls back to one row per line and the mismatch is logged.
- The merge pane is a custom widget, not `TextEdit`: own cursor, selection, clipboard, IME, and region coloring.
- Merge menu: Choose A, B or C Here (Ctrl+1, Ctrl+2, Ctrl+3) toggles that hunk in the region of the row with the caret, in any pane: it is appended to the region's choice, or removed if it is already there; an unresolved or hand-edited region starts from no choice, and removing the last chosen hunk makes the region unresolved again. Each toggle is one undo step. Choose A, B or C for All Unresolved resolves every unresolved region at once. C is offered only in 3-way merges. While a typed edit is unconfirmed, choices wait, since its delta may join regions. D shows a placeholder row with a red `<Merge Conflict>` marker for each unresolved conflict and an empty region row for every other region that differs but has no lines; typing on an empty region row puts the typed text there as whole lines, and the region becomes edited (M). The status bar counts unresolved conflicts after open, reload and every choice.
- Navigation: Merge → Previous and Next Difference (Ctrl+Up, Ctrl+Down) and Previous and Next Conflict (Ctrl+PageUp, Ctrl+PageDown) ask the engine to move the current region from the region of the caret's row. Differences skip unchanged regions, unless D holds a hand edit of them; conflicts are the regions auto-resolve leaves unresolved, whether or not D resolves them by now. If every pane already shows all rows of the region, nothing scrolls; otherwise every scroll group scrolls so the region starts `jump_context_lines` rows below the top. A group whose shortest pane is too short for that keeps fewer rows above, splitting the rows the region leaves free above and below it, so the region is always in view; a region taller than the pane starts at its top. D, when it scrolls on its own, goes to the region in its own rows. The caret goes to the region's start in the pane that has it, or in A, but is hidden until it is moved again by a click, a caret key or an edit; choosing for the caret's region still works on the jumped-to region. With nowhere to go, the status bar says so. With `alignment_stops` on, Previous and Next Difference also stop at the first region of every applied alignment. The jumped-to region gets the `current_region_color` background in every pane, void space included.
- The GUI talks to the engine only through its protocol client and renders from the client mirror.
- Long work (load, diff, align) runs in the host; the GUI shows its progress events, keeps repainting and can cancel.
- Dialogs: open files, save-as, settings, find, go to line, line ending choice, "unsaved changes" confirm.
- Open dialog: shown at start and from File → Open. Each document has a path field and a Browse button that opens a native file dialog; D gets a save dialog, since it may not exist yet. Every file dialog starts in `last_dir`, which is updated whenever a file is confirmed. Run needs A and B; C makes a 3-way merge, and D names the output file (a merge without D keeps D in memory until it is saved). If D is an existing file, Run asks before overwriting it. D starts as the auto-resolved merge, whatever the output file holds.
- Corner radius: applied to widgets, menus, dialogs and panes. On Windows 11 the native window frame follows it too, through the DWM corner preference: 0 is square, 1 to 4 is small rounding, more is normal rounding.
- Theme: light by default; dark and following the system can be chosen. Both themes have a palette and are styled at startup and on every settings change, with the user's theme colors applied, so switching themes needs no other change. Panes are bordered boxes with a header strip; their text uses the palette's pane text color, which has at least 7:1 contrast with the pane background, and menus, dialogs and buttons use its UI text color. Both are black in the light theme.
- Whitespace: while rendering is on, each kind the settings enable gets its whitespace marker, drawn under the text in a faint color. A line's terminator is read from the text after the line, so a file with mixed line ends shows each line's own.
- Settings dialog: Settings → Configure Kdiff4... (Ctrl+,), a fixed-size window whose tab body scrolls when it doesn't fit, in three tabs, each a list of titled sections whose titles start with the name of what they change, such as "Panes: Text" or "Region boxes: Borders". Panes: font family, size, hinting and subpixel positioning; the font list shows only installed monospace families, found on a background thread, plus the built-in font; the pane text color with a preview; line numbers, gutters, void space in the output pane, the overview strip, whitespace markers (a master switch and one toggle per kind), lines above jumps, smooth scrolling with its duration and the wheel step, the jumped-to region's color, the alignment color and whether navigation stops at alignments,
 the corner radius and the opacity of the other regions' borders of region boxes, and the colors of A, B, C and of what differs from all documents (section "Documents: Colors", for the theme in effect, each with a button back to the theme's own color). Appearance, for the window around the panes: the theme, the UI text color, and the corner radius of widgets, dialogs, panes and the window frame. Colors are edited for the theme in effect, each with a button back to the theme's own color. Comparison: the extractor of Heuristics Alignment and, while files are open, the diff algorithm, minimal diff (Myers only), pairing similar changed lines and the character-diff line limit. View changes preview live; OK saves `gui.toml`, Cancel reverts. Comparison changes are sent on OK as one set-comparison-options command, which compares again and rebuilds D, dropping its choices, edits and undo history; they are not saved yet, since the engine config file doesn't exist yet. Aligning B with C is left out, as the built-in engine doesn't support it.
- Colors:
  - Red (removed chars)
  - Green (added chars)
  - Magenta (modified chars)
  - Amber (conflicts)
  - Implemented so far: void hatching and the border of region boxes, as fixed colors of the palette; D's neutral color, also fixed; the jumped-to region's background, from `current_region_color`, whose default keeps text at 7:1 contrast. The colors of A (blue by default), B (green), C (violet) and the red are the one source for everything drawn in them: pane headers, gutter blocks, char-diff text and backgrounds, the overview strip and the `<Merge Conflict>` marker. They are set per theme in `[view.light_colors]` and `[view.dark_colors]`. The `[colors]` section of `gui.toml` is not read yet.
- Character-level diff: drawn in every visible row of A, B and C that has one, in any region; D has none. The char-diff rules, one match in the GUI, map each char run to a style from its document and where its characters are the same. In A, B and C alike: characters that differ from every other document of the row are red; characters that differ from just one other document take that document's color (text changed only in B is B-colored in A and C, and red in B; a document with no line in the row always differs); characters equal in all documents are plain. A 2-way diff follows the same rules with C missing, so A's changes are in B's color and B's in A's, like kdiff3. Neighboring runs with the same style share one square background, which spans the full height of the row so that backgrounds of adjacent lines meet without a gap, and is drawn under the selection and over the jumped-to region's background. A red run has red text on the red at partial alpha; a run of a document keeps the pane's text color on the document's stronger fill (the color of its active gutter blocks). Gutter blocks use the same rules and colors (see Margin).

### Updates

- Source: `GET https://api.github.com/repos/xahon/kdiff4/releases/latest`, which lists published releases only (drafts and pre-releases are left out). The environment variable `KDIFF4_UPDATE_URL` replaces the address, to try the windows against a local server; it also allows a plain HTTP download address.
- A release is newer when its tag, without a leading `v`, is a greater semantic version than the running program's. The tag, notes (Markdown), page and files come from the release. The check is sent with a `kdiff4/<version>` user agent, a 10 second timeout and a 1 MiB limit on the answer.
- An automatic check runs on start when `check_automatically` is on and the last successful one is a day old or more (or none ran, or the clock went back). A failed check is retried at the next start. A manual check (Help → Check for Updates...) ignores the timer and the skipped version, and reports "no update" and failures in a window; an automatic one stays silent unless there is something to offer. Help → Check for Updates Automatically toggles the setting.
- The Update Available window names both versions and shows the notes. Update and Restart installs; Remind Me Later closes the window until the next check; Skip This Version stores the version in `skipped_version`, which silences automatic checks about that version only. When the release has no file for this platform, Open Release Page replaces Update and Restart. A warning says that unsaved edits are lost when D has any.
- Install: the release workflow publishes one executable per platform, named `kdiff4-<os>-<arch>` (`.exe` on Windows), with `os` `windows`, `macos` or `linux`, and `arch` `x86_64` or `aarch64`. The installer downloads over HTTPS to `<exe>.new` next to the running executable (the path it was started from), checks the size against what the server announced and the magic number of an executable for the platform, moves the running executable to `<exe>.old`, renames the download to the original path (putting the old one back if that fails) and starts the new executable with the arguments of the current run, then closes the window. A running Windows executable can be renamed but not deleted, so `<exe>.old` is removed at the next start. A folder that is not writable ends in an error window and nothing is changed.
- While the download runs, a modal window with a progress bar covers the program: shortcuts and pane keys are off, and only Cancel works. Canceling removes the partial file and leaves the program as it was.
- Restarting reopens the same files from the command line; an Open window started without arguments reopens empty. Edits to D that were not saved are lost, hence the warning.
- Not done: a checksum or signature check of the download beyond TLS to GitHub, and updating a copy of the program that was started from a read-only location.

### Releasing

The owner changes the version in `Cargo.toml` by hand. A `post-commit` hook in `.githooks` then creates the annotated tag `vX.Y.Z` on `master` for the commit that changed the workspace version (it never edits the version and never moves an existing tag). Enable it once per clone with `git config core.hooksPath .githooks`, and `sh .githooks/test-post-commit.sh` tests it. Push the commit and its tag together with `git push --follow-tags`. The tag must equal the version in `Cargo.toml`. `.github/workflows/release.yml` tests and builds on Windows, Linux and macOS (Apple silicon and Intel), names the executables as above and opens a draft release with generated notes. Edit the notes and publish the draft; the update check sees it from then on.

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

## Directory compare (v1.1, optional, not implemented)

Nothing of this is built. This section is a digest of how kdiff3 compares and merges folders, which is the behavioral spec, followed by the design for porting it and the decisions the owner has to make first. kdiff3 sources are in `../kdiff3/src`: `directorymergewindow.cpp` (tree model, operation proposals, execution, folder view), `MergeFileInfos.cpp` (per-entry data, equality, ages), `DirectoryInfo.cpp` (the three folders and the destination), `fileaccess.cpp` and `DefaultFileAccessJobHandler.cpp` (listing and name filters), `CvsIgnoreList.cpp`, `GitIgnoreList.cpp` and `CompositeIgnoreList.cpp` (ignore files), `pdiff.cpp` (entry points and view switching), and the chapter on folders in `../kdiff3/doc/en/index.docbook`, which is partly out of date. Where the docs and the code disagree, this section follows the code.

### How kdiff3 compares and merges folders

#### Modes and entry points

- Folder mode starts when A is a folder: from the command line, the Open dialog, or by selecting two or three folders in a folder view. B (and C) must be folders too; a folder next to a file is an error ("Can't compare file with folder") and the Open dialog comes back. The scan then checks that A, B and C exist and are folders, and names each one that is not. The destination is not checked until something is written there.
- Three modes. Two folders: a merge of A and B. Three folders: a merge in which A is the base of B and C. Sync mode: only with exactly two folders and no destination, and only when the option is on; its goal is that A and B are equal afterwards.
- The destination is the output folder when one is given (`-o`), else C when there is a C, else B. With three folders the destination must not be A or B. A destination that is one of the inputs means the merge happens in place.
- The folder view is a mode of the main window with an info pane under the tree. The user switches between folder view, text view and a split view, and the tree stays alive while files are opened from it. Opening an item (double click or Enter) compares its files, or merges them when the session has a destination. Items can also be picked explicitly by clicking their A, B or C cell, up to three, all files or all folders, and then compared or merged; a merge of explicit picks writes to the C pick, else to the B pick. Picked folders open a new folder comparison in a new window.
- With no destination, a freshly scanned comparison jumps into the first differing file by itself, which is what `git difftool --dir-diff` relies on. That mode pairs real files with symlinks into the work tree, so links to files are followed by default.
- A file in A with folders in B and C: kdiff3 looks for a file of the same name inside those folders and uses it (a file-mode feature, not a folder comparison).
- Closing a folder comparison exits with code 0 whether or not anything was saved. `--auto` is ignored for folders, with a message on stderr.

#### Scan

- Each input folder is listed on its own, recursively unless the recursive option is off. A folder's entries are listed first, then filtered, and only then are the subfolders that survived the filter listed, so ignored folders are never entered. Hidden entries are included unless the option is off (a leading dot on Unix, the hidden attribute on Windows). Special files such as devices, sockets and broken links are listed too. Progress is reported per input folder and the scan can be canceled. A folder that cannot be read gives one warning naming the sides, and the user may continue or cancel.
- Filtering at scan time, per listed folder: a file is dropped when it does not match the file pattern or matches the file anti-pattern; a folder is dropped when it matches the folder anti-pattern; anything is dropped when an ignore list matches its name. Patterns are lists of wildcards separated by semicolons, with `*`, `?` and bracket sets, matched against the bare name. Defaults: file pattern `*`, file anti-pattern `*.orig;*.o;*.obj;*.rej;*.bak`, folder anti-pattern `CVS;.deps;.svn;.hg;.git`. Dropped entries are never compared. Showing more after loosening a pattern needs a rescan; showing fewer takes effect at once, because the same patterns are applied again when the tree's visibility is refreshed.
- Links: a link to a folder is entered only with the follow-folder-links option, a link to a file is compared as its target only with the follow-file-links option, otherwise links are compared as links. Both options are on by default in the code (the docs say off). There is no cycle protection when folder links are followed. An entry counts as a normal file when it is a file, a folder or a link, and links are followed up to 15 levels to find out; anything else (devices, pipes, sockets, or links to them) is not normal and ends up as an error operation.
- Ignore files, with the "use ignore file" option (off by default), combine two lists. The CVS list holds the standard CVS names and wildcards (`core`, `tags`, `CVS`, `*.o`, `*.so`, `*~`, `*.bak`, `*.orig`, `*.rej`, `*.exe` and more, but no git names), the user's home ignore file, the `CVSIGNORE` environment variable and the folder's own `.cvsignore`; a lone `!` drops everything collected for that folder; its patterns apply to that folder only. The git list reads a folder's `.gitignore` and treats every non-comment line as a wildcard on the bare name: no negation, no anchoring, no directory-only patterns, and the patterns of a folder apply to every path that starts with the folder's path text, which includes subfolders.

#### Pairing and tree

- Entries of A, B and C are paired by relative path into one entry with up to three sides. Components compare case-sensitively or not by option (insensitive by default on Windows, sensitive elsewhere). An entry's name comes from the first side that exists, in the order A, B, C.
- The parent of an entry is the entry of its folder, which exists because folders are listed too. Children are sorted folders first, then by name ignoring case; the name column toggles the order, and only the name sorts.
- Columns: name; A, B and C (C only with three folders), each an icon whose color is the side's age and whose shape says file, folder, link or missing; operation; status; and four count columns (unsolved, solved, nonwhite, white) that exist only in full-analysis mode, the solved one only with three folders. Age colors default to green (newest), olive (middle), red (oldest) and black (missing).
- An info pane shows the folder paths ("A (Base)" with three folders, "(Dest)" for a side that is the destination) and, per side of the current entry, type (with "-Link"), size, rwx permissions, modification time and link target, or "not available".

#### Equality and ages

- Each entry has equality flags for A-B, A-C and B-C. An entry is equal when A-B is equal, and with three folders also A-C.
- Default (binary) comparison: different sizes are unequal at once; otherwise the files are read side by side in chunks of 100,000 bytes and compared, with progress and cancellation. The other modes are: trust size (same size is equal); trust size and date (same size and modification time is equal); the same but fall back to binary when the date differs. Without follow-file-links, a link against a link is equal when the target texts are equal and a link against a normal file is an error; a non-normal file against a normal one is an error as well. Read errors and these errors are collected and shown as a list afterwards; when 30 have piled up the scan stops, and between 15 and 29 only a generic message is shown. Folders are never read: a folder pair starts as equal when it exists on both sides, and a type mismatch (folder against file, link against non-link) clears the flag. A folder's final equality comes from its children, see below.
- Full analysis replaces the binary comparison: each pair is run through the diff and merge pipeline without showing it, which needs unsaved output saved first and hides the main window. It yields the four conflict counts and the binary-equality flags. With "white space differences considered equal" (default on) a pair whose differences are all whitespace counts as equal. It is much slower and is the only mode with the count columns. (A "binary comparison" option exists too, but it is only the radio button that is on when full analysis is off.)
- Ages: the sides that exist are ranked newest, middle, oldest by modification time, with equal sides sharing a rank; if no side ended up oldest, the middle ones become oldest. The ranks come from a map keyed by timestamp, so when two sides share a timestamp one of them gets no rank in the first pass; a fallback then ranks it and sets the conflicting-ages flag, which is how equal dates with different contents are detected. The flag turns into an error operation in copy-newer mode. Equal contents never set it. Folders get ranks by side order instead: C newest, then B, then A, equal sides sharing.
- A folder is equal when it exists on the same sides and all of its visible descendants are equal. Any visible unequal descendant marks all of its ancestors unequal, and this is recomputed from scratch on each visibility refresh, so it depends on which entries are shown (see below).

#### Visibility

- Five toggles: show identical files (the only one that is saved; the others are on at start), show different files, show files only in A, only in B, only in C. An entry is visible when it exists on every side and is equal and identical files are shown; or it exists on at least two sides, is not equal and different files are shown (folders count as different files whatever the toggle says, so disabling "different" never hides a folder); or it exists on only one side and that side's toggle is on. The name patterns are applied on top: a folder must not match the folder anti-pattern, a file must match the file pattern and not the file anti-pattern.
- Because folder equality counts only visible descendants, hiding the entries that differ makes their folders look identical. Kdiff3 does this.
- Also: fold all, unfold all, and an option to unfold all on load. After a scan a status report counts folders, equal files, different files and, with three folders, manual merges; it can be turned off for good.

#### Operations

- Two-folder and three-folder merge operations: copy A, B or C to the destination, delete from the destination, merge to the destination (a 2-way merge of A and B, or a 3-way merge with A as base; a missing side is passed as nothing, so a missing A makes a base-less merge). Sync mode operations: copy A to B, copy B to A, delete A, delete B, delete both, merge to A, merge to B, merge to both. No operation. Three error operations that block a run: conflicting file types, changed and deleted, conflicting ages.
- Proposal with two folders (merge mode): equal gives copy B to the destination, or nothing when the destination is A or B; both exist and differ gives merge, which for a folder only creates it in the destination; with copy-newer on, a file pair gives copy newer, and equal dates give the conflicting-ages error; only B gives copy B; only A gives copy A. Sync mode: equal gives nothing; differ gives merge to both (or copy newer); only A gives copy A to B; only B gives copy B to A.
- Proposal with three folders: all equal gives copy C, or nothing when the destination is an input (the usual case). All exist: A and B equal or B and C equal gives copy C; A and C equal gives copy B; otherwise merge. A and B exist without C: equal gives delete from the destination, else the changed-and-deleted error; likewise A and C without B. B and C exist without A: equal gives copy C, else merge. Only C gives copy C; only B gives copy B; only A gives delete from the destination. Whatever the above says, an entry whose types conflict (folder against file, link against non-link, a non-normal file) gets the conflicting-file-types error.
- Setting an operation on an entry recomputes its children from it. A copy operation becomes the children's default, so children missing in the copied side get a delete operation; a conflicting-types error gives its children the plain merge default; a merge operation gives the children their own automatic proposals; no operation and the delete operations pass down unchanged. Choosing A, B, C, auto or nothing for all items asks for confirmation and applies to every top-level entry, recursively. The operation column offers only valid choices (C only with three folders and when it exists, merge only when the types fit and at least two sides exist). Keys: Ctrl+1, Ctrl+2, Ctrl+3 choose A, B, C when that side exists; Ctrl+4 merges; Ctrl+Del deletes (in sync mode it deletes whichever of A and B exist); Ctrl+Space does nothing.

#### Execution

- F7 starts or continues the run over all visible entries, F6 runs the current entry and its children, Shift+F5 rescans (after a confirmation while a run is in progress). Starting is refused, with the entry scrolled into view and a message, while an unfinished entry has an error operation. The F7 that starts a run asks first: do it, simulate it, or cancel; F6 and a continuing F7 do not. A simulation goes through every step without touching the disk and lists what it would do in a verbose text window; real runs fill the same window and show it when something fails.
- Entries run in tree order, depth first. Each has a status: none, to do, in progress, done, skipped, not saved or error. A folder is done when all its children are. Done entries are not run again until their operation is changed. Changing an operation clears the status.
- Copy: an existing destination is removed first (renamed to `.orig` when backups are on), unless both are folders of the same kind. A link is copied as a link, unless following applies for its kind. A folder copy only creates the folder, since its children have operations of their own. Parent folders are created as needed. A file copy keeps the modification time and permissions. Delete: with backups on, the target is renamed to its name plus `.orig`, after removing an existing `.orig` without a further backup; otherwise the target is removed, folders recursively but never through a link; a missing target is success. Making a folder replaces a file in the way (always with a backup, whatever the option says) and creates missing parents. Backups default to on.
- Merge of a file: parent folders are created and the file merge opens with A, B, C and the destination path as output, and the run pauses with the entry in progress. When the user saves the output at that path, the entry is done, and the user presses F7 to go on. In sync mode, merge to both saves in B and then copies B over A; if that copy fails the entry is marked as an error and turns into copy B to A. If the output is not saved before leaving, the entry becomes "not saved". Merge of a folder just creates it. Outside a run, the current file can be merged on its own into the destination.
- The destination path of an entry is the destination folder plus the relative path; when the destination folder is B or C, the entry's own path on that side is used.
- On an error the run stops, the status text is shown and the entry is marked. Continuing asks whether to retry the entry or skip it.
- Kdiff3 shows a beta-status warning before a run. It has no working way to save or load a merge state, and no menu entries for sync mode or copy-newer: those actions are commented out, and the two options are settings only.

#### Options

Kdiff3 names, with defaults: `RecursiveDirs` on; `FilePattern`, `FileAntiPattern`, `DirAntiPattern` as above; `UseCvsIgnore` off; `FindHidden` on; `FollowFileLinks` on; `FollowDirLinks` on; `CaseSensitiveFilenameComparison` off on Windows, on elsewhere; `UnfoldSubdirs` off; `SkipDirStatus` off; comparison mode (`BinaryComparison` on, `FullAnalysis`, `TrustDate`, `TrustDateFallbackToBinary`, `TrustSize`, all off); `WhiteSpaceEqual` on; `SyncMode` off; `CopyNewer` off; `CreateBakFiles` on; `ShowIdenticalFiles` on; the four age colors.

### Port design

- Placement: the folder logic goes to `kdiff4-core` as modules for scanning and name filters, ignore lists, pairing and the tree, equality and ages, operation proposals, and execution; the messages go to `kdiff4-protocol`; the tree view goes to `kdiff4-gui`. The engine stays free of GUI crates, and the GUI holds no folder logic: which operations are valid for an entry, which entries are visible and what the status is all come from the engine.
- Sessions: the host gets a folder session next to the file session; they coexist, as kdiff3's folder tree outlives the files opened from it. Opening an item sends the existing open command for the file session and leaves the folder session alone. The folder session has its own revision and its own snapshot, delta and mirror, separate from the file session's. Both the scan and the comparison are cancelable host jobs that report progress.
- Size: trees can hold hundreds of thousands of entries, so the snapshot is a flat list with parent links, deltas are per entry (equality, operation, status, visibility) and the GUI flattens the unfolded part into a virtualized table, never one widget per entry. Visibility toggles can be applied on the host or in the mirror; the folder-equality rule above needs the host to own them.
- Manual merge handoff stays in the host: when a run reaches a merge entry, the host opens the file session with the destination as D, pauses the run, and resumes it on the command to continue once D has been saved at that path. The unsaved-output question before leaving a merge stays a GUI dialog.
- Dependencies: `std::fs` is enough for listing, links, times and permissions; Windows hidden attributes come from the standard Windows file extension traits without `unsafe`. A file copy has to set the modification time itself, since the standard copy keeps only the permissions. The wildcard matcher (`*`, `?`, bracket sets, semicolon lists) is small and goes into core. `walkdir` and `ignore`, planned in the Crates table, fit badly: kdiff3 filters a folder before it descends and has its own link rules, and `ignore` implements real gitignore semantics, which differ from kdiff3's, see the decisions below. Any dependency needs the owner's approval.
- Full analysis needs a headless entry in core that runs the built-in engine on a file pair without a session and returns the conflict counts and binary-equality flags. It can be left to a later slice.
- Config: a `[folder]` section in `config.toml` for the options that change results (recursive, the three patterns, ignore files, hidden, both link options, case sensitivity, comparison mode, white space equal, sync mode, copy newer), sharing the backup option with `[save]`. In `gui.toml`: show identical files, unfold on load, skip the status report, and the four age colors in `[colors]`.
- CLI and Open dialog: a folder as A starts folder mode, with `-o` naming the destination folder; mixing a file with a folder is an error as in kdiff3; Browse in the Open form must be able to pick folders. The two lines of `DEVIATIONS.md` that say directory compare is dropped and a directory argument is an error go away when this lands.
- Tests: kdiff3 has unit tests only for the ignore lists and file access, none for the tree, the proposals or the execution, so those expectations come from this section. Port the ignore-list cases (CVS: exact, prefix, suffix and wildcard patterns, the case flag, `!` reset, isolation between folders; git: comments, case, subfolder inheritance, isolation between folders; the composite list). Test the proposals as a table over which sides exist, which are equal, the mode and the options. Build fixture trees under `test_data/` for the scan, pairing and equality (with `insta` snapshots of the resulting tree), and check execution against real temporary folders, with properties such as: a simulated run changes nothing on disk; copying A to a new destination reproduces A's filtered tree; a run never touches a side that is not its destination.
- Suggested order: scan and filters with ignore lists; pairing, equality, ages and visibility; proposals; execution and simulation; protocol and host; the GUI view; CLI, Open dialog and config.

### kdiff3 quirks found

Per the rules in `AGENTS.md`, port these as they are and ask the owner before fixing any.

- Pattern case handling is inconsistent: the scan matches names case-insensitively on Windows and case-sensitively elsewhere whatever the case option says, while the visibility refresh follows the option; and the compiled-pattern cache is keyed by the pattern text alone, so the case mode of its first use sticks.
- Folder equality depends on which entries are visible (see Visibility).
- The `.gitignore` support is much smaller than git's (see Scan), and a folder's patterns also reach a sibling whose path merely starts with the same text.
- Following folder links has no cycle protection, and the docs promise a crash for a link into its own folder.
- Reloading drops which folders were unfolded and every chosen operation (the code to keep them is commented out).

### Decisions for the owner

Decided on 2026-11-06 (the work plan is `FOLDER_COMPARE_PLAN.md`):

- Scan on `std::fs` and a small matcher, not `walkdir` and `ignore`.
- The first version has ignore files, sync mode, copy-newer and full analysis, besides the plain merge modes.
- Delivery on one branch.

Still open; until the owner says otherwise the plan ports kdiff3 as it is, with one worker thread per job:

- kdiff3's ignore semantics, or real gitignore semantics as a documented deviation.
- Which of the quirks above to fix.
- Whether the binary comparison may run on several threads, which needs a dependency or hand-rolled threading.

## Out of scope

Explorer/Dolphin/KIO integration, translations, word wrap, preprocessor commands, auto-merge of VCS history/keyword blocks.
