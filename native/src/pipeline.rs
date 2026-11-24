//! Freeze source-derived contracts and verified sequential edits. These
//! primitives do not assert that a source plan is exhaustive or semantic.
use crate::{files,parse_json,path,path_text,render,ENGINE_VERSION,REFERENCE_VERSION};
use anyhow::{anyhow,ensure,Context,Result};
use serde_json::{json,Map,Value};
use std::{collections::BTreeSet,fs,io::Write,path::{Path,PathBuf}};

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
    // Hash the actual executable rather than the old Python source files.
    // A different binary cannot accidentally reuse an older engine's job.
    let binary=files::snapshot(&std::env::current_exe()?)?;
    let engine=json!({"schema_version":1,"engine":"rust-native","engine_version":ENGINE_VERSION,
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
fn change<'a>(row:&'a Value,len:usize,i:usize)->Result<(usize,usize,&'a str,&'a str)>{
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
