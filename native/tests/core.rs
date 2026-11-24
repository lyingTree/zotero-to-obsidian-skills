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

#[test]
fn mechanical_acceptance_uses_actual_objects_and_source_bytes(){
    use z2o_native::acceptance::accept;
    let root=tempdir().unwrap();let candidate=root.path().join("候选.md");
    let source=root.path().join("source.pdf");fs::write(&source,b"actual source").unwrap();
    let image=root.path().join("fig one.png");fs::write(&image,b"actual image").unwrap();
    let body="---\ntype: article\n---\n\n![A](<fig%20one.png>)  \n**图A** 示例\n^fig-A\n";
    fs::write(&candidate,body).unwrap();
    let contract=json!({"schema_version":1,"sources":{"s":{"path":source,"sha256":files::digest(b"actual source")}},
        "required_anchors":["fig-A"],"counts":{"figures":1,"images":1,"tables":0,"equations":0,"bibliography":0},
        "resources":{"fig%20one.png":files::digest(b"actual image")}});
    let report=accept(&candidate,&contract,Some(root.path()),&Value::Null).unwrap();
    assert_eq!(report["mechanical_status"],"pass","{report}");
    assert_eq!(report["source_coverage"],"verified_against_contract");
    assert_eq!(report["source_fidelity"],"not_assessed");
    assert_eq!(report,accept(&candidate,&contract,Some(root.path()),&Value::Null).unwrap());
    fs::write(&source,b"changed").unwrap();fs::write(&image,b"changed").unwrap();
    let report=accept(&candidate,&contract,Some(root.path()),&Value::Null).unwrap();
    for code in ["SOURCE_HASH","RESOURCE_HASH"]{assert!(report["checks"].as_array().unwrap().iter().any(|r|r["code"]==code&&r["status"]=="fail"));}
    assert_eq!(report["mechanical_status"],"fail");assert_eq!(report["source_coverage"],"failed");
    fs::write(&candidate,b"---\ntype: article\n---\n\n![](<%2e%2e/outside.png>)").unwrap();
    let report=accept(&candidate,&Value::Null,Some(root.path()),&Value::Null).unwrap();
    assert!(report["checks"].as_array().unwrap().iter().any(|r|r["code"]=="RESOURCE_BOUNDARY"&&r["status"]=="fail"));
}
#[test]
fn grid_rejects_covered_values_overlap_and_ambiguous_dotted_ids(){
    use z2o_native::grid::check_merges;
    let sheet=json!({"columns":[{"id":"c","name":"A"},{"id":"d","name":"B"}],
        "rows":[{"id":"r","cells":{"c":"x","d":"y"}},{"id":"s","cells":{"c":"","d":null}}],
        "merges":[{"anchor":"r.c","end":"s.d"},{"anchor":"r.c","end":"r.d"}]});
    let findings=check_merges("t",None,&sheet);
    assert!(findings.iter().any(|f|f.code=="TABLE_MERGE_COVERED_VALUE"));assert!(findings.iter().any(|f|f.code=="TABLE_MERGE_OVERLAP"));
    let sheet=json!({"columns":[{"id":"b.c","name":"A"},{"id":"c","name":"B"}],
        "rows":[{"id":"a","cells":{}},{"id":"a.b","cells":{}}],
        "merges":[{"anchor":"a.b.c","end":"a.b.c"}]});
    assert!(check_merges("t",None,&sheet).iter().any(|f|f.code=="TABLE_MERGE_ENDPOINT"));
}

