# Folder diff and merge: implementation plan

Status: planned, not started. Written 2026-11-06 and meant to be picked up later. The behavioral spec and the port design are in the "Directory compare" section of `ARCHITECTURE.md`; this file is the work plan on top of them. Delete it when the feature has landed.

## Decisions made

- Scan and name filters on `std::fs` plus a small wildcard matcher. No new dependencies; the `walkdir` and `ignore` row leaves the Crates table.
- First version: plain 2-folder and 3-folder compare and merge, plus ignore files (the CVS and git lists, as kdiff3 does them), sync mode, copy newer, and full analysis.
- Every setting of kdiff3's Configure -> Folder page is copied. A setting that cannot work yet gets a code comment and an entry in the docs.
- One branch for everything, committed area by area, one squash merge at the end. Never pushed.
- kdiff3 quirks (pattern case handling, folder equality depending on visibility, the partial `.gitignore`, no link-cycle guard, reload dropping unfolded state and operations) are ported as they are and reported, not fixed. No deviation is approved, so `DEVIATIONS.md` only loses its two lines about directory compare being dropped.
- Single-threaded comparison: one worker thread per job, no parallel binary compare.

## What exists today

- The host in `crates/kdiff4-core/src/host.rs` is a blocking loop over one optional session. There are no worker threads, no cancel command and no progress events yet, and the host endpoint cannot be cloned.
- The protocol has the open command, the snapshot, delta and current-region events, and the protocol error. Core already depends on `toml` and `dirs`, but no engine config file exists yet.
- The GUI has one mirror, the Open form, and a Settings dialog with the Panes, Appearance and Comparison tabs. `egui_extras`, `rfd` and `egui_kittest` are available. Tests make temp folders by hand under the system temp folder.
- File saves make no `.orig` backup, so the shared backup setting will only work for folder operations at first.

## Design

**Opening.** The open command stays the single entry. The host decides: A is a folder means folder mode, with B (and C) as folders and the output as the destination folder; a file next to a folder is the error "Can't compare file with folder". The front end never checks for folders itself, so this also works over a remote transport. Folder mode answers with a folder snapshot instead of a file snapshot, and both sessions can exist in the host at once.

**Host jobs.** The host endpoint gets a cloneable send handle and a receive with timeout. Scan, comparison and runs execute on a worker thread with a cancel flag and send progress events. While a job runs the loop handles only cancel at once and queues the other requests in order. New commands: cancel, set folder options (with persist), rescan, set the operation of one entry or of all top-level entries, set visibility, run (real or simulated; all visible entries, or one entry and its children), continue (retry or skip after an error, or resume after a manual merge), and open entry (compare or merge the files of an entry in the file session). New events: folder options (sent at start and after every change, so the Settings dialog works before anything is open), folder snapshot, per-entry folder delta, progress and run log.

**Manual merge handoff** stays inside the host. A merge entry opens the file session with the entry's destination path as D and pauses the run. A successful save of D at that path marks the entry done. Replacing the file session first marks it "not saved". In sync mode, merge to both saves to B, then copies B over A.

**Folder model (core).** A flat entry list in tree order with a parent index. Each entry has up to three sides (kind, size, modification time, permissions, link target), equality flags, ranks, an operation, a status and the set of valid operations. The host computes visibility and folder equality, because they depend on each other. The GUI keeps only view state (fold state, selection, scroll) and flattens the unfolded visible part into a virtualized table, never one widget per entry.

**Config.** A new engine config module in core reads `config.toml` from the `kdiff4` folder of the user's config directory. It has a `[folder]` section (recursive, file_pattern, file_anti_pattern, dir_anti_pattern, use_ignore_files, find_hidden, follow_file_links, follow_dir_links, case_sensitive_names, comparison, white_space_equal, sync_mode, copy_newer) and a `[save]` section with create_bak_files. Missing keys take defaults, unknown keys are logged and dropped, writes are atomic, and a broken file is renamed to `.bak` before saving, as for `gui.toml`. The host takes the file path as an argument so tests never touch the real config. `gui.toml` gets a `[folder]` section: show_identical_files, unfold_subdirs, skip_dir_status. The age colors stay fixed palette colors in `theme.rs` until `[colors]` is read, as the document colors are today. `--config`, `--cs` and `--confighelp` stay out of this change.

## Settings of Configure -> Folder

