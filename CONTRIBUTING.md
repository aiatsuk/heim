# Contributing to heim

Thanks for helping. Keep changes small and focused.

## Setup

```bash
brew install cloc dust   # or equivalent on your OS
cargo test
cargo run --release -- --once .
```

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
./target/release/heim --once .
```

## Style

- Match existing module layout (`app`, `collect`, `fmt`, `store`, `theme`, `ui`)
- Keep the UI thread non-blocking; heavy work stays in the collector worker
- Prefer pure helpers in `fmt` / `collect` with unit tests
- No secrets, personal paths, or private process notes in the tree

## Scope

Good first contributions:

- Linux / Windows smoke notes
- README screenshot or short GIF
- Clippy cleanups, dead-code removal, docs
- Better ignores or size-backend edge cases

Please avoid drive-by dependency upgrades unless they fix a bug.
