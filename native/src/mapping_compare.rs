//! Stable mapping identity comparison. This low-level comparison is not a
//! replacement for record_mapping_rerun, which must actually regenerate and
//! validate the maps. Only the fields named by the original contract are used.
use anyhow::{ensure,Context,Result};
use serde_json::{json,Map,Value};
use std::collections::BTreeSet;

pub const FIELDS:&[(&str,&[&str])]=&[
 ("blocks",&["source_id","source_attachment","source_locator","original_sha256","decision","target_units","target_unit"]),
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
                    if !row.contains_key(key){row.insert(key.to_owned(),json!(id));}
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
