//! Real vault discovery and conservative reference identity candidates.
//! A note footer is a locator, not proof of its publication version.
use anyhow::{ensure,Context,Result};
use serde_json::{json,Value};
use std::{collections::{BTreeMap,BTreeSet},fs,path::{Path,PathBuf}};
use crate::{check_ledger,files,handoff_scan::Candidate,path,path_text,properties};
const LIMIT:u64=32*1024*1024;
const MAX_NOTES:usize=250_000;
const BOUNDARY:&str="Read-only vault note and Zotero-record identity candidates. Exact bibliographic metadata and the original-item URI are evidence, not proof of the note's full text or publication version. No note is created, rewritten, linked automatically, or deleted.";
fn text(v:&Value)->Option<&str>{v.as_str().filter(|s|!s.trim().is_empty())}
fn norm(s:&str)->String{
    use unicode_normalization::UnicodeNormalization;
    use unicode_casefold::UnicodeCaseFold;
    s.nfc().collect::<String>().case_fold().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}
fn portable(raw:&str)->Result<PathBuf>{
    ensure!(!raw.is_empty()&&!raw.contains(['\\',':','\0'])&&!raw.split('/').any(|s|s.is_empty()||matches!(s,"."|"..")),"Use explicit vault-relative POSIX path");
    let p=PathBuf::from(raw);ensure!(p.is_relative(),"Absolute vault path prohibited");Ok(p)
}
fn rel(root:&Path,p:&Path)->Result<String>{
    let r=p.strip_prefix(root)?;
    let rows=r.components().map(|c|match c{std::path::Component::Normal(v)=>v.to_str().context("Non-Unicode vault path"),_=>Err(anyhow::anyhow!("Invalid vault path"))}).collect::<Result<Vec<_>>>()?;
    Ok(rows.join("/"))
}
fn contained(root:&Path,p:&Path)->Result<PathBuf>{
    let p=files::canonical(p)?;ensure!(p.starts_with(root),"Resolved path escapes confirmed vault");Ok(p)
}
fn no_link(p:&Path)->Result<()>{
    let m=fs::symlink_metadata(p)?;
    ensure!(!m.file_type().is_symlink(),"Linked vault entry requires explicit review");
    #[cfg(windows)]{use std::os::windows::fs::MetadataExt;ensure!(m.file_attributes()&0x400==0,"Reparse point requires explicit review");}
    Ok(())
}
fn parse_uri(s:&str)->Option<(Option<u64>,String)>{
    let s=s.strip_prefix("zotero://select/")?;
    let (group,k)=if let Some(k)=s.strip_prefix("library/items/"){(None,k)}
        else{let s=s.strip_prefix("groups/")?;let (group,k)=s.split_once("/items/")?;(Some(group.parse::<u64>().ok().filter(|n|*n>0)?),k)};
    if k.len()!=8||!k.bytes().all(|b|b.is_ascii_uppercase()||b.is_ascii_digit()){return None;}
    Some((group,k.to_owned()))
}
fn inspect_note(p:&Path,root:&Path)->Result<Value>{
    no_link(p)?;ensure!(fs::metadata(p)?.len()<=LIMIT,"Note exceeds size budget");
    let (raw,b)=files::read_bound(p)?;ensure!(raw.len() as u64<=LIMIT,"Note exceeds size budget");
    let parts=properties::split(&raw)?;
    let meta=if let Some(payload)=parts.payload{properties::parse_yaml(payload)?}else{Value::Null};
    // The actual final item URI is enumerated outside fences/comments by
    // the native candidate scanner. Ignore cited Zotero links inside text.
    let c=Candidate::new(&raw)?;
    let mut locations=vec![];
    for l in &c.links{
        if l.image||!matches!(norm(&l.label).as_str(),"zotero 原条目"|"zotero原条目"){continue;}
        if let Some((group,k))=parse_uri(&l.href){
            let tail=c.raw[l.span.end..].trim();
            if !tail.is_empty(){continue;}
            locations.push(json!({"group_id":group,"item_key":k,"uri":l.href,"locator":l.locator}));
        }
    }
    ensure!(check_ledger::bind_file(p)?==b,"Note changed during inspection");
    Ok(json!({"relative_path":rel(root,p)?,"file":b,"metadata":meta,"identity_candidates":locations,
        "status":if locations.len()==1{"item_locator_found"}else if locations.is_empty(){"item_locator_missing"}else{"ambiguous_item_locator"},
        "publication_version":"not_assessed"}))
}
fn scan(root:&Path,literature:&Path,excluded:&[PathBuf],mut progress:impl FnMut(usize,usize,&str))->Result<(Vec<Value>,Vec<Value>)>{
    let mut stack=vec![literature.to_owned()];let mut paths=vec![];let mut issues=vec![];let mut count=0usize;
    while let Some(dir)=stack.pop(){
        for entry in fs::read_dir(&dir)?{
            let entry=entry?;let p=entry.path();count+=1;ensure!(count<=MAX_NOTES*10,"Vault traversal exceeds budget");
            if excluded.iter().any(|e|p.starts_with(e)){continue;}
            let name=entry.file_name().to_string_lossy().to_ascii_lowercase();
            if matches!(name.as_str(),".obsidian"|".trash"|".z2o-backups"|".git"|"template"|"templates"|"attachments"|"backups"|"drafts"){continue;}
            let ty=entry.file_type()?;
            if no_link(&p).is_err(){issues.push(json!({"path":path_text(&p)?,"reason":"linked_entry_not_scanned"}));continue;}
            if ty.is_dir(){stack.push(p);continue;}
            if ty.is_file()&&p.extension().and_then(|s|s.to_str()).is_some_and(|s|s.eq_ignore_ascii_case("md")){
                paths.push(p);ensure!(paths.len()<=MAX_NOTES,"Vault note count exceeds budget");
            }
        }
    }
    paths.sort();let n=paths.len();let mut rows=Vec::with_capacity(n);
    for(i,p)in paths.into_iter().enumerate(){
        match inspect_note(&p,root){
            Ok(v)=>rows.push(v),
            Err(e)=>issues.push(json!({"path":path_text(&p)?,"reason":format!("{e:#}")}))
        }
        if (i+1)%64==0||i+1==n{progress(i+1,n,"index actual library note identities");}
    }
    Ok((rows,issues))
}
pub fn index(vault:&Path,literature_relative:&str,excluded_relative:&Value,personal_library_id:Option<u64>,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    ensure!(personal_library_id.is_none_or(|id|id>0),"Confirmed personal library ID must not be the route alias zero");
    let root=files::canonical(vault)?;
    ensure!(root.is_dir()&&root.join(".obsidian").is_dir(),"Confirm an actual Obsidian vault root");
    let literature=contained(&root,&root.join(portable(literature_relative)?))?;
    ensure!(literature.is_dir(),"Literature directory required");
    let mut excluded=Vec::new();
    for raw in excluded_relative.as_array().context("Explicit excluded relative roots required")?{
        let p=root.join(portable(raw.as_str().context("Excluded relative root")?)?);
        let p=if p.exists(){contained(&root,&p)?}else{p};
        ensure!(p!=root&&p!=literature&&!literature.starts_with(&p),"Do not exclude whole literature root");
        excluded.push(p);
    }
    excluded.sort();excluded.dedup();
    let (notes,issues)=scan(&root,&literature,&excluded,&mut progress)?;
    Ok(json!({"schema_version":1,"vault_root":path_text(&root)?,"literature_root":path_text(&literature)?,
        "excluded_roots":excluded,"confirmed_personal_library_id":personal_library_id,"notes":notes,"issues":issues,"scan_complete":issues.is_empty(),
        "source_fidelity":"not_assessed","boundary":BOUNDARY}))
}
fn doi(s:&str)->String{
    let s=s.trim().to_ascii_lowercase();
    s.strip_prefix("https://doi.org/").or_else(||s.strip_prefix("http://doi.org/")).or_else(||s.strip_prefix("doi:")).unwrap_or(&s).trim().to_owned()
}
fn author_tokens(v:&Value)->Option<Vec<String>>{
    let rows=v.as_array()?;let mut result=vec![];
    for r in rows{
        let role=r["creatorType"].as_str()?;
        if !matches!(role,"author"|"inventor"){continue;}
        let name=if let Some(s)=text(&r["name"]){norm(s)}else{
            let last=text(&r["lastName"])?;let first=r["firstName"].as_str().unwrap_or("");
            norm(&format!("{first} {last}"))
        };
        if name.is_empty(){return None;}result.push(name);
    }
    (!result.is_empty()).then_some(result)
}
fn year(s:&str)->Option<String>{
    let r=regex::Regex::new(r"(?:^|[^0-9])([12][0-9]{3})(?:[^0-9]|$)").ok()?;
    let v=r.captures(s)?;Some(v[1].to_owned())
}
fn identity(v:&Value)->Option<(&str,u64,&str)>{
    let kind=v["library"]["type"].as_str()?;let id=v["library"]["id"].as_u64()?;let key=v["key"].as_str()?;
    (matches!(kind,"user"|"group")&&id>0&&key.len()==8&&v["data"]["key"]==key).then_some((kind,id,key))
}
/// Recheck the actual note and the actual bound raw Zotero data. Bibliographic
/// identity requires all confirmed source fields, including publication type.
/// Even unique exact candidates still require publication-version observation.
pub fn match_reference(index_path:&Path,index_sha:&str,catalog_path:&Path,catalog_sha:&str,reference:&Value)->Result<Value>{
    let bind=|p:&Path,hash:&str|->Result<(Value,Value)>{
        ensure!(fs::metadata(p)?.len()<=256*1024*1024,"Index/catalog exceeds input budget");
        let(raw,b)=files::read_bound(p)?;
        ensure!(raw.len()<=256*1024*1024&&b["sha256"]==hash,"Library evidence binding changed");
        Ok((crate::parse_json(std::str::from_utf8(&raw)?)?,b))
    };
    let (index,ib)=bind(index_path,index_sha)?;let (catalog,cb)=bind(catalog_path,catalog_sha)?;
    ensure!(index["schema_version"].as_u64()==Some(1)&&catalog["schema_version"].as_u64()==Some(1),"Unsupported library evidence schema");
    let root=files::canonical(&path(&index["vault_root"])?)?;
    let kind=catalog["library"]["type"].as_str().context("Actual catalog library kind required")?;
    let library_id=catalog["library"]["id"].as_u64().filter(|id|*id>0).context("Actual catalog library ID required")?;
    ensure!(matches!(kind,"user"|"group"),"Unsupported catalog library");
    if kind=="user"{ensure!(index["confirmed_personal_library_id"].as_u64()==Some(library_id),"Confirm the personal Zotero library associated with the vault; route alias 0 is not identity");}
    let title=norm(text(&reference["title"]).context("Actual original source title required")?);
    let date=text(&reference["year"]).context("Actual source year string required")?;
    ensure!(date.len()==4&&date.bytes().all(|b|b.is_ascii_digit()),"Original four-digit year required");
    let item_type=text(&reference["item_type"]).context("Explicit original publication type required")?;
    let authors=reference["authors"].as_array().filter(|v|!v.is_empty()&&v.iter().all(|v|text(v).is_some())).context("Original full author names required")?
        .iter().map(|v|norm(v.as_str().unwrap())).collect::<Vec<_>>();
    let expected_doi=text(&reference["doi"]).map(doi);
    let mut by_key=BTreeMap::<(Option<u64>,String),Vec<&Value>>::new();
    for r in catalog["items"].as_array().context("Actual raw Zotero catalog required")?{
        let Some((record_kind,id,k))=identity(r)else{anyhow::bail!("Invalid raw catalog object");};
        ensure!(record_kind==kind&&id==library_id,"Catalog contains another library");
        by_key.entry((if kind=="group"{Some(id)}else{None},k.to_owned())).or_default().push(r);
    }
    let mut candidates=vec![];let mut unidentified=0usize;let mut missing_catalog_record=0usize;
    for n in index["notes"].as_array().context("Actual indexed note list required")?{
        let loc=&n["identity_candidates"];
        let Some(loc)=loc.as_array().filter(|v|v.len()==1).and_then(|v|v.first())else{unidentified+=1;continue;};
        let key=(loc["group_id"].as_u64(),loc["item_key"].as_str().context("Indexed key")?.to_owned());
        if key.0!=(if kind=="group"{Some(library_id)}else{None}){continue;}
        let Some(rows)=by_key.get(&key)else{missing_catalog_record+=1;continue;};
        for r in rows{
            let data=&r["data"];
            let d=text(&data["DOI"]).map(doi);
            let same_doi=expected_doi.as_ref().is_some_and(|s|d.as_ref()==Some(s));
            let same_title=text(&data["title"]).is_some_and(|s|norm(s)==title);
            if !same_doi&&!same_title{continue;}
            let same_author=author_tokens(&data["creators"]).as_ref()==Some(&authors);
            let same_year=text(&data["date"]).and_then(year).as_deref()==Some(date);
            let same_type=data["itemType"]==item_type;
            let full=same_title&&same_author&&same_year&&same_type&&expected_doi.as_ref().is_none_or(|s|d.as_ref()==Some(s));
            let relative=n["relative_path"].as_str().context("Indexed note relative path")?;
            let p=contained(&root,&root.join(portable(relative)?))?;
            let current=inspect_note(&p,&root)?;
            ensure!(current["file"]==n["file"]&&current["identity_candidates"]==n["identity_candidates"],"Indexed note changed; refresh library index");
            candidates.push(json!({"relative_path":relative,"note":current["file"],"original_item_locator":loc,
                "raw_zotero_record":r,"same_doi":same_doi,"same_title":same_title,"same_authors":same_author,
                "same_year":same_year,"same_item_type":same_type,"all_bibliographic_fields_equal":full,
                "publication_version_review":"pending"}));
        }
    }
    let complete=index["scan_complete"]==true&&catalog["pagination_complete"]==true&&catalog["local_version_consistent"]==true&&unidentified==0&&missing_catalog_record==0;
    let status=if candidates.is_empty(){if complete{"absent_in_bound_snapshot"}else{"unavailable"}}
        else if candidates.len()!=1||candidates[0]["all_bibliographic_fields_equal"]!=true{"ambiguous"}
        else{"unique_bibliographic_candidate_requires_version_review"};
    ensure!(check_ledger::bind_file(index_path)?==ib&&check_ledger::bind_file(catalog_path)?==cb,"Library evidence changed");
    Ok(json!({"schema_version":1,"index":ib,"catalog":cb,"reference":reference,"status":status,"candidates":candidates,
        "scope_complete_at_capture":complete,"live_index_freshness":"not_assessed","unidentified_note_count":unidentified,"missing_catalog_record_count":missing_catalog_record,"match_confirmed":false,"source_fidelity":"not_assessed","boundary":BOUNDARY}))
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn exact_footer_not_cited_zotero_uri(){
        let root=tempfile::tempdir().unwrap();let p=root.path().join("note.md");
        fs::write(&p,"---\n年份: 2020\n---\n[Zotero 原条目](zotero://select/library/items/OTHER001)\n\n[Zotero 原条目](zotero://select/library/items/ACTUAL01)\n").unwrap();
        let r=inspect_note(&p,root.path()).unwrap();
        assert_eq!(r["identity_candidates"].as_array().unwrap().len(),1);
        assert_eq!(r["identity_candidates"][0]["item_key"],"ACTUAL01");
        assert_eq!(parse_uri("zotero://select/groups/10/items/ACTUAL01"),Some((Some(10),"ACTUAL01".into())));
    }
}
