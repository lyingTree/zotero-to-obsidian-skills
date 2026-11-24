//! Native initialization and draft-phase resumption of the file-only workflow.
//! PDF preparation and final review inspection remain explicit hard boundaries.
use anyhow::{ensure,Context,Result};
use serde_json::{json,Map,Value};
use std::{collections::{BTreeMap,BTreeSet},fs,path::{Path,PathBuf}};
use crate::{check_ledger,check_write,draft_queue,files,glossary,path,path_text,pipeline,source_prepare};
const PROFILE:&str="complete_pdf_candidate_v1";

fn actual_path(v:&Value,base:&Path,label:&str)->Result<PathBuf>{
    let s=v.as_str().filter(|s|!s.trim().is_empty()).with_context(||format!("Record the actual {label} path."))?;
    Ok(base.join(s))
}

// Canonicalize existing ancestors without creating the future destination.
fn resolve_future(p:&Path)->Result<PathBuf>{
    let p=std::path::absolute(p)?;
    if p.exists(){return files::canonical(&p);}
    if p.is_symlink(){anyhow::bail!("Refusing a dangling output symlink: {}",p.display());}
    let parent=p.parent().context("output parent required")?;
    let name=p.file_name().context("Choose a non-traversing output path")?;
    let parent=resolve_future(parent)?;
    if parent.exists(){ensure!(parent.is_dir(),"Output parent must be a directory");}
    Ok(parent.join(name))
}

fn new_path(p:&Path)->Result<()>{
    match fs::symlink_metadata(p){
        Ok(_)=>anyhow::bail!("Keep existing evidence: {}",p.display()),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ok(()),
        Err(e)=>Err(e.into())
    }
}

fn skill_input_paths(root:&Path)->Result<Vec<PathBuf>>{
    let mut stack=Vec::new();
    for entry in fs::read_dir(root)?{
        let entry=entry?;let p=entry.path();
        if entry.file_type()?.is_dir()&&(entry.file_name()=="template"||p.join("SKILL.md").is_file()){
            stack.push(p);
        }
    }
    let mut paths=Vec::new();
    while let Some(dir)=stack.pop(){
        for entry in fs::read_dir(&dir)?{
            let entry=entry?;let name=entry.file_name();let name=name.to_string_lossy();
            if name.starts_with('.')||name=="__pycache__"{continue;}
            let p=entry.path();let ty=entry.file_type()?;
            if ty.is_dir(){stack.push(p);continue;}
            if !p.is_file(){continue;}
            if p.extension().and_then(|e|e.to_str()).is_some_and(|s|matches!(s,"md"|"py"|"txt"|"json"|"yaml"|"yml")){
                paths.push(files::canonical(&p)?);
            }
        }
    }
    paths.sort();Ok(paths)
}
fn bindings(root:&Path)->Result<Vec<Value>>{
    skill_input_paths(root)?.iter().map(|p|check_ledger::bind_file(p)).collect()
}
fn now()->String{chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros,false)}

pub struct Initialize<'a>{
    pub skills_root:&'a Path,pub work_root:&'a Path,pub sources:&'a Value,
    pub source_roles:&'a Value,pub candidate_path:&'a Path,pub resource_root:&'a Path,
    pub glossary_path:Option<&'a Path>,pub translation_required:bool,pub run_id:&'a str,
}

