use serde_json::{json,Value};
use std::{fs,path::Path};
use z2o_native::{vault_library,check_ledger};
fn save(p:&Path,v:&Value)->String{
    fs::write(p,serde_json::to_vec(v).unwrap()).unwrap();
    check_ledger::bind_file(p).unwrap()["sha256"].as_str().unwrap().to_owned()
}
#[test]
fn actual_vault_index_and_bibliographic_identity_candidate(){
    let t=tempfile::tempdir().unwrap();let root=t.path().join("vault");
    fs::create_dir_all(root.join(".obsidian")).unwrap();
    fs::create_dir_all(root.join("文献").join("template")).unwrap();
    fs::write(root.join("文献/template/ignored.md"),b"ignored template").unwrap();
    let p=root.join("文献/2020-Smith-Paper.md");
    fs::write(&p,"---\n年份: 2020\n---\n# Paper\n\n[Zotero 原条目](zotero://select/library/items/PAPER001)\n").unwrap();
    let index=vault_library::index(&root,"文献",&json!([]),Some(123),|_,_,_|{}).unwrap();
    assert_eq!(index["notes"].as_array().unwrap().len(),1);
    let cat=json!({"schema_version":1,"library":{"type":"user","id":123},"pagination_complete":true,"local_version_consistent":true,
        "items":[{"library":{"type":"user","id":123},"key":"PAPER001","data":{"key":"PAPER001","title":"Paper","date":"2020","itemType":"journalArticle","DOI":"10.1234/paper","creators":[{"creatorType":"author","firstName":"Jane","lastName":"Smith"}]}}]});
    let ip=t.path().join("index.json");let cp=t.path().join("catalog.json");
    let ih=save(&ip,&index);let ch=save(&cp,&cat);
    let reference=json!({"title":"Paper","year":"2020","item_type":"journalArticle","doi":"10.1234/paper","authors":["Jane Smith"]});
    let result=vault_library::match_reference(&ip,&ih,&cp,&ch,&reference).unwrap();
    assert_eq!(result["status"],"unique_bibliographic_candidate_requires_version_review");
    assert_eq!(result["match_confirmed"],false);
    let mut mismatch=reference.clone();mismatch["item_type"]=json!("preprint");
    assert_eq!(vault_library::match_reference(&ip,&ih,&cp,&ch,&mismatch).unwrap()["status"],"ambiguous");
    fs::write(&p,b"locally modified").unwrap();
    assert!(vault_library::match_reference(&ip,&ih,&cp,&ch,&reference).is_err());
}
