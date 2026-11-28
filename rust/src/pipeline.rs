//! Freeze source-derived contracts and verified sequential edits. These
//! primitives do not assert that a source plan is exhaustive or semantic.
use crate::{acceptance,files,normalize,parse_json,path,path_text,render,yaml_emit,ENGINE_VERSION,REFERENCE_VERSION};
use anyhow::{anyhow,ensure,Context,Result};
use serde_json::{json,Map,Value};
use std::{collections::{BTreeSet,BTreeMap},fs,io::Write,path::Path};

pub fn read_json(p:&Path)->Result<Value>{
    let bytes=fs::read(p)?;
    let text=std::str::from_utf8(&bytes)?;
    ensure!(!text.starts_with('\u{feff}'),"JSON configuration must not start with a BOM");
    let v=parse_json(text)?;
    ensure!(v.is_object(),"Expected a JSON object: {}",p.display());
    Ok(v)
}
fn pretty(v:&Value)->Result<Vec<u8>>{let mut b=serde_json::to_vec_pretty(v)?;b.push(b'\n');Ok(b)}
fn identical(p:&Path,data:&[u8])->Result<()>{
    let m=fs::symlink_metadata(p)?;
    ensure!(!m.file_type().is_symlink()&&m.is_file(),"Refusing a symlink or non-file artifact: {}",p.display());
    ensure!(fs::read(p)?==data,"Existing artifact differs; preserve it and use a new output root: {}",p.display());
    Ok(())
}
pub fn publish(p:&Path,data:&[u8])->Result<()>{
    match fs::symlink_metadata(p){
        Ok(_)=>return identical(p,data),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},
        Err(e)=>return Err(e.into())
    }
    let mut tmp=tempfile::Builder::new().prefix(".pending-").tempfile_in(p.parent().context("artifact parent")?)?;
    tmp.write_all(data)?;tmp.as_file().sync_all()?;
    match tmp.persist_noclobber(p){
        Ok(_)=>identical(p,data),
        Err(e) if e.error.kind()==std::io::ErrorKind::AlreadyExists=>identical(p,data),
        Err(e)=>Err(e.error.into()),
    }
}
pub fn publish_replace(p:&Path,data:&[u8])->Result<()>{
    match fs::symlink_metadata(p){
        Ok(m)=>ensure!(!files::is_linked(p,&m)&&m.is_file(),"Refusing a symlink or non-file artifact: {}",p.display()),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},
        Err(e)=>return Err(e.into()),
    }
    let mut tmp=tempfile::Builder::new().prefix(".pending-").tempfile_in(p.parent().context("artifact parent")?)?;
    tmp.write_all(data)?;tmp.as_file().sync_all()?;
    tmp.persist(p).map_err(|e|anyhow!("Cannot replace {}: {}",p.display(),e.error))?;
    identical(p,data)
}
fn units(plan:&Value)->Result<&Vec<Value>>{plan["units"].as_array().context("plan units must be an array")}
pub fn build_contract(plan:&Value,sources:&Value)->Result<Value>{
    render::content_template(plan)?;
    let sources=sources.as_object().filter(|s|!s.is_empty()).context("Provide source IDs and their actual files")?;
    let required:BTreeSet<&str>=units(plan)?.iter().map(|u|u["source"]["source_id"].as_str().context("source ID")).collect::<Result<_>>()?;
    let actual:BTreeSet<&str>=sources.keys().map(String::as_str).collect();
    ensure!(required==actual,"Source files must match the reviewed plan's source IDs exactly");
    let mut bindings=Map::new();
    for (id,file) in sources{
        let s=files::snapshot(&path(file)?)?;
        bindings.insert(id.clone(),json!({"path":s["path"],"sha256":s["sha256"]}));
    }
    let mut count=[0usize;5];let mut required_units=Vec::new();
    for u in units(plan)?{
        let mut row=Map::new();
        for k in ["unit_id","kind","source","scope","anchor"]{if let Some(v)=u.get(k){row.insert(k.into(),v.clone());}}
        required_units.push(Value::Object(row));
        match u["kind"].as_str().unwrap_or(""){
            "figure"=>{count[0]+=1;count[1]+=u["images"].as_array().context("figure images")?.len();}
            "formula"=>count[2]+=1,
            "table"=>count[3]+=1,
            "reference"|"bibliography"=>count[4]+=1,
            _=>{}
        }
    }
    Ok(json!({"schema_version":1,"sources":bindings,"required_units":required_units,
        "counts":{"figures":count[0],"images":count[1],"equations":count[2],"tables":count[3],"bibliography":count[4]},
        "plan_sha256":files::digest(&serde_json::to_vec(plan)?)}))
}
pub fn prepare(plan:&Value,sources:&Value,output_root:&Path)->Result<Value>{
    let contract=build_contract(plan,sources)?;
    let template=render::content_template(plan)?;
    // Hash the actual executable.
    // A different binary cannot accidentally reuse an older engine's job.
    let binary=files::snapshot(&std::env::current_exe()?)?;
    let engine=json!({"schema_version":1,"engine":"rust","engine_version":ENGINE_VERSION,
        "reference_version":REFERENCE_VERSION,"binary_sha256":binary["sha256"]});
    let request=json!({"plan":plan,"contract":contract,"engine":engine});
    let payload=pretty(&request)?;
    let id=files::digest(&payload);
    fs::create_dir_all(output_root)?;
    let root=files::canonical(output_root)?;
    let directory=root.join(&id);
    match fs::symlink_metadata(&directory){
        Ok(m)=>ensure!(!m.file_type().is_symlink()&&m.is_dir(),"Refusing a symlink or non-directory job directory: {}",directory.display()),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{
            match fs::create_dir(&directory){Ok(())=>{},Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>{},Err(e)=>return Err(e.into())}
        }
        Err(e)=>return Err(e.into())
    }
    let m=fs::symlink_metadata(&directory)?;
    ensure!(!m.file_type().is_symlink()&&m.is_dir(),"Refusing a symlink job directory: {}",directory.display());
    let entries=[("request.json",payload),("plan.json",pretty(plan)?),("contract.json",pretty(&contract)?),("content-template.json",pretty(&template)?)];
    let mut receipts=Map::new();
    for (name,data) in entries{
        let p=directory.join(name);publish(&p,&data)?;
        receipts.insert(name.into(),json!({"path":path_text(&p)?,"sha256":files::digest(&data)}));
    }
    Ok(json!({"job_id":id,"directory":path_text(&directory)?,"files":receipts,
        "next_action":"Fill a new content.json using content-template.json; keep plan.json and contract.json unchanged.",
        "source_fidelity":"not_assessed"}))
}
fn offset(v:&Value)->Option<usize>{v.as_u64().and_then(|n|usize::try_from(n).ok())}
fn change(row:&Value,len:usize,i:usize)->Result<(usize,usize,&str,&str)>{
    let a=offset(&row["start"]).ok_or_else(||anyhow!("Invalid edit offsets at change {i}"))?;
    let b=offset(&row["end"]).ok_or_else(||anyhow!("Invalid edit offsets at change {i}"))?;
    ensure!(a<=b&&b<=len,"Invalid edit offsets at change {i}");
    let before=row["before"].as_str().ok_or_else(||anyhow!("Edit preimage differs at change {i}"))?;
    let after=row["after"].as_str().ok_or_else(||anyhow!("Edit preimage differs at change {i}"))?;
    Ok((a,b,before,after))
}
pub fn replay(original:&str,changes:&Value)->Result<String>{
    let mut chars:Vec<char>=original.chars().collect();
    for (i,row) in changes.as_array().context("changes must be an array")?.iter().enumerate(){
        let(a,b,before,after)=change(row,chars.len(),i)?;
        ensure!(chars[a..b].iter().copied().eq(before.chars()),"Edit preimage differs at change {i}");
        chars.splice(a..b,after.chars());
    }
    Ok(chars.into_iter().collect())
}
fn span(row:&Value,len:usize)->Result<(usize,usize)>{
    let a=offset(&row["start_offset"]).context("invalid span start")?;
    let b=offset(&row["end_offset"]).context("invalid span end")?;
    ensure!(a<=b&&b<=len,"invalid rendering span: {}",row["unit_id"]);
    Ok((a,b))
}
fn byte_offsets(s:&str)->Vec<usize>{s.char_indices().map(|(i,_)|i).chain(std::iter::once(s.len())).collect()}
fn moved(n:usize,delta:i128)->Result<usize>{usize::try_from(n as i128+delta).context("invalid rebased offset")}
pub fn rebase(rendering:&Value,normalized:&Value)->Result<Value>{
    let original=rendering["text"].as_str().context("rendering text")?;
    let updated=normalized["text"].as_str().context("normalized text")?;
    ensure!(replay(original,&normalized["changes"])?==updated,"Normalization edits do not reproduce the normalized text");
    let orig=byte_offsets(original);let mut result=rendering.clone();
    let rows=result["mapping"].as_array_mut().context("rendering mapping")?;
    for row in rows.iter(){
        let(a,b)=span(row,orig.len()-1)?;
        ensure!(row["sha256"]==files::digest(&original.as_bytes()[orig[a]..orig[b]]),"Original rendering span differs: {}",row["unit_id"]);
    }
    let mut len=orig.len()-1;
    for (i,edit) in normalized["changes"].as_array().context("changes")?.iter().enumerate(){
        let(start,end,_,after)=change(edit,len,i)?;
        let delta=after.chars().count() as i128-(end-start) as i128;
        for row in rows.iter_mut(){
            let(a,b)=span(row,len)?;
            if end<=a{row["start_offset"]=json!(moved(a,delta)?);row["end_offset"]=json!(moved(b,delta)?);}
            else if start>=b{continue;}
            else if a<=start&&end<=b{row["end_offset"]=json!(moved(b,delta)?);}
            else{return Err(anyhow!("Formatting edit crosses a unit boundary: {}",row["unit_id"]));}
        }
        len=moved(len,delta)?;
    }
    let offsets=byte_offsets(updated);
    let starts:Vec<usize>=std::iter::once(0).chain(updated.chars().enumerate().filter_map(|(i,c)|(c=='\n').then_some(i+1))).collect();
    for row in rows.iter_mut(){
        let(a,b)=span(row,offsets.len()-1)?;
        ensure!(starts.binary_search(&a).is_ok()&&(b==offsets.len()-1||updated.as_bytes()[offsets[b]]==b'\n'),
            "Normalized unit is not line-aligned: {}",row["unit_id"]);
        row["start_line"]=json!(starts.partition_point(|v|*v<=a));
        row["end_line"]=json!(starts.partition_point(|v|*v<=a.max(b.saturating_sub(1))));
        row["sha256"]=json!(files::digest(&updated.as_bytes()[offsets[a]..offsets[b]]));
    }
    result["text"]=json!(updated);result["output_sha256"]=json!(files::digest(updated.as_bytes()));
    result["normalization_input_sha256"]=normalized.get("input_sha256").cloned().context("normalization input hash")?;
    result["mapping_basis"]=json!("original renderer spans rebased through verified sequential formatting edits");
    Ok(result)
}


