//! File-bound delivery manifest preparation and explicit source-unit contracts.
//! This is not the final delivery or handoff validator.
use crate::{check_ledger,files,path,path_text,parse_json,pipeline,syntax};
use anyhow::{ensure,Context,Result};
use serde_json::{json,Map,Value};
use std::{collections::{BTreeSet,HashMap},fs,path::{Path,PathBuf}};

fn text(v:&Value)->bool{v.as_str().is_some_and(|s|!s.trim().is_empty())}
fn positive(v:&Value)->bool{v.as_u64().is_some_and(|n|n>0)}
pub fn links(text:&str)->Value{
    let mut offsets=HashMap::new();
    for (index,(byte,_)) in text.char_indices().enumerate(){offsets.insert(byte,index);}
    offsets.insert(text.len(),text.chars().count());
    json!(syntax::scan_links(text).into_iter().map(|l|json!({"href":l.href,"start":offsets[&l.start],
        "end":offsets[&l.end],"image":l.image,"label":&text[l.label_start..l.label_end]})).collect::<Vec<_>>())
}
pub fn unit(id:&str,source:&str,kind:&str,evidence:&Value,fields:Map<String,Value>)->Result<Value>{
    ensure!(!id.trim().is_empty()&&!source.trim().is_empty()&&["metadata","paragraph","section","figure","table","formula","reference","citation","cross_reference"].contains(&kind),
        "Supply actual nonempty unit/source IDs and a supported kind.");
    ensure!(evidence.as_array().is_some_and(|a|!a.is_empty()&&a.iter().all(|v|v.as_object().is_some_and(|m|!m.is_empty()))),
        "A source unit needs explicit nonempty source locations.");
    ensure!(fields.keys().all(|k|!["id","source","kind","evidence"].contains(&k.as_str())),"Do not override source-unit identity fields");
    let mut u=Value::Object(fields);u["id"]=json!(id);u["source"]=json!(source);u["kind"]=json!(kind);u["evidence"]=evidence.clone();
    let required:&[&str]=match kind{
        "metadata"=>&["field"],"reference"=>&["bibliography_id","original_text","number"],
        "citation"=>&["references","display_text"],"cross_reference"=>&["target_unit","display_text"],
        "figure"|"table"|"formula"=>&["original_number"],_=>&[],
    };
    for k in required{ensure!(u.get(*k).is_some(),"{id}: record these source fields before translation: {k}");}
    if matches!(kind,"figure"|"table"|"formula"){
        ensure!(u["original_number"].is_null()||text(&u["original_number"]),"{id}: original_number must retain the source's literal string or be null.");
        if u["original_number"].is_null(){ensure!(text(&u["original_number_reason"]),"{id}: explain the observed absence of a source number.");}
    }
    if kind=="figure"{ensure!(u["contains_table"].is_boolean(),"{id}: contains_table must explicitly be true or false after source inspection.");}
    if kind=="table"{
        let shape=&u["shape"];ensure!(shape.is_object()&&positive(&shape["rows"])&&positive(&shape["columns"]),"{id}: shape needs positive integer rows and columns.");
        let merges=u["merges"].as_array().context("merges must explicitly list source merged ranges; use [] when none exist.")?;
        for merge in merges{
            let nums:Option<Vec<u64>>=["row_start","row_end","column_start","column_end"].iter().map(|k|merge[*k].as_u64()).collect();
            let a=nums.context("each merged range needs integer row and column bounds.")?;
            ensure!(a[0]<=a[1]&&a[1]<=shape["rows"].as_u64().unwrap()&&a[2]>0&&a[2]<=a[3]&&a[3]<=shape["columns"].as_u64().unwrap(),
                "{id}: a merged range falls outside the recorded source shape.");
        }
    }
    if kind=="metadata"{ensure!(u["field"].as_str().is_some_and(|s|["title","author","affiliation","publication_date","doi","email","header_note","external_url"].contains(&s)),"{id}: identify one supported metadata field.");}
    if kind=="reference"{
        ensure!(text(&u["bibliography_id"])&&text(&u["original_text"]),"{id}: bibliography_id and original_text must be nonempty source strings.");
        ensure!(text(&u["number"])||u["number"].as_i64().is_some()||u["number"].as_u64().is_some(),"{id}: number must retain a source reference number or label.");
    }
    if matches!(kind,"citation"|"cross_reference"){ensure!(text(&u["display_text"]),"{id}: record the actual source call's display text.");}
    if kind=="citation"{ensure!(u["references"].as_array().is_some_and(|a|!a.is_empty()&&a.iter().all(text)),"{id}: list the actual source reference-unit IDs.");}
    if kind=="cross_reference"{ensure!(text(&u["target_unit"]),"{id}: identify the actual called source object.");}
    Ok(u)
}
fn absolute(p:&Path,base:&Path)->PathBuf{if p.is_absolute(){p.to_owned()}else{base.join(p)}}
fn output(p:&Path)->Result<PathBuf>{
    let parent=p.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or_else(||Path::new("."));
    fs::create_dir_all(parent)?;
    Ok(files::canonical(parent)?.join(p.file_name().context("output filename")?))
}
pub fn manifest(inventory:&Path,candidate:&Path,deliveries:&Value,dest:&Path,root:Option<&Path>,
    translation:Option<&Path>,edges:bool)->Result<PathBuf>{
    let (raw,inv_binding)=files::read_bound(inventory)?;
    let inventory_path=path(&inv_binding["path"])?;
    let inv=parse_json(std::str::from_utf8(&raw)?)?;
    ensure!(inv.is_object(),"The source inventory must be an object");
    let candidate=check_ledger::bind_file(candidate)?;
    let cpath=path(&candidate["path"])?;
    let root=files::canonical(root.unwrap_or(cpath.parent().context("candidate parent")?))?;
    ensure!(root.is_dir(),"resource_root is not a directory");
    let mut data=deliveries.as_array().filter(|a|a.iter().all(Value::is_object)).context("deliveries must be an array of objects")?.clone();
    let mut bindings=vec![inv_binding.clone(),candidate.clone()];
    for row in &mut data{
        if let Some(assets)=row.get_mut("assets"){
            for a in assets.as_array_mut().context("assets must be an array")?{
                let p=absolute(&path(&a["path"])?,&root);
                let b=check_ledger::bind_file(&p)?;
                ensure!(path(&b["path"])?.starts_with(&root),"Delivery asset escapes resource root");
                if let Some(expected)=a.get("sha256"){ensure!(expected==&b["sha256"],"Delivery asset changed after binding");}
                a.as_object_mut().context("asset must be an object")?.extend(b.as_object().unwrap().clone());
                bindings.push(b);
            }
        }
    }
    let mut result=json!({"schema_version":1,"inventory":inv_binding,"candidate":candidate,
        "resource_root":path_text(&root)?,"deliveries":data});
    if edges{result["figure_edge_reviews_required"]=json!(true);}
    for (key,target) in [("terminology_path","terminology"),("figure_reviews_path","figure_reviews"),("checks_path","checks")]{
        if let Some(v)=inv.get(key){
            if key=="terminology_path"&&(v.is_null()||v==""){continue;}
            ensure!(text(v),"{key} must name the actual ledger");
            let p=absolute(&path(v)?,inventory_path.parent().unwrap());
            let b=check_ledger::bind_file(&p)?;
            bindings.push(b.clone());result[target]=b;
        }
    }
    if let Some(p)=translation{
        let b=check_ledger::bind_file(p)?;bindings.push(b.clone());result["translation_review"]=b;
    }
    let dest=output(dest)?;
    let protected:BTreeSet<_>=bindings.iter().filter_map(|v|v["path"].as_str()).collect();
    ensure!(!protected.contains(path_text(&dest)?.as_str()),"Manifest must not replace a bound input");
    for b in &bindings{ensure!(check_ledger::bind_file(Path::new(b["path"].as_str().unwrap()))?==*b,"Input changed before manifest publication");}
    let mut bytes=serde_json::to_vec_pretty(&result)?;bytes.push(b'\n');
    // Existing identical manifests can be reused; different bytes require a
    // distinct version, avoiding silent overwrite of prior review evidence.
    pipeline::publish(&dest,&bytes)?;
    ensure!(pipeline::read_json(&dest)?==result,"Manifest readback differs");
    Ok(dest)
}
pub fn write_workflow_manifest(run:&Path,deliveries:&Value)->Result<PathBuf>{
    let (raw,record)=files::read_bound(run)?;
    let data=parse_json(std::str::from_utf8(&raw)?)?;
    let records=&data["records"];let config=&data["config"];
    let run_path=path(&record["path"])?;let base=run_path.parent().context("Workflow directory")?;
    let inv=absolute(&path(&records["source_inventory"])?,base);let candidate=absolute(&path(&config["candidate_path"])?,base);
    let dest=absolute(&path(&records["delivery_manifest"])?,base);let root=absolute(&path(&config["resource_root"])?,base);
    let tr=if records["translation_review"].is_null(){None}else{Some(absolute(&path(&records["translation_review"])?,base))};
    if let Some(p)=&tr{ensure!(p.is_file(),"Prepare and complete translation-review.json before the final delivery index.");}
    // Bind the final file through the existing locked native writer.
    crate::check_write::bind_candidate(run)?;
    ensure!(check_ledger::bind_file(run)?==record,"Workflow record changed during manifest preparation");
    let out=manifest(&inv,&candidate,deliveries,&dest,Some(&root),tr.as_deref(),true)?;
    ensure!(check_ledger::bind_file(run)?==record,"Workflow changed during manifest publication; preserve the evidence for inspection");
    Ok(out)
}