pub fn initialize(opt:Initialize<'_>,progress:impl FnMut(usize,usize,&str))->Result<PathBuf>{
    initialize_with_ocr(opt,None,progress)
}
pub fn initialize_with_ocr(opt:Initialize<'_>,ocr:Option<&crate::pdf_inventory::OcrSettings>,mut progress:impl FnMut(usize,usize,&str))->Result<PathBuf>{
    let skills=files::canonical(opt.skills_root)?;
    ensure!(skills.is_dir(),"skills_root must be a directory");
    let work=resolve_future(opt.work_root)?;
    let candidate=resolve_future(opt.candidate_path)?;
    let resource=resolve_future(opt.resource_root)?;
    let run=work.join("run.json");
    let source=opt.sources.as_object().filter(|m|!m.is_empty()).context("Provide one explicit role for every source PDF or DOCX.")?;
    let roles=opt.source_roles.as_object().context("source_roles must be an object")?;
    ensure!(source.keys().eq(roles.keys()),"Provide one explicit role for every source PDF or DOCX.");
    ensure!(roles.values().all(|v|matches!(v.as_str(),Some("main"|"supplement"))),"Source roles must be main or supplement.");
    ensure!(!opt.run_id.trim().is_empty(),"run_id must be nonempty.");
    let mut resolved=Map::new();
    let mut original_sources=BTreeMap::new();
    for (id,v) in source{
        let p=files::canonical(&path(v)?)?;
        ensure!(p.extension().and_then(|s|s.to_str()).is_some_and(|s|s.eq_ignore_ascii_case("docx")||s.eq_ignore_ascii_case("pdf")),
            "Unsupported source format: {id}; no Python fallback");
        let b=check_ledger::bind_file(&p)?;
        resolved.insert(id.clone(),json!(path_text(&p)?));
        original_sources.insert(id.clone(),b);
    }
    let glossary_path=opt.glossary_path.map(files::canonical).transpose()?;
    let glossary_before=glossary_path.as_deref().map(check_ledger::bind_file).transpose()?;
    let inventory=work.join("evidence").join("source-inventory.json");
    let checks=work.join("checks.json");
    let queue=work.join("draft-queue.json");
    let snapshot=work.join("draft-queue-source-snapshot.json");
    let source_map=work.join("source-map.json");
    let link_map=work.join("link-map.json");
    let si_map=work.join("si-map.json");
    let delivery=work.join("delivery-manifest.json");
    let translation=work.join("translation-review.json");
    let terminology=work.join("evidence").join("terminology.json");
    let leads=work.join("glossary-candidates.json");
    let mut planned=vec![&run,&inventory,&checks,&queue,&snapshot,&source_map,&link_map,&delivery];
    if roles.values().any(|v|v=="supplement"){planned.push(&si_map);}
    if opt.translation_required{planned.push(&translation);}
    if glossary_path.is_some(){planned.extend([&terminology,&leads]);}
    for p in &planned{
        new_path(p)?;
        ensure!(**p!=candidate,"A workflow output must not replace the candidate.");
    }
    for b in original_sources.values().chain(glossary_before.iter()){
        let p=path(&b["path"])?;
        ensure!(p!=candidate&&!planned.contains(&&p),"A workflow output must not replace a source attachment or glossary.");
    }
    let baseline_created_at=now();
    let baseline=bindings(&skills)?;
    ensure!(!baseline.is_empty(),"The skills baseline is empty");
    let protected: BTreeSet<_>=baseline.iter().filter_map(|b|b["path"].as_str()).collect();
    for p in planned.iter().copied().chain(std::iter::once(&candidate)){
        ensure!(!protected.contains(path_text(p)?.as_str()),"Workflow output overlaps a frozen skill input");
    }

    // Never manufacture ready records: each existing native engine creates
    // actual evidence, an all-pending ledger, and an immutable drafting queue.
    let resolved=Value::Object(resolved);
    source_prepare::prepare_sources(&resolved,inventory.parent().unwrap(),opt.source_roles,
        glossary_path.as_deref(),144,ocr,|n,total,label|progress(n,total,label))?;
    let prepare=check_ledger::PrepareOptions{sources:resolved.clone(),candidates:Value::Null,
        candidate_path:None,scopes:Value::Null,types:Value::Null,environments:Value::Null,
        mode:"acceptance".into(),additional:json!([])};
    check_ledger::prepare_with_inventory(&skills,&checks,prepare,Some(&inventory),false,
        |n,total,label|progress(n,total,label))?;
    draft_queue::prepare(&inventory,&queue,|n,total,label|progress(n,total,label))?;
    let mut records=json!({"source_inventory":path_text(&inventory)?,"checks":path_text(&checks)?,
        "draft_queue":path_text(&queue)?,"source_map":path_text(&source_map)?,
        "link_map":path_text(&link_map)?,"delivery_manifest":path_text(&delivery)?});
    if roles.values().any(|v|v=="supplement"){records["si_map"]=json!(path_text(&si_map)?);}
    if opt.translation_required{records["translation_review"]=json!(path_text(&translation)?);}
    if let Some(p)=glossary_path.as_deref(){
        records["terminology"]=json!(path_text(&terminology)?);
        glossary::prepare(&inventory,p,&leads,|n,total|progress(n,total,"source glossary search leads"))?;
        records["glossary_candidates"]=json!(path_text(&leads)?);
        ensure!(Some(check_ledger::bind_file(p)?)==glossary_before,"Glossary changed during workflow initialization");
    }
    let ledger=pipeline::read_json(&checks)?;
    let actual=ledger["bindings"]["sources"].as_array().context("source ledger bindings")?;
    ensure!(actual.len()==original_sources.len(),"Source binding count changed");
    for b in actual{
        let id=b["id"].as_str().context("source ID")?;
        let expected=original_sources.get(id).context("Unknown workflow source")?;
        ensure!(json!({"path":b["path"],"sha256":b["sha256"]})==*expected&&check_ledger::bind_file(&path(&b["path"])?)?==*expected,
            "Source changed during workflow initialization");
    }
    ensure!(bindings(&skills)?==baseline,"A frozen skill input changed during workflow initialization");
    let run_data=json!({
        "schema_version":1,"run_id":opt.run_id,"workflow_profile":PROFILE,"created_at":now(),
        "scope":{"source_ids":resolved.as_object().unwrap().keys().collect::<Vec<_>>(),
            "source_roles":opt.source_roles,"write_vault":false,"translation_required":opt.translation_required,
            "acceptance":"markdown_and_referenced_resources"},
        "identity":{"sources":actual,"zotero_library":null,"zotero_item":null,
            "library_reason":"This file-only task does not establish a live library identity."},
        "config":{"skills_root":path_text(&skills)?,"work_root":path_text(&work)?,
            "resource_root":path_text(&resource)?,"candidate_path":path_text(&candidate)?,
            "glossary_path":glossary_path.as_deref().map(path_text).transpose()?},
        "versions":{"rule_files":ledger["rule_files"],"files":baseline,"baseline_created_at":baseline_created_at},
        "records":records,
        "naming":{"status":"pending","original_title":null,"author_token":null,"year":null,
            "filename":null,"transformations":[],"conflicts":[]},
        "placement":{"candidate_path":path_text(&candidate)?,"resource_root":path_text(&resource)?,
            "vault_path":null,"library_mapping_status":"unavailable"},
        "content_input":null,"stages":{},"issues":[],
        "publish":{"status":"prepared","zotero_note_status":"not_requested"}
    });
    let mut bytes=serde_json::to_vec_pretty(&run_data)?;bytes.push(b'\n');
    new_path(&run)?;
    // A fresh run is published last; retain partial evidence if any earlier
    // native producer failed, rather than silently reinitializing over it.
    let mut output=tempfile::NamedTempFile::new_in(&work)?;
    use std::io::Write;
    output.write_all(&bytes)?;output.as_file().sync_all()?;
    output.persist_noclobber(&run).map_err(|e|anyhow::anyhow!("Cannot exclusively publish run: {}",e.error))?;
    ensure!(pipeline::read_json(&run)?==run_data,"Workflow record did not round-trip");
    Ok(run)
}

