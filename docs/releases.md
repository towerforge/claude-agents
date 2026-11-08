# Packaging and releases

How the archives that [`install.sh`](../install.sh) downloads are built and
published. Everything goes through the `Makefile` (`make help` lists it all).

## Archives

Each archive holds just the `claude-agents` binary, and its name carries no
version, so `releases/latest/download/<archive>` always points to the latest
one. Every release attaches these six, plus `checksums.txt`:

| Archive | Platform |
|---|---|
| `claude-agents-linux-x86_64.tar.gz` | Linux x86_64, glibc |
| `claude-agents-linux-x86_64-musl.tar.gz` | Linux x86_64, static (musl) |
| `claude-agents-linux-aarch64.tar.gz` | Linux ARM64, glibc |
| `claude-agents-linux-aarch64-musl.tar.gz` | Linux ARM64, static (musl) |
| `claude-agents-macos-x86_64.tar.gz` | macOS Intel |
| `claude-agents-macos-aarch64.tar.gz` | macOS Apple Silicon |

There is no Windows build: open sessions are read with `ps`.

## Building locally

```
make package                          # host: dist/claude-agents-<platform>-<arch>.tar.gz
make package CARGO_TARGET=aarch64-unknown-linux-musl CARGO_CMD=cross
make package-all-linux                # the 4 Linux targets, gnu and musl (cross + Docker)
make package-all-macos                # both macOS architectures (needs a Mac)
make package-all                      # all of the above
make checksums                        # dist/checksums.txt (SHA-256 of every dist/*.tar.gz)
make rust-targets                     # lists the triples package-all builds
```

Linux targets build with [`cross`](https://github.com/cross-rs/cross), which
needs Docker running. Install it once with
`cargo install cross --git https://github.com/cross-rs/cross`. The variables
`CARGO_TARGET`, `CARGO_CMD` (`cargo` or `cross`), `DIST_DIR` (default `dist`),
`LINUX_TARGETS` and `MACOS_TARGETS` change what gets built and where.

## Releasing

Releases follow `beta → main`. `make release` from `beta` asks for the new
version (patch, minor, major or typed), writes it to `VERSION`, `Cargo.toml`
and `Cargo.lock`, commits it on `beta`, merges `beta` into `main`, tags
`vX.Y.Z` and pushes; from `main` it tags the current version and pushes; from
any other branch it refuses. `make version` shows the version and flags drift
between `VERSION` and `Cargo.toml`.

Nothing is built or attached by hand: the pushed tag triggers
[`.github/workflows/release.yml`](../.github/workflows/release.yml), which builds
the six targets (Linux with `cross`, macOS on a macOS runner), checks that the
tag is `v$(cat VERSION)`, runs `make checksums` and publishes the GitHub
release with the archives and `checksums.txt`.

## CI

[`.github/workflows/ci.yml`](../.github/workflows/ci.yml) runs `cargo fmt
--check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` on
Ubuntu and macOS for every push to `main`, every `v*` tag and every pull
request.