#[test]
fn complete_native_mechanical_pipeline_preserves_receipts_and_rejects_changed_evidence(){
    use z2o_native::pipeline::{process,build_contract};
    let root=tempdir().unwrap();let source=root.path().join("原始文献.txt");fs::write(&source,b"source bytes").unwrap();
    let original=root.path().join("候选.md");
    let bytes="---\ntype: article\n---\n\n# Example\n\n## 1 方法\n每60 min观测，增加31.$2\\%$。\n".as_bytes();
    fs::write(&original,bytes).unwrap();let output=root.path().join("结果");
    let contract=json!({"schema_version":1,"sources":{"main":{"path":source,"sha256":files::digest(b"source bytes")}},
        "counts":{"figures":0,"equations":0,"tables":0,"bibliography":0}});
    let run=||process(Some(&original),&Value::Null,&Value::Null,&output,&contract,Some(root.path()),|_,_,_|{});
    let result=run().unwrap();assert_eq!(result["status"],"mechanical_pass","{result}");
    assert_eq!(result["source_fidelity"],"not_assessed");
    let directory=Path::new(result["directory"].as_str().unwrap());
    assert_eq!(fs::read(directory.join("original.md")).unwrap(),bytes);
    assert_eq!(fs::read(&original).unwrap(),bytes);
    assert!(fs::read_to_string(directory.join("normalized.md")).unwrap().contains(r"$60\,\mathrm{min}$"));
    assert_eq!(result,run().unwrap());
    fs::remove_file(directory.join("report.md")).unwrap();fs::remove_file(directory.join("result.json")).unwrap();
    assert_eq!(result,run().unwrap());let receipt=fs::read(directory.join("result.json")).unwrap();
    fs::write(&source,b"changed source").unwrap();assert!(run().is_err());assert_eq!(fs::read(directory.join("result.json")).unwrap(),receipt);
    fs::write(&source,b"source bytes").unwrap();
    fs::write(directory.join("normalized.md"),b"manual edit").unwrap();assert!(run().is_err());
    assert_eq!(fs::read(directory.join("normalized.md")).unwrap(),b"manual edit");
    let plan=json!({"schema_version":1,"properties":{"type":"article"},"units":[
        {"unit_id":"title","kind":"header","original_title":"A source","translated":true,"authors":["Author"],"affiliations":["Institute"],"scope":"main","source":{"source_id":"main","locator":"title"}},
        {"unit_id":"p","kind":"paragraph","scope":"main","source":{"source_id":"main","locator":"p"}}]});
    let content=json!({"title":{"translated_title":"来源"},"p":{"text":"每60 min观测。"}});
    let c=build_contract(&plan,&json!({"main":source})).unwrap();
    let result=process(None,&plan,&content,&output,&c,Some(root.path()),|_,_,_|{}).unwrap();
    assert_eq!(result["status"],"mechanical_pass","{result}");
    let blocked=process(None,&plan,&json!({}),&output,&c,Some(root.path()),|_,_,_|{}).unwrap();
    assert_eq!(blocked["status"],"blocked");assert!(!Path::new(blocked["directory"].as_str().unwrap()).join("normalized.md").exists());
    let changed=root.path().join("changed-during-run");
    let attempt=process(Some(&original),&Value::Null,&Value::Null,&changed,&contract,Some(root.path()),|done,_,_|{
        if done==5{fs::write(&source,b"changed during acceptance").unwrap();}
    });
    assert!(attempt.is_err());assert!(fs::read_dir(&changed).unwrap().all(|e|!e.unwrap().path().join("result.json").exists()));
}

#[test]
fn native_docx_inventory_preserves_xml_order_embedded_bytes_and_pending_objects(){
    use z2o_native::docx;
    use std::io::Write;
    use zip::{ZipWriter,write::SimpleFileOptions};
    let dir=tempdir().unwrap();let source=dir.path().join("补充材料.docx");
    let xml=r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><w:body><w:p><w:r><w:t>中文</w:t><w:noBreakHyphen/><w:t>🌊</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><a:blip r:embed="rImage"/></w:r></w:p></w:tc></w:tr></w:tbl><w:p><m:oMath><m:r><m:t>x</m:t></m:r></m:oMath></w:p><w:sectPr/></w:body></w:document>"#;
    let rels=r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rImage" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image.png"/></Relationships>"#;
    let types=r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="png" ContentType="image/png"/></Types>"#;
    let mut z=ZipWriter::new(fs::File::create(&source).unwrap());
    for (name,data) in [("word/document.xml",xml.as_bytes()),("[Content_Types].xml",types.as_bytes()),("word/_rels/document.xml.rels",rels.as_bytes()),("word/media/image.png",b"original media".as_slice())]{
        z.start_file(name,SimpleFileOptions::default()).unwrap();z.write_all(data).unwrap();
    }z.finish().unwrap();
    let evidence=dir.path().join("实际证据");
    let r=docx::prepare_docx_source(&source,"si",&evidence,|_,_,_|{}).unwrap();
    assert_eq!(r["native_blocks"][0]["text"],"中文‑🌊");assert_eq!(r["native_blocks"][1]["rows"][0]["cells"][0]["grid_span"],"2");
    assert_eq!(r["native_blocks"].as_array().unwrap().len(),3);assert_eq!(r["native_issues"][0]["kind"],"math");
    assert!(r["page_count"].is_null());assert_eq!(r["pages"],json!([]));
    assert_eq!(r["media_calls"][0]["status"],"resolved");assert!(r["media_calls"][0]["figure_number"].is_null());
    assert_eq!(fs::read(evidence.join("document.xml")).unwrap(),xml.as_bytes());assert_eq!(fs::read(evidence.join("media-0001.png")).unwrap(),b"original media");
    let block=&r["native_blocks"][0];let loc=json!({"basis":"native","native_locator":block["locator"],"blocks":[block["id"]],"quote":"中文"});
    assert!(docx::evidence_errors(&r,&loc).is_empty());assert_eq!(docx::source_text(&r,&loc).unwrap(),"中文‑🌊");
    let doc=docx::NativeDocument::new(&source).unwrap();assert_eq!(doc.resolve(&block["locator"]).unwrap(),"中文‑🌊");
    let call=&r["media_calls"][0];let location=json!({"basis":"native","native_locator":call["locator"],"blocks":[call["block_id"]],"media_call":call["id"]});
    assert_eq!(docx::figure_location(&r,&location).unwrap()["source_media"]["sha256"],files::digest(b"original media"));
    assert!(docx::prepare_docx_source(&source,"si",&evidence,|_,_,_|{}).is_err());
    let mut invalid=loc;invalid["page"]=json!(1);assert!(!docx::evidence_errors(&r,&invalid).is_empty());
}


