# Study scope guide

A study request needs only:

- a public Foundry system repository;
- an optional Git ref (otherwise exactly `main`);
- a source whitelist defining the basic published surface;
- optional system-specific manifest path, content roots, or source pointers;
- safety-limit overrides when the defaults are demonstrably too small.

Do not enumerate every excluded supplement. Inventory discovers the available
source identities, and scan derives the excluded-by-whitelist partition from
that universe.

Document categories are observed from the selected repository: manifest pack
types, document types, and top-level subtypes. A system profile should add a
fallback pack/layout classification only when the system lacks usable
per-document source identity. Fallback classification must remain explicit in
the evidence and must not convert ambiguous documents into included documents.

Use the smallest source whitelist that plausibly represents the basic game.
The purpose is to discover reusable RPG building blocks, not to maximize
coverage of published material.