pub fn resume(run_path:&Path)->Result<Value>{
    let (raw,bound)=files::read_bound(run_path)?;
    let run_path=path(&bound["path"])?;
    let run=crate::parse_json(std::str::from_utf8(&raw)?)?;
    ensure!(run["workflow_profile"]==PROFILE,"Use the actual complete-workflow run record.");
    let (config,records,scope)=(&run["config"],&run["records"],&run["scope"]);
    ensure!(config.is_object()&&records.is_object()&&scope.is_object(),"config, records and scope must be objects.");
    let base=run_path.parent().context("run directory")?;
    let mut result=json!({"run":bound,"source_fidelity":"not_assessed",
        "boundary":"One local action, not a semantic pass or a host termination gate."});
    if records.get("draft_queue").is_none(){
        result["phase"]=json!("setup");
        result["next_action"]=json!({"stage":"register_draft_queue",
            "inventory_path":path_text(&actual_path(&records["source_inventory"],base,"source inventory")?)?,
            "message":"Register the existing source-bound queue in run.records.draft_queue; if no queue exists, prepare it from this inventory without replacing old evidence."});
        return Ok(result);
    }
    let queue_path=actual_path(&records["draft_queue"],base,"draft queue")?;
    let snapshot=draft_queue::resume_snapshot(&queue_path)?;
    let data=&snapshot["data"];
    let original=run["identity"]["sources"].as_array().filter(|v|!v.is_empty())
        .context("Retain the workflow's original source bindings.")?;
    let mut expected=BTreeMap::new();
    for b in original{
        let id=b["id"].as_str().filter(|s|!s.trim().is_empty()).context("Retain the workflow's original source bindings.")?;
        ensure!(expected.insert(id,b).is_none(),"The draft queue must retain every original workflow source exactly once.");
    }
    let sources=data["sources"].as_object().context("queue sources")?;
    ensure!(expected.len()==sources.len()&&expected.keys().all(|s|sources.contains_key(*s)),
        "The draft queue must retain every original workflow source exactly once.");
    let roles=scope["source_roles"].as_object().context("Retain the original source scope and roles before resuming.")?;
    let declared=scope["source_ids"].as_array().context("Retain the original source scope and roles before resuming.")?;
    let declared_set: BTreeSet<_>=declared.iter().map(|v|v.as_str().filter(|s|!s.trim().is_empty())
        .context("Retain the original source scope and roles before resuming.")).collect::<Result<_>>()?;
    ensure!(declared_set.len()==declared.len()&&declared_set.len()==expected.len()&&roles.len()==expected.len()
        &&expected.keys().all(|id|declared_set.contains(id)&&roles.contains_key(*id)),
        "Retain the original source scope and roles before resuming.");
    for (id,b) in expected{
        let source=&sources[id];
        let p=files::canonical(&actual_path(&b["path"],base,"source")?)?;
        ensure!(files::canonical(&path(&source["path"])?)?==p&&source["sha256"]==b["sha256"]&&source["role"]==roles[id],
            "The draft queue differs from the workflow source binding: {id}");
    }
    let inventory=files::canonical(&actual_path(&records["source_inventory"],base,"source inventory")?)?;
    ensure!(files::canonical(&path(&data["inventory_origin"])?)?==inventory,
        "The draft queue must originate from this workflow's source inventory.");
    let status=&snapshot["status"];
    let mut counts=Map::new();
    for name in ["source_entry_count","unit_count","written_unit_count","excluded_entry_count"]{
        counts.insert(name.into(),status[name].clone());
    }
    for name in ["unassigned_entries","unwritten_units","unreviewed_pdf_pages","unresolved_source_issues"]{
        counts.insert(format!("{name}_count"),json!(status[name].as_array().context("queue status count")?.len()));
    }
    result["queue"]=snapshot["binding"].clone();
    result["draft_ready"]=status["draft_ready"].clone();
    result["counts"]=Value::Object(counts);
    ensure!(check_ledger::bind_file(&run_path)?==bound,"Workflow record changed during resumption");
    ensure!(check_ledger::bind_file(&queue_path)?==snapshot["binding"],"Draft queue changed during resumption");
    if status["draft_ready"]!=true{
        result["phase"]=json!("draft");result["next_action"]=snapshot["next_task"].clone();return Ok(result);
    }
    let candidate=actual_path(&config["candidate_path"],base,"candidate")?;
    if !candidate.is_file(){
        result["phase"]=json!("assemble");
        result["next_action"]=json!({"stage":"assemble","queue_path":path_text(&queue_path)?,
            "candidate_path":path_text(&candidate)?,"message":"Assemble the saved fragments to a new version with DraftQueue.assemble(), then format the complete candidate and continue its source checks."});
        return Ok(result);
    }
    let report=inspect(&run_path,|_,_,_|{})?;
    ensure!(check_ledger::bind_file(&run_path)?==bound&&check_ledger::bind_file(&queue_path)?==snapshot["binding"],"Workflow or queue changed during review inspection");
    let findings=report["findings"].as_array().context("Workflow inspection findings")?;
    result["phase"]=json!("review");result["records_ready"]=report["records_ready"].clone();result["record_finding_count"]=json!(findings.len());
    if report["records_ready"]==true{
        result["next_action"]=json!({"stage":"records_complete","message":"File-record checks are complete; report only the source comparisons actually performed. Source fidelity remains unassessed."});
        return Ok(result);
    }
    let first=findings.first().context("Unresolved workflow must contain findings")?;
    let mut selected=first;
    if first["code"]=="WORKFLOW_RECORD_MISSING"{
        for name in ["source_inventory","source_map","si_map","link_map","translation_review","terminology","checks","delivery_manifest"]{
            if let Some(f)=findings.iter().find(|f|f["code"]=="WORKFLOW_RECORD_MISSING"&&f["target"]==name){selected=f;break;}
        }
    }
    let mut action=selected.clone();action["stage"]=json!("repair_record");
    if let Some(name)=selected["target"].as_str(){if let Some(v)=records.get(name){action["record_path"]=json!(path_text(&actual_path(v,base,name)?)?);}}
    result["next_action"]=action;Ok(result)
}


