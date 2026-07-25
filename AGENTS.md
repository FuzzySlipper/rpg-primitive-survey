# RPG Primitive Survey agent guidance

This repository contains code and synthetic fixtures for bounded, agent-operated
surveys of public RPG-system data. It has no separate Den project. Resolve work
through the owning `asha-rpg` task or campaign.

## Boundaries

- Never commit source-system clones, decoded source documents, scan databases,
  generated study prose, or generated reports.
- All external repositories and generated study artifacts belong under an
  explicitly selected ignored work root.
- This is a study tool, not a Foundry importer, converter, ASHA IR generator,
  ruleset authoring engine, or unattended content-production pipeline.
- Missing and ambiguous source identities are excluded by default.
- Do not silently change a requested Git ref, silently truncate a study, or
  describe a partial study as complete.
- Keep fixtures synthetic and free of third-party expression.

## Verification

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo run -- tracked-audit --repository-root .
```
