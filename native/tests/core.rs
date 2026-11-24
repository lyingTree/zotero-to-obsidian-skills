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

#[test]
fn url_occurrences_preserve_unicode_offsets_and_one_to_one_mapping(){
    use z2o_native::source_urls;
    let raw="中文 🌊 (https://example.org/Fn_(x)),(https://example.org/Fn_(x)).";
    let inv=json!({"sources":[{"id":"main","pages":[{"page":1,"blocks":[{"id":"b1","text":raw}]}]}],"units":[
        {"id":"u1","kind":"metadata","field":"external_url","source":"main","value":"https://example.org/Fn_(x)","evidence":[{"page":1}]}
    ]});
    let report=source_urls::coverage(&inv,&json!([]),&json!([])).unwrap();
    assert_eq!(report["occurrences"].as_array().unwrap().len(),2);
    assert_eq!(report["unmapped"].as_array().unwrap().len(),1);
    let first=&report["occurrences"][0];
    let extracted:String=raw.chars().skip(first["start"].as_u64().unwrap() as usize)
        .take((first["end"].as_u64().unwrap()-first["start"].as_u64().unwrap()) as usize).collect();
    assert_eq!(extracted,first["raw_text"].as_str().unwrap());
    let value="https://example.org/repeated";
    let inv=json!({"sources":[{"id":"main","pages":[{"page":1,"blocks":[{"id":"b1","text":value},{"id":"b2","text":value}]}]}],
        "units":[{"id":"broad","kind":"metadata","field":"external_url","source":"main","value":value,"evidence":[{"page":1}]},
        {"id":"narrow","kind":"metadata","field":"external_url","source":"main","value":value,"evidence":[{"page":1,"blocks":["b1"]}]}]});
    let result=source_urls::coverage(&inv,&json!([]),&json!([])).unwrap();
    assert_eq!(result["unmapped"],json!([]));
    assert_eq!(result["recorded"][0]["unit"],"narrow");
    assert_eq!(result["recorded"][1]["unit"],"broad");
}


