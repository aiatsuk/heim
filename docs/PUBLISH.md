# Publish checklist (0.1.0+)

Use this after product-market polish is on `main`.

## 1. Preflight

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
./target/release/heim --once --json . | head
```

- [ ] README pitch matches: *Stop vibe-code bloat…*
- [ ] CHANGELOG version section ready
- [ ] `Cargo.toml` version bumped

## 2. GitHub Release (binaries)

```bash
git tag v0.1.0
git push origin main --tags
```

[`.github/workflows/release.yml`](../.github/workflows/release.yml) builds and attaches:

- `heim-*-aarch64-apple-darwin.tar.gz`
- `heim-*-x86_64-apple-darwin.tar.gz`
- `heim-*-x86_64-unknown-linux-gnu.tar.gz`
- `heim-*-aarch64-unknown-linux-gnu.tar.gz`
- `heim-*-x86_64-pc-windows-msvc.zip`

## 3. crates.io

Package name is **`heim-monitor`** (binary **`heim`**). Requires a [crates.io](https://crates.io) API token once:

```bash
cargo login
cargo publish
# dry-run first:
cargo publish --dry-run
```

Install story:

```bash
cargo install heim-monitor --locked
heim -V
```

## 4. Homebrew (optional)

1. Fill sha256 in [`dist/homebrew/heim.rb`](../dist/homebrew/heim.rb) from release assets  
2. Publish in your tap as `Formula/heim.rb`  
3. `brew install <you>/tap/heim`

## 5. Demo GIF

Already in tree. Regenerate after UI changes:

```bash
python3 scripts/gen_demo_gif.py   # docs/assets/demo.gif
# optional real terminal capture:
# brew install vhs && vhs docs/demo.tape
```

## 6. Soft feedback → launch

1. Fill contacts in [soft-feedback.md](soft-feedback.md); message 5 agent power-users  
2. When ≥3 real sessions land, post using [launch-post.md](launch-post.md)  
3. After install works, open awesome-list PRs from [awesome-list-prs.md](awesome-list-prs.md)
