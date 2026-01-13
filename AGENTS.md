# Repository Guidelines

## Project Structure & Module Organization
- `src/main.rs` hosts the egui desktop UI and entrypoint; `src/flasher.rs` handles probe/serial flashing logic; `src/bin/build_dist.rs` packages the Windows distribution.
- `assets/` holds UI resources bundled into builds. `dist/` is generated release output (e.g., `dist/PDES`).
- Installer scripts live in `scripts/windows/*.ps1` and `scripts/linux/pdes-linux-installer.sh`; Windows helper tools ship from `tools/windows/`.
- Builds target nightly Rust (see `rust-toolchain.toml`) and may use unstable features.

## Build, Test, and Development Commands
- `rustup run nightly cargo fmt` — format with rustfmt.
- `rustup run nightly cargo clippy --all-targets --all-features -- -D warnings` — lint and fail on warnings.
- `rustup run nightly cargo test --all-targets` — run unit/integration tests (add them as you go; none are present yet).
- `rustup run nightly cargo run` — launch the app for local development; ensure assets are present.
- `rustup run nightly cargo build --release --target x86_64-pc-windows-gnu` — produce the Windows executable referenced by installers.
- `rustup run nightly cargo run --bin build_dist --release` — assemble the distributable in `dist/PDES` after a release build.

## Coding Style & Naming Conventions
- Follow rustfmt defaults and keep Clippy clean. Prefer `anyhow::Result`/`Context` for error flows over bare `unwrap` in UI or flashing paths.
- Modules and files use `snake_case`; types and traits use `CamelCase`; constants use `SCREAMING_SNAKE_CASE`.
- Keep UI labels short and actionable; log user-visible operations to the console window for traceability.

## Testing Guidelines
- Add focused unit tests in `#[cfg(test)]` modules beside logic, and integration tests under `tests/` for end-to-end flashing pipelines.
- Prefer deterministic tests (mock probe/serial interactions rather than requiring hardware). Run the suite with `cargo test` before raising PRs.

## Commit & Pull Request Guidelines
- Use concise, imperative commit messages (recent history: "Using same method as arduino ide", "Adding modified version of bossac"). Group related changes per commit.
- PRs should describe intent, list platforms validated (Windows/Linux), link issues, and include screenshots of UI changes when relevant.
- Before requesting review: run `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test`. Note any failing or skipped checks explicitly.

## Distribution & Installers
- After building for Windows, rerun `build_dist` to refresh `dist/PDES` and verify installer scripts still reference the correct paths.
- If you modify installer scripts or bundled tools, update both the script copies in `scripts/windows/` and the packaged copies produced in `dist/`.
