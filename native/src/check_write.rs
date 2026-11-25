//! Cooperative native writers preserve one decision per rule, immutable
//! evidence IDs, and the previous rows. No automatic semantic judgments.
use crate::{check_ledger,check_validate,files,path,path_text,pipeline,parse_json,properties,tables};
use anyhow::{anyhow,ensure,Context,Result};
use fs2::FileExt;
use serde_json::{json,Map,Value};
use std::{collections::{HashMap,HashSet},fs::{self,File,OpenOptions},io::Write,path::{Path,PathBuf}};

const STATUSES:&[&str]=&["passed","failed","blocked","not_verified","not_applicable"];
fn nonempty(v:&Value)->bool{v.as_str().is_some_and(|s|!s.trim().is_empty())}
pub(crate) fn no_link(p:&Path)->Result<()>{
    let m=fs::symlink_metadata(p)?;
    ensure!(m.is_file()&&!m.file_type().is_symlink(),"Refusing linked or non-file ledger: {}",p.display());
    #[cfg(windows)]{
        use std::os::windows::fs::MetadataExt;
        ensure!(m.file_attributes()&0x400==0,"Refusing a reparse-point ledger");
    }
    Ok(())
}
#[derive(Debug)]
pub(crate) struct TargetLock(File);
impl Drop for TargetLock{
    fn drop(&mut self){
        // On Unix flock is attached to the open file description. A child
        // spawned in another thread can briefly inherit it before exec;
        // explicitly unlock at scope exit instead of waiting for that last
        // descriptor to close. Still keep the stable lock file in place.
        let _=FileExt::unlock(&self.0);
    }
}
pub(crate) fn lock_target(path:&Path)->Result<TargetLock>{
    let file_name=path.file_name().context("ledger file name")?.to_string_lossy();
    let lock_path=path.with_file_name(format!(".{file_name}.z2o-lock"));
    match fs::symlink_metadata(&lock_path){
        Ok(_)=>no_link(&lock_path)?,
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},
        Err(e)=>return Err(e.into())
    }
    // Retain the stable lock inode rather than unlinking it after unlocking:
    // deleting a lock file while another process opens it can split the lock.
    let file=OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&lock_path)?;
    no_link(&lock_path)?;
    if let Err(e)=FileExt::try_lock_exclusive(&file){
        if e.raw_os_error()==fs2::lock_contended_error().raw_os_error(){
            return Err(e).context("Another native process is updating this ledger");
        }
        return Err(e).with_context(||format!("Cannot acquire native lock: {}",lock_path.display()));
    }
    Ok(TargetLock(file))
}
fn lock(path:&Path)->Result<(PathBuf,TargetLock)>{
    no_link(path)?;
    let path=files::canonical(path)?;
    let file=lock_target(&path)?;
    Ok((path,file))
}
fn load(p:&Path)->Result<(Value,String)>{
    let (data,binding)=files::read_bound(p)?;
    let value=parse_json(std::str::from_utf8(&data)?)?;
    ensure!(value.is_object(),"ledger must be an object");
    Ok((value,binding["sha256"].as_str().unwrap().into()))
}
pub(crate) fn replace_bytes(p:&Path,before:&str,bytes:&[u8])->Result<()>{
    let mut tmp=tempfile::Builder::new().prefix(".pending-ledger-").tempfile_in(p.parent().context("ledger directory")?)?;
    tmp.write_all(&bytes)?;tmp.as_file().sync_all()?;
    no_link(p)?;
    ensure!(files::snapshot(p)?["sha256"]==before,"Ledger changed since it was read; preserve the newer file.");
    tmp.persist(p).map_err(|e|anyhow!("Cannot publish ledger: {}",e.error))?;
    ensure!(fs::read(p)?==bytes,"Check results did not round-trip.");
    Ok(())
}
fn replace(p:&Path,before:&str,v:&Value)->Result<()>{
    let mut bytes=serde_json::to_vec_pretty(v)?;bytes.push(b'\n');
    replace_bytes(p,before,&bytes)?;
    ensure!(pipeline::read_json(p)?==*v,"Check results did not round-trip.");
    Ok(())
}
fn check_index(ledger:&Value)->Result<HashMap<String,usize>>{
    let rows=ledger["checks"].as_array().context("The ledger must contain checks.")?;
    let mut index=HashMap::new();
    for(i,row)in rows.iter().enumerate(){
        let id=row["id"].as_str().filter(|s|!s.trim().is_empty()).context("Every check needs its actual string rule ID")?;
        ensure!(index.insert(id.to_owned(),i).is_none(),"The existing ledger contains duplicate IDs.");
    }Ok(index)
}
fn actual_evidence(item:&Value,base:&Path)->Result<Value>{
    ensure!(item.is_object(),"Evidence entry must be an object");
    let p=path(&item["path"])?;
    let p=if p.is_absolute(){p}else{base.join(p)};
    let actual=check_ledger::bind_file(&p)?;
    ensure!(item["sha256"]==actual["sha256"],"Evidence bytes changed: {}",p.display());
    Ok(actual)
}
fn apply(p:&Path,ledger:&mut Value,results:&Value,evidence:&Value)->Result<()>{
    let results=results.as_array().filter(|v|!v.is_empty()).context("Provide explicit individual check results.")?;
    let index=check_index(ledger)?;
    let registry=ledger["evidence"].as_object().context("The shared evidence registry must be an object.")?;
    let incoming=evidence.as_object().context("The shared evidence registry must be an object.")?;
    let mut bindings=Vec::new();
    for(id,item)in incoming{
        ensure!(!id.trim().is_empty()&&item.is_object(),"Evidence entries need nonempty IDs and file bindings.");
        bindings.push(actual_evidence(item,p.parent().unwrap())?);
        if let Some(old)=registry.get(id){ensure!(old==item,"Use a new evidence ID for changed evidence: {id}");}
    }
    let allowed=["id","status","scope","evidence","reason","applicability_condition"];
    let mut seen=HashSet::new();let mut previous=Vec::with_capacity(results.len());
    for result in results{
        let fields=result.as_object().context("Each result must be an object")?;
        let id=result["id"].as_str().filter(|s|!s.trim().is_empty()).context("Every result needs a nonempty ID string.")?;
        ensure!(seen.insert(id),"Each batch entry must identify a different check.");
        let row_index=*index.get(id).with_context(||format!("Unknown acceptance ID: {id}"))?;
        let row=&ledger["checks"][row_index];
        ensure!(fields.keys().all(|k|allowed.contains(&k.as_str())),"{id}: do not replace a goal, declaration, or requirement type.");
        ensure!(result["status"].as_str().is_some_and(|s|STATUSES.contains(&s)),"{id}: provide an actual per-goal status.");
        ensure!(check_validate::scope_valid(&result["scope"]),"{id}: list the actual objects or tested absence condition.");
        ensure!(nonempty(&result["reason"]),"{id}: record the reason for this individual result.");
        let links=result["evidence"].as_array().filter(|v|v.iter().all(nonempty)).with_context(||format!("{id}: evidence must contain explicit evidence IDs."))?;
        for link in links{
            let name=link.as_str().unwrap();
            let value=incoming.get(name).or_else(||registry.get(name)).with_context(||format!("{id}: an evidence ID has not been registered."))?;
            // Recheck referenced old evidence too; do not silently store a
            // resolved outcome against a previously stale registry entry.
            bindings.push(actual_evidence(value,p.parent().unwrap())?);
        }
        if matches!(result["status"].as_str(),Some("passed"|"failed"|"not_applicable")){
            ensure!(!links.is_empty(),"{id}: an evaluated result needs actual evidence.");
            ensure!(row["requirement_type"]!="unclassified","{id}: classify the rule before recording a resolved result.");
        }
        if result["status"]=="not_applicable"{ensure!(nonempty(&result["applicability_condition"]),"{id}: record the actual applicability condition.");}
        previous.push(row.clone());
    }
    // Check all bindings again at the commit boundary. The final validator
    // independently checks locators, observation kinds and coverage.
    for b in bindings{
        ensure!(check_ledger::bind_file(Path::new(b["path"].as_str().unwrap()))?==b,"Evidence changed before decision publication");
    }
    let obj=ledger.as_object_mut().unwrap();
    let updates=obj.entry("updates").or_insert(json!([])).as_array_mut().context("updates must be an array")?;
    updates.push(json!({"at":chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros,false),"previous":previous}));
    ledger["evidence"].as_object_mut().unwrap().extend(incoming.clone());
    for result in results{
        let i=index[result["id"].as_str().unwrap()];
        ledger["checks"][i].as_object_mut().unwrap().extend(result.as_object().unwrap().clone());
    }
    Ok(())
}
pub fn record_batch(p:&Path,results:&Value,evidence:&Value)->Result<()>{
    let(p,_lock)=lock(p)?;
    let(mut ledger,digest)=load(&p)?;
    apply(&p,&mut ledger,results,evidence)?;
    replace(&p,&digest,&ledger)
}
pub fn record_rows(p:&Path,rows:&Value,contexts:&Value,evidence:&Value)->Result<()>{
    let rows=rows.as_array().filter(|r|!r.is_empty()).context("Provide a nonempty sequence of explicit decision rows.")?;
    let contexts=contexts.as_object().filter(|c|!c.is_empty()).context("Provide explicitly named review contexts.")?;
    for(name,context)in contexts{
        ensure!(!name.trim().is_empty()&&context.as_object().is_some_and(|m|m.len()==2&&m.contains_key("scope")&&m.contains_key("evidence")),
            "{name}: a context contains scope and evidence only, never a status.");
        ensure!(check_validate::scope_valid(&context["scope"]),"{name}: record the actual objects and their locations.");
        ensure!(context["evidence"].as_array().is_some_and(|a|!a.is_empty()&&a.iter().all(nonempty)),"{name}: provide actual evidence IDs.");
    }
    let mut results=Vec::with_capacity(rows.len());
    for row in rows{
        let a=row.as_array().filter(|a|matches!(a.len(),4|5)&&a.iter().all(nonempty)).context("Each decision row needs ID, status, context and reason; only NA adds a condition.")?;
        let id=a[0].as_str().unwrap();
        let context=contexts.get(a[2].as_str().unwrap()).with_context(||format!("{id}: unknown context"))?;
        ensure!((a[1]=="not_applicable")== (a.len()==5),"{id}: supply a condition exactly when the status is not_applicable.");
        let mut result=json!({"id":a[0],"status":a[1],"reason":a[3],"scope":context["scope"],"evidence":context["evidence"]});
        if a.len()==5{result["applicability_condition"]=a[4].clone();}
        results.push(result);
    }
    record_batch(p,&json!(results),evidence)
}
pub(crate) struct CandidateUpdate{
    pub before:Value,
    pub after:Value,
    pub old:Vec<u8>,
    pub new:Option<Vec<u8>>,
}
impl CandidateUpdate{
    // The caller holds the native ledger lock for preparation and publishing.
    pub fn prepare(run:&crate::workflow::CandidateRun)->Result<Self>{
        let p=&run.checks;
        let (old,before)=files::read_bound(p)?;
        let mut ledger=parse_json(std::str::from_utf8(&old)?)?;
        let data=&run.data;
        let actual_root=files::canonical(&p.parent().unwrap().join(path(&ledger["skills_root"])?))?;
        let expected_root=files::canonical(&run.base().join(path(&data["config"]["skills_root"])?))?;
        ensure!(ledger["schema_version"].as_u64()==Some(1)&&actual_root==expected_root&&
            data["versions"]["rule_files"].as_array().is_some_and(|a|!a.is_empty())&&
            ledger["rule_files"]==data["versions"]["rule_files"]&&
            data["identity"]["sources"].as_array().is_some_and(|a|!a.is_empty())&&
            ledger["bindings"]["sources"]==data["identity"]["sources"],
            "Check ledger does not match the original workflow skill catalog and source scope");
        for b in ledger["rule_files"].as_array().unwrap().iter().chain(ledger["bindings"]["sources"].as_array().unwrap()){
            let bound_path=files::canonical(&p.parent().unwrap().join(path(&b["path"])?))?;
            ensure!(bound_path!=*p&&path_text(&bound_path)?!=run.candidate["path"],
                "Candidate or check ledger aliases a frozen rule or source");
        }
        let mut binding=run.candidate.clone();binding["id"]=json!("candidate");
        let bindings=ledger["bindings"].as_object_mut().context("Ledger bindings must be an object")?;
        if let Some(existing)=bindings.get("candidates"){
            let rows=existing.as_array().context("Candidate bindings must be an array")?;
            if rows.as_slice()==[binding.clone()]{
                return Ok(Self{old,before:before.clone(),after:before,new:None});
            }
            ensure!(rows.is_empty(),"The candidate changed. Preserve the earlier ledger, invalidate affected results, and review the new file before rebinding.");
        }
        bindings.insert("candidates".into(),json!([binding]));
        let mut new=serde_json::to_vec_pretty(&ledger)?;new.push(b'\n');
        let after=json!({"path":before["path"],"sha256":files::digest(&new)});
        Ok(Self{old,before,after,new:Some(new)})
    }
    pub fn apply(&self,p:&Path)->Result<()>{
        no_link(p)?;
        ensure!(check_ledger::bind_file(p)?==self.before,"Check ledger changed before candidate binding");
        if let Some(new)=&self.new{replace_bytes(p,self.before["sha256"].as_str().unwrap(),new)?;}
        Ok(())
    }
}
pub fn bind_candidate(run_path:&Path)->Result<Value>{
    let run=crate::workflow::CandidateRun::open(run_path)?;
    let _lock=lock_target(&run.checks)?;
    let update=CandidateUpdate::prepare(&run)?;
    run.verify()?;
    update.apply(&run.checks)?;
    Ok(run.candidate)
}
pub struct AuditOptions<'a>{pub stage:&'a str,pub after:&'a Path,pub before:Option<&'a Path>,pub mode:Option<&'a str>,pub item_type:Option<&'a str>,pub kind:Option<&'a str>}
fn audit(opt:AuditOptions<'_>)->Result<Value>{
    if opt.stage=="tables"{return tables::audit(opt.after);}
    let before=opt.before.context("this audit requires independently supplied before and after files")?;
    match opt.stage{
        "properties"=>properties::audit(before,opt.after,opt.mode.context("mode required")?,opt.item_type.context("item_type required")?),
        "header"|"library"=>properties::audit_unchanged(before,opt.after,opt.stage),
        "write"=>properties::audit_write(before,opt.after),
        "translation"=>files::audit_translation_binding(before,opt.after,opt.kind.context("kind required")?),
        _=>Err(anyhow!("unsupported file audit stage: {}",opt.stage))
    }
}
pub fn record_file_checks(checks:&Path,report:&Path,opt:AuditOptions<'_>)->Result<Value>{
    let(p,_lock)=lock(checks)?;
    let(mut ledger,before)=load(&p)?;
    let row_index=check_index(&ledger)?;
    let result=audit(opt)?;
    let inputs=result["inputs"].as_array().filter(|a|!a.is_empty()).context("file-bound machine inputs required")?;
    let mut snapshots=Vec::new();
    let mut protected=HashSet::from([path_text(&p)?]);
    // A report must not replace a source, rule or other bound evidence file,
    // even if its name was supplied by the caller.
    for list in [&ledger["rule_files"],&ledger["bindings"]["sources"],&ledger["bindings"]["candidates"]]{
        for item in list.as_array().into_iter().flatten(){
            if let Some(s)=item["path"].as_str(){if let Ok(q)=files::canonical(&if Path::new(s).is_absolute(){PathBuf::from(s)}else{p.parent().unwrap().join(s)}){protected.insert(path_text(&q)?);}}
        }
    }
    for item in ledger["evidence"].as_object().into_iter().flat_map(|m|m.values()){
        if let Some(s)=item["path"].as_str(){if let Ok(q)=files::canonical(&if Path::new(s).is_absolute(){PathBuf::from(s)}else{p.parent().unwrap().join(s)}){protected.insert(path_text(&q)?);}}
    }
    for b in inputs{
        let s=files::snapshot(&path(&b["path"])?)?;
        ensure!(s["sha256"]==b["sha256"]&&s["size"].as_u64().is_some_and(|n|n>0),"Audit input changed or empty");
        protected.insert(s["path"].as_str().unwrap().to_owned());snapshots.push(s);
    }
    let parent=report.parent().filter(|s|!s.as_os_str().is_empty()).unwrap_or_else(||Path::new("."));
    fs::create_dir_all(parent)?;
    let output=files::canonical(parent)?.join(report.file_name().context("report file name")?);
    // Reusing an identical machine report is allowed only when it is not an
    // input to this audit. Never overwrite an earlier changed report.
    ensure!(!output.is_symlink(),"Refusing symlink report");
    let mut bytes=serde_json::to_vec_pretty(&result)?;bytes.push(b'\n');
    let hash=files::digest(&bytes);
    let output_text=path_text(&output)?;
    if protected.contains(&output_text){
        let was_report=ledger["evidence"].as_object().into_iter().flat_map(|m|m.iter()).any(|(id,item)|
            id.starts_with(&format!("rust-file-{hash}-"))&&item["path"]==output_text&&
            item["sha256"]==hash&&item["locator"]["kind"]=="json_pointer");
        let other_input=[&ledger["rule_files"],&ledger["bindings"]["sources"],&ledger["bindings"]["candidates"]].into_iter()
            .flat_map(|v|v.as_array().into_iter().flatten()).any(|item|item["path"]==output_text);
        ensure!(was_report&&!other_input&&output!=p&&!snapshots.iter().any(|b|b["path"]==output_text),
            "report would overwrite an input");
    }
    let mut evidence=Map::new();let mut input_ids=Vec::new();
    for(i,b)in snapshots.iter().enumerate(){
        let id=format!("rust-file-{hash}-input-{i}");
        let e=check_ledger::bind_evidence(Path::new(b["path"].as_str().unwrap()),&json!({"kind":"whole_file"}),"offline_result",&Value::Null)?;
        ensure!(e["sha256"]==b["sha256"],"audit input changed");
        evidence.insert(id.clone(),e);input_ids.push(id);
    }
    let mut decisions=Vec::new();
    for(id,decision)in result["checks"].as_object().context("audit checks")?{
        let Some(&i)=row_index.get(id)else{continue;};
        let row=&ledger["checks"][i];
        if row["requirement_type"]!="offline_outcome"||!id.rsplit_once("-A").is_some_and(|(p,n)|!p.is_empty()&&p.bytes().all(|c|c.is_ascii_uppercase())&&!n.is_empty()&&n.bytes().all(|c|c.is_ascii_digit()))||
            !matches!(decision["status"].as_str(),Some("passed"|"failed")){continue;}
        if !matches!(row["status"].as_str(),Some("not_verified"|"blocked")){
            ensure!(row["status"]==decision["status"],"existing decision conflicts with evaluated file result: {id}");continue;
        }
        let eid=format!("rust-file-{hash}-{id}");
        let scope=snapshots.iter().map(|b|json!({"object_id":b["path"],"locator":{"kind":"whole_file"}})).collect::<Vec<_>>();
        let mut links=vec![eid];links.extend(input_ids.iter().cloned());
        decisions.push(json!({"id":id,"status":decision["status"],"reason":decision["reason"],"scope":scope,"evidence":links}));
    }
    pipeline::publish(&output,&bytes)?;
    for d in &decisions{
        let id=d["id"].as_str().unwrap();let eid=format!("rust-file-{hash}-{id}");
        let e=check_ledger::bind_evidence(&output,&json!({"kind":"json_pointer","pointer":format!("/checks/{id}")}),"offline_result",&Value::Null)?;
        ensure!(e["sha256"]==hash,"audit report changed");
        evidence.insert(eid,e);
    }
    for b in &snapshots{ensure!(files::snapshot(Path::new(b["path"].as_str().unwrap()))?==*b,"audit input changed before recording");}
    if !decisions.is_empty(){
        apply(&p,&mut ledger,&json!(decisions),&Value::Object(evidence))?;
        replace(&p,&before,&ledger)?;
    }
    Ok(json!(decisions.iter().map(|d|d["id"].clone()).collect::<Vec<_>>()))
}

#[cfg(all(test,unix))]
mod lock_tests{
    use super::*;

    #[test]
    fn released_target_lock_does_not_wait_for_inherited_description_to_close(){
        let temp=tempfile::tempdir().unwrap();
        let path=temp.path().join("checks.json");
        let holder=lock_target(&path).unwrap();
        // dup shares the same open file description, just as a concurrent
        // spawn/fork can before close-on-exec. A distinct writer still uses a
        // separate open file description and must be rejected while live.
        let inherited=holder.0.try_clone().unwrap();
        let blocked=lock_target(&path).unwrap_err();
        assert!(blocked.to_string().contains("Another native process"));
        drop(holder);
        let reopened=lock_target(&path).unwrap();
        drop(inherited);
        drop(reopened);
        assert!(lock_target(&path).is_ok());
    }
}
