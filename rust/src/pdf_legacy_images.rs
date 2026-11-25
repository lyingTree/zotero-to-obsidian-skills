//! Mechanical correspondence of plain indirect image calls in already bound inventories.
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const ROUNDING_PT: f64 = 0.0001;
const EMPTY: &[Value] = &[];
type Index<'a> = BTreeMap<u64, Vec<&'a Value>>;

fn xref(v: &Value) -> Option<u64> {
    v["image_xref"].as_u64().filter(|x| (1..=u32::MAX.into()).contains(x))
}

fn generation(v: &Value) -> Option<u64> {
    v["generation"].as_u64().filter(|g| *g <= u16::MAX.into())
}

fn sha(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| {
        s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn empty(v: &Value) -> bool {
    v.as_array().is_some_and(Vec::is_empty)
}

fn points_agree(a: &Value, b: &Value, len: usize) -> bool {
    match (a.as_array(), b.as_array()) {
        (Some(a), Some(b)) if a.len() == len && b.len() == len => {
            a.iter().zip(b).all(|(a, b)| match (a.as_f64(), b.as_f64()) {
                (Some(a), Some(b)) => (a - b).abs() <= ROUNDING_PT,
                _ => false,
            })
        }
        _ => false,
    }
}

fn grouped(rows: &[Value]) -> Index<'_> {
    let mut map = Index::new();
    for row in rows {
        if let Some(x) = xref(row) {
            map.entry(x).or_default().push(row);
        }
    }
    map
}

fn ordinal(v: &Value) -> Option<u64> {
    v["occurrence"].as_u64().filter(|x| *x > 0)
}

fn plain(v: &Value) -> bool {
    empty(&v["form_xrefs"])
        && empty(&v["form_clips"])
        && empty(&v["context_effects"])
        && v["fully_inside_page"] == true
        && v["visibility"] == "not_assessed"
        && ordinal(v).is_some()
}

pub(crate) struct LegacyImages<'a> {
    objects: Index<'a>,
    // Legacy object records are keyed only by xref. An independently recorded
    // invocation is the sole historical source of the generation.
    generations: BTreeMap<u64, BTreeSet<Option<u64>>>,
}

impl<'a> LegacyImages<'a> {
    pub(crate) fn new(source: &'a Value, pages: &'a [Value]) -> Self {
        let objects = grouped(source["image_objects"].as_array().map_or(EMPTY, Vec::as_slice));
        let mut generations = BTreeMap::<_, BTreeSet<_>>::new();
        for page in pages {
            if let Some(rows) = page["image_index"]["occurrences"].as_array() {
                for row in rows {
                    if let Some(x) = xref(row) {
                        generations.entry(x).or_default().insert(generation(row));
                    }
                }
            }
        }
        Self { objects, generations }
    }

    pub(crate) fn report(&self, old: &Value, new: &Value) -> Value {
        let a = &old["image_index"];
        let b = &new["image_index"];
        let report = |status: &str, reason: &str, rows: Vec<Value>| json!({
            "kind": "pdf-legacy-image-correspondence/v1", "status": status, "reason": reason,
            "rows": rows, "figure_visibility": "not_assessed", "visual": "pending",
            "human_review": "not_transferred"
        });
        if a["status"] != "indexed" || b["status"] != "structurally_indexed"
            || [a, b].into_iter().any(|p| !empty(&p["issues"]) || !empty(&p["skipped_forms"])
                || p["bbox_meaning"] != "full_object_bounds_before_clipping")
        {
            return report("pending", "page_image_index_missing_or_incomplete", vec![]);
        }
        let coords = json!({"space":"pdf_page","unit":"pt","origin":"top_left","x_axis":"right","y_axis":"down","rotation":"unrotated"});
        if a["coordinate_system"] != coords || b["coordinate_system"] != coords
            || !a["page_rotation"].as_u64().is_some_and(|r| matches!(r, 0 | 90 | 180 | 270))
            || a["page_rotation"] != b["page_rotation"]
            || !points_agree(&a["page_bounds"], &b["page_bounds"], 4)
            || !points_agree(&a["pdf_to_page_transform"], &b["pdf_to_page_transform"], 6)
        {
            return report("pending", "page_coordinate_frames_do_not_match", vec![]);
        }
        let (Some(ar), Some(br), Some(bo)) = (
            a["occurrences"].as_array(), b["occurrences"].as_array(), b["objects"].as_array()
        ) else { return report("pending", "image_records_missing", vec![]) };
        for occurrences in [ar, br] {
            let mut ordinals = BTreeSet::new();
            if occurrences.iter().any(|r| xref(r).is_none()
                || !ordinal(r).is_some_and(|o| ordinals.insert(o)))
            {
                return report("pending", "unidentified_or_duplicate_image_selectors", vec![]);
            }
        }
        let (ai, bi, bo) = (grouped(ar), grouped(br), grouped(bo));
        let xs = ai.keys().chain(bi.keys()).copied().collect::<BTreeSet<_>>();
        let mut rows = Vec::with_capacity(xs.len());
        let mut pending = false;
        for x in xs {
            let or = ai.get(&x).map_or(&[][..], Vec::as_slice);
            let nr = bi.get(&x).map_or(&[][..], Vec::as_slice);
            let result = self.compare(or, nr, bo.get(&x).map_or(&[][..], Vec::as_slice), x);
            rows.push(match result {
                Ok(v) => v,
                Err(reason) => {
                    pending = true;
                    json!({"image_xref": x, "status": "pending", "reason": reason,
                        "old_occurrences": or.iter().filter_map(|r| ordinal(r)).collect::<Vec<_>>(),
                        "native_occurrences": nr.iter().filter_map(|r| ordinal(r)).collect::<Vec<_>>()})
                }
            });
        }
        if rows.is_empty() {
            report("no_image_invocations", "both_complete_indexes_are_empty", rows)
        } else if pending {
            report("pending", "one_or_more_image_correspondences_pending", rows)
        } else {
            report("mechanical_image_correspondence", "plain_indirect_image_calls_only", rows)
        }
    }