#[test]
fn translation_pending_scope_and_independent_file_bindings(){
    use z2o_native::translation_prepare as tr;
    let root=tempdir().unwrap();
    let source=root.path().join("source.pdf");
    // The scaffold binds actual bytes and inventory locations; it does not parse
    // PDF display evidence or assert the quoted source is faithful.
    fs::write(&source,b"synthetic source bytes").unwrap();
    let candidate=root.path().join("candidate.md");fs::write(&candidate,"译文。\n").unwrap();
    let binding=tr::bind_file(&source).unwrap();
    let inv=json!({"schema_version":1,"sources":[{"id":"a","path":source.file_name().unwrap().to_str().unwrap(),"sha256":binding["sha256"],
        "pages":[{"page":1,"blocks":[{"id":"b1","bbox":[1,2,5,6],"text":"Source A"},{"id":"b2","bbox":[0,8,8,10],"text":"Source B"}]}]}],
        "units":[{"id":"p1","source":"a","kind":"paragraph","evidence":[{"page":1,"basis":"text_blocks","blocks":["b2","b1"]}]},
        {"id":"p2","source":"a","kind":"paragraph","evidence":[{"page":1,"quote":"Source B"}]}]});
    let input=root.path().join("inventory.json");fs::write(&input,serde_json::to_vec(&inv).unwrap()).unwrap();
    let output=root.path().join("review.json");
    let path=tr::prepare(&input,&candidate,&output,&json!(["p1"]),None,&json!({}),|_,_,_|{}).unwrap();
    let review=z2o_native::pipeline::read_json(&path).unwrap();
    assert_eq!(review["review_status"],"pending");
    assert_eq!(review["scope"]["unselected_inventory_units"],json!(["p2"]));
    let loc=&review["units"]["p1"]["source_locations"][0];
    assert_eq!(loc["quote"],"Source B\nSource A");
    assert_eq!(loc["bbox"],json!([0.0,2.0,8.0,10.0]));
    assert_eq!(review["units"]["p1"]["checks"]["completeness"]["status"],"pending");
    assert_eq!(review["units"]["p1"]["occurrences"]["claims"],json!([]));
    assert!(tr::prepare(&input,&candidate,&output,&json!(["p1"]),None,&Value::Null,|_,_,_|{}).is_err());
    let next=root.path().join("next.json");fs::write(&source,b"changed").unwrap();
    assert!(tr::prepare(&input,&candidate,&next,&json!(["p1"]),None,&Value::Null,|_,_,_|{}).is_err());
    assert!(!next.exists());
    assert_eq!(tr::occurrence("num1","numbers",&json!({"page":1,"quote":"12"})).unwrap()["difference_type"],Value::Null);
    assert_eq!(tr::term("t","ocean","concept","ocean", &json!({"page":1})).unwrap()["decision"],"pending");
}

#[test]
fn docx_source_orchestrator_blocks_pdf_before_writing(){
    let root=tempdir().unwrap();
    let pdf=root.path().join("source.pdf");fs::write(&pdf,b"%PDF-1.7").unwrap();
    let out=root.path().join("work");
    let result=z2o_native::source_prepare::prepare_docx_sources(&json!({"main":pdf.to_str().unwrap()}),&out,&Value::Null,None,144,|_,_,_|{});
    assert!(result.unwrap_err().to_string().contains("UNMIGRATED_PDF_INVENTORY"));
    assert!(!out.exists());
}


#[test]
fn atomic_rule_catalog_ignores_comments_and_fences_and_binds_exact_bytes(){
    use z2o_native::check_ledger as checks;
    let root=tempdir().unwrap();
    let skills=root.path().join("skills");
    let skill=skills.join("example");
    fs::create_dir_all(skill.join("references")).unwrap();
    fs::write(skill.join("SKILL.md"),"---\nname: example\n---\n# Example\n- [X-C01] Constraint.\n- [ ] [X-A01] Outcome. <!-- check-type: offline_outcome -->\n- [ ] [X-A02] Observe. <!-- check-type: execution_process -->\n<!--\n- [FAKE-A01] Hidden.\n-->\n\x60\x60\x60markdown\n- [FAKE-A02] Code.\n\x60\x60\x60\n").unwrap();
    fs::write(skill.join("references").join("rules.md"),"- [PROP-A01] Actual YAML. <!-- check-type: offline_outcome -->\n").unwrap();
    let cat=checks::coverage(&skills,|_,_,_|{}).unwrap();
    let rows=cat["rules"].as_array().unwrap();
    assert_eq!(rows.len(),4);
    assert_eq!(rows[1]["id"],"X-A01");
    assert_eq!(rows[1]["rule"]["line"],6);
    assert_eq!(rows[1]["declared_type"],"offline_outcome");
    assert_eq!(rows[3]["new_stage_helper"],"audit_properties");
    let source=root.path().join("source.pdf");let candidate=root.path().join("candidate.md");
    fs::write(&source,b"fixture").unwrap();fs::write(&candidate,b"candidate\n").unwrap();
    let options=||checks::PrepareOptions{
        sources:json!({"source":source.to_str().unwrap()}),candidates:Value::Null,candidate_path:Some(candidate.clone()),
        scopes:json!({"X-A01":[{"object_id":"p1","locator":{"kind":"whole_file"}}]}),types:Value::Null,environments:Value::Null,
        mode:"acceptance".into(),additional:json!([]),
    };
    let output=root.path().join("checks.json");
    checks::prepare(&skills,&output,options(),|_,_,_|{}).unwrap();
    let payload=z2o_native::pipeline::read_json(&output).unwrap();
    assert_eq!(payload["catalog"].as_array().unwrap().len(),4);
    assert_eq!(payload["checks"].as_array().unwrap().len(),3);
    assert!(payload["checks"].as_array().unwrap().iter().all(|r|r["status"]=="not_verified"&&r["evidence"]==json!([])));
    assert_eq!(checks::pending_rows(&output,40,Some("X")).unwrap(),json!([["X-A01","Outcome."]]));
    assert!(checks::prepare(&skills,&output,options(),|_,_,_|{}).is_err());
    fs::write(skill.join("references").join("duplicate.md"),"- [X-A01] Duplicate.\n").unwrap();
    assert!(checks::coverage(&skills,|_,_,_|{}).is_err());
}


