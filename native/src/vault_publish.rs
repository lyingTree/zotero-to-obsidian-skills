//! Read-only native vault publication preflight. Bind actual candidate,
//! resources, source and destination bytes before any transaction. Planning
//! is not user authorization, source fidelity or a completed vault write.
use anyhow::{ensure,Context,Result};
use serde_json::{json,Value};
use std::{collections::{BTreeMap,BTreeSet},path::{Path,PathBuf},fs};
use crate::{check_ledger,files,inventory,path,path_text,properties,workflow};
const BOUNDARY:&str="Read-only placement and actual file precondition plan. No vault write or note sync was performed; resource ownership, independently observed source checks and final authorization remain separate.";
fn key(s:&str)->String{
    use unicode_normalization::UnicodeNormalization;
    use unicode_casefold::UnicodeCaseFold;
    s.nfc().collect::<String>().case_fold().collect::<String>().nfc().collect()
}
fn segment(s:&str)->Result<()>{
    ensure!(!s.is_empty()&&!matches!(s,"."|"..")&&!s.ends_with(['.',' '])&&!s.chars().any(|c|c.is_control()||"\\/:<>\"|?*".contains(c)),"Unsafe vault path segment");
    let stem=s.split('.').next().unwrap_or("").trim_end_matches(' ').to_ascii_uppercase();
    ensure!(!["CON","PRN","AUX","NUL","CONIN$","CONOUT$"].contains(&stem.as_str())&&
        !stem.strip_prefix("COM").or_else(||stem.strip_prefix("LPT")).is_some_and(|n|n.chars().count()==1&&"123456789¹²³".contains(n)),"Reserved Windows filename");
    Ok(())
}
fn relative(s:&str)->Result<PathBuf>{let mut p=PathBuf::new();for v in s.split('/'){segment(v)?;p.push(v);}Ok(p)}
fn linked(m:&fs::Metadata)->bool{
    if m.file_type().is_symlink(){return true;}
    #[cfg(windows)]{use std::os::windows::fs::MetadataExt;if m.file_attributes()&0x400!=0{return true;}}
    false
}
fn actual_root(p:&Path)->Result<PathBuf>{
    let m=fs::symlink_metadata(p)?;ensure!(m.is_dir()&&!linked(&m),"Expected real directory");
    files::canonical(p)
}
/// Check every existing parent, including case/normalization aliases, without
/// creating a directory. Future path components are explicit and portable.
fn target(root:&Path,raw:&str,directory:bool)->Result<PathBuf>{
    let r=relative(raw)?;let mut p=root.to_owned();let count=r.components().count();let mut missing=false;
    for (i,c) in r.components().enumerate(){
        let name=c.as_os_str().to_str().context("Non-Unicode target")?;
        if !missing{
            let wanted=key(name);
            for e in fs::read_dir(&p)?{
                let e=e?;let actual=e.file_name();let actual=actual.to_str().context("Non-Unicode sibling path")?;
                ensure!(key(actual)!=wanted||actual==name,"Case/Unicode path collision: {actual} vs {name}");
            }
        }
        p.push(name);
        match fs::symlink_metadata(&p){
            Ok(m)=>{
                ensure!(!linked(&m)&&(if directory||i+1<count{m.is_dir()}else{m.is_file()}),"Unsafe linked or wrong-type vault target");
                ensure!(files::canonical(&p)?.starts_with(root),"Target escapes vault root");
            }
            Err(e) if e.kind()==std::io::ErrorKind::NotFound=>missing=true,
            Err(e)=>return Err(e.into())
        }
    }Ok(p)
}
fn record(p:&Path)->Result<Option<Value>>{
    match fs::symlink_metadata(p){
        Ok(m)=>{ensure!(!linked(&m)&&m.is_file(),"Unsafe existing target");Ok(Some(check_ledger::bind_file(p)?))},
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ok(None),
        Err(e)=>Err(e.into())
    }
}
fn reason(code:&str,path:&Path,message:&str)->Value{json!({"code":code,"path":path.to_string_lossy(),"message":message})}
fn sha(s:&str)->bool{s.len()==64&&s.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))}
pub struct Options<'a>{
    pub run:&'a Path,pub run_sha:&'a str,pub vault:&'a Path,pub literature_relative:&'a str,pub assets_relative:&'a str,
    pub note_relative:&'a str,pub expected_existing_sha:Option<&'a str>,pub inspect_workflow:bool,
}
pub fn plan(opt:Options<'_>,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    ensure!(sha(opt.run_sha),"Actual workflow SHA-256 required");
    if let Some(h)=opt.expected_existing_sha{ensure!(sha(h),"Expected target SHA-256 must be an actual digest");}
    let (raw,run_binding)=files::read_bound(opt.run)?;
    ensure!(run_binding["sha256"]==opt.run_sha,"Workflow changed before placement planning");
    let run=crate::parse_json(std::str::from_utf8(&raw)?)?;
    ensure!(run["schema_version"].as_u64()==Some(1)&&run["workflow_profile"]=="complete_pdf_candidate_v1","Actual complete-workflow record required");
    let root=actual_root(opt.vault)?;
    ensure!(actual_root(&root.join(".obsidian"))?.starts_with(&root),"Confirm an actual vault root");
    let lit=target(&root,opt.literature_relative,true)?;
    ensure!(lit.is_dir(),"Confirmed literature directory must exist");
    let assets=target(&root,opt.assets_relative,true)?;
    ensure!(assets.starts_with(&lit)&&assets!=lit,"Confirm separate asset directory within literature root");
    let note=target(&root,opt.note_relative,false)?;
    ensure!(note.starts_with(&lit)&&!note.starts_with(&assets)&&note.extension().is_some_and(|v|v=="md"),"Note target must be a Markdown file within the literature root, outside assets");
    let rp=path(&run_binding["path"])?;let base=rp.parent().context("Run directory")?;
    let work=actual_root(&base.join(path(&run["config"]["work_root"])?))?;
    let resource_root=actual_root(&base.join(path(&run["config"]["resource_root"])?))?;
    ensure!(!work.starts_with(&root)&&!resource_root.starts_with(&root)&&!rp.starts_with(&root),"Work, candidate resources and receipt must remain outside vault");
    let cp=files::canonical(&base.join(path(&run["config"]["candidate_path"])?))?;
    ensure!(!cp.starts_with(&root)&&cp!=note,"Candidate must be an independent library-external file");
    ensure!(fs::metadata(&cp)?.len()<=32*1024*1024,"Candidate exceeds preflight byte budget");
    let (raw,candidate)=files::read_bound(&cp)?;
    ensure!(raw.len()<=32*1024*1024,"Candidate exceeded preflight budget while reading");
    let text=std::str::from_utf8(&raw)?;
    let mut findings=Vec::new();
    if raw.contains(&b'\r'){findings.push(reason("CANDIDATE_NOT_LF",&cp,"Final note must preserve UTF-8 LF bytes."));}
    let properties=properties::split(&raw)?;
    if let Some(payload)=properties.payload{
        let yaml=properties::parse_yaml(payload)?;
        if !yaml.is_object()||yaml.get("库内参考文献").is_some(){findings.push(reason("CANDIDATE_PROPERTIES",&cp,"Resolve deprecated or malformed YAML before publication."));}
    }else{findings.push(reason("CANDIDATE_PROPERTIES",&cp,"Final candidate has no top-level property mapping."));}
    if run["naming"]["filename"].as_str()!=note.file_name().and_then(|s|s.to_str())||run["naming"]["status"]=="pending"{
        findings.push(reason("NAMING_PLAN_PENDING_OR_DIFFERENT",&note,"Complete the actual naming plan for this target."));
    }
    let existing=record(&note)?;
    match (&existing,opt.expected_existing_sha){
        (None,None)=>{},
        (Some(r),Some(h)) if r["sha256"]==h=>{},
        _=>findings.push(reason("TARGET_PRECONDITION_MISMATCH",&note,"Preserve the current target; explicit expected prior hash or absence differs.")),
    }
    let mut inputs=vec![candidate.clone(),run_binding.clone()];
    let mut sources=Vec::new();let mut identities=BTreeSet::new();
    for s in run["identity"]["sources"].as_array().filter(|a|!a.is_empty()).context("Retain actual workflow sources")?{
        let id=s["id"].as_str().filter(|s|!s.is_empty()).context("Source identity")?;
        ensure!(identities.insert(id),"Duplicate source ID");
        let b=check_ledger::bind_file(&base.join(path(&s["path"])?))?;
        ensure!(b["sha256"]==s["sha256"],"Workflow source changed");
        inputs.push(b.clone());sources.push(json!({"id":id,"file":b}));
    }
    let inv=inventory::inspect(text)?;
    let mut embeds=BTreeMap::<String,Vec<usize>>::new();
    for (href,line,_) in inv.resources{
        let decoded=percent_encoding::percent_decode_str(&href).decode_utf8()?.into_owned();
        ensure!(!decoded.contains(['#','?']),"Use actual vault-relative embed resource paths, not fragments or query parameters");
        relative(&decoded)?;
        embeds.entry(decoded).or_default().push(line);
    }
    let mut resource_rows=Vec::with_capacity(embeds.len());let n=embeds.len();
    for (i,(href,lines)) in embeds.into_iter().enumerate(){
        let dst=target(&root,&href,false)?;
        ensure!(dst.starts_with(&assets),"Embed outside declared vault asset directory");
        let src=target(&resource_root,&href,false)?;
        let source=record(&src)?.context("Actual candidate embed bytes missing")?;
        ensure!(fs::metadata(&src)?.len()>0,"Candidate embed is empty");
        inputs.push(source.clone());
        let prior=record(&dst)?;
        let disposition=if let Some(p)=&prior{
            if p["sha256"]!=source["sha256"]{
                findings.push(reason("ASSET_CONFLICT",&dst,"Different existing asset bytes; never overwrite. Choose a new planned path."));"conflict"
            }else{
                findings.push(reason("ASSET_OWNERSHIP_PENDING",&dst,"Equal bytes alone do not establish resource ownership; verify the existing owner."));"same_bytes_ownership_pending"
            }
        }else{"create_if_absent"};
        resource_rows.push(json!({"relative_path":href,"source":source,"destination":path_text(&dst)?,"existing":prior,
            "disposition":disposition,"candidate_lines":lines}));
        progress(i+1,n,"verify actual vault publication resources");
    }
    let inspection=if opt.inspect_workflow{Some(workflow::inspect(opt.run,&mut progress)?)}else{None};
    if inspection.as_ref().is_none_or(|r|r["records_ready"]!=true){
        findings.push(reason("WORKFLOW_NOT_READY",&rp,"Actual complete delivery inspection must pass before publication."));
    }
    if run["scope"]["write_vault"]!=true{findings.push(reason("VAULT_WRITE_NOT_AUTHORIZED",&rp,"Original run does not authorize vault writes."));}
    for input in &inputs{
        ensure!(check_ledger::bind_file(&path(&input["path"])?)?==*input,"Plan input changed while reading");
    }
    ensure!(record(&note)?==existing,"Note target changed during read-only planning");
    for r in &resource_rows{
        ensure!(record(&path(&r["destination"])?)?==r["existing"].as_object().map(|_|r["existing"].clone()),"Resource target changed during planning");
    }
    let ready=findings.is_empty();
    Ok(json!({"schema_version":1,"run":run_binding,"candidate":candidate,"vault_root":path_text(&root)?,
        "literature_root":path_text(&lit)?,"assets_root":path_text(&assets)?,"note_relative_path":opt.note_relative,
        "target_path":path_text(&note)?,"expected_existing_sha256":opt.expected_existing_sha,"existing":existing,
        "sources":sources,"resources":resource_rows,"findings":findings,"workflow_inspection":inspection,
        "preconditions_ready":ready,"vault_modified":false,"source_fidelity":"not_assessed",
        "backup_required":existing.is_some(),"boundary":BOUNDARY}))
}
