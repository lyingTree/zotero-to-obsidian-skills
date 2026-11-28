//! Static parameter schemas and usage descriptions for every registered
//! CLI operation so callers never need to inspect Rust source files.
use crate::OPERATIONS;
use anyhow::{bail, Result};
use serde_json::{json, Map, Value};

#[derive(Clone, Copy)]
struct Param {
    name: &'static str,
    ty: &'static str,
    required: bool,
    default: Option<&'static str>,
}

#[derive(Clone, Copy)]
struct OpSchema {
    op: &'static str,
    summary: &'static str,
    args: &'static [Param],
    kwargs: &'static [Param],
}

const fn req_arg(name: &'static str, ty: &'static str) -> Param {
    Param { name, ty, required: true, default: None }
}
const fn opt_arg(name: &'static str, ty: &'static str, default: &'static str) -> Param {
    Param { name, ty, required: false, default: Some(default) }
}
const fn req_kw(name: &'static str, ty: &'static str) -> Param {
    Param { name, ty, required: true, default: None }
}
const fn opt_kw(name: &'static str, ty: &'static str, default: &'static str) -> Param {
    Param { name, ty, required: false, default: Some(default) }
}

const SCHEMAS: &[OpSchema] = &[
    OpSchema {
        op: "deterministic_stage_checks.snapshot",
        summary: "Hash and inspect an actual regular file.",
        args: &[req_arg("path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "deterministic_stage_checks.verify_binding",
        summary: "Verify a recorded file binding against the current file on disk.",
        args: &[req_arg("record", "object"), req_arg("actual_path", "path")],
        kwargs: &[req_kw("base", "path")],
    },
    OpSchema {
        op: "deterministic_stage_checks.inspect_relative_files",
        summary: "Inspect readability and hashes of relative POSIX paths under a root directory.",
        args: &[req_arg("root", "path"), req_arg("relative_paths", "array<string>")],
        kwargs: &[],
    },
    OpSchema {
        op: "deterministic_stage_checks.author_token",
        summary: "Format the filename author token from an ordered list of confirmed author markers.",
        args: &[req_arg("confirmed_author_markers", "array<string>")],
        kwargs: &[opt_kw("unknown_reason", "string|null", "null")],
    },
    OpSchema {
        op: "deterministic_stage_checks.collection_ancestry",
        summary: "Reconstruct full Zotero collection parent chains from API records.",
        args: &[req_arg("item", "object"), req_arg("records", "array<object>")],
        kwargs: &[req_kw("pagination_complete", "boolean")],
    },
    OpSchema {
        op: "deterministic_stage_checks.split_frontmatter",
        summary: "Split Markdown UTF-8 text or $bytes payload into prefix, body, and YAML frontmatter payload.",
        args: &[req_arg("data", "string|$bytes")],
        kwargs: &[],
    },
    OpSchema {
        op: "deterministic_stage_checks.build_properties",
        summary: "Prepend validated YAML properties to a body without prior frontmatter (returns $bytes).",
        args: &[req_arg("data", "string|$bytes")],
        kwargs: &[req_kw("item_type", "string"), req_kw("confirmed", "object")],
    },
    OpSchema {
        op: "deterministic_stage_checks.repair_properties",
        summary: "Remove deprecated top-level keys from YAML frontmatter while preserving body and unknown keys (returns $bytes).",
        args: &[req_arg("data", "string|$bytes")],
        kwargs: &[],
    },
    OpSchema {
        op: "deterministic_stage_checks.audit_properties",
        summary: "Audit before/after Markdown files for frontmatter creation or repair (PROP-A*).",
        args: &[req_arg("before", "path"), req_arg("after", "path")],
        kwargs: &[req_kw("mode", "string"), req_kw("item_type", "string")],
    },
    OpSchema {
        op: "deterministic_stage_checks.audit_unchanged_properties",
        summary: "Verify YAML frontmatter bytes remained identical between two files.",
        args: &[req_arg("before", "path"), req_arg("after", "path")],
        kwargs: &[req_kw("stage", "string")],
    },
    OpSchema {
        op: "deterministic_stage_checks.audit_write_readback",
        summary: "Verify written vault note bytes match the source candidate file (WRITE-A04/A05).",
        args: &[req_arg("candidate", "path"), req_arg("written", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "deterministic_stage_checks.audit_translation_binding",
        summary: "Verify a glossary or candidate binding recorded in a translation review ledger.",
        args: &[req_arg("record_path", "path"), req_arg("actual_file", "path")],
        kwargs: &[req_kw("kind", "string")],
    },
    OpSchema {
        op: "deterministic_stage_checks.audit_rich_tables",
        summary: "Audit rich-table fences in a candidate Markdown file (BODY-A83..A87/A111).",
        args: &[req_arg("candidate", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "format_helpers.build_naming_plan",
        summary: "Build a source-bound portable note filename plan (keyword arguments only).",
        args: &[],
        kwargs: &[
            req_kw("year", "integer|null"),
            req_kw("author_token", "string"),
            req_kw("original_title", "string"),
            req_kw("source_evidence", "object"),
            opt_kw("delivery_path", "path|null", "null"),
            opt_kw("old_path", "path|null", "null"),
            opt_kw("target_directory", "path|null", "null"),
            opt_kw("no_year_reason", "string|null", "null"),
            opt_kw("unknown_author_reason", "string|null", "null"),
            opt_kw("disambiguation_suffix", "string|null", "null"),
            opt_kw("suffix_evidence", "object|null", "null"),
            opt_kw("max_utf16_units", "integer", "250"),
            opt_kw("filename_max_utf8_bytes", "integer|null", "null"),
            opt_kw("full_path_max_utf16_units", "integer|null", "null"),
        ],
    },
    OpSchema {
        op: "format_helpers.check_local_name_conflicts",
        summary: "Check nonrecursive filename collisions in a local directory.",
        args: &[],
        kwargs: &[
            req_kw("filename", "string"),
            req_kw("directory", "path"),
            req_kw("case_sensitive", "boolean"),
            req_kw("unicode_normalization", "string"),
        ],
    },
    OpSchema {
        op: "format_helpers.validate_naming_plan",
        summary: "Recompute and verify a serialized naming plan against its bound source evidence.",
        args: &[req_arg("plan", "object")],
        kwargs: &[opt_kw("delivery_path", "path|null", "null")],
    },
    OpSchema {
        op: "format_helpers.new_figure_table_anchor",
        summary: "Allocate an unused ^tbl-fig-<source>-<number> anchor ID.",
        args: &[],
        kwargs: &[
            req_kw("source_token", "string"),
            req_kw("figure_number_token", "string"),
            opt_kw("occupied_ids", "array<string>", "[]"),
        ],
    },
    OpSchema {
        op: "format_helpers.build_figure_block",
        summary: "Render a figure block from image records, bold caption, and ^fig-* block ID.",
        args: &[req_arg("images", "array<object>")],
        kwargs: &[req_kw("caption", "string"), req_kw("anchor", "string"), opt_kw("newline", "string", "\"\\n\"")],
    },
    OpSchema {
        op: "format_helpers.starts_with_bold_source_label",
        summary: "Check whether text begins with a bold Markdown label (**...**).",
        args: &[req_arg("text", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "format_helpers.apply_visible_text_edits",
        summary: "Apply checked visible-text replacements to a draft Markdown file.",
        args: &[],
        kwargs: &[req_kw("text_file", "path"), req_kw("edits", "array<object>")],
    },
    OpSchema {
        op: "draft_queue.prepare_draft_queue",
        summary: "Initialize a draft queue from a frozen source-inventory JSON file.",
        args: &[req_arg("inventory_path", "path"), req_arg("queue_path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "draft_queue.DraftQueue.open",
        summary: "Open and verify an existing draft queue file.",
        args: &[req_arg("queue_path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "draft_queue.DraftQueue.status",
        summary: "Read completion status and pending units/pages of a draft queue.",
        args: &[req_arg("queue_path", "path")],
        kwargs: &[opt_kw("expected_sha256", "string|null", "null")],
    },
    OpSchema {
        op: "draft_queue.DraftQueue.next_task",
        summary: "Return the next unreviewed page, block grouping task, or unit to draft.",
        args: &[req_arg("queue_path", "path")],
        kwargs: &[opt_kw("expected_sha256", "string|null", "null")],
    },
    OpSchema {
        op: "draft_queue.DraftQueue.review_pdf_page_order",
        summary: "Record confirmed reading order across all blocks on one PDF page.",
        args: &[
            req_arg("queue_path", "path"),
            req_arg("source_id", "string"),
            req_arg("page", "integer"),
            req_arg("ordered_block_ids", "array<string>"),
            req_arg("reason", "string"),
        ],
        kwargs: &[opt_kw("expected_sha256", "string|null", "null")],
    },
    OpSchema {
        op: "draft_queue.DraftQueue.review_additional_objects",
        summary: "Record explicit review of non-text or additional visual objects on one PDF page.",
        args: &[
            req_arg("queue_path", "path"),
            req_arg("source_id", "string"),
            req_arg("page", "integer"),
            req_arg("reason", "string"),
        ],
        kwargs: &[opt_kw("expected_sha256", "string|null", "null")],
    },
    OpSchema {
        op: "draft_queue.DraftQueue.add_page_object",
        summary: "Register an additional observed source block on an actual PDF page.",
        args: &[
            req_arg("queue_path", "path"),
            req_arg("block_id", "string"),
            req_arg("source_id", "string"),
            req_arg("page", "integer"),
        ],
        kwargs: &[
            req_kw("kind", "string"),
            req_kw("reason", "string"),
            opt_kw("bbox", "array<number>|null", "null"),
            opt_kw("text", "string", "\"\""),
            opt_kw("expected_sha256", "string|null", "null"),
        ],
    },
    OpSchema {
        op: "draft_queue.DraftQueue.group",
        summary: "Group one or more source block IDs into a single drafting unit.",
        args: &[req_arg("queue_path", "path"), req_arg("unit_id", "string"), req_arg("block_ids", "array<string>")],
        kwargs: &[req_kw("kind", "string"), req_kw("reason", "string"), opt_kw("expected_sha256", "string|null", "null")],
    },
    OpSchema {
        op: "draft_queue.DraftQueue.exclude",
        summary: "Exclude an actual source block with an explicit reason.",
        args: &[req_arg("queue_path", "path"), req_arg("block_id", "string"), req_arg("reason", "string")],
        kwargs: &[opt_kw("expected_sha256", "string|null", "null")],
    },
    OpSchema {
        op: "draft_queue.DraftQueue.write_unit",
        summary: "Write a Markdown draft revision for a grouped unit.",
        args: &[req_arg("queue_path", "path"), req_arg("unit_id", "string"), req_arg("markdown", "string")],
        kwargs: &[opt_kw("assets", "array<path>", "[]"), opt_kw("expected_sha256", "string|null", "null")],
    },
    OpSchema {
        op: "draft_queue.DraftQueue.record_issue_decision",
        summary: "Record resolved/excluded/blocked disposition for a native source issue.",
        args: &[req_arg("queue_path", "path"), req_arg("issue_id", "string")],
        kwargs: &[
            req_kw("status", "string"),
            req_kw("reason", "string"),
            req_kw("evidence_paths", "array<path>"),
            opt_kw("expected_sha256", "string|null", "null"),
        ],
    },
    OpSchema {
        op: "draft_queue.DraftQueue.assemble",
        summary: "Assemble all completed draft fragments into a candidate Markdown file and draft map.",
        args: &[req_arg("queue_path", "path"), req_arg("output_path", "path")],
        kwargs: &[opt_kw("expected_sha256", "string|null", "null")],
    },
    OpSchema {
        op: "format_helpers.plan_supplement_figure_caption",
        summary: "Format a normalized **图S*：** supplement figure caption preserving original number.",
        args: &[req_arg("original_number", "string|null"), req_arg("caption", "string")],
        kwargs: &[opt_kw("display_number", "string|null", "null"), opt_kw("original_number_reason", "string|null", "null")],
    },
    OpSchema {
        op: "source_url_inventory.scan_source_url_occurrences",
        summary: "Scan every visible URL occurrence across included source blocks.",
        args: &[req_arg("inventory", "object")],
        kwargs: &[opt_kw("excluded_blocks", "array<string>", "[]"), opt_kw("excluded_pages", "array<object|integer>", "[]")],
    },
    OpSchema {
        op: "source_url_inventory.source_url_coverage",
        summary: "Compare recorded external_url metadata units against source URL occurrences.",
        args: &[req_arg("inventory", "object")],
        kwargs: &[opt_kw("excluded_blocks", "array<string>", "[]"), opt_kw("excluded_pages", "array<object|integer>", "[]")],
    },
    OpSchema {
        op: "render_note.content_template",
        summary: "Generate the empty content slot skeleton required by a render_note plan.",
        args: &[req_arg("plan", "object")],
        kwargs: &[],
    },
    OpSchema {
        op: "render_note.render_note",
        summary: "Render a complete Markdown note and exact unit character/line mapping from plan + content.",
        args: &[req_arg("plan", "object"), req_arg("content", "object")],
        kwargs: &[],
    },
    OpSchema {
        op: "format_helpers.build_rich_table_block",
        summary: "Emit a rich-table fenced YAML block with ^tbl-* anchor.",
        args: &[req_arg("data", "object")],
        kwargs: &[
            req_kw("anchor", "string"),
            opt_kw("wrapped", "boolean", "true"),
            opt_kw("newline", "string", "\"\\n\""),
        ],
    },
    OpSchema {
        op: "normalize_note.normalize_note",
        summary: "Normalize spacing, quantities, percent signs, and block spacing on UTF-8 Markdown text.",
        args: &[req_arg("text", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "normalize_note.normalize_file",
        summary: "Normalize a Markdown file into a new destination path with change ledger.",
        args: &[req_arg("source", "path"), req_arg("destination", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "validate_note.validate_note",
        summary: "Run structural Markdown, YAML frontmatter, anchor, and rich-table checks on text.",
        args: &[req_arg("text", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "validate_note.validate_file",
        summary: "Run structural Markdown, YAML frontmatter, anchor, and rich-table checks on a file.",
        args: &[req_arg("path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "validate_note.inspect_note",
        summary: "Return both structural findings and parsed rich-table records for Markdown text.",
        args: &[req_arg("text", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "glossary_candidates.search",
        summary: "Search exact token sequences or substrings in a domain glossary Markdown file.",
        args: &[req_arg("glossary_path", "path"), req_arg("query", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "glossary_candidates.suggest",
        summary: "Suggest pending glossary candidate hits for a source excerpt.",
        args: &[req_arg("glossary_path", "path"), req_arg("text", "string")],
        kwargs: &[opt_kw("max_family_entries", "integer", "8")],
    },
    OpSchema {
        op: "glossary_candidates.prepare_glossary_candidates",
        summary: "Scan every source inventory block against a glossary and write leads JSON.",
        args: &[req_arg("inventory_path", "path"), req_arg("glossary_path", "path"), req_arg("output_path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "note_pipeline.read_json",
        summary: "Read a UTF-8 JSON object file (rejecting BOM and duplicate keys).",
        args: &[req_arg("path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "note_pipeline.build_contract",
        summary: "Build an immutable acceptance contract from a render plan and source file map.",
        args: &[req_arg("plan", "object"), req_arg("sources", "object<source_id,path>")],
        kwargs: &[],
    },
    OpSchema {
        op: "note_pipeline.prepare_content_job",
        summary: "Create a content-addressed job directory with plan.json, contract.json, and content-template.json.",
        args: &[req_arg("plan", "object"), req_arg("sources", "object<source_id,path>"), req_arg("output_root", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "note_pipeline.replay_changes",
        summary: "Deterministically replay character-offset normalization edits onto original text.",
        args: &[req_arg("original", "string"), req_arg("changes", "array<object>")],
        kwargs: &[],
    },
    OpSchema {
        op: "note_pipeline.rebase_rendering",
        summary: "Rebase render_note unit line/character spans through normalization edits.",
        args: &[req_arg("rendering", "object"), req_arg("normalized", "object")],
        kwargs: &[],
    },
    OpSchema {
        op: "note_pipeline.process_markdown",
        summary: "Run normalize + validate + lint + accept on an existing Markdown file.",
        args: &[req_arg("source_path", "path"), req_arg("output_root", "path")],
        kwargs: &[
            opt_kw("contract", "object|null", "null"),
            opt_kw("resource_root", "path|null", "null"),
            opt_kw("write_normalized_to", "path|null", "null"),
            opt_kw("strip_display_trailing_punctuation", "boolean", "false"),
            opt_kw("force_lf", "boolean", "write_normalized_to!=null"),
            opt_kw("convert_markdown_tables", "boolean", "false"),
            opt_kw("compact", "boolean", "false"),
        ],
    },
    OpSchema {
        op: "note_pipeline.process_content",
        summary: "Run render -> normalize -> rebase -> accept -> readback from plan + content.",
        args: &[req_arg("plan", "object"), req_arg("content", "object"), req_arg("output_root", "path")],
        kwargs: &[
            opt_kw("contract", "object|null", "null"),
            opt_kw("resource_root", "path|null", "null"),
            opt_kw("write_normalized_to", "path|null", "null"),
            opt_kw("strip_display_trailing_punctuation", "boolean", "false"),
            opt_kw("force_lf", "boolean", "write_normalized_to!=null"),
            opt_kw("convert_markdown_tables", "boolean", "false"),
            opt_kw("compact", "boolean", "false"),
        ],
    },
    OpSchema {
        op: "lint_presentation.lint_presentation",
        summary: "Run read-only presentation diagnostics on Markdown text.",
        args: &[req_arg("text", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "lint_presentation.lint_file",
        summary: "Run read-only presentation diagnostics on a Markdown file.",
        args: &[req_arg("path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "validate_delivery.check_merges",
        summary: "Validate Rich Table merged cell bounds, endpoints, and covered values.",
        args: &[req_arg("identity", "string"), req_arg("unit", "object|null"), req_arg("data", "object")],
        kwargs: &[],
    },
    OpSchema {
        op: "validate_delivery.scan_markdown_links",
        summary: "Scan Markdown inline links and embeds with Unicode character offsets.",
        args: &[req_arg("text", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "validate_delivery.write_delivery_manifest",
        summary: "Write a file-bound delivery manifest linking inventory, candidate, assets, and optional translation review.",
        args: &[
            req_arg("inventory_path", "path"),
            req_arg("candidate_path", "path"),
            req_arg("deliveries", "array<object>|object"),
            req_arg("manifest_path", "path"),
        ],
        kwargs: &[
            opt_kw("resource_root", "path|null", "null"),
            opt_kw("translation_review_path", "path|null", "null"),
            opt_kw("figure_edge_reviews_required", "boolean", "false"),
        ],
    },
    OpSchema {
        op: "validate_delivery.check_source_provenance",
        summary: "Replay and verify PDF/DOCX source inventory provenance without inferring semantics.",
        args: &[req_arg("inventory_path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "validate_delivery.validate_delivery",
        summary: "Audit a delivery manifest against its inventory, candidate, assets, and reviews.",
        args: &[req_arg("manifest_path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "handoff_maps.make_source_unit",
        summary: "Construct a validated source-unit entry (extra kwargs vary by kind).",
        args: &[req_arg("unit_id", "string"), req_arg("source_id", "string"), req_arg("kind", "string"), req_arg("evidence", "array<object>")],
        kwargs: &[
            opt_kw("field", "string", "required for kind=metadata"),
            opt_kw("original_number", "string|null", "required for figure/table/formula"),
            opt_kw("contains_table", "boolean", "required for kind=figure"),
            opt_kw("shape", "object", "required for kind=table"),
            opt_kw("merges", "array<object>", "required for kind=table"),
            opt_kw("bibliography_id", "string", "required for kind=reference"),
            opt_kw("original_text", "string", "required for kind=reference"),
            opt_kw("number", "string|integer", "required for kind=reference"),
            opt_kw("display_text", "string", "required for citation/cross_reference"),
            opt_kw("references", "array<string>", "required for kind=citation"),
            opt_kw("target_unit", "string", "required for kind=cross_reference"),
        ],
    },
    OpSchema {
        op: "handoff_maps.write_handoff_maps",
        summary: "Write source_map and link_map JSON files bound to inventory and candidate.",
        args: &[
            req_arg("inventory_path", "path"),
            req_arg("candidate_path", "path"),
            req_arg("deliveries", "array<object>|object"),
            req_arg("map_paths", "object"),
        ],
        kwargs: &[opt_kw("expected_source_bindings", "object|null", "null")],
    },
    OpSchema {
        op: "handoff_maps.validate_handoff_maps",
        summary: "Validate source_map and link_map against inventory and candidate.",
        args: &[req_arg("inventory_path", "path"), req_arg("candidate_path", "path"), req_arg("map_paths", "object")],
        kwargs: &[opt_kw("expected_source_bindings", "object|null", "null")],
    },
    OpSchema {
        op: "mapping_rerun.record_mapping_rerun",
        summary: "Record an actual mapping rerun comparison inside a workflow run.",
        args: &[req_arg("run_path", "path"), req_arg("deliveries", "array<object>|object"), req_arg("destination", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "handoff_maps.record_text_visual_conflict",
        summary: "Record a PDF text-layer vs visual appearance conflict on an inventory unit.",
        args: &[req_arg("inventory_path", "path"), req_arg("unit_id", "string")],
        kwargs: &[
            req_kw("conflict_id", "string"),
            req_kw("page_number", "integer"),
            req_kw("text_locator", "object"),
            req_kw("text_layer_text", "string"),
            req_kw("visible_text", "string"),
            req_kw("bbox", "array<number>"),
            req_kw("decision", "object"),
        ],
    },
    OpSchema {
        op: "mapping_rerun.compare_stable_projections",
        summary: "Compare structural equality of two in-memory map projections.",
        args: &[req_arg("baseline", "object"), req_arg("current", "object")],
        kwargs: &[],
    },
    OpSchema {
        op: "mapping_rerun.compare_bound_maps",
        summary: "Compare bound baseline and current handoff map files and save report.",
        args: &[req_arg("baseline_paths", "object"), req_arg("comparison_paths", "object"), req_arg("destination", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "pdf_image_objects.index_page_structure",
        summary: "Index raw PDF image XObjects, masks, and transforms on one page.",
        args: &[req_arg("source_pdf", "path")],
        kwargs: &[req_kw("page_number", "integer")],
    },
    OpSchema {
        op: "pdf_image_objects.ImageObjectIndexer.index_page",
        summary: "Index PDF page XObjects and optionally export raw image objects into a directory.",
        args: &[req_arg("source_pdf", "path"), req_arg("page_number", "integer")],
        kwargs: &[opt_kw("object_directory", "path|null", "null"), opt_kw("expected_source_sha256", "string|null", "null")],
    },
    OpSchema {
        op: "pdf_image_objects.export_image_object",
        summary: "Losslessly export an indexed PDF image XObject (compositing SMask/Mask when present).",
        args: &[req_arg("source_pdf", "path")],
        kwargs: &[
            req_kw("page_number", "integer"),
            req_kw("image_xref", "integer"),
            req_kw("destination", "path"),
            opt_kw("expected_source_sha256", "string|null", "null"),
        ],
    },
    OpSchema {
        op: "pdf_image_objects.inspect_crop_boundaries",
        summary: "Check glyph/image crop boundary crossings and render annotated context PNG.",
        args: &[req_arg("source_pdf", "path")],
        kwargs: &[
            req_kw("page_number", "integer"),
            req_kw("bbox", "array<number>"),
            req_kw("coordinate_system", "object"),
            req_kw("context_destination", "path"),
            opt_kw("context_margin_pt", "number", "36.0"),
            opt_kw("dpi", "integer", "144"),
            opt_kw("expected_source_sha256", "string|null", "null"),
            opt_kw("overwrite", "boolean", "false"),
        ],
    },
    OpSchema {
        op: "pdf_image_objects.render_page_region",
        summary: "Render a cropped PDF page region plus boundary context and annotated PNGs (supports overwrite for iterative refinement).",
        args: &[req_arg("source_pdf", "path")],
        kwargs: &[
            req_kw("page_number", "integer"),
            req_kw("bbox", "array<number>"),
            req_kw("coordinate_system", "object"),
            req_kw("destination", "path"),
            opt_kw("dpi", "integer", "300"),
            opt_kw("margin_pt", "number", "6.0"),
            opt_kw("context_destination", "path|null", "null"),
            opt_kw("context_margin_pt", "number", "36.0"),
            opt_kw("context_dpi", "integer", "144"),
            opt_kw("expected_source_sha256", "string|null", "null"),
            opt_kw("overwrite", "boolean", "false"),
            opt_kw("transparent", "boolean", "true"),
        ],
    },
    OpSchema {
        op: "pdf_image_objects.render_page_regions_batch",
        summary: "Render up to 32 PDF crop regions in one PdfCraft session with optional alpha preservation, auto-trim margins, and a numbered contact sheet.",
        args: &[req_arg("source_pdf", "path")],
        kwargs: &[
            req_kw("regions", "array<object>"),
            opt_kw("expected_source_sha256", "string|null", "null"),
            opt_kw("overwrite", "boolean", "true"),
            opt_kw("contact_sheet_destination", "path|null", "null"),
            opt_kw("contact_sheet_columns", "integer|null", "null"),
        ],
    },
    OpSchema {
        op: "pdf_image_objects.render_pdfcraft_page",
        summary: "Render an entire PDF page to PNG via headless PdfCraft.",
        args: &[req_arg("source_pdf", "path")],
        kwargs: &[
            req_kw("page_number", "integer"),
            req_kw("destination", "path"),
            opt_kw("dpi", "integer", "144"),
            opt_kw("expected_source_sha256", "string|null", "null"),
            opt_kw("overwrite", "boolean", "false"),
            opt_kw("transparent", "boolean", "false"),
        ],
    },
    OpSchema {
        op: "pdf_image_objects.render_pdfcraft_region",
        summary: "Render a rectangular PDF page region to PNG without boundary-check overlays.",
        args: &[req_arg("source_pdf", "path")],
        kwargs: &[
            req_kw("page_number", "integer"),
            req_kw("bbox", "array<number>"),
            req_kw("coordinate_system", "object"),
            req_kw("destination", "path"),
            opt_kw("dpi", "integer", "144"),
            opt_kw("expected_source_sha256", "string|null", "null"),
            opt_kw("overwrite", "boolean", "false"),
            opt_kw("transparent", "boolean", "true"),
        ],
    },
    OpSchema {
        op: "pdf_image_objects.extract_pdfcraft_text",
        summary: "Extract PdfCraft per-glyph coordinates and plain text from one PDF page.",
        args: &[req_arg("source_pdf", "path")],
        kwargs: &[req_kw("page_number", "integer"), opt_kw("expected_source_sha256", "string|null", "null")],
    },
    OpSchema {
        op: "prepare_source_inventory.ocr_hardware_plan",
        summary: "Inspect local CPU/GPU hardware suitability for optional PP-OCR packages.",
        args: &[],
        kwargs: &[opt_kw("profile", "string", "\"auto\"")],
    },
    OpSchema {
        op: "prepare_source_inventory.inspect_ocr_package",
        summary: "Inspect and hash-verify an offline PP-OCR package manifest.",
        args: &[],
        kwargs: &[
            req_kw("package_manifest", "path"),
            req_kw("expected_package_sha256", "string"),
            opt_kw("profile", "string", "\"auto\""),
            opt_kw("layout", "string", "\"auto\""),
        ],
    },
    OpSchema {
        op: "prepare_source_inventory.install_ocr_package",
        summary: "Verify and install an offline PP-OCR package into a store directory.",
        args: &[req_arg("package_manifest", "path")],
        kwargs: &[req_kw("expected_package_sha256", "string"), req_kw("store", "path")],
    },
    OpSchema {
        op: "prepare_source_inventory.recognize_ocr_png",
        summary: "Run PP-OCR recognition on a PNG image using an offline package.",
        args: &[req_arg("image_path", "path")],
        kwargs: &[
            req_kw("package_manifest", "path"),
            req_kw("expected_package_sha256", "string"),
            opt_kw("profile", "string", "\"auto\""),
            opt_kw("layout", "string", "\"auto\""),
            opt_kw("expected_image_sha256", "string|null", "null"),
        ],
    },
    OpSchema {
        op: "prepare_source_inventory.recognize_ocr_png_accelerated",
        summary: "Run PP-OCR on a PNG image with isolated optional GPU/CPU acceleration.",
        args: &[req_arg("image_path", "path")],
        kwargs: &[
            req_kw("expected_image_sha256", "string"),
            req_kw("package_manifest", "path"),
            req_kw("expected_package_sha256", "string"),
            opt_kw("gpu_package_manifest", "path|null", "null"),
            opt_kw("expected_gpu_package_sha256", "string|null", "null"),
            opt_kw("profile", "string", "\"auto\""),
            opt_kw("layout", "string", "\"auto\""),
            opt_kw("acceleration", "string", "\"auto\""),
            opt_kw("timeout_seconds", "integer", "180"),
        ],
    },
    OpSchema {
        op: "prepare_source_inventory.recognize_ocr_table_png",
        summary: "Run PP-OCR table structure recognition on a PNG image.",
        args: &[req_arg("image_path", "path")],
        kwargs: &[
            req_kw("package_manifest", "path"),
            req_kw("expected_package_sha256", "string"),
            opt_kw("profile", "string", "\"auto\""),
            opt_kw("cell_mode", "string", "\"off\""),
            opt_kw("expected_image_sha256", "string|null", "null"),
        ],
    },
    OpSchema {
        op: "prepare_source_inventory.recognize_pdf_table_region",
        summary: "Render a PDF table region to PNG and run PP-OCR table recognition.",
        args: &[req_arg("source_pdf", "path")],
        kwargs: &[
            req_kw("package_manifest", "path"),
            req_kw("expected_package_sha256", "string"),
            opt_kw("profile", "string", "\"auto\""),
            opt_kw("cell_mode", "string", "\"off\""),
            req_kw("destination", "path"),
            req_kw("page_number", "integer"),
            req_kw("bbox", "array<number>"),
            req_kw("coordinate_system", "object"),
            opt_kw("dpi", "integer", "200"),
            opt_kw("expected_source_sha256", "string|null", "null"),
        ],
    },
    OpSchema {
        op: "prepare_source_inventory.audit_ocr_evidence",
        summary: "Audit numeric/Greek/math quality and parity on an OCR output record.",
        args: &[req_arg("primary_record", "path")],
        kwargs: &[
            req_kw("expected_primary_sha256", "string"),
            opt_kw("comparison_record", "path|null", "null"),
            opt_kw("expected_comparison_sha256", "string|null", "null"),
        ],
    },
    OpSchema {
        op: "prepare_source_inventory.prepare_pdf_ocr_evidence",
        summary: "Prepare whole-PDF or region OCR evidence artifacts alongside PdfCraft renders.",
        args: &[req_arg("source_pdf", "path")],
        kwargs: &[
            req_kw("destination", "path"),
            opt_kw("package_manifest", "path|null", "null"),
            opt_kw("expected_package_sha256", "string|null", "null"),
            opt_kw("expected_source_sha256", "string|null", "null"),
            opt_kw("profile", "string", "\"auto\""),
            opt_kw("layout", "string", "\"auto\""),
            opt_kw("mode", "string", "\"auto\""),
            opt_kw("dpi", "integer", "200"),
            opt_kw("regions", "object", "{}"),
            opt_kw("coordinate_system", "object|null", "null"),
            opt_kw("accelerator", "object|null", "null"),
        ],
    },
    OpSchema {
        op: "prepare_source_inventory.prepare_pdfcraft_evidence",
        summary: "Render every PDF page, text layer, glyph boxes, and XRef objects using PdfCraft.",
        args: &[req_arg("source_pdf", "path")],
        kwargs: &[
            req_kw("destination", "path"),
            opt_kw("dpi", "integer", "144"),
            opt_kw("expected_source_sha256", "string|null", "null"),
        ],
    },
    OpSchema {
        op: "validate_delivery.replay_native_pdf_inventory",
        summary: "Replay native PDF evidence for one source ID in a source-inventory JSON.",
        args: &[req_arg("inventory_path", "path")],
        kwargs: &[req_kw("source_id", "string")],
    },
    OpSchema {
        op: "validate_delivery.crosswalk_legacy_pdf_text",
        summary: "Crosswalk legacy PyMuPDF inventory records against native PdfCraft evidence.",
        args: &[req_arg("legacy_inventory_path", "path"), req_arg("native_inventory_path", "path")],
        kwargs: &[
            req_kw("expected_legacy_inventory_sha256", "string"),
            req_kw("expected_native_inventory_sha256", "string"),
            req_kw("source_id", "string"),
        ],
    },
    OpSchema {
        op: "prepare_source_inventory.prepare_pdf_structure_evidence",
        summary: "Index structural image XObjects across every page of a PDF.",
        args: &[req_arg("source_pdf", "path"), req_arg("destination", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "prepare_source_inventory.record_figure_edges",
        summary: "Record explicit left/right/top/bottom inspection observations on a figure-review ledger.",
        args: &[req_arg("review_path", "path"), req_arg("unit_id", "string"), req_arg("asset_index", "integer")],
        kwargs: &[req_kw("source_location_indices", "array<integer>"), req_kw("observations", "object")],
    },
    OpSchema {
        op: "prepare_source_inventory.prepare_figure_reviews",
        summary: "Initialize schema-2 pending figure-review ledger bound to final cropped assets.",
        args: &[req_arg("inventory_path", "path"), req_arg("final_assets", "object<unit_id,array<path>>"), req_arg("work_directory", "path")],
        kwargs: &[opt_kw("resource_root", "path|null", "null")],
    },
    OpSchema {
        op: "prepare_source_inventory.file_record",
        summary: "Return path and SHA-256 binding record for a local file.",
        args: &[req_arg("path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "prepare_source_inventory.sha256_file",
        summary: "Return lowercase hex SHA-256 digest of a local file.",
        args: &[req_arg("path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "prepare_source_inventory.prepare_sources",
        summary: "Render and inventory all PDF/DOCX attachments from a {source_id: path} object into work_directory.",
        args: &[req_arg("source_pdfs", "object<source_id,path>"), req_arg("work_directory", "path")],
        kwargs: &[
            opt_kw("source_roles", "object<source_id,main|supplement|pending>|null", "null"),
            opt_kw("glossary_path", "path|null", "null"),
            opt_kw("dpi", "integer", "144"),
            opt_kw("ocr", "object|null", "null"),
        ],
    },
    OpSchema {
        op: "complete_workflow.write_manifest",
        summary: "Write and bind the workflow delivery manifest inside an active workflow run.",
        args: &[req_arg("run_path", "path"), req_arg("deliveries", "array<object>|object")],
        kwargs: &[],
    },
    OpSchema {
        op: "complete_workflow.initialize_workflow",
        summary: "Initialize a complete tracked workflow run directory (keyword arguments only).",
        args: &[],
        kwargs: &[
            req_kw("skills_root", "path"),
            req_kw("work_root", "path"),
            req_kw("sources", "object<source_id,path>"),
            req_kw("source_roles", "object<source_id,main|supplement|pending>"),
            req_kw("candidate_path", "path"),
            req_kw("resource_root", "path"),
            opt_kw("glossary_path", "path|null", "null"),
            opt_kw("translation_required", "boolean", "true"),
            opt_kw("run_id", "string", "\"local-candidate\""),
            opt_kw("ocr", "object|null", "null"),
        ],
    },
    OpSchema {
        op: "complete_workflow.resume_workflow",
        summary: "Inspect the current state and next required steps of a workflow run.",
        args: &[req_arg("run_path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "complete_workflow.inspect_workflow",
        summary: "Re-audit all ledgers and produce the full workflow completion report.",
        args: &[req_arg("run_path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "complete_workflow.require_completed_records",
        summary: "Verify all mechanical workflow records are complete (fails if any finding remains).",
        args: &[req_arg("run_path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "complete_workflow.write_maps",
        summary: "Write bound handoff source_map and link_map for an active workflow run.",
        args: &[req_arg("run_path", "path"), req_arg("deliveries", "array<object>|object")],
        kwargs: &[],
    },
    OpSchema {
        op: "complete_workflow.set_naming_plan",
        summary: "Bind a validated naming plan to an active workflow run.",
        args: &[req_arg("run_path", "path"), req_arg("naming_plan", "object")],
        kwargs: &[opt_kw("expected_run_sha256", "string|null", "null")],
    },
    OpSchema {
        op: "complete_workflow.set_vault_write_scope",
        summary: "Record explicit vault write authorization (or denial) on an active workflow run.",
        args: &[req_arg("run_path", "path"), req_arg("write_vault", "boolean")],
        kwargs: &[
            req_kw("authorized", "boolean"),
            opt_kw("vault_path", "string|null", "null"),
            opt_kw("expected_run_sha256", "string|null", "null"),
        ],
    },
    OpSchema {
        op: "accept_note.accept_note",
        summary: "Run read-only mechanical acceptance checks on a candidate Markdown file.",
        args: &[req_arg("candidate_path", "path"), opt_arg("contract", "object|null", "null")],
        kwargs: &[opt_kw("resource_root", "path|null", "null"), opt_kw("rendering", "object|null", "null")],
    },
    OpSchema {
        op: "docx_source_inventory.read_docx",
        summary: "Parse a DOCX document into native XML blocks, tables, and media calls.",
        args: &[req_arg("path", "path"), req_arg("source_id", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "docx_source_inventory.prepare_docx_source",
        summary: "Extract and hash-bind DOCX XML and embedded media into a directory.",
        args: &[req_arg("path", "path"), req_arg("source_id", "string"), req_arg("directory", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "docx_source_inventory.valid_native_locator",
        summary: "Validate syntax of a native DOCX element locator object.",
        args: &[req_arg("value", "object")],
        kwargs: &[],
    },
    OpSchema {
        op: "docx_source_inventory.native_evidence_errors",
        summary: "Validate DOCX evidence locator errors against a parsed DOCX source record.",
        args: &[req_arg("source", "object"), req_arg("location", "object")],
        kwargs: &[],
    },
    OpSchema {
        op: "docx_source_inventory.source_native_text",
        summary: "Read source text corresponding to a DOCX evidence location.",
        args: &[req_arg("source", "object"), req_arg("location", "object")],
        kwargs: &[],
    },
    OpSchema {
        op: "docx_source_inventory.resolve_native_locator",
        summary: "Resolve a native DOCX element path directly from a DOCX file.",
        args: &[req_arg("path", "path"), req_arg("locator", "object")],
        kwargs: &[],
    },
    OpSchema {
        op: "docx_source_inventory.native_figure_location",
        summary: "Validate and resolve a DOCX media-call figure location.",
        args: &[req_arg("source", "object"), req_arg("location", "object")],
        kwargs: &[],
    },
    OpSchema {
        op: "prepare_translation_review.bind_review_file",
        summary: "Bind an input file path and SHA-256 for translation review.",
        args: &[req_arg("path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "prepare_translation_review.bind_review_evidence",
        summary: "Bind a file and location record for translation review evidence.",
        args: &[req_arg("path", "path"), req_arg("locator", "object")],
        kwargs: &[],
    },
    OpSchema {
        op: "prepare_translation_review.new_occurrence",
        summary: "Construct a pending occurrence record template.",
        args: &[req_arg("occurrence_id", "string"), req_arg("kind", "string")],
        kwargs: &[req_kw("source_span", "object")],
    },
    OpSchema {
        op: "prepare_translation_review.new_term_candidate",
        summary: "Construct a pending term candidate record template.",
        args: &[req_arg("term_id", "string"), req_arg("original", "string"), req_arg("category", "string")],
        kwargs: &[req_kw("source_excerpt", "string"), req_kw("source_locator", "object")],
    },
    OpSchema {
        op: "prepare_translation_review.expand_source_locations",
        summary: "Expand inventory unit evidence into block-level source locations.",
        args: &[req_arg("unit", "object"), req_arg("source", "object")],
        kwargs: &[],
    },
    OpSchema {
        op: "prepare_translation_review.prepare_translation_review",
        summary: "Initialize a pending translation-review skeleton bound to inventory, candidate, and unit_ids.",
        args: &[req_arg("inventory_path", "path"), req_arg("candidate_path", "path"), req_arg("output_path", "path")],
        kwargs: &[
            req_kw("unit_ids", "array<string>"),
            opt_kw("glossary_path", "path|null", "null"),
            opt_kw("candidate_locations", "object|null", "null"),
        ],
    },
    OpSchema {
        op: "validate_translation_review.validate_translation_review",
        summary: "Validate a saved translation review JSON against optional inventory and candidate files.",
        args: &[
            req_arg("path", "path"),
            opt_arg("inventory_path", "path|null", "null"),
            opt_arg("candidate_path", "path|null", "null"),
        ],
        kwargs: &[],
    },
    OpSchema {
        op: "translation_review_writer.replay_decisions",
        summary: "Replay prior decision commands against prepared_path without saving.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[],
    },
    OpSchema {
        op: "translation_review_writer.select_source",
        summary: "Resolve a source excerpt selector within a unit and return a replayable locator.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[req_kw("unit_id", "string"), req_kw("selector", "object")],
    },
    OpSchema {
        op: "translation_review_writer.select_candidate",
        summary: "Resolve a candidate excerpt selector within a unit and return Unicode char_range.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[req_kw("unit_id", "string"), req_kw("selector", "object")],
    },
    OpSchema {
        op: "translation_review_writer.append_decision",
        summary: "Validate and append one raw short-call decision command.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[req_kw("command", "object")],
    },
    OpSchema {
        op: "translation_review_writer.ReviewWriter.scope",
        summary: "Record explicit scope selection_review status and reason.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[req_kw("status", "string"), req_kw("reason", "string")],
    },
    OpSchema {
        op: "translation_review_writer.ReviewWriter.exclude",
        summary: "Record explicit applicability exclusion for an unselected inventory unit.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[req_kw("unit_id", "string"), req_kw("condition", "string"), req_kw("reason", "string")],
    },
    OpSchema {
        op: "translation_review_writer.ReviewWriter.source_issues_review",
        summary: "Record source-issues review disposition across selected units.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[req_kw("status", "string"), req_kw("reason", "string")],
    },
    OpSchema {
        op: "translation_review_writer.ReviewWriter.terminology_review",
        summary: "Record overall terminology review status and covered unit IDs.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[req_kw("status", "string"), req_kw("covered_units", "array<string>"), opt_kw("no_unlisted_reason", "string|null", "null")],
    },
    OpSchema {
        op: "translation_review_writer.ReviewWriter.review",
        summary: "Set overall translation review_status to pending or reviewed.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[req_kw("status", "string")],
    },
    OpSchema {
        op: "translation_review_writer.ReviewWriter.unit",
        summary: "Bind candidate_locations for one selected unit in the translation review.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[req_kw("unit_id", "string"), opt_kw("candidate_locations", "array<object>|null", "null")],
    },
    OpSchema {
        op: "translation_review_writer.UnitReview.check",
        summary: "Record completeness/accuracy check status and reason for one unit.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[req_kw("unit_id", "string"), req_kw("name", "string"), req_kw("status", "string"), req_kw("reason", "string")],
    },
    OpSchema {
        op: "translation_review_writer.UnitReview.scan",
        summary: "Record term scan coverage for one category within a unit.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[
            req_kw("unit_id", "string"),
            req_kw("category", "string"),
            req_kw("status", "string"),
            req_kw("candidate_ids", "array<string>"),
            req_kw("reason", "string"),
        ],
    },
    OpSchema {
        op: "translation_review_writer.UnitReview.occurrence",
        summary: "Record a source-vs-candidate occurrence check (claim, number, formula, citation, cell).",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[
            req_kw("unit_id", "string"),
            req_kw("kind", "string"),
            req_kw("occurrence_id", "string"),
            req_kw("source", "object"),
            req_kw("candidate", "object"),
            req_kw("status", "string"),
            req_kw("reason", "string"),
            opt_kw("comparison", "object", "{}"),
        ],
    },
    OpSchema {
        op: "translation_review_writer.UnitReview.term",
        summary: "Register a domain term candidate located in a unit's source text.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[
            req_kw("unit_id", "string"),
            req_kw("term_id", "string"),
            req_kw("original", "string"),
            req_kw("category", "string"),
            req_kw("source", "object"),
            opt_kw("aliases", "array<string>", "[]"),
        ],
    },
    OpSchema {
        op: "translation_review_writer.UnitReview.issue",
        summary: "Record a source-vs-candidate ambiguity or issue inside a unit.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[
            req_kw("unit_id", "string"),
            req_kw("issue_id", "string"),
            req_kw("source", "object"),
            req_kw("description", "string"),
            req_kw("candidate_readings", "array<string>"),
            req_kw("disposition", "string"),
            req_kw("status", "string"),
            req_kw("reason", "string"),
        ],
    },
    OpSchema {
        op: "translation_review_writer.TermReview.search",
        summary: "Execute and record an actual glossary search query for a registered term.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[
            req_kw("unit_id", "string"),
            req_kw("term_id", "string"),
            req_kw("mode", "string"),
            req_kw("query", "string"),
            req_kw("interpretation", "string"),
        ],
    },
    OpSchema {
        op: "translation_review_writer.TermReview.search_status",
        summary: "Record pending/not_applicable/blocked/unresolved search mode status without fabricating hits.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[
            req_kw("unit_id", "string"),
            req_kw("term_id", "string"),
            req_kw("mode", "string"),
            req_kw("status", "string"),
            req_kw("reason", "string"),
        ],
    },
    OpSchema {
        op: "translation_review_writer.TermReview.decide",
        summary: "Record an explicit translation decision referencing actual glossary hits or unlisted basis.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[
            req_kw("unit_id", "string"),
            req_kw("term_id", "string"),
            req_kw("decision", "string"),
            req_kw("translation", "string"),
            req_kw("reason", "string"),
            opt_kw("matches", "array<string>", "[]"),
            opt_kw("resolution_basis", "string|null", "null"),
        ],
    },
    OpSchema {
        op: "render_note.keys",
        summary: "Validate required and optional object field names at a JSON path.",
        args: &[
            req_arg("value", "object"),
            req_arg("path", "string"),
            opt_arg("required", "string", "\"\""),
            opt_arg("optional", "string", "\"\""),
        ],
        kwargs: &[],
    },
    OpSchema {
        op: "render_note.line",
        summary: "Validate a nonempty single-line string without outer whitespace or control chars.",
        args: &[req_arg("value", "string"), req_arg("path", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "render_note.digest",
        summary: "Return SHA-256 hex digest of a UTF-8 string.",
        args: &[req_arg("value", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "render_note.canonical",
        summary: "Return SHA-256 digest of compact JSON serialization of a value.",
        args: &[req_arg("value", "any"), req_arg("path", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "render_note.literal",
        summary: "Escape literal text for Markdown rendering.",
        args: &[req_arg("value", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "render_note.template_fields",
        summary: "Return empty content slots expected for one unit plan definition.",
        args: &[req_arg("unit", "object")],
        kwargs: &[],
    },
    OpSchema {
        op: "prepare_source_inventory.pdf_page_rect",
        summary: "Read the unrotated page bounding rectangle [0, 0, w, h] in pt for a PDF page.",
        args: &[req_arg("source_pdf", "path")],
        kwargs: &[req_kw("page_number", "integer")],
    },
    OpSchema {
        op: "prepare_source_inventory.valid_pdf_bbox",
        summary: "Check whether a [x0, y0, x1, y1] box lies strictly inside a PDF page_rect.",
        args: &[req_arg("box", "array<number>"), req_arg("page_rect", "array<number>")],
        kwargs: &[],
    },
    OpSchema {
        op: "check_records.AtomicRule.catalog_entry",
        summary: "Look up an atomic C/A rule catalog entry from skills_root by rule_id.",
        args: &[req_arg("skills_root", "path")],
        kwargs: &[req_kw("rule_id", "string")],
    },
    OpSchema {
        op: "check_records.AtomicRule.location",
        summary: "Look up file path and line locator for an atomic rule ID under skills_root.",
        args: &[req_arg("skills_root", "path")],
        kwargs: &[req_kw("rule_id", "string")],
    },
    OpSchema {
        op: "glossary_candidates.GlossaryIndex.search",
        summary: "Search exact token sequences or substrings in a domain glossary Markdown file.",
        args: &[req_arg("glossary_path", "path"), req_arg("query", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "glossary_candidates.GlossaryIndex.suggest",
        summary: "Suggest pending glossary candidate hits for a source excerpt.",
        args: &[req_arg("glossary_path", "path"), req_arg("text", "string")],
        kwargs: &[opt_kw("max_family_entries", "integer", "8")],
    },
    OpSchema {
        op: "translation_review_writer.save_decision_draft",
        summary: "Save replayable incremental translation review decisions to output_path.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[req_kw("output_path", "path")],
    },
    OpSchema {
        op: "translation_review_writer.restore_decision_draft",
        summary: "Restore and verify an incremental translation decision draft file.",
        args: &[req_arg("draft_path", "path")],
        kwargs: &[opt_kw("expected_draft_sha256", "string|null", "null")],
    },
    OpSchema {
        op: "translation_review_writer.save_review",
        summary: "Compile and save validated translation review (and optional terminology) JSON.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[req_kw("output_path", "path"), opt_kw("terminology_path", "path|null", "null")],
    },
    OpSchema {
        op: "translation_review_tasks.unit_record_report",
        summary: "Inspect current validation findings for one selected translation unit.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[req_kw("unit_id", "string")],
    },
    OpSchema {
        op: "translation_review_tasks.require_unit_records",
        summary: "Require zero errors on one selected unit's translation review records.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[req_kw("unit_id", "string")],
    },
    OpSchema {
        op: "translation_review_tasks.next_review_task",
        summary: "Return the next pending task in scope -> terminology -> per-unit review.",
        args: &[req_arg("prepared_path", "path"), opt_arg("decisions", "array<object>", "[]")],
        kwargs: &[opt_kw("scope_limit", "integer", "8")],
    },
    OpSchema {
        op: "prepare_source_inventory.prepare_docx_sources",
        summary: "Prepare inventory from DOCX sources only (blocks if any PDF is given).",
        args: &[req_arg("source_pdfs", "object<source_id,path>"), req_arg("work_directory", "path")],
        kwargs: &[
            opt_kw("source_roles", "object<source_id,main|supplement|pending>|null", "null"),
            opt_kw("glossary_path", "path|null", "null"),
            opt_kw("dpi", "integer", "144"),
        ],
    },
    OpSchema {
        op: "check_records.bind_file",
        summary: "Bind a local file's canonical path and SHA-256 digest.",
        args: &[req_arg("path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "check_records.bind_evidence",
        summary: "Construct a file-bound evidence entry with locator, evidence_type, and observation.",
        args: &[req_arg("path", "path"), req_arg("locator", "object")],
        kwargs: &[opt_kw("evidence_type", "string", "\"offline_result\""), opt_kw("observation", "any", "null")],
    },
    OpSchema {
        op: "check_records.prepare_check_records",
        summary: "Initialize an atomic checks ledger bound to skills_root, sources, and candidates.",
        args: &[req_arg("skills_root", "path"), req_arg("output_path", "path")],
        kwargs: &[
            req_kw("sources", "object<source_id,path>"),
            opt_kw("candidates", "object|null", "null"),
            opt_kw("candidate_path", "path|null", "null"),
            opt_kw("scopes", "object|null", "null"),
            opt_kw("requirement_types", "array<string>|null", "null"),
            opt_kw("environment_kinds", "array<string>|null", "null"),
            opt_kw("rule_selection", "string", "\"acceptance\""),
            opt_kw("additional_ids", "array<string>", "[]"),
            opt_kw("inventory_path", "path|null", "null"),
            opt_kw("overwrite", "boolean", "false"),
        ],
    },
    OpSchema {
        op: "deterministic_stage_checks.coverage_catalog",
        summary: "Enumerate all atomic C/A rules across skills_root and their helper mappings.",
        args: &[req_arg("skills_root", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "complete_workflow.pending_check_batch",
        summary: "Return the next batch of pending atomic check IDs and metadata.",
        args: &[req_arg("checks_path", "path")],
        kwargs: &[opt_kw("limit", "integer", "20"), opt_kw("prefix", "string|null", "null")],
    },
    OpSchema {
        op: "complete_workflow.pending_check_rows",
        summary: "Return compact [rule_id, goal] rows for pending atomic checks.",
        args: &[req_arg("checks_path", "path")],
        kwargs: &[opt_kw("limit", "integer", "40"), opt_kw("prefix", "string|null", "null")],
    },
    OpSchema {
        op: "check_records.validate_check_records",
        summary: "Validate an atomic checks ledger file against its rule catalog and evidence bindings.",
        args: &[req_arg("path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "check_records.scopes_from_deliveries",
        summary: "Derive per-rule object scopes from inventory, candidate, and unit_ids.",
        args: &[req_arg("inventory_path", "path"), req_arg("candidate_path", "path"), req_arg("deliveries", "array<object>|object")],
        kwargs: &[req_kw("unit_ids", "array<string>")],
    },
    OpSchema {
        op: "complete_workflow.record_check_batch",
        summary: "Record explicit statuses and reasons for a batch of atomic checks.",
        args: &[req_arg("checks_path", "path"), req_arg("results", "array<object>")],
        kwargs: &[req_kw("evidence", "any")],
    },
    OpSchema {
        op: "complete_workflow.record_decision_rows",
        summary: "Record compact atomic check decision rows with shared contexts and evidence.",
        args: &[req_arg("checks_path", "path"), req_arg("rows", "array<object|array>")],
        kwargs: &[req_kw("contexts", "object"), req_kw("evidence", "any")],
    },
    OpSchema {
        op: "complete_workflow.bind_candidate",
        summary: "Bind the candidate Markdown file to an active workflow run and run pre-manifest audits.",
        args: &[req_arg("run_path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "deterministic_stage_checks.record_file_checks",
        summary: "Run a file audit stage (properties/tables/unchanged) and record mapped checks into checks_path.",
        args: &[req_arg("checks_path", "path"), req_arg("report_path", "path")],
        kwargs: &[
            req_kw("stage", "string"),
            req_kw("after", "path"),
            opt_kw("before", "path|null", "null"),
            opt_kw("mode", "string|null", "null"),
            opt_kw("item_type", "string|null", "null"),
            opt_kw("kind", "string|null", "null"),
        ],
    },
    OpSchema {
        op: "native_runtime.package",
        summary: "Package the 12 script-free skills, the current z2o executable, and an optional verified PP-OCRv6 runtime/model bundle into an all-in-one offline archive.",
        args: &[req_arg("source_root", "path"), req_arg("destination", "path")],
        kwargs: &[opt_kw("ocr_bundle", "path|null", "null")],
    },
    OpSchema {
        op: "native_runtime.inspect_bundle",
        summary: "Verify and inspect an offline z2o skill bundle using an independently trusted SHA-256.",
        args: &[req_arg("package_manifest", "path"), req_arg("expected_package_sha256", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "native_runtime.verify_install",
        summary: "Verify all installed skill files and runtime binary hashes under skills_root.",
        args: &[req_arg("package_manifest", "path"), req_arg("expected_package_sha256", "string"), req_arg("skills_root", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "native_runtime.install",
        summary: "Install a verified offline skill bundle into skills_root with automatic backup.",
        args: &[req_arg("package_manifest", "path"), req_arg("expected_package_sha256", "string"), req_arg("skills_root", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "zotero_local.inspect_item",
        summary: "Read a Zotero item, collection ancestry, children, and verify local attachments via localhost:23119.",
        args: &[req_arg("library_type", "user|group"), req_arg("library_id", "integer"), req_arg("item_key", "string")],
        kwargs: &[
            opt_kw("storage_root", "path|null", "null"),
            opt_kw("attachment_paths", "object<attachment_key,path>", "{}"),
            opt_kw("output_path", "path|null", "null"),
        ],
    },
    OpSchema {
        op: "zotero_local.catalog",
        summary: "Fetch all top-level items from one local Zotero library (use output_path to avoid large stdout).",
        args: &[req_arg("library_type", "user|group"), req_arg("library_id", "integer")],
        kwargs: &[opt_kw("output_path", "path|null", "null")],
    },
    OpSchema {
        op: "zotero_local.search_items",
        summary: "Search items in local Zotero API v3 by query/item_type with bounded pagination.",
        args: &[req_arg("library_type", "user|group"), req_arg("library_id", "integer"), req_arg("query", "string")],
        kwargs: &[
            opt_kw("qmode", "titleCreatorYear|everything", "\"titleCreatorYear\""),
            opt_kw("item_type", "string|null", "null"),
            opt_kw("top_only", "boolean", "true"),
            opt_kw("limit", "integer(1..=100)", "25"),
            opt_kw("start", "integer", "0"),
            opt_kw("output_path", "path|null", "null"),
        ],
    },
    OpSchema {
        op: "zotero_local.sync_child_note",
        summary: "Preview or explicitly authorize a Zotero 10+ child note sync, with versioned updates and verified readback.",
        args: &[
            req_arg("library_type", "user|group"),
            req_arg("library_id", "integer"),
            req_arg("parent_key", "string"),
            req_arg("sync_id", "string"),
            req_arg("html_path", "path"),
        ],
        kwargs: &[
            opt_kw("authorized", "boolean", "false"),
            opt_kw("expected_version", "integer|null", "null"),
            opt_kw("expected_server_id", "string|null", "null"),
        ],
    },
    OpSchema {
        op: "user_profile.locate",
        summary: "Locate the skill-local persistent profile without reading or writing it.",
        args: &[],
        kwargs: &[opt_kw("skill_root", "path|null", "null")],
    },
    OpSchema {
        op: "user_profile.load",
        summary: "Load the skill-local profile; uninitialized state means first-run discovery is required.",
        args: &[],
        kwargs: &[
            opt_kw("skill_root", "path|null", "null"),
            opt_kw("verify_paths", "boolean", "true"),
        ],
    },
    OpSchema {
        op: "user_profile.save",
        summary: "Atomically save the complete skill-local profile after schema and preference validation.",
        args: &[req_arg("profile", "object")],
        kwargs: &[opt_kw("skill_root", "path|null", "null")],
    },
    OpSchema {
        op: "user_profile.patch",
        summary: "Merge a validated patch into the skill-local profile and save atomically.",
        args: &[req_arg("patch", "object")],
        kwargs: &[opt_kw("skill_root", "path|null", "null")],
    },
    OpSchema {
        op: "user_profile.preflight",
        summary: "Load profile, probe Zotero API, list templates, locate attachments, compute naming/bucket plan and optionally create work directories in one call.",
        args: &[],
        kwargs: &[
            opt_kw("skill_root", "path|null", "null"),
            opt_kw("query", "string|null", "null"),
            opt_kw("item_key", "string|null", "null"),
            opt_kw("zotero_storage_dir", "path|null", "null"),
            opt_kw("create_work_dirs", "boolean", "false"),
        ],
    },
    OpSchema {
        op: "prepare_source_inventory.inspect_tex_source",
        summary: "Parse a .tex, .zip, .tar, .tar.gz or .gz arXiv source into source-bound LaTeX blocks.",
        args: &[req_arg("source_path", "path")],
        kwargs: &[
            req_kw("source_id", "string"),
            opt_kw("expected_source_sha256", "string|null", "null"),
        ],
    },
    OpSchema {
        op: "prepare_source_inventory.prepare_tex_source",
        summary: "Parse an arXiv source and publish its bound source.tex, native-blocks.txt and structure JSON.",
        args: &[req_arg("source_path", "path"), req_arg("destination", "path")],
        kwargs: &[
            req_kw("source_id", "string"),
            opt_kw("expected_source_sha256", "string|null", "null"),
        ],
    },
    OpSchema {
        op: "prepare_source_inventory.fetch_arxiv_tex",
        summary: "Detect arXiv ID, download TeX e-print source automatically, parse with .bbl/.gz expansion, and write clean-source.md.",
        args: &[req_arg("target", "string|object"), req_arg("destination", "path")],
        kwargs: &[
            opt_kw("source_id", "string", "\"main\""),
            opt_kw("pdf_path", "path|null", "null"),
            opt_kw("timeout_seconds", "integer", "60"),
            opt_kw("compact", "boolean", "true"),
        ],
    },
    OpSchema {
        op: "prepare_source_inventory.inspect_pdf_quality",
        summary: "Run pure-Rust pdf-inspector + lopdf Type-3/CMap + Mojibake preflight to classify pages needing OCR, tables, columns, and encoding risks.",
        args: &[req_arg("source_pdf", "path")],
        kwargs: &[opt_kw("expected_source_sha256", "string|null", "null")],
    },
    OpSchema {
        op: "prepare_source_inventory.prepare_pdf_layout",
        summary: "Analyze PDF pages via pure-Rust column/paragraph/formula/caption clustering, figure candidate extraction, and selective page rendering.",
        args: &[req_arg("source_path", "path"), req_arg("destination", "path")],
        kwargs: &[
            opt_kw("source_id", "string", "\"main\""),
            opt_kw("render_pages", "selective|all|none", "\"selective\""),
            opt_kw("dpi", "integer", "150"),
            opt_kw("expected_source_sha256", "string|null", "null"),
        ],
    },
    OpSchema {
        op: "prepare_source_inventory.prepare_paper",
        summary: "One-shot source prep: auto-fetch TeX (when tex_target given), run cached/selective PDF layout + figure_candidates, and generate compact matched-glossary.md.",
        args: &[req_arg("work_dir", "path")],
        kwargs: &[
            opt_kw("pdf_path", "path|null", "null"),
            opt_kw("tex_target", "string|object|null", "null"),
            opt_kw("glossary_path", "path|null", "null"),
            opt_kw("render_pages", "auto|selective|all|none", "\"auto\""),
            opt_kw("dpi", "integer", "150"),
            opt_kw("expected_pdf_sha256", "string|null", "null"),
            opt_kw("arxiv_timeout_seconds", "integer", "60"),
        ],
    },
    OpSchema {
        op: "vault_library.index",
        summary: "Read-only index of existing Obsidian literature notes and terminal Zotero item footers (supports directory fingerprint caching).",
        args: &[req_arg("vault_root", "path"), req_arg("literature_relative", "string")],
        kwargs: &[
            opt_kw("excluded_relative", "array<string>", "[]"),
            opt_kw("personal_library_id", "integer|null", "null"),
            opt_kw("output_path", "path|null", "null"),
            opt_kw("use_cache", "boolean", "false"),
        ],
    },
    OpSchema {
        op: "vault_library.match_reference",
        summary: "Cross-check a cited reference against a bound vault index and bound Zotero catalog.",
        args: &[req_arg("index_path", "path"), req_arg("catalog_path", "path"), req_arg("reference", "object")],
        kwargs: &[req_kw("expected_index_sha256", "string"), req_kw("expected_catalog_sha256", "string")],
    },
    OpSchema {
        op: "vault_library.match_references_batch",
        summary: "Batch cross-check up to 500 cited references in one pass over bound vault index and catalog.",
        args: &[req_arg("index_path", "path"), req_arg("catalog_path", "path"), req_arg("references", "array<object>")],
        kwargs: &[
            opt_kw("expected_index_sha256", "string|null", "null"),
            opt_kw("expected_catalog_sha256", "string|null", "null"),
        ],
    },
    OpSchema {
        op: "vault_publish.plan",
        summary: "Read-only preflight check for publishing a completed workflow run into an Obsidian vault.",
        args: &[req_arg("run_path", "path")],
        kwargs: &[
            req_kw("expected_run_sha256", "string"),
            req_kw("vault_root", "path"),
            req_kw("literature_relative", "string"),
            req_kw("assets_relative", "string"),
            req_kw("note_relative", "string"),
            req_kw("expected_existing_sha256", "string|null"),
            opt_kw("inspect_workflow", "boolean", "true"),
        ],
    },
    OpSchema {
        op: "vault_publish.plan_standalone",
        summary: "One-shot read-only standalone publish planner with identical-asset reuse, conflict disambiguation and commit request output.",
        args: &[req_arg("vault_root", "path")],
        kwargs: &[
            req_kw("literature_relative", "string"),
            req_kw("assets_relative", "string"),
            req_kw("note_relative", "string"),
            req_kw("candidate_path", "path"),
            req_kw("resource_root", "path"),
            opt_kw("asset_source_dir", "path|null", "null"),
            opt_kw("backup_root", "path|null", "null"),
            opt_kw("expected_existing_sha256", "string|null", "null"),
            opt_kw("reuse_identical_assets", "boolean", "true"),
            opt_kw("disambiguate_conflicts", "boolean", "true"),
            opt_kw("commit_request_output", "path|null", "null"),
        ],
    },
    OpSchema {
        op: "vault_publish.publish_standalone",
        summary: "One-shot standalone publish: preflight plan + conflict disambiguation + transactional commit + SHA-256 readback + optional report.md.",
        args: &[req_arg("vault_root", "path")],
        kwargs: &[
            req_kw("backup_root", "path"),
            req_kw("literature_relative", "string"),
            req_kw("assets_relative", "string"),
            req_kw("note_relative", "string"),
            req_kw("candidate_path", "path"),
            req_kw("resource_root", "path"),
            opt_kw("asset_source_dir", "path|null", "null"),
            opt_kw("expected_existing_sha256", "string|null", "null"),
            opt_kw("reuse_identical_assets", "boolean", "true"),
            opt_kw("disambiguate_conflicts", "boolean", "true"),
            req_kw("authorized", "boolean"),
            opt_kw("report_output", "path|null", "null"),
        ],
    },
    OpSchema {
        op: "vault_publish.commit_markdown",
        summary: "Transactional backup, write, and SHA-256 readback of a Markdown note and assets into an Obsidian vault.",
        args: &[req_arg("vault_root", "path")],
        kwargs: &[
            req_kw("backup_root", "path"),
            req_kw("literature_relative", "string"),
            req_kw("assets_relative", "string"),
            req_kw("note_relative", "string"),
            req_kw("candidate_path", "path"),
            req_kw("expected_candidate_sha256", "string"),
            req_kw("expected_existing_sha256", "string|null"),
            req_kw("resources", "array<object>"),
            req_kw("scope", "standalone_markdown|complete_workflow"),
            req_kw("authorized", "boolean"),
            opt_kw("workflow_run", "path|null", "null"),
            opt_kw("expected_run_sha256", "string|null", "null"),
        ],
    },
    OpSchema {
        op: "vault_publish.rollback",
        summary: "Roll back a prior vault_publish.commit_markdown transaction preserving newer manual edits.",
        args: &[req_arg("transaction_path", "path")],
        kwargs: &[req_kw("expected_transaction_sha256", "string"), req_kw("authorized", "boolean")],
    },
    OpSchema {
        op: "check_records.CheckFinding.to_dict",
        summary: "Return one finding object from check_records validation.",
        args: &[req_arg("path", "path")],
        kwargs: &[req_kw("finding_index", "integer")],
    },
    OpSchema {
        op: "check_records.CheckRecordsReport.offline_records_ready",
        summary: "Return offline_records_ready boolean for a check records file.",
        args: &[req_arg("path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "check_records.CheckRecordsReport.recorded_checks_complete",
        summary: "Return recorded_checks_complete boolean for a check records file.",
        args: &[req_arg("path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "check_records.CheckRecordsReport.structurally_consistent",
        summary: "Return structurally_consistent boolean for a check records file.",
        args: &[req_arg("path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "check_records.CheckRecordsReport.to_dict",
        summary: "Return full CheckRecordsReport dictionary for a check records file.",
        args: &[req_arg("path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "complete_workflow.CompletionReport.as_dict",
        summary: "Return full workflow CompletionReport dictionary.",
        args: &[req_arg("run_path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "complete_workflow.CompletionReport.records_ready",
        summary: "Return records_ready boolean for a workflow run.",
        args: &[req_arg("run_path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "complete_workflow.CompletionReport.summary",
        summary: "Return grouped summary of findings and next pending goals for a workflow run.",
        args: &[req_arg("run_path", "path")],
        kwargs: &[opt_kw("examples_per_code", "integer", "2")],
    },
    OpSchema {
        op: "handoff_maps.HandoffReport.as_dict",
        summary: "Return full handoff map validation report dictionary.",
        args: &[req_arg("inventory_path", "path"), req_arg("candidate_path", "path"), req_arg("map_paths", "object")],
        kwargs: &[opt_kw("expected_source_bindings", "object|null", "null")],
    },
    OpSchema {
        op: "handoff_maps.HandoffReport.records_ready",
        summary: "Return records_ready boolean from handoff map validation.",
        args: &[req_arg("inventory_path", "path"), req_arg("candidate_path", "path"), req_arg("map_paths", "object")],
        kwargs: &[opt_kw("expected_source_bindings", "object|null", "null")],
    },
    OpSchema {
        op: "handoff_maps.HandoffReport.to_dict",
        summary: "Return full handoff map validation report dictionary.",
        args: &[req_arg("inventory_path", "path"), req_arg("candidate_path", "path"), req_arg("map_paths", "object")],
        kwargs: &[opt_kw("expected_source_bindings", "object|null", "null")],
    },
    OpSchema {
        op: "validate_delivery.DeliveryReport.structurally_consistent",
        summary: "Return structurally_consistent boolean for a delivery manifest.",
        args: &[req_arg("manifest_path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "validate_delivery.DeliveryReport.to_dict",
        summary: "Return full DeliveryReport dictionary for a delivery manifest.",
        args: &[req_arg("manifest_path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "validate_translation_review.TranslationReviewFinding.to_dict",
        summary: "Return one finding object from translation review validation.",
        args: &[
            req_arg("path", "path"),
            opt_arg("inventory_path", "path|null", "null"),
            opt_arg("candidate_path", "path|null", "null"),
        ],
        kwargs: &[req_kw("finding_index", "integer")],
    },
    OpSchema {
        op: "validate_translation_review.TranslationReviewReport.records_ready",
        summary: "Return records_ready boolean from translation review validation.",
        args: &[
            req_arg("path", "path"),
            opt_arg("inventory_path", "path|null", "null"),
            opt_arg("candidate_path", "path|null", "null"),
        ],
        kwargs: &[],
    },
    OpSchema {
        op: "validate_translation_review.TranslationReviewReport.to_dict",
        summary: "Return full TranslationReviewReport dictionary.",
        args: &[
            req_arg("path", "path"),
            opt_arg("inventory_path", "path|null", "null"),
            opt_arg("candidate_path", "path|null", "null"),
        ],
        kwargs: &[],
    },
    OpSchema {
        op: "draft_queue.progress_bar",
        summary: "Emit a progress line on stderr.",
        args: &[req_arg("done", "integer"), req_arg("total", "integer"), req_arg("label", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "handoff_maps.progress_bar",
        summary: "Emit a progress line on stderr.",
        args: &[req_arg("done", "integer"), req_arg("total", "integer"), req_arg("label", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "note_pipeline.progress_bar",
        summary: "Emit a progress line on stderr.",
        args: &[req_arg("done", "integer"), req_arg("total", "integer"), req_arg("label", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "pdf_image_objects.progress_bar",
        summary: "Emit a progress line on stderr.",
        args: &[req_arg("done", "integer"), req_arg("total", "integer"), req_arg("label", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "prepare_source_inventory.progress_bar",
        summary: "Emit a progress line on stderr.",
        args: &[req_arg("done", "integer"), req_arg("total", "integer"), req_arg("label", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "prepare_translation_review.progress_bar",
        summary: "Emit a progress line on stderr.",
        args: &[req_arg("done", "integer"), req_arg("total", "integer"), req_arg("label", "string")],
        kwargs: &[],
    },
    OpSchema {
        op: "lint_presentation.main",
        summary: "CLI wrapper running presentation lint on a Markdown file.",
        args: &[req_arg("path", "path")],
        kwargs: &[],
    },
    OpSchema {
        op: "pdf_image_objects.main",
        summary: "CLI wrapper exporting an indexed PDF image object with optional export_record_path.",
        args: &[req_arg("source_pdf", "path")],
        kwargs: &[
            req_kw("page_number", "integer"),
            req_kw("image_xref", "integer"),
            req_kw("destination", "path"),
            opt_kw("expected_source_sha256", "string|null", "null"),
            opt_kw("export_record_path", "path|null", "null"),
        ],
    },
    OpSchema {
        op: "prepare_source_inventory.main",
        summary: "CLI wrapper running prepare_sources and returning exit_code + inventory_path.",
        args: &[req_arg("source_pdfs", "object<source_id,path>"), req_arg("work_directory", "path")],
        kwargs: &[
            opt_kw("source_roles", "object<source_id,main|supplement|pending>|null", "null"),
            opt_kw("glossary_path", "path|null", "null"),
            opt_kw("dpi", "integer", "144"),
            opt_kw("ocr", "object|null", "null"),
        ],
    },
    OpSchema {
        op: "validate_delivery.main",
        summary: "CLI wrapper running validate_delivery with optional report_path.",
        args: &[req_arg("manifest_path", "path")],
        kwargs: &[opt_kw("report_path", "path|null", "null")],
    },
    OpSchema {
        op: "validate_note.main",
        summary: "CLI wrapper running validate_file and returning exit_code + findings.",
        args: &[req_arg("path", "path")],
        kwargs: &[],
    },
];

fn param_json(p: &Param) -> Value {
    let mut obj = json!({"name": p.name, "type": p.ty, "required": p.required});
    if let Some(d) = p.default {
        obj["default"] = Value::String(d.to_owned());
    }
    obj
}

fn parameter_examples(op: &str) -> Option<Value> {
    match op {
        "format_helpers.build_rich_table_block" => Some(json!({
            "args": [
                {
                    "version": 2,
                    "columns": [
                        {"id": "c_000001", "name": "方法"},
                        {"id": "c_000002", "name": "精度"}
                    ],
                    "rows": [
                        {"id": "r_000001", "cells": {"c_000001": "Mayer$\\,\\Delta_k$", "c_000002": "$0.95$"}}
                    ],
                    "theme": "grid",
                    "locked": true
                }
            ],
            "kwargs": {"anchor": "tbl-1", "wrapped": true}
        })),
        "user_profile.preflight" => Some(json!({
            "args": [],
            "kwargs": {"query": "Persistent Mayer path homology", "create_work_dirs": true}
        })),
        "prepare_source_inventory.prepare_paper" => Some(json!({
            "args": ["D:/work/item-dir"],
            "kwargs": {
                "pdf_path": "D:/Zotero/storage/ABCD1234/paper.pdf",
                "tex_target": "2401.01234",
                "glossary_path": "D:/Vault/文献/template/glossary.md",
                "render_pages": "auto",
                "dpi": 150
            }
        })),
        "pdf_image_objects.render_page_regions_batch" => Some(json!({
            "args": ["D:/work/paper.pdf"],
            "kwargs": {
                "regions": [
                    {"page_number": 4, "bbox": [60.0, 95.0, 520.0, 340.0], "destination": "D:/work/output/assets/fig1.png", "transparent": true, "auto_trim": true, "label": "fig1"}
                ],
                "overwrite": true,
                "contact_sheet_destination": "D:/work/evidence/crop-contact-sheet.png"
            }
        })),
        "note_pipeline.process_markdown" => Some(json!({
            "args": ["D:/work/work/candidate.md", "D:/work/work/pipeline-run"],
            "kwargs": {
                "resource_root": "D:/work/output/assets",
                "write_normalized_to": "D:/work/work/normalized.md",
                "strip_display_trailing_punctuation": true,
                "force_lf": true,
                "convert_markdown_tables": true
            }
        })),
        "vault_library.match_references_batch" => Some(json!({
            "args": [
                "D:/work/evidence/vault-index.json",
                "D:/work/evidence/zotero-catalog.json",
                [{"title": "Path homology", "year": "2012", "item_type": "journalArticle", "authors": ["Alexander Grigor'yan"], "doi": "10.1000/x"}]
            ],
            "kwargs": {}
        })),
        "vault_publish.publish_standalone" => Some(json!({
            "args": ["D:/Vault"],
            "kwargs": {
                "backup_root": "D:/work/work/backups",
                "literature_relative": "文献",
                "assets_relative": "文献/attachments",
                "note_relative": "文献/拓扑/2024-Fan-Paper.md",
                "candidate_path": "D:/work/work/normalized.md",
                "resource_root": "D:/work/output/assets",
                "asset_source_dir": "D:/work/output/assets",
                "expected_existing_sha256": null,
                "authorized": true,
                "report_output": "D:/work/work/publish-report.md"
            }
        })),
        _ => None,
    }
}

fn schema_json(s: &OpSchema) -> Value {
    let args_template: Vec<Value> = s.args.iter().map(|a| json!(format!("<{}:{}>", a.name, a.ty))).collect();
    let mut kwargs_template = Map::new();
    for k in s.kwargs.iter().filter(|k| k.required) {
        kwargs_template.insert(k.name.into(), json!(format!("<{}:{}>", k.name, k.ty)));
    }
    let mut obj = json!({
        "summary": s.summary,
        "args": s.args.iter().map(param_json).collect::<Vec<_>>(),
        "kwargs": s.kwargs.iter().map(param_json).collect::<Vec<_>>(),
        "request_template": {
            "schema_version": 1,
            "operation": s.op,
            "args": args_template,
            "kwargs": kwargs_template,
        }
    });
    if let Some(ex) = parameter_examples(s.op) {
        obj["parameter_examples"] = ex;
    }
    obj
}

pub fn describe_operation(op: &str) -> Result<Value> {
    if let Some(s) = SCHEMAS.iter().find(|s| s.op == op) {
        let mut out = schema_json(s);
        out["schema_version"] = json!(crate::SCHEMA_VERSION);
        out["operation"] = json!(s.op);
        return Ok(out);
    }
    let needle = op.to_ascii_lowercase();
    let tail = needle.rsplit('.').next().unwrap_or(&needle);
    let ns = needle.split('.').next().unwrap_or("");
    let mut primary: Vec<&str> = OPERATIONS
        .iter()
        .copied()
        .filter(|cand| {
            let low = cand.to_ascii_lowercase();
            low.contains(&needle) || (!tail.is_empty() && low.contains(tail))
        })
        .collect();
    primary.sort_unstable();
    if primary.len() < 8 && !ns.is_empty() {
        let prefix = format!("{ns}.");
        let mut secondary: Vec<&str> = OPERATIONS
            .iter()
            .copied()
            .filter(|cand| cand.to_ascii_lowercase().starts_with(&prefix) && !primary.contains(cand))
            .collect();
        secondary.sort_unstable();
        primary.extend(secondary);
    }
    primary.truncate(8);
    if primary.is_empty() {
        bail!("UNSUPPORTED_OPERATION: {op}; run `z2o describe` to list all registered operations");
    }
    bail!(
        "UNSUPPORTED_OPERATION: {op}; closest operations: [{}]; run `z2o describe <operation>` for parameters",
        primary.join(", ")
    );
}

pub fn all_schemas() -> Value {
    let mut map = Map::new();
    for s in SCHEMAS {
        map.insert(s.op.to_owned(), schema_json(s));
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, HashMap};

    #[test]
    fn every_registered_operation_has_schema() {
        let registered: BTreeSet<&str> = OPERATIONS.iter().copied().collect();
        let mut seen = BTreeSet::new();
        for s in SCHEMAS {
            assert!(seen.insert(s.op), "duplicate schema for {}", s.op);
        }
        let missing: Vec<_> = registered.difference(&seen).copied().collect();
        let extra: Vec<_> = seen.difference(&registered).copied().collect();
        assert!(missing.is_empty(), "missing schemas: {missing:?}");
        assert!(extra.is_empty(), "extra schemas: {extra:?}");
        assert_eq!(seen.len(), OPERATIONS.len());

        let mut kw_names: HashMap<&str, BTreeSet<&str>> = HashMap::new();
        for s in SCHEMAS {
            let names: BTreeSet<&str> = s.kwargs.iter().map(|k| k.name).collect();
            kw_names.insert(s.op, names);
        }
        assert!(kw_names["zotero_local.inspect_item"].contains("storage_root"));
        assert!(kw_names["zotero_local.search_items"].contains("limit"));
        assert_eq!(
            SCHEMAS.iter().find(|s| s.op == "prepare_source_inventory.prepare_sources").unwrap().args[0].ty,
            "object<source_id,path>"
        );
        assert!(kw_names["pdf_image_objects.render_page_region"].contains("overwrite"));
        assert!(kw_names["vault_publish.commit_markdown"].contains("authorized"));
        assert!(kw_names["complete_workflow.set_vault_write_scope"].contains("authorized"));
        assert!(kw_names["format_helpers.build_naming_plan"].contains("original_title"));
        assert!(describe_operation("zotero_local.unknown").unwrap_err().to_string().contains("zotero_local.search_items"));
    }
}
