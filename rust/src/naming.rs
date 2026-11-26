//! Portable naming with independently checked source bindings.
use crate::{path,path_text,string,files};
use anyhow::{anyhow,ensure,Context,Result};
use serde_json::{json,Map,Value};
use std::{collections::{BTreeSet,HashMap},fs,path::{Path,PathBuf}};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

const ILLEGAL:&str="<>:\"/\\|?*";
const AMBIGUOUS:&str="[]#^|";
fn units(s:&str)->usize{s.encode_utf16().count()}
fn control(ch:char)->bool{ch.is_control()}
fn clean(s:&str,field:&str)->(String,Vec<Value>){
    let mut text=String::with_capacity(s.len());
    let mut changes=vec![];
    for (i,ch) in s.chars().enumerate(){
        if !ILLEGAL.contains(ch)&&!control(ch){text.push(ch);continue;}
        text.push('_');
        changes.push(json!({"field":field,"operation":"replace_illegal_character","start":i,"end":i+1,
            "unit":"unicode_code_points","before":ch.to_string(),"after":"_",
            "reason":if control(ch){"control_character"}else{"portable_filename_character"}}));
    }
    (text,changes)
}
fn prefix(text:&str,remaining:usize,remaining_bytes:Option<usize>)->String{
    let mut consumed=0;
    let mut size=0;
    for (i,ch) in text.char_indices(){
        consumed+=ch.len_utf16();size+=ch.len_utf8();
        if consumed>remaining||remaining_bytes.is_some_and(|n|size>n){return text[..i].to_owned();}
    }
    text.to_owned()
}
fn reserved(s:&str)->bool{
    let upper=s.split('.').next().unwrap_or("").trim_end_matches([' ','.']).to_uppercase();
    if ["CON","PRN","AUX","NUL","CONIN$","CONOUT$"].contains(&upper.as_str()){return true;}
    if let Some(rest)=upper.strip_prefix("COM").or_else(||upper.strip_prefix("LPT")){
        return rest.chars().count()==1&&"123456789¹²³".contains(rest);
    }
    false
}
fn positive(v:&Value,name:&str)->Result<usize>{
    let n=v.as_u64().filter(|n|*n>0).ok_or_else(||anyhow!("{name} must be a positive integer"))?;
    usize::try_from(n).context("integer exceeds platform range")
}
fn limit(v:&Value,name:&str)->Result<Option<usize>>{
    if v.is_null(){Ok(None)}else{Ok(Some(positive(v,name)?))}
}
fn take(values:&mut Map<String,Value>,key:&str)->Result<Value>{
    values.remove(key).ok_or_else(||anyhow!("missing argument: {key}"))
}
fn opt(values:&mut Map<String,Value>,key:&str)->Value{values.remove(key).unwrap_or(Value::Null)}
fn planned(p:&Path)->Result<PathBuf>{
    let absolute=if p.is_absolute(){p.to_path_buf()}else{std::env::current_dir()?.join(p)};
    fn existing_prefix(p:&Path)->Result<PathBuf>{
        match dunce::canonicalize(p){
            Ok(path)=>Ok(path),
            Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{
                let name=p.file_name().ok_or_else(||anyhow!("cannot resolve planned path: {}",p.display()))?;
                let parent=p.parent().ok_or_else(||anyhow!("cannot resolve planned parent"))?;
                Ok(existing_prefix(parent)?.join(name))
            }
            Err(e)=>Err(e.into())
        }
    }
    existing_prefix(&absolute)
}
fn optional_path(v:&Value)->Result<Option<PathBuf>>{
    if v.is_null(){Ok(None)}else{Ok(Some(planned(&path(v)?)?))}
}
fn path_value(p:&Option<PathBuf>)->Result<Value>{
    Ok(match p {Some(p)=>json!(path_text(p)?),None=>Value::Null})
}
fn evidence(record:&Value,cache:&mut HashMap<PathBuf,String>)->Result<Value>{
    let locator=record.get("locator").ok_or_else(||anyhow!("source evidence requires a locator"))?;
    ensure!(locator.as_str().is_some_and(|s|!s.trim().is_empty())||
        locator.as_object().is_some_and(|m|!m.is_empty()),"source evidence requires a nonempty locator");
    let file=record.get("file").ok_or_else(||anyhow!("source evidence requires file evidence"))?;
    let expected=string(&file["sha256"],"sha256")?;
    ensure!(expected.len()==64&&expected.bytes().all(|b|b.is_ascii_hexdigit()),"invalid SHA-256");
    let p=files::canonical(&path(&file["path"])?)?;
    let hash=if let Some(hash)=cache.get(&p){hash.clone()}else{
        let current=files::snapshot(&p)?;
        let hash=string(&current["sha256"],"sha256")?;
        cache.insert(p.clone(),hash.clone());hash
    };
    ensure!(hash==expected.to_ascii_lowercase(),"evidence file SHA-256 mismatch");
    Ok(json!({"file":{"path":path_text(&p)?,"sha256":hash},"locator":locator}))
}
fn chars(s:&str,predicate:impl Fn(char)->bool)->Vec<String>{
    s.chars().filter(|ch|predicate(*ch)).collect::<BTreeSet<_>>().into_iter().map(|ch|ch.to_string()).collect()
}
pub fn build_plan(mut a:Map<String,Value>)->Result<Value>{
    let year=take(&mut a,"year")?;
    let author=string(&take(&mut a,"author_token")?,"author_token")?;
    let title=string(&take(&mut a,"original_title")?,"original_title")?;
    let source=take(&mut a,"source_evidence")?;
    let delivery=optional_path(&opt(&mut a,"delivery_path"))?;
    let old=optional_path(&opt(&mut a,"old_path"))?;
    let target_dir=optional_path(&opt(&mut a,"target_directory"))?;
    let no_year=opt(&mut a,"no_year_reason");
    let unknown_author=opt(&mut a,"unknown_author_reason");
    let suffix=opt(&mut a,"disambiguation_suffix");
    let suffix_source=opt(&mut a,"suffix_evidence");
    let maximum=positive(&a.remove("max_utf16_units").unwrap_or(json!(250)),"max_utf16_units")?;
    let byte_limit=limit(&opt(&mut a,"filename_max_utf8_bytes"),"filename_max_utf8_bytes")?;
    let path_limit=limit(&opt(&mut a,"full_path_max_utf16_units"),"full_path_max_utf16_units")?;
    if !a.is_empty(){
        let mut extra:Vec<_>=a.keys().map(String::as_str).collect();
        extra.sort_unstable();
        anyhow::bail!("unknown naming arguments: [{}]; run `z2o describe format_helpers.build_naming_plan` for accepted parameters",extra.join(", "));
    }
    ensure!(maximum<=250,"max_utf16_units must not exceed 250");
    ensure!(path_limit.is_none()||target_dir.is_some(),"a full-path limit requires target_directory");
    let year_token=if year.is_null(){string(&no_year,"no_year_reason")?;"无年份".to_owned()}else{
        let n=positive(&year,"year")?;ensure!(n<=9999,"year must contain at most four digits");n.to_string()
    };
    if author=="未知作者"{string(&unknown_author,"unknown_author_reason")?;}
    let mut cache=HashMap::new();
    let mut sources=Map::new();
    for field in ["year","author_token","original_title"]{
        sources.insert(field.into(),evidence(&source[field],&mut cache)?);
    }
    let mut checked_suffix=Value::Null;
    let suffix_text=if suffix.is_null(){
        ensure!(suffix_source.is_null(),"suffix_evidence requires disambiguation_suffix");
        String::new()
    }else{
        let value=string(&suffix,"disambiguation_suffix")?;
        ensure!(value.split('-').all(|p|!p.is_empty()&&p.bytes().all(|b|b.is_ascii_alphanumeric())),
            "disambiguation_suffix must be an explicit ASCII token");
        checked_suffix=evidence(&suffix_source,&mut cache)?;
        format!("-{value}")
    };
    let (clean_author,mut changes)=clean(&author,"author_token");
    let (clean_title,title_changes)=clean(&title,"original_title");changes.extend(title_changes);
    let pre=format!("{year_token}-{clean_author}-");
    let end=format!("{suffix_text}.md");
    let fixed=units(&pre)+units(&end);
    let fixed_bytes=pre.len()+end.len();
    let used=prefix(&clean_title,maximum.saturating_sub(fixed),byte_limit.map(|n|n.saturating_sub(fixed_bytes)));
    let filename=if used.is_empty(){None}else{Some(format!("{pre}{used}{end}"))};
    let mut issues=vec![];
    if used!=clean_title{
        changes.push(json!({"field":"original_title","operation":"truncate_title_suffix","start":used.chars().count(),
            "end":clean_title.chars().count(),"unit":"unicode_code_points","before":&clean_title[used.len()..],
            "after":"","reason":"declared_filename_length_limit"}));
    }
    if filename.is_none(){
        issues.push(json!({"code":"fixed_prefix_or_suffix_leaves_no_title_space","severity":"block",
            "reason":"The year/author prefix, suffix and extension are not shortened; no whole title character fits."}));
    }
    let ambiguous=chars(filename.as_deref().unwrap_or(""),|ch|AMBIGUOUS.contains(ch));
    if !ambiguous.is_empty(){issues.push(json!({"code":"link_ambiguity","severity":"review","characters":ambiguous}));}
    let target=target_dir.as_ref().zip(filename.as_ref()).map(|(dir,name)|dir.join(name));
    let target_text=path_value(&target)?;
    let path_units=target_text.as_str().map(units);
    let within=path_units.zip(path_limit).map(|(n,limit)|n<=limit);
    if within==Some(false){issues.push(json!({"code":"full_path_exceeds_declared_limit","severity":"block"}));}
    let blocked=issues.iter().any(|v|v["severity"]=="block");
    Ok(json!({
        "schema_version":1,"status":if blocked{"blocked"}else{"planned"},
        "year":year,"year_token":year_token,"no_year_reason":no_year,
        "author_token":author,"unknown_author_reason":unknown_author,
        "original_title":title,"source_evidence":sources,"old_path":path_value(&old)?,
        "filename":filename,"delivery_path":path_value(&delivery)?,"target_directory":path_value(&target_dir)?,"target_path":target_text,
        "disambiguation_suffix":suffix,"suffix_evidence":checked_suffix,"transformations":changes,
        "title_mapping":{"original":title,"cleaned":clean_title,"used":used,"truncated":used!=clean_title},
        "author_mapping":{"original":author,"cleaned":clean_author},
        "length":{"max_utf16_units":maximum,"filename_utf16_units":filename.as_deref().map(units),
            "fixed_prefix_suffix_utf16_units":fixed,"filename_max_utf8_bytes":byte_limit,
            "filename_utf8_bytes":filename.as_ref().map(|s|s.len())},
        "character_checks":{"illegal_characters":filename.as_ref().map(|s|chars(s,|ch|ILLEGAL.contains(ch))),
            "control_characters":filename.as_ref().map(|s|chars(s,control)),
            "trailing_dot":filename.as_ref().map(|s|s.ends_with('.')),
            "trailing_space":filename.as_ref().map(|s|s.ends_with(' ')),
            "reserved_name":filename.as_deref().map(reserved),"link_ambiguity_characters":ambiguous,
            "scope":"complete filename including .md; legal title punctuation is preserved"},
        "path_check":{"status":if within.is_some(){"compared_to_declared_limit"}else{"not_assessed"},
            "utf16_units":path_units,"declared_max_utf16_units":path_limit,"within_declared_limit":within,
            "boundary":"Does not discover filesystem or tool limits, or certify every target-path restriction."},
        "local_conflicts":{"status":"not_checked"},"library_conflicts":{"status":"not_checked"},
        "identity_review":{"status":"not_assessed"},"semantic_review":{"status":"not_assessed"},
        "overwrite_authorized":false,"issues":issues
    }))
}