#[test]
fn native_atomic_ledger_validates_evidence_and_records_without_rebinding(){
    use z2o_native::{check_ledger,check_validate,check_write};
    let root=tempdir().unwrap();
    let skill=root.path().join("skills").join("example");fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"),"- [ ] [EX-A01] Actual file. <!-- check-type: offline_outcome -->\n- [ ] [EX-A02] Actual event. <!-- check-type: execution_process -->\n").unwrap();
    let source=root.path().join("source.md");let candidate=root.path().join("candidate.md");
    fs::write(&source,"source\n").unwrap();fs::write(&candidate,"候选\n").unwrap();
    let scope=json!([{"object_id":"p1","locator":{"kind":"whole_file"}}]);
    let p=root.path().join("checks.json");
    check_ledger::prepare(&skill,&p,check_ledger::PrepareOptions{
        sources:json!({"main":source.to_str().unwrap()}),candidates:Value::Null,candidate_path:Some(candidate.clone()),
        scopes:json!({"EX-A01":scope,"EX-A02":scope}),types:Value::Null,environments:Value::Null,
        mode:"acceptance".into(),additional:json!([])
    },|_,_,_|{}).unwrap();
    let validate=||check_validate::validate(&p,|_,_,_|{}).unwrap();
    let start=validate();assert_eq!(start["structurally_consistent"],true);assert_eq!(start["offline_records_ready"],false);
    let binding=check_ledger::bind_evidence(&candidate,&json!({"kind":"lines","start":1,"end":1}),"offline_result",&Value::Null).unwrap();
    let evidence=json!({"file":binding});
    let row=json!({"id":"EX-A01","status":"passed","reason":"Actual file reviewed.","scope":scope,"evidence":["file"]});
    check_write::record_batch(&p,&json!([row]),&evidence).unwrap();
    let after=validate();assert_eq!(after["offline_records_ready"],true);assert_eq!(after["recorded_checks_complete"],false);
    assert_eq!(after["source_fidelity"],"not_assessed");
    let saved=fs::read(&p).unwrap();
    let mut bad=row.clone();bad["requirement_type"]=json!("unclassified");
    assert!(check_write::record_batch(&p,&json!([bad]),&json!({})).is_err());assert_eq!(fs::read(&p).unwrap(),saved);
    let trace=json!({"id":"EX-A02","status":"passed","reason":"Unobserved.","scope":scope,"evidence":["file"]});
    check_write::record_batch(&p,&json!([trace]),&json!({})).unwrap();
    assert!(validate()["findings"].as_array().unwrap().iter().any(|f|f["code"]=="CHECK_PROCESS_NOT_OBSERVED"));
    let records=z2o_native::pipeline::read_json(&p).unwrap();assert_eq!(records["updates"].as_array().unwrap().len(),2);
    fs::write(&candidate,"changed\n").unwrap();
    assert!(validate()["findings"].as_array().unwrap().iter().any(|f|f["code"]=="CHECK_HASH_CHANGED"));
}
#[test]
fn native_check_writer_rejects_competing_writer_and_preserves_record(){
    use fs2::FileExt;
    let root=tempdir().unwrap();
    let p=root.path().join("checks.json");fs::write(&p,b"{}\n").unwrap();
    let f=std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(root.path().join(".checks.json.z2o-lock")).unwrap();
    FileExt::try_lock_exclusive(&f).unwrap();
    let err=z2o_native::check_write::record_batch(&p,&json!([{}]),&json!({})).unwrap_err();
    assert!(format!("{err:#}").contains("Another native process"));
    assert_eq!(fs::read(&p).unwrap(),b"{}\n");
    FileExt::unlock(&f).unwrap();
}
#[test]
fn native_machine_audit_binds_real_files_and_never_accepts_caller_report(){
    use z2o_native::{check_ledger,check_write,check_validate,properties};
    let root=tempdir().unwrap();
    let skill=root.path().join("skills").join("example");fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"),"- [ ] [WRITE-A04] Bytes match. <!-- check-type: offline_outcome -->\n- [ ] [WRITE-A05] No obsolete key. <!-- check-type: offline_outcome -->\n").unwrap();
    let a=root.path().join("candidate.md");let b=root.path().join("written.md");
    let bytes=properties::build(b"# Title\n","thesis",&json!({"类型":"学位论文","年份":2024,"关键词":[]})).unwrap();
    fs::write(&a,&bytes).unwrap();fs::write(&b,&bytes).unwrap();
    let p=root.path().join("checks.json");let report=root.path().join("report.json");
    check_ledger::prepare(&skill,&p,check_ledger::PrepareOptions{
        sources:json!({"source":a.to_str().unwrap()}),candidates:Value::Null,candidate_path:Some(b.clone()),
        scopes:Value::Null,types:Value::Null,environments:Value::Null,mode:"acceptance".into(),additional:json!([])
    },|_,_,_|{}).unwrap();
    let opt=||check_write::AuditOptions{stage:"write",after:&b,before:Some(&a),mode:None,item_type:None,kind:None};
    let ids=check_write::record_file_checks(&p,&report,opt()).unwrap();
    assert_eq!(ids,json!(["WRITE-A04","WRITE-A05"]));
    assert_eq!(check_validate::validate(&p,|_,_,_|{}).unwrap()["offline_records_ready"],true);
    assert_eq!(check_write::record_file_checks(&p,&report,opt()).unwrap(),json!([]));
    assert!(check_write::record_file_checks(&p,&a,opt()).is_err());
    assert_eq!(fs::read(&a).unwrap(),bytes);
}


