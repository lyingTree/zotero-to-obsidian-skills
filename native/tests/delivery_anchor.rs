//! Real CLI delivery scans, with an actual OOXML source and bound Markdown.
use serde_json::{json, Value};
use std::{fs, io::Write, path::Path, process::Command};
use z2o_native::{check_ledger, delivery_prepare, pipeline, source_prepare};

fn docx(path: &Path) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let document = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>See study [1].</w:t></w:r></w:p><w:p><w:r><w:t>A source reference.</w:t></w:r></w:p></w:body></w:document>"#;
    for (name, body) in [("[Content_Types].xml", types), ("_rels/.rels", rels), ("word/document.xml", document)] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn citation_preview_grammar_applies_to_unmapped_delivery_objects_and_calls() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let source = root.join("真实源.docx");
    docx(&source);
    let inventory = source_prepare::prepare_sources(&json!({"main": source}), &root.join("evidence"),
        &json!({"main":"main"}), None, 144, None, |_,_,_| {}).unwrap();
    let mut inv = pipeline::read_json(&inventory).unwrap();
    let blocks = inv["sources"][0]["native_blocks"].as_array().unwrap();
    let locs = blocks.iter().map(|b| json!({"basis":"native","blocks":[b["id"]],"native_locator":b["locator"]}))
        .collect::<Vec<_>>();
    let absence = json!({"status":"absent_in_source","reason":"This generated source has no header metadata.",
        "evidence":[locs[0]]});
    inv["sources"][0]["header_fields"] = json!({"title":absence,"authors":absence,"affiliations":absence,"publication_date":absence});
    let paragraph = json!({"id":"body","source":"main","kind":"paragraph","display_text":"[1]","evidence":[locs[0],locs[1]]});
    let bibliography = json!({"id":"reference","source":"main","kind":"reference","number":"1","bibliography_id":"test-bibliography",
        "original_text":"A source reference.","evidence":[locs[1]]});
    let citation = json!({"id":"call","source":"main","kind":"citation","references":["reference"],
        "display_text":"[1]","evidence":[locs[0]]});
    let save = |path: &Path, value: &Value| fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();

    let exe = std::env::var_os("Z2O_DELIVERY_TEST_CLI").unwrap_or_else(|| env!("CARGO_BIN_EXE_z2o").into());
    let mut run = |i:usize, anchor:&str, as_reference:bool, with_citation:bool| -> Value {
        let candidate = root.join(format!("candidate-{i}.md"));
        let manifest = root.join(format!("manifest-{i}.json"));
        let request = root.join(format!("request-{i}.json"));
        let link = format!("[1](#^{anchor})");
        let mut body = paragraph.clone();
        let mut deliveries = vec![json!({"unit":"body","target":{"anchor":anchor}})];
        let mut content = format!("See study {link}. A source reference. ^{anchor}\n");
        if as_reference {
            body["evidence"] = json!([locs[0]]);
            inv["units"] = json!([body,bibliography]);
            content = format!("See study {link}. ^body\n\n1. A source reference. ^{anchor}\n");
            deliveries[0]["target"] = json!({"anchor":"body"});
            deliveries.push(json!({"unit":"reference","target":{"anchor":anchor}}));
            if with_citation {
                inv["units"].as_array_mut().unwrap().push(citation.clone());
                deliveries.push(json!({"unit":"call","target":{"quote":link}}));
            }
        } else {
            inv["units"] = json!([body]);
        }
        // Two source blocks make an implicit link origin ambiguous. Bind the
        // actual decoded display and original source block explicitly.
        if !with_citation {
            deliveries[0]["origin_block"] = locs[0]["blocks"][0].clone();
        }
        save(&inventory, &inv);
        fs::write(&candidate, content).unwrap();
        delivery_prepare::manifest(&inventory, &candidate, &json!(deliveries), &manifest, Some(root), None, false).unwrap();
        save(&request, &json!({"schema_version":1,"operation":"validate_delivery.validate_delivery","args":[manifest]}));
        let before = check_ledger::bind_file(&candidate).unwrap();
        let out = Command::new(&exe).arg("invoke").arg(&request).env("PATH","").env_remove("PYTHONPATH").output().unwrap();
        assert!(out.status.success(),"{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(check_ledger::bind_file(&candidate).unwrap(), before);
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"].clone()
    };

    let has = |report:&Value, code:&str, id:&str| report["findings"].as_array().unwrap().iter()
        .any(|f| f["code"] == code && f["object_id"] == id);
    for (i,(anchor,recognized)) in [
        ("ref1",true), ("SREF2",true), ("sref-sourceA-Main2-27",true),
        ("ref-note",false), ("ref-0",false), ("ref-3-part-2",false), ("ref-X-0002",false),
    ].into_iter().enumerate() {
        let report=run(i,anchor,false,false);
        assert_eq!(has(&report,"OUTPUT_OBJECT_UNMAPPED",anchor),recognized,"{report}");
        assert_eq!(has(&report,"OUTPUT_CITATION_UNMAPPED",anchor),recognized,"{report}");
        assert!(!has(&report,"DELIVERY_HANDOFF","manifest"),"{report}");
        assert!(has(&report,"NATIVE_DELIVERY_PARITY_PENDING","manifest"));
        assert_eq!(report["structurally_consistent"],false);
    }
    // A genuine mapped reference and its mapped caller remain valid.
    let mapped=run(7,"ref1",true,true);
    assert!(!has(&mapped,"OUTPUT_OBJECT_UNMAPPED","ref1"));
    assert!(!has(&mapped,"OUTPUT_CITATION_UNMAPPED","ref1"));
    assert!(!has(&mapped,"DELIVERY_HANDOFF","manifest"),"{mapped}");
    // Registered explicit bibliography IDs remain recognized even when their
    // candidate anchor is custom and not understood by Citation Preview.
    let custom=run(8,"shared-reading-note",true,false);
    assert!(has(&custom,"OUTPUT_CITATION_UNMAPPED","shared-reading-note"),"{custom}");
    assert!(!has(&custom,"OUTPUT_OBJECT_UNMAPPED","shared-reading-note"));
}
