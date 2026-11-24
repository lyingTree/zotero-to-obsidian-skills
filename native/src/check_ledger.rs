//! Atomic rules and fresh evidence ledgers. No generated result claims that a
//! source was read, an execution was observed, or a check actually passed.
use crate::{files,path,path_text,pipeline,syntax};
use anyhow::{anyhow,ensure,Context,Result};
use regex::Regex;
use serde_json::{json,Map,Value};
use std::{collections::{HashMap,HashSet,BTreeSet},fs,io::Write,path::{Path,PathBuf},sync::OnceLock};

const TYPES:&[&str]=&["unclassified","execution_process","offline_outcome","live_environment"];
const ENV:&[&str]=&["actual_ui","native_cache","actual_cli"];
const EVIDENCE:&[&str]=&["source_location","offline_result","execution_trace","environment_observation"];
const MAX_RULE_FILES:usize=20_000;
const MAX_RULE_SIZE:u64=16*1024*1024;
fn nonempty(v:&Value)->bool{v.as_str().is_some_and(|s|!s.trim().is_empty())}
fn mapping(v:&Value,label:&str)->Result<Map<String,Value>>{
    if v.is_null(){Ok(Map::new())}else{Ok(v.as_object().with_context(||format!("{label} must be an object"))?.clone())}
}
fn is_symlink_or_reparse(meta:&fs::Metadata)->bool{
    if meta.file_type().is_symlink(){return true;}
    #[cfg(windows)]{
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes()&0x400!=0{return true;}
    }
    false
}
fn list_dir(path:&Path)->Result<Vec<PathBuf>>{
    let mut rows=fs::read_dir(path)?.map(|r|r.map(|e|e.path())).collect::<std::io::Result<Vec<_>>>()?;
    rows.sort();Ok(rows)
}
fn collect_refs(root:&Path,files:&mut Vec<PathBuf>)->Result<()>{
    let mut pending=vec![root.to_owned()];
    while let Some(dir)=pending.pop(){
        for p in list_dir(&dir)?{
            if p.file_name().and_then(|s|s.to_str())==Some("runs"){continue;}
            let m=fs::symlink_metadata(&p)?;
            ensure!(!is_symlink_or_reparse(&m),"Refusing linked rule/reference path: {}",p.display());
            if m.is_dir(){pending.push(p);}
            else if m.is_file()&&p.extension().and_then(|s|s.to_str())==Some("md"){
                files.push(p);ensure!(files.len()<=MAX_RULE_FILES,"Rule file budget exceeded");
            }
        }
    }
    files.sort();Ok(())
}
fn rule_files(root:&Path)->Result<Vec<(String,PathBuf)>>{
    ensure!(root.is_dir(),"skills_root is not a directory");
    let mut directories=Vec::new();
    if root.join("SKILL.md").is_file(){directories.push(root.to_owned());}
    for p in list_dir(root)?{
        if p.file_name().and_then(|s|s.to_str())==Some("runs"){continue;}
        let m=fs::symlink_metadata(&p)?;
        if p.is_dir()&&p.join("SKILL.md").is_file(){
            ensure!(!is_symlink_or_reparse(&m),"Refusing linked skill directory: {}",p.display());
            directories.push(p);
        }
    }
    let mut files=Vec::new();
    for dir in directories{
        let skill=dir.file_name().and_then(|v|v.to_str()).context("non-Unicode skill name")?.to_owned();
        let main=dir.join("SKILL.md");
        ensure!(!is_symlink_or_reparse(&fs::symlink_metadata(&main)?),"Refusing linked SKILL.md");
        files.push((skill.clone(),main));
        let refs=dir.join("references");
        if refs.is_dir(){
            ensure!(!is_symlink_or_reparse(&fs::symlink_metadata(&refs)?),"Refusing linked references directory");
            let mut found=Vec::new();collect_refs(&refs,&mut found)?;
            files.extend(found.into_iter().map(|p|(skill.clone(),p)));
        }
        ensure!(files.len()<=MAX_RULE_FILES,"Rule file budget exceeded");
    }
    ensure!(!files.is_empty(),"No actual SKILL.md files were found under skills_root.");Ok(files)
}
#[derive(Clone)]
pub(crate) struct Rule{pub(crate) id:String,pub(crate) skill:String,pub(crate) objective:String,pub(crate) location:Value,pub(crate) typ:String,pub(crate) environment:Option<String>}
impl Rule{
    pub(crate) fn entry(&self)->Value{json!({"id":self.id,"skill":self.skill,"objective":self.objective,"rule":self.location,
        "declared_type":self.typ,"declared_environment_kind":self.environment})}
}
pub(crate) struct Catalog{pub(crate) rules:Vec<Rule>,pub(crate) bindings:Vec<Value>}
fn atomic_lines(text:&str,skill:&str,p:&Path,digest:&str)->Result<Vec<Rule>>{
    static RULE:OnceLock<Regex>=OnceLock::new();static FENCE:OnceLock<Regex>=OnceLock::new();static TYPE:OnceLock<Regex>=OnceLock::new();
    let rule=RULE.get_or_init(||Regex::new(r"^ {0,3}(?:[-+*]|\d+[.)])\s+(?:\[[ xX]\]\s+)?\[(?P<id>[A-Z][A-Z0-9]*(?:-[A-Z0-9]+)*-[A-Z][0-9]+)\]\s+(?P<text>\S.*)$").unwrap());
    let fence=FENCE.get_or_init(||Regex::new(r"^ {0,3}(\x60{3,}|~{3,})(.*)$").unwrap());
    let annotation=TYPE.get_or_init(||Regex::new(r"<!--\s*check-type:\s*(?P<type>[a-z_]+)(?:\s*;\s*environment:\s*(?P<environment>[a-z_]+))?\s*-->\s*$").unwrap());
    let mut active:Option<(char,usize)>=None;let mut in_comment=false;let mut frontmatter=false;
    let path=path_text(p)?;let mut rows=Vec::new();
    for (i,line) in syntax::lines(text).iter().enumerate(){
        let number=i+1;
        let mut line=syntax::body(text,*line).trim_end_matches(['\r','\n','\u{b}','\u{c}','\u{1c}','\u{1d}','\u{1e}','\u{85}','\u{2028}','\u{2029}']);
        if i==0&&line.trim()=="---"{frontmatter=true;continue;}
        if frontmatter{if matches!(line.trim(),"---"|"..."){frontmatter=false;}continue;}
        if let Some((marker,width))=active{
            let trimmed=line.trim_start_matches(' ');
            let count=trimmed.chars().take_while(|ch|*ch==marker).count();
            if line.len()-trimmed.len()<=3&&count>=width&&trimmed[count..].trim().is_empty(){active=None;}
            continue;
        }
        if !in_comment{
            if let Some(c)=fence.captures(line){active=Some((c[1].chars().next().unwrap(),c[1].len()));continue;}
        }
        let declared=if in_comment{None}else{annotation.captures(line)};
        let typ=declared.as_ref().map(|c|c["type"].to_owned()).unwrap_or_else(||"unclassified".into());
        let environment=declared.as_ref().and_then(|c|c.name("environment")).map(|m|m.as_str().to_owned())
            .or_else(||(typ=="live_environment").then(||"actual_ui".to_owned()));
        if let Some(c)=declared{line=line[..c.get(0).unwrap().start()].trim_end();}
        let mut visible=String::new();let mut cursor=0;
        while cursor<line.len(){
            if in_comment{
                let Some(close)=line[cursor..].find("-->")else{break;};
                cursor+=close+3;in_comment=false;continue;
            }
            if let Some(open)=line[cursor..].find("<!--"){
                visible.push_str(&line[cursor..cursor+open]);cursor+=open+4;in_comment=true;
            }else{visible.push_str(&line[cursor..]);break;}
        }
        let Some(c)=rule.captures(visible.trim_end())else{continue;};
        ensure!(TYPES.contains(&typ.as_str()),"Invalid check-type at line {number}: {typ}");
        if let Some(env)=environment.as_ref(){
            ensure!(typ=="live_environment"&&ENV.contains(&env.as_str()),"Invalid environment kind at line {number}: {env}");
        }
        rows.push(Rule{id:c["id"].to_owned(),skill:skill.to_owned(),objective:c["text"].to_owned(),
            location:json!({"path":path,"line":number,"sha256":digest}),typ,environment});
    }Ok(rows)
}
pub(crate) fn catalog(root:&Path,mut progress:impl FnMut(usize,usize,&str))->Result<Catalog>{
    let root=files::canonical(root)?;
    let paths=rule_files(&root)?;
    let mut seen=HashMap::<String,Value>::new();let mut rules=Vec::new();let mut bindings=Vec::new();
    for(i,(skill,p))in paths.iter().enumerate(){
        let p=files::canonical(p)?;
        ensure!(p.starts_with(&root),"Rule file escapes skills root");
        ensure!(fs::metadata(&p)?.len()<=MAX_RULE_SIZE,"Rule file exceeds size budget");
        let(data,binding)=files::read_bound(&p)?;
        for rule in atomic_lines(std::str::from_utf8(&data)?,skill,&p,binding["sha256"].as_str().unwrap())?{
            if let Some(old)=seen.insert(rule.id.clone(),rule.location.clone()){anyhow::bail!("Duplicate atomic ID {}: {} and {}",rule.id,old,rule.location);}
            rules.push(rule);
        }
        bindings.push(binding);progress(i+1,paths.len(),"rule_files");
    }
    ensure!(!rules.is_empty(),"The skill files contain no atomic rule declarations.");
    Ok(Catalog{rules,bindings})
}
pub fn bind_file(p:&Path)->Result<Value>{
    let b=files::snapshot(p)?;Ok(json!({"path":b["path"],"sha256":b["sha256"]}))
}
pub fn bind_evidence(p:&Path,locator:&Value,typ:&str,observation:&Value)->Result<Value>{
    ensure!(EVIDENCE.contains(&typ),"Unknown evidence_type: {typ}");
    let mut result=bind_file(p)?;result["locator"]=locator.clone();result["evidence_type"]=json!(typ);
    if !observation.is_null(){result["observation"]=observation.clone();}
    Ok(result)
}
fn mapped_helper(id:&str)->Option<&'static str>{
    match id{
        "PROP-A01"|"PROP-A02"|"PROP-A04"|"PROP-A05"|"PROP-A06"|"PROP-A08"|"PROP-C10"|"PROP-C18"=>Some("audit_properties"),
        "PROP-C01"|"PROP-C02"|"PROP-C03"|"PROP-C05"|"PROP-C06"|"PROP-C07"|"PROP-C08"|"PROP-C09"|"PROP-C13"|"PROP-C14"|"PROP-C17"=>Some("build_properties/repair_properties"),
        "NAME-C04"|"NAME-C07"|"NAME-C08"|"NAME-C09"=>Some("author_token"),
        "ZOT-C05"|"ZOT-C06"|"ZOT-C07"|"ZOT-C08"|"ZOT-C09"|"ZOT-C12"|"ZOT-C13"|"ZOT-C14"=>Some("collection_ancestry"),
        "ZOT-C20"|"ZOT-C21"=>Some("snapshot"),
        "HDR-A14"|"LIB-A08"=>Some("audit_unchanged_properties"),
        "WRITE-A04"|"WRITE-A05"=>Some("audit_write_readback"),
        "TRN-A17"|"TRN-A25"=>Some("audit_translation_binding"),
        "BODY-A83"|"BODY-A84"|"BODY-A85"|"BODY-A86"|"BODY-A87"|"BODY-A111"=>Some("audit_rich_tables"),
        _=>None
    }
}
pub fn coverage(root:&Path,progress:impl FnMut(usize,usize,&str))->Result<Value>{
    let catalog=catalog(root,progress)?;
    let known:HashSet<_>=catalog.rules.iter().map(|r|r.id.as_str()).collect();
    let declared=["PROP-A01","PROP-A02","PROP-A04","PROP-A05","PROP-A06","PROP-A08","PROP-C10","PROP-C18",
        "PROP-C01","PROP-C02","PROP-C03","PROP-C05","PROP-C06","PROP-C07","PROP-C08","PROP-C09","PROP-C13","PROP-C14","PROP-C17",
        "NAME-C04","NAME-C07","NAME-C08","NAME-C09","ZOT-C05","ZOT-C06","ZOT-C07","ZOT-C08","ZOT-C09","ZOT-C12","ZOT-C13","ZOT-C14",
        "ZOT-C20","ZOT-C21","HDR-A14","LIB-A08","WRITE-A04","WRITE-A05","TRN-A17","TRN-A25",
        "BODY-A83","BODY-A84","BODY-A85","BODY-A86","BODY-A87","BODY-A111"];
    let unknown:BTreeSet<_>=declared.iter().copied().filter(|s|!known.contains(s)).collect();
    let mut counts=HashMap::<String,usize>::new();let mut rows=Vec::new();
    for rule in &catalog.rules{
        let mut row=rule.entry();let h=mapped_helper(&rule.id);
        let classification=if h.is_some(){"mapped"}else{"not_classified_by_this_module"};
        *counts.entry(classification.into()).or_default()+=1;
        row["new_stage_helper"]=json!(h);row["classification"]=json!(classification);rows.push(row);
    }
    Ok(json!({"rule_files":catalog.bindings,"rules":rows,"counts":counts,"unknown_mapping_ids":unknown}))
}
fn output_path(p:&Path)->Result<PathBuf>{
    let parent=p.parent().filter(|v|!v.as_os_str().is_empty()).unwrap_or_else(||Path::new("."));
    let name=p.file_name().context("output file name required")?;
    fs::create_dir_all(parent)?;
    Ok(files::canonical(parent)?.join(name))
}
fn publish_new(p:&Path,data:&[u8])->Result<()>{
    let mut f=tempfile::Builder::new().prefix(".pending-checks-").tempfile_in(p.parent().context("ledger parent")?)?;
    f.write_all(data)?;f.as_file().sync_all()?;
    f.persist_noclobber(p).map_err(|e|anyhow!("Cannot exclusively publish {}: {}",p.display(),e.error))?;
    ensure!(fs::read(p)?==data,"Ledger readback differs");Ok(())
}
pub(crate) fn selected(catalog:&Catalog,mode:&str,extra:&Value)->Result<HashSet<String>>{
    ensure!(matches!(mode,"acceptance"|"all"),"rule_selection must be acceptance or all.");
    let values=extra.as_array().context("additional_ids must be an explicit sequence of individual IDs.")?;
    let mut ids=HashSet::new();
    let known:HashSet<_>=catalog.rules.iter().map(|r|r.id.as_str()).collect();
    for id in values{
        let id=id.as_str().filter(|v|!v.trim().is_empty()).context("additional_ids must contain nonempty ID strings.")?;
        ensure!(known.contains(id),"Unknown additional rule ID: {id}");
        ensure!(ids.insert(id.to_owned()),"additional_ids must not repeat an ID.");
    }
    static ACCEPTANCE:OnceLock<Regex>=OnceLock::new();
    let re=ACCEPTANCE.get_or_init(||Regex::new(r"-A[0-9]+$").unwrap());
    for rule in &catalog.rules{if mode=="all"||re.is_match(&rule.id){ids.insert(rule.id.clone());}}
    Ok(ids)
}
pub struct PrepareOptions{
    pub sources:Value,pub candidates:Value,pub candidate_path:Option<PathBuf>,pub scopes:Value,
    pub types:Value,pub environments:Value,pub mode:String,pub additional:Value,
}
fn build_payload(root:&Path,out:&Path,opt:PrepareOptions,progress:impl FnMut(usize,usize,&str))->Result<Value>{
    let root=files::canonical(root)?;
    let source=opt.sources.as_object().filter(|m|!m.is_empty()).context("At least one actual source file must be bound.")?;
    ensure!(opt.candidates.is_null()||opt.candidate_path.is_none(),"Use candidates or candidate_path, not both.");
    let candidates=if let Some(p)=opt.candidate_path{Map::from_iter([("candidate".into(),json!(path_text(&p)?))])}
        else{mapping(&opt.candidates,"candidates")?};
    let scopes=mapping(&opt.scopes,"scopes")?;let types=mapping(&opt.types,"requirement_types")?;
    let environments=mapping(&opt.environments,"environment_kinds")?;
    let catalog=catalog(&root,progress)?;
    let selected=selected(&catalog,&opt.mode,&opt.additional)?;
    let known:HashSet<_>=catalog.rules.iter().map(|r|r.id.as_str()).collect();
    for id in scopes.keys().chain(types.keys()).chain(environments.keys()){ensure!(known.contains(id.as_str()),"Unknown rule ID in explicit mappings: {id}");}
    let mut protected:HashSet<String>=catalog.bindings.iter().map(|b|b["path"].as_str().unwrap().to_owned()).collect();
    let mut bindings=Map::new();
    for(label,values)in [("sources",source),("candidates",&candidates)]{
        let mut entries=Vec::new();
        for (id,v) in values{
            ensure!(!id.trim().is_empty(),"Empty {label} binding identity");
            let mut binding=bind_file(&path(v)?)?;
            protected.insert(binding["path"].as_str().unwrap().to_owned());
            binding["id"]=json!(id);entries.push(binding);
        }bindings.insert(label.into(),json!(entries));
    }
    ensure!(!protected.contains(&path_text(&out)?),"The ledger output would overwrite a bound input file.");
    let mut checks=Vec::with_capacity(selected.len());let mut entries=Vec::with_capacity(catalog.rules.len());
    for r in &catalog.rules{
        entries.push(r.entry());if !selected.contains(&r.id){continue;}
        let typ=match types.get(&r.id){Some(v)=>v.as_str().context("requirement type must be a string")?,None=>r.typ.as_str()};
        ensure!(TYPES.contains(&typ),"Unknown requirement type for {}: {typ}",r.id);
        ensure!(r.typ=="unclassified"||typ==r.typ,"Explicit type conflicts with the source declaration for {}",r.id);
        let environment=environments.get(&r.id).cloned().unwrap_or_else(||json!(r.environment.as_deref().or_else(||(typ=="live_environment").then_some("actual_ui"))));
        if !environment.is_null(){ensure!(typ=="live_environment"&&environment.as_str().is_some_and(|v|ENV.contains(&v)),"Invalid environment kind for {}",r.id);}
        if let Some(declared)=r.environment.as_deref(){ensure!(environment==declared,"Explicit environment kind conflicts with the source declaration for {}",r.id);}
        checks.push(json!({"id":r.id,"skill":r.skill,"objective":r.objective,"rule":r.location,"requirement_type":typ,
            "environment_kind":environment,"scope":scopes.get(&r.id).cloned().unwrap_or(json!([])),"status":"not_verified","evidence":[],
            "reason":"尚未逐项核验；此骨架不构成执行或内容通过证据。","applicability_condition":""}));
    }
    let payload=json!({"schema_version":1,"created_at":chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros,false),
        "skills_root":path_text(&root)?,"rule_files":catalog.bindings,"catalog":entries,
        "rule_selection":{"mode":opt.mode,"additional_ids":opt.additional},"bindings":bindings,"evidence":{},"checks":checks});
    for item in payload["rule_files"].as_array().unwrap().iter()
        .chain(payload["bindings"]["sources"].as_array().unwrap())
        .chain(payload["bindings"]["candidates"].as_array().unwrap()){
        let actual=bind_file(Path::new(item["path"].as_str().unwrap()))?;
        ensure!(actual["sha256"]==item["sha256"],"Ledger input changed before publication");
    }
    Ok(payload)
}

