# Deviations from kdiff3

Reference: kdiff3 at `../kdiff3`, HEAD `33e56f0c`, v1.14.70. Anything not listed here should behave like kdiff3.

## Fixed kdiff3 bugs

The owner decides which kdiff3 bugs get fixed. When you find one while porting, port kdiff3's behavior, report the bug to the owner, and leave it unfixed until they approve. When a fix is approved, add a row here in the same change as the fix:

| # | Area | kdiff3 behavior | Kdiff4 behavior | Reference | Test |
|---|---|---|---|---|---|
| 1 | Character diff | A short match (fewer than 4 characters) between two changes is merged into the change after it only in the first step of the line diff, so a replaced word is marked in fragments (`quantity` against `count + tax` as `q[u]a[nt]i[t]y`) | Every short match is merged except the first step's, and that one only when another match is long, like kdiff3 0.9.98, so a replaced word is marked whole | `diff.cpp`, `DiffList::optimize`: commit `92b1326e` turned `dli == begin()` into `index > 0` | `fine_diff::tests::short_matches_inside_a_change_are_merged_into_it` |
| 2 | Character diff | A change can start with a character both lines have at the same place, because the match search pulls it back into the change: the space after the `=` of `let total = cost * quantity;` and `let total = price * count + tax;` is marked in both | When the lines have a long match, a change starts at its first differing character, so that space is not marked | `diff.cpp`, `DiffList::calcDiff`, the step that goes back over equal characters | `fine_diff::tests::a_change_does_not_start_with_the_space_both_lines_have_there` |

- Reference: kdiff3 file and function, or bug report URL.
- Test: name of the regression test that pins the fixed behavior.

## Intentional differences

### Dropped features

- Explorer/Dolphin/KIO integration.
- Translations (English only).
- Word wrap.
- Preprocessor and line-matching preprocessor commands.
- Auto-merge of VCS history and keyword blocks.
- Directory compare and merge: deferred to v1.1 (optional).

### Added features

- Splitting a region into smaller regions (see Splitting regions in `ARCHITECTURE.md`). kdiff3 cannot split its merge blocks.
- Session files (`.kd4`, see Session file in `ARCHITECTURE.md`): a partially resolved merge can be saved with its source documents and continued later or on another computer. kdiff3 has no session files.
- Pluggable comparison engines (see Comparison engine in `ARCHITECTURE.md`). kdiff3 has one built-in algorithm.
- Void space in the output pane: D can line up with A, B and C, padded with void rows, and scroll with them. kdiff3's merge pane always has just its own lines.
- Language selector in the status bar (see GUI in `ARCHITECTURE.md`): the language of the documents is detected from their paths and first lines and can be picked from a list, each with its logo. kdiff3 has no notion of a document's language. For now it is a label only; nothing is highlighted.

### Merge result

- A hand edit that crosses region boundaries joins the touched regions into one edited region; undo restores them (see Merge model in `ARCHITECTURE.md`).
- A hand edit inside a region whose hunks are all equal splits each touched line into an edited region of its own, boxed with a gutter block in every pane and reached by Previous and Next Difference; choosing A, B or C for it joins it back with the regions around it (see Merge model in `ARCHITECTURE.md`). kdiff3 keeps the edited lines inside the unchanged block, which navigation skips.
- Toggling off the last chosen hunk of a region (Ctrl+1, Ctrl+2, Ctrl+3) makes it an unresolved conflict again, so D can't be saved until it is resolved. kdiff3 keeps such a region with no lines, shown as "<No src line>", which counts as resolved.
- A region that differs but has no lines in D is shown as a hatched void row, without kdiff3's "<No src line>" text, so D has a region box wherever the source panes have one. Typing on it gives the region the typed lines and marks it hand-edited, as typing over "<No src line>" does in kdiff3.
- Typing on an unresolved conflict's placeholder line is not possible yet; kdiff3 lets the user type over its "<Merge Conflict>" line. Choose a hunk first, then edit it.

### Manual alignment

kdiff3's "Add Manual Diff Alignment" is reworked (see Manual alignment in `ARCHITECTURE.md`):

- kdiff3 builds an alignment one window at a time and may leave a window out. Kdiff4 requires a part in every source document: a line range or an anchor.
- kdiff3 has no anchors; content that is missing in one document cannot be pinned to a position there.
- kdiff3 silently removes an older range in a window when a new range overlaps it. It also keeps each window's ranges sorted on their own and pairs the n-th range of every window, so an alignment that would cross others is silently re-paired. Kdiff4 rejects overlapping and crossing alignments with an error message and leaves the existing ones unchanged.

### CLI

- `-u`, `--query`, `--html` and `--abort` are accepted and ignored, like kdiff3's default. kdiff3 lets users extend this list with the `IgnorableCmdLineOptions` setting; Kdiff4 has no such setting, and any other unknown option is a usage error.
- KDE and Qt framework options (`--author`, `--license`, `--desktopfile`, `-style`, `-platform` and similar) are not supported.
- `--cs` takes Kdiff4 keys (`section.key=value`, see Config in `ARCHITECTURE.md`), not kdiff3 setting names. Accepting kdiff3 names is planned for v1.1.
- Until directory compare lands in v1.1, a directory argument is an error (exit code 1) instead of opening directory compare.

### Fonts

- kdiff3 lets the user pick any installed font, style and weight. Kdiff4 offers only monospace families, in their regular face.
- Font hinting and subpixel glyph positioning can be switched off; kdiff3 has no such options.

### Navigation

- A jump scrolls only when some pane doesn't show all rows of the region yet, and then leaves 12 lines above it by default, fewer in a pane too short to show the region below them. The caret moves to the region's start but stays hidden until it is moved again. kdiff3 always scrolls to the difference and shows its cursor there.
- Clicking a gutter block makes its region current and scrolls every pane there like a jump, from the source panes and from D alike.
- Wheel scrolling and jumps can be smooth, with a configurable duration, and the rows a wheel notch scrolls are configurable. kdiff3 scrolls at once, by the system's wheel step.


### Appearance

- Whitespace markers have one toggle per kind (spaces, tabs, line feeds, carriage returns, other Unicode spaces) under a master switch, and line ends are marked per line from the line's own terminator. kdiff3 has two switches, for whitespace in the text and in the differences.
- The theme can be light, dark or follow the system, and the UI text and pane text colors can be overridden per theme. Light text is black by default.

### Open dialog

- Run needs A and B; C and D are optional. A session with C or D is a merge. kdiff3 shows A, B and C without an output as a 3-way diff unless Merge is checked; Kdiff4 makes it a merge whose output stays in memory until it is saved.
- File dialogs remember the directory of the last confirmed file between sessions.

### Configuration

- Settings are stored in `config.toml` and `gui.toml`, not `kdiff3rc`. Import of `kdiff3rc` is planned for v1.1.
