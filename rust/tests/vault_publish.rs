use serde_json::{json,Value};
use std::{fs,path::Path};
use z2o_native::{check_ledger,vault_publish::{self,Options}};
fn save(p:&Path,v:&Value)->String{
    fs::write(p,serde_json::to_vec(v).unwrap()).unwrap();
    check_ledger::bind_file(p).unwrap()["sha256"].as_str().unwrap().to_owned()
}
#[test]
fn preflight_is_read_only_binds_embeds_and_preserves_occupied_targets(){
    let t=tempfile::tempdir().unwrap();let root=t.path();
    let vault=root.join("vault");let work=root.join("work");let resources=root.join("candidate-resources");
    fs::create_dir_all(vault.join(".obsidian")).unwrap();
    fs::create_dir_all(vault.join("文献")).unwrap();fs::create_dir_all(&work).unwrap();
    fs::create_dir_all(resources.join("文献/attachments/2020/Smith/paper")).unwrap();
    let asset=resources.join("文献/attachments/2020/Smith/paper/fig1.png");
    fs::write(&asset,b"actual fixture asset bytes").unwrap();
    let source=work.join("source.pdf");fs::write(&source,b"actual fixture source").unwrap();
    let candidate=work.join("final.md");
    fs::write(&candidate,"---\n年份: 2020\n---\n# Paper\n\n![[文献/attachments/2020/Smith/paper/fig1.png]]\n").unwrap();
    let b=check_ledger::bind_file(&source).unwrap();
    let run=work.join("run.json");
    let hash=save(&run,&json!({"schema_version":1,"workflow_profile":"complete_pdf_candidate_v1",
        "scope":{"write_vault":false},"identity":{"sources":[{"id":"main","path":b["path"],"sha256":b["sha256"]}]},
        "config":{"work_root":work,"candidate_path":candidate,"resource_root":resources},
        "naming":{"status":"planned","filename":"2020-Smith-Paper.md"}}));
    let plan=|relative:&str,expected:Option<&str>|vault_publish::plan(Options{
        run:&run,run_sha:&hash,vault:&vault,literature_relative:"文献",assets_relative:"文献/attachments",
        note_relative:relative,expected_existing_sha:expected,inspect_workflow:false},|_,_,_|{});
    let note="文献/2020-Smith-Paper.md";
    let r=plan(note,None).unwrap();
    assert_eq!(r["vault_modified"],false);assert_eq!(r["preconditions_ready"],false);
    assert_eq!(r["resources"][0]["disposition"],"create_if_absent");
    assert!(!vault.join("文献/attachments").exists());assert!(!vault.join(note).exists());
    assert!(plan("文献/../escape.md",None).is_err());
    fs::write(vault.join(note),b"existing user note").unwrap();
    let actual=plan(note,None).unwrap();
    assert!(actual["findings"].as_array().unwrap().iter().any(|f|f["code"]=="TARGET_PRECONDITION_MISMATCH"));
    let dst=vault.join("文献/attachments/2020/Smith/paper/fig1.png");fs::create_dir_all(dst.parent().unwrap()).unwrap();
    fs::write(&dst,b"old asset").unwrap();
    let r=plan(note,None).unwrap();
    assert_eq!(r["resources"][0]["disposition"],"conflict");
    assert_eq!(fs::read(&dst).unwrap(),b"old asset");assert_eq!(fs::read(vault.join(note)).unwrap(),b"existing user note");
}
