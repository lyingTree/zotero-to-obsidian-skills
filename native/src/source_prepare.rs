//! Prepare one or more real DOCX sources using the common inventory contract.
//! PDF requests must remain blocked until full page/image provenance is native.
use crate::{docx,files,path,path_text};
use anyhow::{anyhow,ensure,Context,Result};
use serde_json::{json,Value,Map};
use std::{fs,io::Write,path::{Path,PathBuf}};

fn binding(p:&Path)->Result<Value>{
    let b=files::snapshot(p)?;Ok(json!({"path":b["path"],"sha256":b["sha256"]}))
}
fn no_existing(p:&Path)->Result<()>{
    match fs::symlink_metadata(p){
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ok(()),
        Err(e)=>Err(e.into()),Ok(_)=>Err(anyhow!("Keep the existing evidence record: {}",p.display())),
    }
}
fn exclusive(p:&Path,value:&Value)->Result<()>{
    no_existing(p)?;
    let mut bytes=serde_json::to_vec_pretty(value)?;bytes.push(b'\n');
    let mut f=tempfile::Builder::new().prefix(".pending-source-").tempfile_in(p.parent().context("source parent")?)?;
    f.write_all(&bytes)?;f.as_file().sync_all()?;
    f.persist_noclobber(p).map_err(|e|anyhow!("Cannot exclusively save {}: {}",p.display(),e.error))?;
    ensure!(crate::pipeline::read_json(p)?==*value,"Source inventory did not round-trip");Ok(())
}
fn safe_dir(dir:&Path)->Result<PathBuf>{
    if let Ok(m)=fs::symlink_metadata(dir){ensure!(!m.file_type().is_symlink()&&m.is_dir(),"Refusing a symlink or non-directory source evidence root");}
    fs::create_dir_all(dir)?;files::canonical(dir)
}
fn id_ok(id:&str)->bool{id.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)&&id.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-'))}
pub fn prepare_docx_sources(sources:&Value,root:&Path,roles:&Value,glossary:Option<&Path>,dpi:u64,mut progress:impl FnMut(usize,usize,&str))->Result<PathBuf>{
    let map=sources.as_object().filter(|m|!m.is_empty()).context("Provide at least one source PDF or DOCX")?;
    ensure!((72..=600).contains(&dpi),"dpi must be between 72 and 600");
    ensure!(roles.is_null()||roles.is_object(),"source_roles must be an object");
    let mut ids=Vec::with_capacity(map.len());
    let mut bound=Vec::with_capacity(map.len());
    // Reject non-native inputs before creating any evidence output.
    for (id,v) in map{
        ensure!(id_ok(id),"Invalid source ID: {id}");
        let p=files::canonical(&path(v)?)?;
        ensure!(p.extension().and_then(|s|s.to_str()).is_some_and(|s|s.eq_ignore_ascii_case("docx")),
            "UNMIGRATED_PDF_INVENTORY: {id}; full PDF provenance is required and there is no Python fallback");
        let role=roles.get(id).cloned().unwrap_or(json!("pending"));
        ensure!(role.is_string(),"source role must be a string");
        ids.push((id,p.clone(),role));bound.push(binding(&p)?);
    }
    let glossary=match glossary{Some(p)=>Some(binding(p)?),None=>None};
    let root=safe_dir(root)?;
    let output=root.join("source-inventory.json");let terminology=root.join("terminology.json");
    no_existing(&output)?;if glossary.is_some(){no_existing(&terminology)?;}
    let evidence=safe_dir(&root.join("source-evidence"))?;
    ensure!(evidence.starts_with(&root),"source evidence directory escapes work root");
    // Preflight all destinations, including pre-existing partial preparations.
    for (id,_,_) in &ids{no_existing(&evidence.join(id))?;}
    let mut rows=Vec::with_capacity(ids.len());
    for (i,(id,p,role)) in ids.iter().enumerate(){
        let destination=evidence.join(id);
        fs::create_dir(&destination)?;
        let mut native=docx::prepare_docx_source(p,id,&destination,|n,total,label|progress(n,total,label))?;
        ensure!(binding(p)?==bound[i],"{id}: source changed during evidence preparation");
        native["id"]=json!(id);native["role"]=role.clone();
        native["path"]=bound[i]["path"].clone();native["sha256"]=bound[i]["sha256"].clone();
        let headers:Map<String,Value>=["title","authors","affiliations","publication_date"].iter().map(|key|((*key).into(),json!({"status":"pending"}))).collect();
        native["header_fields"]=json!(headers);rows.push(native);
        progress(i+1,ids.len(),"DOCX sources");
    }
    let mut inventory=json!({"schema_version":1,"evidence_boundary":"Rendered files do not prove that a model viewed or understood them.",
        "sources":rows,"units":[],"excluded_blocks":[],"excluded_pages":[]});
    if let Some(g)=glossary{
        ensure!(binding(Path::new(g["path"].as_str().unwrap()))?==g,"glossary changed during inventory preparation");
        let value=json!({"schema_version":1,"glossary":g,"review_status":"pending","covered_units":[],"adopted":[],"unlisted":[],
            "no_unlisted_reason":null,"evidence_boundary":"A ledger records decisions; it does not prove translation correctness."});
        exclusive(&terminology,&value)?;inventory["terminology_path"]=json!(path_text(&terminology)?);
    }
    for (i,(_,p,_)) in ids.iter().enumerate(){ensure!(binding(p)?==bound[i],"source changed before inventory publication");}
    exclusive(&output,&inventory)?;
    Ok(output)
}
