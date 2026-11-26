//! Derive file-bound handoff maps from explicit source-unit registrations
//! and actual candidate selectors. Never infer semantic correctness.
use anyhow::{ensure,Context,Result};
use regex::Regex;
use serde_json::{json,Map,Value};
use std::{collections::{BTreeMap,BTreeSet,HashSet},fs,io::Write,path::{Path,PathBuf},sync::OnceLock};
use crate::{check_ledger,docx,files,handoff_scan::{Candidate,Span},path,path_text,pdf_structure::Pdf};

pub const BOUNDARY:&str="Bindings and recorded relationships are checked against the supplied inventory and candidate. Source PDFs are not reinterpreted here. Generated views are extracted, pending records; they do not certify observation, semantic correctness, inventory completeness, or real-library matching.";
const KINDS:&[&str]=&["metadata","paragraph","section","reference","figure","table","formula","equation","video","citation","cross_reference"];
const OBJECTS:&[&str]=&["figure","table","formula","equation","video"];
const CALLS:&[&str]=&["citation","cross_reference"];
const LINKS:&[&str]=&["internal_links","library_matches","external_links","resource_links"];
fn nonempty(v:&Value)->bool{v.as_str().is_some_and(|s|!s.trim().is_empty())}
fn text<'a>(v:&'a Value,label:&str)->Result<&'a str>{v.as_str().filter(|s|!s.trim().is_empty()).with_context(||format!("MAP_ID_REQUIRED: {label}"))}
fn rows<'a>(v:&'a Value,key:&str)->Result<Vec<&'a Value>>{
    if let Some(a)=v.as_array(){ensure!(a.iter().all(Value::is_object),"MAP_COLLECTION_ELEMENT: {key}");return Ok(a.iter().collect());}
    let m=v.as_object().with_context(||format!("MAP_COLLECTION: {key}"))?;
    for(id,row)in m{ensure!(row.is_object()&&row[key]==id.as_str(),"MAP_COLLECTION_KEY: {id}");}Ok(m.values().collect())
}
fn index<'a>(v:&'a Value,key:&str)->Result<BTreeMap<String,&'a Value>>{
    let mut out=BTreeMap::new();
    for row in rows(v,key)?{let id=text(&row[key],key)?.to_owned();ensure!(out.insert(id.clone(),row).is_none(),"MAP_DUPLICATE_ID: {id}");}Ok(out)
}
fn bbox(v:&Value)->bool{
    let Some(a)=v.as_array().filter(|a|a.len()==4)else{return false;};
    let n:Option<Vec<_>>=a.iter().map(Value::as_f64).collect();
    n.is_some_and(|b|b.iter().all(|n|n.is_finite())&&b[0]<b[2]&&b[1]<b[3])
}
fn hash_ok(v:&Value)->bool{v.as_str().is_some_and(|s|s.len()==64&&s.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)))}
pub(crate) struct Bindings{files:BTreeMap<PathBuf,Value>}
impl Bindings{
    pub fn new()->Self{Self{files:BTreeMap::new()}}
    pub fn capture(&mut self,p:&Path)->Result<Value>{
        let p=files::canonical(p)?;
        if let Some(b)=self.files.get(&p){return Ok(b.clone());}
        let b=check_ledger::bind_file(&p)?;self.files.insert(p,b.clone());Ok(b)
    }
    pub(crate) fn recorded(&mut self,v:&Value,base:&Path)->Result<Value>{
        ensure!(hash_ok(&v["sha256"]),"MAP_FILE_BINDING: actual lowercase SHA-256 required");
        let b=self.capture(&base.join(path(&v["path"])?))?;
        ensure!(b["sha256"]==v["sha256"],"MAP_BINDING_HASH: {}",b["path"]);Ok(b)
    }
    fn read(&mut self,p:&Path,max:u64)->Result<(Vec<u8>,Value)>{
        let b=self.capture(p)?;let p=path(&b["path"])?;
        ensure!(fs::metadata(&p)?.len()<=max,"Mapping input exceeds byte budget");
        let(raw,actual)=files::read_bound(&p)?;
        ensure!(actual==b&&raw.len() as u64<=max,"MAP_INPUT_CHANGED: {}",p.display());Ok((raw,b))
    }
    pub(crate) fn json_value(&mut self,p:&Path)->Result<(Value,Value)>{
        let(raw,b)=self.read(p,128*1024*1024)?;
        let v=crate::parse_json(std::str::from_utf8(&raw)?.trim_start_matches('\u{feff}'))?;
        Ok((v,b))
    }
    pub fn json(&mut self,p:&Path)->Result<(Value,Value)>{
        let(v,b)=self.json_value(p)?;
        ensure!(v.is_object()&&v["schema_version"].as_u64()==Some(1),"MAP_SCHEMA_VERSION");Ok((v,b))
    }
    pub(crate) fn all(&self)->Vec<Value>{self.files.values().cloned().collect()}
    pub(crate) fn merge(&mut self,other:&Self)->Result<()>{
        other.verify()?;
        for(p,b)in &other.files{
            if let Some(old)=self.files.get(p){ensure!(old==b,"MAP_INPUT_CHANGED between stages: {}",p.display());}
            else{self.files.insert(p.clone(),b.clone());}
        }
        Ok(())
    }
    pub fn verify(&self)->Result<()>{for(p,b)in &self.files{ensure!(check_ledger::bind_file(p)?==*b,"MAP_INPUT_CHANGED: {}",p.display());}Ok(())}
}
struct Block{source:String,page:Option<u64>,record:Value,order:usize}
impl Block{
    fn locator(&self)->Value{if self.page.is_none(){json!({"native_locator":self.record["locator"]})}else{json!({"page":self.page,"bbox":self.record["bbox"]})}}
}
struct Ctx{
    bindings:Bindings,sources:BTreeMap<String,Value>,source_bindings:BTreeMap<String,Value>,units:BTreeMap<String,Value>,
    blocks:BTreeMap<String,Block>,block_order:Vec<String>,block_units:BTreeMap<String,BTreeSet<String>>,unit_blocks:BTreeMap<String,BTreeSet<String>>,
    excluded:BTreeMap<String,String>,candidate:Candidate,candidate_path:PathBuf,common:Value,
}
impl Ctx{
    fn block(&mut self,source:&str,page:Option<u64>,record:&Value,order:usize)->Result<()>{
        let id=text(&record["id"],"block id")?.to_owned();
        ensure!(self.blocks.len()<500_000&&record["text"].is_string(),"MAP_BLOCK_RECORD");
        if page.is_some(){ensure!(bbox(&record["bbox"]),"MAP_BLOCK_RECORD: invalid PDF bbox");}
        else{ensure!(docx::valid_locator(&record["locator"]),"MAP_NATIVE_BLOCK_RECORD");}
        ensure!(self.blocks.insert(id.clone(),Block{source:source.into(),page,record:record.clone(),order}).is_none(),"MAP_BLOCK_ID: {id}");
        self.block_order.push(id);Ok(())
    }
    fn load(inventory:&Path,candidate:&Path,expected:&Value,resolved:bool,progress:&mut impl FnMut(usize,usize,&str))->Result<Self>{
        let mut bindings=Bindings::new();let (inv,ib)=bindings.json(inventory)?;
        let (raw,cb)=bindings.read(candidate,32*1024*1024)?;
        let inv_path=path(&ib["path"])?;let cp=path(&cb["path"])?;
        ensure!(inv_path!=cp,"Candidate and inventory must differ");
        let base=inv_path.parent().context("Inventory parent")?;
        let mut c=Self{bindings,sources:BTreeMap::new(),source_bindings:BTreeMap::new(),units:BTreeMap::new(),blocks:BTreeMap::new(),
            block_order:vec![],block_units:BTreeMap::new(),unit_blocks:BTreeMap::new(),excluded:BTreeMap::new(),
            candidate:Candidate::new(&raw)?,candidate_path:cp,common:Value::Null};
        let ss=index(&inv["sources"],"id")?;ensure!(!ss.is_empty(),"MAP_SOURCES_EMPTY");
        let mut pdfs=BTreeMap::<String,Pdf>::new();let mut pages=BTreeMap::<String,BTreeSet<u64>>::new();
        for(i,(id,s))in ss.iter().enumerate(){
            ensure!(matches!(s["role"].as_str(),Some("main"|"supplement")),"MAP_SOURCE_ROLE: {id}");
            let b=c.bindings.recorded(s,base)?;let p=path(&b["path"])?;
            ensure!(p!=inv_path&&p!=c.candidate_path,"Source aliases candidate/inventory");
            c.source_bindings.insert(id.clone(),b);c.sources.insert(id.clone(),(*s).clone());
            if s["format"]=="docx"{
                for(order,block)in rows(&s["native_blocks"],"id")?.into_iter().enumerate(){c.block(id,None,block,order)?;}
            }else{
                let pdf=Pdf::open(&p)?;let mut seen=BTreeSet::new();let mut order=0;
                for row in rows(&s["pages"],"page")?{
                    let page=row["page"].as_u64().filter(|n|*n>0&&*n<=pdf.len() as u64).context("MAP_PAGE_ID")?;
                    ensure!(seen.insert(page),"MAP_PAGE_ID: duplicate physical page");
                    if s["inventory_backend"]==crate::pdf_inventory::BACKEND{
                        crate::pdf_inventory::verify_page_files(row)?;
                        for key in ["render","text"]{c.bindings.recorded(&row[key],base)?;}
                        for key in ["glyphs","image_structure"]{c.bindings.recorded(&row["pdf_evidence"][key],base)?;}
                        if let Some(r)=row["pdf_evidence"]["ocr"].get("record"){c.bindings.recorded(r,base)?;}
                    }
                    for block in rows(&row["blocks"],"id")?{c.block(id,Some(page),block,order)?;order+=1;}
                }
                pages.insert(id.clone(),seen);pdfs.insert(id.clone(),pdf);
            }
            progress(i+1,ss.len(),"actual source bindings and block inventory");
        }
        if !expected.is_null(){
            let exp:BTreeMap<String,&Value>=if let Some(m)=expected.as_object(){m.iter().map(|(k,v)|(k.clone(),v)).collect()}else{index(expected,"id")?};
            ensure!(exp.keys().eq(c.sources.keys()),"MAP_SOURCE_SET");
            for(id,v)in exp{
                ensure!(c.bindings.recorded(v,base)?==c.source_bindings[&id],"MAP_BINDING_TARGET: {id}");
                ensure!(v.get("role").is_none()||v["role"]==c.sources[&id]["role"],"MAP_SOURCE_ROLE: {id}");
            }
        }
        let us=index(&inv["units"],"id")?;
        for(i,(id,u))in us.iter().enumerate(){
            let sid=text(&u["source"],"unit source")?;let s=c.sources.get(sid).context("MAP_UNIT_SOURCE")?;
            let kind=text(&u["kind"],"unit kind")?;ensure!(KINDS.contains(&kind),"MAP_UNIT_KIND: {id}");
            let locs=rows(&u["evidence"],"id")?;ensure!(!locs.is_empty(),"MAP_UNIT_EVIDENCE: {id}");
            let mut refs=BTreeSet::new();
            for loc in locs{
                let r=if s["format"]=="docx"{
                    let errors=docx::evidence_errors(s,loc);ensure!(errors.is_empty(),"MAP_NATIVE_LOCATOR: {}",errors.join(" "));
                    loc["blocks"].as_array().unwrap().clone()
                }else{
                    let page=loc["page"].as_u64().filter(|n|pages[sid].contains(n)).context("MAP_UNIT_PAGE")?;
                    let native=s["inventory_backend"]==crate::pdf_inventory::BACKEND;
                    let image=loc.get("image_xref").filter(|v|!v.is_null());
                    ensure!(bbox(&loc["bbox"])||nonempty(&loc["quote"])||native&&kind=="figure"&&image.is_some(),"MAP_UNIT_LOCATOR");
                    if let Some(b)=loc.get("bbox").filter(|v|!v.is_null()){ensure!(pdfs[sid].contains_bbox(page,b)?,"MAP_UNIT_LOCATOR: outside page");}
                    if let Some(x)=image{
                        ensure!(!native||kind=="figure","MAP_UNIT_XREF: native image locator requires a figure");
                        let x=u32::try_from(x.as_u64().context("MAP_UNIT_XREF")?)?;
                        ensure!(pdfs[sid].matches_image_locator(page,x,native)?,"MAP_UNIT_XREF: {}",
                            if native{"not actually invoked on the page"}else{"not a page image resource"});
                    }
                    loc.get("blocks").map(|v|v.as_array().context("MAP_BLOCK_REFERENCE").cloned()).transpose()?.unwrap_or_default()
                };
                for b in r{
                    let bid=text(&b,"source block")?;
                    let block=c.blocks.get(bid).context("MAP_BLOCK_REFERENCE: unknown block")?;
                    ensure!(block.source==sid&&block.page==loc["page"].as_u64(),"MAP_BLOCK_REFERENCE: wrong source/page");
                    refs.insert(bid.to_owned());c.block_units.entry(bid.to_owned()).or_default().insert(id.clone());
                }
            }
            if let Some(r)=u.get("text_visual_conflicts"){
                ensure!(s["format"]!="docx","MAP_CONFLICT_PAGE");
                let checked=crate::handoff_conflicts::validate_records(u,s,&c.source_bindings[sid],base,&pdfs[sid],r,resolved)?;
                for b in checked{c.bindings.recorded(&b,base)?;}
            }
            c.unit_blocks.insert(id.clone(),refs);c.units.insert(id.clone(),(*u).clone());progress(i+1,us.len(),"registered source units");
        }
        c.relationships()?;
        if let Some(v)=inv.get("excluded_blocks"){
            for r in rows(v,"block")?{
                let b=text(&r["block"],"excluded block")?;let reason=text(&r["reason"],"exclusion reason")?;
                ensure!(c.blocks.contains_key(b)&&!c.block_units.contains_key(b)&&!c.excluded.contains_key(b),"MAP_EXCLUSION_CONFLICT: {b}");
                c.excluded.insert(b.into(),reason.into());
            }
        }
        if let Some(v)=inv.get("excluded_pages"){
            for r in rows(v,"page")?{
                let sid=text(&r["source"],"excluded source")?;let p=r["page"].as_u64().context("MAP_EXCLUSION")?;
                ensure!(pages.get(sid).is_some_and(|v|v.contains(&p)),"MAP_EXCLUSION: unknown page");
                let reason=text(&r["reason"],"exclusion reason")?;
                for(id,b)in &c.blocks{if b.source==sid&&b.page==Some(p){
                    ensure!(!c.block_units.contains_key(id),"MAP_EXCLUSION_CONFLICT: {id}");c.excluded.entry(id.clone()).or_insert_with(||reason.to_owned());
                }}
            }
        }
        for b in c.blocks.keys(){ensure!(c.block_units.contains_key(b)||c.excluded.contains_key(b),"MAP_BLOCK_UNACCOUNTED: {b}");}
        c.common=json!({"source_inventory":ib,"candidate":cb,"sources":c.source_bindings});c.bindings.verify()?;Ok(c)
    }
    fn relationships(&self)->Result<()>{
        for(id,u)in &self.units{
            let kind=u["kind"].as_str().unwrap();let sid=u["source"].as_str().unwrap();
            match kind{
                "metadata"=>ensure!(nonempty(&u["field"]),"MAP_SOURCE_FIELD_MISSING: {id}.field"),
                "reference"=>ensure!(nonempty(&u["bibliography_id"])&&nonempty(&u["original_text"])&&u.get("number").is_some(),"MAP_REFERENCE_FIELD: {id}"),
                "citation"=>{
                    ensure!(nonempty(&u["display_text"]),"MAP_SOURCE_FIELD_MISSING: {id}.display_text");
                    let a=u["references"].as_array().filter(|a|!a.is_empty()).context("MAP_CITATION_REFERENCES")?;
                    let mut seen=HashSet::new();
                    for r in a{
                        let r=text(r,"reference unit")?;
                        ensure!(seen.insert(r),"MAP_DUPLICATE_REFERENCE");
                        let t=self.units.get(r).context("MAP_REFERENCE_UNKNOWN")?;
                        ensure!(t["kind"]=="reference","MAP_REFERENCE_UNKNOWN");
                        ensure!(t["source"]==sid||nonempty(&u["cross_source_reason"]),"MAP_CROSS_SOURCE_REASON");
                    }
                },
                "cross_reference"=>ensure!(nonempty(&u["display_text"])&&u["target_unit"].as_str().is_some_and(|s|self.units.contains_key(s)),"MAP_TARGET_UNIT_UNKNOWN"),
                k if OBJECTS.contains(&k)=>ensure!(u.get("original_number").is_some(),"MAP_SOURCE_FIELD_MISSING: {id}.original_number"),
                _=>{},
            }
            if let Some(p)=u.get("parent_id").filter(|v|!v.is_null()){
                ensure!(p.as_str().and_then(|v|self.units.get(v)).is_some_and(|v|v["source"]==sid),"MAP_PARENT_UNIT");
            }
            if kind=="metadata"&&u["field"]=="author"{
                let a=u["affiliation_units"].as_array().context("MAP_AFFILIATION_REFERENCE: actual parallel source markers and units required")?;
                let numbers=u["affiliation_numbers"].as_array().filter(|b|b.len()==a.len()).context("MAP_AFFILIATION_REFERENCE: parallel arrays required")?;
                ensure!(numbers.iter().all(|n|n.is_null()||nonempty(n)),"MAP_AFFILIATION_NUMBER: source markers are literal strings or null");
                if a.is_empty()||numbers.iter().any(Value::is_null){ensure!(nonempty(&u["affiliation_reason"]),"MAP_AFFILIATION_REASON: explain absent or unnumbered source affiliation");}
                let mut seen=HashSet::new();
                for v in a{
                    let id=v.as_str().context("MAP_AFFILIATION_REFERENCE")?;
                    ensure!(seen.insert(id),"MAP_AFFILIATION_DUPLICATE");
                    ensure!(self.units.get(id).is_some_and(|x|x["source"]==sid&&x["kind"]=="metadata"&&x["field"]=="affiliation"),"MAP_AFFILIATION_REFERENCE");
                }
            }
        }
        for(sid,s)in &self.sources{
            if let Some(h)=s.get("header_fields"){
                for(name,v)in h.as_object().context("MAP_HEADER_REFERENCE")?{
                    if let Some(ids)=v.get("units"){
                        let field=match name.as_str(){"title"=>"title","authors"=>"author","affiliations"=>"affiliation","publication_date"=>"publication_date",_=>anyhow::bail!("MAP_HEADER_REFERENCE")};
                        for id in ids.as_array().context("MAP_HEADER_REFERENCE")?{
                            ensure!(id.as_str().and_then(|x|self.units.get(x)).is_some_and(|u|u["source"]==sid.as_str()&&u["kind"]=="metadata"&&u["field"]==field),"MAP_HEADER_REFERENCE");
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn names(&self)->BTreeSet<String>{
        let mut out=BTreeSet::from(["source_map".to_owned(),"link_map".to_owned()]);
        if self.sources.values().any(|s|s["role"]=="supplement"){out.insert("si_map".into());}out
    }
    fn derive(&mut self,deliveries:&Value,progress:&mut impl FnMut(usize,usize,&str))->Result<Value>{
        let ds=index(deliveries,"unit")?;ensure!(ds.keys().eq(self.units.keys()),"MAP_DELIVERY_SET: every unit needs exactly one delivery");
        let mut targets=BTreeMap::<String,Span>::new();let mut mapped=BTreeMap::<String,Value>::new();
        for(i,(id,d))in ds.iter().enumerate(){
            let u=&self.units[id];let sid=u["source"].as_str().unwrap();
            let span=self.candidate.resolve(&d["target"],u["kind"].as_str().unwrap()).with_context(||format!("{id}: target"))?;
            targets.insert(id.clone(),span);
            let mut row=u.clone();row.as_object_mut().unwrap().extend(json!({"unit":id,"source_id":sid,"source_role":self.sources[sid]["role"],
                "source_attachment":self.source_bindings[sid],"source_locator":u["evidence"],"registered_unit":u,"target":d["target"],"delivery":d,
                "candidate_locator":self.candidate.locator(span)?,"status":"extracted","review_status":"pending"}).as_object().unwrap().clone());
            if let Some(v)=u["original_text"].as_str(){row["original_sha256"]=json!(files::digest(v.as_bytes()));}
            if let Some(assets)=d.get("assets"){
                let root=if let Some(r)=d.get("resource_root"){files::canonical(&path(r)?)?}else{self.candidate_path.parent().unwrap().to_owned()};
                ensure!(root.is_dir(),"MAP_RESOURCE_ROOT");let mut resources=vec![];
                for a in rows(assets,"path")?{
                    let actual=files::canonical(&root.join(path(&a["path"])?))?;
                    ensure!(actual.starts_with(&root),"MAP_RESOURCE_OUTSIDE_ROOT");
                    let b=self.bindings.capture(&actual)?;
                    ensure!(a.get("sha256").is_none()||a["sha256"]==b["sha256"],"MAP_RESOURCE_HASH");resources.push(b);
                }
                row["resources"]=json!(resources);
            }
            mapped.insert(id.clone(),row);progress(i+1,ds.len(),"explicit delivery targets");
        }
        let mut out=Map::new();
        for(name,role)in [("source_map","main"),("si_map","supplement")]{
            if !self.names().contains(name){continue;}
            let sources:Vec<_>=self.sources.iter().filter(|(_,s)|s["role"]==role).map(|(id,_)|{
                let b=&self.source_bindings[id];json!({"id":id,"role":role,"path":b["path"],"sha256":b["sha256"]})
            }).collect();
            let units:Vec<_>=mapped.values().filter(|u|u["source_role"]==role).cloned().collect();
            let mut blocks=vec![];
            for id in &self.block_order{
                let b=&self.blocks[id];if self.sources[&b.source]["role"]!=role{continue;}
                let refs:Vec<_>=self.block_units.get(id).into_iter().flatten().cloned().collect();
                let kinds:BTreeSet<_>=refs.iter().map(|r|self.units[r]["kind"].as_str().unwrap()).collect();
                let kind=if kinds.len()==1{*kinds.iter().next().unwrap()}else if refs.is_empty(){"excluded"}else{"mixed"};
                let exclusion=self.excluded.get(id);
                let mut r=json!({"block_id":id,"source_id":b.source,"source_blocks":[id],"source_attachment":self.source_bindings[&b.source],
                    "source_locator":b.locator(),"order":b.order,"kind":kind,"original_text":b.record["text"],
                    "original_sha256":files::digest(b.record["text"].as_str().unwrap().as_bytes()),
                    "decision":if exclusion.is_some(){"exclude"}else{"keep"},"target_units":refs,
                    "reason":exclusion.map(String::as_str).unwrap_or("Accounted for by explicit inventory units and delivery targets; semantic review remains pending."),
                    "status":"extracted","review_status":"pending"});
                if refs.len()==1{r["target_unit"]=json!(refs[0]);}blocks.push(r);
            }
            out.insert(name.into(),json!({"schema_version":1,"bindings":self.common,"status":"extracted","review_status":"pending",
                "source_fidelity":"not_assessed","boundary":BOUNDARY,"sources":sources,"blocks":blocks,"units":units,
                "metadata":units.iter().filter(|u|u["kind"]=="metadata").collect::<Vec<_>>(),
                "objects":units.iter().filter(|u|OBJECTS.contains(&u["kind"].as_str().unwrap())).collect::<Vec<_>>(),
                "references":units.iter().filter(|u|u["kind"]=="reference").collect::<Vec<_>>(),
                "citations":units.iter().filter(|u|CALLS.contains(&u["kind"].as_str().unwrap())).collect::<Vec<_>>()}));
        }
        out.insert("link_map".into(),self.links(&ds,&targets,progress)?);self.bindings.verify()?;Ok(Value::Object(out))
    }
    fn links(&self,ds:&BTreeMap<String,&Value>,targets:&BTreeMap<String,Span>,progress:&mut impl FnMut(usize,usize,&str))->Result<Value>{
        let mut owners=BTreeMap::new();let mut anchors=vec![];
        for(id,span)in &self.candidate.anchors{
            let u=owner(targets,*span).with_context(||format!("MAP_ANCHOR_DELIVERY_MISSING: {id}"))?;
            owners.insert(id.clone(),u.to_owned());
            anchors.push(json!({"id":id,"target_id":id,"target_unit":u,"source_id":self.units[u]["source"],
                "candidate_locator":self.candidate.locator(*span)?,"status":"extracted","review_status":"pending"}));
        }
        let actual:BTreeSet<_>=self.candidate.links.iter().map(|o|o.locator.to_string()).collect();
        let mut overrides=BTreeMap::<String,(&str,&Value)>::new();
        for(id,d)in ds{if let Some(v)=d.get("link_bindings"){
            for r in rows(v,"id")?{
                let key=r["candidate_locator"].to_string();
                ensure!(actual.contains(&key)&&overrides.insert(key,(id.as_str(),r)).is_none(),"MAP_LINK_OVERRIDE");
            }
        }}
        static SCHEME_RE:OnceLock<Regex>=OnceLock::new();
        let scheme=SCHEME_RE.get_or_init(||Regex::new(r"^[A-Za-z][A-Za-z0-9+.-]*:").expect("static regex"));
        let mut out:BTreeMap<&str,Vec<Value>>=LINKS.iter().map(|s|(*s,vec![])).collect();
        let mut unresolved=vec![];let mut calls=BTreeMap::<String,BTreeSet<String>>::new();
        for(i,o)in self.candidate.links.iter().enumerate(){
            let key=o.locator.to_string();let id=format!("link-{}",files::digest(key.as_bytes()));
            let explicit=overrides.get(&key);
            let u=if let Some((u,_))=explicit{*u}else{owner(targets,o.span).with_context(||format!("MAP_LINK_DELIVERY_MISSING: {key}"))?};
            ensure!(targets[u].contains(o.span),"MAP_LINK_ORIGIN_TARGET: {id}");
            let over=explicit.map(|(_,r)|*r).unwrap_or(&Value::Null);
            let unit=&self.units[u];let blocks=&self.unit_blocks[u];
            let origin=over.get("origin_block").or_else(||ds[u].get("origin_block")).and_then(Value::as_str)
                .or_else(||if blocks.len()==1{blocks.iter().next().map(String::as_str)}else{None}).context("MAP_LINK_SOURCE_BLOCK_MISSING")?;
            ensure!(blocks.contains(origin),"MAP_LINK_ORIGIN_BLOCK");
            let display=over.get("source_display_text").or_else(||unit.get("display_text")).and_then(Value::as_str).context("MAP_LINK_SOURCE_DISPLAY_MISSING")?;
            ensure!(unit.get("display_text").is_none()||unit["display_text"]==display,"MAP_LINK_SOURCE_DISPLAY");
            let href=percent_encoding::percent_decode_str(&o.href).decode_utf8_lossy();
            let fragment=if let Some(f)=href.strip_prefix('#'){Some(f)}
            else if let Some((p,f))=href.split_once('#'){
                if files::canonical(&self.candidate_path.parent().unwrap().join(p)).ok().as_ref()==Some(&self.candidate_path){Some(f)}else{None}
            }else{None};
            let collection=if o.image{"resource_links"}else if fragment.is_some(){"internal_links"}else if scheme.is_match(&href)||href.starts_with("//"){"external_links"}else{"library_matches"};
            let mut row=json!({"id":id,"source_id":unit["source"],"origin_unit":u,"origin_block":origin,"candidate_locator":o.locator,
                "href":o.href,"display_text":o.label,"source_display_text":display,"syntax":o.syntax,"image":o.image,"status":"extracted","review_status":"pending"});
            if collection=="internal_links"{
                let f=fragment.unwrap();let (tid,target)=if let Some(a)=f.strip_prefix('^'){
                    (a,owners.get(a).map(String::as_str).context("MAP_LINK_TARGET: unknown anchor")?)
                }else{
                    let spans=self.candidate.headings.get(f).filter(|s|s.len()==1).context("MAP_LINK_TARGET: absent/ambiguous heading")?;
                    (f,owner(targets,spans[0]).context("MAP_LINK_TARGET_UNIT")?)
                };
                if self.units[target]["kind"]=="reference"{
                    ensure!(unit["kind"]=="citation","MAP_OUTPUT_CITATION_UNMAPPED: actual bibliography call requires its own source citation occurrence");
                }
                if unit["kind"]=="citation"{ensure!(self.units[target]["kind"]=="reference"&&unit["references"].as_array().unwrap().iter().any(|r|r==target),"MAP_CITATION_TARGET");}
                if unit["kind"]=="cross_reference"{ensure!(unit["target_unit"]==target,"MAP_CROSS_REFERENCE_TARGET");}
                ensure!(over.get("target_unit").is_none()||over["target_unit"]==target,"MAP_LINK_TARGET_UNIT");
                if CALLS.contains(&unit["kind"].as_str().unwrap()){calls.entry(u.to_owned()).or_default().insert(target.to_owned());}
                row["target_id"]=json!(tid);row["target_unit"]=json!(target);
            }else{row["target_path"]=json!(o.href);}
            if collection=="library_matches"{
                let reason="No real-library identity lookup was supplied or performed.";
                row["status"]=json!("unavailable");row["match_status"]=json!("unavailable");row["reason"]=json!(reason);
                unresolved.push(json!({"id":format!("unresolved-{id}"),"occurrence_id":id,"unit":u,"source_id":unit["source"],"status":"unavailable","reason":reason}));
            }
            out.get_mut(collection).unwrap().push(row);progress(i+1,self.candidate.links.len(),"actual link occurrences");
        }
        for(id,u)in &self.units{
            let wanted:BTreeSet<String>=match u["kind"].as_str().unwrap(){
                "citation"=>u["references"].as_array().unwrap().iter().map(|r|r.as_str().unwrap().to_owned()).collect(),
                "cross_reference"=>BTreeSet::from([u["target_unit"].as_str().unwrap().to_owned()]),_=>continue
            };
            ensure!(calls.get(id)==Some(&wanted),"MAP_CALL_COVERAGE: {id}");
        }
        let mut v=json!({"schema_version":1,"bindings":self.common,"status":"extracted","review_status":"pending","source_fidelity":"not_assessed",
            "boundary":BOUNDARY,"anchors":anchors,"unresolved":unresolved});
        for(k,rows)in out{v[k]=json!(rows);}Ok(v)
    }
}
fn owner(targets:&BTreeMap<String,Span>,span:Span)->Option<&str>{
    let mut result=None;let mut size=usize::MAX;
    for(id,s)in targets{if !s.contains(span){continue;}
        if s.len()<size{size=s.len();result=Some(id.as_str());} else if s.len()==size{result=None;}}result
}
fn future(p:&Path)->Result<PathBuf>{
    let p=std::path::absolute(p)?;if p.exists(){return files::canonical(&p);}
    ensure!(!p.is_symlink(),"MAP_OUTPUT_EXISTS: dangling symlink");
    Ok(future(p.parent().context("Output parent")?)?.join(p.file_name().context("Output filename")?))
}
fn map_paths(v:&Value,names:&BTreeSet<String>,new:bool,bindings:&Bindings)->Result<BTreeMap<String,PathBuf>>{
    let m=v.as_object().context("MAP_PATHS")?;
    ensure!(m.keys().cloned().collect::<BTreeSet<_>>()==*names,"MAP_PATHS: exact main/link and applicable supplement maps");
    let mut used=HashSet::new();let mut out=BTreeMap::new();
    for(name,v)in m{
        let p=path(v)?;
        if new{ensure!(!p.exists()&&!p.is_symlink(),"MAP_OUTPUT_EXISTS: {}",p.display());}
        let p=if new{future(&p)?}else{files::canonical(&p)?};
        ensure!(used.insert(p.clone())&&!bindings.files.contains_key(&p),"MAP_PATH_ALIAS: {name}");
        if new{ensure!(!p.exists()&&!p.is_symlink(),"MAP_OUTPUT_EXISTS");}out.insert(name.clone(),p);
    }Ok(out)
}
pub(crate) fn publish(p:&Path,v:&Value)->Result<Value>{
    ensure!(!p.exists()&&!p.is_symlink(),"MAP_OUTPUT_EXISTS: {}",p.display());
    let parent=p.parent().context("Map output parent")?;fs::create_dir_all(parent)?;
    ensure!(files::canonical(parent)?==parent,"MAP_OUTPUT_PARENT_CHANGED");
    let mut bytes=serde_json::to_vec_pretty(v)?;bytes.push(b'\n');
    let mut f=tempfile::NamedTempFile::new_in(parent)?;f.write_all(&bytes)?;f.as_file().sync_all()?;
    f.persist_noclobber(p).map_err(|e|anyhow::anyhow!("MAP_WRITE_FAILED: {}",e.error))?;
    let b=check_ledger::bind_file(p)?;
    ensure!(b["sha256"]==files::digest(&bytes)&&crate::pipeline::read_json(p)?==*v,"MAP_WRITE_READBACK");Ok(b)
}
fn finding(e:anyhow::Error)->Value{
    let msg=format!("{e:#}");let code=msg.split(|c:char|!(c.is_ascii_uppercase()||c=='_'||c.is_ascii_digit())).find(|s|s.starts_with("MAP_")).unwrap_or("MAP_NATIVE_PREPARATION");
    json!({"code":code,"object_id":"native_handoff","message":msg,"level":"error"})
}
fn report(checked:Map<String,Value>,written:Map<String,Value>,findings:Vec<Value>)->Value{
    json!({"findings":findings,"checked_files":checked,"written_paths":written,"records_ready":findings.is_empty(),
        "source_fidelity":"not_assessed","boundary":BOUNDARY,"native_generation_candidate":true})
}
pub fn write(inventory:&Path,candidate:&Path,deliveries:&Value,paths:&Value,expected:&Value,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    let mut checked=Map::new();let mut written=Map::new();
    let result=(||->Result<()>{
        let mut c=Ctx::load(inventory,candidate,expected,false,&mut progress)?;
        checked.insert("source_inventory".into(),c.common["source_inventory"].clone());checked.insert("candidate".into(),c.common["candidate"].clone());
        let docs=c.derive(deliveries,&mut progress)?;let dest=map_paths(paths,&c.names(),true,&c.bindings)?;
        c.bindings.verify()?;
        for(i,(name,p))in dest.iter().enumerate(){
            let b=publish(p,&docs[name])?;written.insert(name.clone(),b["path"].clone());checked.insert(name.clone(),b);
            progress(i+1,dest.len(),"new handoff maps written and read back");
        }
        c.bindings.verify()?;Ok(())
    })();
    Ok(report(checked,written,result.err().map(finding).into_iter().collect()))
}
// Check every generated field against independently re-derived records.
// Manual merged/compact historical maps are not treated as equivalent.
/// Some historical maps omit nonrequired empty collections.  A missing
/// field is not the same as a present null or malformed collection, and it
/// must not mask any independently derived rows.
fn check_collection(actual:&Value,expected:&Value,group:&str,key:&str,name:&str,allow_omitted_empty:bool)->Result<()>{
    if allow_omitted_empty&&actual.get(group).is_none()&&expected[group].as_array().is_some_and(Vec::is_empty){return Ok(());}
    check_rows(&actual[group],&expected[group],key,&format!("{name}.{group}"))
}
fn check_rows(actual:&Value,expected:&Value,key:&str,name:&str)->Result<()>{
    let a=index(actual,key)?;let b=index(expected,key)?;
    ensure!(a.keys().eq(b.keys()),"MAP_COLLECTION_COVERAGE: {name}");
    for(id,want)in b{
        let got=a[&id];
        for(k,v)in want.as_object().unwrap(){
            if matches!(k.as_str(),"status"|"review_status"){ensure!(nonempty(&got[k]),"MAP_RECORD_STATUS: {id}");continue;}
            ensure!(got.get(k)==Some(v),"MAP_RECORD_MISMATCH: {name}.{id}.{k}");
        }
        if let Some(v)=got.get("target_unit"){ensure!(want.get("target_unit")==Some(v),"MAP_BLOCK_TARGET_SET: {id}");}
    }Ok(())
}
// A historical block view may explicitly merge several actual source blocks.
// Validate the partition and the hash of the joined original text, rather than
// pretending a generated per-block view is its only legitimate representation.
fn check_block_partition(c:&Ctx,actual:&Value,expected:&Value,name:&str,legacy:bool)->Result<BTreeMap<String,BTreeSet<String>>>{
    let a=index(actual,"block_id")?;let b=index(expected,"block_id")?;
    let mut aliases=BTreeMap::new();let mut covered=BTreeSet::new();let mut orders=BTreeSet::new();
    for(id,row)in a{
        let originals=match row.get("source_blocks"){
            Some(v)=>v.as_array().filter(|a|!a.is_empty()).context("MAP_BLOCK_UNKNOWN: source_blocks")?.iter()
                .map(|v|Ok(text(v,"original block")?.to_owned())).collect::<Result<Vec<_>>>()?,
            None=>vec![id.clone()],
        };
        let mut identities=BTreeSet::new();let mut locations=vec![];let mut readings=vec![];
        let mut units=BTreeSet::new();let mut source=None;let mut reasons=BTreeSet::new();let mut excluded=None;
        for bid in &originals{
            ensure!(identities.insert(bid.clone())&&covered.insert(bid.clone()),"MAP_BLOCK_DUPLICATE_COVERAGE: {bid}");
            let want=b.get(bid).context("MAP_BLOCK_UNKNOWN: original belongs to another source view")?;
            let block=c.blocks.get(bid).context("MAP_BLOCK_UNKNOWN")?;
            let sid=block.source.as_str();
            ensure!(source.is_none_or(|s|s==sid),"MAP_BLOCK_SOURCE: merged block crosses sources");source=Some(sid);
            ensure!(row["source_id"]==sid&&row["source_attachment"]==want["source_attachment"],"MAP_BLOCK_SOURCE: {id}");
            let is_excluded=want["decision"]=="exclude";
            ensure!(excluded.is_none_or(|s|s==is_excluded),"MAP_BLOCK_DECISION: kept and excluded blocks cannot be merged");excluded=Some(is_excluded);
            if is_excluded{reasons.insert(want["reason"].as_str().context("MAP_EXCLUSION_REASON")?);}
            locations.push(block.locator());readings.push(block.record["text"].as_str().context("MAP_BLOCK_TEXT")?);
            for u in want["target_units"].as_array().context("MAP_BLOCK_TARGET_SET")?{units.insert(text(u,"target unit")?.to_owned());}
        }
        let locator=row.get("source_locators").or_else(||row.get("source_locator")).context("MAP_BLOCK_LOCATOR")?;
        if let Some(v)=locator.as_array(){ensure!(v==&locations,"MAP_BLOCK_LOCATOR: {id}");}
        else{ensure!(locations.len()==1&&locator==&locations[0],"MAP_BLOCK_LOCATOR: {id}");}
        if let (Some(one),Some(many))=(row.get("source_locator"),row.get("source_locators")){
            ensure!(one==many||locations.len()==1&&one==&locations[0]&&many==&json!(locations),"MAP_BLOCK_LOCATOR: conflicting aliases");
        }
        let joiner=match row.get("joiner"){Some(v)=>v.as_str().context("MAP_BLOCK_JOINER")?,None if originals.len()==1=>"",None=>anyhow::bail!("MAP_BLOCK_JOINER: merged blocks require an explicit joiner")};
        let original=readings.join(joiner);
        ensure!(row["original_sha256"]==files::digest(original.as_bytes())&&row.get("original_text").is_none_or(|v|v==&json!(original)),"MAP_BLOCK_ORIGINAL_HASH: {id}");
        let order=row["order"].as_u64().context("MAP_BLOCK_ORDER")?;
        ensure!(orders.insert((source.unwrap().to_owned(),order)),"MAP_BLOCK_ORDER: duplicate source order");
        ensure!(nonempty(&row["kind"])&&nonempty(&row["reason"])&&
            (nonempty(&row["status"])||legacy&&row.get("status").is_none()),"MAP_BLOCK_DECISION: {id}");
        if let Some(s)=row.get("review_status"){ensure!(nonempty(s),"MAP_RECORD_STATUS: {id}");}
        let values=match row.get("target_units"){
            Some(v)=>v.as_array().context("MAP_BLOCK_TARGET_SET")?.clone(),
            None=>row.get("target_unit").cloned().into_iter().collect(),
        };
        let mut actual_units=BTreeSet::new();
        for v in &values{ensure!(actual_units.insert(text(v,"target unit")?.to_owned()),"MAP_DUPLICATE_REFERENCE");}
        if let Some(v)=row.get("target_unit"){ensure!(actual_units.contains(text(v,"target unit")?),"MAP_BLOCK_TARGET_SET: contradictory singular target");}
        ensure!(actual_units==units,"MAP_BLOCK_TARGET_SET: {id}");
        let kinds:BTreeSet<_>=units.iter().map(|u|c.units[u]["kind"].as_str().unwrap()).collect();
        let expected_kind=if kinds.len()==1{*kinds.iter().next().unwrap()}else if kinds.is_empty(){"excluded"}else{"mixed"};
        let historical_title=legacy&&row["kind"]=="title"&&expected_kind=="metadata"&&
            units.iter().all(|u|c.units[u]["field"]=="title");
        ensure!(row["kind"]==expected_kind||historical_title,"MAP_BLOCK_KIND: {id}");
        if excluded==Some(true){
            ensure!(row["decision"]=="exclude"&&units.is_empty()&&reasons.len()==1&&reasons.contains(text(&row["reason"],"exclusion reason")?),"MAP_EXCLUSION_CONFLICT: {id}");
        }else{ensure!(row["decision"]=="keep"&&!units.is_empty(),"MAP_BLOCK_DECISION: {id}");}
        aliases.insert(id,identities);
    }
    ensure!(covered==b.keys().cloned().collect(),"MAP_BLOCK_MISSING: {name}");
    Ok(aliases)
}
fn check_link_origins(actual:&Value,expected:&Value,aliases:&BTreeMap<String,BTreeSet<String>>,name:&str)->Result<()>{
    let a=index(actual,"id")?;let b=index(expected,"id")?;
    ensure!(a.keys().eq(b.keys()),"MAP_COLLECTION_COVERAGE: {name}");
    let mut normalized=vec![];
    for(id,want)in b{
        let mut got=a[&id].clone();
        let recorded=text(&got["origin_block"],"recorded origin block")?;
        let original=text(&want["origin_block"],"original origin block")?;
        ensure!(aliases.get(recorded).is_some_and(|ids|ids.contains(original)),"MAP_LINK_ORIGIN_BLOCK: {id}");
        got["origin_block"]=json!(original);normalized.push(got);
    }
    check_rows(&json!(normalized),expected,"id",name)
}
pub(crate) struct VerifiedMaps{pub docs:Value,pub checked:Map<String,Value>,pub bindings:Bindings}
fn check_map_status(got:&Value,name:&str,legacy:bool)->Result<()>{
    ensure!((got["source_fidelity"]=="not_assessed"||legacy&&got.get("source_fidelity").is_none())&&
        ["status","review_status"].iter().all(|k|nonempty(&got[*k])||legacy&&got.get(*k).is_none()),"MAP_RECORD_STATUS: {name}");
    Ok(())
}
fn check_compact_anchors(actual:&Value,expected:&Value)->Result<()>{
    let a=index(actual,"id")?;let b=index(expected,"id")?;
    ensure!(a.keys().eq(b.keys()),"MAP_COLLECTION_COVERAGE: link_map.anchors");
    for(id,want)in b{
        let got=a[&id];
        ensure!(got["target_id"]==want["target_id"]&&got["source_id"]==want["source_id"]&&nonempty(&got["status"]),
            "MAP_ANCHOR_SOURCE_OR_TARGET: {id}");
        for (k,v) in want.as_object().unwrap(){
            if !matches!(k.as_str(),"status"|"review_status"){
                if let Some(recorded)=got.get(k){ensure!(recorded==v,"MAP_RECORD_MISMATCH: link_map.anchors.{id}.{k}");}
            }
        }
        if let Some(v)=got.get("unit"){ensure!(v==&want["target_unit"],"MAP_ANCHOR_UNIT: {id}");}
        if let Some(v)=got.get("review_status"){ensure!(nonempty(v),"MAP_RECORD_STATUS: {id}");}
    }
    Ok(())
}
pub(crate) fn validate_full(inventory:&Path,candidate:&Path,paths:&Value,expected:&Value,progress:&mut impl FnMut(usize,usize,&str))->Result<VerifiedMaps>{
    validate_full_inner(inventory,candidate,paths,expected,None,progress)
}
fn validate_full_inner(inventory:&Path,candidate:&Path,paths:&Value,expected:&Value,workflow_manifest:Option<(&Path,&Value)>,
    progress:&mut impl FnMut(usize,usize,&str))->Result<VerifiedMaps>{
    let mut c=Ctx::load(inventory,candidate,expected,true,progress)?;let dest=map_paths(paths,&c.names(),false,&c.bindings)?;
    let mut actual=Map::new();let mut checked=Map::new();let mut deliveries=vec![];
    let mut legacy=BTreeSet::new();
    checked.insert("source_inventory".into(),c.common["source_inventory"].clone());checked.insert("candidate".into(),c.common["candidate"].clone());
    for(name,p)in &dest{
        let (v,b)=c.bindings.json(p)?;ensure!(v["bindings"]==c.common,"MAP_BINDINGS: {name}");
        if name!="link_map"{
            if workflow_manifest.is_some()&&v.get("units").is_none(){legacy.insert(name.as_str());}
            else{for row in rows(&v["units"],"unit")?{
                let d=&row["delivery"];ensure!(d.is_object()&&d["unit"]==row["unit"]&&d["target"]==row["target"],"MAP_DELIVERY_CONFLICT");
                deliveries.push(d.clone());
            }}
        }
        checked.insert(name.clone(),b);actual.insert(name.clone(),v);
    }
    if !legacy.is_empty(){
        ensure!(expected.as_object().is_some_and(|sources|!sources.is_empty()),
            "MAP_LEGACY_SOURCE_SCOPE: verified original workflow sources are required");
        let (manifest_path,bound)=workflow_manifest.unwrap();
        let (manifest,actual)=c.bindings.json(manifest_path)?;
        ensure!(&actual==bound&&manifest["inventory"]==c.common["source_inventory"]&&
            manifest["candidate"]==c.common["candidate"],"MAP_LEGACY_MANIFEST_BINDING");
        let manifest_deliveries=index(&manifest["deliveries"],"unit")?;
        ensure!(manifest_deliveries.keys().eq(c.units.keys()),"MAP_DELIVERY_SET: every unit needs exactly one manifest delivery");
        for d in &deliveries{
            let id=text(&d["unit"],"delivery unit")?;
            let other=manifest_deliveries.get(id).context("MAP_LEGACY_MANIFEST_UNIT")?;
            ensure!(d["target"]==other["target"]&&d["sheet"]==other["sheet"],"MAP_DELIVERY_CONFLICT: {id}");
        }
        let resource_root=if let Some(v)=manifest.get("resource_root"){
            Some(files::canonical(&manifest_path.parent().context("Manifest directory")?.join(path(v)?))?)
        }else{None};
        for(did,delivery)in manifest_deliveries{
            let unit=&c.units[&did];
            let role=&c.sources[unit["source"].as_str().unwrap()]["role"];
            if legacy.contains(if role=="main"{"source_map"}else{"si_map"}){
                let mut delivery=delivery.clone();
                if delivery.get("assets").is_some()&&delivery.get("resource_root").is_none(){
                    if let Some(root)=&resource_root{delivery["resource_root"]=json!(path_text(root)?);}
                }
                deliveries.push(delivery);
            }
        }
    }
    let docs=c.derive(&json!(deliveries),progress)?;
    let mut aliases=BTreeMap::new();
    for name in dest.keys().filter(|n|n.as_str()!="link_map"){
        let got=&actual[name];let want=&docs[name];let compact=legacy.contains(name.as_str());
        check_map_status(got,name,compact)?;
        let block_groups=check_block_partition(&c,&got["blocks"],&want["blocks"],name,compact)?;
        for(id,group)in &block_groups{
            ensure!(aliases.insert(id.clone(),group.clone()).is_none(),"MAP_BLOCK_ID: duplicate view identity {id}");
        }
        let mut covered=BTreeSet::new();
        if compact{
            for bid in block_groups.values().flatten(){
                covered.extend(c.block_units.get(bid).into_iter().flatten().map(String::as_str));
            }
            // This historical shape is recoverable only when no mapped
            // object/call or text/visual decision substitutes for an actual
            // unit record. Their dedicated collections are still exact.
            let mut category_covered=BTreeSet::new();
            for group in ["metadata","objects","references","citations"]{
                if let Some(value)=got.get(group){
                    for r in index(value,"unit")?.into_values(){category_covered.insert(r["unit"].as_str().unwrap());}
                }
            }
            for u in rows(&want["units"],"unit")?{
                let id=text(&u["unit"],"compact manifest unit")?;
                ensure!(covered.contains(id)||category_covered.contains(id),"MAP_UNIT_MISSING: {name}.{id}");
                if u["text_visual_conflicts"].as_array().is_some_and(|r|!r.is_empty()){
                    ensure!(category_covered.contains(id),"MAP_CONFLICT_MISSING: {name}.{id}");
                }
            }
        }
        for(group,key)in [("sources","id"),("units","unit"),("metadata","unit"),("objects","unit"),("references","unit"),("citations","unit")]{
            if compact&&got.get(group).is_none(){
                if group=="units"{continue;}
                if group=="sources"&&name=="source_map"{
                    let proven:BTreeSet<_>=block_groups.values().flatten().map(|id|c.blocks[id].source.as_str()).collect();
                    ensure!(rows(&want["sources"],"id")?.iter().all(|r|r["id"].as_str().is_some_and(|id|proven.contains(id))),
                        "MAP_ROLE_SOURCE_SET: omitted main-source rows require a bound block for each source");
                    continue;
                }
                if group=="metadata"{
                    ensure!(rows(&want["metadata"],"unit")?.iter().all(|r|r["field"]=="title"),
                        "MAP_UNIT_MISSING: omitted compact metadata is supported only for block-covered titles");
                    continue;
                }
            }
            let optional_empty=group=="metadata"||(name=="si_map"&&matches!(group,"objects"|"references"|"citations"));
            check_collection(got,want,group,key,name,optional_empty)?;
        }
    }
    let got=&actual["link_map"];let want=&docs["link_map"];
    check_map_status(got,"link_map",!legacy.is_empty())?;
    if legacy.is_empty(){check_collection(got,want,"anchors","id","link_map",false)?;}
    else{check_compact_anchors(&got["anchors"],&want["anchors"])?;}
    check_collection(got,want,"unresolved","id","link_map",false)?;
    for &group in LINKS{
        if matches!(group,"external_links"|"resource_links")&&got.get(group).is_none()&&want[group].as_array().is_some_and(Vec::is_empty){continue;}
        check_link_origins(&got[group],&want[group],&aliases,&format!("link_map.{group}"))?;
    }
    c.bindings.verify()?;Ok(VerifiedMaps{docs:Value::Object(actual),checked,bindings:c.bindings})
}
pub fn validate(inventory:&Path,candidate:&Path,paths:&Value,expected:&Value,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    validate_with_manifest(inventory,candidate,paths,expected,None,&mut progress)
}
pub(crate) fn validate_with_manifest(inventory:&Path,candidate:&Path,paths:&Value,expected:&Value,
    manifest:Option<(&Path,&Value)>,progress:&mut impl FnMut(usize,usize,&str))->Result<Value>{
    match validate_full_inner(inventory,candidate,paths,expected,manifest,progress){
        Ok(v)=>{
            let mut r=report(v.checked,Map::new(),vec![]);
            r["evidence_files"]=json!(v.bindings.all());Ok(r)
        },Err(e)=>Ok(report(Map::new(),Map::new(),vec![finding(e)])),
    }
}


/// Independently check an explicit delivery set without creating maps.
/// The caller still has to inspect content-specific delivery requirements.
pub(crate) fn derive_for_delivery(inventory:&Path,candidate:&Path,deliveries:&Value,progress:&mut impl FnMut(usize,usize,&str))->Result<(Value,Bindings)>{
    let mut c=Ctx::load(inventory,candidate,&Value::Null,true,progress)?;
    let docs=c.derive(deliveries,progress)?;
    c.bindings.verify()?;Ok((docs,c.bindings))
}
