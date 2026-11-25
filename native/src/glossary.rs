//! Source-bound glossary retrieval. A match is a search lead, not a translation.
use crate::{files,parse_json,path,string,syntax};
use anyhow::{anyhow,ensure,Context,Result};
use regex::Regex;
use serde_json::{json,Map,Value};
use std::{collections::{BTreeMap,BTreeSet,HashMap,HashSet},fs,io::Write,path::{Path,PathBuf},sync::OnceLock};
use unicode_casefold::UnicodeCaseFold;

fn token_re()->&'static Regex {
    static RE:OnceLock<Regex>=OnceLock::new();
    RE.get_or_init(||Regex::new(r"[A-Za-z][A-Za-z0-9]*").unwrap())
}
fn tokens(text:&str)->Vec<String>{token_re().find_iter(text).map(|m|m.as_str().to_ascii_lowercase()).collect()}
fn cells(line:&str)->Vec<String>{
    let mut raw=Vec::new();let mut start=0;
    for (i,b) in line.bytes().enumerate(){
        if b==b'|'&&(i==0||line.as_bytes()[i-1]!=b'\\'){raw.push(line[start..i].trim().replace(r"\|","|"));start=i+1;}
    }
    raw.push(line[start..].trim().replace(r"\|","|"));
    if raw.first().is_some_and(|s|s.is_empty()){raw.remove(0);}
    if raw.last().is_some_and(|s|s.is_empty()){raw.pop();}
    raw
}
fn separator(v:&[String])->bool{
    static RE:OnceLock<Regex>=OnceLock::new();
    let re=RE.get_or_init(||Regex::new(r"^:?-{3,}:?$").unwrap());
    v.len()>=3&&v.iter().all(|s|re.is_match(s))
}
fn text_snapshot(p:&Path)->Result<(String,Value)>{
    let (raw,binding)=files::read_bound(p)?;
    let text=std::str::from_utf8(&raw)?;
    Ok((text.strip_prefix('\u{feff}').unwrap_or(text).to_owned(),binding))
}
fn binding(p:&Path)->Result<Value>{let v=files::snapshot(p)?;Ok(json!({"path":v["path"],"sha256":v["sha256"]}))}
pub struct Glossary {
    pub binding:Value,
    line_count:usize,
    entries:BTreeMap<String,Value>,
    order:Vec<String>,
    postings:HashMap<String,BTreeSet<String>>,
    phrases:HashMap<String,Vec<(Vec<String>,String,String)>>,
    entry_phrases:HashMap<String,Vec<Vec<String>>>,
}
impl Glossary{
    pub fn new(p:&Path)->Result<Self>{
        let (text,binding)=text_snapshot(p)?;
        // Match the legacy splitlines() numbering and strip just the line
        // separator, not spaces or escapes inside the recorded excerpt.
        let lines:Vec<_>=syntax::lines(&text).into_iter().map(|line|{
            syntax::body(&text,line).trim_end_matches(['\u{b}','\u{c}','\u{1c}','\u{1d}','\u{1e}','\u{85}','\u{2028}','\u{2029}'])
        }).collect();
        let mut index=Self{binding,line_count:lines.len(),entries:BTreeMap::new(),order:vec![],postings:HashMap::new(),phrases:HashMap::new(),entry_phrases:HashMap::new()};
        let identifier=Regex::new(r"^[A-Za-z][A-Za-z0-9_-]*$")?;
        let alias=Regex::new(r"[;；]\s*别名(?:/缩写)?\s*[:：]\s*")?;
        let parsed:Vec<_>=lines.iter().map(|s|cells(s)).collect();
        for (i,c) in parsed.iter().enumerate(){
            if c.len()<3||separator(c)||parsed.get(i+1).is_some_and(|n|n.len()==c.len()&&separator(n))||!identifier.is_match(&c[0]){continue;}
            let id=&c[0];
            ensure!(!index.entries.contains_key(id),"Duplicate glossary entry: {id}");
            ensure!(!c[1].is_empty()&&!c[2].is_empty(),"A glossary entry needs its source phrase and translation: {id}");
            let entry=json!({"id":id,"english":c[1],"translation":c[2],"context":c.get(3).map(String::as_str).unwrap_or(""),
                "line":i+1,"raw_line":lines[i],"evidence":{"path":index.binding["path"],"sha256":index.binding["sha256"],"locator":{"kind":"lines","start":i+1,"end":i+1}}});
            index.entries.insert(id.clone(),entry);index.order.push(id.clone());
            let mut parts=alias.splitn(&c[1],2);let mut phrases=vec![parts.next().unwrap_or("")];
            if let Some(tail)=parts.next(){phrases.extend(tail.split([';','；','、']).map(str::trim).filter(|s|!s.is_empty()));}
            let mut seen=HashSet::new();
            for phrase in phrases{
                let tok=tokens(phrase);if tok.is_empty()||!seen.insert(tok.clone()){continue;}
                index.phrases.entry(tok[0].clone()).or_default().push((tok.clone(),id.clone(),phrase.to_owned()));
                for t in &tok{index.postings.entry(t.clone()).or_default().insert(id.clone());}
                index.entry_phrases.entry(id.clone()).or_default().push(tok);
            }
        }
        ensure!(!index.entries.is_empty(),"The glossary has no identifiable term rows.");
        Ok(index)
    }
    pub(crate) fn line_count(&self)->usize{self.line_count}
    pub fn search(&self,query:&str)->Result<Value>{
        ensure!(!query.trim().is_empty(),"Provide the actual nonempty search query.");
        let tok=tokens(query);
        if tok.is_empty(){
            let q:String=query.case_fold().collect();
            return Ok(json!(self.order.iter().filter_map(|id|{
                let entry=&self.entries[id];let raw:String=entry["raw_line"].as_str().unwrap().case_fold().collect();
                raw.contains(&q).then_some(entry.clone())
            }).collect::<Vec<_>>()));
        }
        let mut postings=Vec::with_capacity(tok.len());
        for t in &tok{let Some(p)=self.postings.get(t) else{return Ok(json!([]));};postings.push(p);}
        postings.sort_by_key(|p|p.len());
        let found:Vec<_>=postings[0].iter().filter(|id|postings[1..].iter().all(|p|p.contains(*id)))
            .filter(|id|self.entry_phrases[*id].iter().any(|p|p.windows(tok.len()).any(|w|w==tok)))
            .map(|id|self.entries[id].clone()).collect();
        Ok(json!(found))
    }
    pub fn suggest(&self,text:&str,max_family_entries:usize)->Result<Value>{
        ensure!(max_family_entries>0,"max_family_entries must be a positive integer.");
        let spans:Vec<_>=token_re().find_iter(text).collect();
        let tok:Vec<_>=spans.iter().map(|m|m.as_str().to_ascii_lowercase()).collect();
        // Translate UTF-8 byte positions to Unicode code-point positions in a
        // single pass instead of repeatedly rescanning each source prefix.
        let mut offsets=HashMap::with_capacity(spans.len()*2);
        let (mut byte,mut cp)=(0,0);
        for span in &spans{
            cp+=text[byte..span.start()].chars().count();offsets.insert(span.start(),cp);
            cp+=span.as_str().chars().count();offsets.insert(span.end(),cp);byte=span.end();
        }
        let mut matches:BTreeMap<(usize,String,usize),Value>=BTreeMap::new();
        let mut add=|id:&str,start:usize,end:usize,mode:&str,query:&str|{
            let key=(offsets[&start],id.to_owned(),offsets[&end]);
            if let Some(v)=matches.get_mut(&key){v["match_types"].as_array_mut().unwrap().push(json!(mode));return;}
            matches.insert(key.clone(),json!({"entry":self.entries[id],"match_types":[mode],"query":query,
                "source_span":{"start":key.0,"end":key.2,"quote":&text[start..end]},"decision":"pending"}));
        };
        for (i,span) in spans.iter().enumerate(){
            if let Some(phrases)=self.phrases.get(&tok[i]){
                for (phrase,id,raw) in phrases{
                    if let Some(end)=i.checked_add(phrase.len()).filter(|e|*e<=tok.len()){
                        if tok[i..end]==phrase[..]{add(id,span.start(),spans[end-1].end(),"exact_token_phrase",raw);}
                    }
                }
            }
            if let Some(ids)=self.postings.get(&tok[i]).filter(|s|tok[i].len()>=4&&s.len()<=max_family_entries){
                for id in ids{add(id,span.start(),span.end(),"shared_token_family",span.as_str());}
            }
        }
        Ok(json!({"glossary":self.binding,"source_text_sha256":files::digest(text.as_bytes()),"matches":matches.into_values().collect::<Vec<_>>(),
            "source_fidelity":"not_assessed","boundary":"Search leads only. Read each hit in its source context; a shared token is not an equivalent concept or an adopted translation. Unmatched terms still require manual discovery."}))
    }
}
fn array<'a>(v:&'a Value,field:&str)->Result<&'a Vec<Value>>{v.as_array().with_context(||format!("{field} must be an array"))}
pub fn prepare(inventory:&Path,glossary:&Path,output:&Path,mut progress:impl FnMut(usize,usize))->Result<PathBuf>{
    ensure!(!output.exists()&&!output.is_symlink(),"Keep existing glossary search evidence: {}",output.display());
    let (text,inventory_binding)=text_snapshot(inventory)?;
    let inventory_path=path(&inventory_binding["path"])?;
    let doc=parse_json(&text)?;
    let sources=array(&doc["sources"],"sources")?;
    let index=Glossary::new(glossary)?;
    let mut blocks=Vec::new();
    let mut source_bindings=Map::new();
    for source in sources{
        let id=string(&source["id"],"source id")?;
        ensure!(!source_bindings.contains_key(&id),"duplicate source ID: {id}");
        let mut p=path(&source["path"])?;
        if !p.is_absolute(){p=inventory_path.parent().context("inventory parent")?.join(p);}
        let actual=binding(&p)?;
        ensure!(actual["sha256"]==source["sha256"],"Source bytes differ from inventory: {id}");
        source_bindings.insert(id.clone(),actual);
        if source["format"]=="docx"{
            for block in array(&source["native_blocks"],"native_blocks")?{
                ensure!(block.get("locator").is_some(),"missing native locator");
                blocks.push((id.clone(),json!({"native_locator":block["locator"]}),block));
            }
        }else{
            for page in array(&source["pages"],"pages")?{
                for block in array(&page["blocks"],"blocks")?{
                    ensure!(page.get("page").is_some()&&block.get("bbox").is_some(),"missing PDF source location");
                    blocks.push((id.clone(),json!({"page":page["page"],"bbox":block["bbox"]}),block));
                }
            }
        }
    }
    let mut rows=Vec::with_capacity(blocks.len());
    let total=blocks.len();
    for (i,(source_id,location,block)) in blocks.into_iter().enumerate(){
        let block_id=string(&block["id"],"block ID")?;
        let text=block["text"].as_str().ok_or_else(||anyhow!("block text must be a string"))?;
        let result=index.suggest(text,8)?;
        let mut row=json!({"block_id":block_id,"source_id":source_id,"source_text_sha256":result["source_text_sha256"],"matches":result["matches"]});
        row.as_object_mut().unwrap().extend(location.as_object().unwrap().clone());rows.push(row);
        progress(i+1,total);
    }
    for recorded in std::iter::once(&inventory_binding).chain(std::iter::once(&index.binding)).chain(source_bindings.values()){
        ensure!(binding(&path(&recorded["path"])?)?==*recorded,"Input changed during glossary search: {}",recorded["path"]);
    }
    let payload=json!({"schema_version":1,"bindings":{"source_inventory":inventory_binding,"glossary":index.binding,"sources":source_bindings},
        "blocks":rows,"source_fidelity":"not_assessed","boundary":"Automatic retrieval only; no term is adopted or excluded, and no semantic review is completed by this file."});
    let parent=output.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let dest=files::canonical(parent)?.join(output.file_name().context("missing output file name")?);
    let mut tmp=tempfile::NamedTempFile::new_in(dest.parent().context("output parent")?)?;
    serde_json::to_writer_pretty(&mut tmp,&payload)?;tmp.write_all(b"\n")?;tmp.as_file().sync_all()?;
    tmp.persist_noclobber(&dest).map_err(|e|anyhow!("Keep existing glossary search evidence: {}: {}",dest.display(),e.error))?;
    Ok(dest)
}
