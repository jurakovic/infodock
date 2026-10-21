# Architecture review findings

From the M5 architecture review (plan item S14a, D79): four clean-context reviewers on the whole tree at `f3297a9`, 2026-10-21, one per area: the bar; the sensor framework and the sensor types; the platform layer, providers and sampler; Settings, config and the app's wiring. Unlike the milestone reviews (`review-m1.md`–`review-m4.md`), which looked for defects in each milestone's diff, this one looks at structure: SOLID, module boundaries and dependencies, trait design, design patterns, duplication, oversized files and types, naming, readability, testability, and consistency with `CLAUDE.md`'s conventions.

Each finding is checked against the code and weighed against what a change costs this close to the release. Those worth changing are grouped into plan items (S14b…); the others are listed under *Not changed* with the reason. Line numbers are as of `f3297a9`.

Status: the reviewers are running; findings are added here as each area's report is checked.

## The bar (`src/bar`)

*Pending.*

## Sensors (`src/sensors`)

*Pending.*

## Platform, providers and sampler (`src/platform`, `src/providers`, `src/sampler`)

*Pending.*

## Settings, config and the app (`src/settings`, `src/config`, `ui/`, `src/app.rs`, `src/lib.rs`, `src/transfer.rs` and the other top-level modules)

*Pending.*
