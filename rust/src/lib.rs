//! Rust implementation. Unsupported operations fail explicitly.
pub mod files;
pub mod figures;
pub mod metadata;
pub mod naming;
pub mod properties;
pub mod tables;
pub mod yaml_emit;
pub mod render;
pub mod syntax;
pub mod normalize;
pub mod yaml_nodes;
pub mod validate;
pub mod source_urls;
pub mod glossary;
pub mod pipeline;
pub mod presentation;
pub mod grid;
pub mod inventory;
pub mod acceptance;
pub mod docx;
pub mod translation_prepare;
pub mod source_prepare;
pub mod check_ledger;
pub mod check_validate;
pub mod check_write;
pub mod check_scope;
pub mod visible_edits;
pub mod draft_queue;
pub mod delivery_prepare;
pub mod delivery_provenance;
pub mod delivery_validate;
mod svg_asset;
pub mod runtime_reports;
pub mod native_install;
pub mod zotero_local;
pub mod vault_library;
pub mod vault_publish;
pub mod workflow;
pub mod mapping_compare;
pub mod handoff_conflicts;
pub mod handoff_native;
pub mod handoff_scan;
pub mod pdf_structure;
pub mod pdf_boundary;
pub mod pdfcraft_backend;
pub mod pdf_inventory;
mod pdf_legacy_images;
pub mod pdf_legacy_text;
pub mod ocr_backend;
pub mod ocr_acceleration;
pub mod ocr_pdf;
pub mod ocr_table;
pub mod ocr_quality;
pub mod translation_validate;
pub mod translation_writer;
pub mod translation_tasks;
pub mod schema;
pub mod pdf_layout;
pub mod tex_source;
pub mod user_profile;
pub mod mojibake;
pub mod pdf_inspector_bridge;
pub mod text_formats;

