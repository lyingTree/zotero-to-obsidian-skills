//! File-bound delivery manifest preparation and explicit source-unit contracts.
//! This is not the final delivery or handoff validator.
use crate::{check_ledger,check_write,files,path,path_text,parse_json,pipeline,syntax,workflow};
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
struct ManifestPlan{
    dest:PathBuf,
    bytes:Vec<u8>,
    bindings:Vec<Value>,
}
impl ManifestPlan{
    fn verify(&self,update:Option<&check_write::CandidateUpdate>)->Result<()>{
        for b in &self.bindings{
            let expected=update.filter(|u|u.before==*b).map_or(b,|u|&u.after);
            ensure!(check_ledger::bind_file(&path(&b["path"])?)?==*expected,
                "Input changed before/during manifest publication: {}",b["path"]);
        }
        Ok(())
    }
    fn existing_matches(&self)->Result<Option<bool>>{
        match fs::symlink_metadata(&self.dest){
            Ok(m)=>{
                check_write::no_link(&self.dest)?;
                Ok(Some(m.len()==self.bytes.len() as u64&&fs::read(&self.dest)?==self.bytes))
            }
            Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ok(None),
            Err(e)=>Err(e.into())
        }
    }
    fn preflight(&self)->Result<()>{
        ensure!(self.existing_matches()?!=Some(false),
            "Existing manifest differs; preserve it and use a new output path: {}",self.dest.display());
        Ok(())
    }
    fn publish(&self)->Result<()>{
        // Existing identical manifests can be reused. Differing bytes are
        // never overwritten, including a destination created after preflight.
        pipeline::publish(&self.dest,&self.bytes)?;
        ensure!(self.existing_matches()?==Some(true),"Manifest readback differs");
        Ok(())
    }
}
#[allow(clippy::too_many_arguments)]
fn prepare_manifest(inventory:&Path,candidate:&Path,deliveries:&Value,dest:&Path,root:Option<&Path>,
    translation:Option<&Path>,edges:bool,update:Option<&check_write::CandidateUpdate>)->Result<ManifestPlan>{
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
            result[target]=if key=="checks_path"{
                if let Some(u)=update{
                    ensure!(u.before==b,"Inventory check ledger changed during manifest preparation");
                    u.after.clone()
                }else{b.clone()}
            }else{b.clone()};
            bindings.push(b);
        }
    }
    if let Some(p)=translation{
        let b=check_ledger::bind_file(p)?;bindings.push(b.clone());result["translation_review"]=b;
    }
    if update.is_some(){ensure!(result["checks"].is_object(),"Workflow manifest must bind the exact workflow check ledger");}
    let parent=dest.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or_else(||Path::new("."));
    let dest=workflow::resolve_future(parent)?.join(dest.file_name().context("output filename")?);
    let protected:BTreeSet<_>=bindings.iter().filter_map(|v|v["path"].as_str()).collect();
    ensure!(!protected.contains(path_text(&dest)?.as_str()),"Manifest must not replace a bound input");
    let mut bytes=serde_json::to_vec_pretty(&result)?;bytes.push(b'\n');
    let plan=ManifestPlan{dest,bytes,bindings};
    plan.verify(None)?;
    plan.preflight()?;
    Ok(plan)
}
pub fn manifest(inventory:&Path,candidate:&Path,deliveries:&Value,dest:&Path,root:Option<&Path>,
    translation:Option<&Path>,edges:bool)->Result<PathBuf>{
    let plan=prepare_manifest(inventory,candidate,deliveries,dest,root,translation,edges,None)?;
    fs::create_dir_all(plan.dest.parent().unwrap())?;
    plan.verify(None)?;
    plan.publish()?;
    Ok(plan.dest)
}
fn receipt(p:&Path,v:&Value)->Result<()>{
    let mut bytes=serde_json::to_vec_pretty(v)?;bytes.push(b'\n');pipeline::publish(p,&bytes)
}
fn commit_workflow_manifest(run:&workflow::CandidateRun,plan:&ManifestPlan,update:&check_write::CandidateUpdate,
    before_manifest:impl FnOnce())->Result<()>{
    let verify=|applied|{
        run.verify()?;
        plan.verify(if applied{Some(update)}else{None})?;
        plan.preflight()
    };
    verify(false)?;
    if update.new.is_none(){
        before_manifest();
        verify(true)?;
        plan.publish()?;
        return verify(true);
    }
    // Filesystems cannot atomically rename two outputs together. Retain
    // prepared bytes and original evidence so even an interruption after a
    // single-file rename is diagnosable without overwriting external edits.
    let recovery=tempfile::Builder::new().prefix(".z2o-workflow-manifest-").tempdir_in(run.checks.parent().unwrap())?.keep();
    pipeline::publish(&recovery.join("checks.before"),&update.old)?;
    pipeline::publish(&recovery.join("checks.after"),update.new.as_ref().unwrap())?;
    pipeline::publish(&recovery.join("manifest.after"),&plan.bytes)?;
    receipt(&recovery.join("plan.json"),&json!({"schema_version":1,
        "run":run.run,"inventory":run.inventory,"candidate":run.candidate,
        "checks_before":update.before,"checks_after":update.after,
        "manifest_after":{"path":path_text(&plan.dest)?,"sha256":files::digest(&plan.bytes)},
        "boundary":"Individual files are atomic. Inspect actual hashes against retained preimages before recovery."}))?;
    let publication=(||->Result<()>{
        verify(false)?;
        update.apply(&run.checks)?;
        before_manifest();
        verify(true)?;
        plan.publish()?;
        verify(true)
    })();
    if let Err(error)=publication{
        // Never remove a manifest or overwrite intervening ledger edits. Only
        // restore our exact replacement when the intended manifest is known
        // not to be present, and this lock is still held.
        let rollback=match (plan.existing_matches(),check_write::no_link(&run.checks).and_then(|()|check_ledger::bind_file(&run.checks))){
            (_,Ok(current)) if current==update.before=>"unchanged",
            (Ok(None|Some(false)),Ok(current)) if current==update.after=>{
                match check_write::replace_bytes(&run.checks,update.after["sha256"].as_str().unwrap(),&update.old){
                    Ok(())=>"restored",
                    Err(_)=>"manual_review"
                }
            },
            _=>"manual_review"
        };
        let _=receipt(&recovery.join("failure.json"),&json!({"status":"failed","ledger_rollback":rollback}));
        return Err(error.context(format!("Workflow manifest failed; ledger {rollback}; recovery data: {}",recovery.display())));
    }
    receipt(&recovery.join("completed.json"),&json!({"status":"completed"}))
        .with_context(||format!("Ledger and manifest were written; completion receipt failed: {}",recovery.display()))?;
    Ok(())
}
pub fn write_workflow_manifest(run_path:&Path,deliveries:&Value)->Result<PathBuf>{
    write_workflow_manifest_with(run_path,deliveries,||{})
}
fn write_workflow_manifest_with(run_path:&Path,deliveries:&Value,before_manifest:impl FnOnce())->Result<PathBuf>{
    let run=workflow::CandidateRun::open(run_path)?;
    let records=&run.data["records"];let config=&run.data["config"];
    let inv=path(&run.inventory["path"])?;let candidate=path(&run.candidate["path"])?;
    let dest=absolute(&path(&records["delivery_manifest"])?,run.base());
    let root=absolute(&path(&config["resource_root"])?,run.base());
    let tr=if records["translation_review"].is_null(){None}else{Some(absolute(&path(&records["translation_review"])?,run.base()))};
    let translation_required=run.data["scope"]["translation_required"].as_bool().context("Declare whether translation is required")?;
    if translation_required{ensure!(tr.is_some(),"Record the required workflow translation review path");}
    if let Some(p)=&tr{ensure!(p.is_file(),"Prepare and complete translation-review.json before the final delivery index.");}
    let _ledger_lock=check_write::lock_target(&run.checks)?;
    let update=check_write::CandidateUpdate::prepare(&run)?;
    let plan=prepare_manifest(&inv,&candidate,deliveries,&dest,Some(&root),tr.as_deref(),true,Some(&update))?;
    ensure!(plan.bindings.contains(&run.inventory)&&plan.bindings.contains(&run.candidate),
        "Candidate or inventory changed during manifest preparation");
    ensure!(path_text(&plan.dest)?!=run.run["path"],"Manifest must not alias the workflow run");
    for (name,value) in records.as_object().unwrap(){
        if name=="delivery_manifest"{continue;}
        if let Some(other)=value.as_str().filter(|v|!v.trim().is_empty()){
            ensure!(workflow::resolve_future(&run.base().join(other))?!=plan.dest,
                "Manifest must not alias workflow record {name}");
        }
    }
    fs::create_dir_all(plan.dest.parent().unwrap())?;
    let _manifest_lock=check_write::lock_target(&plan.dest)?;
    commit_workflow_manifest(&run,&plan,&update,before_manifest)?;
    Ok(plan.dest)
}

