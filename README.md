# RPG Primitive Survey

`rpg-primitive-survey` is a bounded evidence-gathering tool for studying the
structural requirements exposed by Foundry VTT system data. Its output is meant
for a later reasoning pass: a survey agent identifies candidate RPG primitives
and representative structural witnesses, then separate implementation work
decides whether and how ASHA should support them.

It is intentionally **not** a Foundry importer, content converter, ASHA IR
generator, ruleset engine, or unattended porting pipeline.

## Workflow

Inventory pins the requested Git ref (default: `main`) and records repository,
manifest, pack, document-type, subtype, source-identity, count, size, and shape
evidence:

```bash
cargo run -- inventory \
  --study example-system \
  --repository https://github.com/example/foundry-system.git \
  --work-root .work
```

If `main` does not exist, inventory reports the detected default branch and
available heads and exits. It never silently substitutes another branch.
The exact commit is fetched with depth one under an operating-system file-size
limit before checkout. The projected Git objects, working tree, and index must
fit `--max-repository-bytes`; an oversized or interrupted fetch is removed and
recorded as a resumable partial receipt. This conservative path requires
`prlimit` (util-linux) and never falls back to an unbounded clone.

Scan applies an explicit source whitelist to the pinned inventory:

```bash
cargo run -- scan \
  --study example-system \
  --include-source core-rulebook \
  --include-source game-master-guide \
  --work-root .work
```

The complete source audit partitions evidence into included,
excluded-by-whitelist, and unclassified/ambiguous groups. Missing and ambiguous
source identities remain excluded. Structural signatures collapse identical
JSON shapes before deterministic examples are selected.

Both commands enforce configurable limits. A limit produces a machine-readable
`partial` result with a resume cursor and diagnostic; it never silently
truncates or claims completeness. Rerunning against the same study continues at
the original pinned revision. Raise the relevant limit when the recorded total
has reached the old ceiling.

An existing checkout must have the recorded `HEAD` and no tracked, untracked,
or ignored additions. Local filesystem edits never become evidence under an
old pin.

Generated output is local-only under:

```text
<work-root>/studies/<study-id>/
  checkout/
  inventory.json
  scan.json
  source-audit.json
  study-summary.md
  primitive-candidates.md
  representative-examples.md
  asha-coverage.md
  open-questions.md
```

The Markdown files are study worksheets, not implementation specifications.
They are created only when absent, so rerunning a scan updates machine receipts
without destroying interpreted work. They retain source pointers, evidence
strength, boundaries, and unanswered questions so a later coding task can make
an explicit ASHA design decision.

## Source layouts

The generic profile discovers JSON, JSONL, and YAML documents from pack paths
declared by `system.json`, plus any repeated `--content-root` paths. It reads source
identities from a small set of common JSON pointers. System-specific survey
profiles may add explicit content roots and source pointers without weakening
the default exclusion policy. When provenance genuinely exists only at the
pack/layout level, `--source-fallback PACK=SOURCE` classifies missing identities
for that pack. It never overrides conflicting document-level evidence.

`--profile foundry-dnd5e` maps declared `packs/<id>` build outputs to authored
`packs/_source/<id>` YAML, excludes `_folder.yml`/`_folder.yaml` organization records, and
uses a non-empty manifest `flags.dnd5e.sourceBook` as missing-only pack
provenance. Conflicting document evidence remains ambiguous and excluded.

`--profile foundry-pf2e` maps declared `packs/<id>` build outputs to
`packs/pf2e/<id>`, excludes `_folders.json` organization records, and retains
per-document `system.publication.title` identities. The profile does not infer
publication identity from pack names or labels.
Profile behavior is structural; it does not embed or reproduce source content.

## Repository hygiene

Only code, contracts, templates, documentation, tests, and synthetic fixtures
belong in Git. Verify that tracked files obey this boundary with:

```bash
cargo run -- tracked-audit --repository-root .
```

See [the output contract](docs/output-contract.md) and
[the scope guide](docs/scope-guide.md).