use anyhow::{anyhow, bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REFERENCE_VERSION: &str = "1.1.0";
pub const OPERATIONS: &[&str] = &[
    "deterministic_stage_checks.snapshot",
    "deterministic_stage_checks.verify_binding",
    "deterministic_stage_checks.inspect_relative_files",
    "deterministic_stage_checks.author_token",
    "deterministic_stage_checks.collection_ancestry",
    "deterministic_stage_checks.split_frontmatter",
    "deterministic_stage_checks.build_properties",
    "deterministic_stage_checks.repair_properties",
    "deterministic_stage_checks.audit_properties",
    "deterministic_stage_checks.audit_unchanged_properties",
    "deterministic_stage_checks.audit_write_readback",
    "deterministic_stage_checks.audit_translation_binding",
    "deterministic_stage_checks.audit_rich_tables",
    "format_helpers.build_naming_plan",
    "format_helpers.check_local_name_conflicts",
    "format_helpers.validate_naming_plan",
    "format_helpers.new_figure_table_anchor",
    "format_helpers.build_figure_block",
    "format_helpers.starts_with_bold_source_label",
    "format_helpers.apply_visible_text_edits",
    "draft_queue.prepare_draft_queue",
    "draft_queue.DraftQueue.open",
    "draft_queue.DraftQueue.status",
    "draft_queue.DraftQueue.next_task",
    "draft_queue.DraftQueue.review_pdf_page_order",
    "draft_queue.DraftQueue.review_additional_objects",
    "draft_queue.DraftQueue.add_page_object",
    "draft_queue.DraftQueue.group",
    "draft_queue.DraftQueue.exclude",
    "draft_queue.DraftQueue.write_unit",
    "draft_queue.DraftQueue.record_issue_decision",
    "draft_queue.DraftQueue.assemble",
    "format_helpers.plan_supplement_figure_caption",
    "source_url_inventory.scan_source_url_occurrences",
    "source_url_inventory.source_url_coverage",
    "render_note.content_template",
    "render_note.render_note",
    "format_helpers.build_rich_table_block",
    "normalize_note.normalize_note",
    "normalize_note.normalize_file",
    "validate_note.validate_note",
    "validate_note.validate_file",
    "validate_note.inspect_note",
    "glossary_candidates.search",
    "glossary_candidates.suggest",
    "glossary_candidates.prepare_glossary_candidates",
    "note_pipeline.read_json",
    "note_pipeline.build_contract",
    "note_pipeline.prepare_content_job",
    "note_pipeline.replay_changes",
    "note_pipeline.rebase_rendering",
    "note_pipeline.process_markdown",
    "note_pipeline.process_content",
    "lint_presentation.lint_presentation",
    "lint_presentation.lint_file",
    "validate_delivery.check_merges",
    "validate_delivery.scan_markdown_links",
    "validate_delivery.write_delivery_manifest",
    "validate_delivery.check_source_provenance",
    "validate_delivery.validate_delivery",
    "handoff_maps.make_source_unit",
    "handoff_maps.write_handoff_maps",
    "handoff_maps.validate_handoff_maps",
    "mapping_rerun.record_mapping_rerun",
    "handoff_maps.record_text_visual_conflict",
    "mapping_rerun.compare_stable_projections",
    "mapping_rerun.compare_bound_maps",
    "pdf_image_objects.index_page_structure",
    "pdf_image_objects.ImageObjectIndexer.index_page",
    "pdf_image_objects.export_image_object",
    "pdf_image_objects.inspect_crop_boundaries",
    "pdf_image_objects.render_page_region",
    "pdf_image_objects.render_pdfcraft_page",
    "pdf_image_objects.render_pdfcraft_region",
    "pdf_image_objects.extract_pdfcraft_text",
    "prepare_source_inventory.ocr_hardware_plan",
    "prepare_source_inventory.inspect_ocr_package",
    "prepare_source_inventory.install_ocr_package",
    "prepare_source_inventory.recognize_ocr_png",
    "prepare_source_inventory.recognize_ocr_png_accelerated",
    "prepare_source_inventory.recognize_ocr_table_png",
    "prepare_source_inventory.recognize_pdf_table_region",
    "prepare_source_inventory.audit_ocr_evidence",
    "prepare_source_inventory.prepare_pdf_ocr_evidence",
    "prepare_source_inventory.prepare_pdfcraft_evidence",
    "validate_delivery.replay_native_pdf_inventory",
    "validate_delivery.crosswalk_legacy_pdf_text",
    "prepare_source_inventory.prepare_pdf_structure_evidence",
    "prepare_source_inventory.record_figure_edges",
    "prepare_source_inventory.prepare_figure_reviews",
    "prepare_source_inventory.file_record",
    "prepare_source_inventory.sha256_file",
    "prepare_source_inventory.prepare_sources",
    "complete_workflow.write_manifest",
    "complete_workflow.initialize_workflow",
    "complete_workflow.resume_workflow",
    "complete_workflow.inspect_workflow",
    "complete_workflow.require_completed_records",
    "complete_workflow.write_maps",
    "complete_workflow.set_naming_plan",
    "complete_workflow.set_vault_write_scope",
    "accept_note.accept_note",
    "docx_source_inventory.read_docx",
    "docx_source_inventory.prepare_docx_source",
    "docx_source_inventory.valid_native_locator",
    "docx_source_inventory.native_evidence_errors",
    "docx_source_inventory.source_native_text",
    "docx_source_inventory.resolve_native_locator",
    "docx_source_inventory.native_figure_location",
    "prepare_translation_review.bind_review_file",
    "prepare_translation_review.bind_review_evidence",
    "prepare_translation_review.new_occurrence",
    "prepare_translation_review.new_term_candidate",
    "prepare_translation_review.expand_source_locations",
    "prepare_translation_review.prepare_translation_review",
    "validate_translation_review.validate_translation_review",
    "translation_review_writer.replay_decisions",
    "translation_review_writer.select_source",
    "translation_review_writer.select_candidate",
    "translation_review_writer.append_decision",
    "translation_review_writer.ReviewWriter.scope",
    "translation_review_writer.ReviewWriter.exclude",
    "translation_review_writer.ReviewWriter.source_issues_review",
    "translation_review_writer.ReviewWriter.terminology_review",
    "translation_review_writer.ReviewWriter.review",
    "translation_review_writer.ReviewWriter.unit",
    "translation_review_writer.UnitReview.check",
    "translation_review_writer.UnitReview.scan",
    "translation_review_writer.UnitReview.occurrence",
    "translation_review_writer.UnitReview.term",
    "translation_review_writer.UnitReview.issue",
    "translation_review_writer.TermReview.search",
    "translation_review_writer.TermReview.search_status",
    "translation_review_writer.TermReview.decide",
    "render_note.keys",
    "render_note.line",
    "render_note.digest",
    "render_note.canonical",
    "render_note.literal",
    "render_note.template_fields",
    "prepare_source_inventory.pdf_page_rect",
    "prepare_source_inventory.valid_pdf_bbox",
    "check_records.AtomicRule.catalog_entry",
    "check_records.AtomicRule.location",
    "glossary_candidates.GlossaryIndex.search",
    "glossary_candidates.GlossaryIndex.suggest",
    "translation_review_writer.save_decision_draft",
    "translation_review_writer.restore_decision_draft",
    "translation_review_writer.save_review",
    "translation_review_tasks.unit_record_report",
    "translation_review_tasks.require_unit_records",
    "translation_review_tasks.next_review_task",
    "prepare_source_inventory.prepare_docx_sources",
    "check_records.bind_file",
    "check_records.bind_evidence",
    "check_records.prepare_check_records",
    "deterministic_stage_checks.coverage_catalog",
    "complete_workflow.pending_check_batch",
    "complete_workflow.pending_check_rows",
    "check_records.validate_check_records",
    "check_records.scopes_from_deliveries",
    "complete_workflow.record_check_batch",
    "complete_workflow.record_decision_rows",
    "complete_workflow.bind_candidate",
    "deterministic_stage_checks.record_file_checks",
    "native_runtime.package",
    "native_runtime.inspect_bundle",
    "native_runtime.verify_install",
    "native_runtime.install",
    "zotero_local.inspect_item",
    "zotero_local.catalog",
    "zotero_local.search_items",
    "zotero_local.sync_child_note",
    "user_profile.locate",
    "user_profile.load",
    "user_profile.save",
    "user_profile.patch",
    "user_profile.preflight",
    "prepare_source_inventory.inspect_tex_source",
    "prepare_source_inventory.prepare_tex_source",
    "prepare_source_inventory.fetch_arxiv_tex",
    "prepare_source_inventory.inspect_pdf_quality",
    "prepare_source_inventory.prepare_pdf_layout",
    "prepare_source_inventory.prepare_paper",
    "pdf_image_objects.render_page_regions_batch",
    "vault_library.index",
    "vault_library.match_reference",
    "vault_library.match_references_batch",
    "vault_publish.plan",
    "vault_publish.plan_standalone",
    "vault_publish.publish_standalone",
    "vault_publish.commit_markdown",
    "vault_publish.rollback",
    "check_records.CheckFinding.to_dict",
    "check_records.CheckRecordsReport.offline_records_ready",
    "check_records.CheckRecordsReport.recorded_checks_complete",
    "check_records.CheckRecordsReport.structurally_consistent",
    "check_records.CheckRecordsReport.to_dict",
    "complete_workflow.CompletionReport.as_dict",
    "complete_workflow.CompletionReport.records_ready",
    "complete_workflow.CompletionReport.summary",
    "handoff_maps.HandoffReport.as_dict",
    "handoff_maps.HandoffReport.records_ready",
    "handoff_maps.HandoffReport.to_dict",
    "validate_delivery.DeliveryReport.structurally_consistent",
    "validate_delivery.DeliveryReport.to_dict",
    "validate_translation_review.TranslationReviewFinding.to_dict",
    "validate_translation_review.TranslationReviewReport.records_ready",
    "validate_translation_review.TranslationReviewReport.to_dict",
    "draft_queue.progress_bar",
    "handoff_maps.progress_bar",
    "note_pipeline.progress_bar",
    "pdf_image_objects.progress_bar",
    "prepare_source_inventory.progress_bar",
    "prepare_translation_review.progress_bar",
    "lint_presentation.main",
    "pdf_image_objects.main",
    "prepare_source_inventory.main",
    "validate_delivery.main",
    "validate_note.main",
];

/// JSON object keys are unique at every depth. Serde's usual Value parser
/// overwrites duplicates; accepting such an object would change its evidence.
pub struct StrictValue(pub Value);
impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct StrictVisitor;
        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictValue;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON-compatible value with unique string mapping keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Self::Value,E> {
                Ok(StrictValue(json!(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Self::Value,E> {
                Ok(StrictValue(json!(v)))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Self::Value,E> {
                Ok(StrictValue(json!(v)))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Self::Value,E> {
                if !v.is_finite() { return Err(E::custom("nonfinite number")); }
                Ok(StrictValue(json!(v)))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Self::Value,E> {
                Ok(StrictValue(Value::String(v.into())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Self::Value,E> {
                Ok(StrictValue(Value::String(v)))
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<Self::Value,E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> std::result::Result<Self::Value,E> { self.visit_unit() }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Self::Value,A::Error> {
                let mut result = Vec::new();
                while let Some(value) = seq.next_element::<StrictValue>()? { result.push(value.0); }
                Ok(StrictValue(Value::Array(result)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Self::Value,A::Error> {
                let mut result = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if result.contains_key(&key) { return Err(de::Error::custom(format!("duplicate key: {key}"))); }
                    result.insert(key, map.next_value::<StrictValue>()?.0);
                }
                Ok(StrictValue(Value::Object(result)))
            }
        }
        deserializer.deserialize_any(StrictVisitor)
    }
}

pub fn parse_json(text: &str) -> Result<Value> {
    Ok(serde_json::from_str::<StrictValue>(text.strip_prefix('\u{feff}').unwrap_or(text))?.0)
}

pub fn string(v: &Value, name: &str) -> Result<String> {
    let s = v.as_str().ok_or_else(|| anyhow!("{name} must be a string"))?;
    ensure!(!s.trim().is_empty(), "{name} must not be empty");
    Ok(s.to_owned())
}

pub fn path(v: &Value) -> Result<PathBuf> {
    if let Some(obj) = v.as_object() {
        ensure!(obj.len() == 1 && obj.contains_key("$path"), "invalid path wrapper");
        return Ok(PathBuf::from(string(&obj["$path"], "path")?));
    }
    // A plain path is permitted for nested source-file records and direct calls.
    Ok(PathBuf::from(string(v, "path")?))
}

pub fn bytes(v: &Value) -> Result<Vec<u8>> {
    if let Some(s) = v.as_str() {
        return Ok(s.as_bytes().to_vec());
    }
    let obj = v.as_object().ok_or_else(|| anyhow!("expected UTF-8 string or $bytes wrapper"))?;
    ensure!(obj.len() == 1, "invalid $bytes wrapper");
    Ok(STANDARD.decode(obj.get("$bytes").and_then(Value::as_str)
        .ok_or_else(|| anyhow!("expected base64 $bytes string"))?)?)
}

pub fn encoded_bytes(v: &[u8]) -> Value { json!({"$bytes": STANDARD.encode(v)}) }

pub fn path_text(p: &Path) -> Result<String> {
    p.to_str().map(str::to_owned).ok_or_else(|| anyhow!("non-Unicode path: {p:?}"))
}

pub fn describe() -> Value {
    let manifest: Value=serde_json::from_str(include_str!("../migration-status.json"))
        .expect("valid native migration manifest");
    let modules=manifest["module_status"].as_object().expect("module status");
    let mut blockers:Vec<String>=manifest["capabilities"].as_object().expect("capability gates").values()
        .filter(|v|v["qualified"]!=true).filter_map(|v|v["blocker"].as_str().map(str::to_owned)).collect();
    blockers.extend(modules.iter().filter(|(_,v)|*v!="qualified").map(|(name,status)|format!("Unqualified module: {name} ({})",status.as_str().unwrap_or("unknown"))));
    if ENGINE_VERSION!=manifest["release_target"].as_str().unwrap_or(""){blockers.push("Rust engine version mismatch".into());}
    json!({
        "schema_version": SCHEMA_VERSION, "engine": "rust", "version": format!("v{ENGINE_VERSION}"), "engine_version": ENGINE_VERSION,
        "reference_version": REFERENCE_VERSION, "platform": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "pdf_backend": {"name":"pdfcraft-render","revision":pdfcraft_backend::PDFCRAFT_REV,"qualification":"qualified"},
        "pdf_inspector": {"name":"pdf-inspector","version":pdf_inspector_bridge::PDF_INSPECTOR_VERSION,"features":"pure_rust_no_pdfium","qualification":"qualified"},
        "ocr_backend":{"name":"ppocr-rs","revision":ocr_backend::OCR_REV,"policy":"cpu_first","dynamic_runtime":"ONNX API 28; 1.28.3 or 1.32.0 candidate","gpu_required":false,"qualification":"bundled_package","bundled_status":ocr_backend::bundled_status()},
        "full_workflow_ready": blockers.is_empty(), "operations": OPERATIONS,
        "release_target": manifest["release_target"], "module_status": modules,
        "required_targets": manifest["required_targets"], "release_blockers": blockers,
    })
}


pub fn describe_operation(op: &str) -> Result<Value> {
    schema::describe_operation(op)
}

struct Call { op: String, args: VecDeque<Value>, kwargs: Map<String, Value> }
impl Call {
    fn new(op: &str, args: Value, kwargs: Value) -> Result<Self> {
        let args = args.as_array().ok_or_else(|| anyhow!("args must be an array"))?.clone();
        let kwargs = kwargs.as_object().ok_or_else(|| anyhow!("kwargs must be an object"))?.clone();
        Ok(Self { op: op.to_owned(), args: args.into(), kwargs })
    }
    fn positional(&mut self, name: &str) -> Result<Value> {
        match self.args.pop_front() {
            Some(v) => { ensure!(!self.kwargs.contains_key(name), "duplicate argument: {name}"); Ok(v) }
            None => self.required(name),
        }
    }
    fn required(&mut self, name: &str) -> Result<Value> {
        self.kwargs.remove(name).ok_or_else(|| anyhow!("missing argument '{name}' for {}; run `z2o describe {}` for accepted parameters", self.op, self.op))
    }
    fn optional_positional(&mut self, name: &str, default: Value) -> Result<Value> {
        match self.args.pop_front() {
            Some(v) => { ensure!(!self.kwargs.contains_key(name), "duplicate argument: {name}"); Ok(v) }
            None => Ok(self.option(name, default)),
        }
    }
    fn option(&mut self, name: &str, default: Value) -> Value { self.kwargs.remove(name).unwrap_or(default) }
    fn finish(self) -> Result<()> {
        ensure!(self.args.is_empty(), "too many positional arguments for {}; run `z2o describe {}` for accepted parameters", self.op, self.op);
        if !self.kwargs.is_empty() {
            let mut keys: Vec<&str> = self.kwargs.keys().map(String::as_str).collect();
            keys.sort_unstable();
            bail!("unsupported keyword arguments for {}: [{}]; run `z2o describe {}` for accepted parameters", self.op, keys.join(", "), self.op);
        }
        Ok(())
    }
}

/// Resolve exact registered operation names. A missing operation is a hard failure, not a pass.
pub fn invoke(request: Value) -> Result<Value> {
    let mut r = request.as_object().ok_or_else(|| anyhow!("request must be an object"))?.clone();
    ensure!(r.remove("schema_version").and_then(|v| v.as_u64()) == Some(1), "schema_version must be 1");
    let op = string(&r.remove("operation").ok_or_else(|| anyhow!("operation required"))?, "operation")?;
    let mut c = Call::new(&op, r.remove("args").unwrap_or(json!([])), r.remove("kwargs").unwrap_or(json!({})))?;
    ensure!(r.is_empty(), "unknown request fields");
    let result = match op.as_str() {
        "deterministic_stage_checks.snapshot" => {
            let p = path(&c.positional("path")?)?; c.finish()?; files::snapshot(&p)?
        }
        "deterministic_stage_checks.verify_binding" => {
            let record = c.positional("record")?;
            let actual = path(&c.positional("actual_path")?)?;
            let base = path(&c.required("base")?)?;
            c.finish()?; files::verify_binding(&record,&actual,&base)?
        }
        "deterministic_stage_checks.inspect_relative_files" => {
            let root = path(&c.positional("root")?)?;
            let paths = c.positional("relative_paths")?;
            c.finish()?; files::inspect_relative_files(&root,&paths)?
        }
        "deterministic_stage_checks.author_token" => {
            let markers = c.positional("confirmed_author_markers")?;
            let reason = c.option("unknown_reason",Value::Null);
            c.finish()?; json!(metadata::author_token(&markers,&reason)?)
        }
        "deterministic_stage_checks.collection_ancestry" => {
            let item = c.positional("item")?;
            let records = c.positional("records")?;
            let complete = c.required("pagination_complete")?.as_bool().ok_or_else(||anyhow!("pagination_complete must be bool"))?;
            c.finish()?; metadata::collection_ancestry(&item,&records,complete)?
        }
        "deterministic_stage_checks.split_frontmatter" => {
            let data = bytes(&c.positional("data")?)?;
            c.finish()?; let parts = properties::split(&data)?;
            json!([encoded_bytes(parts.prefix),encoded_bytes(parts.body),parts.payload.map(encoded_bytes)])
        }
        "deterministic_stage_checks.build_properties" => {
            let data = bytes(&c.positional("data")?)?;
            let item_type = string(&c.required("item_type")?,"item_type")?;
            let confirmed = c.required("confirmed")?;
            c.finish()?; encoded_bytes(&properties::build(&data,&item_type,&confirmed)?)
        }
        "deterministic_stage_checks.repair_properties" => {
            let data = bytes(&c.positional("data")?)?;
            c.finish()?; encoded_bytes(&properties::repair(&data)?)
        }
        "deterministic_stage_checks.audit_properties" => {
            let before = path(&c.positional("before")?)?;
            let after = path(&c.positional("after")?)?;
            let mode = string(&c.required("mode")?,"mode")?;
            let item_type = string(&c.required("item_type")?,"item_type")?;
            c.finish()?; properties::audit(&before,&after,&mode,&item_type)?
        }
        "deterministic_stage_checks.audit_unchanged_properties" => {
            let before = path(&c.positional("before")?)?;
            let after = path(&c.positional("after")?)?;
            let stage = string(&c.required("stage")?,"stage")?;
            c.finish()?; properties::audit_unchanged(&before,&after,&stage)?
        }
        "deterministic_stage_checks.audit_write_readback" => {
            let candidate = path(&c.positional("candidate")?)?;
            let written = path(&c.positional("written")?)?;
            c.finish()?; properties::audit_write(&candidate,&written)?
        }
        "deterministic_stage_checks.audit_translation_binding" => {
            let record = path(&c.positional("record_path")?)?;
            let actual = path(&c.positional("actual_file")?)?;
            let kind = string(&c.required("kind")?,"kind")?;
            c.finish()?; files::audit_translation_binding(&record,&actual,&kind)?
        }
        "deterministic_stage_checks.audit_rich_tables" => {
            let p = path(&c.positional("candidate")?)?;
            c.finish()?; tables::audit(&p)?
        }
        "format_helpers.build_naming_plan" => {
            ensure!(c.args.is_empty(),"naming plan uses keyword arguments only");
            naming::build_plan(c.kwargs)?
        }
        "format_helpers.check_local_name_conflicts" => {
            let name = string(&c.required("filename")?,"filename")?;
            let root = path(&c.required("directory")?)?;
            let sensitive = c.required("case_sensitive")?.as_bool().context("case_sensitive must be bool")?;
            let normalization = string(&c.required("unicode_normalization")?,"unicode_normalization")?;
            c.finish()?; naming::conflicts(&name,&root,sensitive,&normalization)?
        }
        "format_helpers.validate_naming_plan" => {
            let plan = c.positional("plan")?;
            let delivery = c.option("delivery_path", Value::Null);
            c.finish()?; naming::validate_plan(&plan,&delivery)?
        }
        "format_helpers.new_figure_table_anchor" => {
            let source = string(&c.required("source_token")?,"source_token")?;
            let number = string(&c.required("figure_number_token")?,"figure_number_token")?;
            let occupied = c.option("occupied_ids",json!([]));
            c.finish()?; json!(figures::new_anchor(&source,&number,&occupied)?)
        }
        "format_helpers.build_figure_block" => {
            let images = c.positional("images")?;
            let caption = string(&c.required("caption")?,"caption")?;
            let anchor = string(&c.required("anchor")?,"anchor")?;
            let nl = c.option("newline",json!("\n"));
            let nl = nl.as_str().ok_or_else(||anyhow!("newline must be LF or CRLF"))?;
            c.finish()?; json!(figures::build(&images,&caption,&anchor,nl)?)
        }
        "format_helpers.plan_supplement_figure_caption" => {
            let original = c.positional("original_number")?;
            let caption = string(&c.positional("caption")?,"caption")?;
            let display = c.option("display_number",Value::Null);
            let reason = c.option("original_number_reason",Value::Null);
            c.finish()?; figures::supplement(&original,&caption,&display,&reason)?
        }
        "source_url_inventory.scan_source_url_occurrences" | "source_url_inventory.source_url_coverage" => {
            let inventory = c.positional("inventory")?;
            let blocks = c.option("excluded_blocks",json!([]));
            let pages = c.option("excluded_pages",json!([]));
            c.finish()?;
            if op.ends_with("scan_source_url_occurrences"){source_urls::scan(&inventory,&blocks,&pages)?}
            else{source_urls::coverage(&inventory,&blocks,&pages)?}
        }
        "render_note.keys" => {
            let v=c.positional("value")?;let p=string(&c.positional("path")?,"path")?;
            let required=c.optional_positional("required",json!(""))?;let optional=c.optional_positional("optional",json!(""))?;
            let required=required.as_str().context("required must be text")?;let optional=optional.as_str().context("optional must be text")?;
            c.finish()?;render::keys(&v,&p,required,optional)?;Value::Null
        }
        "render_note.line" => {
            let v=c.positional("value")?;let p=string(&c.positional("path")?,"path")?;
            c.finish()?;json!(render::line(&v,&p)?)
        }
        "render_note.digest" | "render_note.literal" => {
            let v=c.positional("value")?;
            let s=v.as_str().context("value must be text")?;
            c.finish()?;json!(if op.ends_with(".digest"){render::digest(s)}else{render::literal(s)})
        }
        "render_note.canonical" => {
            let v=c.positional("value")?;let _p=string(&c.positional("path")?,"path")?;
            c.finish()?;json!(render::canonical(&v))
        }
        "render_note.template_fields" => {
            let v=c.positional("unit")?;c.finish()?;render::template_fields(&v)?
        }
        "render_note.content_template" => {
            let plan = c.positional("plan")?; c.finish()?;
            render::content_template(&plan)?
        }
        "render_note.render_note" => {
            let plan = c.positional("plan")?; let content = c.positional("content")?; c.finish()?;
            render::render_note(&plan,&content)
        }
        "format_helpers.build_rich_table_block" => {
            let data = c.positional("data")?;
            let anchor = string(&c.required("anchor")?,"anchor")?;
            let wrapped = c.option("wrapped",json!(true)).as_bool().context("wrapped must be bool")?;
            let nl = c.option("newline",json!("\n"));
            let nl = nl.as_str().ok_or_else(||anyhow!("newline must be LF or CRLF"))?;
            c.finish()?; json!(yaml_emit::rich_table(&data,&anchor,wrapped,nl)?)
        }
        "normalize_note.normalize_note" => {
            let text = c.positional("text")?;
            let text = text.as_str().ok_or_else(||anyhow!("text must be a UTF-8 string"))?;
            c.finish()?; normalize::normalize(text)?
        }
        "normalize_note.normalize_file" => {
            let source = path(&c.positional("source")?)?;
            let destination = path(&c.positional("destination")?)?;
            c.finish()?; normalize::normalize_file(&source,&destination)?
        }
        "validate_note.validate_note" | "validate_note.inspect_note" => {
            let text = c.positional("text")?;
            let text = text.as_str().ok_or_else(||anyhow!("text must be a UTF-8 string"))?;
            c.finish()?;
            if op.ends_with("inspect_note"){serde_json::to_value(validate::inspect(text))?}
            else{serde_json::to_value(validate::validate_note(text))?}
        }
        "validate_note.main" => {
            let p=path(&c.positional("path")?)?;c.finish()?;
            let findings=validate::validate_file(&p);
            json!({"exit_code":if findings.iter().any(|f|f.level=="error"){1}else{0},"findings":findings,"source_fidelity":"not_assessed"})
        }
        "lint_presentation.main" => {
            let p=path(&c.positional("path")?)?;c.finish()?;
            json!({"prompts":presentation::lint_file(&p)?,"source_fidelity":"not_verified"})
        }
        "validate_note.validate_file" => {
            let p = path(&c.positional("path")?)?;
            c.finish()?; serde_json::to_value(validate::validate_file(&p))?
        }
        "glossary_candidates.search" | "glossary_candidates.suggest" |
        "glossary_candidates.GlossaryIndex.search" | "glossary_candidates.GlossaryIndex.suggest" => {
            let p=path(&c.positional("glossary_path")?)?;
            let input=c.positional(if op.ends_with(".search"){"query"}else{"text"})?;
            let input=input.as_str().context("source or query must be a string")?;
            let index=glossary::Glossary::new(&p)?;
            if op.ends_with(".search"){c.finish()?;index.search(input)?}
            else{
                let count=c.option("max_family_entries",json!(8)).as_u64().context("max_family_entries must be a positive integer")?;
                let count=usize::try_from(count)?;c.finish()?;index.suggest(input,count)?
            }
        }
        "glossary_candidates.prepare_glossary_candidates" => {
            let inv=path(&c.positional("inventory_path")?)?;
            let terms=path(&c.positional("glossary_path")?)?;
            let output=path(&c.positional("output_path")?)?;c.finish()?;
            let dest=glossary::prepare(&inv,&terms,&output,|i,n|runtime_reports::throttled_progress(i,n,"source glossary search leads"))?;
            json!(path_text(&dest)?)
        }
        "note_pipeline.read_json" => {
            let p=path(&c.positional("path")?)?;c.finish()?;pipeline::read_json(&p)?
        }
        "note_pipeline.build_contract" | "note_pipeline.prepare_content_job" => {
            let plan=c.positional("plan")?;let sources=c.positional("sources")?;
            if op.ends_with("build_contract"){c.finish()?;pipeline::build_contract(&plan,&sources)?}
            else{let root=path(&c.positional("output_root")?)?;c.finish()?;pipeline::prepare(&plan,&sources,&root)?}
        }
        "note_pipeline.replay_changes" => {
            let original=c.positional("original")?;let changes=c.positional("changes")?;c.finish()?;
            json!(pipeline::replay(original.as_str().context("original must be a string")?,&changes)?)
        }
        "note_pipeline.rebase_rendering" => {
            let rendering=c.positional("rendering")?;let normalized=c.positional("normalized")?;c.finish()?;
            pipeline::rebase(&rendering,&normalized)?
        }
        "note_pipeline.process_markdown" | "note_pipeline.process_content" => {
            let (source,plan,content)=if op.ends_with("process_markdown"){
                (Some(path(&c.positional("source_path")?)?),Value::Null,Value::Null)
            }else{(None,c.positional("plan")?,c.positional("content")?)};
            let output=path(&c.positional("output_root")?)?;
            let contract=c.option("contract",Value::Null);
            let root=c.option("resource_root",Value::Null);
            let root=if root.is_null(){None}else{Some(path(&root)?)};
            let write_to=c.option("write_normalized_to",Value::Null);
            let write_to=if write_to.is_null(){None}else{Some(path(&write_to)?)};
            let strip_punct=c.option("strip_display_trailing_punctuation",json!(false)).as_bool().context("strip_display_trailing_punctuation must be boolean")?;
            let force_lf=c.option("force_lf",json!(write_to.is_some())).as_bool().context("force_lf must be boolean")?;
            let convert_tables=c.option("convert_markdown_tables",json!(false)).as_bool().context("convert_markdown_tables must be boolean")?;
            let compact=c.option("compact",json!(false)).as_bool().context("compact must be boolean")?;
            c.finish()?;
            let result=pipeline::process_with_options(source.as_deref(),&plan,&content,&output,&contract,root.as_deref(),
                pipeline::ProcessOptions{write_normalized_to:write_to.as_deref(),strip_display_trailing_punctuation:strip_punct,
                    force_lf,convert_markdown_tables:convert_tables},
                runtime_reports::throttled_progress)?;
            if compact {
                json!({"schema_version":result["schema_version"],"status":result["status"],
                    "source_fidelity":result["source_fidelity"],"summary":result["summary"],
                    "published_candidate":result["published_candidate"],"timings_ms":result["timings_ms"],
                    "receipt":path_text(&path(&result["directory"])?.join("result.json"))?})
            } else { result }
        }
        "lint_presentation.lint_presentation" => {
            let text=c.positional("text")?;c.finish()?;
            serde_json::to_value(presentation::lint(text.as_str().context("text must be UTF-8")?)?)?
        }
        "lint_presentation.lint_file" => {
            let p=path(&c.positional("path")?)?;c.finish()?;
            serde_json::to_value(presentation::lint_file(&p)?)?
        }
        "validate_delivery.check_merges" => {
            let id=string(&c.positional("identity")?,"identity")?;
            let unit=c.positional("unit")?;
            let data=c.positional("data")?;c.finish()?;
            grid::report(&id,if unit.is_null(){None}else{Some(&unit)},&data)
        }
        "accept_note.accept_note" => {
            let candidate=path(&c.positional("candidate_path")?)?;
            let contract=c.optional_positional("contract",Value::Null)?;
            let root=c.option("resource_root",Value::Null);
            let root=if root.is_null(){None}else{Some(path(&root)?)};
            let rendering=c.option("rendering",Value::Null);c.finish()?;
            acceptance::accept(&candidate,&contract,root.as_deref(),&rendering)?
        }
        "docx_source_inventory.read_docx" | "docx_source_inventory.prepare_docx_source" => {
            let p=path(&c.positional("path")?)?;let id=string(&c.positional("source_id")?,"source_id")?;
            if op.ends_with(".read_docx"){c.finish()?;docx::read_docx(&p,&id,|i,n,label|if i%128==0||i==n{eprintln!("[{i}/{n}] {label}");})?}
            else{let directory=path(&c.positional("directory")?)?;c.finish()?;docx::prepare_docx_source(&p,&id,&directory,|i,n,label|if i%128==0||i==n{eprintln!("[{i}/{n}] {label}");})?}
        }
        "docx_source_inventory.valid_native_locator" => {
            let v=c.positional("value")?;c.finish()?;json!(docx::valid_locator(&v))
        }
        "docx_source_inventory.native_evidence_errors" | "docx_source_inventory.source_native_text" | "docx_source_inventory.native_figure_location" => {
            let source=c.positional("source")?;let location=c.positional("location")?;c.finish()?;
            match op.as_str(){
                "docx_source_inventory.native_evidence_errors"=>json!(docx::evidence_errors(&source,&location)),
                "docx_source_inventory.source_native_text"=>json!(docx::source_text(&source,&location)?),
                _=>docx::figure_location(&source,&location)?,
            }
        }
        "docx_source_inventory.resolve_native_locator" => {
            let p=path(&c.positional("path")?)?;let location=c.positional("locator")?;c.finish()?;
            json!(docx::NativeDocument::new(&p)?.resolve(&location)?)
        }
        "prepare_translation_review.bind_review_file" | "prepare_translation_review.bind_review_evidence" => {
            let p=path(&c.positional("path")?)?;
            if op.ends_with("bind_review_file"){c.finish()?;translation_prepare::bind_file(&p)?}
            else{let loc=c.positional("locator")?;c.finish()?;translation_prepare::bind_evidence(&p,&loc)?}
        }
        "prepare_translation_review.new_occurrence" => {
            let id=string(&c.positional("occurrence_id")?,"occurrence_id")?;
            let kind=string(&c.positional("kind")?,"kind")?;
            let span=c.required("source_span")?;c.finish()?;translation_prepare::occurrence(&id,&kind,&span)?
        }
        "prepare_translation_review.new_term_candidate" => {
            let id=string(&c.positional("term_id")?,"term_id")?;
            let original=string(&c.positional("original")?,"original")?;
            let category=string(&c.positional("category")?,"category")?;
            let excerpt=string(&c.required("source_excerpt")?,"source_excerpt")?;
            let loc=c.required("source_locator")?;c.finish()?;translation_prepare::term(&id,&original,&category,&excerpt,&loc)?
        }
        "prepare_translation_review.expand_source_locations" => {
            let unit=c.positional("unit")?;let source=c.positional("source")?;c.finish()?;
            translation_prepare::expand(&unit,&source)?
        }
        "prepare_translation_review.prepare_translation_review" => {
            let inv=path(&c.positional("inventory_path")?)?;
            let candidate=path(&c.positional("candidate_path")?)?;
            let output=path(&c.positional("output_path")?)?;
            let ids=c.required("unit_ids")?;
            let glossary=c.option("glossary_path",Value::Null);
            let glossary=if glossary.is_null(){None}else{Some(path(&glossary)?)};
            let targets=c.option("candidate_locations",Value::Null);c.finish()?;
            json!(path_text(&translation_prepare::prepare(&inv,&candidate,&output,&ids,glossary.as_deref(),&targets,
                |i,n,label|if i%128==0||i==n{eprintln!("[{i}/{n}] {label}");})?)?)
        }
        "prepare_source_inventory.prepare_docx_sources" | "prepare_source_inventory.prepare_sources" | "prepare_source_inventory.main" => {
            let sources=c.positional("source_pdfs")?;
            let root=path(&c.positional("work_directory")?)?;
            let roles=c.option("source_roles",Value::Null);
            let glossary=c.option("glossary_path",Value::Null);
            let glossary=if glossary.is_null(){None}else{Some(path(&glossary)?)};
            let dpi=c.option("dpi",json!(144)).as_u64().context("dpi must be an integer")?;
            let ocr=crate::pdf_inventory::OcrSettings::from_value(&c.option("ocr",Value::Null))?;
            c.finish()?;
            let result=if op.ends_with("prepare_docx_sources"){
                ensure!(ocr.is_none(),"DOCX-only preparation does not use OCR");
                source_prepare::prepare_docx_sources(&sources,&root,&roles,glossary.as_deref(),dpi,|i,n,label|eprintln!("[{i}/{n}] {label}"))?
            }else{
                source_prepare::prepare_sources(&sources,&root,&roles,glossary.as_deref(),dpi,ocr.as_ref(),|i,n,label|eprintln!("[{i}/{n}] {label}"))?
            };
            if op.ends_with(".main"){json!({"exit_code":0,"inventory_path":path_text(&result)?,"source_fidelity":"not_assessed"})}else{json!(path_text(&result)?)}
        }
        "check_records.bind_file" | "check_records.bind_evidence" => {
            let p=path(&c.positional("path")?)?;
            if op.ends_with("bind_file"){c.finish()?;check_ledger::bind_file(&p)?}
            else{
                let loc=c.positional("locator")?;let typ=string(&c.option("evidence_type",json!("offline_result")),"evidence_type")?;
                let obs=c.option("observation",Value::Null);c.finish()?;check_ledger::bind_evidence(&p,&loc,&typ,&obs)?
            }
        }
        "deterministic_stage_checks.coverage_catalog" => {
            let p=path(&c.positional("skills_root")?)?;c.finish()?;
            check_ledger::coverage(&p,|i,n,label|if i%128==0||i==n{eprintln!("[{i}/{n}] {label}");})?
        }
        "check_records.prepare_check_records" => {
            let root=path(&c.positional("skills_root")?)?;let output=path(&c.positional("output_path")?)?;
            let sources=c.required("sources")?;let candidates=c.option("candidates",Value::Null);
            let candidate=c.option("candidate_path",Value::Null);
            let candidate_path=if candidate.is_null(){None}else{Some(path(&candidate)?)};
            let scopes=c.option("scopes",Value::Null);let types=c.option("requirement_types",Value::Null);
            let environments=c.option("environment_kinds",Value::Null);
            let mode=string(&c.option("rule_selection",json!("acceptance")),"rule_selection")?;
            let additional=c.option("additional_ids",json!([]));
            let inventory=c.option("inventory_path",Value::Null);
            let overwrite=c.option("overwrite",json!(false));
            c.finish()?;
            let inventory=if inventory.is_null(){None}else{Some(path(&inventory)?)};
            let overwrite=overwrite.as_bool().context("overwrite must be bool")?;
            let options=check_ledger::PrepareOptions{sources,candidates,candidate_path,scopes,types,environments,mode,additional};
            json!(path_text(&check_ledger::prepare_with_inventory(&root,&output,options,inventory.as_deref(),overwrite,|i,n,label|if i%128==0||i==n{eprintln!("[{i}/{n}] {label}");})?)?)
        }
        "complete_workflow.pending_check_batch" | "complete_workflow.pending_check_rows" => {
            let p=path(&c.positional("checks_path")?)?;
            let limit=c.option("limit",json!(if op.ends_with("pending_check_batch"){20}else{40})).as_u64().context("limit must be a positive integer")?;
            let limit=usize::try_from(limit)?;
            let prefix=c.option("prefix",Value::Null);
            let prefix=if prefix.is_null(){None}else{Some(string(&prefix,"prefix")?)};
            c.finish()?;
            if op.ends_with("pending_check_batch"){check_ledger::pending_batch(&p,limit,prefix.as_deref())?}
            else{check_ledger::pending_rows(&p,limit,prefix.as_deref())?}
        }
        "check_records.CheckFinding.to_dict" | "check_records.CheckRecordsReport.offline_records_ready" |
        "check_records.CheckRecordsReport.recorded_checks_complete" | "check_records.CheckRecordsReport.structurally_consistent" |
        "check_records.CheckRecordsReport.to_dict" => {
            let p=path(&c.positional("path")?)?;
            let index=if op.contains(".CheckFinding."){Some(usize::try_from(c.required("finding_index")?.as_u64().context("finding_index must be an integer")?)?)}else{None};
            c.finish()?;let r=check_validate::validate(&p,runtime_reports::progress_line)?;
            runtime_reports::project_report(&r,op.rsplit('.').next().unwrap(),index)?
        }
        "check_records.validate_check_records" => {
            let p=path(&c.positional("path")?)?;c.finish()?;
            check_validate::validate(&p,|i,n,label|if i%128==0||i==n{eprintln!("[{i}/{n}] {label}");})?
        }
        "complete_workflow.record_check_batch" | "complete_workflow.record_decision_rows" => {
            let p=path(&c.positional("checks_path")?)?;
            let rows=c.positional(if op.ends_with("record_check_batch"){"results"}else{"rows"})?;
            let evidence=c.required("evidence")?;
            if op.ends_with("record_check_batch"){c.finish()?;check_write::record_batch(&p,&rows,&evidence)?;}
            else{let contexts=c.required("contexts")?;c.finish()?;check_write::record_rows(&p,&rows,&contexts,&evidence)?;}
            Value::Null
        }
        "complete_workflow.bind_candidate" => {
            let p=path(&c.positional("run_path")?)?;c.finish()?;check_write::bind_candidate(&p)?
        }
        "deterministic_stage_checks.record_file_checks" => {
            let checks=path(&c.positional("checks_path")?)?;let report=path(&c.positional("report_path")?)?;
            let stage=string(&c.required("stage")?,"stage")?;let after=path(&c.required("after")?)?;
            let before=c.option("before",Value::Null);
            let before=if before.is_null(){None}else{Some(path(&before)?)};
            let mode=c.option("mode",Value::Null);let item_type=c.option("item_type",Value::Null);let kind=c.option("kind",Value::Null);
            c.finish()?;
            check_write::record_file_checks(&checks,&report,check_write::AuditOptions{stage:&stage,after:&after,
                before:before.as_deref(),mode:mode.as_str(),item_type:item_type.as_str(),kind:kind.as_str()})?
        }
        "check_records.scopes_from_deliveries" => {
            let inventory=path(&c.positional("inventory_path")?)?;let candidate=path(&c.positional("candidate_path")?)?;
            let deliveries=c.positional("deliveries")?;let ids=c.required("unit_ids")?;c.finish()?;
            check_scope::from_deliveries(&inventory,&candidate,&deliveries,&ids)?
        }
        "format_helpers.starts_with_bold_source_label" => {
            let input=c.positional("text")?;c.finish()?;
            json!(input.as_str().is_some_and(figures::bold_label))
        }
        "format_helpers.apply_visible_text_edits" => {
            let file=c.required("text_file")?;let edits=c.required("edits")?;c.finish()?;
            visible_edits::apply(&file,&edits)?
        }
        "draft_queue.prepare_draft_queue" => {
            let inventory=path(&c.positional("inventory_path")?)?;let queue=path(&c.positional("queue_path")?)?;c.finish()?;
            json!(path_text(&draft_queue::prepare(&inventory,&queue,|i,n,label|if i%128==0||i==n{eprintln!("[{i}/{n}] {label}");})?)?)
        }
        "draft_queue.DraftQueue.open" => {
            let p=path(&c.positional("queue_path")?)?;c.finish()?;draft_queue::open(&p)?
        }
        "draft_queue.DraftQueue.status" | "draft_queue.DraftQueue.next_task" |
        "draft_queue.DraftQueue.review_pdf_page_order" | "draft_queue.DraftQueue.review_additional_objects" |
        "draft_queue.DraftQueue.add_page_object" | "draft_queue.DraftQueue.group" |
        "draft_queue.DraftQueue.exclude" | "draft_queue.DraftQueue.write_unit" |
        "draft_queue.DraftQueue.record_issue_decision" | "draft_queue.DraftQueue.assemble" => {
            let p=path(&c.positional("queue_path")?)?;
            let expected=c.option("expected_sha256",Value::Null);
            let action=op.rsplit('.').next().unwrap();
            let mut params=Map::new();
            let (pos,required,optional):(&[&str],&[&str],&[(&str,Value)])=match action{
                "status"|"next_task"=>(&[],&[],&[]),
                "review_pdf_page_order"=>(&["source_id","page","ordered_block_ids","reason"],&[],&[]),
                "review_additional_objects"=>(&["source_id","page","reason"],&[],&[]),
                "add_page_object"=>(&["block_id","source_id","page"],&["kind","reason"],&[("bbox",Value::Null),("text",json!(""))]),
                "group"=>(&["unit_id","block_ids"],&["kind","reason"],&[]),
                "exclude"=>(&["block_id","reason"],&[],&[]),
                "write_unit"=>(&["unit_id","markdown"],&[],&[("assets",json!([]))]),
                "record_issue_decision"=>(&["issue_id"],&["status","reason","evidence_paths"],&[]),
                "assemble"=>(&["output_path"],&[],&[]),
                _=>unreachable!(),
            };
            for name in pos{params.insert((*name).into(),c.positional(name)?);}
            for name in required{params.insert((*name).into(),c.required(name)?);}
            for (name,default) in optional{params.insert((*name).into(),c.option(name,default.clone()));}
            c.finish()?;
            draft_queue::operate(&p,action,&Value::Object(params),&expected,|i,n,label|if i%128==0||i==n{eprintln!("[{i}/{n}] {label}");})?
        }
        "validate_delivery.scan_markdown_links" => {
            let text=c.positional("text")?;c.finish()?;delivery_prepare::links(text.as_str().context("text must be a string")?)
        }
        "handoff_maps.write_handoff_maps" | "handoff_maps.validate_handoff_maps" |
        "handoff_maps.HandoffReport.as_dict" | "handoff_maps.HandoffReport.records_ready" | "handoff_maps.HandoffReport.to_dict" => {
            let inv=path(&c.positional("inventory_path")?)?;
            let candidate=path(&c.positional("candidate_path")?)?;
            let write=op=="handoff_maps.write_handoff_maps";
            let deliveries=if write{Some(c.positional("deliveries")?)}else{None};
            let paths=c.positional("map_paths")?;
            let expected=c.option("expected_source_bindings",Value::Null);c.finish()?;
            if let Some(d)=deliveries{handoff_native::write(&inv,&candidate,&d,&paths,&expected,|i,n,label|eprintln!("[{i}/{n}] {label}"))?}
            else{let r=handoff_native::validate(&inv,&candidate,&paths,&expected,runtime_reports::progress_line)?;if op.ends_with(".records_ready"){r["records_ready"].clone()}else{r}}
        }
        "mapping_rerun.record_mapping_rerun" => {
            let run=path(&c.positional("run_path")?)?;let deliveries=c.positional("deliveries")?;
            let dest=path(&c.positional("destination")?)?;c.finish()?;
            json!(path_text(&mapping_compare::record_mapping_rerun(&run,&deliveries,&dest,|i,n,label|eprintln!("[{i}/{n}] {label}"))?)?)
        }
        "handoff_maps.make_source_unit" => {
            let id=string(&c.positional("unit_id")?,"unit_id")?;let source=string(&c.positional("source_id")?,"source_id")?;
            let kind=string(&c.positional("kind")?,"kind")?;let evidence=c.positional("evidence")?;
            ensure!(c.args.is_empty(),"unexpected positional argument");delivery_prepare::unit(&id,&source,&kind,&evidence,c.kwargs)?
        }
        "validate_delivery.write_delivery_manifest" => {
            let inv=path(&c.positional("inventory_path")?)?;let candidate=path(&c.positional("candidate_path")?)?;
            let deliveries=c.positional("deliveries")?;let dest=path(&c.positional("manifest_path")?)?;
            let root=c.option("resource_root",Value::Null);let root=if root.is_null(){None}else{Some(path(&root)?)};
            let translation=c.option("translation_review_path",Value::Null);let translation=if translation.is_null(){None}else{Some(path(&translation)?)};
            let edges=c.option("figure_edge_reviews_required",json!(false)).as_bool().context("figure_edge_reviews_required must be a bool")?;
            c.finish()?;json!(path_text(&delivery_prepare::manifest(&inv,&candidate,&deliveries,&dest,root.as_deref(),translation.as_deref(),edges)?)?)
        }
        "complete_workflow.initialize_workflow" => {
            let skills=path(&c.required("skills_root")?)?;
            let work=path(&c.required("work_root")?)?;
            let sources=c.required("sources")?;
            let roles=c.required("source_roles")?;
            let candidate=path(&c.required("candidate_path")?)?;
            let resource=path(&c.required("resource_root")?)?;
            let g=c.option("glossary_path",Value::Null);
            let g=if g.is_null(){None}else{Some(path(&g)?)};
            let translation=c.option("translation_required",json!(true)).as_bool().context("translation_required must be an explicit boolean")?;
            let id=string(&c.option("run_id",json!("local-candidate")),"run_id")?;
            let ocr=crate::pdf_inventory::OcrSettings::from_value(&c.option("ocr",Value::Null))?;
            c.finish()?;
            let opt=workflow::Initialize{skills_root:&skills,work_root:&work,sources:&sources,
                source_roles:&roles,candidate_path:&candidate,resource_root:&resource,
                glossary_path:g.as_deref(),translation_required:translation,run_id:&id};
            json!(path_text(&workflow::initialize_with_ocr(opt,ocr.as_ref(),runtime_reports::throttled_progress)?)?)
        }
        "complete_workflow.CompletionReport.as_dict" | "complete_workflow.CompletionReport.records_ready" |
        "complete_workflow.CompletionReport.summary" => {
            let p=path(&c.positional("run_path")?)?;
            let count=if op.ends_with(".summary"){c.option("examples_per_code",json!(2)).as_u64().context("examples_per_code must be a positive integer")?}else{2};
            ensure!(count>0,"examples_per_code must be positive");c.finish()?;
            let r=workflow::inspect(&p,runtime_reports::throttled_progress)?;
            if op.ends_with(".records_ready"){r["records_ready"].clone()}
            else if op.ends_with(".summary"){runtime_reports::completion_summary(&r,usize::try_from(count)?)?}
            else{r}
        }
        "complete_workflow.inspect_workflow" | "complete_workflow.require_completed_records" => {
            let p=path(&c.positional("run_path")?)?;c.finish()?;
            if op.ends_with("require_completed_records"){workflow::require_completed(&p,runtime_reports::throttled_progress)?}
            else{workflow::inspect(&p,runtime_reports::throttled_progress)?}
        }
        "complete_workflow.write_maps" => {
            let p=path(&c.positional("run_path")?)?;let deliveries=c.positional("deliveries")?;c.finish()?;
            workflow::write_maps(&p,&deliveries,runtime_reports::throttled_progress)?
        }
        "complete_workflow.resume_workflow" => {
            let p=path(&c.positional("run_path")?)?;
            c.finish()?;workflow::resume(&p)?
        }
        "complete_workflow.set_naming_plan" => {
            let p=path(&c.positional("run_path")?)?;
            let plan=c.positional("naming_plan")?;
            let expected=c.option("expected_run_sha256",Value::Null);
            let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_run_sha256")?)};
            c.finish()?;
            workflow::set_naming_plan(&p,&plan,expected.as_deref())?
        }
        "complete_workflow.set_vault_write_scope" => {
            let p=path(&c.positional("run_path")?)?;
            let write=c.positional("write_vault")?.as_bool().context("write_vault must be bool")?;
            let auth=c.required("authorized")?.as_bool().context("authorized must be bool")?;
            let vp=c.option("vault_path",Value::Null);
            let vp=if vp.is_null(){None}else{Some(string(&vp,"vault_path")?)};
            let expected=c.option("expected_run_sha256",Value::Null);
            let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_run_sha256")?)};
            c.finish()?;
            workflow::set_vault_write_scope(&p,write,auth,vp.as_deref(),expected.as_deref())?
        }
        "complete_workflow.write_manifest" => {
            let run=path(&c.positional("run_path")?)?;let deliveries=c.positional("deliveries")?;c.finish()?;
            json!(path_text(&delivery_prepare::write_workflow_manifest(&run,&deliveries)?)?)
        }
        "handoff_maps.record_text_visual_conflict" => {
            let p=path(&c.positional("inventory_path")?)?;
            let unit=string(&c.positional("unit_id")?,"unit_id")?;
            let id=string(&c.required("conflict_id")?,"conflict_id")?;
            let page=c.required("page_number")?.as_u64().context("page_number must be an integer")?;
            let locator=c.required("text_locator")?;let layer=c.required("text_layer_text")?;
            let visible=c.required("visible_text")?;let bbox=c.required("bbox")?;let decision=c.required("decision")?;
            c.finish()?;
            handoff_conflicts::record(&p,&unit,handoff_conflicts::Conflict{conflict_id:&id,page_number:page,
                text_locator:&locator,text_layer_text:&layer,visible_text:&visible,bbox:&bbox,decision:&decision})?
        }
        "validate_delivery.crosswalk_legacy_pdf_text" => {
            let old=path(&c.positional("legacy_inventory_path")?)?;
            let new=path(&c.positional("native_inventory_path")?)?;
            let old_sha=string(&c.required("expected_legacy_inventory_sha256")?,"expected_legacy_inventory_sha256")?;
            let new_sha=string(&c.required("expected_native_inventory_sha256")?,"expected_native_inventory_sha256")?;
            let id=string(&c.required("source_id")?,"source_id")?;
            c.finish()?;pdf_legacy_text::crosswalk(&old,&old_sha,&new,&new_sha,&id,|i,n,label|eprintln!("[{i}/{n}] {label}"))?
        }
        "validate_delivery.replay_native_pdf_inventory" => {
            let inventory=path(&c.positional("inventory_path")?)?;
            let source_id=string(&c.required("source_id")?,"source_id")?;
            c.finish()?;
            let (raw,binding)=files::read_bound(&inventory)?;
            let inv=parse_json(std::str::from_utf8(&raw)?)?;
            let rows=inv["sources"].as_array().context("Inventory sources required")?;
            let selected:Vec<_>=rows.iter().filter(|r|r["id"]==source_id).collect();
            ensure!(selected.len()==1,"Choose one actual source ID");
            let source=selected[0];
            let p=path(&source["path"])?;
            let result=pdf_inventory::replay(source,&p,|i,n,label|eprintln!("[{i}/{n}] {label}"))?;
            ensure!(check_ledger::bind_file(&inventory)?==binding,"Source inventory changed during replay");
            result
        }
        "validate_delivery.validate_delivery" | "validate_delivery.DeliveryReport.structurally_consistent" |
        "validate_delivery.DeliveryReport.to_dict" | "validate_delivery.main" => {
            let p=path(&c.positional("manifest_path")?)?;
            let output=if op.ends_with(".main"){c.option("report_path",Value::Null)}else{Value::Null};
            let output=if output.is_null(){None}else{Some(path(&output)?)};
            c.finish()?;
            let report=delivery_validate::validate_delivery(&p,runtime_reports::progress_line)?;
            if op.ends_with(".structurally_consistent"){report["structurally_consistent"].clone()}
            else if op.ends_with(".main"){
                if let Some(out)=output{runtime_reports::save_new_report(&out,&report)?;}
                json!({"exit_code":if report["structurally_consistent"]==true{0}else{1},"report":report})
            }else{report}
        }
        "validate_delivery.check_source_provenance" => {
            let inv=path(&c.positional("inventory_path")?)?;c.finish()?;
            delivery_provenance::validate(&inv,|i,n,label|eprintln!("[{i}/{n}] {label}"))?
        }
        "mapping_rerun.compare_bound_maps" => {
            let previous=c.positional("baseline_paths")?;let current=c.positional("comparison_paths")?;
            let dest=path(&c.positional("destination")?)?;c.finish()?;
            json!(path_text(&mapping_compare::compare_bound_maps(&previous,&current,&dest)?)?)
        }
        "mapping_rerun.compare_stable_projections" => {
            let a=c.positional("baseline")?;let b=c.positional("current")?;c.finish()?;mapping_compare::compare(&a,&b)?
        }
        "prepare_source_inventory.audit_ocr_evidence" => {
            let primary=path(&c.positional("primary_record")?)?;
            let sha=string(&c.required("expected_primary_sha256")?,"expected_primary_sha256")?;
            let comparison=c.option("comparison_record",Value::Null);
            let comparison_sha=c.option("expected_comparison_sha256",Value::Null);
            let comparison=if comparison.is_null(){None}else{Some(path(&comparison)?)};
            let comparison_sha=if comparison_sha.is_null(){None}else{Some(string(&comparison_sha,"expected_comparison_sha256")?)};
            ensure!(comparison.is_some()==comparison_sha.is_some(),"Supply both comparison record and expected comparison SHA-256, or neither");
            c.finish()?;
            ocr_quality::audit(&primary,&sha,comparison.as_deref().zip(comparison_sha.as_deref()))?
        }
        "prepare_source_inventory.recognize_ocr_table_png" | "prepare_source_inventory.recognize_pdf_table_region" => {
            let package=path(&c.required("package_manifest")?)?;
            let sha=string(&c.required("expected_package_sha256")?,"expected_package_sha256")?;
            let profile=string(&c.option("profile",json!("auto")),"profile")?;
            let cell_mode=string(&c.option("cell_mode",json!("off")),"cell_mode")?;
            if op.ends_with("recognize_ocr_table_png"){
                let png=path(&c.positional("image_path")?)?;
                let expected=c.option("expected_image_sha256",Value::Null);
                let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_image_sha256")?)};
                c.finish()?;
                ocr_backend::recognize_table_png(&png,&package,&sha,&profile,&cell_mode,expected.as_deref())?
            }else{
                let source=path(&c.positional("source_pdf")?)?;
                let dest=path(&c.required("destination")?)?;
                let page=c.required("page_number")?.as_u64().context("page_number must be an integer")?;
                let bbox=c.required("bbox")?;let coords=c.required("coordinate_system")?;
                let dpi=c.option("dpi",json!(200)).as_u64().context("dpi must be an integer")?;
                let expected=c.option("expected_source_sha256",Value::Null);
                let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_source_sha256")?)};
                c.finish()?;
                let options=ocr_pdf::TableOptions{page,bbox:&bbox,coordinate_system:&coords,dpi,
                    package:&package,package_sha:&sha,profile:&profile,cell_mode:&cell_mode,expected_source_sha:expected.as_deref()};
                json!(path_text(&ocr_pdf::recognize_table_region(&source,&dest,options)?)?)
            }
        }
        "prepare_source_inventory.prepare_pdf_ocr_evidence" => {
            let source=path(&c.positional("source_pdf")?)?;
            let dest=path(&c.required("destination")?)?;
            let manifest=c.option("package_manifest",Value::Null);
            let manifest=if manifest.is_null(){None}else{Some(path(&manifest)?)};
            let package_sha=c.option("expected_package_sha256",Value::Null);
            let package_sha=if package_sha.is_null(){None}else{Some(string(&package_sha,"expected_package_sha256")?)};
            let expected=c.option("expected_source_sha256",Value::Null);
            let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_source_sha256")?)};
            let profile=string(&c.option("profile",json!("auto")),"profile")?;
            let layout=string(&c.option("layout",json!("auto")),"layout")?;
            let mode=string(&c.option("mode",json!("auto")),"mode")?;
            let dpi=c.option("dpi",json!(200)).as_u64().context("dpi must be an integer")?;
            let regions=c.option("regions",json!({}));let coordinate_system=c.option("coordinate_system",Value::Null);
            let accelerator=ocr_acceleration::Settings::from_value(&c.option("accelerator",Value::Null))?;
            c.finish()?;
            let opt=ocr_pdf::Options{package_manifest:manifest.as_deref(),package_sha:package_sha.as_deref(),
                profile:&profile,layout:&layout,mode:&mode,dpi,expected_source_sha:expected.as_deref(),
                regions:&regions,coordinate_system:&coordinate_system};
            json!(path_text(&ocr_pdf::prepare_with_accelerator(&source,&dest,opt,accelerator.as_ref(),|i,n,label|eprintln!("[{i}/{n}] {label}"))?)?)
        }
        "prepare_source_inventory.install_ocr_package" => {
            let manifest=path(&c.positional("package_manifest")?)?;
            let expected=string(&c.required("expected_package_sha256")?,"expected_package_sha256")?;
            let store=path(&c.required("store")?)?;c.finish()?;
            ocr_backend::install_package(&manifest,&expected,&store,|i,n,label|eprintln!("[{i}/{n}] install OCR resource {label}"))?
        }
        "prepare_source_inventory.ocr_hardware_plan" => {
            let profile=string(&c.option("profile",json!("auto")),"profile")?;c.finish()?;
            ocr_backend::hardware_plan(&profile)?
        }
        "prepare_source_inventory.recognize_ocr_png_accelerated" => {
            let png=path(&c.positional("image_path")?)?;
            let image_sha=string(&c.required("expected_image_sha256")?,"expected_image_sha256")?;
            let cpu=path(&c.required("package_manifest")?)?;
            let cpu_sha=string(&c.required("expected_package_sha256")?,"expected_package_sha256")?;
            let gpu=c.option("gpu_package_manifest",Value::Null);let gsha=c.option("expected_gpu_package_sha256",Value::Null);
            let gpu=if gpu.is_null(){None}else{Some(path(&gpu)?)};
            let gsha=if gsha.is_null(){None}else{Some(string(&gsha,"expected_gpu_package_sha256")?)};
            ensure!(gpu.is_some()==gsha.is_some(),"GPU package and actual manifest hash required together");
            let profile=string(&c.option("profile",json!("auto")),"profile")?;
            let layout=string(&c.option("layout",json!("auto")),"layout")?;
            let mode=string(&c.option("acceleration",json!("auto")),"acceleration")?;
            let timeout=c.option("timeout_seconds",json!(180)).as_u64().context("timeout must be integer")?;
            c.finish()?;
            ocr_acceleration::recognize(ocr_acceleration::Options{image:&png,image_sha:&image_sha,cpu:&cpu,cpu_sha:&cpu_sha,
                gpu:gpu.as_deref().zip(gsha.as_deref()),profile:&profile,layout:&layout,acceleration:&mode,timeout})?
        }
        "prepare_source_inventory.inspect_ocr_package" | "prepare_source_inventory.recognize_ocr_png" => {
            let png=if op.ends_with("recognize_ocr_png"){Some(path(&c.positional("image_path")?)?)}else{None};
            let package=path(&c.required("package_manifest")?)?;
            let sha=string(&c.required("expected_package_sha256")?,"expected_package_sha256")?;
            let profile=string(&c.option("profile",json!("auto")),"profile")?;
            let layout=string(&c.option("layout",json!("auto")),"layout")?;
            if let Some(png)=png{
                let expected=c.option("expected_image_sha256",Value::Null);
                let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_image_sha256")?)};
                c.finish()?;ocr_backend::recognize_png(&png,&package,&sha,&profile,&layout,expected.as_deref())?
            }else{c.finish()?;ocr_backend::inspect_package(&package,&sha,&profile,&layout)?}
        }
        "pdf_image_objects.render_pdfcraft_page" | "pdf_image_objects.render_pdfcraft_region" |
        "pdf_image_objects.extract_pdfcraft_text" | "prepare_source_inventory.prepare_pdfcraft_evidence" => {
            let p=path(&c.positional("source_pdf")?)?;
            let expected=c.option("expected_source_sha256",Value::Null);
            let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_source_sha256")?)};
            if op=="prepare_source_inventory.prepare_pdfcraft_evidence"{
                let root=path(&c.required("destination")?)?;
                let dpi=c.option("dpi",json!(144)).as_u64().context("dpi must be an integer")?;
                c.finish()?;
                json!(path_text(&pdfcraft_backend::prepare(&p,&root,dpi,expected.as_deref(),
                    |i,n,label|eprintln!("[{i}/{n}] {label}"))?)?)
            }else{
                let page=c.required("page_number")?.as_u64().context("page_number must be an integer")?;
                if op=="pdf_image_objects.extract_pdfcraft_text"{
                    c.finish()?;pdfcraft_backend::extract_text(&p,page,expected.as_deref())?
                }else{
                    let dpi=c.option("dpi",json!(144)).as_u64().context("dpi must be an integer")?;
                    let dest=path(&c.required("destination")?)?;
                    let overwrite=c.option("overwrite",json!(false)).as_bool().context("overwrite must be boolean")?;
                    let transparent=c.option("transparent",json!(op=="pdf_image_objects.render_pdfcraft_region"))
                        .as_bool().context("transparent must be boolean")?;
                    if op=="pdf_image_objects.render_pdfcraft_region"{
                        let bbox=c.required("bbox")?;let coordinate_system=c.required("coordinate_system")?;c.finish()?;
                        pdfcraft_backend::render_region_with_options(&p,page,&bbox,&coordinate_system,dpi,&dest,expected.as_deref(),overwrite,transparent)?
                    }else{
                        c.finish()?;pdfcraft_backend::render_page_with_options(&p,page,dpi,&dest,expected.as_deref(),overwrite,transparent)?
                    }
                }
            }
        }
        "pdf_image_objects.export_image_object" | "pdf_image_objects.main" => {
            let source=path(&c.positional("source_pdf")?)?;
            let page=c.required("page_number")?.as_u64().context("page_number must be an integer")?;
            let xref=u32::try_from(c.required("image_xref")?.as_u64().context("image_xref must be an integer")?)?;
            let dest=path(&c.required("destination")?)?;
            let e=c.option("expected_source_sha256",Value::Null);
            let e=if e.is_null(){None}else{Some(string(&e,"expected_source_sha256")?)};
            let receipt=if op.ends_with(".main"){c.option("export_record_path",Value::Null)}else{Value::Null};
            let receipt=if receipt.is_null(){None}else{Some(path(&receipt)?)};
            c.finish()?;let r=pdf_structure::export_image_object(&source,page,xref,&dest,e.as_deref())?;
            if let Some(p)=receipt{runtime_reports::save_new_report(&p,&r)?;}
            if op.ends_with(".main"){json!({"exit_code":0,"record":r})}else{r}
        }
        "pdf_image_objects.inspect_crop_boundaries" | "pdf_image_objects.render_page_region" => {
            let source=path(&c.positional("source_pdf")?)?;
            let page=c.required("page_number")?.as_u64().context("page_number must be an integer")?;
            let bbox=c.required("bbox")?;let coords=c.required("coordinate_system")?;
            let context_margin=c.option("context_margin_pt",json!(36.0)).as_f64().context("Invalid context margin")?;
            let e=c.option("expected_source_sha256",Value::Null);
            let e=if e.is_null(){None}else{Some(string(&e,"expected_source_sha256")?)};
            let overwrite=c.option("overwrite",json!(false)).as_bool().context("overwrite must be boolean")?;
            if op.ends_with("inspect_crop_boundaries"){
                let dest=path(&c.required("context_destination")?)?;let dpi=c.option("dpi",json!(144)).as_u64().context("Invalid dpi")?;
                c.finish()?;pdf_boundary::inspect_crop_boundaries_with_overwrite(&source,page,&bbox,&coords,&dest,context_margin,dpi,e.as_deref(),overwrite)?
            }else{
                let dest=path(&c.required("destination")?)?;let dpi=c.option("dpi",json!(300)).as_u64().context("Invalid dpi")?;
                let margin=c.option("margin_pt",json!(6.0)).as_f64().context("Invalid margin")?;
                let context=c.option("context_destination",Value::Null);
                let context=if context.is_null(){None}else{Some(path(&context)?)};
                let context_dpi=c.option("context_dpi",json!(144)).as_u64().context("Invalid context dpi")?;
                let transparent=c.option("transparent",json!(true)).as_bool().context("transparent must be boolean")?;
                c.finish()?;
                pdf_boundary::render_page_region(&source,pdf_boundary::RegionOptions{page,bbox:&bbox,coordinate_system:&coords,destination:&dest,
                    margin,dpi,expected:e.as_deref(),context:context.as_deref(),context_margin,context_dpi,overwrite,transparent})?
            }
        }
        "pdf_image_objects.render_page_regions_batch" => {
            let source=path(&c.positional("source_pdf")?)?;
            let regions=c.required("regions")?;
            let expected=c.option("expected_source_sha256",Value::Null);
            let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_source_sha256")?)};
            let overwrite=c.option("overwrite",json!(true)).as_bool().context("overwrite must be boolean")?;
            let cs=c.option("contact_sheet_destination",Value::Null);
            let cs=if cs.is_null(){None}else{Some(path(&cs)?)};
            let cols=c.option("contact_sheet_columns",Value::Null);
            let cols=if cols.is_null(){None}else{Some(usize::try_from(cols.as_u64().context("contact_sheet_columns must be an integer")?)?)};
            c.finish()?;
            pdf_boundary::render_page_regions_batch(&source,&regions,pdf_boundary::BatchRegionOptions{
                expected:expected.as_deref(),overwrite,contact_sheet_destination:cs.as_deref(),contact_sheet_columns:cols,
            })?
        }
        "prepare_source_inventory.pdf_page_rect" => {
            let p=path(&c.positional("source_pdf")?)?;let n=c.required("page_number")?.as_u64().context("page_number must be an integer")?;
            c.finish()?;pdf_structure::pdf_page_rect(&p,n)?
        }
        "prepare_source_inventory.valid_pdf_bbox" => {
            let b=c.positional("box")?;let page=c.positional("page_rect")?;c.finish()?;
            json!(pdf_structure::valid_pdf_bbox(&b,&page))
        }
        "check_records.AtomicRule.catalog_entry" | "check_records.AtomicRule.location" => {
            let root=path(&c.positional("skills_root")?)?;let id=string(&c.required("rule_id")?,"rule_id")?;
            c.finish()?;check_ledger::atomic_rule(&root,&id,op.ends_with(".location"),|i,n,label|eprintln!("[{i}/{n}] {label}"))?
        }
        "pdf_image_objects.ImageObjectIndexer.index_page" => {
            let source=path(&c.positional("source_pdf")?)?;
            let n=c.positional("page_number")?.as_u64().context("page_number must be an integer")?;
            let dir=c.option("object_directory",Value::Null);
            let dir=if dir.is_null(){None}else{Some(path(&dir)?)};
            let expected=c.option("expected_source_sha256",Value::Null);
            let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_source_sha256")?)};
            c.finish()?;
            pdf_structure::index_page_with_objects(&source,n,dir.as_deref(),expected.as_deref(),|i,n,label|eprintln!("[{i}/{n}] {label}"))?
        }
        "pdf_image_objects.index_page_structure" => {
            let p=path(&c.positional("source_pdf")?)?;let page=c.required("page_number")?.as_u64().context("page_number must be an integer")?;
            c.finish()?;pdf_structure::index_page(&p,page)?
        }
        "prepare_source_inventory.prepare_pdf_structure_evidence" => {
            let p=path(&c.positional("source_pdf")?)?;let dest=path(&c.positional("destination")?)?;c.finish()?;
            json!(path_text(&pdf_structure::prepare_structure(&p,&dest,|i,n,label|eprintln!("[{i}/{n}] {label}"))?)?)
        }
        "prepare_source_inventory.prepare_figure_reviews" => {
            let inventory=path(&c.positional("inventory_path")?)?;
            let assets=c.positional("final_assets")?;
            let work=path(&c.positional("work_directory")?)?;
            let root=c.option("resource_root",Value::Null);
            let root=if root.is_null(){None}else{Some(path(&root)?)};
            c.finish()?;
            json!(path_text(&source_prepare::prepare_figure_reviews(&inventory,&assets,&work,root.as_deref(),
                |i,n,label|eprintln!("[{i}/{n}] {label}"))?)?)
        }
        "prepare_source_inventory.record_figure_edges" => {
            let review=path(&c.positional("review_path")?)?;
            let unit=string(&c.positional("unit_id")?,"unit_id")?;
            let index=usize::try_from(c.positional("asset_index")?.as_u64().context("asset_index must be a nonnegative integer")?)?;
            let locations=c.required("source_location_indices")?;
            let observations=c.required("observations")?;
            c.finish()?;
            json!(path_text(&source_prepare::record_figure_edges(&review,&unit,index,&locations,&observations)?)?)
        }
        "prepare_source_inventory.file_record" | "prepare_source_inventory.sha256_file" => {
            let p=path(&c.positional("path")?)?;c.finish()?;
            let binding=check_ledger::bind_file(&p)?;
            if op.ends_with("sha256_file"){binding["sha256"].clone()}else{binding}
        }
        "validate_translation_review.validate_translation_review" |
        "validate_translation_review.TranslationReviewFinding.to_dict" |
        "validate_translation_review.TranslationReviewReport.records_ready" |
        "validate_translation_review.TranslationReviewReport.to_dict" => {
            let p=path(&c.positional("path")?)?;
            let inv=c.optional_positional("inventory_path",Value::Null)?;
            let candidate=c.optional_positional("candidate_path",Value::Null)?;
            let inv=if inv.is_null(){None}else{Some(path(&inv)?)};
            let candidate=if candidate.is_null(){None}else{Some(path(&candidate)?)};
            let index=if op.contains(".TranslationReviewFinding."){Some(usize::try_from(c.required("finding_index")?.as_u64().context("finding_index must be an integer")?)?)}else{None};
            c.finish()?;let report=translation_validate::validate(&p,inv.as_deref(),candidate.as_deref())?;
            if op.ends_with(".validate_translation_review"){report}else{runtime_reports::project_report(&report,op.rsplit('.').next().unwrap(),index)?}
        }
        "translation_review_writer.ReviewWriter.scope" |
        "translation_review_writer.ReviewWriter.exclude" |
        "translation_review_writer.ReviewWriter.source_issues_review" |
        "translation_review_writer.ReviewWriter.terminology_review" |
        "translation_review_writer.ReviewWriter.review" |
        "translation_review_writer.ReviewWriter.unit" |
        "translation_review_writer.UnitReview.check" |
        "translation_review_writer.UnitReview.scan" |
        "translation_review_writer.UnitReview.occurrence" |
        "translation_review_writer.UnitReview.term" |
        "translation_review_writer.UnitReview.issue" |
        "translation_review_writer.TermReview.search" |
        "translation_review_writer.TermReview.search_status" |
        "translation_review_writer.TermReview.decide" => {
            let prepared=path(&c.positional("prepared_path")?)?;
            let decisions=c.optional_positional("decisions",json!([]))?;
            ensure!(c.args.is_empty(),"TRW_METHOD: use named method arguments after prepared_path and decisions");
            let method=op.strip_prefix("translation_review_writer.").context("TRW_METHOD")?;
            translation_writer::call_public_method(&prepared,&decisions,method,c.kwargs)?
        }
        "translation_review_writer.select_source" | "translation_review_writer.select_candidate" |
        "translation_review_writer.append_decision" => {
            let p=path(&c.positional("prepared_path")?)?;
            let decisions=c.optional_positional("decisions",json!([]))?;
            let mut writer=translation_writer::replay(&p,&decisions)?;
            if op.ends_with("append_decision"){
                let command=c.required("command")?;c.finish()?;writer.append_decision(&command)?
            }else{
                let id=string(&c.required("unit_id")?,"unit_id")?;
                let selector=c.required("selector")?;c.finish()?;
                writer.select(&id,if op.ends_with("select_source"){"source"}else{"candidate"},&selector)?
            }
        }
        "translation_review_writer.replay_decisions" |
        "translation_review_writer.save_decision_draft" | "translation_review_writer.save_review" |
        "translation_review_tasks.unit_record_report" | "translation_review_tasks.require_unit_records" |
        "translation_review_tasks.next_review_task" => {
            let p=path(&c.positional("prepared_path")?)?;
            let decisions=c.optional_positional("decisions",json!([]))?;
            let mut writer=translation_writer::replay(&p,&decisions)?;
            match op.as_str(){
                "translation_review_writer.replay_decisions"=>{c.finish()?;json!({"data":writer.data,"prepared":writer.prepared,"decisions":writer.commands,"terminology_summary":writer.summary})}
                "translation_review_writer.save_decision_draft"=>{
                    let dest=path(&c.required("output_path")?)?;c.finish()?;json!(path_text(&writer.draft(&dest)?)?)
                }
                "translation_review_writer.save_review"=>{
                    let dest=path(&c.required("output_path")?)?;
                    let term=c.option("terminology_path",Value::Null);let term=if term.is_null(){None}else{Some(path(&term)?)};
                    c.finish()?;writer.save(&dest,term.as_deref())?
                }
                "translation_review_tasks.unit_record_report"|"translation_review_tasks.require_unit_records"=>{
                    let id=string(&c.required("unit_id")?,"unit_id")?;c.finish()?;translation_tasks::unit_report(&mut writer,&id,op.ends_with("require_unit_records"))?
                }
                _=>{
                    let limit=c.option("scope_limit",json!(8)).as_u64().context("scope_limit must be a positive integer")?;
                    c.finish()?;translation_tasks::next(&mut writer,usize::try_from(limit)?)?
                }
            }
        }
        "translation_review_writer.restore_decision_draft" => {
            let p=path(&c.positional("draft_path")?)?;
            let expected=c.option("expected_draft_sha256",Value::Null);
            let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_draft_sha256")?)};
            c.finish()?;
            let writer=translation_writer::Writer::from_draft_bound(&p,expected.as_deref())?;
            json!({"data":writer.data,"prepared":writer.prepared,"decisions":writer.commands,"terminology_summary":writer.summary})
        }
        "zotero_local.inspect_item" | "zotero_local.catalog" => {
            let kind=string(&c.positional("library_type")?,"library_type")?;
            let id=c.positional("library_id")?.as_u64().context("library_id must be an integer")?;
            let output=c.option("output_path",Value::Null);let output=if output.is_null(){None}else{Some(path(&output)?)};
            let mut report=if op.ends_with(".inspect_item"){
                let key=string(&c.positional("item_key")?,"item_key")?;
                let storage=c.option("storage_root",Value::Null);
                let storage=if storage.is_null(){None}else{Some(path(&storage)?)};
                let paths=c.option("attachment_paths",json!({}));c.finish()?;
                zotero_local::inspect_item(&kind,id,&key,storage.as_deref(),&paths,runtime_reports::progress_line)?
            }else{c.finish()?;zotero_local::catalog(&kind,id,runtime_reports::progress_line)?};
            if let Some(p)=output{let b=runtime_reports::save_new_report(&p,&report)?;report["saved_record"]=b;}
            report
        }
        "zotero_local.sync_child_note" => {
            let kind = string(&c.positional("library_type")?, "library_type")?;
            let id = c.positional("library_id")?.as_u64().context("library_id must be an integer")?;
            let parent_key = string(&c.positional("parent_key")?, "parent_key")?;
            let sync_id = string(&c.positional("sync_id")?, "sync_id")?;
            let html_path = path(&c.positional("html_path")?)?;
            let authorized = c.option("authorized", json!(false)).as_bool().context("authorized must be boolean")?;
            let version = c.option("expected_version", Value::Null);
            let expected_version = if version.is_null() { None } else {
                Some(version.as_u64().context("expected_version must be a nonnegative integer")?)
            };
            let server = c.option("expected_server_id", Value::Null);
            let expected_server_id = if server.is_null() { None } else {
                Some(string(&server, "expected_server_id")?)
            };
            c.finish()?;
            zotero_local::sync_child_note(&kind, id, &parent_key, zotero_local::NoteSyncOptions {
                sync_id: &sync_id,
                html_path: &html_path,
                authorized,
                expected_version,
                expected_server_id: expected_server_id.as_deref(),
            })?
        }
        "zotero_local.search_items" => {
            let kind=string(&c.positional("library_type")?,"library_type")?;
            let id=c.positional("library_id")?.as_u64().context("library_id must be an integer")?;
            let query=string(&c.positional("query")?,"query")?;
            let qmode=string(&c.option("qmode",json!("titleCreatorYear")),"qmode")?;
            let item_type=c.option("item_type",Value::Null);
            let item_type=if item_type.is_null(){None}else{Some(string(&item_type,"item_type")?)};
            let top_only=c.option("top_only",json!(true)).as_bool().context("top_only must be boolean")?;
            let limit=usize::try_from(c.option("limit",json!(25)).as_u64().context("limit must be a positive integer")?)?;
            let start=usize::try_from(c.option("start",json!(0)).as_u64().context("start must be a nonnegative integer")?)?;
            let output=c.option("output_path",Value::Null);
            let output=if output.is_null(){None}else{Some(path(&output)?)};
            c.finish()?;
            let mut report=zotero_local::search_items(&kind,id,zotero_local::SearchOptions{
                query:&query,qmode:&qmode,item_type:item_type.as_deref(),top_only,limit,start
            })?;
            if let Some(p)=output{let b=runtime_reports::save_new_report(&p,&report)?;report["saved_record"]=b;}
            report
        }
        "user_profile.locate" => {
            let root=c.option("skill_root",Value::Null);
            let root=if root.is_null(){None}else{Some(path(&root)?)};
            c.finish()?;
            user_profile::locate(root.as_deref())?
        }
        "user_profile.load" => {
            let root=c.option("skill_root",Value::Null);
            let root=if root.is_null(){None}else{Some(path(&root)?)};
            let verify=c.option("verify_paths",json!(true)).as_bool().context("verify_paths must be boolean")?;
            c.finish()?;
            user_profile::load(root.as_deref(),verify)?
        }
        "user_profile.save" => {
            let profile=c.positional("profile")?;
            let root=c.option("skill_root",Value::Null);
            let root=if root.is_null(){None}else{Some(path(&root)?)};
            c.finish()?;
            user_profile::save(profile,root.as_deref())?
        }
        "user_profile.patch" => {
            let patch=c.positional("patch")?;
            let root=c.option("skill_root",Value::Null);
            let root=if root.is_null(){None}else{Some(path(&root)?)};
            c.finish()?;
            user_profile::patch(patch,root.as_deref())?
        }
        "user_profile.preflight" => {
            let root=c.option("skill_root",Value::Null);
            let root=if root.is_null(){None}else{Some(path(&root)?)};
            let query=c.option("query",Value::Null);
            let query=if query.is_null(){None}else{Some(string(&query,"query")?)};
            let key=c.option("item_key",Value::Null);
            let key=if key.is_null(){None}else{Some(string(&key,"item_key")?)};
            let storage=c.option("zotero_storage_dir",Value::Null);
            let storage=if storage.is_null(){None}else{Some(path(&storage)?)};
            let create_work_dirs=c.option("create_work_dirs",json!(false)).as_bool().context("create_work_dirs must be boolean")?;
            c.finish()?;
            user_profile::preflight(root.as_deref(),query.as_deref(),key.as_deref(),storage.as_deref(),create_work_dirs)?
        }
        "prepare_source_inventory.inspect_tex_source" => {
            let source=path(&c.positional("source_path")?)?;
            let id=string(&c.required("source_id")?,"source_id")?;
            let expected=c.option("expected_source_sha256",Value::Null);
            let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_source_sha256")?)};
            c.finish()?;
            tex_source::inspect(&source,&id,expected.as_deref())?
        }
        "prepare_source_inventory.prepare_tex_source" => {
            let source=path(&c.positional("source_path")?)?;
            let destination=path(&c.positional("destination")?)?;
            let id=string(&c.required("source_id")?,"source_id")?;
            let expected=c.option("expected_source_sha256",Value::Null);
            let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_source_sha256")?)};
            c.finish()?;
            tex_source::prepare(&source,&id,&destination,expected.as_deref())?
        }
        "prepare_source_inventory.fetch_arxiv_tex" => {
            let target=c.positional("target")?;
            let destination=path(&c.positional("destination")?)?;
            let id=c.option("source_id",json!("main"));
            let id=string(&id,"source_id")?;
            let pdf=c.option("pdf_path",Value::Null);
            let pdf=if pdf.is_null(){None}else{Some(path(&pdf)?)};
            let timeout=c.option("timeout_seconds",json!(60)).as_u64().context("timeout_seconds must be a positive integer")?;
            let compact=c.option("compact",json!(true)).as_bool().context("compact must be boolean")?;
            c.finish()?;
            tex_source::fetch_and_prepare(&target,&destination,&id,pdf.as_deref(),timeout,compact)?
        }
        "prepare_source_inventory.inspect_pdf_quality" => {
            let source=path(&c.positional("source_pdf")?)?;
            let expected=c.option("expected_source_sha256",Value::Null);
            let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_source_sha256")?)};
            c.finish()?;
            pdf_inspector_bridge::inspect_pdf_file(&source,expected.as_deref())?
        }
        "prepare_source_inventory.prepare_pdf_layout" => {
            let source=path(&c.positional("source_path")?)?;
            let destination=path(&c.positional("destination")?)?;
            let id=c.option("source_id",json!("main"));
            let id=string(&id,"source_id")?;
            let render=c.option("render_pages",json!("selective"));
            let render=string(&render,"render_pages")?;
            let dpi=c.option("dpi",json!(150)).as_u64().context("dpi must be a positive integer")?;
            let expected=c.option("expected_source_sha256",Value::Null);
            let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_source_sha256")?)};
            c.finish()?;
            pdf_layout::prepare_pdf_layout(&source,&destination,&id,&render,dpi,expected.as_deref())?
        }
        "prepare_source_inventory.prepare_paper" => {
            let work_dir=path(&c.positional("work_dir")?)?;
            let pdf=c.option("pdf_path",Value::Null);
            let pdf=if pdf.is_null(){None}else{Some(path(&pdf)?)};
            let tex=c.option("tex_target",Value::Null);
            let tex_opt=if tex.is_null(){None}else{Some(&tex)};
            let glossary=c.option("glossary_path",Value::Null);
            let glossary=if glossary.is_null(){None}else{Some(path(&glossary)?)};
            let render=c.option("render_pages",json!("auto"));
            let render=string(&render,"render_pages")?;
            let dpi=c.option("dpi",json!(150)).as_u64().context("dpi must be a positive integer")?;
            let expected=c.option("expected_pdf_sha256",Value::Null);
            let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_pdf_sha256")?)};
            let timeout=c.option("arxiv_timeout_seconds",json!(60)).as_u64().context("arxiv_timeout_seconds must be an integer")?;
            c.finish()?;
            pdf_layout::prepare_paper(pdf_layout::PreparePaperOptions{
                pdf_path:pdf.as_deref(),tex_target:tex_opt,work_dir:&work_dir,
                glossary_path:glossary.as_deref(),render_pages:&render,dpi,
                expected_pdf_sha256:expected.as_deref(),arxiv_timeout_seconds:timeout,
            })?
        }
        "vault_library.index" => {
            let root=path(&c.positional("vault_root")?)?;
            let lit=string(&c.positional("literature_relative")?,"literature_relative")?;
            let exclude=c.option("excluded_relative",json!([]));
            let id=c.option("personal_library_id",Value::Null);
            let id=if id.is_null(){None}else{Some(id.as_u64().context("personal_library_id must be an actual integer ID")?)};
            let output=c.option("output_path",Value::Null);let output=if output.is_null(){None}else{Some(path(&output)?)};
            let use_cache=c.option("use_cache",json!(false)).as_bool().context("use_cache must be boolean")?;
            c.finish()?;
            if use_cache{
                let out=output.as_ref().context("output_path required when use_cache=true")?;
                let r=vault_library::index_cached(&root,&lit,&exclude,id,out,runtime_reports::progress_line)?;
                if r["cache_hit"]==true{r}else{
                    let mut saved=r;
                    pipeline::ensure_outside_vault(out)?;
                    if let Some(p)=out.parent().filter(|p|!p.as_os_str().is_empty()){std::fs::create_dir_all(p)?;}
                    let mut bytes=serde_json::to_vec_pretty(&saved)?;
                    bytes.push(b'\n');
                    pipeline::publish_replace(out,&bytes)?;
                    saved["saved_record"]=check_ledger::bind_file(out)?;
                    saved
                }
            }else{
                let mut r=vault_library::index(&root,&lit,&exclude,id,runtime_reports::progress_line)?;
                if let Some(p)=output{let b=runtime_reports::save_new_report(&p,&r)?;r["saved_record"]=b;}
                r
            }
        }
        "vault_library.match_reference" => {
            let index=path(&c.positional("index_path")?)?;let ih=string(&c.required("expected_index_sha256")?,"expected_index_sha256")?;
            let catalog=path(&c.positional("catalog_path")?)?;let ch=string(&c.required("expected_catalog_sha256")?,"expected_catalog_sha256")?;
            let reference=c.positional("reference")?;c.finish()?;
            vault_library::match_reference(&index,&ih,&catalog,&ch,&reference)?
        }
        "vault_library.match_references_batch" => {
            let index=path(&c.positional("index_path")?)?;
            let catalog=path(&c.positional("catalog_path")?)?;
            let references=c.positional("references")?;
            let ih=c.option("expected_index_sha256",Value::Null);
            let ih=if ih.is_null(){None}else{Some(string(&ih,"expected_index_sha256")?)};
            let ch=c.option("expected_catalog_sha256",Value::Null);
            let ch=if ch.is_null(){None}else{Some(string(&ch,"expected_catalog_sha256")?)};
            c.finish()?;
            vault_library::match_references_batch(&index,ih.as_deref(),&catalog,ch.as_deref(),&references)?
        }
        "vault_publish.commit_markdown" => {
            let vault=path(&c.positional("vault_root")?)?;
            let backup=path(&c.required("backup_root")?)?;
            let literature=string(&c.required("literature_relative")?,"literature_relative")?;
            let assets=string(&c.required("assets_relative")?,"assets_relative")?;
            let note=string(&c.required("note_relative")?,"note_relative")?;
            let candidate=path(&c.required("candidate_path")?)?;
            let sha=string(&c.required("expected_candidate_sha256")?,"expected_candidate_sha256")?;
            let prior=c.required("expected_existing_sha256")?;
            let prior=if prior.is_null(){None}else{Some(string(&prior,"expected_existing_sha256")?)};
            let resources=c.required("resources")?;
            let scope=string(&c.required("scope")?,"scope")?;
            let authorized=c.required("authorized")?.as_bool().context("authorized must be boolean")?;
            let run=c.option("workflow_run",Value::Null);let run_sha=c.option("expected_run_sha256",Value::Null);
            let run=if run.is_null(){None}else{Some(path(&run)?)};
            let run_sha=if run_sha.is_null(){None}else{Some(string(&run_sha,"expected_run_sha256")?)};
            ensure!(run.is_some()==run_sha.is_some(),"Run and hash required together");c.finish()?;
            vault_publish::commit(vault_publish::CommitOptions{vault:&vault,backup_root:&backup,literature_relative:&literature,
                assets_relative:&assets,note_relative:&note,candidate:&candidate,candidate_sha:&sha,expected_existing_sha:prior.as_deref(),
                resources:&resources,authorized,scope:&scope,workflow_run:run.as_deref().zip(run_sha.as_deref())},runtime_reports::progress_line)?
        }
        "vault_publish.rollback" => {
            let p=path(&c.positional("transaction_path")?)?;
            let sha=string(&c.required("expected_transaction_sha256")?,"expected_transaction_sha256")?;
            let authorized=c.required("authorized")?.as_bool().context("authorized must be boolean")?;
            c.finish()?;vault_publish::rollback(&p,&sha,authorized)?
        }
        "vault_publish.plan" => {
            let run=path(&c.positional("run_path")?)?;
            let run_sha=string(&c.required("expected_run_sha256")?,"expected_run_sha256")?;
            let vault=path(&c.required("vault_root")?)?;
            let literature=string(&c.required("literature_relative")?,"literature_relative")?;
            let assets=string(&c.required("assets_relative")?,"assets_relative")?;
            let note=string(&c.required("note_relative")?,"note_relative")?;
            let expected=c.required("expected_existing_sha256")?;
            let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_existing_sha256")?)};
            let inspect=c.option("inspect_workflow",json!(true)).as_bool().context("inspect_workflow must be boolean")?;
            c.finish()?;
            vault_publish::plan(vault_publish::Options{run:&run,run_sha:&run_sha,vault:&vault,literature_relative:&literature,
                assets_relative:&assets,note_relative:&note,expected_existing_sha:expected.as_deref(),inspect_workflow:inspect},runtime_reports::progress_line)?
        }
        "vault_publish.plan_standalone" => {
            let vault=path(&c.positional("vault_root")?)?;
            let literature=string(&c.required("literature_relative")?,"literature_relative")?;
            let assets=string(&c.required("assets_relative")?,"assets_relative")?;
            let note=string(&c.required("note_relative")?,"note_relative")?;
            let candidate=path(&c.required("candidate_path")?)?;
            let resource_root=path(&c.required("resource_root")?)?;
            let asset_src=c.option("asset_source_dir",Value::Null);
            let asset_src=if asset_src.is_null(){None}else{Some(path(&asset_src)?)};
            let backup=c.option("backup_root",Value::Null);
            let backup=if backup.is_null(){None}else{Some(path(&backup)?)};
            let expected=c.option("expected_existing_sha256",Value::Null);
            let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_existing_sha256")?)};
            let reuse_identical=c.option("reuse_identical_assets",json!(true)).as_bool().context("reuse_identical_assets must be boolean")?;
            let disambiguate=c.option("disambiguate_conflicts",json!(true)).as_bool().context("disambiguate_conflicts must be boolean")?;
            let req_out=c.option("commit_request_output",Value::Null);
            let req_out=if req_out.is_null(){None}else{Some(path(&req_out)?)};
            c.finish()?;
            vault_publish::plan_standalone(vault_publish::StandalonePlanOptions{
                vault:&vault,literature_relative:&literature,assets_relative:&assets,note_relative:&note,
                candidate:&candidate,resource_root:&resource_root,asset_source_dir:asset_src.as_deref(),
                backup_root:backup.as_deref(),
                expected_existing_sha:expected.as_deref(),reuse_identical_assets:reuse_identical,
                disambiguate_conflicts:disambiguate,commit_request_output:req_out.as_deref(),
            },runtime_reports::throttled_progress)?
        }
        "vault_publish.publish_standalone" => {
            let vault=path(&c.positional("vault_root")?)?;
            let backup=path(&c.required("backup_root")?)?;
            let literature=string(&c.required("literature_relative")?,"literature_relative")?;
            let assets=string(&c.required("assets_relative")?,"assets_relative")?;
            let note=string(&c.required("note_relative")?,"note_relative")?;
            let candidate=path(&c.required("candidate_path")?)?;
            let resource_root=path(&c.required("resource_root")?)?;
            let asset_src=c.option("asset_source_dir",Value::Null);
            let asset_src=if asset_src.is_null(){None}else{Some(path(&asset_src)?)};
            let expected=c.option("expected_existing_sha256",Value::Null);
            let expected=if expected.is_null(){None}else{Some(string(&expected,"expected_existing_sha256")?)};
            let reuse_identical=c.option("reuse_identical_assets",json!(true)).as_bool().context("reuse_identical_assets must be boolean")?;
            let disambiguate=c.option("disambiguate_conflicts",json!(true)).as_bool().context("disambiguate_conflicts must be boolean")?;
            let authorized=c.required("authorized")?.as_bool().context("authorized must be boolean")?;
            let rep_out=c.option("report_output",Value::Null);
            let rep_out=if rep_out.is_null(){None}else{Some(path(&rep_out)?)};
            c.finish()?;
            vault_publish::publish_standalone(vault_publish::PublishStandaloneOptions{
                vault:&vault,backup_root:&backup,literature_relative:&literature,assets_relative:&assets,
                note_relative:&note,candidate:&candidate,resource_root:&resource_root,asset_source_dir:asset_src.as_deref(),
                expected_existing_sha:expected.as_deref(),reuse_identical_assets:reuse_identical,
                disambiguate_conflicts:disambiguate,authorized,report_output:rep_out.as_deref(),
            },runtime_reports::progress_line)?
        }
        "native_runtime.package" => {
            let source=path(&c.positional("source_root")?)?;let dest=path(&c.positional("destination")?)?;
            let ocr_bundle=c.option("ocr_bundle",Value::Null);
            let ocr_bundle=if ocr_bundle.is_null(){None}else{Some(path(&ocr_bundle)?)};
            c.finish()?;native_install::package_with_ocr(&source,&dest,ocr_bundle.as_deref(),runtime_reports::progress_line)?
        }
        "native_runtime.inspect_bundle" | "native_runtime.install" | "native_runtime.verify_install" => {
            let manifest=path(&c.positional("package_manifest")?)?;
            let sha=string(&c.positional("expected_package_sha256")?,"expected_package_sha256")?;
            if op=="native_runtime.inspect_bundle"{c.finish()?;native_install::inspect_bundle(&manifest,&sha)?}
            else{
                let root=path(&c.positional("skills_root")?)?;c.finish()?;
                if op=="native_runtime.verify_install"{native_install::verify_install(&manifest,&sha,&root)?}
                else{native_install::install(&manifest,&sha,&root,runtime_reports::progress_line)?}
            }
        }
        "draft_queue.progress_bar" |
        "handoff_maps.progress_bar" |
        "note_pipeline.progress_bar" |
        "pdf_image_objects.progress_bar" |
        "prepare_source_inventory.progress_bar" |
        "prepare_translation_review.progress_bar" => {
            let done=usize::try_from(c.positional("done")?.as_u64().context("done must be a nonnegative integer")?)?;
            let total=usize::try_from(c.positional("total")?.as_u64().context("total must be a nonnegative integer")?)?;
            let label=c.positional("label")?;
            let label=label.as_str().context("label must be a string")?;
            c.finish()?;ensure!(done<=total,"Progress exceeds declared total");
            runtime_reports::progress_line(done,total,label);Value::Null
        }
        _ => {
            if let Err(e) = describe_operation(&op) {
                return Err(e);
            }
            bail!("UNSUPPORTED_OPERATION: {op}");
        }
    };
    Ok(json!({"schema_version": SCHEMA_VERSION, "result": result}))
}

pub fn decision(id: &str, value: Option<bool>, reason: &str) -> Value {
    json!({"id":id, "status":match value {Some(true)=>"passed",Some(false)=>"failed",None=>"not_verified"},"reason":reason})
}
