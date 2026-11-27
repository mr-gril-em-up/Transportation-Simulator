# Documentation standard

Documentation is part of the work, not a follow-up. A PR that changes behavior without updating the docs that describe it is not done. The test for every doc: a developer (human or agent) who has never seen this code can tell **what was built, why, how to use it, and how to check it works**, without asking anyone.

This standard applies to every crate, the app, and any module large enough to have its own interface.

## 1. Every component has a README

Each crate under `crates/` and the `app/` directory has a `README.md` at its root with these sections, in this order. Omit a section only if it truly does not apply, and say so in one line.

| Section | Must answer | Example of specific vs vague |
|---|---|---|
| **Summary** (first paragraph, no heading) | What this component does and which roadmap features it delivers | "Builds zones, LODES flows and the road graph for Travis County (F02)", not "Handles data" |
| **Run** | Exact commands to build and run it, from a fresh clone | A fenced block that can be pasted into a shell |
| **Test** | Exact command to run its tests, what they cover, and any data or fixtures they need | `cargo test -p sim-core`; "fixture: `tests/fixtures/tiny.json`" |
| **Interfaces** | Public API, CLI flags, input and output files, messages. Types, units, coordinate systems, id rules | "Coordinates WGS84 degrees, money 2024 USD, ids `u32`" |
| **Design and rationale** | Why it is built this way; what alternatives were rejected and why; which design-doc rules it enforces | "Lanes are per direction because capacity is per direction" |
| **Known gaps** | What is missing, wrong, approximate or temporary, and which roadmap item fixes it | "JSON is interim; binary format lands with F33" |
| **Status** | Which roadmap features are done, partial or not started in this component | "F02 done; F04 not started" |

Keep the README as the entry point. If it grows past roughly 400 lines, move detail into `docs/` inside the component and link it.

## 2. Code-level docs

- Every public item (`pub fn`, `pub struct`, `pub enum`, fields of public structs, Tauri commands, TS exports) has a doc comment stating what it is, its units, and its invariants. `cargo doc` must build without warnings.
- Every module (`lib.rs`, each `mod`) opens with a `//!` comment: what the module owns and what it must not do.
- Comments explain **why**, not what. Write one wherever a reader would otherwise ask "why not the obvious way?" Link the design doc section or decision ID (`SIM-3`, or a frozen legacy ID like `D7`) when the reason lives there.
- Magic numbers carry their source: `// 1,800 veh/h/lane, HCM basic freeway segment` or `// NHTS 2017: 3.5 trips/person/day`.
- Calibration targets and tolerances live in named constants with a doc comment naming the data they come from.

## 3. Interfaces between components

- The owner of an interface documents it (sim-core owns `ScenarioInput`; sim-core owns the `Sim` API; the app owns its UI events).
- A breaking change to an interface updates, in the same PR: the owner's README, every consumer's README, and the [architecture page](ARCHITECTURE.md#interfaces).
- Each interface has a small checked-in example (fixture) that its tests load, so the doc and the code cannot drift silently.

## 4. Decisions

Add a row to [DECISIONS.md](DECISIONS.md) when you make a choice a future developer would otherwise have to ask about: a dependency, a data source, a format, an algorithm, a deviation from the design doc, a scope call. One line of decision, one line of why.

IDs are per stream so parallel PRs don't collide: `SIM-` (sim-core), `DATA-` (data-pipeline), `APP-` (app), `DOC-` (project-level and docs). Take the next number in your stream's section and **append the row at the bottom of that section**. Never add rows to the frozen legacy `D1`–`D31` table. To reverse a decision, add a new row and mark the old one "Superseded by <ID>".

## 5. Roadmap and architecture upkeep

- A PR that completes or starts a roadmap feature updates its Status row and Gantt tag in [ROADMAP.md](ROADMAP.md).
- A PR that adds a component, changes a data flow, or breaks a tier invariant updates [ARCHITECTURE.md](ARCHITECTURE.md).
- A new doc gets a row in the [docs index](README.md).

## 6. Writing style

- Specific over general. Name the file, the command, the number, the unit. Delete any sentence that could describe any project.
- Present tense for what exists; "planned (F##)" for what does not.
- No marketing words, no filler intros. Lead with the answer.
- Mark estimates and guesses as such, with their source.
- Relative links inside the repo so they work on GitHub and locally.
- Diagrams in Mermaid (GitHub renders it), not images, so they stay editable in a diff.

## 7. PR checklist

The [PR template](../.github/pull_request_template.md) carries this checklist. Reviewers block merges that skip it.

- [ ] Component README updated (summary, run, test, interfaces, rationale, gaps, status)
- [ ] Public items have doc comments; `cargo doc` builds clean
- [ ] Interface change? Owner and consumer docs and ARCHITECTURE.md updated, fixture updated
- [ ] New decision? Row appended to your stream's section of DECISIONS.md (`SIM-`/`DATA-`/`APP-`/`DOC-` ID)
- [ ] Roadmap Status and Gantt tag updated for features touched
- [ ] New doc listed in docs/README.md
