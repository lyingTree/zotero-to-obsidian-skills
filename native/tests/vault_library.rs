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

#[test]
fn actual_cli_vault_index_does_not_confuse_terminal_citations_with_item_footers() {
    use std::process::Command;
    let temp=tempfile::tempdir().unwrap();
    let root=temp.path().join("研究 vault");
    let lit=root.join("文献");
    fs::create_dir_all(root.join(".obsidian")).unwrap();
    fs::create_dir(&lit).unwrap();
    let misleading="[Zotero原条目](zotero://select/library/items/PAPER001)";
    // The same exact link decoded in two YAML fields has no unique raw link
    // span. The table scanner may legitimately fall back to the whole fence.
    let table=json!({"version":2,"title":misleading,"footer":misleading,"columns":[],"rows":[]});
    // A single escaped spelling can trigger that fallback as well. This
    // catches the case where the old index even declared a unique locator.
    let escaped_table=json!({"version":2,"title":misleading,"columns":[],"rows":[]}).to_string()
        .replacen(misleading,&misleading.replacen('[',"\\u005b",1),1);
    let cases=[
        ("01-正文末句.md",format!("这一段引用了另一个 Zotero 条目：{misleading}\n")),
        ("02-表格字段.md",format!("~~~rich-table\n{table}\n~~~\n")),
        ("03-引文本.md",format!("> {misleading}\n")),
        ("04-列表项.md",format!("- {misleading}\n")),
        ("05-重复题头.md",format!("## [Zotero原条目](zotero://select/library/items/OTHER001)\n\n文本。\n\n## {misleading}\n")),
        ("06-真正末项.md",format!("---\r\n年份: 2020\r\n---\r\n\r\n# Pacific waves\r\n\r\n另见 [Zotero原条目](zotero://select/library/items/OTHER001)。\r\n\r\n---\r\n\r\n## {misleading}\r\n\r\n")),
        ("07-历史单行.md","正文。\n\n[Zotero 原条目](zotero://select/library/items/LEGACY01)\n".to_owned()),
        ("08-团队末项.md","正文。\n\n---\n\n## [Zotero原条目](zotero://select/groups/71/items/GROUP001)\n".to_owned()),
        ("09-注释链接.md",format!("<!--\n## {misleading}\n-->\n")),
        ("10-代码链接.md",format!("~~~text\n## {misleading}\n~~~\n")),
        ("11-段落换行.md",format!("同一段落中的引用：\n{misleading}\n")),
        ("12-列表延续.md",format!("- 讨论其他条目：\n\n  {misleading}\n")),
        ("13-转义表格.md",format!("~~~rich-table\n{escaped_table}\n~~~\n")),
    ];
    for (name,bytes) in &cases {fs::write(lit.join(name),bytes).unwrap();}
    let exe=std::env::var_os("Z2O_VAULT_LIBRARY_TEST_CLI").unwrap_or_else(||env!("CARGO_BIN_EXE_z2o").into());
    let request=temp.path().join("请求.json");
    let invoke=|operation:&str,args:Value,kwargs:Value|->Value{
        save(&request,&json!({"schema_version":1,"operation":operation,"args":args,"kwargs":kwargs}));
        let output=Command::new(&exe).arg("invoke").arg(&request).env("PATH","").env_remove("PYTHONPATH").output().unwrap();
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["result"].clone()
    };
    let index_path=temp.path().join("绑定 index.json");
    let index=invoke("vault_library.index",json!([root,"文献"]),json!({"personal_library_id":123,"output_path":index_path}));
    let records=index["notes"].as_array().unwrap();
    assert_eq!(records.len(),cases.len(),"{index}");
    assert!(index["issues"].as_array().unwrap().is_empty(),"{index}");
    let note=|name:&str|records.iter().find(|row|row["relative_path"]==format!("文献/{name}")).unwrap();
    for name in ["01-正文末句.md","02-表格字段.md","03-引文本.md","04-列表项.md","09-注释链接.md","10-代码链接.md",
        "11-段落换行.md","12-列表延续.md","13-转义表格.md"]{
        assert_eq!(note(name)["status"],"item_locator_missing","{name}: {}",note(name));
        assert_eq!(note(name)["identity_candidates"],json!([]));
    }
    let ambiguous=note("05-重复题头.md");
    assert_eq!(ambiguous["status"],"ambiguous_item_locator");
    assert_eq!(ambiguous["identity_candidates"][0]["item_key"],"OTHER001");
    assert_eq!(ambiguous["identity_candidates"][1]["item_key"],"PAPER001");
    let direct=&note("06-真正末项.md")["identity_candidates"][0];
    let actual_text=&cases[5].1;
    let at=actual_text.rfind(misleading).unwrap();
    assert_eq!(direct["locator"]["start"].as_u64(),Some(actual_text[..at].chars().count() as u64));
    assert_eq!(direct["locator"]["end"].as_u64(),Some((actual_text[..at].chars().count()+misleading.chars().count()) as u64));
    assert_eq!(direct["locator"]["sha256"],z2o_native::files::digest(misleading.as_bytes()));
    assert_eq!(note("07-历史单行.md")["identity_candidates"][0]["item_key"],"LEGACY01");

    let row=|kind:&str,id:u64,key:&str,title:&str,doi:&str|json!({"key":key,"library":{"type":kind,"id":id},
        "data":{"key":key,"title":title,"date":"2020-02-03","itemType":"journalArticle","DOI":doi,
            "creators":[{"creatorType":"author","firstName":"Jane","lastName":"Smith"}]}});
    let catalog=|kind:&str,id:u64,rows:Value|json!({"schema_version":1,"library":{"type":kind,"id":id},
        "pagination_complete":true,"local_version_consistent":true,"items":rows});
    let personal_path=temp.path().join("personal.json");
    let group_path=temp.path().join("group.json");
    let personal_sha=save(&personal_path,&catalog("user",123,json!([
        row("user",123,"PAPER001","Pacific waves","10.1234/waves"),
        row("user",123,"OTHER001","Another paper","10.1234/other"),
        row("user",123,"LEGACY01","Historical paper","10.1234/historical"),
    ])));
    let group_sha=save(&group_path,&catalog("group",71,json!([row("group",71,"GROUP001","Group waves","10.1234/group")])));
    let reference=|title:&str,doi:&str|json!({"title":title,"year":"2020","item_type":"journalArticle","doi":doi,"authors":["Jane Smith"]});
    let match_note=|catalog_path:&Path,catalog_sha:&str,reference:Value|invoke("vault_library.match_reference",
        json!([index_path,catalog_path,reference]),json!({"expected_index_sha256":index["saved_record"]["sha256"],
            "expected_catalog_sha256":catalog_sha}));
    for (path,sha,reference,target) in [
        (personal_path.as_path(),personal_sha.as_str(),reference("Pacific waves","10.1234/waves"),"文献/06-真正末项.md"),
        (personal_path.as_path(),personal_sha.as_str(),reference("Historical paper","10.1234/historical"),"文献/07-历史单行.md"),
        (group_path.as_path(),group_sha.as_str(),reference("Group waves","10.1234/group"),"文献/08-团队末项.md"),
    ]{
        let result=match_note(path,sha,reference);
        assert_eq!(result["status"],"unique_bibliographic_candidate_requires_version_review","{result}");
        assert_eq!(result["candidates"].as_array().unwrap().len(),1,"{result}");
        assert_eq!(result["candidates"][0]["relative_path"],target);
        assert_eq!(result["match_confirmed"],false);
    }
    assert_eq!(match_note(&personal_path,&personal_sha,reference("Another paper","10.1234/other"))["status"],"unavailable");
    for (name,bytes) in &cases {assert_eq!(&fs::read_to_string(lit.join(name)).unwrap(),bytes);}
}
