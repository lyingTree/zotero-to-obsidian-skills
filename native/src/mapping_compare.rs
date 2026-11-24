//! Stable mapping identity comparison. This low-level comparison is not a
//! replacement for record_mapping_rerun, which must actually regenerate and
//! validate the maps. Only the fields named by the original contract are used.
use anyhow::{ensure,Context,Result};
use serde_json::{json,Map,Value};
use std::collections::BTreeSet;

pub const FIELDS:&[(&str,&[&str])]=&[
 ("blocks",&["source_id","source_attachment","source_locator","source_locators","source_blocks","joiner","original_sha256","decision","target_units","target_unit"]),
 ("units",&["source_id","source_role","source_attachment","source_locator","registered_unit","target","candidate_locator","resources"]),
 ("anchors",&["target_id","target_unit","source_id","candidate_locator"]),
 ("links",&["source_id","origin_unit","origin_block","candidate_locator","href","display_text","source_display_text","syntax","image","target_id","target_unit","target_path"]),
];
fn project(doc:&Value,name:&str)->Result<Value>{
    ensure!(doc.is_object(),"A mapping document must be a JSON object");
    let kinds:&[(&str,&str,&str)]=match name{
        "source_map"|"si_map"=>&[("blocks","block_id","blocks"),("units","unit","units")],
        "link_map"=>&[("anchors","id","anchors"),("internal_links","id","links"),("library_matches","id","links"),("external_links","id","links"),("resource_links","id","links")],
        _=>anyhow::bail!("Unknown mapping name {name}"),
    };
    let mut result=Map::new();
    for &(collection,key,group) in kinds{
        let mut index=Map::new();
        let fields=FIELDS.iter().find(|(k,_)|*k==group).unwrap().1;
        let mut rows=Vec::new();
        if let Some(v)=doc.get(collection){
            if let Some(map)=v.as_object(){
                for (id,row) in map{
                    let mut row=row.as_object().context("mapping row must be an object")?.clone();
                    if let Some(declared)=row.get(key){
                        ensure!(declared.as_str()==Some(id.as_str()),"Mapping object key disagrees with row identity: {name}.{collection}.{id}");
                    }else{row.insert(key.to_owned(),json!(id));}
                    rows.push(Value::Object(row));
                }
            }else{rows=v.as_array().context("mapping collection must be array or object")?.clone();}
        }
        for row in rows{
            ensure!(row.is_object(),"mapping row must be an object");
            let id=row[key].as_str().filter(|s|!s.is_empty()).with_context(||format!("Each stable mapping needs its unique identity: {name}.{collection}"))?;
            ensure!(!index.contains_key(id),"Each stable mapping needs its unique identity: {name}.{collection}");
            index.insert(id.into(),Value::Object(fields.iter().filter_map(|k|row.get(*k).map(|v|((*k).to_owned(),v.clone()))).collect()));
        }
        result.insert(collection.into(),Value::Object(index));
    }
    Ok(Value::Object(result))
}
pub fn compare(previous:&Value,current:&Value)->Result<Value>{
    let a=previous.as_object().context("baseline maps must be an object")?;
    let b=current.as_object().context("comparison maps must be an object")?;
    ensure!(a.keys().eq(b.keys())&&a.contains_key("source_map")&&a.contains_key("link_map"),"Keep the actual source and link maps and matching map names");
    let mut counts=Map::new();let mut differences=Vec::new();
    for (name,left) in a{
        let left=project(left,name)?;let right=project(&b[name],name)?;let mut per=Map::new();
        for (collection,rows) in left.as_object().unwrap(){
            let rows=rows.as_object().unwrap();let other=right[collection].as_object().unwrap();
            per.insert(collection.clone(),json!(rows.len()));
            let identities:BTreeSet<_>=rows.keys().chain(other.keys()).collect();
            for id in identities{
                if rows.get(id)!=other.get(id){differences.push(json!({"map":name,"collection":collection,"id":id,"previous":rows.get(id),"rerun":other.get(id)}));}
            }
        }
        counts.insert(name.clone(),Value::Object(per));
    }
    Ok(json!({"baseline_counts":counts,"mapping_equal":differences.is_empty(),"differences":differences,
        "source_fidelity":"not_assessed","acceptance_status_written":false,"actual_rerun_performed":false,
        "boundary":"Stable projection comparison of supplied maps only. Actual regeneration and validation are required for rerun evidence."}))
}