    fn compare(&self, old: &[&Value], new: &[&Value], actual: &[&Value], x: u64) -> Result<Value, &'static str> {
        let ([old], [new]) = (old, new) else { return Err("invocation_missing_repeated_or_ambiguous") };
        let g = generation(old).ok_or("invocation_generation_missing")?;
        if generation(new) != Some(g)
            || !self.generations.get(&x).is_some_and(|s| s.len() == 1 && s.contains(&Some(g)))
        {
            return Err("image_generation_differs_or_is_ambiguous");
        }
        let [historical] = self.objects.get(&x).map_or(&[][..], Vec::as_slice)
            else { return Err("legacy_raw_stream_record_missing_or_ambiguous") };
        let [actual] = actual else { return Err("native_raw_stream_record_missing_or_ambiguous") };
        let hash = sha(&historical["pdf_stream_sha256"]).ok_or("legacy_raw_stream_hash_missing")?;
        if historical["status"] != "extracted" || actual["status"] != "raw_stream_indexed"
            || generation(actual) != Some(g)
            || (historical.get("generation").is_some() && generation(historical) != Some(g))
            || sha(&actual["encoded_stream_sha256"]) != Some(hash)
        {
            return Err("raw_stream_identity_differs_or_is_unavailable");
        }
        if historical["smask"]["status"] != "absent" || historical["smask"]["image_xref"] != 0
            || historical["image_mask"] != false
            || ["explicit_mask", "decode", "smask_in_data"].into_iter().any(|k| historical.get(k) != Some(&Value::Null))
            || !actual["mask"].is_null() || !actual["smask"].is_null()
            || !plain(old) || !plain(new)
        {
            return Err("image_dependencies_or_display_context_require_review");
        }
        let (Some(name), Some(container), Some(operation)) = (
            old["resource_name"].as_str().filter(|s| {
                s.strip_prefix('/').is_some_and(|name| !name.is_empty()
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)))
            }),
            old["container_xref"].as_u64().filter(|n| (1..=u32::MAX.into()).contains(n)),
            old["operation_index"].as_u64(),
        ) else { return Err("normalized_locator_missing") };
        if new["resource_name"] != name || new["container_xref"].as_u64() != Some(container)
            || new["operation_index"].as_u64() != Some(operation)
            || old["resource_path"] != json!([name]) || new["resource_path"] != json!([name])
            || !points_agree(&old["bbox"], &new["bbox"], 4)
            || !points_agree(&old["transform"], &new["transform"], 6)
        {
            return Err("normalized_locator_differs");
        }
        Ok(json!({
            "status": "mechanical_image_correspondence", "image_xref": x, "generation": g,
            "pdf_raw_stream_sha256": hash, "old_occurrence": old["occurrence"],
            "native_occurrence": new["occurrence"],
            "locator": {"container_xref":container,"operation_index":operation,"resource_path":[name],
                "bbox":new["bbox"],"transform":new["transform"]},
        }))
    }
}