fn render_unit(uid:&str,kind:&str,scope:&str,extras:Value)->Value{
    let mut v=json!({"unit_id":uid,"kind":kind,"scope":scope,"source":{"source_id":"article","locator":format!("box/{uid}")}});
    v.as_object_mut().unwrap().extend(extras.as_object().unwrap().clone());v
}
fn render_fixture()->(Value,Value){
    let units=vec![
        render_unit("h","header","main",json!({"original_title":"Alpha_7","authors":[{"name":"A_B","numbers":["1"],"email":"a@test.example","corresponding":true}],"affiliations":[{"number":"1","name":"Institute"}],"translated":true})),
        render_unit("section","heading","main",json!({"level":2})),
        render_unit("body","paragraph","main",json!({"links":{"first":{"target":"r","label":"Example"}},"literals":{"scale":"$7.00$"}})),
        render_unit("f","figure","main",json!({"anchor":"fig-1","label":"图1。","images":[{"target":"a#b.png","alt":"x[y]"}]})),
        render_unit("eq","formula","main",json!({"latex":"a_b = 7.00\\tag{1}","anchor":"eq-1"})),
        render_unit("tab","table","main",json!({"data":{"version":2,"locked":true,"theme":"grid","columns":["a"],"rows":[["07.00"]]},"anchor":"tbl-1"})),
        render_unit("r","reference","main",json!({"source_text":"First entry_7.00"})),
        render_unit("note","translator_note","supplement:A",json!({})),
        render_unit("sr","bibliography","supplement:A",json!({"source_text":"Supplement 1."})),
        render_unit("end","zotero_link","main",json!({"uri":"zotero://select/library/items/ABCDEFGH"})),
    ];
    let plan=json!({"schema_version":1,"units":units,"properties":{"tags":["example"],"year":2020}});
    let mut content=z2o_native::render::content_template(&plan).unwrap();
    content["h"]["translated_title"]=json!("甲");
    content["section"]["text"]=json!("引言");
    content["body"]["text"]=json!("含{{literal:scale}}，见{{link:first}}；保留{not_a_template}。");
    content["f"]["caption"]=json!("图注。");
    content["note"]["text"]=json!("图中未标出。");
    (plan,content)
}
#[test]
fn renderer_preserves_structure_and_unicode_mapping(){
    let (plan,content)=render_fixture();
    let report=z2o_native::render::render_note(&plan,&content);
    assert_eq!(report["status"],"rendered","{report}");
    let text=report["text"].as_str().unwrap();
    assert!(text.contains("# Alpha\\_7\n甲\n\n[A\\_B](mailto:a@test.example)"));
    assert!(text.contains("## 1 引言\n含$7.00$，见[Example](<#^ref-1>)"));
    assert!(text.contains("rows:\n- - '07.00'"));
    assert!(text.contains("1. Supplement 1. ^sref-1"));
    assert!(text.contains("> 译注：图中未标出。"));
    for row in report["mapping"].as_array().unwrap(){
        let a=row["start_offset"].as_u64().unwrap() as usize;
        let b=row["end_offset"].as_u64().unwrap() as usize;
        let chunk:String=text.chars().skip(a).take(b-a).collect();
        assert_eq!(files::digest(chunk.as_bytes()),row["sha256"].as_str().unwrap());
        let line_a=row["start_line"].as_u64().unwrap() as usize-1;
        let line_b=row["end_line"].as_u64().unwrap() as usize;
        assert_eq!(chunk,text.lines().skip(line_a).take(line_b-line_a).collect::<Vec<_>>().join("\n"));
    }
    assert_eq!(report["source_fidelity"],"not_assessed");
}
#[test]
fn renderer_blocks_invalid_contracts_without_partial_output(){
    let (original_plan,original_content)=render_fixture();
    let mut cases=Vec::new();
    let mut p=original_plan.clone();p["units"][4]["anchor"]=json!("fig-1");cases.push((p,original_content.clone()));
    let mut p=original_plan.clone();p["units"][4]["latex"]=json!("a\n$$\n## injected");cases.push((p,original_content.clone()));
    let mut p=original_plan.clone();p["units"][1]["level"]=json!(4);cases.push((p,original_content.clone()));
    let mut p=original_plan.clone();p["units"][2]["literals"]["scale"]=json!("# forged");cases.push((p,original_content.clone()));
    let mut c=original_content.clone();c["eq"]["latex"]=json!("tamper");cases.push((original_plan.clone(),c));
    let mut c=original_content.clone();c["body"]["text"]=json!("{{link:first}}{{link:first}}");cases.push((original_plan.clone(),c));
    let mut c=original_content.clone();c["body"]["text"]=json!("{{link:unknown}}");cases.push((original_plan.clone(),c));
    let mut c=original_content.clone();c["body"]["text"]=json!("one\ntwo");cases.push((original_plan.clone(),c));
    for (p,c) in cases{
        let report=z2o_native::render::render_note(&p,&c);
        assert_eq!(report["status"],"blocked","{report}");
        assert_eq!(report["text"],Value::Null);
        assert_eq!(report["mapping"],json!([]));
        assert_eq!(report["findings"][0]["code"],"RENDER_INVALID");
    }
}
#[test]
fn table_writer_roundtrip_and_fence_length(){
    use z2o_native::yaml_emit;
    let v=json!({"version":3,"sheets":[{"locked":true,"theme":"grid","columns":[{"id":"c","name":"列"}],
        "rows":[{"cells":{"c":"\x60\x60\x60\x60 sample"}}],"footer":"2024-01-01"}]});
    let block=yaml_emit::rich_table(&v,"^tbl-1",true,"\r\n").unwrap();
    assert!(block.starts_with("\x60\x60\x60\x60\x60rich-table\r\n"));
    let lines:Vec<_>=block.lines().collect();
    let payload=lines[2..lines.len()-3].join("\n");
    assert_eq!(properties::parse_yaml(payload.as_bytes()).unwrap(),v);
    let mut invalid=v.clone();invalid["sheets"][0]["locked"]=json!(false);
    assert!(yaml_emit::rich_table(&invalid,"tbl-1",true,"\n").is_err());
}


