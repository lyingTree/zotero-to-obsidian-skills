use serde_json::{json,Value};
use std::{fs,path::Path};
use z2o_native::{check_ledger,delivery_validate,files,workflow};
fn save(p:&Path,v:&Value){fs::write(p,serde_json::to_vec_pretty(v).unwrap()).unwrap();}
#[test]
fn incomplete_workflow_inspection_never_grants_completion(){
    let root=tempfile::tempdir().unwrap();let base=root.path();let run=base.join("run.json");
    save(&run,&json!({"schema_version":1,"workflow_profile":"complete_pdf_candidate_v1","config":{
        "skills_root":base,"candidate_path":base.join("missing.md"),"glossary_path":base.join("glossary.md")},
        "scope":{"translation_required":true,"source_roles":{"si":"supplement"},"source_ids":["si"]},
        "identity":{"sources":[]},"records":{},"versions":{"files":[]},"naming":{"status":"pending"}}));
    let before=fs::read(&run).unwrap();let report=workflow::inspect(&run,|_,_,_|{}).unwrap();
    assert_eq!(report["records_ready"],false);
    let rows=report["findings"].as_array().unwrap();
    for name in ["source_inventory","source_map","link_map","si_map","checks","delivery_manifest","translation_review","terminology"]{
        assert!(rows.iter().any(|r|r["code"]=="WORKFLOW_RECORD_MISSING"&&r["target"]==name),"{name}: {report}");
    }
    assert!(workflow::require_completed(&run,|_,_,_|{}).is_err());
    assert_eq!(fs::read(&run).unwrap(),before);
}
#[test]
fn delivery_rejects_stale_manifest_before_accepting_targets(){
    let root=tempfile::tempdir().unwrap();let base=root.path();
    let inv=base.join("inventory.json");let candidate=base.join("candidate.md");let manifest=base.join("manifest.json");
    save(&inv,&json!({"schema_version":1,"sources":[],"units":[]}));fs::write(&candidate,"Actual draft.\n").unwrap();
    let b=z2o_native::check_ledger::bind_file(&inv).unwrap();let c=z2o_native::check_ledger::bind_file(&candidate).unwrap();
    save(&manifest,&json!({"schema_version":1,"inventory":b,"candidate":c,"deliveries":[],"resource_root":base}));
    let before=fs::read(&manifest).unwrap();fs::write(&candidate,"Different draft.\n").unwrap();
    let report=delivery_validate::validate_delivery(&manifest,|_,_,_|{}).unwrap();
    assert_eq!(report["structurally_consistent"],false);
    assert!(report["findings"].as_array().unwrap().iter().any(|r|r["message"].as_str().unwrap_or("").contains("MAP_BINDING_HASH")));
    assert_eq!(before,fs::read(&manifest).unwrap());
}
#[test]
fn empty_sources_and_units_are_not_valid_delivery(){
    let root=tempfile::tempdir().unwrap();let base=root.path();
    let inv=base.join("inventory.json");let candidate=base.join("candidate.md");let manifest=base.join("manifest.json");
    save(&inv,&json!({"schema_version":1,"sources":[],"units":[]}));fs::write(&candidate,"Draft.\n").unwrap();
    save(&manifest,&json!({"schema_version":1,"inventory":z2o_native::check_ledger::bind_file(&inv).unwrap(),
        "candidate":z2o_native::check_ledger::bind_file(&candidate).unwrap(),"deliveries":[],"resource_root":base}));
    let r=delivery_validate::validate_delivery(&manifest,|_,_,_|{}).unwrap();
    assert_eq!(r["structurally_consistent"],false);assert_eq!(r["native_subset_consistent"],false);
    assert!(r["findings"].as_array().unwrap().iter().any(|v|v["code"]=="INVENTORY_ACCOUNTING"||v["code"]=="MANIFEST_OR_EVIDENCE_INVALID"));
}

#[test]
fn manifest_preserves_existing_asset_hash_and_resource_boundary(){
    use z2o_native::delivery_prepare;
    let tmp=tempfile::tempdir().unwrap();let root=tmp.path();
    let resources=root.join("resources");fs::create_dir(&resources).unwrap();
    let inventory=root.join("inventory.json");save(&inventory,&json!({"schema_version":1,"sources":[],"units":[]}));
    let candidate=root.join("candidate.md");fs::write(&candidate,"note").unwrap();
    let image=resources.join("figure.png");fs::write(&image,"original").unwrap();
    let out=root.join("manifest.json");
    let bound=z2o_native::check_ledger::bind_file(&image).unwrap();
    let stale=json!([{"unit":"f","assets":[bound]}]);
    fs::write(&image,"changed").unwrap();
    assert!(delivery_prepare::manifest(&inventory,&candidate,&stale,&out,Some(&resources),None,false).is_err());
    assert!(!out.exists());
    let outside=root.join("outside.png");fs::write(&outside,"outside").unwrap();
    let escaped=json!([{"unit":"f","assets":[{"path":outside}]}]);
    assert!(delivery_prepare::manifest(&inventory,&candidate,&escaped,&out,Some(&resources),None,false).is_err());
    assert!(!out.exists());
    let valid=json!([{"unit":"f","assets":[{"path":"figure.png"}]}]);
    assert!(delivery_prepare::manifest(&inventory,&candidate,&valid,&out,Some(&resources),None,false).is_ok());
}

