# Working in this repo

Transpo-Sim: a Texas transportation-network game. Rust workspace (`crates/*`) plus a Tauri app (`app/`, TypeScript client; `app/src-tauri` is its own Cargo workspace).

Read before changing code:

1. [docs/README.md](docs/README.md): index of every doc.
2. [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): components, interfaces, and the simulation invariants you must not break (flows down, corrections up; the camera never changes the score; seeded determinism; one clock).
3. The README of the component you are changing.
4. [docs/ROADMAP.md](docs/ROADMAP.md): find the feature ID (F##) your work delivers.

Rules:

- **Docs ship with the code.** Follow [docs/DOCS_STANDARD.md](docs/DOCS_STANDARD.md). Update the component README, doc comments, ROADMAP status, and DECISIONS.md in the same PR as the change.
- Decision IDs are per stream: `SIM-`, `DATA-`, `APP-`, `DOC-`. Append to your stream's section of docs/DECISIONS.md, never to the frozen `D1`–`D31` table.
- Do not add scope that is not in the roadmap or design doc without recording the decision. No hyperloop, no multiplayer.
- sim-core stays headless: no app, UI or pipeline dependencies inside it.
- `ScenarioInput` (`crates/sim-core/src/schema.rs`) is the contract between pipeline and core. Change it only with both READMEs and the fixture updated.
- Before opening a PR, run the checks for what you touched:
  - Rust crates: `cargo fmt --all --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo doc --workspace --no-deps` (CI runs the same in `.github/workflows/rust.yml`, warnings as errors)
  - App: `cd app && npm ci && npm run build && npm test && (cd src-tauri && cargo check)` (not covered by the root workspace)