#[test]
fn scope_derivation_checks_candidate_visible_anchor_and_exact_line_bytes(){
    use z2o_native::{check_scope,files};
    let tmp=tempdir().unwrap();
    let inventory=tmp.path().join("inventory.json");let candidate=tmp.path().join("candidate.md");
    let input=json!({"sources":[{"id":"a","page_count":1}],"units":[{"id":"p1","source":"a","evidence":[{"page":1,"quote":"source text"}]}]});
    fs::write(&inventory,serde_json::to_vec(&input).unwrap()).unwrap();
    let deliveries=json!([{"unit":"p1","target":{"anchor":"present"}}]);
    for text in ["# A\n\nMissing.\n","<!--\nHidden ^present\n-->\n","    Code ^present\n","First ^present\n\nSecond ^present\n",
        "\x60\x60\x60rich-table\ntext: ^present\n\x60\x60\x60\n","$$\nx ^present\n$$\n","\x60Multi\nLine ^present\n\x60\n"]{
        fs::write(&candidate,text).unwrap();
        assert!(check_scope::from_deliveries(&inventory,&candidate,&deliveries,&json!(["p1"])).is_err(),"{text}");
    }
    let text="# A\n\n候选。 ^present\n";
    fs::write(&candidate,format!("\u{feff}{}",text.replace('\n',"\r\n"))).unwrap();
    let r=check_scope::from_deliveries(&inventory,&candidate,&deliveries,&json!(["p1"])).unwrap();
    assert_eq!(r[0]["object_id"],"p1");
    let target=json!([{"unit":"p1","target":{"lines":[1,3],"sha256":files::digest(text.as_bytes())}}]);
    assert!(check_scope::from_deliveries(&inventory,&candidate,&target,&json!(["p1"])).is_ok());
    let target=json!([{"unit":"p1","target":{"lines":[1,3],"sha256":"0"}}]);
    assert!(check_scope::from_deliveries(&inventory,&candidate,&target,&json!(["p1"])).is_err());
}


#[test]
fn ledger_preparation_inventory_writeback_and_recovery_preimages(){
    use z2o_native::{check_ledger,check_validate,pipeline};
    let root=tempdir().unwrap();let skill=root.path().join("example");fs::create_dir(&skill).unwrap();
    fs::write(skill.join("SKILL.md"),"- [ ] [X-A01] Outcome. <!-- check-type: offline_outcome -->\n").unwrap();
    let source=root.path().join("source.txt");let candidate=root.path().join("candidate.md");
    fs::write(&source,"source\n").unwrap();fs::write(&candidate,"candidate\n").unwrap();
    let inventory=root.path().join("inventory.json");let original_inventory=b"{\"sources\":[]}\n";
    fs::write(&inventory,original_inventory).unwrap();
    let p=root.path().join("checks.json");
    let options=||check_ledger::PrepareOptions{
        sources:json!({"main":source.to_str().unwrap()}),candidates:Value::Null,candidate_path:Some(candidate.clone()),
        scopes:json!({"X-A01":[{"object_id":"p1","locator":{"kind":"whole_file"}}]}),types:Value::Null,environments:Value::Null,
        mode:"acceptance".into(),additional:json!([])
    };
    check_ledger::prepare_with_inventory(&skill,&p,options(),Some(&inventory),false,|_,_,_|{}).unwrap();
    let ledger=pipeline::read_json(&p).unwrap();
    let inv=pipeline::read_json(&inventory).unwrap();
    assert_eq!(inv["checks_path"],z2o_native::path_text(&z2o_native::files::canonical(&p).unwrap()).unwrap());
    assert!(!ledger.to_string().contains("inventory.json"));
    assert_eq!(check_validate::validate(&p,|_,_,_|{}).unwrap()["structurally_consistent"],true);
    let before=fs::read(&p).unwrap();
    let mut changed=options();changed.scopes=json!({"X-A01":[{"object_id":"p2","locator":"new review"}]});
    check_ledger::prepare_with_inventory(&skill,&p,changed,Some(&inventory),true,|_,_,_|{}).unwrap();
    let recovery:Vec<_>=fs::read_dir(root.path()).unwrap().map(|r|r.unwrap().path())
        .filter(|p|p.file_name().unwrap().to_string_lossy().starts_with(".z2o-check-transaction-")).collect();
    assert_eq!(recovery.len(),2);
    assert!(recovery.iter().any(|p|fs::read(p.join("ledger.before")).ok().as_ref()==Some(&before)));
    assert!(recovery.iter().any(|p|fs::read(p.join("inventory.before")).ok().as_deref()==Some(original_inventory.as_slice())));
    assert!(recovery.iter().all(|p|p.join("completed.json").is_file()));
    let mut forbidden=options();forbidden.sources=json!({"mutable":inventory.to_str().unwrap()});
    let rejected=root.path().join("rejected.json");
    assert!(check_ledger::prepare_with_inventory(&skill,&rejected,forbidden,Some(&inventory),false,|_,_,_|{}).is_err());
    assert!(!rejected.exists());
    assert!(check_ledger::prepare_with_inventory(&skill,&candidate,options(),None,true,|_,_,_|{}).is_err());
    assert_eq!(fs::read(&candidate).unwrap(),b"candidate\n");
}


