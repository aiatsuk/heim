# Contributing to heim

Thanks for helping. Keep changes small and focused.

## Setup

```bash
# Optional: dust for --size-backend dust
# brew install dust   # or: cargo install du-dust

cargo test
cargo run --release -- --once --json .
```

No external LOC tool is required (tokei is linked in-process).

## Workflow

1. Open an issue for larger ideas when possible
2. Branch from `main`
3. Prefer focused commits (one concern each)
4. Run before opening a PR:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
./target/release/heim --once --json .
```

## Style

- Match existing module layout (`app`, `collect`, `fmt`, `report`, `store`, `theme`, `ui`)
- Keep the UI thread non-blocking; heavy work stays in the collector worker
- Prefer pure helpers in `fmt` / `collect` with unit tests
- No secrets, personal paths, or private process notes in the tree

## Scope

Good first contributions:

- Linux / Windows smoke notes
- Live demo GIF (`docs/demo.tape` + VHS, or asciinema)
- Clippy cleanups, dead-code removal, docs
- Better ignores or size-backend edge cases
- Agent skill improvements (`skills/heim-audit`)

Please avoid drive-by dependency upgrades unless they fix a bug.

## Releases

1. Bump version in `Cargo.toml` + `CHANGELOG.md`
2. Tag `vX.Y.Z` and push — [Release workflow](.github/workflows/release.yml) builds multi-target archives
3. `cargo publish` for `heim-monitor` (binary remains `heim`)
4. Optionally refresh `dist/homebrew/heim.rb` checksums in a homebrew tap
