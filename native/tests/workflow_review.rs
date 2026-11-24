use serde_json::{json,Value};
use std::{fs,path::Path};
use z2o_native::{workflow,delivery_validate,files};
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