| kdiff3 setting | Key | Owner | Works |
|---|---|---|---|
| RecursiveDirs | folder.recursive | engine | yes |
| FilePattern, FileAntiPattern, DirAntiPattern | folder.file_pattern, file_anti_pattern, dir_anti_pattern | engine | yes |
| UseCvsIgnore | folder.use_ignore_files | engine | yes |
| FindHidden | folder.find_hidden | engine | yes |
| FollowFileLinks, FollowDirLinks | folder.follow_file_links, follow_dir_links | engine | yes |
| CaseSensitiveFilenameComparison | folder.case_sensitive_names | engine | yes |
| UnfoldSubdirs | folder.unfold_subdirs | GUI | yes |
| SkipDirStatus | folder.skip_dir_status | GUI | yes, through the status report dialog |
| Binary comparison, Full analysis, Trust date, Trust date with binary fallback, Trust size | folder.comparison | engine | yes |
| SyncMode, CopyNewer | folder.sync_mode, folder.copy_newer | engine | yes |
| WhiteSpaceEqual | folder.white_space_equal | engine | yes, only with full analysis |
| CreateBakFiles | save.create_bak_files | engine | folder operations only; file saves make no `.orig` backup yet (code comment at the key plus an entry in the docs) |

If anything else turns out not to work, it gets the same treatment. The Settings dialog gets a "Folder" tab with all of these, kdiff3's tooltips and the same enable rule (white space equal only with full analysis).

## Work areas

One commit each, in this order.

1. **Wildcards and ignore lists** (core): semicolon lists with `*`, `?`, bracket sets and a case flag; the CVS list (standard names, home file, `CVSIGNORE`, `.cvsignore`, a lone `!`); kdiff3's simplified git list; the composite list. Port the cases of the kdiff3 ignore list tests in `../kdiff3/src/autotests/`.
2. **Folder options and engine config** (core, protocol vocabulary).
3. **Scan and tree** (core): listing, hidden entries, link rules, filters applied before descending, pairing by relative path with the case option, the name from the first existing side, sorting (folders first, then name ignoring case), cancel and progress, and the read-error list with the 15 and 30 limits.
4. **Equality, ages and visibility** (core): the chunked binary compare (100,000 bytes), the trust modes, the link and non-normal rules, ranks and the conflicting-ages flag, folder equality from visible descendants, the five toggles, and the counts for the status report.
5. **Operations** (core): proposals for merge, 3-folder and sync modes and copy newer, as a table; recursion when an operation is set; the three error operations; valid choices per entry; the "all items" choices.
6. **Execution** (core): copy (sets the modification time itself; a link is copied as a link unless followed), delete and make-folder with the `.orig` rules, parent creation, statuses, simulation with a log, stop on error with retry or skip, and the scope of F6.
7. **Full analysis** (core): a headless entry that runs the built-in engine on a file pair, reusing the open pipeline without a session, and returns the four conflict counts and the whitespace-equal flag.
8. **Protocol and host**: the messages above, the send handle, receive with timeout, the job runner, the folder session, the handoff, and the options event with persistence.
9. **GUI**: a folder mirror and a folder view (a virtualized table with the name; A, B and C with age colors and kind icons; the operation; the status; the count columns in full analysis; an info pane; an operation menu limited to valid choices), a switch between folder and text view, menus and keys (F7, F6, Shift+F5, Ctrl+1, Ctrl+2, Ctrl+3, Ctrl+4, Ctrl+Del, Ctrl+Space, fold and unfold all, the visibility toggles, Enter or double click opens an entry), the run, simulate and continue dialogs with the log window, the status report dialog, Browse for folders in the Open form, and the Settings "Folder" tab.
10. **Docs**: turn the "Directory compare" section from spec into implemented behavior (keep the kdiff3 digest), and update Modules, Crates, Protocol, Config, GUI, CLI and the glossary. Add a "Folder settings status" list for settings that do not work yet. In `DEVIATIONS.md` delete the two lines about directory compare being dropped. The CLI help text says files or folders.

Reuse: the open pipeline of the file session, the source text loader, the comparison engine registry (for analysis), the load, save and backup pattern of the GUI config for the engine config, the Open form, the Settings dialog, and the pending-request and status pattern of the app.

## Tests

- Unit tests per module: wildcards, both ignore lists, options and config round trip with a broken file, scan filters and links (link tests skip when the OS refuses to create links), pairing, equality modes, ages, visibility and folder equality, the proposal table (which sides exist, which are equal, the mode, the options), recursion, and execution on real temp folders.
- Properties with `proptest`: a simulated run changes nothing on disk; copying A to a new destination reproduces A's filtered tree; a run never touches a side that is not its destination.
- Fixture trees are built in temp folders from files under `test_data/folders/` (git cannot hold empty folders), with `insta` snapshots of the resulting tree.
- Host tests through the protocol client: open, cancel, set operation, run, the manual merge handoff and options persistence, like `crates/kdiff4-core/tests/host.rs`.
- GUI tests with `egui_kittest`: rows follow fold state, the operation menu offers only valid choices, keys, and the Settings Folder tab.

## Verification

`cargo build` with no new warnings and `cargo test`. Then a manual run on two small folder trees and on a 3-folder merge with `-o`: scan, toggles, change an operation, simulate, real run, a manual merge of one file, and Shift+F5 rescan. After finishing, ask whether to launch the app for testing.

## Notes

- This is a large change, several thousand lines. Commits per area keep it reviewable; the single branch ends in one squash merge.
- Name the session `Core: Folder diff and merge`.
