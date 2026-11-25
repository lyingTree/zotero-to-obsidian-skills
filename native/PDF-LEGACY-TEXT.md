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

## Indirect image invocation correspondence

The same receipt also includes an additive, separately versioned
`pages[].image_correspondence` diagnostic. The native page's structure and
encoded image stream hash have already been checked against the actual PDF by
independent replay. The historical comparator is `pdf_stream_sha256` (raw
encoded PDF stream), **not** the extracted PNG/JPEG asset digest. The old
object table keys by xref alone, so the legacy generation must come from
actual invocation records; a missing or disputed generation is not inferred.

A match requires complete image indexes, one invocation per xref on each
side of that page, and exact xref, generation and raw-stream SHA-256.
Unrotated page point frames, container xref, operation index, simple resource
name/path, full object bounds and affine placement must agree. A 0.0001 pt
absolute tolerance in placement and page-frame components accounts only for
floating-point recording. The ordinal alone never supplies image identity.
Only direct, fully in-page calls without forms, clipping flags, other
recorded display effects, explicit masks, soft masks or Decode overrides
are mapped. Incomplete or non-equivalent evidence remains pending, including
repeated calls, and the text result is independent of this image diagnostic.

A mapping matches selectors for an encoded indirect stream and its plain
invocation. It does not validate the figure's visuals, visibility, composition,
reading order, or source acceptance, and does not import any human decision.
