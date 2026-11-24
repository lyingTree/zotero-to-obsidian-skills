//! Native implementation. No embedded Python, subprocess fallback or model
//! self-certification. Unsupported operations fail explicitly.
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
    "accept_note.accept_note",
    "docx_source_inventory.read_docx",
    "docx_source_inventory.prepare_docx_source",
    "docx_source_inventory.valid_native_locator",
    "docx_source_inventory.native_evidence_errors",
    "docx_source_inventory.source_native_text",
    "docx_source_inventory.resolve_native_locator",
    "docx_source_inventory.native_figure_location",
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
    let obj = v.as_object().ok_or_else(|| anyhow!("expected $bytes wrapper"))?;
    ensure!(obj.len() == 1, "invalid $bytes wrapper");
    Ok(STANDARD.decode(obj.get("$bytes").and_then(Value::as_str)
        .ok_or_else(|| anyhow!("expected base64 $bytes string"))?)?)
}

pub fn encoded_bytes(v: &[u8]) -> Value { json!({"$bytes": STANDARD.encode(v)}) }

pub fn path_text(p: &Path) -> Result<String> {
    p.to_str().map(str::to_owned).ok_or_else(|| anyhow!("non-Unicode path: {p:?}"))
}

pub fn describe() -> Value {
    json!({
        "schema_version": SCHEMA_VERSION, "engine": "rust-native", "engine_version": ENGINE_VERSION,
        "reference_version": REFERENCE_VERSION, "platform": std::env::consts::OS,
        "architecture": std::env::consts::ARCH, "system_python_required": false,
        "python_fallback": false, "full_workflow_ready": false, "operations": OPERATIONS,
        "release_blockers": ["PDF provenance and visual crop parity", "native DOCX inventory parity",
            "translation review and source/delivery mapping parity", "full Markdown rendering and normalization parity",
            "native installer and Zotero/Obsidian integration parity", "all historical regression tests and platform qualification"],
    })
}

struct Call { args: VecDeque<Value>, kwargs: Map<String, Value> }
impl Call {
    fn new(args: Value, kwargs: Value) -> Result<Self> {
        let args = args.as_array().ok_or_else(|| anyhow!("args must be an array"))?.clone();
        let kwargs = kwargs.as_object().ok_or_else(|| anyhow!("kwargs must be an object"))?.clone();
        Ok(Self { args: args.into(), kwargs })
    }
    fn positional(&mut self, name: &str) -> Result<Value> {
        match self.args.pop_front() {
            Some(v) => { ensure!(!self.kwargs.contains_key(name), "duplicate argument: {name}"); Ok(v) }
            None => self.required(name),
        }
    }
    fn required(&mut self, name: &str) -> Result<Value> {
        self.kwargs.remove(name).ok_or_else(|| anyhow!("missing argument: {name}"))
    }
    fn optional_positional(&mut self, name: &str, default: Value) -> Result<Value> {
        match self.args.pop_front() {
            Some(v) => { ensure!(!self.kwargs.contains_key(name), "duplicate argument: {name}"); Ok(v) }
            None => Ok(self.option(name, default)),
        }
    }
    fn option(&mut self, name: &str, default: Value) -> Value { self.kwargs.remove(name).unwrap_or(default) }
    fn finish(self) -> Result<()> {
        ensure!(self.args.is_empty(), "too many positional arguments");
        ensure!(self.kwargs.is_empty(), "unsupported keyword arguments: {:?}", self.kwargs.keys());
        Ok(())
    }
}

/// Resolve exact registered operation names; no eval, arbitrary imports, or
/// Python subprocess. A missing operation is a hard failure, not a pass.
pub fn invoke(request: Value) -> Result<Value> {
    let mut r = request.as_object().ok_or_else(|| anyhow!("request must be an object"))?.clone();
    ensure!(r.remove("schema_version").and_then(|v| v.as_u64()) == Some(1), "schema_version must be 1");
    let op = string(&r.remove("operation").ok_or_else(|| anyhow!("operation required"))?, "operation")?;
    let mut c = Call::new(r.remove("args").unwrap_or(json!([])), r.remove("kwargs").unwrap_or(json!({})))?;
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
        "validate_note.validate_file" => {
            let p = path(&c.positional("path")?)?;
            c.finish()?; serde_json::to_value(validate::validate_file(&p))?
        }
        "glossary_candidates.search" | "glossary_candidates.suggest" => {
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
            let dest=glossary::prepare(&inv,&terms,&output,|i,n|if i%128==0||i==n{eprintln!("[{i}/{n}] source glossary search leads");})?;
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
            c.finish()?;
            pipeline::process(source.as_deref(),&plan,&content,&output,&contract,root.as_deref(),|done,total,label|eprintln!("[{done}/{total}] {label}"))?
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
        _ => bail!("UNMIGRATED_OPERATION: {op}; no Python fallback"),
    };
    Ok(json!({"schema_version": SCHEMA_VERSION, "result": result}))
}

pub fn decision(id: &str, value: Option<bool>, reason: &str) -> Value {
    json!({"id":id, "status":match value {Some(true)=>"passed",Some(false)=>"failed",None=>"not_verified"},"reason":reason})
}