/// Compare real, separately supplied mapping files and preserve the exact
/// input bindings. This must never be reported as an actual mapping rerun:
/// the full mapping generator and validator must first be native.
pub fn compare_bound_maps(previous:&Value,current:&Value,destination:&std::path::Path)->Result<std::path::PathBuf>{
    use crate::{check_ledger,files,path,path_text,parse_json};
    use std::{collections::HashSet,io::Write,path::Path};
    let a=previous.as_object().context("baseline map paths must be an object")?;
    let b=current.as_object().context("new map paths must be an object")?;
    ensure!(a.keys().eq(b.keys())&&a.contains_key("source_map")&&a.contains_key("link_map"),
        "Keep the actual source and link maps and matching map names");
    let mut bindings=Map::new();let mut docs_a=Map::new();let mut docs_b=Map::new();
    let mut seen=HashSet::new();
    for (label,paths,docs) in [("baseline",a,&mut docs_a),("comparison",b,&mut docs_b)]{
        let mut records=Map::new();
        for (name,value) in paths{
            let (raw,record)=files::read_bound(&path(value)?)?;
            ensure!(seen.insert(record["path"].as_str().context("map path")?.to_owned()),
                "Each baseline/comparison map must be a separate actual file");
            let document=parse_json(std::str::from_utf8(&raw)?)?;
            ensure!(document.is_object(),"A mapping document must be a JSON object");
            records.insert(name.clone(),record);docs.insert(name.clone(),document);
        }
        bindings.insert(label.into(),Value::Object(records));
    }
    let mut report=compare(&Value::Object(docs_a),&Value::Object(docs_b))?;
    report["inputs"]=Value::Object(bindings);
    report["compared_fields"]=json!(FIELDS.iter().map(|(group,fields)|(*group,*fields)).collect::<std::collections::BTreeMap<_,_>>());
    report["schema_version"]=json!(1);
    let parent=destination.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or_else(||Path::new("."));
    std::fs::create_dir_all(parent)?;
    let out=files::canonical(parent)?.join(destination.file_name().context("comparison filename")?);
    ensure!(!seen.contains(&path_text(&out)?),"Comparison must not replace a map");
    ensure!(!out.exists()&&!out.is_symlink(),"Keep the existing comparison evidence");
    for records in report["inputs"].as_object().unwrap().values(){
        for b in records.as_object().unwrap().values(){
            ensure!(check_ledger::bind_file(&path(&b["path"])?)?==*b,"A comparison input changed before publication");
        }
    }
    let mut bytes=serde_json::to_vec_pretty(&report)?;bytes.push(b'\n');
    let mut temp=tempfile::NamedTempFile::new_in(out.parent().unwrap())?;
    temp.write_all(&bytes)?;temp.as_file().sync_all()?;
    temp.persist_noclobber(&out).map_err(|e|anyhow::anyhow!("Cannot exclusively publish comparison: {}",e.error))?;
    ensure!(std::fs::read(&out)?==bytes,"Comparison report did not round-trip");
    Ok(out)
}


