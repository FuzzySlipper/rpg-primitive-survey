# Output contract

Every inventory and scan receipt records:

- schema version and tool version;
- `complete` or `partial` status;
- the requested repository/ref and exact pinned commit;
- the effective safety limits;
- diagnostics and an explicit resumability record;
- deterministic evidence pointers relative to the pinned checkout.

`complete` means only that the configured repository surface was traversed
within the configured limits. It does not mean that the RPG system, its
published sources, or its semantic primitives are complete.

`partial` is mandatory whenever a safety bound interrupts traversal. The
receipt identifies the bound, observed value, configured maximum, and next
cursor. A consumer must not promote a partial receipt to a completed primitive
study.

Source identity is evidence, not a guess. A document with no configured source
identity is `missing`; a document with conflicting identities is `ambiguous`.
Both are listed under `unclassifiedOrAmbiguous` and excluded from scan samples.

Structural signatures encode JSON shape and scalar kinds, not scalar values.
They support bounded deduplication; they do not prove semantic equivalence.

The generated Markdown documents are worksheets. Candidate IDs, descriptions,
ASHA coverage claims, and implementation recommendations require a later
reasoning pass and must cite the local evidence pointers retained in
`scan.json`.