// One native invocation owns the complete mechanical pipeline. Original source
// files are read only, and run folders are content and executable addressed.
const STAGES:[(&str,&str);6]=[
    ("input","输入与版本绑定"),("render","内容单元与段落组装"),
    ("normalize","量值、空行与块格式化"),("mapping","来源与交付位置更新"),
    ("accept","结构、链接与资源验收"),("save","保存与回读"),
];
fn stage(events:&mut Vec<Value>,index:usize,status:&str,progress:&mut impl FnMut(usize,usize,&str)){
    let (id,label)=STAGES[index];
    events.push(json!({"stage":id,"label":label,"status":status,"observation":"direct_rust_execution"}));
    progress(events.len(),STAGES.len(),&format!("{label}：{status}"));
}
fn engine()->Result<Value>{
    let binary=files::snapshot(&std::env::current_exe()?)?;
    Ok(json!({"schema_version":1,"engine":"rust","engine_version":ENGINE_VERSION,
        "reference_version":REFERENCE_VERSION,"binary_sha256":binary["sha256"]}))
}
fn resolve_contract(contract:&Value,base:&Path)->Result<Value>{
    let mut c=contract.clone();
    if let Some(sources)=c.get_mut("sources").and_then(Value::as_object_mut){
        for v in sources.values_mut(){
            if let Some(s)=v["path"].as_str(){
                let p=Path::new(s);
                let resolved=acceptance::resolve(&if p.is_absolute(){p.to_path_buf()}else{base.join(p)})?;
                v["path"]=json!(path_text(&resolved)?);
            }
        }
    }
    Ok(c)
}
fn publish_record(directory:&Path,receipts:&mut BTreeMap<String,String>,name:&str,data:&[u8])->Result<()>{
    publish(&directory.join(name),data)?;
    receipts.insert(name.into(),files::digest(data));Ok(())
}
fn phases(norm:&Option<Value>,audit:&Option<Value>,events:&[Value])->Value{
    let changes=norm.as_ref().and_then(|v|v["changes"].as_array()).map(Vec::as_slice).unwrap_or(&[]);
    let repairs=changes.iter().map(|row|{
        let mut v=Map::new();
        for k in ["stage","rule","line","column","start","end","before","after"]{if let Some(x)=row.get(k){v.insert(k.into(),x.clone());}}
        Value::Object(v)
    }).collect::<Vec<_>>();
    let mut pending=Vec::new();let mut failures=Vec::new();
    if let Some(n)=norm{
        if let Some(rows)=n["unresolved"].as_array(){
            for row in rows.iter().filter(|r|r["severity"]!="info"){
                let mut r=row.clone();r["detected_in"]=json!(row["stage"].as_str().unwrap_or("normalize"));r["cause_status"]=json!("not_established");pending.push(r);
            }
        }
    }
    if let Some(a)=audit{
        for row in a["checks"].as_array().into_iter().flatten().filter(|r|r["status"]=="fail"){
            let mut r=row.clone();r["detected_in"]=row["stage"].clone();r["cause_status"]=json!("not_established");failures.push(r);
        }
        for row in a["review_prompts"].as_array().into_iter().flatten(){
            let mut r=row.clone();r["detected_in"]=row["stage"].clone();r["cause_status"]=json!("not_established");pending.push(r);
        }
    }
    json!({"schema_version":1,"executed_stages":events,"automatic_repairs":repairs,"observed_failures":failures,
        "review_prompts":pending,"unobserved_model_processes":["pdf_text_extraction","docx_extraction","source_segmentation",
            "title_recognition","author_affiliation_recognition","translation","citation_identity_matching"],
        "boundary":"This local run observes rendering and file checks; an output defect alone does not prove which upstream model operation caused it."})
}
fn report(result:&Value,norm:&Option<Value>,audit:&Option<Value>)->String{
    let mut s=format!("# Rust 规范化与机械验收\n\n运行标识：{}\n\n程序状态：**{}**。来源语义核对：**not_assessed**。\n\n该状态仅描述程序实际执行的机械检查，不自动确认译义、裁剪和来源清单完整性。\n\n## 阶段记录\n\n| 阶段 | 状态 |\n|---|---|\n",result["run_id"].as_str().unwrap_or(""),result["status"].as_str().unwrap_or(""));
    for row in result["stages"].as_array().into_iter().flatten(){
        s+=&format!("| {} | {} |\n",row["label"].as_str().unwrap_or(""),row["status"].as_str().unwrap_or(""));
    }
    s+="\n## 定位信息\n\n";
    for row in result["issues"].as_array().into_iter().flatten(){s+=&format!("- {}：{}\n",row["code"].as_str().unwrap_or(""),row["message"].as_str().unwrap_or(""));}
    if result["issues"].as_array().is_some_and(Vec::is_empty){s+="没有程序执行异常。具体检查与保留提示见下方。\n";}
    if let Some(n)=norm{
        s+=&format!("\n## 自动规范化\n\n记录 {} 次变换；已逐项回放并验证二次运行字节一致。\n\n原稿保存在 original.md，实际输出为 normalized.md；原值、新值与位置见 normalization.json。\n",n["changes"].as_array().map_or(0,Vec::len));
        for row in n["unresolved"].as_array().into_iter().flatten(){s+=&format!("- {}，行 {}：{}\n",row["code"].as_str().unwrap_or("NORMALIZE_REVIEW"),row["line"],row["message"].as_str().unwrap_or(""));}
    }
    if let Some(a)=audit{
        s+=&format!("\n## 机械检查\n\n机械规则：**{}**；来源合同覆盖：**{}**。\n\n详细 expected / actual、检查代码与位置保存在 acceptance.json。\n",a["mechanical_status"].as_str().unwrap_or(""),a["source_coverage"].as_str().unwrap_or(""));
        for row in a["checks"].as_array().into_iter().flatten().filter(|r|r["status"]=="fail"){
            s+=&format!("- {} / {} / {}：期望 {}；实际 {}。\n",row["stage"].as_str().unwrap_or(""),row["code"].as_str().unwrap_or(""),row["location"].as_str().unwrap_or(""),row["expected"],row["actual"]);
        }
        s+=&format!("\n另有 {} 条启发式提示；未将提示直接计作来源语义失败。\n",a["review_prompts"].as_array().map_or(0,Vec::len));
    }
    s+="\n## 结果边界\n\n缺少来源合同或单元映射时，未核实范围明确保留。程序不修改源 PDF、DOCX、已有原稿或原资源，不写入 Zotero / Obsidian。\n";s
}
fn trunc_str(s:&str,max_chars:usize)->String{
    let mut it=s.char_indices();
    if let Some((idx,_))=it.nth(max_chars){format!("{}...",&s[..idx])}else{s.to_owned()}
}
fn summary(norm:&Option<Value>,audit:&Option<Value>,tables_converted:usize)->Value{
    let norm_changes=norm.as_ref().and_then(|n|n["changes"].as_array()).map_or(0,Vec::len);
    let norm_unresolved=norm.as_ref().and_then(|n|n["unresolved"].as_array()).map_or(0,Vec::len);
    let unresolved_items:Vec<Value>=norm.as_ref().and_then(|n|n["unresolved"].as_array()).into_iter().flatten().take(10).map(|row|{
        let mut m=Map::new();
        for k in ["code","line","start"]{if let Some(v)=row.get(k){m.insert(k.into(),v.clone());}}
        if let Some(msg)=row.get("message").and_then(Value::as_str){m.insert("message".into(),json!(trunc_str(msg,160)));}
        Value::Object(m)
    }).collect();
    let mut failed_checks=Vec::new();
    let mut prompt_total=0usize;
    let mut prompt_counts:BTreeMap<String,usize>=BTreeMap::new();
    let mut review_prompts_sample=Vec::new();
    if let Some(a)=audit{
        for row in a["checks"].as_array().into_iter().flatten().filter(|r|r["status"]=="fail"){
            let mut item=json!({"stage":row["stage"],"code":row["code"],"location":row["location"]});
            if let Some(exp)=row.get("expected").filter(|v|!v.is_null()){
                item["expected"]=match exp.as_str(){Some(s)=>json!(trunc_str(s,160)),None=>json!(trunc_str(&exp.to_string(),160))};
            }
            if let Some(act)=row.get("actual").filter(|v|!v.is_null()){
                item["actual"]=match act.as_str(){Some(s)=>json!(trunc_str(s,160)),None=>json!(trunc_str(&act.to_string(),160))};
            }
            failed_checks.push(item);
        }
        if let Some(prompts)=a["review_prompts"].as_array(){
            prompt_total=prompts.len();
            for row in prompts{
                let code=row["code"].as_str().unwrap_or("UNKNOWN").to_owned();
                *prompt_counts.entry(code).or_insert(0)+=1;
                if review_prompts_sample.len()<10{
                    let mut p=json!({"code":row["code"],"location":row["location"]});
                    if let Some(msg)=row.get("message").and_then(Value::as_str){p["message"]=json!(trunc_str(msg,160));}
                    review_prompts_sample.push(p);
                }
            }
        }
    }
    json!({
        "normalization_changes":norm_changes,
        "normalization_unresolved":norm_unresolved,
        "unresolved_items":unresolved_items,
        "tables_converted":tables_converted,
        "mechanical_status":audit.as_ref().and_then(|a|a["mechanical_status"].as_str()),
        "source_coverage":audit.as_ref().and_then(|a|a["source_coverage"].as_str()),
        "failed_checks":failed_checks,
        "review_prompts_total":prompt_total,
        "review_prompts_by_code":prompt_counts,
        "review_prompts_sample":review_prompts_sample,
    })
}
pub(crate) fn ensure_outside_vault(dest:&Path)->Result<()>{
    if let Ok(m)=fs::symlink_metadata(dest){
        ensure!(!files::is_linked(dest,&m),"Refusing symlink or reparse point write_normalized_to target: {}",dest.display());
    }
    let abs=if dest.is_absolute(){dest.to_path_buf()}else{std::env::current_dir()?.join(dest)};
    let mut cur=abs.parent();
    while let Some(dir)=cur{
        ensure!(!dir.join(".obsidian").exists(),"Refusing write_normalized_to inside an Obsidian vault: {}",dest.display());
        cur=dir.parent();
    }
    let mut existing=abs.parent();
    while let Some(dir)=existing{
        if dir.exists(){
            let canon=files::canonical(dir)?;
            let mut ccur=Some(canon.as_path());
            while let Some(cd)=ccur{
                ensure!(!cd.join(".obsidian").exists(),"Refusing write_normalized_to inside an Obsidian vault: {}",dest.display());
                ccur=cd.parent();
            }
            break;
        }
        existing=dir.parent();
    }
    Ok(())
}
#[derive(Clone,Copy,Debug,Default)]
pub struct ProcessOptions<'a>{
    pub write_normalized_to:Option<&'a Path>,
    pub strip_display_trailing_punctuation:bool,
    pub force_lf:bool,
    pub convert_markdown_tables:bool,
}
pub fn process(source:Option<&Path>,plan:&Value,content:&Value,output:&Path,contract:&Value,
               resource_root:Option<&Path>,progress:impl FnMut(usize,usize,&str))->Result<Value>{
    process_with_options(source,plan,content,output,contract,resource_root,ProcessOptions::default(),progress)
}
pub fn process_with_options(source:Option<&Path>,plan:&Value,content:&Value,output:&Path,contract:&Value,
                            resource_root:Option<&Path>,options:ProcessOptions<'_>,progress:impl FnMut(usize,usize,&str))->Result<Value>{
    Ok(process_with_timings(source,plan,content,output,contract,resource_root,options,false,progress)?.0)
}
fn resource_preflight(text:&str,root:&Path,started:std::time::Instant)->Result<Option<(Value,Value)>>{
    let missing=acceptance::missing_resources(text,root)?;
    if missing.is_empty(){return Ok(None);}
    let timings=json!({"total":started.elapsed().as_millis()});
    Ok(Some((json!({"schema_version":1,"status":"needs_resources","resource_root":path_text(root)?,
        "issues":missing,"timings_ms":timings,"source_fidelity":"not_assessed"}),timings)))
}
pub(crate) fn process_with_timings(source:Option<&Path>,plan:&Value,content:&Value,output:&Path,contract:&Value,
                                   resource_root:Option<&Path>,options:ProcessOptions<'_>,preflight_resources:bool,mut progress:impl FnMut(usize,usize,&str))->Result<(Value,Value)>{
    let started=std::time::Instant::now();
    if let Some(dest)=options.write_normalized_to{ensure_outside_vault(dest)?;}
    let source_path=source.map(files::canonical).transpose()?;
    let original=source_path.as_ref().map(fs::read).transpose()?;
    let base=if let Some(p)=&source_path{p.parent().context("source parent")?.to_path_buf()}else{std::env::current_dir()?};
    let contract=resolve_contract(contract,&base)?;
    let resource_root=acceptance::resolve(resource_root.unwrap_or(&base))?;
    if preflight_resources{
        if let Some(text)=original.as_deref().and_then(|bytes|std::str::from_utf8(bytes).ok()){
            if let Some(issue)=resource_preflight(text,&resource_root,started)?{return Ok(issue);}
        }
    }
    let eng=engine()?;
    let input=match (&source_path,&original){(Some(p),Some(b))=>json!({"path":path_text(p)?,"sha256":files::digest(b)}),_=>Value::Null};
    let mut request=json!({"schema_version":1,"mode":if source.is_some(){"markdown"}else{"structured"},"input":input,
        "plan":plan,"content":content,"contract":contract,"resource_root":path_text(&resource_root)?,"engine":eng});
    if options.strip_display_trailing_punctuation{request["strip_display_trailing_punctuation"]=json!(true);}
    if options.force_lf{request["force_lf"]=json!(true);}
    if options.convert_markdown_tables{request["convert_markdown_tables"]=json!(true);}
    let request_bytes=pretty(&request)?;let id=files::digest(&request_bytes);
    fs::create_dir_all(output)?;let root=files::canonical(output)?;let directory=root.join(&id);
    match fs::create_dir(&directory){Ok(())=>{},Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>{},Err(e)=>return Err(e.into())}
    let m=fs::symlink_metadata(&directory)?;
    ensure!(m.is_dir()&&!m.file_type().is_symlink(),"Refusing a symlink or non-directory run folder");
    let mut receipts=BTreeMap::new();let mut events=Vec::new();let mut issues=Vec::new();
    publish_record(&directory,&mut receipts,"request.json",&request_bytes)?;
    stage(&mut events,0,"verified",&mut progress);
    let mut rendering=None;
    let raw=if let Some(bytes)=&original{
        publish_record(&directory,&mut receipts,"original.md",bytes)?;
        stage(&mut events,1,"not_requested",&mut progress);
        match std::str::from_utf8(bytes){Ok(s)=>Some(s.to_owned()),Err(e)=>{issues.push(json!({"stage":"input","code":"INPUT_ENCODING","message":e.to_string()}));None}}
    }else{
        let r=render::render_note(plan,content);
        publish_record(&directory,&mut receipts,"rendering-original.json",&pretty(&r)?)?;
        let text=r["text"].as_str().map(str::to_owned);
        if let Some(s)=&text{
            publish_record(&directory,&mut receipts,"original.md",s.as_bytes())?;
            stage(&mut events,1,"rendered",&mut progress);
        }else{
            for f in r["findings"].as_array().into_iter().flatten(){let mut row=f.clone();row["stage"]=json!("render");issues.push(row);}
            stage(&mut events,1,"blocked",&mut progress);
        }
        rendering=Some(r);text
    };
    if original.is_none() && preflight_resources{
        if let Some(text)=raw.as_deref(){
            if let Some(issue)=resource_preflight(text,&resource_root,started)?{return Ok(issue);}
        }
    }
    let norm_opts=normalize::Options{strip_display_trailing_punctuation:options.strip_display_trailing_punctuation,normalize_line_endings:options.force_lf};
    let conversion_started=std::time::Instant::now();
    let (raw_for_norm,tables_converted)=match (&raw,options.convert_markdown_tables){
        (Some(s),true)=>{let (converted,count)=yaml_emit::convert_markdown_tables(s)?;(Some(converted),count)},
        _=> (raw.clone(),0),
    };
    let normalization_started=std::time::Instant::now();
    let normalized=if let Some(s)=&raw_for_norm{
        let (n,fixed_point)=normalize::normalize_with_replay_proof(s,norm_opts)?;
        let t=n["text"].as_str().context("normalized text")?;
        ensure!(fixed_point&&replay(s,&n["changes"])?==t,
            "Normalization failed replay or idempotence verification");
        let mut slim_n=n.clone();
        if let Some(obj)=slim_n.as_object_mut(){obj.remove("text");}
        publish_record(&directory,&mut receipts,"normalization.json",&pretty(&slim_n)?)?;
        publish_record(&directory,&mut receipts,"normalized.md",t.as_bytes())?;
        stage(&mut events,2,"verified",&mut progress);Some(n)
    }else{stage(&mut events,2,"blocked",&mut progress);None};
    let mapping_started=std::time::Instant::now();
    let rendering=if let (Some(r),Some(n))=(rendering.as_ref(),normalized.as_ref()){
        match rebase(r,n){
            Ok(rebased)=>{
                publish_record(&directory,&mut receipts,"rendering.json",&pretty(&rebased)?)?;
                stage(&mut events,3,"verified",&mut progress);rebased
            },
            Err(e)=>{issues.push(json!({"stage":"mapping","code":"MAPPING_REBASE","message":e.to_string()}));stage(&mut events,3,"blocked",&mut progress);Value::Null}
        }
    }else{stage(&mut events,3,"not_available",&mut progress);Value::Null};
    let note=directory.join("normalized.md");
    let acceptance_started=std::time::Instant::now();
    let audit=if normalized.is_some(){
        let a=acceptance::accept(&note,&contract,Some(&resource_root),&rendering)?;
        publish_record(&directory,&mut receipts,"acceptance.json",&pretty(&a)?)?;
        stage(&mut events,4,a["mechanical_status"].as_str().context("mechanical status")?,&mut progress);Some(a)
    }else{stage(&mut events,4,"blocked",&mut progress);None};
    let unsafe_parse=normalized.as_ref().and_then(|n|n["unresolved"].as_array()).is_some_and(|a|a.iter().any(|r|r["code"].as_str().is_some_and(|s|s.starts_with("UNCLOSED_")||s=="INTERACTING_RULES")));
    let status=if normalized.is_none(){"blocked"}else if audit.as_ref().is_some_and(|a|a["mechanical_status"]=="pass"&&a["source_coverage"]=="verified_against_contract")&&issues.is_empty()&&!unsafe_parse{"mechanical_pass"}else{"needs_attention"};
    let readback_started=std::time::Instant::now();
    stage(&mut events,5,"committing",&mut progress);
    if let Some(a)=&audit{
        ensure!(acceptance::accept(&note,&contract,Some(&resource_root),&rendering)?==*a,
            "Source, resource, candidate or evidence changed after acceptance; no final receipt was published");
    }
    if let Some(p)=&source_path{ensure!(files::snapshot(p)?["sha256"]==request["input"]["sha256"],"Original changed during processing");}
    ensure!(engine()?==eng,"Engine binary changed during processing; no final receipt was published");
    for (name,hash) in &receipts{
        let p=directory.join(name);
        let m=fs::symlink_metadata(&p)?;
        ensure!(m.is_file()&&!m.file_type().is_symlink()&&files::snapshot(&p)?["sha256"]==hash.as_str(),"Published artifact changed during processing: {}",p.display());
    }
    events.last_mut().unwrap()["status"]=json!("verified");
    publish_record(&directory,&mut receipts,"phases.json",&pretty(&phases(&normalized,&audit,&events))?)?;
    let outputs=receipts.iter().map(|(k,v)|Ok((k.clone(),json!({"path":path_text(&directory.join(k))?,"sha256":v})))).collect::<Result<Map<String,Value>>>()?;
    let mut result=json!({"schema_version":1,"run_id":id,"directory":path_text(&directory)?,"status":status,
        "source_fidelity":"not_assessed","summary":summary(&normalized,&audit,tables_converted),"stages":events,"issues":issues,"outputs":outputs});
    let markdown=report(&result,&normalized,&audit);
    publish_record(&directory,&mut receipts,"report.md",markdown.as_bytes())?;
    result["outputs"]["report.md"]=json!({"path":path_text(&directory.join("report.md"))?,"sha256":files::digest(markdown.as_bytes())});
    // All advertised outputs are independently read back before committing the receipt.
    for(name,hash)in &receipts{
        let p=directory.join(name);let m=fs::symlink_metadata(&p)?;
        ensure!(m.is_file()&&!m.file_type().is_symlink()&&files::snapshot(&p)?["sha256"]==hash.as_str(),"Artifact changed before receipt: {}",p.display());
    }
    publish(&directory.join("result.json"),&pretty(&result)?)?;
    if let Some(dest)=options.write_normalized_to{
        ensure_outside_vault(dest)?;
        let norm_text=normalized.as_ref().and_then(|n|n["text"].as_str());
        if !unsafe_parse{
            if let Some(text)=norm_text{
                let parent=dest.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(Path::new("."));
                fs::create_dir_all(parent)?;
                publish_replace(dest,text.as_bytes())?;
                result["published_candidate"]=json!({
                    "written":true,
                    "path":path_text(&files::canonical(dest)?)?,
                    "sha256":files::digest(text.as_bytes()),
                });
            }else{
                result["published_candidate"]=json!({"written":false,"path":path_text(dest)?,"reason":"no_normalized_text"});
            }
        }else{
            result["published_candidate"]=json!({"written":false,"path":path_text(dest)?,"reason":"unsafe_parse"});
        }
    }
    let timings=json!({
        "input_and_render":conversion_started.duration_since(started).as_millis(),
        "convert_tables":normalization_started.duration_since(conversion_started).as_millis(),
        "normalize_and_replay":mapping_started.duration_since(normalization_started).as_millis(),
        "rebase_mapping":acceptance_started.duration_since(mapping_started).as_millis(),
        "accept":readback_started.duration_since(acceptance_started).as_millis(),
        "readback_and_save":readback_started.elapsed().as_millis(),
        "total":started.elapsed().as_millis()
    });
    Ok((result,timings))
}
