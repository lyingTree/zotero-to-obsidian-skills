//! Skill-local persistent configuration. User state lives beside the skill,
//! never in a generic home-directory config location.
use anyhow::{anyhow,ensure,Context,Result};
use serde_json::{json,Map,Value};
use std::{fs,path::{Path,PathBuf}};
use crate::{acceptance,check_write,files,path,path_text,pipeline,zotero_local};

pub const SKILL:&str="literature-zotero2obsidian";
pub const RELATIVE:&str="user/profile.json";
const MAX_BYTES:u64=256*1024;
const PATH_FIELDS:&[&str]=&[
    "zotero_local_api","zotero_data_dir","zotero_storage_dir","obsidian_vault_root",
    "literature_root","template_root","glossary_path","assets_resource_dir","work_root","obsidian_cli",
];

fn skill_root(explicit:Option<&Path>)->Result<PathBuf>{
    if let Some(p)=explicit{
        let p=files::canonical(p)?;
        ensure!(p.join("SKILL.md").is_file()&&p.file_name().and_then(|s|s.to_str())==Some(SKILL),
            "skill_root must name the {SKILL} skill directory");
        return Ok(p);
    }
    let exe=files::canonical(&std::env::current_exe()?)?;
    let runtime=exe.parent().context("runtime directory")?;
    let root=runtime.parent().context("skill directory")?;
    ensure!(root.file_name().and_then(|s|s.to_str())==Some(SKILL),
        "Cannot locate {SKILL} from the running executable; pass skill_root explicitly");
    files::canonical(root)
}
pub fn defaults()->Value{
    json!({
        "schema_version":1,
        "skill":SKILL,
        "paths":{
            "zotero_local_api":"http://127.0.0.1:23119/api",
            "zotero_data_dir":null,
            "zotero_storage_dir":null,
            "obsidian_vault_root":null,
            "literature_root":null,
            "template_root":null,
            "glossary_path":null,
            "assets_resource_dir":null,
            "work_root":null,
            "obsidian_cli":null
        },
        "zotero_library":null,
        "templates":{},
        "preferences":{
            "formula_table_enhancement":"unasked",
            "arxiv_tex_auto":true,
            "transparent_figure":"preserve_original"
        },
        "provenance":{"recorded_by":"first_run_discovery","notes":[]},
        "updated_at":null
    })
}
fn merge(base:&mut Value,patch:&Value){
    if let (Some(a),Some(b))=(base.as_object_mut(),patch.as_object()){
        for (k,v) in b{
            match (a.get_mut(k),v){
                (Some(Value::Object(old)),Value::Object(new))=>merge_value(old,new),
                _=>{a.insert(k.clone(),v.clone());}
            }
        }
    }
}
fn merge_value(base:&mut Map<String,Value>,patch:&Map<String,Value>){
    for (k,v) in patch{
        match (base.get_mut(k),v){
            (Some(Value::Object(old)),Value::Object(new))=>merge_value(old,new),
            _=>{base.insert(k.clone(),v.clone());}
        }
    }
}
fn looks_like_item_work_dir(p:&Path)->bool{
    let Some(name)=p.file_name().and_then(|s|s.to_str()) else{return false;};
    if matches!(name,"work"|"output"){return true;}
    if p.join("work").join("run.json").exists() || p.join("output").join("candidate.md").exists(){return true;}
    if let Some((_,tail))=name.rsplit_once('-'){
        if tail.len()==8 && tail.bytes().all(|b|b.is_ascii_uppercase()||b.is_ascii_digit()){
            return true;
        }
    }
    false
}
fn work_inside_vault(work:&Path,vault:&Path)->bool{
    let w=acceptance::resolve(work).unwrap_or_else(|_|work.to_path_buf());
    let v=acceptance::resolve(vault).unwrap_or_else(|_|vault.to_path_buf());
    w==v || w.starts_with(&v)
}
fn validate(profile:&Value)->Result<()>{
    let m=profile.as_object().context("profile must be a JSON object")?;
    ensure!(m.get("schema_version").and_then(Value::as_u64)==Some(1),"profile schema_version must be 1");
    ensure!(m.get("skill").and_then(Value::as_str).unwrap_or(SKILL)==SKILL,"profile skill identity differs");
    let paths=m.get("paths").and_then(Value::as_object).context("profile paths must be an object")?;
    for (key,value) in paths{
        ensure!(PATH_FIELDS.contains(&key.as_str()),"unknown profile path field: {key}");
        ensure!(value.is_null()||value.as_str().is_some_and(|s|!s.trim().is_empty()&&s.trim()==s),
            "profile path {key} must be null or nonempty trimmed text");
    }
    if let Some(wr)=paths.get("work_root").and_then(Value::as_str){
        let p=Path::new(wr);
        ensure!(p.is_absolute(),"paths.work_root must be an absolute path");
        ensure!(!looks_like_item_work_dir(p),
            "paths.work_root must be the shared base work directory (e.g. .../literature-pipeline-work/<Vault>), not a per-item subdirectory");
        if let Some(vr)=paths.get("obsidian_vault_root").and_then(Value::as_str){
            let vp=Path::new(vr);
            if vp.is_absolute(){
                ensure!(!work_inside_vault(p,vp),"paths.work_root must reside strictly outside obsidian_vault_root");
            }
        }
    }
    if let Some(lib)=m.get("zotero_library").filter(|v|!v.is_null()){
        let lm=lib.as_object().context("zotero_library must be an object or null")?;
        ensure!(lm.get("type").and_then(Value::as_str).is_some_and(|s|matches!(s,"user"|"group")),"zotero_library.type must be user or group");
        ensure!(lm.get("id").and_then(Value::as_u64).is_some(),"zotero_library.id must be an integer");
        ensure!(lm.get("name").is_none_or(|v|v.is_null()||v.as_str().is_some()),"zotero_library.name must be string or null");
    }
    if let Some(tpl)=m.get("templates").filter(|v|!v.is_null()){
        let tm=tpl.as_object().context("templates must be an object or null")?;
        for (k,v) in tm{
            ensure!(v.is_null()||v.as_str().is_some_and(|s|!s.trim().is_empty()&&s.trim()==s),
                "templates.{k} must be null or nonempty trimmed text");
        }
    }
    let preferences=m.get("preferences").and_then(Value::as_object).context("profile preferences must be an object")?;
    ensure!(preferences.get("formula_table_enhancement").and_then(Value::as_str)
        .is_some_and(|s|matches!(s,"unasked"|"native"|"ocr")),"formula_table_enhancement must be unasked, native or ocr");
    ensure!(preferences.get("arxiv_tex_auto").is_none_or(Value::is_boolean),"arxiv_tex_auto must be boolean");
    ensure!(preferences.get("transparent_figure").and_then(Value::as_str)
        .is_none_or(|s|matches!(s,"preserve_original"|"white")),"transparent_figure must be preserve_original or white");
    Ok(())
}
fn resolve_command_on_path(cmd:&str)->Option<PathBuf>{
    let path_var=std::env::var_os("PATH")?;
    let has_ext=Path::new(cmd).extension().is_some();
    let exts:Vec<String>=if cfg!(windows)&&!has_ext{
        std::env::var("PATHEXT").unwrap_or_else(|_|".COM;.EXE;.BAT;.CMD".to_owned())
            .split(';').filter(|s|!s.trim().is_empty()).map(|s|s.trim().to_owned()).collect()
    }else{vec![String::new()]};
    for dir in std::env::split_paths(&path_var){
        if dir.as_os_str().is_empty(){continue;}
        for ext in &exts{
            let candidate=if ext.is_empty(){dir.join(cmd)}else{dir.join(format!("{cmd}{ext}"))};
            if fs::symlink_metadata(&candidate).is_ok_and(|m|m.is_file()){
                return Some(files::canonical(&candidate).unwrap_or(candidate));
            }
        }
    }
    None
}
fn cli_state(value:&Value)->Value{
    let Some(raw)=value.as_str() else{return json!({"status":"not_configured"});};
    if raw.contains('/')||raw.contains('\\')||raw.contains(':'){
        return path_state(value);
    }
    if let Some(resolved)=resolve_command_on_path(raw){
        json!({"status":"ok","value":raw,"resolved":path_text(&resolved).unwrap_or_else(|_|raw.to_owned()),"kind":"command"})
    }else{
        json!({"status":"command_not_found","value":raw,"kind":"command"})
    }
}
fn path_state(value:&Value)->Value{
    let Some(raw)=value.as_str()else{return json!({"status":"not_configured"});};
    let p=Path::new(raw);
    if !p.is_absolute(){return json!({"status":"relative_path_rejected","value":raw});}
    match fs::symlink_metadata(p){
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>json!({"status":"missing","value":raw}),
        Err(e)=>json!({"status":"unreadable","value":raw,"reason":e.to_string()}),
        Ok(m)=>{
            let kind=if m.is_file(){"file"}else if m.is_dir(){"directory"}else{"other"};
            match files::canonical(p){
                Ok(actual)=>json!({"status":"ok","value":raw,"canonical":path_text(&actual).unwrap_or_else(|_|raw.to_owned()),"kind":kind}),
                Err(e)=>json!({"status":"unreadable","value":raw,"reason":format!("{e:#}")}),
            }
        }
    }
}
fn work_root_state(value:&Value,vault_root:Option<&str>)->Value{
    let Some(raw)=value.as_str() else{return json!({"status":"not_configured"});};
    let p=Path::new(raw);
    if !p.is_absolute(){return json!({"status":"relative_path_rejected","value":raw});}
    if looks_like_item_work_dir(p){return json!({"status":"item_subdir_rejected","value":raw});}
    if let Some(vr)=vault_root{
        let vp=Path::new(vr);
        if vp.is_absolute() && work_inside_vault(p,vp){
            return json!({"status":"inside_vault_rejected","value":raw});
        }
    }
    match fs::symlink_metadata(p){
        Ok(m) if !m.is_dir()=>json!({"status":"not_directory","value":raw}),
        Ok(_)=>path_state(value),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{
            let mut ancestor=p.parent();
            while let Some(a)=ancestor{
                match fs::symlink_metadata(a){
                    Ok(m) if m.is_dir()&&!files::is_linked(a,&m)=>return json!({"status":"creatable","value":raw,"existing_ancestor":path_text(a).ok()}),
                    Ok(_)=>return json!({"status":"unwritable_ancestor","value":raw}),
                    Err(e2) if e2.kind()==std::io::ErrorKind::NotFound=>{ancestor=a.parent();}
                    Err(e2)=>return json!({"status":"unreadable","value":raw,"reason":e2.to_string()}),
                }
            }
            json!({"status":"missing","value":raw})
        }
        Err(e)=>json!({"status":"unreadable","value":raw,"reason":e.to_string()}),
    }
}
fn validate_paths(profile:&Value)->Result<Value>{
    let paths=profile["paths"].as_object().context("profile paths")?;
    let vault_root=paths.get("obsidian_vault_root").and_then(Value::as_str);
    let mut rows=Map::new();let mut valid=true;
    for (key,value) in paths{
        let row=if key=="zotero_local_api"{
            let ok=value.as_str().is_some_and(|s|s.starts_with("http://127.0.0.1:")||s.starts_with("http://localhost:"));
            if !ok{valid=false;}
            json!({"status":if ok{"configured"}else{"invalid"},"value":value})
        }else if key=="obsidian_cli"{
            let row=cli_state(value);
            if !matches!(row["status"].as_str(),Some("ok"|"not_configured"|"command_not_found")){valid=false;}
            row
        }else if key=="work_root"{
            let row=work_root_state(value,vault_root);
            if !matches!(row["status"].as_str(),Some("ok"|"creatable"|"not_configured")){valid=false;}
            row
        }else{
            let row=path_state(value);
            if !row["status"].is_null()&&!matches!(row["status"].as_str(),Some("ok"|"not_configured")){valid=false;}
            row
        };
        rows.insert(key.clone(),row);
    }
    Ok(json!({"valid":valid,"paths":rows}))
}
pub fn locate(skill_root:Option<&Path>)->Result<Value>{
    let root=match skill_root{
        Some(p)=>files::canonical(p)?,
        None=>self::skill_root(None)?,
    };
    let p=root.join(RELATIVE);
    let exists=p.is_file();
    let writable=fs::symlink_metadata(&root).is_ok_and(|m|m.is_dir())&&!root.join(RELATIVE).is_symlink();
    Ok(json!({"schema_version":1,"status":if exists{"configured"}else{"uninitialized"},
        "skill_root":path_text(&root)?,"profile_path":path_text(&p)?,"exists":exists,"writable":writable,
        "scope":"skill_local","generic_user_directory_used":false}))
}
pub fn load(skill_root:Option<&Path>,verify_paths:bool)->Result<Value>{
    let loc=locate(skill_root)?;
    let p=path(&loc["profile_path"])?;
    if !p.is_file(){
        let mut profile=defaults();
        profile["onboarding"]=json!({"formula_table_enhancement_question":"pending"});
        return Ok(json!({"schema_version":1,"status":"uninitialized","profile":profile,
            "profile_path":loc["profile_path"],"onboarding_pending":true,"generic_user_directory_used":false}));
    }
    check_write::no_link(&p)?;
    ensure!(fs::metadata(&p)?.len()<=MAX_BYTES,"profile exceeds size budget");
    let(raw,b)=files::read_bound(&p)?;
    ensure!(raw.len() as u64<=MAX_BYTES,"profile exceeds size budget");
    let mut profile=crate::parse_json(std::str::from_utf8(&raw)?.trim_start_matches('\u{feff}'))?;
    validate(&profile)?;
    let defaults=defaults();
    for (k,v) in defaults.as_object().unwrap(){
        if k!="updated_at"&&profile.get(k).is_none(){profile[k]=v.clone();}
    }
    let validation=if verify_paths{validate_paths(&profile)?}else{json!({"valid":null,"paths":{}})};
    let pending=profile["preferences"]["formula_table_enhancement"]=="unasked";
    Ok(json!({"schema_version":1,"status":if validation["valid"]==false{"stale_paths"}else{"configured"},
        "profile":profile,"profile_binding":b,"profile_path":loc["profile_path"],
        "validation":validation,"onboarding_pending":pending,
        "next_step":if pending{"Ask once whether to install optional formula/table OCR enhancement; otherwise use the built-in PDF path."}else{"Reuse the recorded paths; do not search again unless a recorded path is missing or the user changes it."},
        "generic_user_directory_used":false}))
}
pub fn save(profile:Value,skill_root:Option<&Path>)->Result<Value>{
    validate(&profile)?;
    let loc=locate(skill_root)?;
    let p=path(&loc["profile_path"])?;
    let parent=p.parent().context("profile parent")?;
    match fs::symlink_metadata(parent){
        Ok(m)=>ensure!(m.is_dir()&&!files::is_linked(parent,&m),"Refusing a linked profile directory"),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>fs::create_dir_all(parent)?,
        Err(e)=>return Err(e.into()),
    }
    let _lock=check_write::lock_target(&p)?;
    let mut profile=profile;
    profile["updated_at"]=json!(chrono::Utc::now().to_rfc3339());
    let mut bytes=serde_json::to_vec_pretty(&profile)?;bytes.push(b'\n');
    ensure!(bytes.len() as u64<=MAX_BYTES,"profile exceeds size budget");
    pipeline::publish_replace(&p,&bytes)?;
    let(raw,b)=files::read_bound(&p)?;
    ensure!(raw==bytes&&b["sha256"]==files::digest(&bytes),"profile readback mismatch");
    let saved=crate::parse_json(std::str::from_utf8(&raw)?)?;
    Ok(json!({"schema_version":1,"status":"saved","profile":saved,"profile_binding":b,
        "profile_path":loc["profile_path"],"scope":"skill_local","generic_user_directory_used":false}))
}
pub fn patch(patch:Value,skill_root:Option<&Path>)->Result<Value>{
    ensure!(patch.is_object(),"profile patch must be an object");
    let current=if locate(skill_root)?.get("exists").and_then(Value::as_bool)==Some(true){
        load(skill_root,false)?["profile"].clone()
    }else{defaults()};
    let mut merged=current;
    merge(&mut merged,&patch);
    save(merged,skill_root)
}
pub fn arxiv_tex_enabled(profile:Option<&Value>)->bool{
    profile.and_then(|p|p["preferences"]["arxiv_tex_auto"].as_bool()).unwrap_or(true)
}
pub fn formula_table_mode(profile:Option<&Value>)->&str{
    profile.and_then(|p|p["preferences"]["formula_table_enhancement"].as_str()).unwrap_or("unasked")
}
pub fn ensure_profile_binding(profile:&Value)->Result<()>{
    validate(profile).map_err(|e|anyhow!("invalid persisted profile: {e:#}"))
}
fn slugify_item_dir(title:&str,key:&str)->String{
    let mut words=Vec::new();
    let mut cur=String::new();
    for ch in title.chars(){
        if ch.is_ascii_alphanumeric(){
            cur.push(ch.to_ascii_lowercase());
        }else if !cur.is_empty(){
            words.push(std::mem::take(&mut cur));
            if words.len()>=6{break;}
        }
    }
    if !cur.is_empty()&&words.len()<6{words.push(cur);}
    let mut slug=words.join("-");
    if slug.len()>48{slug.truncate(48);slug=slug.trim_end_matches('-').to_owned();}
    if slug.is_empty(){format!("item-{key}")}else{format!("{slug}-{key}")}
}
fn list_templates(template_root:Option<&str>)->Vec<String>{
    let Some(dir)=template_root.map(Path::new).filter(|p|p.is_dir()) else{return vec![];};
    let mut out=Vec::new();
    if let Ok(rd)=fs::read_dir(dir){
        for e in rd.flatten(){
            let p=e.path();
            if p.is_file()&&p.extension().and_then(|s|s.to_str()).is_some_and(|ext|ext.eq_ignore_ascii_case("md")){
                if let Some(name)=p.file_name().and_then(|s|s.to_str()){out.push(name.to_owned());}
            }
        }
    }
    out.sort();
    out
}
pub fn preflight(skill_root:Option<&Path>,query:Option<&str>,item_key:Option<&str>,storage_override:Option<&Path>)->Result<Value>{
    let loaded=load(skill_root,true)?;
    let profile=&loaded["profile"];
    let api_url=profile["paths"]["zotero_local_api"].as_str().unwrap_or("http://127.0.0.1:23119/api");
    let zotero_probe=zotero_local::probe_api(api_url);
    let available_templates=list_templates(profile["paths"]["template_root"].as_str());
    let lib_obj=profile.get("zotero_library").filter(|v|v.is_object())
        .or_else(||zotero_probe.get("library").filter(|v|v.is_object()));
    let lib_type=lib_obj.and_then(|v|v["type"].as_str()).unwrap_or("user");
    let lib_id=lib_obj.and_then(|v|v["id"].as_u64()).unwrap_or(0);
    let storage_owned=profile["paths"]["zotero_storage_dir"].as_str().map(PathBuf::from);
    let storage=storage_override.or(storage_owned.as_deref()).filter(|p|p.is_dir());
    let work_root=profile["paths"]["work_root"].as_str();
    let mut selected_key=item_key.map(|s|s.trim().to_owned()).filter(|s|!s.is_empty());
    let mut search_summary=Value::Null;
    if zotero_probe["reachable"]==true{
        if let Ok(addr)=zotero_local::parse_loopback_api_addr(api_url){
            if selected_key.is_none(){
                if let Some(q)=query.map(str::trim).filter(|s|!s.is_empty()){
                    match zotero_local::search_items_at(addr,lib_type,lib_id,zotero_local::SearchOptions{
                        query:q,qmode:"titleCreatorYear",item_type:None,top_only:true,limit:5,start:0,
                    }){
                        Ok(res)=>{
                            let hits:Vec<Value>=res["items"].as_array().into_iter().flatten().map(|it|json!({
                                "key":it["key"],"itemType":it["data"]["itemType"],"title":it["data"]["title"],
                                "date":it["data"]["date"],"collections":it["data"]["collections"],
                            })).collect();
                            if hits.len()==1{selected_key=hits[0]["key"].as_str().map(str::to_owned);}
                            search_summary=json!({"returned_count":res["returned_count"],"total_results":res["total_results"],"hits":hits});
                        }
                        Err(e)=>{search_summary=json!({"error":format!("{e:#}")});}
                    }
                }
            }
            if let Some(key)=selected_key.as_deref(){
                let inspected=zotero_local::inspect_item_at(addr,lib_type,lib_id,key,storage,&json!({}),|_,_,_|{});
                match inspected{
                    Ok(rep)=>{
                        let data=&rep["raw"]["data"];
                        let title=data["title"].as_str().unwrap_or("");
                        let item_type=data["itemType"].as_str().unwrap_or("");
                        let suggested_dir=work_root.map(|wr|path_text(&Path::new(wr).join(slugify_item_dir(title,key))).unwrap_or_default());
                        let arxiv_id=crate::tex_source::detect_arxiv_id(&rep["raw"],None);
                        let matched_tpl=profile["templates"].as_object().and_then(|m|m.get(item_type).and_then(Value::as_str).map(str::to_owned));
                        let atts:Vec<Value>=rep["attachments"].as_array().into_iter().flatten().map(|a|json!({
                            "key":a["key"],"link_mode":a["link_mode"],"declared_mime":a["declared_mime"],
                            "filename":a["raw"]["data"]["filename"],"local":a["local"],
                        })).collect();
                        return Ok(json!({
                            "schema_version":1,"status":loaded["status"],"onboarding_pending":loaded["onboarding_pending"],
                            "profile_path":loaded["profile_path"],"profile":profile,"validation":loaded["validation"],
                            "zotero_probe":zotero_probe,"available_templates":available_templates,"search":search_summary,
                            "item_inspection":{
                                "item_key":key,"itemType":item_type,"title":title,"date":data["date"],
                                "DOI":data["DOI"],"url":data["url"],"declared_language":rep["declared_language"],
                                "collection_ancestry":rep["collection_ancestry"],"arxiv_id":arxiv_id,
                                "selected_template":matched_tpl,"attachments":atts,
                                "local_version_consistent":rep["local_version_consistent"],"raw":rep["raw"],
                            },
                            "suggested_item_work_dir":suggested_dir,
                            "generic_user_directory_used":false,
                        }));
                    }
                    Err(e)=>{
                        return Ok(json!({
                            "schema_version":1,"status":loaded["status"],"onboarding_pending":loaded["onboarding_pending"],
                            "profile_path":loaded["profile_path"],"profile":profile,"validation":loaded["validation"],
                            "zotero_probe":zotero_probe,"available_templates":available_templates,"search":search_summary,
                            "item_inspection_error":format!("{e:#}"),
                            "generic_user_directory_used":false,
                        }));
                    }
                }
            }
        }
    }
    Ok(json!({
        "schema_version":1,"status":loaded["status"],"onboarding_pending":loaded["onboarding_pending"],
        "profile_path":loaded["profile_path"],"profile":profile,"validation":loaded["validation"],
        "zotero_probe":zotero_probe,"available_templates":available_templates,"search":search_summary,
        "generic_user_directory_used":false,
    }))
}