#[test]
fn native_normalizer_protects_and_replays(){
    use z2o_native::normalize;
    let input=r"持续60 min，速度+81.860节，变化−0.0040%，占31.$2\%$。另有$12$分钟。";
    let report=normalize::normalize(input).unwrap();
    assert_eq!(report["text"],r"持续$60\,\mathrm{min}$，速度$+81.860\,\text{节}$，变化$−0.0040\%$，占$31.2\%$。另有$12\,\text{分钟}$。");
    let mut replay:Vec<char>=input.chars().collect();
    for change in report["changes"].as_array().unwrap(){
        let a=change["start"].as_u64().unwrap() as usize;
        let b=change["end"].as_u64().unwrap() as usize;
        assert_eq!(replay[a..b].iter().collect::<String>(),change["before"].as_str().unwrap());
        replay.splice(a..b,change["after"].as_str().unwrap().chars());
    }
    assert_eq!(replay.iter().collect::<String>(),report["text"].as_str().unwrap());
    assert_eq!(normalize::normalize(report["text"].as_str().unwrap()).unwrap()["changes"],json!([]));
    assert_eq!(report["idempotent"],true);
    let protected=["2024年；12:45 UTC；UTC 1200 h；第3节；图2秒；A3m；1,250 kg；3±2 m；5–7 km；60 km/h；1.3e-4 m。",
        "\x60a 60 min\x60 和 \x60\x60含内部 \x60 81节 — 中 A\x60\x60",
        "https://host/中—A/60min DOI 10.1022/ab—6min",
        "[id]: <目标/60 min—ab>\n  \"测试 中 A\"",
        "> 原文摘录\n其长度为 5 m。",
        "[x](<folder/part) 90 min—b> \"caption) 40 min—x\")",
        "<PrE lang='txt'>\n5 m\n</pre>",
        "---\n中 A — 30 min\n\n## 3 段\n\n尾"];
    for source in protected{
        let result=normalize::normalize(source).unwrap();
        assert_eq!(result["text"],source,"{result}");
    }
    let linked=normalize::normalize("[持续60 min](https://x/a(b)—c) [[图—1|值25%]]").unwrap();
    assert_eq!(linked["text"],r"[持续$60\,\mathrm{min}$](https://x/a(b)—c) [[图—1|值$25\%$]]");
}
#[test]
fn native_normalizer_crlf_and_exclusive_destination(){
    use z2o_native::normalize;
    let source=("# Original title\n\n中文标题\n作者：甲\n## 2 实验\n\n正文\n".to_owned()+
        "![源](path/abc123.png)\n\n**图1：** 描述\n\n^fig-1\n$$\nx=1\n$$\n\n^eq-1\n下一段").replace('\n',"\r\n");
    let expected=("# Original title\n中文标题\n\n作者：甲\n\n## 2 实验\n正文\n\n".to_owned()+
        "![源](path/abc123.png)  \n**图1：** 描述\n^fig-1\n\n$$\nx=1\n$$\n^eq-1\n\n下一段").replace('\n',"\r\n");
    assert_eq!(normalize::normalize(&source).unwrap()["text"],expected);
    let directory=tempdir().unwrap();let input=directory.path().join("原稿.md");let output=directory.path().join("规范化.md");
    fs::write(&input,&source).unwrap();
    let result=normalize::normalize_file(&input,&output).unwrap();
    assert_eq!(fs::read_to_string(&output).unwrap(),result["text"].as_str().unwrap());
    assert!(normalize::normalize_file(&input,&input).is_err());
    assert!(normalize::normalize_file(&input,&output).is_err());
    assert_eq!(fs::read_to_string(&input).unwrap(),source);
}


