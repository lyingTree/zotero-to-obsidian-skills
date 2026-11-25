use anyhow::{anyhow,ensure,Result};
use serde_json::{json,Value};
use std::collections::{HashMap,HashSet};

pub fn author_token(markers:&Value,reason:&Value)->Result<String>{
    let list=markers.as_array().ok_or_else(||anyhow!("supply the ordered confirmed author markers as a sequence"))?;
    let names=list.iter().map(|v|{
        let s=v.as_str().ok_or_else(||anyhow!("invalid confirmed author marker"))?;
        ensure!(!s.trim().is_empty()&&s==s.trim()&&!s.contains(['\r','\n']),"invalid confirmed author marker");
        Ok(s)
    }).collect::<Result<Vec<_>>>()?;
    match names.as_slice(){
        []=>{ensure!(reason.as_str().is_some_and(|s|!s.trim().is_empty()),"missing evidence-based reason for unknown author");Ok("未知作者".into())}
        [one]=>Ok((*one).to_owned()),
        [one,two]=>Ok(format!("{one} and {two}")),
        [first,..]=>Ok(format!("{first} et al"))
    }
}
fn identity(value:&Value)->Option<(&str,u64)>{
    let kind=value.get("type")?.as_str()?;
    let id=value.get("id")?.as_u64()?;
    if id>0 && ["user","group"].contains(&kind){Some((kind,id))}else{None}
}
pub fn collection_ancestry(item:&Value,records:&Value,complete:bool)->Result<Value>{
    let data=item.get("data").and_then(Value::as_object).ok_or_else(||anyhow!("expected original Zotero item data"))?;
    let library=item.get("library").ok_or_else(||anyhow!("actual library identity required"))?;
    let id=identity(library).ok_or_else(||anyhow!("actual library identity required; route alias 0 is not identity"))?;
    let direct=data.get("collections").and_then(Value::as_array).ok_or_else(||anyhow!("missing or invalid original direct collections"))?;
    let mut unique=HashSet::new();
    for key in direct{
        let key=key.as_str().filter(|s|!s.is_empty()).ok_or_else(||anyhow!("invalid direct collection key"))?;
        ensure!(unique.insert(key),"duplicate original direct collection");
    }
    let records=records.as_array().ok_or_else(||anyhow!("expected collection records array"))?;
    let mut index=HashMap::new();
    let mut issues=vec![];
    for record in records{
        let key=record.get("key").and_then(Value::as_str).filter(|s|!s.is_empty()).ok_or_else(||anyhow!("collection key missing or inconsistent"))?;
        let data=record.get("data").filter(|v|v.is_object()).ok_or_else(||anyhow!("invalid collection API record"))?;
        ensure!(data.get("key").is_none_or(|v|v.as_str()==Some(key)),"collection key missing or inconsistent");
        ensure!(!index.contains_key(key),"duplicate collection key: {key}");
        if record.get("library").and_then(identity)!=Some(id){
            issues.push(json!({"key":key,"reason":"collection_library_unverified_or_different"}));continue;
        }
        index.insert(key,data);
    }
    if !complete{issues.push(json!({"reason":"pagination_incomplete"}));}
    let mut paths=Vec::with_capacity(direct.len());
    let missing = Value::Null;
    for (i,leaf) in direct.iter().enumerate(){
        let mut key=leaf;
        let mut seen=HashSet::new();
        let mut chain=vec![];
        let reason=loop{
            if key.as_bool()==Some(false){break None;}
            let Some(k)=key.as_str().filter(|s|!s.is_empty()) else {break Some("invalid_or_missing_parent_field")};
            if !seen.insert(k){break Some("collection_cycle");}
            let Some(data)=index.get(k) else {break Some("collection_record_missing")};
            let Some(name)=data.get("name").and_then(Value::as_str).filter(|s|!s.is_empty()) else {break Some("collection_name_missing")};
            chain.push(json!({"key":k,"name":name,"library":library}));
            key=data.get("parentCollection").unwrap_or(&missing);
        };
        if let Some(r)=reason{issues.push(json!({"key":leaf,"reason":r}));}
        chain.reverse();
        paths.push(json!({"library":library,"collection_key":leaf,"ancestry":chain,"status":if reason.is_some(){"incomplete"}else{"traced"}}));
        if (i+1)%128==0||i+1==direct.len(){eprintln!("[{}/{}] collections",i+1,direct.len());}
    }
    Ok(json!({"collections":paths,"issues":issues,"status":if issues.is_empty(){"complete"}else{"incomplete"},"pagination_complete":complete}))
}