#[test]
fn visible_text_edits_preserve_original_unicode_offsets_and_declarations(){
    use z2o_native::{visible_edits,check_ledger};
    let root=tempdir().unwrap();let text=root.path().join("原文.txt");let page=root.path().join("page.svg");
    let original="中文 🌊 micro-\u{ad}\u{2010}mesh; micro-\u{ad}\u{2010}mesh; alpha--beta; 6−2\r\n";
    fs::write(&text,original).unwrap();fs::write(&page,"<svg>micro-mesh</svg>").unwrap();
    let binding=check_ledger::bind_file(&text).unwrap();
    let page_file=check_ledger::bind_file(&page).unwrap();
    let expected="-\u{ad}\u{2010}";
    let occurrences:Vec<_>=original.match_indices(expected).map(|(i,_)|original[..i].chars().count()).collect();
    let make=|id:&str,start:usize|json!({"id":id,"start":start,"end":start+3,"expected_text":expected,
        "visible_page":{"file":page_file,"page":1,"bbox":[0,0,10,10],"coordinate_system":"rendered_image_pixels","text":"-"},
        "decision":{"status":"resolved","selected_text":"-","reason":"Explicit synthetic reading","reviewer":"fixture","observed":true}});
    let a=make("a",occurrences[0]);let b=make("b",occurrences[1]);
    let result=visible_edits::apply(&binding,&json!([b,a])).unwrap();
    assert_eq!(result["text"],original.replace(expected,"-"));
    assert_eq!(result["text_visual_conflicts"][0]["id"],"a");
    assert_eq!(result["observation_certified"],false);
    assert_eq!(result["semantic_review"]["status"],"not_assessed");
    assert_eq!(fs::read(&text).unwrap(),original.as_bytes());
    let mut invalid=a.clone();invalid["decision"]["observed"]=json!(false);
    assert!(visible_edits::apply(&binding,&json!([invalid])).is_err());
    fs::write(&page,"changed").unwrap();
    assert!(visible_edits::apply(&binding,&json!([a])).is_err());
}