#[test]
fn validator_uses_actual_yaml_source_style_and_anchor_positions(){
    use z2o_native::validate;
    let invalid="---\n年份: 2024\n年份: 2025\n---\n[x](#^missing)\n";
    let findings=validate::validate_note(invalid);
    assert!(findings.iter().any(|f|f.code=="FRONTMATTER_YAML"&&f.line==3),"{findings:?}");
    assert!(findings.iter().any(|f|f.code=="MISSING_ANCHOR"&&f.line==5));
    assert!(validate::validate_note("---\ndate: 2025-02-30\n---\n").iter().any(|f|f.code=="FRONTMATTER_YAML"));
    assert!(validate::validate_note("---\nbase: &base\n  color: blue\nlocal:\n  <<: *base\n  color: red\n---\n").is_empty());
    let table="\x60\x60\x60rich-table\nversion: 2\nlocked: true\ntheme: grid\ncolumns: [a]\nrows: [b]\ntitle: '[表1](#^tbl-1)'\n\x60\x60\x60\n^tbl-1\n";
    let inspected=validate::inspect(table);
    assert_eq!(inspected.table_records.len(),1);
    assert_eq!(inspected.table_records[0].anchor.as_deref(),Some("tbl-1"));
    for code in ["TABLE_COLUMNS_FLOW_STYLE","TABLE_ROWS_FLOW_STYLE","TABLE_CAPTION_SELF_LINK"]{
        assert!(inspected.findings.iter().any(|f|f.code==code),"{:?}",inspected.findings);
    }
}
#[test]
fn validator_protects_code_comments_and_tracks_bibliography(){
    use z2o_native::validate;
    let text="[\u{7532}](#^ref-1)\n\n## 2 参考文献\n1. Author\n    journal. ^ref-1\n";
    assert!(validate::validate_note(text).is_empty());
    let text="\x60literal\n%%\nend\x60\n\n[x](#^absent)\n";
    let findings=validate::validate_note(text);
    assert_eq!(findings.len(),1,"{findings:?}");
    assert_eq!(findings[0].code,"MISSING_ANCHOR");
    let text="$$\nx=1\n$$\n\n^eq-1\n后文\n";
    let codes:Vec<_>=validate::validate_note(text).into_iter().map(|f|f.code).collect();
    for expected in ["ANCHOR_GAP","ANCHOR_SPACING","EQUATION_ANCHOR"]{assert!(codes.contains(&expected),"{codes:?}");}
}

#[test]
fn glossary_index_and_source_binding(){
    use z2o_native::glossary::{Glossary,prepare};
    let dir=tempdir().unwrap();
    let g=dir.path().join("术语表.md");
    fs::write(&g,"\u{feff}| ID | English | Chinese |\r\n| --- | --- | --- |\r\n| T1 | radial velocity | 径向速度 |\r\n| T2 | root mean square；别名/缩写：RMS | 均方根 |\r\n| T3 | tangential velocity | 切向速度 |\r\n").unwrap();
    let idx=Glossary::new(&g).unwrap();
    assert_eq!(idx.search("RMS").unwrap()[0]["id"],"T2");
    assert_eq!(idx.search("切向").unwrap()[0]["id"],"T3");
    assert_eq!(idx.search("rad").unwrap(),json!([]));
    let text="🌊径向: Radial-velocity and RMS";
    let report=idx.suggest(text,1).unwrap();
    let rows=report["matches"].as_array().unwrap();
    assert!(rows.iter().any(|x|x["entry"]["id"]=="T1"&&x["match_types"]==json!(["exact_token_phrase"])));
    for row in rows{
        let a=row["source_span"]["start"].as_u64().unwrap() as usize;
        let b=row["source_span"]["end"].as_u64().unwrap() as usize;
        assert_eq!(row["source_span"]["quote"],text.chars().skip(a).take(b-a).collect::<String>());
        assert_eq!(row["decision"],"pending");
    }
    let p=dir.path().join("source.pdf");fs::write(&p,b"synthetic PDF identity").unwrap();
    let inventory=dir.path().join("inventory.json");
    let data=json!({"sources":[{"id":"main","path":"source.pdf","sha256":files::digest(b"synthetic PDF identity"),
        "pages":[{"page":1,"blocks":[{"id":"b1","bbox":[0,0,2,3],"text":text}]}]}]});
    fs::write(&inventory,serde_json::to_vec(&data).unwrap()).unwrap();
    let output=dir.path().join("leads.json");
    assert!(prepare(&inventory,&g,&output,|_,_|{}).is_ok());
    let prior=fs::read(&output).unwrap();
    assert!(prepare(&inventory,&g,&output,|_,_|{}).is_err());
    assert_eq!(fs::read(&output).unwrap(),prior);
    let changed=dir.path().join("changed.json");
    assert!(prepare(&inventory,&g,&changed,|_,_|fs::write(&p,b"changed").unwrap()).is_err());
    assert!(!changed.exists());
    assert!(prepare(&inventory,&g,&changed,|_,_|{}).is_err());
    fs::write(&p,b"synthetic PDF identity").unwrap();
    let raced=dir.path().join("raced.json");
    assert!(prepare(&inventory,&g,&raced,|_,_|fs::write(&raced,b"other").unwrap()).is_err());
    assert_eq!(fs::read(&raced).unwrap(),b"other");
    fs::write(&g,"| T1 | a term | 术语 |\n| T1 | duplicate | 重复 |").unwrap();
    assert!(Glossary::new(&g).is_err());
}