pub fn prepare(root:&Path,out:&Path,opt:PrepareOptions,progress:impl FnMut(usize,usize,&str))->Result<PathBuf>{
    prepare_with_inventory(root,out,opt,None,false,progress)
}

fn bindings(payload:&Value)->impl Iterator<Item=&Value>{
    payload["rule_files"].as_array().into_iter().flatten()
        .chain(payload["bindings"]["sources"].as_array().into_iter().flatten())
        .chain(payload["bindings"]["candidates"].as_array().into_iter().flatten())
}
fn json_bytes(payload:&Value)->Result<Vec<u8>>{
    let mut bytes=serde_json::to_vec_pretty(payload)?;bytes.push(b'\n');Ok(bytes)
}
pub fn prepare_with_inventory(root:&Path,out:&Path,opt:PrepareOptions,inventory:Option<&Path>,
    overwrite:bool,progress:impl FnMut(usize,usize,&str))->Result<PathBuf>{
    let out=output_path(out)?;
    let inventory=inventory.map(|p|{
        crate::check_write::no_link(p)?;
        files::canonical(p)
    }).transpose()?;
    ensure!(inventory.as_ref()!=Some(&out),"The ledger output would overwrite the mutable inventory.");
    let mut lock_paths=vec![out.clone()];
    if let Some(p)=&inventory{lock_paths.push(p.clone());}
    lock_paths.sort();
    let mut locks=Vec::new();
    for p in &lock_paths{locks.push(crate::check_write::lock_target(p)?);}
    let old=match fs::symlink_metadata(&out){
        Ok(_)=>{
            crate::check_write::no_link(&out)?;
            ensure!(overwrite,"Refusing to replace an existing ledger: {}",out.display());
            Some(fs::read(&out)?)
        }
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>None,
        Err(e)=>return Err(e.into())
    };
    let payload=build_payload(root,&out,opt,progress)?;
    let inventory_update=if let Some(p)=inventory.as_ref(){
        let text=path_text(p)?;
        ensure!(!bindings(&payload).any(|item|item["path"]==text),
            "Do not hash-bind the mutable inventory as a source, candidate or rule file.");
        let bytes=fs::read(p)?;
        let mut doc=crate::parse_json(std::str::from_utf8(&bytes)?)?;
        ensure!(doc.is_object(),"inventory_path must contain a JSON object.");
        doc["checks_path"]=json!(path_text(&out)?);
        Some((p.clone(),bytes,json_bytes(&doc)?))
    }else{None};
    let bytes=json_bytes(&payload)?;
    for item in bindings(&payload){
        let actual=bind_file(Path::new(item["path"].as_str().unwrap()))?;
        ensure!(actual["sha256"]==item["sha256"],"Ledger input changed before publication");
    }
    // A fresh single-file ledger needs no multi-file recovery receipt.
    if old.is_none()&&inventory_update.is_none(){
        publish_new(&out,&bytes)?;
        ensure!(pipeline::read_json(&out)?==payload,"Ledger did not round-trip");
        return Ok(out);
    }
    // Filesystems do not provide a cross-file atomic rename. Preserve the
    // actual preimages and intended new bytes in an independent recovery
    // directory before changing either file. A crash leaves the plan intact.
    // Successful explicit overwrite also retains the prior ledger.
    let recovery=tempfile::Builder::new().prefix(".z2o-check-transaction-").tempdir_in(out.parent().unwrap())?.keep();
    if let Some(old)=old.as_ref(){publish_new(&recovery.join("ledger.before"),old)?;}
    publish_new(&recovery.join("ledger.after"),&bytes)?;
    if let Some((_,old,new))=inventory_update.as_ref(){
        publish_new(&recovery.join("inventory.before"),old)?;
        publish_new(&recovery.join("inventory.after"),new)?;
    }
    let plan=json!({"schema_version":1,"ledger":path_text(&out)?,
        "ledger_before_sha256":old.as_ref().map(|v|files::digest(v)),
        "ledger_after_sha256":files::digest(&bytes),
        "inventory":inventory_update.as_ref().map(|(p,old,new)|json!({"path":path_text(p).unwrap_or_default(),
            "before_sha256":files::digest(old),"after_sha256":files::digest(new)})),
        "boundary":"Individual files are atomically published. If interrupted, inspect current hashes against the retained preimages before recovery."});
    publish_new(&recovery.join("plan.json"),&json_bytes(&plan)?)?;
    let changed=match old.as_ref(){
        Some(old)=>crate::check_write::replace_bytes(&out,&files::digest(old),&bytes),
        None=>publish_new(&out,&bytes),
    };
    if let Err(e)=changed{
        return Err(e.context(format!("Ledger publication failed; recovery data: {}",recovery.display())));
    }
    if let Some((p,old_inventory,new_inventory))=inventory_update.as_ref(){
        if let Err(error)=crate::check_write::replace_bytes(p,&files::digest(old_inventory),new_inventory){
            // Never overwrite an unrelated external edit. Keep both versions
            // and roll back only the ledger we just successfully published.
            let rollback=if fs::read(&out).ok().as_deref()==Some(bytes.as_slice()){
                if let Some(old)=old.as_ref(){crate::check_write::replace_bytes(&out,&files::digest(&bytes),old)}
                else{fs::remove_file(&out).map_err(anyhow::Error::from)}
            }else{Err(anyhow!("Ledger changed after publication; manual recovery required"))};
            let status=json!({"status":"inventory_write_failed","ledger_rollback_succeeded":rollback.is_ok()});
            let _=publish_new(&recovery.join("failure.json"),&json_bytes(&status)?);
            return Err(error.context(format!("Inventory update failed; rollback={}; recovery data: {}",rollback.is_ok(),recovery.display())));
        }
    }
    publish_new(&recovery.join("completed.json"),&json_bytes(&json!({"status":"completed"}))?)?;
    ensure!(pipeline::read_json(&out)?==payload,"Ledger did not round-trip");
    Ok(out)
}
pub fn pending_batch(p:&Path,limit:usize,prefix:Option<&str>)->Result<Value>{
    ensure!(limit>0,"limit must be a positive integer.");
    let ledger=pipeline::read_json(p)?;
    let rows=ledger["checks"].as_array().context("The ledger must contain checks.")?;
    ensure!(rows.iter().all(|v|v.is_object()&&v["id"].is_string()),"Every check needs its actual string rule ID.");
    fn group(id:&str)->&str{id.split('-').next().unwrap_or("")}
    if let Some(p)=prefix{ensure!(rows.iter().any(|r|group(r["id"].as_str().unwrap())==p),"prefix must name an actual rule group in this ledger.");}
    let mut pending=Vec::new();
    for row in rows{
        if row["requirement_type"]!="offline_outcome"||prefix.is_some_and(|p|group(row["id"].as_str().unwrap())!=p)||
            matches!(row["status"].as_str(),Some("passed"|"not_applicable")){continue;}
        let fields=["id","objective","rule","scope","status","reason"];
        pending.push(Value::Object(fields.iter().map(|k|((*k).into(),row[*k].clone())).collect()));
        if pending.len()==limit{break;}
    }
    Ok(json!(pending))
}
pub fn pending_rows(p:&Path,limit:usize,prefix:Option<&str>)->Result<Value>{
    let rows=pending_batch(p,limit,prefix)?;
    Ok(json!(rows.as_array().unwrap().iter().map(|r|json!([r["id"],r["objective"]])).collect::<Vec<_>>()))
}