#[test]
fn workflow_manifest_cli_preflights_without_freezing_a_rejected_candidate() {
    use std::{io::Write, process::Command};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let work = root.join("工作 workflow");
    let caller = root.join("outside-working-directory");
    let resources = root.join("资源 resources");
    fs::create_dir(&caller).unwrap();
    fs::create_dir(&resources).unwrap();
    let source = root.join("源 source.docx");
    {
        let mut zip = zip::ZipWriter::new(fs::File::create(&source).unwrap());
        for (name, data) in [
            ("[Content_Types].xml", r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#),
            ("_rels/.rels", r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#),
            ("word/document.xml", r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>一个实际来源。</w:t></w:r></w:p></w:body></w:document>"#),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(data.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    let request_path = root.join("request.json");
    let exe = std::env::var_os("Z2O_WORKFLOW_TEST_CLI").unwrap_or_else(|| env!("CARGO_BIN_EXE_z2o").into());
    let invoke = |op: &str, args: Value, kwargs: Value| {
        save(&request_path, &json!({"schema_version":1,"operation":op,"args":args,"kwargs":kwargs}));
        Command::new(&exe).args(["invoke", request_path.to_str().unwrap()])
            .current_dir(&caller).env("PATH","").output().unwrap()
    };
    let get = |p: &Path| -> Value { serde_json::from_slice(&fs::read(p).unwrap()).unwrap() };
    let passed = |out: std::process::Output| -> Value {
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"].clone()
    };
    let candidate = root.join("候选 note.md");
    let skills = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let run = passed(invoke("complete_workflow.initialize_workflow", json!([]), json!({
        "skills_root":skills, "work_root":work, "sources":{"main":source},
        "source_roles":{"main":"main"}, "candidate_path":candidate,
        "resource_root":resources, "translation_required":false, "run_id":"manifest-transaction"
    })));
    let run = Path::new(run.as_str().unwrap());
    let data = get(run);
    let checks = Path::new(data["records"]["checks"].as_str().unwrap());
    let manifest = Path::new(data["records"]["delivery_manifest"].as_str().unwrap());
    let checks_before = fs::read(checks).unwrap();
    let work = run.parent().unwrap();
    let write = |deliveries:Value|invoke("complete_workflow.write_manifest",json!([run,deliveries]),json!({}));

    fs::write(&candidate, "# Rejected draft\n").unwrap();
    let failed = write(json!([{"unit":"missing","assets":[{"path":"not-present.png"}]}]));
    assert!(!failed.status.success());
    assert_eq!(files::digest(&fs::read(checks).unwrap()), files::digest(&checks_before), "A missing asset must not freeze the rejected candidate");
    assert!(!manifest.exists());

    fs::write(manifest, "Keep unrelated existing evidence.\n").unwrap();
    let failed = write(json!([]));
    assert!(!failed.status.success());
    assert_eq!(fs::read(checks).unwrap(), checks_before);
    assert_eq!(fs::read(manifest).unwrap(), b"Keep unrelated existing evidence.\n");

    let other = root.join("unrelated-checks.json");
    fs::write(&other, &checks_before).unwrap();
    let mut wrong = data.clone(); wrong["records"]["checks"] = json!(other);
    save(run, &wrong);
    let failed = invoke("complete_workflow.bind_candidate", json!([run]), json!({}));
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("Inventory must select"));
    assert_eq!(fs::read(&other).unwrap(), checks_before);
    assert_eq!(fs::read(checks).unwrap(), checks_before);

    // Relative paths are relative to the selected run, not the caller.
    let mut relative = data.clone();
    relative["records"]["checks"] = json!("checks.json");
    relative["records"]["source_inventory"] = json!("evidence/source-inventory.json");
    relative["records"]["delivery_manifest"] = json!("final/manifest.json");
    save(run, &relative);
    let run_before_retry = fs::read(run).unwrap();
    let final_manifest = work.join("final/manifest.json");
    let good_candidate = b"# Accepted forensic fixture\n\nActual Markdown bytes.\n";
    fs::write(&candidate, good_candidate).unwrap();
    let binding = passed(write(json!([])));
    assert_eq!(Path::new(binding.as_str().unwrap()), files::canonical(&final_manifest).unwrap());
    let bound_ledger = get(checks);
    assert_eq!(bound_ledger["bindings"]["candidates"][0]["sha256"], files::digest(good_candidate));
    assert_eq!(get(&final_manifest)["checks"], z2o_native::check_ledger::bind_file(checks).unwrap());
    assert_eq!(get(&final_manifest)["figure_edge_reviews_required"], true);
    assert_eq!(get(&final_manifest)["translation_review"], Value::Null);
    assert_eq!(get(&final_manifest)["candidate"]["sha256"], files::digest(good_candidate));
    let stable = fs::read(checks).unwrap();
    assert_eq!(passed(write(json!([]))), binding);
    assert_eq!(passed(invoke("complete_workflow.bind_candidate", json!([run]), json!({})))["sha256"],
        files::digest(good_candidate));
    assert_eq!(fs::read(checks).unwrap(), stable, "An exact retry must not rewrite the ledger");

    fs::write(&candidate, "# New candidate version\n").unwrap();
    assert!(!write(json!([])).status.success());
    assert_eq!(fs::read(checks).unwrap(), stable);
    assert_eq!(fs::read(manifest).unwrap(), b"Keep unrelated existing evidence.\n");
    assert_eq!(fs::read(run).unwrap(), run_before_retry);
    let inspect = passed(invoke("complete_workflow.inspect_workflow", json!([run]), json!({})));
    assert_eq!(inspect["records_ready"], false);
    assert_eq!(inspect["source_fidelity"], "not_assessed");
}

#[test]
fn complete_workflow_reaches_records_ready_and_commits_to_vault() {
    use std::{io::Write, process::Command};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let skills = root.join("skills");
    fs::create_dir_all(skills.join("minimal-skill")).unwrap();
    fs::write(
        skills.join("minimal-skill/SKILL.md"),
        "---\nname: minimal-skill\nversion: v2.0.4\ndescription: E2E fixture\n---\n\n- [ ] [MIN-A01] Actual candidate check recorded. <!-- check-type: offline_outcome -->\n",
    )
    .unwrap();
    let work = root.join("work");
    let resources = root.join("resources");
    let vault = root.join("vault");
    let backup = root.join("backup");
    fs::create_dir(&resources).unwrap();
    fs::create_dir(&backup).unwrap();
    fs::create_dir_all(vault.join(".obsidian")).unwrap();
    fs::create_dir_all(vault.join("文献")).unwrap();

    let source = root.join("source.docx");
    {
        let mut zip = zip::ZipWriter::new(fs::File::create(&source).unwrap());
        for (name, data) in [
            ("[Content_Types].xml", r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#),
            ("_rels/.rels", r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#),
            ("word/document.xml", r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Verified paragraph text.</w:t></w:r></w:p></w:body></w:document>"#),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(data.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    let req = root.join("request.json");
    let exe = env!("CARGO_BIN_EXE_z2o");
    let invoke = |op: &str, args: Value, kwargs: Value| {
        save(&req, &json!({"schema_version":1,"operation":op,"args":args,"kwargs":kwargs}));
        let out = Command::new(exe).args(["invoke", req.to_str().unwrap()]).env("PATH", "").output().unwrap();
        assert!(out.status.success(), "{op} failed: {}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"].clone()
    };
    let get = |p: &Path| -> Value { serde_json::from_slice(&fs::read(p).unwrap()).unwrap() };

    let candidate = work.join("candidate.md");
    let run_val = invoke("complete_workflow.initialize_workflow", json!([]), json!({
        "skills_root": skills, "work_root": work, "sources": {"main": source},
        "source_roles": {"main": "main"}, "candidate_path": candidate,
        "resource_root": resources, "translation_required": false, "run_id": "e2e-complete"
    }));
    let run_path = Path::new(run_val.as_str().unwrap());
    let run_doc = get(run_path);
    let inv_path = Path::new(run_doc["records"]["source_inventory"].as_str().unwrap());
    let queue_path = Path::new(run_doc["records"]["draft_queue"].as_str().unwrap());
    let checks_path = Path::new(run_doc["records"]["checks"].as_str().unwrap());

    let mut inv = get(inv_path);
    let block = &inv["sources"][0]["native_blocks"][0];
    let block_id = block["id"].as_str().unwrap().to_owned();
    let loc = json!({"basis":"native","blocks":[block_id],"native_locator":block["locator"]});
    let absent = json!({"status":"absent_in_source","reason":"Fixture lacks header metadata.","evidence":[loc]});
    inv["sources"][0]["header_fields"] = json!({"title":absent,"authors":absent,"affiliations":absent,"publication_date":absent});
    inv["units"] = json!([{"id":"p1","source":"main","kind":"paragraph","evidence":[loc]}]);
    save(inv_path, &inv);

    invoke("draft_queue.DraftQueue.group", json!([queue_path,"p1",[block_id]]), json!({"kind":"paragraph","reason":"Single body paragraph"}));
    invoke("draft_queue.DraftQueue.write_unit", json!([queue_path,"p1","Verified paragraph text. ^p1"]), json!({}));
    let assembled = work.join("assembled.md");
    invoke("draft_queue.DraftQueue.assemble", json!([queue_path, assembled]), json!({}));

    let note_bytes = concat!(
        "---\n",
        "类型: \"期刊论文\"\n",
        "年份: 2020\n",
        "关键词: []\n",
        "期刊名: \"Journal of Testing\"\n",
        "---\n\n",
        "# Test Title\n\n",
        "## 1 正文\n",
        "Verified paragraph text. ^p1\n\n",
        "## 2 参考文献\n",
        "无。\n",
    );
    fs::write(&candidate, note_bytes).unwrap();
    let src_bind = check_ledger::bind_file(&source).unwrap();
    let cand_bind = check_ledger::bind_file(&candidate).unwrap();

    let ev = json!({"file": src_bind, "locator": {"kind":"native","part":"word/document.xml"}});
    let naming_plan = invoke("format_helpers.build_naming_plan", json!([]), json!({
        "year": 2020,
        "author_token": "Smith",
        "original_title": "Test Title",
        "source_evidence": {"year": ev, "author_token": ev, "original_title": ev},
        "delivery_path": candidate,
        "target_directory": vault.join("文献"),
    }));
    invoke("complete_workflow.set_naming_plan", json!([run_path, naming_plan]), json!({}));

    invoke("complete_workflow.record_check_batch", json!([]), json!({
        "checks_path": checks_path,
        "evidence": {"cand": {"path": cand_bind["path"], "sha256": cand_bind["sha256"], "locator": {"kind":"whole_file"}, "evidence_type":"offline_result"}},
        "results": [{"id":"MIN-A01","status":"passed","scope":[{"object_id":cand_bind["path"],"locator":{"kind":"whole_file"}}],"evidence":["cand"],"reason":"Verified candidate file for E2E fixture."}]
    }));

    let deliveries = json!([{"unit":"p1","target":{"anchor":"p1"}}]);
    invoke("complete_workflow.write_manifest", json!([run_path, deliveries]), json!({}));
    let maps = invoke("complete_workflow.write_maps", json!([run_path, deliveries]), json!({}));
    assert_eq!(maps["records_ready"], true, "{maps}");

    let report = invoke("complete_workflow.inspect_workflow", json!([run_path]), json!({}));
    assert_eq!(report["records_ready"], true, "{report}");
    assert_eq!(report["findings"], json!([]));
    assert_eq!(report["source_fidelity"], "not_assessed");

    let required = invoke("complete_workflow.require_completed_records", json!([run_path]), json!({}));
    assert_eq!(required["records_ready"], true);

    let resumed = invoke("complete_workflow.resume_workflow", json!([run_path]), json!({}));
    assert_eq!(resumed["phase"], "review");
    assert_eq!(resumed["records_ready"], true);
    assert_eq!(resumed["next_action"]["stage"], "records_complete");

    let expected_name = naming_plan["filename"].as_str().unwrap();
    let note_rel = format!("文献/{expected_name}");
    invoke("complete_workflow.set_vault_write_scope", json!([run_path, true]), json!({
        "authorized": true,
        "vault_path": note_rel,
    }));
    let run_sha = check_ledger::bind_file(run_path).unwrap()["sha256"].as_str().unwrap().to_owned();

    let planned = invoke("vault_publish.plan", json!([run_path]), json!({
        "expected_run_sha256": run_sha,
        "vault_root": vault,
        "literature_relative": "文献",
        "assets_relative": "文献/attachments",
        "note_relative": note_rel,
        "expected_existing_sha256": Value::Null,
        "inspect_workflow": true,
    }));
    assert_eq!(planned["preconditions_ready"], true, "{planned}");

    let commit = invoke("vault_publish.commit_markdown", json!([vault]), json!({
        "backup_root": backup,
        "literature_relative": "文献",
        "assets_relative": "文献/attachments",
        "note_relative": note_rel,
        "candidate_path": candidate,
        "expected_candidate_sha256": cand_bind["sha256"],
        "expected_existing_sha256": Value::Null,
        "resources": [],
        "scope": "complete_workflow",
        "authorized": true,
        "workflow_run": run_path,
        "expected_run_sha256": run_sha,
    }));
    assert_eq!(commit["status"], "verified_file_write", "{commit}");
    assert_eq!(commit["scope"], "complete_workflow", "{commit}");
    assert_eq!(commit["full_workflow_accepted"], false, "{commit}");
    assert_eq!(commit["source_fidelity"], "not_assessed", "{commit}");
    assert_eq!(fs::read(vault.join(&note_rel)).unwrap(), fs::read(&candidate).unwrap());
}