// Review is read-only. Keep original source/skill scope, all required record
// checks and explicit final delivery blockers. Never infer readiness from an
// empty subset or route around an unfinished source comparison.
use crate::handoff_native::Bindings;
struct Review{
    findings:Vec<Value>,checked:Map<String,Value>,files:Bindings,pending:Value,stages:Map<String,Value>,
}
impl Review{
    fn new()->Self{Self{findings:vec![],checked:Map::new(),files:Bindings::new(),pending:json!([]),stages:Map::new()}}
    fn fail(&mut self,code:&str,target:&str,msg:impl AsRef<str>){self.findings.push(json!({"code":code,"target":target,"message":msg.as_ref()}));}
    fn issue(&mut self,code:&str,target:&str,r:Result<()>){if let Err(e)=r{self.fail(code,target,format!("{e:#}"));}}
    fn bind(&mut self,p:&Path,label:&str)->Result<Value>{
        let b=self.files.capture(p)?;self.checked.insert(label.into(),b.clone());Ok(b)
    }
    fn record(&mut self,v:&Value,base:&Path,name:&str)->Option<(PathBuf,Value)>{
        let result=(||->Result<(PathBuf,Value)>{
            let p=files::canonical(&actual_path(v,base,name)?)?;
            ensure!(fs::metadata(&p)?.len()<=128*1024*1024,"Record exceeds JSON input budget");
            let (raw,b)=files::read_bound(&p)?;
            ensure!(b==self.bind(&p,name)?&&raw.len()<=128*1024*1024,"Record changed during reading");
            let v=crate::parse_json(std::str::from_utf8(&raw)?.trim_start_matches('\u{feff}'))?;
            ensure!(v.as_object().is_some_and(|m|!m.is_empty()),"A nonempty record object is required");
            Ok((p,v))
        })();
        match result{Ok(v)=>Some(v),Err(e)=>{self.fail("WORKFLOW_RECORD_MISSING",name,format!("{e:#}"));None}}
    }
    fn import(&mut self,v:&Value){
        if let Some(rows)=v["findings"].as_array(){for r in rows{
            if r.get("level").is_some()&&r["level"]!="error"{continue;}
            self.fail(r["code"].as_str().unwrap_or("WORKFLOW_VALIDATION"),r["object_id"].as_str().unwrap_or("record"),
                r["message"].as_str().unwrap_or("Unresolved record finding"));
        }}
    }
    fn report(self)->Value{
        let ready=self.findings.is_empty();
        json!({"findings":self.findings,"checked_files":self.checked,"pending_acceptance":self.pending,
            "source_fidelity":"not_assessed","records_ready":ready,"stage_reports":self.stages,
            "boundary":"Record completeness and syntax do not establish source fidelity."})
    }
}
fn source_scope(run:&Value,base:&Path)->Result<Value>{
    let roles=run["scope"]["source_roles"].as_object().context("Original source roles required")?;
    let original=run["identity"]["sources"].as_array().filter(|v|!v.is_empty()).context("Original source bindings required")?;
    let mut rows=Map::new();
    for source in original{
        let id=source["id"].as_str().filter(|s|!s.trim().is_empty()).context("Original source ID")?;
        let role=roles.get(id).filter(|v|matches!(v.as_str(),Some("main"|"supplement"))).context("Original source role")?;
        let p=actual_path(&source["path"],base,"original source")?;
        let b=check_ledger::bind_file(&p)?;
        ensure!(b["sha256"]==source["sha256"],"Original source changed: {id}");
        let mut row=b;row["role"]=role.clone();
        ensure!(rows.insert(id.into(),row).is_none(),"Duplicate source ID: {id}");
    }
    let declared=run["scope"]["source_ids"].as_array().context("Original source scope")?;
    let ids:BTreeSet<_>=declared.iter().map(|v|v.as_str().context("Invalid source scope ID")).collect::<Result<_>>()?;
    ensure!(ids.len()==declared.len()&&rows.keys().eq(roles.keys())&&ids==rows.keys().map(String::as_str).collect(),"Original source scope or role set differs");
    Ok(Value::Object(rows))
}
pub fn write_maps(run_path:&Path,deliveries:&Value,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    let (raw,b)=files::read_bound(run_path)?;let rp=path(&b["path"])?;
    let run=crate::parse_json(std::str::from_utf8(&raw)?)?;
    ensure!(run["workflow_profile"]==PROFILE,"Actual complete workflow run required");
    let base=rp.parent().context("Workflow directory")?;let expected=source_scope(&run,base)?;
    let inventory=actual_path(&run["records"]["source_inventory"],base,"source inventory")?;
    let candidate=actual_path(&run["config"]["candidate_path"],base,"candidate")?;
    let mut paths=Map::new();
    for name in ["source_map","si_map","link_map"]{
        if let Some(v)=run["records"].get(name){paths.insert(name.into(),json!(path_text(&actual_path(v,base,name)?)?));}
    }
    ensure!(check_ledger::bind_file(&rp)?==b,"Workflow changed before mapping");
    let report=crate::handoff_native::write(&inventory,&candidate,deliveries,&Value::Object(paths),&expected,&mut progress)?;
    ensure!(check_ledger::bind_file(&rp)?==b,"Workflow changed during mapping; preserve newly generated evidence");
    ensure!(source_scope(&run,base)?==expected,"Original sources changed during mapping");Ok(report)
}
fn matching_delivery_selectors(manifest:&Value,maps:&[&Value])->Result<()>{
    let mut expected=BTreeMap::<String,Value>::new();
    for d in manifest["deliveries"].as_array().context("Explicit manifest deliveries required")?{
        let id=d["unit"].as_str().filter(|s|!s.is_empty()).context("Missing manifest unit")?;
        ensure!(d["target"].is_object(),"Missing manifest selector for {id}");
        ensure!(expected.insert(id.into(),json!({"target":d["target"],"sheet":d["sheet"]})).is_none(),"Duplicate manifest unit {id}");
    }
    let mut actual=BTreeMap::<String,Value>::new();
    for map in maps{
        let units=&map["units"];
        let rows:Vec<&Value>=if let Some(a)=units.as_array(){a.iter().collect()}else if let Some(m)=units.as_object(){m.values().collect()}else{anyhow::bail!("Source map units are required")};
        for row in rows{
            let id=row["unit"].as_str().filter(|s|!s.is_empty()).context("Missing mapped unit")?;
            let d=&row["delivery"];
            ensure!(d["unit"]==id&&row["target"].is_object()&&row["target"]==d["target"],"Contradictory mapped selector for {id}");
            ensure!(actual.insert(id.into(),json!({"target":d["target"],"sheet":d["sheet"]})).is_none(),"Duplicate mapped unit {id}");
        }
    }
    ensure!(expected==actual,"Manifest and saved source maps must cover identical units, target selectors and worksheet selectors");
    Ok(())
}
pub fn inspect(run_path:&Path,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    let mut a=Review::new();
    let outcome=(||->Result<()>{
        let rp=files::canonical(run_path)?;let base=rp.parent().context("Workflow directory")?;
        let Some((_,run))=a.record(&json!(path_text(&rp)?),base,"run")else{return Ok(());};
        if run["workflow_profile"]!=PROFILE{a.fail("WORKFLOW_PROFILE","run","Use the actual complete-workflow run record.");return Ok(());}
        let (config,scope,records)=(&run["config"],&run["scope"],&run["records"]);
        if !config.is_object()||!scope.is_object()||!records.is_object(){a.fail("WORKFLOW_SCHEMA","run","config, scope and records must be objects.");return Ok(());}
        let r=(||->Result<()>{
            let baseline=run["versions"]["files"].as_array().filter(|a|!a.is_empty()).context("Retain actual skill input baseline")?;
            let skills=files::canonical(&actual_path(&config["skills_root"],base,"skills root")?)?;
            let mut expected=BTreeSet::new();
            for b in baseline{
                let p=path(&b["path"])?;
                let actual=a.files.capture(&p)?;
                ensure!(actual==*b&&expected.insert(path(&actual["path"])?),"Skill input changed or duplicated: {}",p.display());
            }
            let current:BTreeSet<_>=skill_input_paths(&skills)?.into_iter().collect();
            ensure!(expected==current,"Skill input file set changed after initialization");
            Ok(())
        })();a.issue("WORKFLOW_SKILL_BASELINE","versions.files",r);
        let expected=match source_scope(&run,base){Ok(v)=>v,Err(e)=>{a.fail("WORKFLOW_SOURCE_BASELINE","identity.sources",format!("{e:#}"));Value::Null}};
        if let Some(rows)=expected.as_object(){
            for(id,v)in rows{if let Err(e)=a.files.recorded(v,base){a.fail("WORKFLOW_SOURCE_BINDING",id,format!("{e:#}"));}}
        }
        let mut required=BTreeSet::from(["source_inventory","source_map","link_map","checks","delivery_manifest"]);
        if scope["translation_required"]==true{required.insert("translation_review");}
        else if scope["translation_required"]!=false{a.fail("WORKFLOW_SCOPE","translation_required","Declare whether translation is required.");}
        if config["glossary_path"].as_str().is_some_and(|s|!s.trim().is_empty()){required.insert("terminology");}
        if scope["source_roles"].as_object().is_some_and(|m|m.values().any(|v|v=="supplement")){required.insert("si_map");}
        let mut loaded=BTreeMap::<String,(PathBuf,Value)>::new();
        if let Some((p,v))=a.record(&records["source_inventory"],base,"source_inventory"){
            let r=(||->Result<()>{
                let sources=v["sources"].as_array().context("Inventory sources must be an array")?;
                if sources.iter().any(|s|s["role"]=="supplement"){required.insert("si_map");}
                let mut actual=Map::new();
                for s in sources{
                    let id=s["id"].as_str().context("Inventory source ID")?;
                    let b=a.files.recorded(s,p.parent().unwrap())?;
                    let mut row=b;row["role"]=s["role"].clone();
                    ensure!(actual.insert(id.into(),row).is_none(),"Duplicate inventory source ID");
                }
                ensure!(Value::Object(actual)==expected,"Inventory source set, identity or role differs from original scope");
                Ok(())
            })();a.issue("WORKFLOW_SOURCE_COVERAGE","source_inventory.sources",r);
            loaded.insert("source_inventory".into(),(p,v));
        }
        let total=required.len().saturating_sub(1);
        for(i,name)in required.iter().filter(|s|**s!="source_inventory").enumerate(){
            if let Some(v)=a.record(&records[*name],base,name){loaded.insert((*name).into(),v);}
            progress(i+1,total,name);
        }
        for(name,fields)in [("source_map",&["blocks","objects","references","citations"][..]),("si_map",&["sources","blocks","objects"][..]),
            ("link_map",&["anchors","internal_links","library_matches","unresolved"][..])]{
            if let Some((_,v))=loaded.get(name){for field in fields{
                if !v[*field].is_array()&&!v[*field].is_object(){a.fail("WORKFLOW_MAP_FIELD_MISSING",&format!("{name}.{field}"),"Retain the actual object collection.");}
            }}
        }
        let candidate=match actual_path(&config["candidate_path"],base,"candidate").and_then(|p|files::canonical(&p)){
            Ok(p)=>match a.bind(&p,"candidate"){Ok(_)=>Some(p),Err(e)=>{a.fail("WORKFLOW_CANDIDATE_MISSING","candidate",e.to_string());None}},
            Err(e)=>{a.fail("WORKFLOW_CANDIDATE_MISSING","candidate",e.to_string());None}
        };
        let naming=&run["naming"];
        if !naming["filename"].as_str().is_some_and(|s|!s.trim().is_empty()){a.fail("WORKFLOW_NAMING_MISSING","run.naming.filename","Record the source-based naming plan.");}
        else if naming["status"]=="pending"{a.fail("WORKFLOW_NAMING_PENDING","run.naming","Resolve the naming decision.");}
        let r=(||->Result<()>{
            let d=if let Some(p)=&candidate{json!(path_text(p)?)}else{Value::Null};
            let report=crate::naming::validate_plan(naming,&d)?;
            for f in report["findings"].as_array().context("Naming findings")?{
                a.fail(f["code"].as_str().unwrap_or("NAMING_PLAN"),&format!("run.naming{}",f["field"].as_str().unwrap_or("")),f["message"].as_str().unwrap_or("Invalid naming plan"));
            }
            if report["plan_status"]=="blocked"{a.fail("WORKFLOW_NAMING_BLOCKED","run.naming/status","Resolve the naming obstruction.");}
            a.stages.insert("naming".into(),report);Ok(())
        })();a.issue("WORKFLOW_NAMING","run.naming",r);
        if let Some((p,l))=loaded.get("checks"){
            let r=(||->Result<()>{
                ensure!(files::canonical(&actual_path(&l["skills_root"],p.parent().unwrap(),"ledger skills root")?)?==
                    files::canonical(&actual_path(&config["skills_root"],base,"original skills root")?)?,"Ledger uses another skill catalog");
                ensure!(run["versions"]["rule_files"].is_array()&&l["rule_files"]==run["versions"]["rule_files"],"Ledger rule files differ from original catalog");
                let report=crate::check_validate::validate(p,&mut progress)?;
                a.import(&report);
                let count=report["expected_count"].as_u64().unwrap_or(0);
                if count==0{a.fail("WORKFLOW_CHECK_EMPTY","checks","Keep the full acceptance catalog.");}
                a.pending=check_ledger::pending_batch(p,usize::try_from(count.max(1))?,None)?;
                for(key,code)in [("offline_unresolved_ids","WORKFLOW_CHECK_PENDING"),("unclassified_ids","WORKFLOW_CHECK_UNCLASSIFIED")]{
                    for id in report[key].as_array().context("Atomic check report schema")?{a.fail(code,id.as_str().unwrap_or("check"),"Complete the original file goal with its own scope and evidence.");}
                }
                if let Some((ip,inv))=loaded.get("source_inventory"){
                    ensure!(files::canonical(&actual_path(&inv["checks_path"],ip.parent().unwrap(),"inventory check pointer")?)?==*p,"Inventory must bind this exact check ledger");
                }
                a.stages.insert("checks".into(),report);Ok(())
            })();a.issue("WORKFLOW_CHECK_BINDING","checks",r);
        }
        if let Some((p,m))=loaded.get("delivery_manifest"){
            if m["figure_edge_reviews_required"]!=true{a.fail("WORKFLOW_FIGURE_EDGES_REQUIRED","delivery_manifest.figure_edge_reviews_required","Final per-asset edge observations required.");}
            for (field,name)in [("inventory","source_inventory"),("checks","checks"),("translation_review","translation_review"),("terminology","terminology"),("candidate","candidate")]{
                if let Some(expected)=a.checked.get(name){
                    if m[field]!=*expected{a.fail("WORKFLOW_MANIFEST_BINDING",field,"Bind the exact inspected file and hash.");}
                }
            }
            let r=(||->Result<()>{
                let manifest_root=files::canonical(&actual_path(&m["resource_root"],p.parent().unwrap(),"manifest resource root")?)?;
                let original_root=files::canonical(&actual_path(&config["resource_root"],base,"workflow resource root")?)?;
                ensure!(manifest_root==original_root,"Manifest resource root differs from workflow configuration");Ok(())
            })();a.issue("WORKFLOW_RESOURCE_ROOT","delivery_manifest.resource_root",r);
            if let Some((_,main))=loaded.get("source_map"){
                let mut maps=vec![main];
                if let Some((_,si))=loaded.get("si_map"){maps.push(si);}
                let r=matching_delivery_selectors(m,&maps);a.issue("WORKFLOW_DELIVERY_SELECTOR_MISMATCH","delivery_manifest.deliveries",r);
            }
            match crate::delivery_validate::validate_delivery(p,&mut progress){
                Ok(report)=>{
                    if let Some(rows)=report["checked_files"].as_array(){
                        for b in rows{let r=a.files.recorded(b,base).map(|_|());a.issue("WORKFLOW_DELIVERY_EVIDENCE","delivery_manifest",r);}
                    }
                    a.import(&report);a.stages.insert("delivery".into(),report);
                },
                Err(e)=>a.fail("WORKFLOW_DELIVERY","delivery_manifest",format!("{e:#}"))
            }
        }
        if let (Some(cp),Some((ip,_)))=(&candidate,loaded.get("source_inventory")){
            let mut paths=Map::new();
            for name in ["source_map","si_map","link_map"]{
                if let Some((p,_))=loaded.get(name){paths.insert(name.into(),json!(path_text(p)?));}
            }
            let report=crate::handoff_native::validate(ip,cp,&Value::Object(paths),&expected,&mut progress)?;
            if let Some(rows)=report["evidence_files"].as_array(){
                for b in rows{let r=a.files.recorded(b,base).map(|_|());a.issue("WORKFLOW_MAP_EVIDENCE","handoff_maps",r);}
            }
            a.import(&report);
            if report["records_ready"]!=true{a.fail("WORKFLOW_MAPS_UNRESOLVED","handoff_maps","Complete source and link mappings for this exact scope.");}
            a.stages.insert("handoff".into(),report);
        }
        if let Some((p,_))=loaded.get("translation_review"){
            let ip=loaded.get("source_inventory").map(|(p,_)|p.as_path());
            let report=crate::translation_validate::validate(p,ip,candidate.as_deref())?;
            a.import(&report);
            if report["records_ready"]!=true{a.fail("WORKFLOW_TRANSLATION_UNRESOLVED","translation_review","Complete the remaining translation goals.");}
            a.stages.insert("translation".into(),report);
        }
        if let Some(cp)=&candidate{
            for f in crate::validate::validate_file(cp){if f.level=="error"{a.fail(f.code,&format!("candidate:{}",f.line),f.message);}}
            match crate::presentation::lint_file(cp){
                Ok(prompts) if !prompts.is_empty()=>{
                    let mut rows=vec![];
                    if let Some((_,v))=a.record(&records["presentation_review"],base,"presentation_review"){
                        if v["bindings"]["candidate"]!=a.checked["candidate"]{a.fail("WORKFLOW_PRESENTATION_BINDING","presentation_review.bindings.candidate","Bind the current candidate before reusing prompt decisions.");}
                        if let Some(items)=v["items"].as_array().filter(|a|a.iter().all(Value::is_object)){rows=items.clone();}
                        else{a.fail("WORKFLOW_PRESENTATION_SCHEMA","presentation_review.items","Each disposition must be an object.");}
                    }
                    for prompt in prompts{
                        let found:Vec<_>=rows.iter().filter(|r|r["code"]==prompt.code&&r["line"].as_u64()==Some(prompt.line as u64)).collect();
                        if found.len()!=1||found[0]["status"]!="not_an_issue"||!found[0]["reason"].as_str().is_some_and(|s|!s.trim().is_empty()){
                            a.fail("WORKFLOW_PRESENTATION_UNRESOLVED",&format!("{}:{}",prompt.code,prompt.line),prompt.message);
                        }
                    }
                },
                Ok(_)=>{},
                Err(e)=>a.fail("WORKFLOW_PRESENTATION","candidate",format!("{e:#}")),
            }
        }
        a.files.verify()?;
        Ok(())
    })();a.issue("WORKFLOW_INSPECTION","run",outcome);
    // A malformed or incomplete run must never produce an empty finding set
    // that looks like qualified delivery.
    if a.findings.is_empty(){a.fail("NATIVE_WORKFLOW_PARITY_PENDING","run","Complete historical delivery equivalence remains unqualified.");}
    Ok(a.report())
}
pub fn require_completed(run:&Path,progress:impl FnMut(usize,usize,&str))->Result<Value>{
    let report=inspect(run,progress)?;
    ensure!(report["records_ready"]==true,"Local records are unfinished; continue the actual unresolved requirements: {}",report["findings"]);
    Ok(report)
}

#[cfg(test)]
mod delivery_selector_tests{
    use super::*;
    #[test]
    fn identical_targets_across_main_and_supplement_are_required(){
        let first=json!({"unit":"p","target":{"lines":[1,2]}});
        let second=json!({"unit":"t","target":{"lines":[5,8]},"sheet":{"name":"Data"}});
        let manifest=json!({"deliveries":[first.clone(),second.clone()]});
        let main=json!({"units":[{"unit":"p","target":first["target"],"delivery":first}]});
        let si=json!({"units":[{"unit":"t","target":second["target"],"delivery":second}]});
        assert!(matching_delivery_selectors(&manifest,&[&main,&si]).is_ok());
        assert!(matching_delivery_selectors(&manifest,&[&main]).is_err());
        assert!(matching_delivery_selectors(&manifest,&[&main,&si,&main]).is_err());
        let mut altered=si.clone();altered["units"][0]["delivery"]["sheet"]=json!({"name":"Other"});
        assert!(matching_delivery_selectors(&manifest,&[&main,&altered]).is_err());
        let mut altered=main.clone();altered["units"][0]["target"]=json!({"lines":[2,3]});
        assert!(matching_delivery_selectors(&manifest,&[&altered,&si]).is_err());
    }
}
