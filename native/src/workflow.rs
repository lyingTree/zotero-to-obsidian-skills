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

pub fn initialize(opt:Initialize<'_>,mut progress:impl FnMut(usize,usize,&str))->Result<PathBuf>{
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
        // The complete native PDF inventory is not yet implemented.
        ensure!(p.extension().and_then(|s|s.to_str()).is_some_and(|s|s.eq_ignore_ascii_case("docx")),
            "UNMIGRATED_PDF_INVENTORY: {id}; no Python fallback");
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
    source_prepare::prepare_docx_sources(&resolved,inventory.parent().unwrap(),opt.source_roles,
        glossary_path.as_deref(),144,|n,total,label|progress(n,total,label))?;
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
    // Do not replace the original complete inspector with a weaker checklist.
    anyhow::bail!("UNMIGRATED_WORKFLOW_INSPECTION: candidate exists; full mapping and delivery validation must be native before review routing can be certified")
}