#[cfg(test)]
mod workflow_transaction_tests{
    use super::*;
    fn save(p:&Path,v:&Value){fs::write(p,serde_json::to_vec(v).unwrap()).unwrap();}
    #[test]
    fn failed_second_publication_restores_only_our_ledger_replacement(){
        for external_edit in [false,true]{
            let temp=tempfile::tempdir().unwrap();
            let base=temp.path();
            let run=base.join("run.json");
            let inventory=base.join("inventory.json");
            let checks=base.join("checks.json");
            let candidate=base.join("candidate.md");
            let manifest=base.join("manifest.json");
            let rules=base.join("rules.md");
            let source=base.join("actual.docx");
            fs::write(&rules,"A rule").unwrap();fs::write(&source,"Source bytes").unwrap();
            fs::write(&candidate,"Actual candidate").unwrap();
            let mut s=check_ledger::bind_file(&source).unwrap();s["id"]=json!("main");
            let rules=json!([check_ledger::bind_file(&rules).unwrap()]);
            save(&inventory,&json!({"checks_path":checks}));
            save(&checks,&json!({"schema_version":1,"skills_root":base,"rule_files":rules,
                "bindings":{"sources":[s],"candidates":[]}}));
            save(&run,&json!({"schema_version":1,"workflow_profile":"complete_pdf_candidate_v1",
                "config":{"skills_root":base,"candidate_path":candidate,"resource_root":base},
                "scope":{"translation_required":false},"identity":{"sources":[s]},"versions":{"rule_files":rules},
                "records":{"source_inventory":inventory,"checks":checks,"delivery_manifest":manifest}}));
            let original=fs::read(&checks).unwrap();
            let intervening=b"An external writer's newer ledger.";
            let conflict=b"A different manifest created independently.";
            let error=write_workflow_manifest_with(&run,&json!([]),||{
                let applied=pipeline::read_json(&checks).unwrap();
                assert_eq!(applied["bindings"]["candidates"][0]["id"],"candidate");
                if external_edit{fs::write(&checks,intervening).unwrap();}
                fs::write(&manifest,conflict).unwrap();
            }).unwrap_err();
            assert!(error.to_string().contains(if external_edit{"ledger manual_review"}else{"ledger restored"}));
            assert_eq!(fs::read(&checks).unwrap(),if external_edit{intervening.as_slice()}else{original.as_slice()});
            assert_eq!(fs::read(&manifest).unwrap(),conflict);
            let recovery=fs::read_dir(base).unwrap().map(Result::unwrap).find(|e|e.file_name().to_string_lossy()
                .starts_with(".z2o-workflow-manifest-")).unwrap().path();
            assert_eq!(fs::read(recovery.join("checks.before")).unwrap(),original);
            assert!(recovery.join("plan.json").is_file()&&recovery.join("manifest.after").is_file());
            assert!(recovery.join("failure.json").is_file()&&!recovery.join("completed.json").exists());
        }
    }
}
