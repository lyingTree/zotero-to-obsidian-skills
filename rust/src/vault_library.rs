//! Real vault discovery and conservative reference identity candidates.
//! A note footer is a locator, not proof of its publication version.
use anyhow::{ensure,Context,Result};
use serde_json::{json,Value};
use std::{borrow::Cow,collections::{BTreeMap,BTreeSet},fs,path::{Path,PathBuf}};
use crate::{check_ledger,files,path,path_text,properties,syntax};
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
    ensure!(!files::is_linked(p,&m),"Linked vault entry requires explicit review");
    Ok(())
}
fn parse_uri(s:&str)->Option<(Option<u64>,String)>{
    let s=s.strip_prefix("zotero://select/")?;
    let (group,k)=if let Some(k)=s.strip_prefix("library/items/"){(None,k)}
        else{let s=s.strip_prefix("groups/")?;let (group,k)=s.split_once("/items/")?;(Some(group.parse::<u64>().ok().filter(|n|*n>0)?),k)};
    if k.len()!=8||!k.bytes().all(|b|b.is_ascii_uppercase()||b.is_ascii_digit()){return None;}
    Some((group,k.to_owned()))
}
fn fence(s:&str)->Option<(u8,usize,&str)>{
    let body=s.trim_start_matches(' ');if s.len()-body.len()>3{return None;}
    let first=*body.as_bytes().first()?;
    if first!=b'~'&&first!=96{return None;}
    let n=body.as_bytes().iter().take_while(|v|**v==first).count();
    (n>=3).then(||(first,n,body[n..].trim()))
}
fn blank_bytes(data:&mut[u8],a:usize,b:usize){
    for c in &mut data[a..b]{if !matches!(*c,b'\r'|b'\n'){*c=b' ';}}
}
fn mask_line_comments<'a>(line:&'a str,state:&mut Option<&'static str>)->Cow<'a,str>{
    if state.is_none()&&!line.contains("<!--")&&!line.contains("%%"){return Cow::Borrowed(line);}
    let mut buf=line.as_bytes().to_vec();
    let mut i=0usize;
    let b=line.len();
    while i<b{
        if let Some(end)=*state{
            if let Some(n)=line[i..b].find(end){let n=i+n+end.len();blank_bytes(&mut buf,i,n);i=n;*state=None;}
            else{blank_bytes(&mut buf,i,b);break;}
        }else{
            let x=line[i..b].find("<!--").map(|n|(i+n,4,"-->"));
            let y=line[i..b].find("%%").map(|n|(i+n,2,"%%"));
            let Some((at,n,end))=(match(x,y){(Some(a),Some(b))=>Some(if a.0<b.0{a}else{b}),(a,b)=>a.or(b)})else{break;};
            if syntax::escaped(line,at){i=at+n;continue;}
            blank_bytes(&mut buf,at,at+n);i=at+n;*state=Some(end);
        }
    }
    Cow::Owned(String::from_utf8(buf).expect("equal-byte mask"))
}
fn collect_delimiter_spans(prose_lines:&[(usize,Cow<'_,str>)],mark:u8,masked:&[(usize,usize)])->Vec<(usize,usize)>{
    let mut ranges=Vec::new();
    let mut open:Option<(usize,usize)>=None;
    let mut mask_idx=0usize;
    for (line_start,line) in prose_lines{
        let bytes=line.as_bytes();
        if !bytes.contains(&mark){continue;}
        let mut i=0usize;
        while i<bytes.len(){
            let g=line_start+i;
            while mask_idx<masked.len()&&masked[mask_idx].1<=g{mask_idx+=1;}
            if mask_idx<masked.len()&&masked[mask_idx].0<=g{
                let end_in_line=(masked[mask_idx].1-line_start).min(bytes.len());
                i=end_in_line.max(i+1);
                continue;
            }
            if bytes[i]!=mark||syntax::escaped(line,i){i+=1;continue;}
            let a=i;
            while i<bytes.len()&&bytes[i]==mark{
                let gi=line_start+i;
                while mask_idx<masked.len()&&masked[mask_idx].1<=gi{mask_idx+=1;}
                if mask_idx<masked.len()&&masked[mask_idx].0<=gi{break;}
                i+=1;
            }
            let n=i-a;
            if let Some((b,m))=open{
                if n==m{ranges.push((b,line_start+i));open=None;}
            }else{
                open=Some((line_start+a,n));
            }
        }
    }
    ranges
}
fn in_delimited_span(prose_lines:&[(usize,Cow<'_,str>)],target_byte:usize,mark:u8)->bool{
    let masked=if mark==b'$'{collect_delimiter_spans(prose_lines,96,&[])}else{vec![]};
    let spans=collect_delimiter_spans(prose_lines,mark,&masked);
    spans.partition_point(|&(a,_)|a<=target_byte).checked_sub(1).is_some_and(|idx|target_byte<spans[idx].1)
}
fn any_delimited(prose_lines:&[(usize,Cow<'_,str>)],targets:&[usize])->Vec<bool>{
    let backticks=collect_delimiter_spans(prose_lines,96,&[]);
    let dollars=collect_delimiter_spans(prose_lines,b'$',&backticks);
    targets.iter().map(|&pos|{
        let in_bt=backticks.partition_point(|&(a,_)|a<=pos).checked_sub(1).is_some_and(|idx|pos<backticks[idx].1);
        let in_dl=dollars.partition_point(|&(a,_)|a<=pos).checked_sub(1).is_some_and(|idx|pos<dollars[idx].1);
        in_bt||in_dl
    }).collect()
}
fn find_zotero_footers(bytes:&[u8])->Result<Vec<Value>>{
    let text=std::str::from_utf8(bytes)?;
    let raw=text.strip_prefix('\u{feff}').unwrap_or(text);
    let rows=syntax::lines(raw);
    let trimmed_end=raw.trim_end().len();
    let mut front=rows.first().is_some_and(|r|syntax::body(raw,*r).trim()=="---");
    let mut opened:Option<(u8,usize)>=None;
    let mut comment:Option<&'static str>=None;
    let mut prose_lines:Vec<(usize,Cow<'_,str>)>=Vec::with_capacity(rows.len());
    let mut prior_headings:Vec<(usize,Value)>=Vec::new();
    let mut final_locator:Option<(usize,Value)>=None;
    for(i,&r)in rows.iter().enumerate(){
        let body=syntax::body(raw,r);
        if front{
            if i>0&&body.trim()=="---"{front=false;}
            continue;
        }
        if let Some((mark,count))=opened{
            if fence(body).is_some_and(|(m,n,x)|m==mark&&n>=count&&x.is_empty()){opened=None;}
            continue;
        }
        let safe=mask_line_comments(body,&mut comment);
        if let Some((mark,n,_))=fence(&safe){opened=Some((mark,n));continue;}
        let indent=safe.chars().take_while(|c|matches!(c,' '|'\t')).fold(0usize,|n,c|if c=='\t'{n+4-n%4}else{n+1});
        if indent>=4{continue;}
        let has_candidate=safe.contains("](")&&safe.contains("select");
        prose_lines.push((r.start,safe));
        if !has_candidate{continue;}
        let safe_ref=prose_lines.last().unwrap().1.as_ref();
        for l in syntax::scan_links(safe_ref){
            if l.image{continue;}
            let span_start=r.start+l.start;
            let span_end=r.start+l.end;
            let label=&raw[r.start+l.label_start..r.start+l.label_end];
            if !matches!(norm(label).as_str(),"zotero 原条目"|"zotero原条目"){continue;}
            let Some((group,k))=parse_uri(&l.href)else{continue;};
            let prefix=&raw[r.start..span_start];
            let heading=prefix.strip_prefix("##").is_some_and(|rest|!rest.is_empty()&&rest.bytes().all(|b|matches!(b,b' '|b'\t')));
            if !heading{
                if !prefix.is_empty(){continue;}
                let previous=&raw[..r.start];
                let previous=previous.strip_suffix("\r\n").or_else(||previous.strip_suffix(['\r','\n'])).unwrap_or(previous);
                if !previous.rsplit(['\r','\n']).next().unwrap_or_default().trim().is_empty(){continue;}
            }
            let same_line=&raw[span_end..r.body_end];
            if !same_line.bytes().all(|b|matches!(b,b' '|b'\t')){continue;}
            if span_end==trimmed_end||heading{
                let snippet=&raw[span_start..span_end];
                let start_cp=raw[..span_start].chars().count();
                let end_cp=start_cp+snippet.chars().count();
                let locator=json!({"group_id":group,"item_key":k,"uri":l.href,"locator":{"kind":"characters","unit":"unicode_code_points","start":start_cp,"end":end_cp,"sha256":files::digest(snippet.as_bytes())}});
                if span_end==trimmed_end{final_locator=Some((span_start,locator));}
                else{prior_headings.push((span_start,locator));}
            }
        }
    }
    ensure!(!front&&opened.is_none()&&comment.is_none(),"MAP_UNCLOSED_MARKUP: finish frontmatter/fences/comments");
    let Some((final_start,locator))=final_locator else{return Ok(vec![]);};
    if prior_headings.is_empty(){
        if in_delimited_span(&prose_lines,final_start,96)||in_delimited_span(&prose_lines,final_start,b'$'){return Ok(vec![]);}
        return Ok(vec![locator]);
    }
    let mut positions:Vec<usize>=prior_headings.iter().map(|(pos,_)|*pos).collect();
    positions.push(final_start);
    let hidden=any_delimited(&prose_lines,&positions);
    if *hidden.last().unwrap(){return Ok(vec![]);}
    let mut out:Vec<Value>=prior_headings.into_iter().zip(hidden).filter_map(|((_,loc),is_hidden)|(!is_hidden).then_some(loc)).collect();
    out.push(locator);
    Ok(out)
}
fn inspect_note(p:&Path,root:&Path)->Result<Value>{
    no_link(p)?;ensure!(fs::metadata(p)?.len()<=LIMIT,"Note exceeds size budget");
    let (raw,b)=files::read_bound(p)?;ensure!(raw.len() as u64<=LIMIT,"Note exceeds size budget");
    let parts=properties::split(&raw)?;
    let meta=if let Some(payload)=parts.payload{properties::parse_yaml(payload)?}else{Value::Null};
    // The source link must itself be the final visible standalone footer
    // line. A matching link at the end of a sentence, a list/blockquote, or
    // a table field is not a footer; unrelated historical HTML tags or prose
    // inequalities in the body must not block library indexing.
    let locations=find_zotero_footers(&raw)?;
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
pub fn directory_fingerprint(root:&Path,literature:&Path,excluded:&[PathBuf],personal_library_id:Option<u64>)->Result<Value>{
    let mut stack=vec![literature.to_owned()];
    let mut entries=Vec::new();
    let mut count=0usize;
    while let Some(dir)=stack.pop(){
        for entry in fs::read_dir(&dir)?{
            let entry=entry?;
            let p=entry.path();
            count+=1;
            ensure!(count<=MAX_NOTES*10,"Vault traversal exceeds budget");
            if excluded.iter().any(|e|p.starts_with(e)){continue;}
            let name=entry.file_name().to_string_lossy().to_ascii_lowercase();
            if matches!(name.as_str(),".obsidian"|".trash"|".z2o-backups"|".git"|"template"|"templates"|".attachments"|"attachments"|"backups"|"drafts"){continue;}
            let ty=entry.file_type()?;
            if no_link(&p).is_err(){continue;}
            if ty.is_dir(){stack.push(p);continue;}
            if ty.is_file()&&p.extension().and_then(|s|s.to_str()).is_some_and(|s|s.eq_ignore_ascii_case("md")){
                let meta=entry.metadata()?;
                let mtime_ns=meta.modified().ok().and_then(|t|t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0u128,|d|d.as_nanos());
                entries.push(json!([rel(root,&p)?,meta.len(),mtime_ns.to_string()]));
            }
        }
    }
    entries.sort_by(|a,b|a[0].as_str().cmp(&b[0].as_str()));
    let excl_text:Vec<String>=excluded.iter().filter_map(|p|path_text(p).ok()).collect();
    let payload=serde_json::to_vec(&json!({
        "vault_root":path_text(root)?,
        "literature_root":path_text(literature)?,
        "excluded_roots":excl_text,
        "personal_library_id":personal_library_id,
        "entries":entries,
    }))?;
    Ok(json!({"fingerprint":files::digest(&payload),"file_count":entries.len()}))
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
    let fp=directory_fingerprint(&root,&literature,&excluded,personal_library_id)?;
    let (notes,issues)=scan(&root,&literature,&excluded,&mut progress)?;
    Ok(json!({"schema_version":1,"vault_root":path_text(&root)?,"literature_root":path_text(&literature)?,
        "excluded_roots":excluded,"confirmed_personal_library_id":personal_library_id,
        "directory_fingerprint":fp["fingerprint"],"indexed_file_count":fp["file_count"],
        "notes":notes,"issues":issues,"scan_complete":issues.is_empty(),
        "source_fidelity":"not_assessed","boundary":BOUNDARY}))
}
pub fn index_cached(vault:&Path,literature_relative:&str,excluded_relative:&Value,personal_library_id:Option<u64>,cache_output:&Path,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
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
    let fp=directory_fingerprint(&root,&literature,&excluded,personal_library_id)?;
    if cache_output.is_file(){
        if let Ok((raw,b))=files::read_bound(cache_output){
            if let Ok(s)=std::str::from_utf8(&raw){
                if let Ok(mut cached)=crate::parse_json(s){
                    if cached["schema_version"].as_u64()==Some(1)&&cached["directory_fingerprint"]==fp["fingerprint"]{
                        cached["cache_hit"]=json!(true);
                        cached["saved_record"]=b;
                        return Ok(cached);
                    }
                }
            }
        }
    }
    let mut fresh=index(vault,literature_relative,excluded_relative,personal_library_id,&mut progress)?;
    fresh["cache_hit"]=json!(false);
    Ok(fresh)
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
    static RE:std::sync::OnceLock<regex::Regex>=std::sync::OnceLock::new();
    let r=RE.get_or_init(||regex::Regex::new(r"(?:^|[^0-9])([12][0-9]{3})(?:[^0-9]|$)").expect("static year regex"));
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
pub fn match_references_batch(
    index_path:&Path,
    expected_index_sha:Option<&str>,
    catalog_path:&Path,
    expected_catalog_sha:Option<&str>,
    references:&Value,
)->Result<Value>{
    let load_bound=|p:&Path,expected:Option<&str>|->Result<(Value,Value)>{
        ensure!(fs::metadata(p)?.len()<=256*1024*1024,"Index/catalog exceeds input budget");
        let(raw,b)=files::read_bound(p)?;
        ensure!(raw.len()<=256*1024*1024,"Index/catalog exceeds input budget");
        if let Some(h)=expected{ensure!(b["sha256"]==h,"Library evidence binding changed");}
        Ok((crate::parse_json(std::str::from_utf8(&raw)?)?,b))
    };
    let (index,ib)=load_bound(index_path,expected_index_sha)?;
    let (catalog,cb)=load_bound(catalog_path,expected_catalog_sha)?;
    ensure!(index["schema_version"].as_u64()==Some(1)&&catalog["schema_version"].as_u64()==Some(1),"Unsupported library evidence schema");
    let root=files::canonical(&path(&index["vault_root"])?)?;
    let kind=catalog["library"]["type"].as_str().context("Actual catalog library kind required")?;
    let library_id=catalog["library"]["id"].as_u64().filter(|id|*id>0).context("Actual catalog library ID required")?;
    ensure!(matches!(kind,"user"|"group"),"Unsupported catalog library");
    if kind=="user"{ensure!(index["confirmed_personal_library_id"].as_u64()==Some(library_id),"Confirm the personal Zotero library associated with the vault; route alias 0 is not identity");}
    let mut by_key=BTreeMap::<(Option<u64>,String),Vec<&Value>>::new();
    for r in catalog["items"].as_array().context("Actual raw Zotero catalog required")?{
        let Some((record_kind,id,k))=identity(r)else{anyhow::bail!("Invalid raw catalog object");};
        ensure!(record_kind==kind&&id==library_id,"Catalog contains another library");
        by_key.entry((if kind=="group"{Some(id)}else{None},k.to_owned())).or_default().push(r);
    }
    let notes=index["notes"].as_array().context("Actual indexed note list required")?;
    let mut indexed=Vec::new();
    let mut unidentified=0usize;
    let mut missing_catalog_record=0usize;
    for n in notes{
        let loc=&n["identity_candidates"];
        let Some(loc)=loc.as_array().filter(|v|v.len()==1).and_then(|v|v.first())else{unidentified+=1;continue;};
        let key=(loc["group_id"].as_u64(),loc["item_key"].as_str().context("Indexed key")?.to_owned());
        if key.0!=(if kind=="group"{Some(library_id)}else{None}){continue;}
        let Some(rows)=by_key.get(&key)else{missing_catalog_record+=1;continue;};
        indexed.push((n,loc,rows.as_slice()));
    }
    let mut by_title_idx=BTreeMap::<String,Vec<usize>>::new();
    let mut by_doi_idx=BTreeMap::<String,Vec<usize>>::new();
    for (idx,(_,_,rows)) in indexed.iter().enumerate(){
        for r in *rows{
            let data=&r["data"];
            if let Some(t)=text(&data["title"]).map(norm){by_title_idx.entry(t).or_default().push(idx);}
            if let Some(d)=text(&data["DOI"]).map(doi){by_doi_idx.entry(d).or_default().push(idx);}
        }
    }
    let refs=references.as_array().context("references must be an array")?;
    ensure!(refs.len()<=500,"Batch size exceeds 500 references");
    let complete=index["scan_complete"]==true&&catalog["pagination_complete"]==true&&catalog["local_version_consistent"]==true&&unidentified==0&&missing_catalog_record==0;
    let mut results=Vec::with_capacity(refs.len());
    let mut counts=BTreeMap::<&str,usize>::new();
    for ref_item in refs{
        let title=norm(text(&ref_item["title"]).context("Actual original source title required")?);
        let date=text(&ref_item["year"]).context("Actual source year string required")?;
        ensure!(date.len()==4&&date.bytes().all(|b|b.is_ascii_digit()),"Original four-digit year required");
        let item_type=text(&ref_item["item_type"]).context("Explicit original publication type required")?;
        let authors=ref_item["authors"].as_array().filter(|v|!v.is_empty()&&v.iter().all(|v|text(v).is_some())).context("Original full author names required")?
            .iter().map(|v|norm(v.as_str().unwrap())).collect::<Vec<_>>();
        let expected_doi=text(&ref_item["doi"]).map(doi);

        let mut hit_indices=BTreeSet::new();
        if let Some(is)=by_title_idx.get(&title){hit_indices.extend(is.iter().copied());}
        if let Some(ed)=&expected_doi{
            if let Some(is)=by_doi_idx.get(ed){hit_indices.extend(is.iter().copied());}
        }
        let mut candidates=Vec::new();
        for idx in hit_indices{
            let (n,loc,rows)=indexed[idx];
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
        let status=if candidates.is_empty(){if complete{"absent_in_bound_snapshot"}else{"unavailable"}}
            else if candidates.len()!=1||candidates[0]["all_bibliographic_fields_equal"]!=true{"ambiguous"}
            else{"unique_bibliographic_candidate_requires_version_review"};
        *counts.entry(status).or_insert(0)+=1;
        results.push(json!({"reference":ref_item,"status":status,"candidates":candidates,"match_confirmed":false}));
    }
    ensure!(check_ledger::bind_file(index_path)?==ib&&check_ledger::bind_file(catalog_path)?==cb,"Library evidence changed");
    Ok(json!({
        "schema_version":1,"index":ib,"catalog":cb,
        "scope_complete_at_capture":complete,"unidentified_note_count":unidentified,"missing_catalog_record_count":missing_catalog_record,
        "summary":{
            "total":results.len(),
            "unique_bibliographic_candidate_requires_version_review":counts.get("unique_bibliographic_candidate_requires_version_review").copied().unwrap_or(0),
            "ambiguous":counts.get("ambiguous").copied().unwrap_or(0),
            "absent_in_bound_snapshot":counts.get("absent_in_bound_snapshot").copied().unwrap_or(0),
            "unavailable":counts.get("unavailable").copied().unwrap_or(0),
        },
        "results":results,"source_fidelity":"not_assessed","boundary":BOUNDARY,
    }))
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
    #[test]
    fn prose_index_skips_unrelated_rich_table_yaml_and_never_promotes_table_links(){
        let root=tempfile::tempdir().unwrap();
        let good=root.path().join("with-table-and-footer.md");
        fs::write(&good,"---\n年份: 2020\n---\n~~~rich-table\nnot: [valid\n~~~\n\n---\n\n## [Zotero原条目](zotero://select/library/items/GOOD0001)\n").unwrap();
        let r=inspect_note(&good,root.path()).unwrap();
        assert_eq!(r["status"],"item_locator_found");
        assert_eq!(r["identity_candidates"][0]["item_key"],"GOOD0001");

        let bad=root.path().join("table-only.md");
        fs::write(&bad,"~~~rich-table\n## [Zotero原条目](zotero://select/library/items/BAD00001)\n~~~\n").unwrap();
        let r=inspect_note(&bad,root.path()).unwrap();
        assert_eq!(r["status"],"item_locator_missing");
        assert_eq!(r["identity_candidates"],json!([]));
    }
    #[test]
    fn historical_notes_with_html_anchors_media_and_inequalities_index_cleanly(){
        let root=tempfile::tempdir().unwrap();
        let note=root.path().join("historical.md");
        fs::write(&note,concat!(
            "---\n年份: 2022\n---\n",
            "<a id=\"q53-table01\"></a>\n\n",
            "Comparison B,C<A，后续说明。\n\n",
            "<video controls preload=\"metadata\" src=\"文献/attachments/v.mp4\"></video>\n\n",
            "<tr><td><img alt=\"图\" src=\"../img.png\" /></td></tr>\n\n",
            "---\n\n",
            "## [Zotero原条目](zotero://select/library/items/HIST0001)\n"
        )).unwrap();
        let r=inspect_note(&note,root.path()).unwrap();
        assert_eq!(r["status"],"item_locator_found");
        assert_eq!(r["identity_candidates"][0]["item_key"],"HIST0001");
    }
    #[test]
    fn comments_and_delimited_spans_hide_fake_headings_and_reject_unclosed_markup(){
        let root=tempfile::tempdir().unwrap();
        let commented=root.path().join("commented.md");
        fs::write(&commented,concat!(
            "<!--\n## [Zotero原条目](zotero://select/library/items/FAKE0001)\n-->\n",
            "%% ## [Zotero原条目](zotero://select/library/items/FAKE0002) %%\n",
            "`\n## [Zotero原条目](zotero://select/library/items/FAKE0003)\n`\n\n",
            "## [Zotero原条目](zotero://select/library/items/REAL0001)\n"
        )).unwrap();
        let r=inspect_note(&commented,root.path()).unwrap();
        assert_eq!(r["status"],"item_locator_found");
        assert_eq!(r["identity_candidates"].as_array().unwrap().len(),1);
        assert_eq!(r["identity_candidates"][0]["item_key"],"REAL0001");

        let unclosed=root.path().join("unclosed.md");
        fs::write(&unclosed,"<!-- unclosed comment\n## [Zotero原条目](zotero://select/library/items/REAL0001)\n").unwrap();
        let err=inspect_note(&unclosed,root.path()).unwrap_err();
        assert!(format!("{err:#}").contains("MAP_UNCLOSED_MARKUP"));
    }
    #[test]
    fn batch_match_and_directory_fingerprint_caching_work(){
        let dir=tempfile::tempdir().unwrap();
        let vault=dir.path().join("vault");
        fs::create_dir_all(vault.join(".obsidian")).unwrap();
        fs::create_dir_all(vault.join("文献")).unwrap();
        let n1=vault.join("文献/A.md");
        fs::write(&n1,"---\n年份: 2023\n---\n正文。\n\n## [Zotero原条目](zotero://select/library/items/ABCD1234)\n").unwrap();
        let idx_path=dir.path().join("index.json");
        let idx=index(&vault,"文献",&json!([]),Some(99),|_,_,_|{}).unwrap();
        let idx_bytes=serde_json::to_vec_pretty(&idx).unwrap();
        fs::write(&idx_path,&idx_bytes).unwrap();
        let cached=index_cached(&vault,"文献",&json!([]),Some(99),&idx_path,|_,_,_|panic!("should hit cache")).unwrap();
        assert_eq!(cached["cache_hit"],true);

        let cat_path=dir.path().join("catalog.json");
        let cat=json!({
            "schema_version":1,"library":{"type":"user","id":99},
            "pagination_complete":true,"local_version_consistent":true,
            "items":[{"key":"ABCD1234","library":{"type":"user","id":99},"data":{"key":"ABCD1234","itemType":"journalArticle","title":"Alpha Homology","date":"2023-01-01","DOI":"10.1000/a","creators":[{"creatorType":"author","firstName":"Alice","lastName":"Smith"}]}}]
        });
        fs::write(&cat_path,serde_json::to_vec_pretty(&cat).unwrap()).unwrap();
        let batch=match_references_batch(&idx_path,None,&cat_path,None,&json!([
            {"title":"Alpha Homology","year":"2023","item_type":"journalArticle","authors":["Alice Smith"],"doi":"10.1000/a"},
            {"title":"Missing Paper","year":"2022","item_type":"preprint","authors":["Bob Lee"]}
        ])).unwrap();
        assert_eq!(batch["summary"]["total"],2);
        assert_eq!(batch["results"][0]["status"],"unique_bibliographic_candidate_requires_version_review");
        assert_eq!(batch["results"][1]["status"],"absent_in_bound_snapshot");
    }
}
