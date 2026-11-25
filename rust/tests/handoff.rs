use lopdf::{Document,Object,Stream,dictionary};
use serde_json::{json,Value};
use std::{fs,path::{Path,PathBuf}};
use z2o_native::{files,handoff_native as maps,mapping_compare,pipeline};
fn pdf(path:&Path){
    let mut d=Document::with_version("1.5");let pages=d.new_object_id();
    let font=d.add_object(dictionary!{"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
    let stream=d.add_object(Stream::new(dictionary!{},b"BT /F1 12 Tf 1 0 0 1 10 760 Tm (Body [1]) Tj 1 0 0 1 10 730 Tm (Reference entry) Tj ET".to_vec()));
    let page=d.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>stream,"Resources"=>dictionary!{"Font"=>dictionary!{"F1"=>font}}});
    d.objects.insert(pages,Object::Dictionary(dictionary!{"Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1,"MediaBox"=>vec![0.into(),0.into(),600.into(),800.into()]}));
    let catalog=d.add_object(dictionary!{"Type"=>"Catalog","Pages"=>pages});d.trailer.set("Root",catalog);d.save(path).unwrap();
}
fn binding(p:&Path)->Value{let b=files::snapshot(p).unwrap();json!({"path":b["path"],"sha256":b["sha256"]})}
fn save(p:&Path,v:&Value){fs::write(p,serde_json::to_vec_pretty(v).unwrap()).unwrap();}
struct Fixture{root:tempfile::TempDir,inventory:PathBuf,candidate:PathBuf,paths:Value,deliveries:Value,expected:Value,run:PathBuf}
impl Fixture{
    fn new()->Self{
        let root=tempfile::tempdir().unwrap();let p=root.path();
        let main=p.join("main.pdf");let si=p.join("supplement.pdf");pdf(&main);pdf(&si);
        let candidate=p.join("candidate.md");
        fs::write(&candidate,"---\r\nkind: test\r\n---\r\n# 海洋\r\n\r\n正文 [1](#^ref-a)\r\n^body\r\n\r\nReference entry ^ref-a\r\n\r\n补充材料 ^si-body\r\n").unwrap();
        let mut a=binding(&main);a["id"]=json!("main");a["role"]=json!("main");
        a["pages"]=json!([{"page":1,"blocks":[{"id":"b1","bbox":[10,10,100,20],"text":"Body [1]"},{"id":"b2","bbox":[10,30,100,40],"text":"Reference entry"}]}]);
        let mut b=binding(&si);b["id"]=json!("si");b["role"]=json!("supplement");
        b["pages"]=json!([{"page":1,"blocks":[{"id":"s1","bbox":[10,10,100,20],"text":"Body [1]"}]}]);
        let loc=|block:&str,text:&str|json!([{"page":1,"blocks":[block],"quote":text}]);
        let inv=json!({"schema_version":1,"sources":[a,b],"units":[
            {"id":"body","kind":"paragraph","source":"main","evidence":loc("b1","Body")},
            {"id":"call","kind":"citation","source":"main","references":["reference"],"display_text":"[1]","evidence":loc("b1","[1]")},
            {"id":"reference","kind":"reference","source":"main","bibliography_id":"main-bib","number":"1","original_text":"Reference entry","evidence":loc("b2","Reference entry")},
            {"id":"si-body","kind":"paragraph","source":"si","evidence":loc("s1","Body")}
        ],"excluded_blocks":[],"excluded_pages":[]});
        let inventory=p.join("inventory.json");save(&inventory,&inv);
        let deliveries=json!([{"unit":"body","target":{"anchor":"body"}},{"unit":"call","target":{"quote":"[1](#^ref-a)"}},
            {"unit":"reference","target":{"anchor":"ref-a"}},{"unit":"si-body","target":{"anchor":"si-body"}}]);
        let paths=json!({"source_map":p.join("source-map.json"),"si_map":p.join("si-map.json"),"link_map":p.join("link-map.json")});
        let expected=json!({"main":{"path":a["path"],"sha256":a["sha256"],"role":"main"},"si":{"path":b["path"],"sha256":b["sha256"],"role":"supplement"}});
        let run=p.join("run.json");save(&run,&json!({"schema_version":1,"records":{"source_inventory":inventory,"source_map":paths["source_map"],"si_map":paths["si_map"],"link_map":paths["link_map"]},
            "config":{"candidate_path":candidate},"identity":{"sources":[a,b]},"scope":{"source_roles":{"main":"main","si":"supplement"}}}));
        Self{root,inventory,candidate,paths,deliveries,expected,run}
    }
    fn write(&self)->Value{maps::write(&self.inventory,&self.candidate,&self.deliveries,&self.paths,&self.expected,|_,_,_|{}).unwrap()}
    fn validate(&self)->Value{maps::validate(&self.inventory,&self.candidate,&self.paths,&self.expected,|_,_,_|{}).unwrap()}
    fn map(&self,name:&str)->PathBuf{PathBuf::from(self.paths[name].as_str().unwrap())}
}

fn invoke_rerun(f:&Fixture,destination:&Path,deliveries:&Value)->std::process::Output{
    let request=f.root.path().join("rerun-request.json");
    save(&request,&json!({"schema_version":1,"operation":"mapping_rerun.record_mapping_rerun",
        "args":[f.run,deliveries,destination]}));
    std::process::Command::new(env!("CARGO_BIN_EXE_z2o")).arg("invoke").arg(request)
        .env("PATH","").output().unwrap()
}

#[test]
fn real_cli_reruns_unversioned_legacy_manifest_and_keeps_prior_bytes(){
    let f=Fixture::new();assert_eq!(f.write()["records_ready"],true);
    let mut run=pipeline::read_json(&f.run).unwrap();
    run.as_object_mut().unwrap().remove("schema_version");
    save(&f.run,&run);
    let before=[&f.run,&f.candidate,&f.inventory,&f.map("source_map"),&f.map("si_map"),&f.map("link_map")]
        .into_iter().map(|p|(p.clone(),fs::read(p).unwrap())).collect::<Vec<_>>();

    let destination=f.root.path().join("legacy-rerun");
    let output=invoke_rerun(&f,&destination,&f.deliveries);
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    let payload:Value=serde_json::from_slice(&output.stdout).unwrap();
    let report=PathBuf::from(payload["result"].as_str().unwrap());
    assert_eq!(report,files::canonical(&destination.join("rerun-comparison.json")).unwrap());
    let r=pipeline::read_json(&report).unwrap();
    assert_eq!(r["mapping_equal"],true);assert_eq!(r["actual_rerun_performed"],true);
    assert_eq!(r["baseline_validated"],true);assert_eq!(r["rerun_validated"],true);
    assert_eq!(r["source_fidelity"],"not_assessed");assert_eq!(r["acceptance_status_written"],false);
    for (path,bytes) in before{
        assert_eq!(fs::read(path).unwrap(),bytes);
    }

    let report_before=fs::read(&report).unwrap();
    let output=invoke_rerun(&f,&destination,&f.deliveries);
    assert!(!output.status.success(),"existing evidence must not be replaced");
    assert!(String::from_utf8_lossy(&output.stderr).contains("Use a new mapping rerun directory"));
    assert_eq!(fs::read(&report).unwrap(),report_before);

    run["schema_version"]=json!(2);save(&f.run,&run);
    let rejected=f.root.path().join("unsupported-schema");
    let output=invoke_rerun(&f,&rejected,&f.deliveries);
    assert!(!output.status.success(),"unknown explicitly versioned manifests must be rejected");
    assert!(String::from_utf8_lossy(&output.stderr).contains("MAP_SCHEMA_VERSION"));
    assert!(!rejected.exists());
}

#[test]
fn rerun_does_not_recapture_a_valid_baseline_edited_during_source_loading(){
    let f=Fixture::new();assert_eq!(f.write()["records_ready"],true);
    let prior=f.map("link_map");
    let mut changed=pipeline::read_json(&prior).unwrap();
    changed["review_status"]=json!("checked");
    let mut edit_performed=false;
    let output=f.root.path().join("racing-baseline");
    let result=mapping_compare::record_mapping_rerun(&f.run,&f.deliveries,&output,|_,_,label|{
        if !edit_performed&&label=="actual source bindings and block inventory"{
            save(&prior,&changed);edit_performed=true;
        }
    });
    assert!(edit_performed&&f.validate()["records_ready"]==true,"changed metadata is independently valid");
    let error=result.expect_err("pre-validation baseline bytes were not frozen");
    assert!(format!("{error:#}").contains("MAP_INPUT_CHANGED"),"{error:#}");
    assert_eq!(pipeline::read_json(&prior).unwrap(),changed);
    assert!(!output.exists(),"do not generate evidence from the newly captured baseline");
}

#[test]
fn native_generation_validation_and_actual_rerun(){
    let f=Fixture::new();let r=f.write();assert_eq!(r["records_ready"],true,"{r}");
    let source=pipeline::read_json(&f.map("source_map")).unwrap();let link=pipeline::read_json(&f.map("link_map")).unwrap();
    assert_eq!(source["blocks"].as_array().unwrap().len(),2);assert_eq!(link["internal_links"][0]["origin_unit"],"call");
    assert_eq!(link["internal_links"][0]["target_unit"],"reference");
    assert_eq!(link["internal_links"][0]["source_display_text"],"[1]");
    assert_eq!(source["review_status"],"pending");assert_eq!(f.validate()["records_ready"],true);
    let old=fs::read(f.map("source_map")).unwrap();assert_eq!(f.write()["records_ready"],false);assert_eq!(fs::read(f.map("source_map")).unwrap(),old);
    let p=mapping_compare::record_mapping_rerun(&f.run,&f.deliveries,&f.root.path().join("rerun"),|_,_,_|{}).unwrap();
    let r=pipeline::read_json(&p).unwrap();assert_eq!(r["actual_rerun_performed"],true);
    assert_eq!(r["mapping_equal"],true);assert_eq!(r["acceptance_status_written"],false);
    assert_eq!(fs::read(f.map("source_map")).unwrap(),old);
    let mut changed=link;changed["internal_links"][0]["source_display_text"]=json!("fabricated");save(&f.map("link_map"),&changed);
    assert_eq!(f.validate()["records_ready"],false);
    assert!(mapping_compare::record_mapping_rerun(&f.run,&f.deliveries,&f.root.path().join("bad-rerun"),|_,_,_|{}).is_err());
    assert!(!f.root.path().join("bad-rerun").exists());
}
#[test]
fn preflight_preserves_complete_map_group(){
    let f=Fixture::new();let mut d=f.deliveries.clone();d.as_array_mut().unwrap().pop();
    let r=maps::write(&f.inventory,&f.candidate,&d,&f.paths,&f.expected,|_,_,_|{}).unwrap();
    assert_eq!(r["records_ready"],false);
    for name in ["source_map","si_map","link_map"]{assert!(!f.map(name).exists());}
    let mut inv=pipeline::read_json(&f.inventory).unwrap();inv["sources"][0]["pages"][0]["blocks"].as_array_mut().unwrap().push(json!({"id":"unaccounted","text":"lost","bbox":[10,50,100,60]}));
    save(&f.inventory,&inv);
    let r=f.write();assert_eq!(r["records_ready"],false);assert!(r["findings"][0]["message"].as_str().unwrap().contains("MAP_BLOCK_UNACCOUNTED"));
    assert!(!f.map("link_map").exists());
}
#[test]
fn actual_rerun_records_selector_change(){
    let f=Fixture::new();let r=f.write();assert_eq!(r["records_ready"],true,"{r}");
    let mut d=f.deliveries.clone();d[0]["target"]=json!({"quote":"正文 [1](#^ref-a)\n^body"});
    d[3]["target"]=json!({"quote":"补充材料 ^si-body"});
    let destination=f.root.path().join("changed");
    let output=invoke_rerun(&f,&destination,&d);
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    let payload:Value=serde_json::from_slice(&output.stdout).unwrap();
    let p=PathBuf::from(payload["result"].as_str().unwrap());
    let r=pipeline::read_json(&p).unwrap();assert_eq!(r["actual_rerun_performed"],true);
    assert_eq!(r["mapping_equal"],false);assert!(!r["differences"].as_array().unwrap().is_empty());
    let ordered=r["differences"].as_array().unwrap().iter()
        .map(|row|(&row["map"],&row["collection"],&row["id"])).collect::<Vec<_>>();
    assert_eq!(ordered,vec![(&json!("source_map"),&json!("units"),&json!("body")),
        (&json!("si_map"),&json!("units"),&json!("si-body"))]);
    assert_eq!(r["differences"][0]["previous"]["target"],json!({"anchor":"body"}));
    assert_eq!(r["differences"][1]["rerun"]["target"],json!({"quote":"补充材料 ^si-body"}));
}


#[test]
fn reject_bibliography_call_without_explicit_citation_unit(){
    let f=Fixture::new();
    let mut inv=pipeline::read_json(&f.inventory).unwrap();
    inv["units"].as_array_mut().unwrap().retain(|u|u["id"]!="call");
    save(&f.inventory,&inv);
    let mut deliveries=f.deliveries.clone();
    deliveries.as_array_mut().unwrap().retain(|d|d["unit"]!="call");
    // Source display data is explicit so failure must be the missing citation
    // unit, not an unrelated missing display-text prerequisite.
    let body=inv["units"].as_array_mut().unwrap().iter_mut().find(|u|u["id"]=="body").unwrap();
    body["display_text"]=json!("[1]");save(&f.inventory,&inv);
    let r=maps::write(&f.inventory,&f.candidate,&deliveries,&f.paths,&f.expected,|_,_,_|{}).unwrap();
    assert_eq!(r["records_ready"],false);
    assert!(r["findings"][0]["message"].as_str().unwrap().contains("MAP_OUTPUT_CITATION_UNMAPPED"),"{r}");
}
#[test]
fn author_metadata_requires_literal_affiliation_mapping_or_reason(){
    let f=Fixture::new();
    let mut inv=pipeline::read_json(&f.inventory).unwrap();
    let body=inv["units"].as_array_mut().unwrap().iter_mut().find(|u|u["id"]=="body").unwrap();
    body["kind"]=json!("metadata");body["field"]=json!("author");
    save(&f.inventory,&inv);
    let r=f.write();
    assert_eq!(r["records_ready"],false);assert!(r["findings"][0]["message"].as_str().unwrap().contains("MAP_AFFILIATION_REFERENCE"),"{r}");
}

#[test]
fn validates_explicit_merged_block_view_and_rerun_reports_changed_partition(){
    let f=Fixture::new();assert_eq!(f.write()["records_ready"],true);
    let mut source=pipeline::read_json(&f.map("source_map")).unwrap();
    let rows=source["blocks"].as_array().unwrap();
    let mut combined=rows[0].clone();
    combined["block_id"]=json!("merged-main");
    combined["source_blocks"]=json!(["b1","b2"]);
    combined["source_locators"]=json!([rows[0]["source_locator"],rows[1]["source_locator"]]);
    combined.as_object_mut().unwrap().remove("source_locator");
    combined["joiner"]=json!("\n");
    let original=format!("{}\n{}",rows[0]["original_text"].as_str().unwrap(),rows[1]["original_text"].as_str().unwrap());
    combined["original_text"]=json!(original);
    combined["original_sha256"]=json!(files::digest(original.as_bytes()));
    combined["target_units"]=json!(["body","call","reference"]);
    combined.as_object_mut().unwrap().remove("target_unit");
    combined["kind"]=json!("mixed");
    source["blocks"]=json!([combined]);
    save(&f.map("source_map"),&source);
    let mut links=pipeline::read_json(&f.map("link_map")).unwrap();
    links["internal_links"][0]["origin_block"]=json!("merged-main");
    save(&f.map("link_map"),&links);
    let r=f.validate();assert_eq!(r["records_ready"],true,"{r}");
    let p=mapping_compare::record_mapping_rerun(&f.run,&f.deliveries,&f.root.path().join("merged-rerun"),|_,_,_|{}).unwrap();
    let r=pipeline::read_json(&p).unwrap();
    assert_eq!(r["actual_rerun_performed"],true);assert_eq!(r["mapping_equal"],false);
    let mut bad=source.clone();bad["blocks"][0]["joiner"]=json!(" ");
    save(&f.map("source_map"),&bad);
    assert_eq!(f.validate()["records_ready"],false);
    let mut bad=source.clone();bad["blocks"][0]["source_blocks"]=json!(["b1","b1"]);
    save(&f.map("source_map"),&bad);
    assert_eq!(f.validate()["records_ready"],false);
    let mut bad=source.clone();bad["blocks"][0]["target_units"]=json!(["body","reference"]);
    save(&f.map("source_map"),&bad);
    assert_eq!(f.validate()["records_ready"],false);
    let mut bad=source.clone();bad["blocks"][0]["kind"]=json!("paragraph");
    save(&f.map("source_map"),&bad);
    assert_eq!(f.validate()["records_ready"],false);
    save(&f.map("source_map"),&source);
    links["internal_links"][0]["origin_block"]=json!("s1");
    save(&f.map("link_map"),&links);
    assert_eq!(f.validate()["records_ready"],false);
}

#[test]
fn rerun_checks_explicit_original_source_scope_before_writing(){
    let f=Fixture::new();assert_eq!(f.write()["records_ready"],true);
    let mut run=pipeline::read_json(&f.run).unwrap();
    run["scope"]["source_ids"]=json!(["main"]);
    save(&f.run,&run);
    let output=f.root.path().join("invalid-scope-rerun");
    assert!(mapping_compare::record_mapping_rerun(&f.run,&f.deliveries,&output,|_,_,_|{}).is_err());
    assert!(!output.exists());
}

#[test]
fn rerun_validates_legacy_empty_view_omissions_without_weakening_coverage(){
    let f=Fixture::new();assert_eq!(f.write()["records_ready"],true);
    let mut source=pipeline::read_json(&f.map("source_map")).unwrap();
    source.as_object_mut().unwrap().remove("metadata");
    save(&f.map("source_map"),&source);
    let mut si=pipeline::read_json(&f.map("si_map")).unwrap();
    for key in ["metadata","objects","references","citations"]{
        assert_eq!(si.as_object_mut().unwrap().remove(key),Some(json!([])));
    }
    save(&f.map("si_map"),&si);
    let mut link=pipeline::read_json(&f.map("link_map")).unwrap();
    for key in ["external_links","resource_links"]{
        assert_eq!(link.as_object_mut().unwrap().remove(key),Some(json!([])));
    }
    save(&f.map("link_map"),&link);
    let old=fs::read(f.map("si_map")).unwrap();
    let r=f.validate();assert_eq!(r["records_ready"],true,"{r}");
    let p=mapping_compare::record_mapping_rerun(&f.run,&f.deliveries,&f.root.path().join("legacy-empty-rerun"),|_,_,_|{}).unwrap();
    let r=pipeline::read_json(&p).unwrap();
    assert_eq!(r["actual_rerun_performed"],true);
    assert_eq!(r["mapping_equal"],true);
    assert_eq!(r["acceptance_status_written"],false);
    assert_eq!(fs::read(f.map("si_map")).unwrap(),old);

    let mut malformed=link.clone();
    malformed["external_links"]=Value::Null;
    save(&f.map("link_map"),&malformed);
    let r=f.validate();assert_eq!(r["records_ready"],false,"{r}");
    save(&f.map("link_map"),&link);
    source.as_object_mut().unwrap().remove("objects");
    save(&f.map("source_map"),&source);
    let r=f.validate();assert_eq!(r["records_ready"],false,"{r}");
}

#[test]
fn historical_omission_cannot_hide_an_actual_external_link(){
    let f=Fixture::new();
    let candidate=fs::read_to_string(&f.candidate).unwrap().replace(
        "补充材料 ^si-body","补充材料 [source](https://example.invalid) ^si-body");
    fs::write(&f.candidate,candidate).unwrap();
    let mut inventory=pipeline::read_json(&f.inventory).unwrap();
    let si=inventory["units"].as_array_mut().unwrap().iter_mut().find(|v|v["id"]=="si-body").unwrap();
    si["display_text"]=json!("Source link");
    save(&f.inventory,&inventory);
    let r=f.write();assert_eq!(r["records_ready"],true,"{r}");
    let mut link=pipeline::read_json(&f.map("link_map")).unwrap();
    let removed=link.as_object_mut().unwrap().remove("external_links").unwrap();
    assert_eq!(removed.as_array().unwrap().len(),1);
    save(&f.map("link_map"),&link);
    let r=f.validate();
    assert_eq!(r["records_ready"],false,"{r}");
    assert!(r["findings"][0]["message"].as_str().unwrap().contains("MAP_COLLECTION"),"{r}");
}

#[test]
fn writes_validates_and_reruns_multiline_reference_links(){
    let f=Fixture::new();
    let raw=fs::read_to_string(&f.candidate).unwrap().replace("[1](#^ref-a)","[1][引用]")
        +"\r\n[引用]:\r\n    <#^ref-a>\r\n    \"title https://hidden.example/\r\n    continued\"\r\n";
    fs::write(&f.candidate,&raw).unwrap();
    let mut deliver=f.deliveries.clone();
    deliver[1]["target"]=json!({"quote":"[1][引用]"});
    let r=maps::write(&f.inventory,&f.candidate,&deliver,&f.paths,&f.expected,|_,_,_|{}).unwrap();
    assert_eq!(r["records_ready"],true,"{r}");
    let links=pipeline::read_json(&f.map("link_map")).unwrap();
    assert_eq!(links["internal_links"].as_array().unwrap().len(),1);
    assert_eq!(links["external_links"],json!([]));
    let link=&links["internal_links"][0];
    assert_eq!(link["syntax"],"reference_markdown");
    assert_eq!(link["href"],"#^ref-a");
    assert_eq!(link["origin_unit"],"call");
    assert_eq!(link["target_unit"],"reference");
    assert_eq!(f.validate()["records_ready"],true);
    let path=mapping_compare::record_mapping_rerun(&f.run,&deliver,&f.root.path().join("reference-rerun"),|_,_,_|{}).unwrap();
    assert_eq!(pipeline::read_json(&path).unwrap()["mapping_equal"],true);
}

#[test]
fn complete_workflow_cli_replays_compact_maps_using_only_actual_manifest_and_covered_units() {
    let f=Fixture::new();
    assert_eq!(f.write()["records_ready"],true);
    let mut run=pipeline::read_json(&f.run).unwrap();
    let root=f.root.path();
    let manifest=root.join("manifest.json");
    run["workflow_profile"]=json!("complete_pdf_candidate_v1");
    run["scope"]["source_ids"]=json!(["main","si"]);
    run["scope"]["translation_required"]=json!(false);
    run["config"]["skills_root"]=json!(root);
    run["config"]["resource_root"]=json!(root);
    run["records"]["delivery_manifest"]=json!(manifest);
    save(&f.run,&run);
    let original_manifest=json!({"schema_version":1,"candidate":binding(&f.candidate),
        "inventory":binding(&f.inventory),"resource_root":root,"figure_edge_reviews_required":true,
        "deliveries":f.deliveries});
    save(&manifest,&original_manifest);
    for p in [f.map("source_map"),f.map("si_map")]{
        let mut doc=pipeline::read_json(&p).unwrap();
        let fields=doc.as_object_mut().unwrap();
        for k in ["units","status","review_status","source_fidelity","metadata"]{fields.remove(k);}
        for block in doc["blocks"].as_array_mut().unwrap(){block.as_object_mut().unwrap().remove("status");}
        save(&p,&doc);
    }
    let main_path=f.map("source_map");
    let link_path=f.map("link_map");
    let mut main=pipeline::read_json(&main_path).unwrap();
    main.as_object_mut().unwrap().remove("sources");
    save(&main_path,&main);
    let mut links=pipeline::read_json(&link_path).unwrap();
    for anchor in links["anchors"].as_array_mut().unwrap(){
        let a=anchor.as_object_mut().unwrap();
        for k in ["target_unit","candidate_locator","review_status"]{a.remove(k);}
    }
    for k in ["status","review_status","source_fidelity"]{links.as_object_mut().unwrap().remove(k);}
    save(&link_path,&links);
    assert_eq!(f.validate()["records_ready"],false,"standalone validation has no separately bound manifest");

    let request=root.join("inspect-request.json");
    save(&request,&json!({"schema_version":1,"operation":"complete_workflow.inspect_workflow","args":[f.run]}));
    let inspect=|| {
        let output=std::process::Command::new(env!("CARGO_BIN_EXE_z2o")).arg("invoke").arg(&request)
            .env("PATH","").output().unwrap();
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["result"].clone()
    };
    let before=[f.run.clone(),f.inventory.clone(),f.candidate.clone(),manifest.clone(),
        main_path.clone(),f.map("si_map"),link_path.clone()]
        .into_iter().map(|p|{let bytes=fs::read(&p).unwrap();(p,bytes)}).collect::<Vec<_>>();
    let r=inspect();
    assert_eq!(r["stage_reports"]["handoff"]["records_ready"],true,"{}",r["stage_reports"]["handoff"]);
    assert_eq!(r["records_ready"],false);
    assert_eq!(r["source_fidelity"],"not_assessed");
    for (path,bytes) in before{assert_eq!(fs::read(path).unwrap(),bytes);}
    let unfinished=|code:&str|{
        let r=inspect();let h=&r["stage_reports"]["handoff"];
        assert_eq!(h["records_ready"],false,"{h}");
        assert!(h["findings"][0]["message"].as_str().unwrap().contains(code),"{h}");
    };
    let mut absent=main.clone();
    absent["blocks"].as_array_mut().unwrap().pop();
    save(&main_path,&absent);unfinished("MAP_BLOCK_MISSING");save(&main_path,&main);
    let mut forgery=links.clone();
    forgery["anchors"][0]["source_id"]=json!("nonexistent-source");
    save(&link_path,&forgery);unfinished("MAP_ANCHOR_SOURCE_OR_TARGET");save(&link_path,&links);
    let mut weakened=original_manifest.clone();
    weakened["candidate"]["sha256"]=json!("0".repeat(64));
    save(&manifest,&weakened);unfinished("MAP_LEGACY_MANIFEST_BINDING");
    let mut incomplete=original_manifest.clone();
    incomplete["deliveries"].as_array_mut().unwrap().pop();
    save(&manifest,&incomplete);unfinished("MAP_DELIVERY_SET");
    save(&manifest,&original_manifest);
    let mut explicit_null=main.clone();explicit_null["units"]=Value::Null;
    save(&main_path,&explicit_null);unfinished("MAP_COLLECTION");
}
