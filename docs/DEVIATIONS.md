# Deviations from kdiff3

Reference: kdiff3 at `../kdiff3`, HEAD `33e56f0c`, v1.14.70. Anything not listed here should behave like kdiff3.

## Fixed kdiff3 bugs

None yet. The owner decides which kdiff3 bugs get fixed. When you find one while porting, port kdiff3's behavior, report the bug to the owner, and leave it unfixed until they approve. When a fix is approved, add a row here in the same change as the fix:

| # | Area | kdiff3 behavior | Kdiff4 behavior | Reference | Test |
|---|---|---|---|---|---|

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

### Configuration

- Settings are stored in `config.toml` and `gui.toml`, not `kdiff3rc`. Import of `kdiff3rc` is planned for v1.1.