pub fn conflicts(filename:&str,root:&Path,case_sensitive:bool,normalization:&str)->Result<Value>{
    ensure!(!filename.contains(['/','\\','\0'])&&![".",".."].contains(&filename),"filename must be one directory-entry name");
    ensure!(["none","NFC","NFD"].contains(&normalization),"unicode_normalization must be none, NFC or NFD");
    let root=files::canonical(root)?;
    ensure!(root.is_dir(),"local comparison directory does not exist");
    let comparable=|name:&str|{
        let value:String=match normalization{"NFC"=>name.nfc().collect(),"NFD"=>name.nfd().collect(),_=>name.into()};
        if case_sensitive{value}else{value.as_str().case_fold().collect()}
    };
    let mut names=vec![];
    for entry in fs::read_dir(&root)?{
        names.push(entry?.file_name().into_string().map_err(|_|anyhow!("non-Unicode directory entry"))?);
    }
    names.sort();
    let wanted=comparable(filename);
    let mut matches=vec![];
    for name in &names {
        if comparable(name)==wanted{matches.push(json!({"name":name,"path":path_text(&root.join(name))?}));}
    }
    let hash=files::digest(&serde_json::to_vec(&names)?);
    Ok(json!({"status":if matches.is_empty(){"clear_in_directory_snapshot"}else{"name_collision"},
        "directory":path_text(&root)?,"filename":filename,"entries":names,"entry_names_sha256":hash,
        "matches":matches,"comparison":{"case_sensitive":case_sensitive,"unicode_normalization":normalization,
        "case_insensitive_algorithm":"Unicode casefold","policy_origin":"caller"},
        "scope":"nonrecursive names only; entry contents, identities and other directories are not checked",
        "identity_review":{"status":"not_assessed"},"library_conflicts":{"status":"not_checked"},"overwrite_authorized":false}))
}