/// Generate genuinely new mappings from the bound run inputs, not two
/// arbitrarily supplied map snapshots. Validate both before comparing.
pub fn record_mapping_rerun(run_path:&std::path::Path,deliveries:&Value,destination:&std::path::Path,mut progress:impl FnMut(usize,usize,&str))->Result<std::path::PathBuf>{
    use crate::{check_ledger,files,handoff_native::{self,Bindings},path,path_text};
    use std::path::Path;
    let mut run_inputs=Bindings::new();let (run,run_binding)=run_inputs.json(run_path)?;
    let run_path=path(&run_binding["path"])?;let base=run_path.parent().context("Run directory")?;
    let records=run["records"].as_object().context("Workflow records required")?;
    let config=run["config"].as_object().context("Workflow config required")?;
    let inventory=base.join(path(records.get("source_inventory").context("Source inventory required")?)?);
    let candidate=base.join(path(config.get("candidate_path").context("Candidate required")?)?);
    let mut original=Map::new();
    for name in ["source_map","si_map","link_map"]{
        if let Some(v)=records.get(name){original.insert(name.into(),json!(path_text(&base.join(path(v)?))?));}
    }
    ensure!(original.contains_key("source_map")&&original.contains_key("link_map"),"Keep original source and link maps");
    let identity=run["identity"]["sources"].as_array().filter(|a|!a.is_empty()).context("Original source bindings required")?;
    // Refuse duplicate source paths and a contradictory declared scope before
    // regenerating any actual mapping output.
    let mut source_paths=BTreeSet::new();
    let roles=run["scope"]["source_roles"].as_object().context("Original source roles required")?;
    let mut expected=Map::new();
    for row in identity{
        let id=row["id"].as_str().filter(|s|!s.is_empty()).context("Original source ID required")?;
        let role=roles.get(id).context("Original source role required")?;
        ensure!(matches!(role.as_str(),Some("main"|"supplement")),"Invalid original source role");
        let original_path=files::canonical(&base.join(path(&row["path"])?) )?;
        ensure!(source_paths.insert(original_path.clone()),"Duplicate original source attachment");
        let b=json!({"path":path_text(&original_path)?,"sha256":row["sha256"],"role":role});
        ensure!(expected.insert(id.into(),b).is_none(),"Duplicate original source ID");
    }
    ensure!(expected.keys().eq(roles.keys()),"Original source scope differs");
    if let Some(scope)=run["scope"].get("source_ids"){
        let list=scope.as_array().context("Original source IDs must be an array")?;
        let ids:BTreeSet<_>=list.iter().map(|v|v.as_str().context("Invalid source scope ID")).collect::<Result<_>>()?;
        ensure!(ids.len()==list.len()&&ids==expected.keys().map(String::as_str).collect(),"Original declared source scope differs");
    }
    let expected=Value::Object(expected);let original=Value::Object(original);
    let old=handoff_native::validate_full(&inventory,&candidate,&original,&expected,&mut progress).context("Resolve original native mapping diagnostics before rerun")?;
    let dest=std::path::absolute(destination)?;
    ensure!(!dest.exists()&&!dest.is_symlink(),"Use a new mapping rerun directory");
    let parent=dest.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let dest=files::canonical(parent)?.join(dest.file_name().context("Rerun destination name")?);
    std::fs::create_dir(&dest)?;
    let paths:Map<String,Value>=original.as_object().unwrap().keys().map(|name|{
        Ok((name.clone(),json!(path_text(&dest.join(format!("{}.json",name.replace('_',"-"))))?)))
    }).collect::<Result<_>>()?;
    let paths=Value::Object(paths);
    old.bindings.verify()?;run_inputs.verify()?;
    let generation=handoff_native::write(&inventory,&candidate,deliveries,&paths,&expected,&mut progress)?;
    ensure!(generation["records_ready"]==true,"Actual mapping rerun generation failed: {}",generation["findings"]);
    let new=handoff_native::validate_full(&inventory,&candidate,&paths,&expected,&mut progress).context("Actual rerun validation failed")?;
    for name in original.as_object().unwrap().keys(){
        ensure!(generation["checked_files"][name]==new.checked[name],"A generated map changed before independent validation: {name}");
    }
    for name in ["source_inventory","candidate"]{
        ensure!(old.checked[name]==new.checked[name]&&generation["checked_files"][name]==old.checked[name],"A bound source/candidate changed during actual rerun: {name}");
    }
    old.bindings.verify()?;new.bindings.verify()?;run_inputs.verify()?;
    let mut report=compare(&old.docs,&new.docs)?;
    let mut inputs=Map::new();inputs.insert("run".into(),run_binding);
    inputs.insert("inventory".into(),old.checked["source_inventory"].clone());
    inputs.insert("candidate".into(),old.checked["candidate"].clone());
    let mut rerun_maps=Map::new();
    for name in original.as_object().unwrap().keys(){
        inputs.insert(format!("baseline:{name}"),old.checked[name].clone());
        rerun_maps.insert(name.clone(),new.checked[name].clone());
    }
    for(id,row)in expected.as_object().unwrap(){
        let p=path(&row["path"])?;let b=check_ledger::bind_file(&p)?;
        ensure!(b["sha256"]==row["sha256"],"Source changed during rerun");
        inputs.insert(format!("source:{id}"),b);
    }
    report["schema_version"]=json!(1);report["created_at"]=json!(chrono::Utc::now().to_rfc3339());
    report["inputs"]=json!(inputs);report["rerun_maps"]=json!(rerun_maps);
    report["compared_fields"]=json!(FIELDS.iter().map(|(g,f)|(*g,*f)).collect::<std::collections::BTreeMap<_,_>>());
    report["actual_rerun_performed"]=json!(true);report["baseline_validated"]=json!(true);report["rerun_validated"]=json!(true);
    report["boundary"]=json!("Actual native mapping regeneration and stable identity comparison only. No translation, source identity or human observation is certified.");
    old.bindings.verify()?;new.bindings.verify()?;run_inputs.verify()?;
    let output=dest.join("rerun-comparison.json");handoff_native::publish(&output,&report)?;Ok(output)
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    #[test]
    fn object_key_cannot_disagree_with_embedded_identity() {
        let doc=json!({"source_map":{"blocks":{"block-a":{"block_id":"block-b"}}},"link_map":{}});
        assert!(compare(&doc,&doc).is_err());
    }
    #[test]
    fn implicit_object_identity_matches_explicit_array_identity() {
        let a=json!({"source_map":{"blocks":{"block-a":{"source_id":"main"}}},"link_map":{}});
        let b=json!({"source_map":{"blocks":[{"block_id":"block-a","source_id":"main"}]},"link_map":{}});
        assert_eq!(compare(&a,&b).unwrap()["mapping_equal"],true);
    }
}