#[test]
fn draft_queue_preserves_frozen_objects_and_cross_page_fragments(){
    use z2o_native::{draft_queue,check_ledger,files};
    let root=tempdir().unwrap();let source=root.path().join("source.pdf");let si=root.path().join("supplement.docx");
    fs::write(&source,"source bytes").unwrap();fs::write(&si,"supplement bytes").unwrap();
    let s=check_ledger::bind_file(&source).unwrap();let d=check_ledger::bind_file(&si).unwrap();
    let inv=root.path().join("inventory.json");let q=root.path().join("draft.json");
    let payload=json!({"sources":[
        {"id":"main","path":s["path"],"sha256":s["sha256"],"role":"main","page_count":2,"pages":[
            {"page":1,"blocks":[{"id":"a","text":"Start","bbox":[0,0,1,1]}],"image_index":{"page_bounds":[0,0,100,100],"occurrences":[]}},
            {"page":2,"blocks":[{"id":"b","text":"End","bbox":[0,0,1,1]}],"image_index":{"page_bounds":[0,0,100,100],"occurrences":[]}}]},
        {"id":"si","path":d["path"],"sha256":d["sha256"],"format":"docx","native_blocks":[{"id":"s1","kind":"paragraph","text":"SI","locator":{"part":"word/document.xml","element_path":[0]}}],
            "media_calls":[],"native_issues":[]}],"units":[]});
    fs::write(&inv,serde_json::to_vec(&payload).unwrap()).unwrap();
    draft_queue::prepare(&inv,&q,|_,_,_|{}).unwrap();
    let first=draft_queue::open(&q).unwrap();
    let run=|action:&str,params:Value,expected:Value|draft_queue::operate(&q,action,&params,&expected,|_,_,_|{});
    assert_eq!(run("next_task",json!({}),Value::Null).unwrap()["stage"],"inventory_page");
    assert!(run("assemble",json!({"output_path":root.path().join("bad.md").to_str().unwrap()}),Value::Null).is_err());
    for (page,id) in [(1,"a"),(2,"b")]{
        run("review_pdf_page_order",json!({"source_id":"main","page":page,"ordered_block_ids":[id],"reason":"Explicit order"}),Value::Null).unwrap();
        run("review_additional_objects",json!({"source_id":"main","page":page,"reason":"Explicit review"}),Value::Null).unwrap();
    }
    assert!(run("group",json!({"unit_id":"bad","block_ids":["a","s1"],"kind":"paragraph","reason":"invalid grouping"}),Value::Null).is_err());
    assert!(run("exclude",json!({"block_id":"a","reason":"stale writer"}),first["sha256"].clone()).is_err());
    run("group",json!({"unit_id":"p","block_ids":["a","b"],"kind":"paragraph","reason":"Cross-page paragraph"}),Value::Null).unwrap();
    run("write_unit",json!({"unit_id":"p","markdown":"正文\r\n跨页。","assets":[]}),Value::Null).unwrap();
    run("group",json!({"unit_id":"si-p","block_ids":["s1"],"kind":"paragraph","reason":"Native order"}),Value::Null).unwrap();
    run("write_unit",json!({"unit_id":"si-p","markdown":"补充。","assets":[]}),Value::Null).unwrap();
    assert_eq!(run("status",json!({}),Value::Null).unwrap()["draft_ready"],true);
    fs::write(&inv,b"changed working inventory").unwrap();
    let out=root.path().join("candidate.md");
    let report=run("assemble",json!({"output_path":out.to_str().unwrap()}),Value::Null).unwrap();
    let bytes=fs::read(&out).unwrap();assert_eq!(bytes,"正文\r\n跨页。\n\n补充。\n".as_bytes());
    assert_eq!(report["candidate"]["sha256"],files::digest(&bytes));
    assert_eq!(report["units"][0]["block_ids"],json!(["a","b"]));
    assert_eq!(report["source_fidelity"],"not_assessed");
    assert!(run("assemble",json!({"output_path":out.to_str().unwrap()}),Value::Null).is_err());
}