#[cfg(test)]
mod tests{
    use super::*;
    fn root()->tempfile::TempDir{
        let d=tempfile::tempdir().unwrap();
        fs::write(d.path().join("SKILL.md"),"---\nname: literature-zotero2obsidian\nversion: v0\n---\n").unwrap();
        d
    }
    #[test]
    fn profile_is_skill_local_atomic_and_round_trips(){
        let d=root();let skill=d.path();
        let first=load(Some(skill),true).unwrap();
        assert_eq!(first["status"],"uninitialized");assert_eq!(first["onboarding_pending"],true);
        assert!(first["profile_path"].as_str().unwrap().starts_with(&path_text(skill).unwrap()));
        let mut profile=first["profile"].clone();
        profile["paths"]["obsidian_vault_root"]=json!(path_text(skill).unwrap());
        profile["preferences"]["formula_table_enhancement"]=json!("native");
        let saved=save(profile,Some(skill)).unwrap();
        assert_eq!(saved["status"],"saved");
        let again=load(Some(skill),true).unwrap();
        assert_eq!(again["status"],"configured");
        assert_eq!(again["onboarding_pending"],false);
        assert_eq!(again["validation"]["paths"]["obsidian_vault_root"]["status"],"ok");
        assert_eq!(again["profile"]["preferences"]["arxiv_tex_auto"],true);
    }
    #[test]
    fn patch_updates_only_given_preferences_and_rejects_bad_values(){
        let d=root();let skill=d.path();
        patch(json!({"preferences":{"formula_table_enhancement":"ocr"}}),Some(skill)).unwrap();
        let row=load(Some(skill),false).unwrap();
        assert_eq!(row["profile"]["preferences"]["formula_table_enhancement"],"ocr");
        assert_eq!(row["profile"]["preferences"]["transparent_figure"],"preserve_original");
        assert!(patch(json!({"preferences":{"formula_table_enhancement":"always"}}),Some(skill)).is_err());
    }
    #[test]
    fn missing_recorded_path_is_reported_for_targeted_refresh(){
        let d=root();let skill=d.path();
        let mut p=defaults();
        p["paths"]["literature_root"]=json!(skill.join("missing-root").to_string_lossy());
        save(p,Some(skill)).unwrap();
        let row=load(Some(skill),true).unwrap();
        assert_eq!(row["status"],"stale_paths");
        assert_eq!(row["validation"]["paths"]["literature_root"]["status"],"missing");
    }
    #[test]
    fn cli_command_is_valid_and_item_subdir_work_root_is_rejected(){
        let d=root();let skill=d.path();
        let vault=skill.join("vault");fs::create_dir(&vault).unwrap();
        let mut p=defaults();
        p["paths"]["obsidian_vault_root"]=json!(path_text(&vault).unwrap());
        p["paths"]["obsidian_cli"]=json!("obsidian");
        let work_base=skill.join("work-root");fs::create_dir(&work_base).unwrap();
        p["paths"]["work_root"]=json!(path_text(&work_base).unwrap());
        save(p.clone(),Some(skill)).unwrap();
        let loaded=load(Some(skill),true).unwrap();
        assert_eq!(loaded["status"],"configured");
        assert_eq!(loaded["validation"]["valid"],true);
        assert!(matches!(loaded["validation"]["paths"]["obsidian_cli"]["status"].as_str(),Some("ok"|"command_not_found")));

        let mut bad=p.clone();
        bad["paths"]["work_root"]=json!(path_text(&work_base.join("persistent-mayer-5U5RW3TA")).unwrap());
        assert!(save(bad,Some(skill)).is_err());

        let mut inside=p;
        inside["paths"]["work_root"]=json!(path_text(&vault.join("work")).unwrap());
        assert!(save(inside,Some(skill)).is_err());
    }
}