#[test]
fn native_contract_job_and_replay_preserve_evidence(){
    use z2o_native::pipeline;
    let dir=tempdir().unwrap();let source=dir.path().join("论文.txt");fs::write(&source,b"source").unwrap();
    let plan=json!({"schema_version":1,"properties":{"type":"article"},"units":[
        {"unit_id":"p","kind":"paragraph","source":{"source_id":"main","locator":"p1"},"scope":"main"}]});
    let sources=json!({"main":source});
    let contract=pipeline::build_contract(&plan,&sources).unwrap();
    assert_eq!(contract["sources"]["main"]["sha256"],files::digest(b"source"));
    assert!(pipeline::build_contract(&plan,&json!({"different":source})).is_err());
    let out=dir.path().join("jobs");
    let receipt=pipeline::prepare(&plan,&sources,&out).unwrap();
    let job=Path::new(receipt["directory"].as_str().unwrap());let filled=job.join("content.json");
    fs::write(&filled,b"model output").unwrap();
    assert_eq!(pipeline::prepare(&plan,&sources,&out).unwrap(),receipt);
    assert_eq!(fs::read(&filled).unwrap(),b"model output");
    fs::write(&source,b"new source").unwrap();
    assert_ne!(pipeline::prepare(&plan,&sources,&out).unwrap()["job_id"],receipt["job_id"]);
    let p=dir.path().join("receipt.bin");
    std::thread::scope(|scope|for _ in 0..8{let p=&p;scope.spawn(move||pipeline::publish(p,b"complete").unwrap());});
    assert_eq!(fs::read(&p).unwrap(),b"complete");assert!(pipeline::publish(&p,b"wrong").is_err());
    let original="🌊旧文\n第二行";
    let changes=json!([{"start":1,"end":2,"before":"旧","after":"新"}]);
    assert_eq!(pipeline::replay(original,&changes).unwrap(),"🌊新文\n第二行");
    assert!(pipeline::replay(original,&json!([{"start":1,"end":2,"before":"错","after":"新"}])).is_err());
    let raw=json!({"text":original,"mapping":[{"unit_id":"p","start_offset":0,"end_offset":3,"sha256":files::digest("🌊旧文".as_bytes())}]});
    let normalized=json!({"text":"🌊新文\n第二行","changes":changes,"input_sha256":files::digest(original.as_bytes())});
    let r=pipeline::rebase(&raw,&normalized).unwrap();assert_eq!(r["mapping"][0]["sha256"],files::digest("🌊新文".as_bytes()));
    assert_eq!(r["mapping"][0]["start_line"],1);assert_eq!(r["mapping"][0]["end_line"],1);
    let broken=json!({"text":"🌊新第二行","changes":[{"start":1,"end":4,"before":"旧文\n","after":"新"}],"input_sha256":files::digest(original.as_bytes())});
    assert!(pipeline::rebase(&raw,&broken).is_err());
}

#[test]
fn presentation_visible_prose_excludes_opaque_destinations(){
    use z2o_native::presentation::lint;
    let body=r#"## 1 方法
正文 A定义，过去4-5年，30%，裸 DOI 10.1234/xyz。
[图2a](#^fig-2)、[2b](#^fig-2)
[可见标签](<hidden 60 min—9%> "hidden 2 m")
"#;
    let p=lint(body).unwrap();
    for code in ["CJK_SPACE","QUANTITY_OUTSIDE_MATH","PERCENT_OUTSIDE_MATH","NAKED_DOI","SPLIT_PANEL_CALL"]{
        assert!(p.iter().any(|x|x.code==code),"missing {code}: {p:?}");
    }
    assert!(p.iter().all(|x|!x.excerpt.contains("hidden")));
    let body="<!-- hidden 60 min -->\n".to_owned()+ "\x60\x60\x60markdown\n30%\n\x60\x60\x60\n"+r"$30\%$";
    assert!(lint(&body).unwrap().is_empty());
}
