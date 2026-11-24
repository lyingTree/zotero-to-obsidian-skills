use serde_json::{json,Value};
use std::{fs,process::Command,path::Path};
use tempfile::tempdir;
use z2o_native::{invoke,parse_json,encoded_bytes,files,metadata,naming,properties,figures,tables};

fn call(operation:&str,args:Value,kwargs:Value)->Value{
    invoke(json!({"schema_version":1,"operation":operation,"args":args,"kwargs":kwargs})).unwrap()["result"].clone()
}
#[test]
fn json_rejects_nested_duplicates_and_unsupported_calls(){
    assert!(parse_json(r#"{"args":[{"a":1,"a":2}]}"#).is_err());
    assert!(parse_json(r#"{"x":NaN}"#).is_err());
    assert!(invoke(json!({"schema_version":1,"operation":"os.system"})).is_err());
    assert!(invoke(json!({"schema_version":1,"operation":"note_pipeline.process_content"})).is_err());
    assert!(invoke(json!({"schema_version":1,"operation":"deterministic_stage_checks.author_token",
        "args":[["A"]],"kwargs":{"confirmed_author_markers":["B"]}})).is_err());
}
#[test]
fn author_counts_and_unicode(){
    for (names,expected) in [(json!(["欧阳"]), "欧阳"),(json!(["van der Waals","Smith Jr."]),"van der Waals and Smith Jr."),
        (json!(["联合国","B","C"]),"联合国 et al")]{
        assert_eq!(metadata::author_token(&names,&Value::Null).unwrap(),expected);
    }
    assert!(metadata::author_token(&json!([]),&Value::Null).is_err());
    assert_eq!(metadata::author_token(&json!([]),&json!("source lacks authors")).unwrap(),"未知作者");
    assert!(metadata::author_token(&json!([" A"]),&Value::Null).is_err());
}
#[test]
fn snapshot_paths_and_hashes(){
    let dir=tempdir().unwrap();
    let p=dir.path().join("论文 空格 🌊.md");
    fs::write(&p,b"abc").unwrap();
    let snap=files::snapshot(&p).unwrap();
    assert_eq!(snap["sha256"],"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(snap["size"],3);
    assert!(files::verify_binding(&snap,&p,dir.path()).unwrap()["matches"]==true);
    let copy=dir.path().join("copy.md");fs::write(&copy,b"abc").unwrap();
    assert!(files::verify_binding(&snap,&copy,dir.path()).unwrap()["matches"]==false);
    let empty=dir.path().join("empty");fs::write(empty,b"").unwrap();
    let rows=files::inspect_relative_files(dir.path(),&json!(["论文 空格 🌊.md","../escape","C:/escape","a\\b","empty","missing"])).unwrap();
    assert_eq!(rows.as_array().unwrap().iter().map(|v|v["status"].as_str().unwrap()).collect::<Vec<_>>(),
        vec!["readable","blocked","blocked","blocked","empty","blocked"]);
}
#[cfg(unix)]
#[test]
fn resource_symlink_cannot_escape(){
    use std::os::unix::fs::symlink;
    let root=tempdir().unwrap();let outside=tempdir().unwrap();
    fs::write(outside.path().join("x"),b"outside").unwrap();
    symlink(outside.path().join("x"),root.path().join("x")).unwrap();
    assert_eq!(files::inspect_relative_files(root.path(),&json!(["x"])).unwrap()[0]["status"],"blocked");
}
#[test]
fn collection_cycles_and_missing_parents(){
    let library=json!({"type":"user","id":123});
    let item=json!({"library":library,"data":{"collections":["B"]}});
    let a=json!({"key":"A","library":library,"data":{"key":"A","name":"Root","parentCollection":false}});
    let b=json!({"key":"B","library":library,"data":{"key":"B","name":"Child","parentCollection":"A"}});
    let records=json!([b,a]);
    let valid=metadata::collection_ancestry(&item,&records,true).unwrap();
    assert_eq!(valid["collections"][0]["ancestry"][0]["name"],"Root");
    assert_eq!(valid["status"],"complete");
    assert_eq!(metadata::collection_ancestry(&item,&records,false).unwrap()["status"],"incomplete");
    assert_eq!(metadata::collection_ancestry(&item,&json!([records[0]]),true).unwrap()["status"],"incomplete");
    let mut cyclic=records;cyclic[1]["data"]["parentCollection"]=json!("B");
    assert_eq!(metadata::collection_ancestry(&item,&cyclic,true).unwrap()["issues"][0]["reason"],"collection_cycle");
}
#[test]
fn properties_new_and_repair_exact_bytes(){
    let original=b"\xef\xbb\xbf# Title\r\n\r\n---\r\nText\r\n";
    let fields=json!({"类型":"期刊论文","年份":2024,"关键词":["a: b","a # b","2020"],"期刊名":"J: X"});
    let result=properties::build(original,"journalArticle",&fields).unwrap();
    assert_eq!(properties::split(&result).unwrap().body,&original[3..]);
    assert!(properties::build(&result,"journalArticle",&fields).is_err());
    let old="---\r\n类型: '期刊论文'\r\n库内参考文献:\r\n  - 'old'\r\ncustom: \"2020\"\r\nnested:\r\n  库内参考文献: retain\r\n---\r\n# Title\r\n";
    let expected=old.replacen("库内参考文献:\r\n  - 'old'\r\n","",1);
    let repaired=properties::repair(old.as_bytes()).unwrap();
    assert_eq!(repaired,expected.as_bytes());
    assert_eq!(properties::repair(&repaired).unwrap(),repaired);
    assert!(properties::repair(b"---\nx: 1\nx: 2\n---\n").is_err());
    assert!(properties::repair("---\n库内参考文献: &a [1]\ncustom: *a\n---\n".as_bytes()).is_err());
    assert!(properties::split(b"---\nx: 1").is_err());
    assert!(properties::build(b"# x\n","thesis",&json!({"类型":"学位论文","年份":true,"关键词":[]})).is_err());
}
#[test]
fn atomic_check_status_and_source_fidelity(){
    let dir=tempdir().unwrap();let a=dir.path().join("a.md");let b=dir.path().join("b.md");
    let input=b"# Title\n";fs::write(&a,input).unwrap();
    fs::write(&b,properties::build(input,"thesis",&json!({"类型":"学位论文","年份":2024,"关键词":[]})).unwrap()).unwrap();
    let report=properties::audit(&a,&b,"new","thesis").unwrap();
    assert_eq!(report["checks"]["PROP-A08"]["status"],"passed");
    assert_eq!(report["checks"]["PROP-A06"]["status"],"passed");
    assert_eq!(report["source_fidelity"],"not_assessed");
    fs::write(&b,b"---\nx: 1\nx: 2\n---\n# Title\n").unwrap();
    assert_eq!(properties::audit(&a,&b,"new","thesis").unwrap()["checks"]["PROP-A02"]["status"],"failed");
    assert!(properties::audit_write(&a,&a).is_err());
}
#[test]
fn naming_plan_unicode_limits_and_evidence(){
    let dir=tempdir().unwrap();let p=dir.path().join("source.txt");fs::write(&p,"真实题名").unwrap();
    let snap=files::snapshot(&p).unwrap();
    let file=json!({"path":snap["path"],"sha256":snap["sha256"]});
    let evidence=json!({"file":file,"locator":"line 1"});
    let args=json!({"year":2024,"author_token":"Smith","original_title":"a:🌊🌊🌊",
        "source_evidence":{"year":evidence,"author_token":evidence,"original_title":evidence},
        "max_utf16_units":17,"target_directory":dir.path().to_str().unwrap()});
    let plan=naming::build_plan(args.as_object().unwrap().clone()).unwrap();
    assert!(plan["length"]["filename_utf16_units"].as_u64().unwrap()<=17);
    assert_eq!(plan["title_mapping"]["cleaned"],"a_🌊🌊🌊");
    assert_eq!(naming::validate_plan(&plan,&Value::Null).unwrap()["structurally_consistent"],true);
    let mut altered=plan.clone();altered["filename"]=json!("incorrect.md");
    assert_eq!(naming::validate_plan(&altered,&Value::Null).unwrap()["structurally_consistent"],false);
    fs::write(&p,"changed").unwrap();
    assert_eq!(naming::validate_plan(&plan,&Value::Null).unwrap()["structurally_consistent"],false);
}
#[test]
fn normalization_policy_and_casefold(){
    let dir=tempdir().unwrap();
    fs::write(dir.path().join("Straße.md"),b"x").unwrap();
    let result=naming::conflicts("STRASSE.md",dir.path(),false,"NFC").unwrap();
    assert_eq!(result["status"],"name_collision");
    assert_eq!(result["overwrite_authorized"],false);
    assert_eq!(naming::conflicts("STRASSE.md",dir.path(),true,"NFC").unwrap()["status"],"clear_in_directory_snapshot");
}
#[test]
fn rich_tables_all_sheets_and_examples(){
    let dir=tempdir().unwrap();let file=dir.path().join("table.md");
    let fence="\x60\x60\x60";
    let text=format!("{fence}text\n{fence}rich-table\n{fence}\n{fence}rich-table\nversion: 3\nsheets:\n  - locked: true\n    theme: grid\n    columns: [a]\n    rows: [b]\n  - locked: false\n    theme: grid\n    columns: [a]\n    rows: [b]\n    footer: 123\n{fence}\n^tbl-1\n");
    fs::write(&file,text).unwrap();
    let report=tables::audit(&file).unwrap();
    assert_eq!(report["tables"].as_array().unwrap().len(),1);
    assert_eq!(report["checks"]["BODY-A84"]["status"],"failed");
    assert_eq!(report["checks"]["BODY-A85"]["status"],"passed");
    assert_eq!(report["checks"]["BODY-A111"]["status"],"failed");
    fs::write(&file,"# Empty scope\n").unwrap();
    assert_eq!(tables::audit(&file).unwrap()["checks"]["BODY-A83"]["status"],"not_verified");
}
#[test]
fn figure_blocks_and_si_ids(){
    let block=figures::build(&json!([{"target":"a#b.png","alt":"[A]"}]),"**图1：** 内容","fig-1","\r\n").unwrap();
    assert_eq!(block,"![\\[A\\]](<a%23b.png>)  \r\n**图1：** 内容\r\n^fig-1\r\n");
    assert_eq!(figures::new_anchor("ab12","S7c",&json!([])).unwrap(),"^tbl-fig-ab12-S7c");
    assert!(figures::new_anchor("ab12","S7c",&json!(["tbl-fig-ab12-S7c"])).is_err());
    assert_eq!(figures::supplement(&json!("7c"),"内容",&Value::Null,&Value::Null).unwrap()["caption"],
        "**图S7c：** 内容（来源原号：7c）");
}
#[test]
fn native_binary_with_empty_path(){
    let dir=tempdir().unwrap();
    let empty=dir.path().join("empty-path");fs::create_dir(&empty).unwrap();
    let binary=Path::new(env!("CARGO_BIN_EXE_z2o"));
    let result=Command::new(binary).arg("describe").current_dir(dir.path()).env("PATH",&empty)
        .env_remove("PYTHONHOME").env_remove("PYTHONPATH").output().unwrap();
    assert!(result.status.success());
    let description:Value=serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(description["engine"],"rust-native");
    assert_eq!(description["system_python_required"],false);
    assert_eq!(description["full_workflow_ready"],false);
    let request=dir.path().join("request.json");
    fs::write(&request,serde_json::to_vec(&json!({"schema_version":1,"operation":"deterministic_stage_checks.author_token",
        "args":[["欧阳","Smith"]]})).unwrap()).unwrap();
    let output=Command::new(binary).args(["invoke"]).arg(&request).current_dir(dir.path()).env("PATH",&empty).output().unwrap();
    assert!(output.status.success());
    assert_eq!(serde_json::from_slice::<Value>(&output.stdout).unwrap()["result"],"欧阳 and Smith");
    assert_eq!(call("deterministic_stage_checks.split_frontmatter",json!([encoded_bytes(b"# x\n")]),json!({}))[1],
        encoded_bytes(b"# x\n"));
}
