use serde_json::{json, Value};
use std::{fs, io::Write, path::PathBuf, process::Command};
use tempfile::TempDir;
use zip::{write::SimpleFileOptions, ZipWriter};

struct Fixture {
    dir: TempDir,
    source: PathBuf,
    candidate: PathBuf,
    prepared: PathBuf,
}

impl Fixture {
    fn new(text: &str, unit_locations: Value) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("supplement.docx");
        let candidate = dir.path().join("译文.md");
        let prepared = dir.path().join("pending.json");
        let f = Self { dir, source, candidate, prepared };

        let mut docx = ZipWriter::new(fs::File::create(&f.source).unwrap());
        docx.start_file("[Content_Types].xml", SimpleFileOptions::default()).unwrap();
        docx.write_all(br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#).unwrap();
        docx.start_file("word/document.xml", SimpleFileOptions::default()).unwrap();
        docx.write_all(br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>The wave appears.</w:t></w:r></w:p></w:body></w:document>"#).unwrap();
        docx.finish().unwrap();

        fs::write(&f.candidate, text).unwrap();
        let mut inventory_source = f.ok("docx_source_inventory.read_docx", json!([f.source, "S"]), json!({}));
        inventory_source["id"] = json!("S");
        inventory_source["path"] = json!(f.source);
        inventory_source["sha256"] = json!(Self::digest(&f.source));
        let block = &inventory_source["native_blocks"][0];
        let unit = json!({"id":"U","source":"S","kind":"paragraph","evidence":[{
            "basis":"native","native_locator":block["locator"],"blocks":[block["id"]]
        }]});
        let inv = f.dir.path().join("inventory.json");
        fs::write(&inv, serde_json::to_vec(&json!({"schema_version":1,"sources":[inventory_source],"units":[unit]})).unwrap()).unwrap();
        let glossary = f.dir.path().join("glossary.md");
        fs::write(&glossary, "| ID | English | Chinese |\n| --- | --- | --- |\n| WAVE | wave | 波浪 |\n").unwrap();
        f.ok("prepare_translation_review.prepare_translation_review",
            json!([inv, f.candidate, f.prepared]),
            json!({"unit_ids":["U"], "glossary_path":glossary, "candidate_locations":{"U":unit_locations}}));
        f
    }

    fn digest(path: &std::path::Path) -> String {
        z2o_native::files::digest(&fs::read(path).unwrap())
    }

    fn call(&self, op: &str, args: Value, kwargs: Value) -> Result<Value, String> {
        let request = self.dir.path().join("request.json");
        fs::write(&request, serde_json::to_vec(&json!({"schema_version":1,"operation":op,"args":args,"kwargs":kwargs})).unwrap()).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_z2o")).arg("invoke").arg(request).output().unwrap();
        if out.status.success() {
            let parsed: Value = serde_json::from_slice(&out.stdout).unwrap();
            Ok(parsed["result"].clone())
        } else {
            assert!(out.stdout.is_empty());
            Err(String::from_utf8(out.stderr).unwrap())
        }
    }

    fn ok(&self, op: &str, args: Value, kwargs: Value) -> Value {
        self.call(op, args, kwargs).unwrap_or_else(|e| panic!("{op}: {e}"))
    }
}

fn occurrence(source: Value, candidate: Value) -> Value {
    json!({"unit_id":"U","kind":"claims","occurrence_id":"c1","source":source,"candidate":candidate,
        "status":"passed","reason":"Explicit independent comparison in this synthetic fixture."})
}

#[test]
fn repeated_inline_selection_preserves_unicode_offset_through_replay_and_save() {
    // Prefix has multibyte BMP and astral characters; this source also uses
    // CR-only line boundaries. The same quote appears four times, with two
    // feasible selections in the first of two identical inline units.
    let f = Fixture::new("🌊前缀\r波浪波 独立 波浪波 独立\r", json!([
        {"quote":"波浪波 独立","char_range":[4,10]},
        {"quote":"波浪波 独立","char_range":[11,17]}
    ]));
    let pick = json!({"quote":"波","occurrence":2});
    let got = f.ok("translation_review_writer.select_candidate",json!([f.prepared]),
        json!({"unit_id":"U","selector":pick}));
    assert_eq!(got["locator"], json!({"lines":[2,2],"quote":"波","char_range":[6,7]}));

    let result = f.ok("translation_review_writer.UnitReview.occurrence",json!([f.prepared]),
        occurrence(json!({"quote":"The wave"}),pick));
    assert_eq!(result["data"]["units"]["U"]["occurrences"]["claims"][0]["candidate_span"],got["locator"]);
    let draft = f.dir.path().join("decisions.json");
    f.ok("translation_review_writer.save_decision_draft",json!([f.prepared,result["decisions"]]),json!({"output_path":draft}));
    let restored = f.ok("translation_review_writer.restore_decision_draft",json!([draft]),json!({"expected_draft_sha256":Fixture::digest(&draft)}));
    assert_eq!(restored["data"],result["data"]);

    let output = f.dir.path().join("review.json");
    let report = f.ok("translation_review_writer.save_review",json!([f.prepared,restored["decisions"]]),json!({"output_path":output}));
    let findings = report["findings"].as_array().unwrap();
    assert!(findings.iter().all(|r|r["code"]!="TRN_LOCATION_INVALID" && r["code"]!="TRN_CANDIDATE_SPAN_OUTSIDE_UNIT"),"{findings:?}");
    assert_eq!(report["records_ready"],false,"unperformed semantic checks remain pending");

    // Pretending the neighboring character was the selected literal used to
    // pass structural validation because the same quote exists on that line.
    let mut tampered: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    tampered["units"]["U"]["occurrences"]["claims"][0]["candidate_span"]["char_range"] = json!([5,6]);
    let changed = f.dir.path().join("tampered.json");
    fs::write(&changed,serde_json::to_vec(&tampered).unwrap()).unwrap();
    let report = f.ok("validate_translation_review.validate_translation_review",json!([changed]),json!({}));
    assert!(report["findings"].as_array().unwrap().iter().any(|r|r["code"]=="TRN_LOCATION_INVALID"
        &&r["object_id"].as_str().unwrap().ends_with("/candidate_span")),"{report}");
}

#[test]
fn nested_selector_checks_are_shared_by_direct_short_call_and_replayed_draft() {
    let f = Fixture::new("波浪出现。\n",json!([{"lines":[1,1]}]));
    for (source, candidate, error) in [
        (json!({"quote":"The wave","block_id":17}),json!({"quote":"波浪"}),"invalid block_id"),
        (json!({"quote":"The wave"}),json!({"quote":"波浪","locator":{"char_range":[0,2]}}),"unknown selector field")
    ] {
        let mut call = occurrence(source,candidate);
        let got = f.call("translation_review_writer.UnitReview.occurrence",json!([f.prepared]),call.clone()).unwrap_err();
        assert!(got.contains(error),"{got}");
        call["op"] = json!("occurrence");
        call["comparison"] = json!({});
        let draft = f.dir.path().join("bad-draft.json");
        fs::write(&draft,serde_json::to_vec(&json!({
            "schema_version":1,"format":"translation-review-decisions-v1",
            "prepared":z2o_native::translation_prepare::bind_file(&f.prepared).unwrap(),
            "decisions":[call]
        })).unwrap()).unwrap();
        let got = f.call("translation_review_writer.restore_decision_draft",json!([draft]),json!({})).unwrap_err();
        assert!(got.contains(error),"{got}");
    }
    assert!(!f.dir.path().join("review.json").exists());
}

#[test]
fn quote_only_locations_reject_overlaps_and_legacy_anchored_offsets_still_validate() {
    use z2o_native::translation_validate::text_range;
    assert!(text_range("ababa", &json!({"quote":"aba"})).is_err());
    assert_eq!(text_range("🌊波浪", &json!({"quote":"浪","char_range":[2,3]})).unwrap(),(7,10));
    let loc = json!({"quote":"汉字：重重","selected_quote":"重","source_lines":[2,2],"char_range":[5,6]});
    let text = "🌊\n汉字：重重\n";
    let (start, end) = text_range(text, &loc).unwrap();
    assert_eq!(&text[start..end],"重");
    let mut displaced = loc;
    displaced["source_lines"] = json!([1,1]);
    assert!(text_range(text, &displaced).is_err());
}

#[test]
fn explicit_complete_review_and_terminology_publish_without_claiming_semantic_certification() {
    use z2o_native::translation_validate::{CATEGORIES, CHECKS};

    let f = Fixture::new("波浪出现。\n", json!([{"lines":[1,1]}]));
    let draft_before = fs::read(&f.prepared).unwrap();
    let mut commands = vec![
        json!({"op":"scope","status":"passed","reason":"Only the actual source sentence is in this fixture."}),
        json!({"op":"occurrence","unit_id":"U","kind":"claims","occurrence_id":"c1",
            "source":{"quote":"The wave appears."},"candidate":{"quote":"波浪出现。"},
            "status":"passed","reason":"Explicit fixture claim comparison.","comparison":{}}),
        json!({"op":"term","unit_id":"U","term_id":"t","original":"wave","category":"concept","source":{"quote":"wave"}}),
        json!({"op":"search","unit_id":"U","term_id":"t","mode":"full_phrase","query":"wave","interpretation":"Actual phrase search."}),
        json!({"op":"search","unit_id":"U","term_id":"t","mode":"family","query":"wave","interpretation":"Actual family search."}),
        json!({"op":"search_status","unit_id":"U","term_id":"t","mode":"aliases","status":"not_applicable","reason":"This fixture term has no aliases."}),
        json!({"op":"decide","unit_id":"U","term_id":"t","decision":"adopted","translation":"波浪","reason":"Fixture glossary matches the single concept.","matches":["WAVE"]}),
    ];
    for name in CHECKS {
        let passed = matches!(*name, "alignment" | "completeness" | "terminology");
        let reason = if passed {
            format!("Compared the explicit {name} evidence in this one-sentence fixture.")
        } else {
            format!("The single affirmative fixture statement has no applicable {name} object.")
        };
        commands.push(json!({"op":"check","unit_id":"U","name":name,
            "status":if passed {"passed"} else {"not_applicable"},"reason":reason}));
    }
    for category in CATEGORIES {
        let present = *category == "concept";
        commands.push(json!({"op":"scan","unit_id":"U","category":category,
            "status":if present {"scanned"} else {"not_applicable"},
            "candidate_ids":if present {json!(["t"])} else {json!([])},
            "reason":format!("Explicit scan of the one-sentence fixture's {category} category.")}));
    }
    commands.extend([
        json!({"op":"source_issues_review","status":"passed","reason":"The complete fixture source sentence was inspected."}),
        json!({"op":"terminology_review","status":"reviewed","covered_units":["U"],"no_unlisted_reason":"The one fixture concept was adopted from the actual glossary."}),
        json!({"op":"review","status":"reviewed"}),
    ]);
    let output = f.dir.path().join("review-complete.json");
    let terminology = f.dir.path().join("terminology-complete.json");
    let report = f.ok("translation_review_writer.save_review",json!([f.prepared,commands]),
        json!({"output_path":output,"terminology_path":terminology}));
    assert_eq!(report["findings"],json!([]),"{report}");
    assert_eq!(report["records_ready"],true);
    assert_eq!(report["source_fidelity"],"not_assessed");
    assert_eq!(report["execution_truth"],"not_assessed");
    assert_eq!(report["independent_review_required"],true);
    assert_eq!(fs::read(&f.prepared).unwrap(),draft_before);
    let report_again = f.ok("validate_translation_review.validate_translation_review",json!([output]),json!({}));
    assert_eq!(report,report_again);
    let published: Value = serde_json::from_slice(&fs::read(&terminology).unwrap()).unwrap();
    assert_eq!(published["adopted"][0]["translation"],"波浪");
    assert_eq!(published["adopted"][0]["source_units"],json!(["U"]));
}
