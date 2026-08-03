# Profiles Implementation Phases

## Task Tree

- T1: Fix compile blockers and warnings.
- T2: Complete profile path routing and verify production call sites. Depends on T1.
- T3: Add profile isolation, selection, migration, and picker coverage. Depends on T2.
- T4: Update README, in-app help, CHANGELOG, and OpenWiki. Depends on T2.
- T5: Run `just test`, formatting, and clippy. Depends on T3 and T4.
- T6: Run independent adversarial review. Depends on T5.
- T7: Validate and fix verified review findings. Depends on T6.
- T8: Commit, push, and open PR against `development`. Depends on T7.

## Merge Gates

- T1/T2: library compilation and focused tests.
- T3/T4: focused tests and documentation/path audit.
- T5: `just test`, `cargo fmt`, `cargo fmt --check`, `cargo clippy --all-targets`.
- T6/T7: no verified blocker or high-severity finding.