#[test]
fn native_delivery_manifest_and_stable_projection_preserve_actual_bindings(){
    use z2o_native::{delivery_prepare,mapping_compare,check_ledger,pipeline};
    let tmp=tempdir().unwrap();let root=tmp.path();
    let inv=root.join("inventory.json");let note=root.join("candidate.md");let asset=root.join("figure.png");let dest=root.join("delivery.json");
    fs::write(&inv,b"{\"schema_version\":1,\"sources\":[],\"units\":[]}\n").unwrap();
    fs::write(&note,"文献 [标签](<a b.md>)\n").unwrap();fs::write(&asset,b"fixture asset").unwrap();
    delivery_prepare::manifest(&inv,&note,&json!([{"unit":"f","target":{"anchor":"fig-1"},"assets":[{"path":"figure.png"}]}]),&dest,Some(root),None,true).unwrap();
    let m=pipeline::read_json(&dest).unwrap();
    assert_eq!(m["inventory"],check_ledger::bind_file(&inv).unwrap());
    assert_eq!(m["deliveries"][0]["assets"][0],check_ledger::bind_file(&asset).unwrap());
    assert!(delivery_prepare::manifest(&inv,&note,&json!([]),&inv,Some(root),None,false).is_err());
    let links=delivery_prepare::links("中文 [标签](<a b.md>)");
    assert_eq!(links[0]["start"],3);
    assert_eq!(links[0]["href"],"a b.md");
    let a=json!({"source_map":{"units":[{"unit":"p","source_id":"main","review_status":"pending"}]},"link_map":{}});
    let mut b=a.clone();b["source_map"]["units"][0]["review_status"]=json!("reviewed");
    assert_eq!(mapping_compare::compare(&a,&b).unwrap()["mapping_equal"],true);
    b["source_map"]["units"][0]["source_id"]=json!("si");
    let compared=mapping_compare::compare(&a,&b).unwrap();
    assert_eq!(compared["mapping_equal"],false);
    assert_eq!(compared["actual_rerun_performed"],false);
}
#[test]
fn structural_pdf_index_tracks_two_calls_and_actual_encoded_stream(){
    use lopdf::{Document,Object,Stream,dictionary};
    use z2o_native::pdf_structure;
    let tmp=tempdir().unwrap();let path=tmp.path().join("fixture.pdf");
    let mut doc=Document::with_version("1.5");
    let pages=doc.new_object_id();
    let image=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>1,"Height"=>1,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8},vec![1,2,3]));
    let contents=doc.add_object(Stream::new(dictionary!{},b"q 20 0 0 10 30 40 cm /Im Do Q q 5 0 0 7 60 10 cm /Im Do Q".to_vec()));
    let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>contents,"Resources"=>dictionary!{"XObject"=>dictionary!{"Im"=>image}}});
    doc.objects.insert(pages,Object::Dictionary(dictionary!{"Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1,"MediaBox"=>vec![0.into(),0.into(),100.into(),200.into()]}));
    let catalog=doc.add_object(dictionary!{"Type"=>"Catalog","Pages"=>pages});doc.trailer.set("Root",catalog);
    doc.save(&path).unwrap();
    let r=pdf_structure::index_page(&path,1).unwrap();
    assert_eq!(r["occurrences"].as_array().unwrap().len(),2,"{r}");
    assert_eq!(r["occurrences"][0]["image_xref"],image.0);
    assert_eq!(r["occurrences"][0]["bbox"],json!([30.0,150.0,50.0,160.0]));
    assert_eq!(r["objects"].as_array().unwrap().len(),1);
    assert_eq!(r["objects"][0]["encoded_stream_sha256"],z2o_native::files::digest(&[1,2,3]));
    assert_eq!(r["render_status"],"not_rendered");
    assert!(pdf_structure::index_page(&path,2).is_err());
}


#[test]
fn native_translation_review_replays_explicit_decisions_and_pending_gate(){
    use lopdf::{Document,Object,Stream,dictionary};
    use z2o_native::{translation_prepare as prep,translation_writer,translation_tasks,translation_validate,files};
    let tmp=tempdir().unwrap();let root=tmp.path();
    let source=root.join("source.pdf");let mut doc=Document::with_version("1.5");
    let pages=doc.new_object_id();
    let contents=doc.add_object(Stream::new(dictionary!{},vec![]));
    let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>contents});
    doc.objects.insert(pages,Object::Dictionary(dictionary!{"Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1,"MediaBox"=>vec![0.into(),0.into(),100.into(),200.into()]}));
    let catalog=doc.add_object(dictionary!{"Type"=>"Catalog","Pages"=>pages});doc.trailer.set("Root",catalog);doc.save(&source).unwrap();
    let candidate=root.join("candidate.md");fs::write(&candidate,"海洋。\n").unwrap();
    let glossary=root.join("glossary.md");fs::write(&glossary,"| OCEAN | ocean | 海洋 |\n").unwrap();
    let b=prep::bind_file(&source).unwrap();let inv=root.join("inventory.json");
    fs::write(&inv,serde_json::to_vec(&json!({"schema_version":1,"sources":[{"id":"s","path":b["path"],"sha256":b["sha256"],"page_count":1}],
        "units":[{"id":"p","source":"s","kind":"paragraph","evidence":[{"page":1,"bbox":[0,0,50,50],"quote":"ocean"}]}]})).unwrap()).unwrap();
    let prepared=root.join("prepared.json");prep::prepare(&inv,&candidate,&prepared,&json!(["p"]),Some(&glossary),&Value::Null,|_,_,_|{}).unwrap();
    let before=fs::read(&prepared).unwrap();
    let mut w=translation_writer::replay(&prepared,&json!([])).unwrap();
    assert_eq!(translation_tasks::next(&mut w,8).unwrap()["stage"],"scope");
    let commands=json!([
        {"op":"scope","status":"passed","reason":"Explicit source scope review."},
        {"op":"unit","unit_id":"p","candidate_locations":[{"lines":[1,1]}]},
        {"op":"occurrence","unit_id":"p","kind":"claims","occurrence_id":"claim","source":{"quote":"ocean"},"candidate":{"quote":"海洋"},
         "status":"passed","reason":"Explicit comparison.","comparison":{}},
        {"op":"check","unit_id":"p","name":"completeness","status":"passed","reason":"Recorded claim."},
        {"op":"term","unit_id":"p","term_id":"t","original":"ocean","category":"concept","source":{"quote":"ocean"},"aliases":[]},
        {"op":"search","unit_id":"p","term_id":"t","mode":"full_phrase","query":"ocean","interpretation":"Actual query"},
        {"op":"search","unit_id":"p","term_id":"t","mode":"family","query":"ocean","interpretation":"Actual family query"},
        {"op":"search_status","unit_id":"p","term_id":"t","mode":"aliases","status":"not_applicable","reason":"No alias in fixture"},
        {"op":"decide","unit_id":"p","term_id":"t","decision":"adopted","translation":"海洋","reason":"Explicit decision","matches":["OCEAN"]}
    ]);
    let mut w=translation_writer::replay(&prepared,&commands).unwrap();
    assert_eq!(translation_tasks::next(&mut w,8).unwrap()["stage"],"unit");
    let draft=root.join("draft.json");w.draft(&draft).unwrap();
    let mut restored=translation_writer::Writer::from_draft(&draft).unwrap();
    assert_eq!(w.data,restored.data);
    let output=root.join("review.json");let term=root.join("terms.json");
    let report=restored.save(&output,Some(&term)).unwrap();
    assert_eq!(report["records_ready"],false);
    assert_eq!(report["source_fidelity"],"not_assessed");
    assert_eq!(report,translation_validate::validate(&output,Some(&inv),Some(&candidate)).unwrap());
    assert_eq!(fs::read(&prepared).unwrap(),before);
    fs::write(&candidate,"changed").unwrap();
    assert!(translation_writer::Writer::from_draft(&draft).is_err());
}
