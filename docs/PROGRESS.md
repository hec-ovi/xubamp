# xubamp progress

Resume point after any context compaction: read this file, then
[ARCHITECTURE.md](ARCHITECTURE.md), then `git log --oneline`. Everything durable is
in the repo and git history, nothing important lives only in chat.

## Done

- Phase 0: Cargo workspace scaffold, lean release profile, GPL-2.0 license, README,
  architecture and plan doc.
- Phase 0: `crates/skin` BMP decoder (1/4/8/24/32-bit, top-down and bottom-up) with 5
  unit tests, clippy clean.

## In progress

- Phase 1: `.wsz` container reader (zip, case-insensitive lookup, default-skin
  fallback) and the config parsers (region.txt, pledit.txt, viscolor.txt).

## Next

- Phase 2: native Wayland window showing a static render of a skin, verified by a
  headless offscreen render diff.

## Working rules

- Commit and push per green sub-unit: `cargo build` + `cargo test` + `cargo clippy`
  all pass first. Small and frequent.
- Each phase ends on a runnable, testable artifact; that push is a safe point to
  compact context.
- No CI and no GitHub Actions. Tests run locally via `cargo test`.
