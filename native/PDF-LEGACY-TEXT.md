# Read-only legacy PDF text crosswalk

`validate_delivery.crosswalk_legacy_pdf_text` is an optional developer migration
diagnostic, not part of acceptance. It takes two positional inventory paths
(legacy schema 1 first, native schema 1 second) and the keyword arguments
`source_id`, `expected_legacy_inventory_sha256`,
`expected_native_inventory_sha256`. Obtain the two expected hashes from
trusted existing bindings rather than trusting any digest named in an input.

The invocation replays the selected native inventory against the original
source bytes, then returns a `pdf-legacy-text-crosswalk/v1` receipt. It never
writes either inventory, their evidence, an existing draft or a review ledger.
A legacy inventory can be read after relocation if its source ID, role and PDF
digest match independently replayed native source bytes. The source path in
the historical input is not reinterpreted as a different source locator.

A mapped page requires the documented unrotated page frame and physical page
index, finite intersecting block locations, and byte-exact UTF-8 tokens
after separating only ASCII ordinary spaces, tabs, CR and LF. It retains
whole-block IDs in both directions and a zero-based, half-open token span.
Repeated block/group signatures, crossing partitions, reordered or changed
text, unmappable glyphs, and OCR/regional issues remain pending. Coordinate
tolerance of 0.01 pt is only to compare the recorded page frame and bounds
rounding; an intersection check is not font or rendering parity. No Unicode
canonicalization, punctuation replacement, ligature inference or reading-order
conclusion is made.

No proof of a historical engine's authorship, legacy page raster provenance,
or source-fidelity acceptance is inferred; the original material remains
authoritative for the original decisions only. OCR, visual checks and human
decisions still require explicit evidence in the new workflow.
