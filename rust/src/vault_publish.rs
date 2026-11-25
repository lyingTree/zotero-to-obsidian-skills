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
fn actual_root(p:&Path)->Result<PathBuf>{
    let m=fs::symlink_metadata(p)?;ensure!(m.is_dir()&&!files::is_linked(p,&m),"Expected real directory");
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
                ensure!(!files::is_linked(&p,&m)&&(if directory||i+1<count{m.is_dir()}else{m.is_file()}),"Unsafe linked or wrong-type vault target");
                ensure!(files::canonical(&p)?.starts_with(root),"Target escapes vault root");
            }
            Err(e) if e.kind()==std::io::ErrorKind::NotFound=>missing=true,
            Err(e)=>return Err(e.into())
        }
    }Ok(p)
}
fn record(p:&Path)->Result<Option<Value>>{
    match fs::symlink_metadata(p){
        Ok(m)=>{ensure!(!files::is_linked(p,&m)&&m.is_file(),"Unsafe existing target");Ok(Some(check_ledger::bind_file(p)?))},
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


// Independent Markdown file transaction. The full workflow gate remains
// mandatory for complete_workflow; standalone edits do not grant acceptance.
use std::io::{self,Write};
const TX_BOUNDARY:&str="Exact file write/readback only. Locks coordinate native writers sharing the backup root, not external editors. Rollback preserves later user changes and all possibly shared assets. No scientific acceptance or Zotero sync is inferred.";
const NOTE_LIMIT:u64=32*1024*1024;
const RESOURCE_LIMIT:u64=4*1024*1024*1024;
fn hash(v:&Value)->Result<&str>{v.as_str().filter(|s|sha(s)).context("Actual SHA-256 required")}
fn prior(v:&Value)->Result<Option<&str>>{if v.is_null(){Ok(None)}else{Ok(Some(hash(v)?))}}
fn verify_prior(p:&Path,h:Option<&str>)->Result<Option<Value>>{
    let current=record(p)?;
    ensure!(current.as_ref().and_then(|b|b["sha256"].as_str())==h,"TARGET_PRECONDITION_MISMATCH: {}",p.display());Ok(current)
}
fn bounded(p:&Path,h:&str)->Result<(Vec<u8>,Value)>{
    crate::check_write::no_link(p)?;
    ensure!(fs::metadata(p)?.len()<=NOTE_LIMIT,"File exceeds transaction input budget");
    let(raw,b)=files::read_bound(p)?;
    ensure!(raw.len() as u64<=NOTE_LIMIT&&b["sha256"]==h&&check_ledger::bind_file(p)?==b,"File changed while reading");
    Ok((raw,b))
}
fn new_bytes(p:&Path,raw:&[u8])->Result<Value>{
    ensure!(record(p)?.is_none(),"Keep existing evidence or target");
    let mut tmp=tempfile::NamedTempFile::new_in(p.parent().context("Output parent")?)?;
    tmp.write_all(raw)?;tmp.as_file().sync_all()?;
    tmp.persist_noclobber(p).map_err(|e|anyhow::anyhow!("Exclusive write failed: {}",e.error))?;
    let b=check_ledger::bind_file(p)?;ensure!(b["sha256"]==files::digest(raw),"Readback differs");Ok(b)
}
fn journal(p:&Path,v:&Value)->Result<Value>{
    let mut bytes=serde_json::to_vec_pretty(v)?;bytes.push(b'\n');new_bytes(p,&bytes)
}
fn parents(root:&Path,rel:&str)->Result<PathBuf>{
    let parts:Vec<_>=rel.split('/').collect();let mut prefix=String::new();
    for name in &parts[..parts.len().saturating_sub(1)]{
        if !prefix.is_empty(){prefix.push('/');}prefix.push_str(name);
        let p=target(root,&prefix,true)?;
        match fs::create_dir(&p){Ok(())=>{},Err(e) if e.kind()==io::ErrorKind::AlreadyExists=>{},Err(e)=>return Err(e.into())}
        ensure!(actual_root(&p)?.starts_with(root),"Parent escaped vault");
    }target(root,rel,false)
}
fn replace_note(p:&Path,old:&str,raw:&[u8])->Result<Value>{
    crate::check_write::no_link(p)?;
    let mut tmp=tempfile::NamedTempFile::new_in(p.parent().context("Note parent")?)?;
    tmp.write_all(raw)?;tmp.as_file().sync_all()?;fs::set_permissions(tmp.path(),fs::metadata(p)?.permissions())?;
    verify_prior(p,Some(old))?;
    tmp.persist(p).map_err(|e|anyhow::anyhow!("Atomic replacement failed: {}",e.error))?;
    let b=check_ledger::bind_file(p)?;ensure!(b["sha256"]==files::digest(raw),"Note readback differs");Ok(b)
}
fn copy_resource(src:&Path,h:&str,dst:&Path)->Result<Value>{
    crate::check_write::no_link(src)?;verify_prior(dst,None)?;
    let size=fs::metadata(src)?.len();ensure!(size>0&&size<=RESOURCE_LIMIT,"Resource exceeds budget or is empty");
    let mut tmp=tempfile::NamedTempFile::new_in(dst.parent().context("Asset parent")?)?;
    use std::io::Read;
    let n=io::copy(&mut fs::File::open(src)?.take(RESOURCE_LIMIT+1),&mut tmp)?;
    tmp.as_file().sync_all()?;
    ensure!(n==size&&files::snapshot(tmp.path())?["sha256"]==h&&check_ledger::bind_file(src)?["sha256"]==h,"Asset changed during copy");
    tmp.persist_noclobber(dst).map_err(|e|anyhow::anyhow!("Asset already exists or publication failed: {}",e.error))?;
    let b=check_ledger::bind_file(dst)?;ensure!(b["sha256"]==h,"Asset readback differs");Ok(b)
}
fn tx_lock(vault:&Path,backup:&Path)->Result<crate::check_write::TargetLock>{
    ensure!(!backup.starts_with(vault)&&!vault.starts_with(backup),"Use a separate existing backup root outside vault");
    crate::check_write::lock_target(&backup.join(format!("vault-{}",files::digest(path_text(vault)?.as_bytes()))))
}
pub struct CommitOptions<'a>{
    pub vault:&'a Path,pub backup_root:&'a Path,pub literature_relative:&'a str,pub assets_relative:&'a str,
    pub note_relative:&'a str,pub candidate:&'a Path,pub candidate_sha:&'a str,pub expected_existing_sha:Option<&'a str>,
    pub resources:&'a Value,pub authorized:bool,pub scope:&'a str,pub workflow_run:Option<(&'a Path,&'a str)>,
}
pub fn commit(opt:CommitOptions<'_>,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    ensure!(opt.authorized,"Explicit write authorization required");
    ensure!(matches!(opt.scope,"standalone_markdown"|"complete_workflow")&&sha(opt.candidate_sha)&&opt.expected_existing_sha.is_none_or(sha),"Explicit scope and SHA-256 preconditions required");
    let root=actual_root(opt.vault)?;actual_root(&root.join(".obsidian"))?;
    let backup=actual_root(opt.backup_root)?;let _lock=tx_lock(&root,&backup)?;
    let lit=target(&root,opt.literature_relative,true)?;ensure!(lit.is_dir(),"Existing literature root required");
    let assets=target(&root,opt.assets_relative,true)?;ensure!(assets!=lit&&assets.starts_with(&lit),"Asset root must lie inside literature root");
    let dst=target(&root,opt.note_relative,false)?;
    ensure!(dst.starts_with(&lit)&&!dst.starts_with(&assets)&&dst.extension().is_some_and(|e|e=="md"),"Only scoped Markdown target");
    crate::check_write::no_link(opt.candidate)?;let cp=files::canonical(opt.candidate)?;
    ensure!(!cp.starts_with(&root),"Independent candidate outside vault required");
    let(raw,cb)=bounded(&cp,opt.candidate_sha)?;
    let text=std::str::from_utf8(&raw)?;
    ensure!(!raw.contains(&b'\r'),"Exact UTF-8 LF candidate required");
    let yaml=properties::parse_yaml(properties::split(&raw)?.payload.context("Beginning YAML required")?)?;
    ensure!(yaml.is_object()&&yaml.get("库内参考文献").is_none(),"Invalid or deprecated YAML");
    let findings=crate::validate::validate_note(text);
    ensure!(!findings.iter().any(|f|f.level=="error"),"Candidate has unresolved structure: {}",json!(findings));
    let before=verify_prior(&dst,opt.expected_existing_sha)?;
    let full=if opt.scope=="complete_workflow"{
        let(p,h)=opt.workflow_run.context("Original bound workflow required")?;
        let r=plan(Options{run:p,run_sha:h,vault:&root,literature_relative:opt.literature_relative,assets_relative:opt.assets_relative,
            note_relative:opt.note_relative,expected_existing_sha:opt.expected_existing_sha,inspect_workflow:true},&mut progress)?;
        ensure!(r["preconditions_ready"]==true&&r["candidate"]==cb,"Complete workflow is not ready");Some(r)
    }else{ensure!(opt.workflow_run.is_none(),"Standalone scope cannot claim full workflow");None};
    let mut embeds=BTreeSet::new();
    for(href,_,_)in inventory::inspect(text)?.resources{
        let decoded=percent_encoding::percent_decode_str(&href).decode_utf8()?.into_owned();
        ensure!(!decoded.contains(['#','?']),"Actual relative resource path required");relative(&decoded)?;embeds.insert(decoded);
    }
    let mut rows=BTreeMap::<String,Value>::new();
    for r in opt.resources.as_array().filter(|a|a.len()<=4096).context("Explicit resource list required")?{
        let rel=r["relative_path"].as_str().context("Resource relative path")?;
        ensure!(embeds.contains(rel)&&!rows.contains_key(rel),"Resource must match one actual embed");
        let dest=target(&root,rel,false)?;ensure!(dest.starts_with(&assets),"Resource outside asset root");
        let h=hash(&r["expected_source_sha256"])?;
        let expected=prior(r.get("expected_existing_sha256").context("Explicit prior hash or null")?)?;
        let current=verify_prior(&dest,expected)?;
        let source=if let Some(p)=r.get("source_path").filter(|v|!v.is_null()){
            let p=path(p)?;crate::check_write::no_link(&p)?;let p=files::canonical(&p)?;
            ensure!(!p.starts_with(&root),"New resource input must be outside vault");
            let b=check_ledger::bind_file(&p)?;ensure!(b["sha256"]==h,"Source asset changed");b
        }else{ensure!(current.is_some()&&expected==Some(h),"New asset requires source file");current.clone().unwrap()};
        if current.is_some(){ensure!(expected==Some(h)&&r["reuse_existing"]==true&&r["ownership_confirmed"]==true,"Never overwrite existing asset; reuse needs identical bytes and explicit ownership");}
        else{ensure!(r["reuse_existing"]!=true,"Cannot reuse missing asset");}
        ensure!(fs::metadata(path(&source["path"])?)?.len()>0,"Empty resource");
        rows.insert(rel.into(),json!({"relative_path":rel,"source":source,"expected_sha256":h,"before":current,"action":if expected.is_some(){"reuse"}else{"create"}}));
    }
    ensure!(rows.keys().eq(embeds.iter()),"Every actual embed needs explicit resource disposition");
    if let Some(r)=&full{
        let assets=r["resources"].as_array().context("Full workflow resources")?;ensure!(assets.len()==rows.len(),"Full resource coverage differs");
        for a in assets{let r=rows.get(a["relative_path"].as_str().context("Resource path")?).context("Unplanned asset")?;
            ensure!(a["source"]==r["source"]&&a["existing"]==r["before"],"Full resource binding differs");}
    }
    let tx=tempfile::Builder::new().prefix("z2o-write-").tempdir_in(&backup)?.keep();
    let old=if let Some(b)=&before{
        let(raw,actual)=bounded(&dst,hash(&b["sha256"])?)?;ensure!(actual==*b,"Prior note changed");
        Some(new_bytes(&tx.join("before.md"),&raw)?)
    }else{None};
    let new=new_bytes(&tx.join("candidate.md"),&raw)?;
    let plan=json!({"schema_version":1,"kind":"markdown_file_transaction","vault_root":path_text(&root)?,"backup_root":path_text(&backup)?,
        "scope":opt.scope,"note_relative":opt.note_relative,"literature_relative":opt.literature_relative,"assets_relative":opt.assets_relative,
        "before":before,"before_copy":old,"candidate":cb,"candidate_copy":new,"resources":rows.values().collect::<Vec<_>>(),
        "workflow_inspection":full,"boundary":TX_BOUNDARY});
    let pb=journal(&tx.join("transaction.json"),&plan)?;
    let mut created=vec![];let mut reused=vec![];
    let result=(||->Result<Value>{
        ensure!(check_ledger::bind_file(&cp)?==cb,"Candidate changed");
        for(i,(rel,r))in rows.iter().enumerate(){
            let dest=target(&root,rel,false)?;let h=hash(&r["expected_sha256"])?;
            verify_prior(&dest,r["before"]["sha256"].as_str())?;
            if r["action"]=="reuse"{reused.push(r["before"].clone());}
            else{
                journal(&tx.join(format!("asset-{i:04}-intent.json")),r)?;
                let b=copy_resource(&path(&r["source"]["path"])?,h,&parents(&root,rel)?)?;
                created.push(b.clone());journal(&tx.join(format!("asset-{i:04}-applied.json")),&b)?;
            }
            progress(i+1,rows.len()+1,"publish actual resource");
        }
        for(rel,r)in &rows{verify_prior(&target(&root,rel,false)?,Some(hash(&r["expected_sha256"])?))?;}
        ensure!(check_ledger::bind_file(&cp)?==cb,"Candidate changed before note publication");
        let dest=parents(&root,opt.note_relative)?;verify_prior(&dest,opt.expected_existing_sha)?;
        journal(&tx.join("note-intent.json"),&json!({"target":path_text(&dest)?,"sha256":opt.candidate_sha}))?;
        let after=if let Some(h)=opt.expected_existing_sha{replace_note(&dest,h,&raw)?}else{new_bytes(&dest,&raw)?};
        journal(&tx.join("note-applied.json"),&after)?;
        let readback=properties::audit_write(&cp,&dest)?;
        ensure!(readback["checks"]["WRITE-A04"]["status"]=="passed"&&readback["checks"]["WRITE-A05"]["status"]=="passed","Actual readback audit failed");
        for(rel,r)in &rows{verify_prior(&target(&root,rel,false)?,Some(hash(&r["expected_sha256"])?))?;}
        progress(rows.len()+1,rows.len()+1,"read back actual note and assets");
        Ok(json!({"schema_version":1,"status":"verified_file_write","transaction":pb,"backup_directory":path_text(&tx)?,"note":after,
            "created_resources":created,"reused_resources":reused,"readback":readback,"scope":opt.scope,
            "full_workflow_accepted":false,"source_fidelity":"not_assessed","zotero_note_status":"not_requested","boundary":TX_BOUNDARY}))
    })();
    let(mut receipt,name)=match result{
        Ok(r)=>(r,"write-receipt.json"),
        Err(e)=>{
            let recovery=rollback_locked(&plan,&tx,&root).unwrap_or_else(|e|json!({"status":"recovery_blocked","reason":format!("{e:#}")}));
            (json!({"schema_version":1,"status":"partial_or_failed","transaction":pb,"backup_directory":path_text(&tx)?,"error":format!("{e:#}"),
                "created_resources":created,"recovery":recovery,"full_workflow_accepted":false,"source_fidelity":"not_assessed","boundary":TX_BOUNDARY}),"failed-receipt.json")
        }
    };
    let b=journal(&tx.join(name),&receipt)?;receipt["receipt"]=b;Ok(receipt)
}
fn rollback_locked(plan:&Value,tx:&Path,root:&Path)->Result<Value>{
    let rel=plan["note_relative"].as_str().context("Note relative path")?;
    let lit=target(root,plan["literature_relative"].as_str().context("Literature root")?,true)?;
    let assets=target(root,plan["assets_relative"].as_str().context("Asset root")?,true)?;
    let dst=target(root,rel,false)?;
    ensure!(dst.starts_with(&lit)&&!dst.starts_with(&assets)&&dst.extension().is_some_and(|e|e=="md"),"Invalid rollback scope");
    let before=plan["before"]["sha256"].as_str();let after=hash(&plan["candidate"]["sha256"])?;
    let current=record(&dst)?;let marker=tx.join("note-applied.json");
    let state=if current.as_ref().and_then(|b|b["sha256"].as_str())==before{"already_restored_or_not_written"}
    else if !marker.is_file(){"preserved_unconfirmed_write"}
    else{
        crate::check_write::no_link(&marker)?;ensure!(fs::metadata(&marker)?.len()<=NOTE_LIMIT,"Invalid marker");
        let applied=crate::pipeline::read_json(&marker)?;
        if applied["sha256"]!=after||applied["path"]!=path_text(&dst)?||current.as_ref()!=Some(&applied){"preserved_newer_or_unknown_note"}
        else if let Some(h)=before{
            let(raw,b)=bounded(&tx.join("before.md"),h)?;ensure!(b==plan["before_copy"],"Backup binding changed");
            replace_note(&dst,after,&raw)?;"restored_original"
        }else{
            let(raw,b)=bounded(&dst,after)?;ensure!(b==applied,"Created note changed");
            let saved=tx.join("removed-created-note.md");
            if saved.exists(){verify_prior(&saved,Some(after))?;}else{new_bytes(&saved,&raw)?;}
            verify_prior(&dst,Some(after))?;fs::remove_file(&dst)?;"removed_unchanged_created_note"
        }
    };
    let mut retained=vec![];
    for r in plan["resources"].as_array().context("Original resource list")?{
        let rel=r["relative_path"].as_str().context("Relative path")?;
        let p=target(root,rel,false)?;ensure!(p.starts_with(&assets),"Asset outside original scope");
        if let Some(b)=record(&p)?{retained.push(json!({"relative_path":rel,"file":b,"reason":"May be shared; never delete automatically"}));}
    }
    Ok(json!({"note_disposition":state,"current_note":record(&dst)?,"retained_resources":retained,
        "complete_resource_cleanup":false,"backup_directory":path_text(tx)?,"boundary":TX_BOUNDARY}))
}
pub fn rollback(p:&Path,h:&str,authorized:bool)->Result<Value>{
    ensure!(authorized,"Explicit rollback authorization required");
    let(raw,b)=bounded(p,h)?;let plan=crate::parse_json(std::str::from_utf8(&raw)?)?;
    ensure!(plan["schema_version"].as_u64()==Some(1)&&plan["kind"]=="markdown_file_transaction","Actual transaction required");
    let p=path(&b["path"])?;let tx=actual_root(p.parent().context("Transaction directory")?)?;
    ensure!(p==tx.join("transaction.json"),"Use original transaction plan");
    let root=actual_root(&path(&plan["vault_root"])?)?;
    let backup=actual_root(&path(&plan["backup_root"])?)?;
    ensure!(tx.parent()==Some(backup.as_path()),"Wrong external backup root");
    let _lock=tx_lock(&root,&backup)?;ensure!(check_ledger::bind_file(&p)?==b,"Plan changed");
    let mut r=rollback_locked(&plan,&tx,&root)?;r["transaction"]=b;
    let mut log=tempfile::Builder::new().prefix("rollback-").suffix(".json").tempfile_in(&tx)?;
    let mut raw=serde_json::to_vec_pretty(&r)?;raw.push(b'\n');log.write_all(&raw)?;log.as_file().sync_all()?;
    let(_,p)=log.keep().map_err(|e|anyhow::anyhow!("Cannot retain rollback receipt: {}",e.error))?;
    r["receipt"]=check_ledger::bind_file(&p)?;Ok(r)
}