/// Recompute a serialized current-schema plan from source files. Older
/// ambiguous path records stay unresolved; never infer missing limits.
pub fn validate_plan(plan:&Value,delivery:&Value)->Result<Value>{
    let mut findings=Vec::new();
    let mut verified=Vec::new();
    let mut recomputed=false;
    if let Some(obj)=plan.as_object(){
        let fields=["year","author_token","original_title","source_evidence","delivery_path","old_path",
            "target_directory","no_year_reason","unknown_author_reason","disambiguation_suffix","suffix_evidence"];
        let mut args=Map::new();
        for key in fields {
            if let Some(value)=obj.get(key){args.insert(key.into(),value.clone());}
            else{findings.push(json!({"code":"NAMING_PLAN_FIELD_MISSING","field":format!("/{key}"),"message":"required current-schema input missing"}));}
        }
        for (key,parent,child) in [("max_utf16_units","length","max_utf16_units"),
            ("filename_max_utf8_bytes","length","filename_max_utf8_bytes"),
            ("full_path_max_utf16_units","path_check","declared_max_utf16_units")]{
            if let Some(value)=obj.get(parent).and_then(|p|p.get(child)){args.insert(key.into(),value.clone());}
            else{findings.push(json!({"code":"NAMING_PLAN_FIELD_MISSING","field":format!("/{parent}/{child}"),"message":"required declared limit missing"}));}
        }
        if !delivery.is_null(){
            let actual=optional_path(delivery)?;
            let recorded=optional_path(&plan["delivery_path"])?;
            if actual!=recorded{
                findings.push(json!({"code":"NAMING_PLAN_DELIVERY_MISMATCH","field":"/delivery_path",
                    "message":"recorded delivery path differs from independently supplied output path"}));
            }
        }
        if findings.is_empty(){
            match build_plan(args){
                Ok(rebuilt)=>{
                    recomputed=true;
                    verified.extend(["year","author_token","original_title"]);
                    if !plan["disambiguation_suffix"].is_null(){verified.push("disambiguation_suffix");}
                    let mut compare=|key:&str,code:&str|{
                        if plan.get(key)!=rebuilt.get(key){
                            findings.push(json!({"code":code,"field":format!("/{key}"),"message":"recorded value differs from native recomputation"}));
                        }
                    };
                    for (key,code) in [
                        ("schema_version","NAMING_PLAN_SCHEMA"),("year_token","NAMING_PLAN_MAPPING_MISMATCH"),
                        ("title_mapping","NAMING_PLAN_MAPPING_MISMATCH"),("author_mapping","NAMING_PLAN_MAPPING_MISMATCH"),
                        ("transformations","NAMING_PLAN_TRANSFORMATION_MISMATCH"),("filename","NAMING_PLAN_FILENAME_MISMATCH"),
                        ("length","NAMING_PLAN_LENGTH_MISMATCH"),("character_checks","NAMING_PLAN_CHARACTER_CHECK_MISMATCH"),
                        ("path_check","NAMING_PLAN_PATH_MISMATCH"),("target_path","NAMING_PLAN_PATH_MISMATCH"),
                        ("target_directory","NAMING_PLAN_PATH_MISMATCH"),("delivery_path","NAMING_PLAN_DELIVERY_MISMATCH"),
                        ("old_path","NAMING_PLAN_PATH_MISMATCH"),("status","NAMING_PLAN_STATUS_MISMATCH"),("issues","NAMING_PLAN_ISSUES_MISMATCH")]{
                        compare(key,code);
                    }
                }
                Err(error)=>findings.push(json!({"code":"NAMING_PLAN_INPUT_INVALID","field":"/source_evidence","message":error.to_string()})),
            }
        }
    }else{findings.push(json!({"code":"NAMING_PLAN_SHAPE","field":"","message":"plan must be a mapping"}));}
    Ok(json!({"structurally_consistent":findings.is_empty(),"plan_status":plan.get("status"),
        "recomputed":recomputed,"validated_source_fields":verified,"findings":findings,
        "semantic_review":{"status":"not_assessed"},"library_conflicts":{"status":"not_assessed"},
        "authorization_granted":false,"boundary":"Current-schema record consistency and file bindings only; no metadata identity or write authorization."}))
}
