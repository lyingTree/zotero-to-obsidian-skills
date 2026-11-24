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
pub mod source_urls;

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
        _ => bail!("UNMIGRATED_OPERATION: {op}; no Python fallback"),
    };
    Ok(json!({"schema_version": SCHEMA_VERSION, "result": result}))
}

pub fn decision(id: &str, value: Option<bool>, reason: &str) -> Value {
    json!({"id":id, "status":match value {Some(true)=>"passed",Some(false)=>"failed",None=>"not_verified"},"reason":reason})
}
