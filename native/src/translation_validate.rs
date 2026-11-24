//! Translation-review record validation. Resolved locators, scope closure and
//! recorded decisions never establish semantic fidelity or observed actions.
use crate::{files,parse_json,path,path_text,syntax,docx,pdf_structure,translation_prepare};
use anyhow::{anyhow,ensure,Context,Result};
use serde::Serialize;
use serde_json::{json,Map,Value};
use std::{collections::{BTreeMap,BTreeSet,HashMap,HashSet},fs,path::{Path,PathBuf}};

pub const CHECKS:&[&str]=&["alignment","ambiguity","negation","strength","number_changes","terminology","units","symbols","formulas","citation_labels","completeness","directions","citation_claims","table_text","conditions","antecedents","modifier_scope"];
pub const CATEGORIES:&[&str]=&["concept","method","estimator","statistic","metric","procedure","model","parameter","classification","abbreviation"];
pub const KINDS:&[&str]=&["claims","directions","negations","conditions","antecedents","modifiers","strength","numbers","units","symbols","formulas","citation_labels","citation_claims","table_cells"];
pub const STATUSES:&[&str]=&["pending","passed","failed","blocked","not_applicable","unresolved"];
pub const SCANS:&[&str]=&["pending","scanned","not_applicable","blocked","unresolved"];
pub const SEARCHES:&[&str]=&["pending","searched","not_applicable","blocked","unresolved"];
const CHECK_OCC:&[(&str,&str)]=&[("completeness","claims"),("directions","directions"),("negation","negations"),("conditions","conditions"),("antecedents","antecedents"),("modifier_scope","modifiers"),("strength","strength"),("number_changes","numbers"),("units","units"),("symbols","symbols"),("formulas","formulas"),("citation_labels","citation_labels"),("citation_claims","citation_claims"),("table_text","table_cells")];
const STRUCTURE:&[&str]=&["denominators","radicals","exponents","brackets"];
const BOUNDARY:&str="Only recorded scope coverage, required fields, current file hashes and resolvable locations are checked. Source PDF quotes, meaning, applicability, term choice, exhaustiveness within a paragraph and actual review/search actions require independent source comparison. records_ready never certifies a translation or grants any atomic-ID pass.";
pub fn fields(kind:&str)->&'static[&'static str]{match kind{
    "claims"=>&["source_claim","candidate_claim"],"directions"=>&["source_relation","candidate_relation"],
    "negations"=>&["source_scope","candidate_scope"],"conditions"=>&["source_condition","candidate_condition"],
    "antecedents"|"modifiers"=>&["source_candidates","candidate_candidates"],"strength"=>&["source_strength","candidate_strength"],
    "numbers"=>&["source_expression","candidate_expression","difference_type"],"units"=>&["source_expression","candidate_expression","source_quantity","candidate_quantity"],
    "symbols"=>&["source_expression","candidate_expression","source_definition","candidate_definition"],
    "formulas"=>&["source_expression","candidate_expression","source_structure","candidate_structure"],
    "citation_labels"=>&["source_label","candidate_label","source_target","candidate_target"],
    "citation_claims"=>&["source_claim","candidate_claim","source_target","candidate_target"],"table_cells"=>&["source_cell","candidate_cell"],_=>&[]
}}
fn nonempty(v:&Value)->bool{v.as_str().is_some_and(|s|!s.trim().is_empty())}
fn filled(v:&Value)->bool{match v{Value::String(s)=>!s.trim().is_empty(),Value::Array(a)=>!a.is_empty()&&a.iter().all(filled),Value::Object(m)=>!m.is_empty()&&m.values().all(filled),Value::Number(n)=>n.as_f64().is_some_and(f64::is_finite),_=>false}}
fn string(v:&Value)->Option<&str>{v.as_str().filter(|s|!s.trim().is_empty())}
fn list(v:&Value)->&[Value]{v.as_array().map(Vec::as_slice).unwrap_or(&[])}
fn box_values(v:&Value)->Option<[f64;4]>{
    let a=v.as_array().filter(|a|a.len()==4)?;
    let b=[a[0].as_f64()?,a[1].as_f64()?,a[2].as_f64()?,a[3].as_f64()?];
    b.iter().all(|n|n.is_finite()).then_some(b)
}
fn contains_box(a:[f64;4],b:[f64;4],tol:f64)->bool{a[0]-tol<=b[0]&&b[0]<b[2]&&b[2]<=a[2]+tol&&a[1]-tol<=b[1]&&b[1]<b[3]&&b[3]<=a[3]+tol}
fn norm(s:&str)->String{s.split_whitespace().collect::<Vec<_>>().join(" ")}
pub fn pointer<'a>(v:&'a Value,p:&Value)->Result<&'a Value>{
    let s=p.as_str().filter(|s|s.starts_with('/')).context("Use a non-root JSON Pointer starting with /.")?;
    let mut value=v;
    for segment in s[1..].split('/'){
        let mut key=String::new();let mut chars=segment.chars();
        while let Some(c)=chars.next(){if c!='~'{key.push(c);continue;}match chars.next(){Some('0')=>key.push('~'),Some('1')=>key.push('/'),_=>return Err(anyhow!("Invalid JSON Pointer escape."))}}
        value=if let Some(m)=value.as_object(){m.get(&key)}else if let Some(a)=value.as_array(){
            if key=="0"||(!key.is_empty()&&!key.starts_with('0')&&key.bytes().all(|b|b.is_ascii_digit())){key.parse::<usize>().ok().and_then(|i|a.get(i))}else{None}
        }else{None}.context("JSON Pointer does not identify an existing value.")?;
    }Ok(value)
}
#[derive(Clone,Serialize)]
pub struct Finding{pub code:String,pub object_id:String,pub message:String,pub level:String}
#[derive(Default,Serialize)]
pub struct Report{
    pub findings:Vec<Finding>,pub inventory_unit_count:usize,pub selected_unit_count:usize,pub recorded_unit_count:usize,pub excluded_unit_count:usize,
    pub status_counts:BTreeMap<String,usize>,pub pending_ids:Vec<String>,pub unresolved_ids:Vec<String>,pub failed_ids:Vec<String>,pub review_status:Option<String>,
}
impl Report{
    pub fn ready(&self)->bool{self.review_status.as_deref()==Some("reviewed")&&self.selected_unit_count>0&&self.part_ready()}
    pub fn part_ready(&self)->bool{!self.findings.iter().any(|f|f.level=="error")&&self.pending_ids.is_empty()&&self.failed_ids.is_empty()&&self.unresolved_ids.is_empty()}
    pub fn value(&self)->Result<Value>{let mut out=serde_json::to_value(self)?;out["records_ready"]=json!(self.ready());
        out["source_fidelity"]=json!("not_assessed");out["execution_truth"]=json!("not_assessed");out["independent_review_required"]=json!(true);out["evidence_boundary"]=json!(BOUNDARY);Ok(out)}
    fn finding(&mut self,code:&str,id:&str,msg:impl Into<String>,level:&str){self.findings.push(Finding{code:code.into(),object_id:id.into(),message:msg.into(),level:level.into()});}
}
#[derive(Default)]
pub struct Cache{hashes:HashMap<PathBuf,String>,texts:HashMap<PathBuf,String>,jsons:HashMap<PathBuf,Value>,pdf:HashMap<PathBuf,pdf_structure::Pdf>,native:HashMap<PathBuf,docx::NativeDocument>}
impl Cache{
    pub fn hash(&mut self,p:&Path)->Result<String>{
        if let Some(h)=self.hashes.get(p){return Ok(h.clone());}
        let h=files::snapshot(p)?["sha256"].as_str().context("hash")?.to_owned();self.hashes.insert(p.to_owned(),h.clone());Ok(h)
    }
    pub fn text(&mut self,p:&Path)->Result<&str>{
        if !self.texts.contains_key(p){
            let bytes=fs::read(p)?;let h=files::digest(&bytes);
            if let Some(before)=self.hashes.get(p){ensure!(*before==h,"Evidence bytes changed during validation");}
            self.hashes.insert(p.to_owned(),h);self.texts.insert(p.to_owned(),String::from_utf8(bytes)?);
        }Ok(&self.texts[p])
    }
    fn json(&mut self,p:&Path)->Result<Value>{
        if let Some(v)=self.jsons.get(p){return Ok(v.clone());}
        let v=parse_json(self.text(p)?)?;self.jsons.insert(p.to_owned(),v.clone());Ok(v)
    }
    pub fn pdf(&mut self,p:&Path)->Result<&pdf_structure::Pdf>{
        if !self.pdf.contains_key(p){self.hash(p)?;self.pdf.insert(p.to_owned(),pdf_structure::Pdf::open(p)?);}Ok(&self.pdf[p])
    }
    pub fn native(&mut self,p:&Path)->Result<&docx::NativeDocument>{
        if !self.native.contains_key(p){self.hash(p)?;self.native.insert(p.to_owned(),docx::NativeDocument::new(p)?);}Ok(&self.native[p])
    }
    pub fn verify_current(&self)->Result<()>{
        for(p,h)in &self.hashes{ensure!(files::snapshot(p)?["sha256"]==h.as_str(),"Bound bytes changed during validation: {}",p.display());}Ok(())
    }
}
pub fn text_range(text:&str,loc:&Value)->Result<(usize,usize)>{
    ensure!(loc.is_object(),"Use an actual text locator");
    let kind=loc["kind"].as_str();ensure!(loc["kind"].is_null()||matches!(kind,Some("lines"|"quote")),"Unsupported text locator kind");
    if loc.get("lines").is_some()||kind==Some("lines"){
        let bounds=loc.get("lines").cloned().unwrap_or_else(||json!([loc["start"],loc["end"]]));
        let a=bounds.as_array().filter(|a|a.len()==2).context("Use lines: [start, end], with positive integer line numbers.")?;
        let (start,end)=a[0].as_u64().zip(a[1].as_u64()).context("Use lines: [start, end], with positive integer line numbers.")?;
        let lines=syntax::lines(text);
        ensure!(start>0&&start<=end&&end<=lines.len() as u64,"Line range is outside the actual UTF-8 file.");
        let begin=lines[start as usize-1].start;let finish=lines[end as usize-1].end;
        let end=begin+text[begin..finish].trim_end_matches(['\r','\n']).len();
        if let Some(q)=loc.get("quote"){ensure!(string(q).is_some_and(|s|text[begin..end].contains(s)),"The supplied exact quote is absent from its recorded lines.");}
        return Ok((begin,end))
    }
    let q=string(&loc["quote"]).context("Use a line range or a unique exact quote, not an unlocated claim.")?;
    let mut matches=text.match_indices(q);let start=matches.next().context("Quote-only location must occur exactly once; add explicit lines for repeated text.")?.0;
    ensure!(matches.next().is_none(),"Quote-only location must occur exactly once; add explicit lines for repeated text.");
    Ok((start,start+q.len()))
}
impl Cache{
    pub fn location(&mut self,p:&Path,v:&Value)->Result<Option<(usize,usize)>>{
        ensure!(v.as_object().is_some_and(|m|!m.is_empty()),"Supply a concrete source/candidate/evidence location.");
        match v["kind"].as_str(){
            Some("json_pointer")=>{pointer(&self.json(p)?,&v["pointer"])?;return Ok(None);}
            Some("image_region")=>{
                self.hash(p)?;let(w,h)=image::ImageReader::open(p)?.with_guessed_format()?.into_dimensions()?;
                let b=box_values(&v["bbox"]).context("Use four finite numeric bbox coordinates.")?;
                ensure!(contains_box([0.,0.,w as f64,h as f64],b,0.01),"The bbox is outside the source page/image or has reversed bounds.");
                return Ok(None)
            }
            Some("pdf_pages")=>{
                let(start,end)=v["start"].as_u64().zip(v["end"].as_u64()).context("PDF page range is outside the actual document.")?;
                ensure!(start>0&&start<=end&&end<=self.pdf(p)?.len() as u64,"PDF page range is outside the actual document.");return Ok(None)
            }
            _=>{}
        }
        if v.get("native_locator").is_some(){
            ensure!(["page","bbox","coordinate_system"].iter().all(|k|v.get(k).is_none()),"Native OOXML locations cannot invent PDF page coordinates.");
            let actual=self.native(p)?.resolve(&v["native_locator"])?;
            if let Some(q)=v.get("quote"){ensure!(q.as_str().is_some_and(|q|actual.contains(q)),"The quote is absent from its actual native XML block.");}
            return Ok(None)
        }
        if let Some(n)=v.get("page"){
            let n=n.as_u64().filter(|n|*n>0).context("Source page must be a positive page in the actual PDF.")?;
            let bounds=self.pdf(p)?.bounds(n)?;
            if let Some(b)=v.get("bbox"){
                let b=box_values(b).context("Use four finite numeric bbox coordinates.")?;
                ensure!(contains_box(bounds,b,0.01),"The bbox is outside the source page/image or has reversed bounds.");
            }else{ensure!(nonempty(&v["quote"]),"A PDF source span needs a bbox or a nonempty original quote.");}
            return Ok(None)
        }
        Ok(Some(text_range(self.text(p)?,v)?))
    }
}

pub struct Validator{
    pub report:Report,pub base:PathBuf,pub cache:Cache,pub units:BTreeMap<String,Value>,pub sources:BTreeMap<String,Value>,
    pub source_paths:HashMap<String,PathBuf>,pub candidate:Option<PathBuf>,pub glossary:Option<PathBuf>,pub terminology:Value,
    selected:BTreeSet<String>,unresolved:BTreeSet<(String,String)>,term_mappings:BTreeSet<(String,String)>,
}
impl Validator{
    pub fn new(base:&Path)->Self{Self{report:Report::default(),base:base.into(),cache:Cache::default(),units:BTreeMap::new(),sources:BTreeMap::new(),
        source_paths:HashMap::new(),candidate:None,glossary:None,terminology:json!({}),selected:BTreeSet::new(),unresolved:BTreeSet::new(),term_mappings:BTreeSet::new()}}
    fn fail(&mut self,c:&str,id:&str,msg:impl Into<String>){self.report.finding(c,id,msg,"error")}
    fn warning(&mut self,c:&str,id:&str,msg:impl Into<String>){self.report.finding(c,id,msg,"warning")}
    fn object(&mut self,v:&Value,id:&str)->Map<String,Value>{
        if let Some(m)=v.as_object(){return m.clone();}self.fail("TRN_OBJECT_REQUIRED",id,"Record an object with the documented fields.");Map::new()
    }
    fn rows<'a>(&mut self,v:&'a Value,id:&str)->&'a[Value]{
        if let Some(a)=v.as_array(){return a;}self.fail("TRN_LIST_REQUIRED",id,"Record a list; an absent field is not an empty reviewed set.");&[]
    }
    fn ids(&mut self,v:&Value,id:&str,required:bool,unique:bool)->Vec<String>{
        let mut out=Vec::new();let mut seen=HashSet::new();
        for(i,v)in self.rows(v,id).iter().enumerate(){
            let Some(s)=string(v)else{self.fail("TRN_ID_INVALID",&format!("{id}/{i}"),"Use a nonempty string ID.");continue;};
            if unique&&!seen.insert(s){self.fail("TRN_ID_DUPLICATE",id,format!("ID occurs more than once: {s}"));continue;}
            out.push(s.to_owned());
        }
        if required&&out.is_empty(){self.fail("TRN_IDS_EMPTY",id,"List the actual nonempty scope explicitly.");}out
    }
    fn index(&mut self,v:&Value,id:&str)->BTreeMap<String,Value>{
        let mut out=BTreeMap::new();
        for(i,v)in self.rows(v,id).iter().enumerate(){
            let row=self.object(v,&format!("{id}/{i}"));let row=Value::Object(row);
            let Some(s)=string(&row["id"]).map(str::to_owned)else{self.fail("TRN_ID_INVALID",&format!("{id}/{i}"),"Use a nonempty string id.");continue;};
            if out.contains_key(&s){self.fail("TRN_ID_DUPLICATE",id,format!("Duplicate id: {s}"));continue;}
            out.insert(s,row);
        }out
    }
    fn read(&mut self,p:&Path,id:&str)->Value{match self.cache.json(p){Ok(v)=>v,Err(e)=>{self.fail("TRN_JSON_UNREADABLE",id,format!("Cannot read a unique-key UTF-8 JSON file: {e}"));Value::Null}}}
    fn binding(&mut self,v:&Value,id:&str,base:&Path,expected:Option<&Path>)->Option<PathBuf>{
        self.object(v,id);
        let Some(s)=string(&v["path"])else{self.fail("TRN_FILE_PATH_MISSING",id,"Record the actual file path.");return None;};
        let p=PathBuf::from(s);let p=if p.is_absolute(){p}else{base.join(p)};
        let p=match files::canonical(&p){Ok(p)=>p,Err(e)=>{self.fail("TRN_FILE_UNREADABLE",id,e.to_string());return None;}};
        let h=match self.cache.hash(&p){Ok(h)=>h,Err(e)=>{self.fail("TRN_FILE_UNREADABLE",id,e.to_string());return None;}};
        let expected_hash=v["sha256"].as_str().filter(|s|s.len()==64&&s.bytes().all(|b|b.is_ascii_digit()||matches!(b,b'a'..=b'f')));
        match expected_hash{None=>self.fail("TRN_HASH_MISSING",id,"Record the real lowercase SHA-256 digest."),Some(s) if s!=h=>self.fail("TRN_HASH_CHANGED",id,format!("Bound bytes differ from the current file: {}",p.display())),_=>{}}
        if expected.is_some_and(|q|q!=p){self.fail("TRN_BINDING_IDENTITY",id,"Bind the requested file, not a copy or alternate path.");}Some(p)
    }
    fn location(&mut self,p:Option<&Path>,v:&Value,id:&str)->Option<(usize,usize)>{
        let p=p?;
        if !v.is_object(){self.object(v,id);}
        if !v.as_object().is_some_and(|m|!m.is_empty()){self.fail("TRN_LOCATION_MISSING",id,"Supply a concrete source/candidate/evidence location.");return None;}
        match self.cache.location(p,v){Ok(r)=>r,Err(e)=>{self.fail("TRN_LOCATION_INVALID",id,e.to_string());None}}
    }
    fn evidence(&mut self,v:&Value,id:&str,required:bool){
        let rows=self.rows(v,id);if required&&rows.is_empty(){self.fail("TRN_EVIDENCE_MISSING",id,"Attach actual file/hash/location evidence for this recorded conclusion.");}
        let base=self.base.clone();
        for(i,row)in rows.iter().enumerate(){let where_=format!("{id}/{i}");let p=self.binding(row,&where_,&base,None);self.location(p.as_deref(),&row["locator"],&format!("{where_}/locator"));}
    }
    fn state(&mut self,v:&Value,id:&str,allowed:&[&str])->Option<String>{
        self.object(v,id);
        let Some(status)=v["status"].as_str().filter(|s|allowed.contains(s))else{self.fail("TRN_STATUS_INVALID",id,"Use a documented explicit status.");return None;};
        *self.report.status_counts.entry(status.into()).or_default()+=1;
        if status=="pending"{
            self.report.pending_ids.push(id.into());self.warning("TRN_PENDING",id,"This item still requires an explicit source-based decision.");
            if v["evidence"].as_array().is_some_and(|a|!a.is_empty()){self.evidence(&v["evidence"],&format!("{id}/evidence"),false);}
            return Some(status.into())
        }
        if !nonempty(&v["reason"]){self.fail("TRN_REASON_MISSING",id,"Explain this single conclusion; for not_applicable name the actual condition and scope.");}
        self.evidence(&v["evidence"],&format!("{id}/evidence"),true);
        if matches!(status,"blocked"|"unresolved"){self.report.unresolved_ids.push(id.into());self.warning("TRN_UNRESOLVED",id,format!("Recorder reports {status}; retain the issue until it is resolved."));}
        if status=="failed"{self.report.failed_ids.push(id.into());self.warning("TRN_RECORDED_FAILURE",id,"Recorder reports a failed comparison; no automatic semantic judgment was made.");}
        Some(status.into())
    }
    fn expected(&mut self,v:&Value,keys:&[&str],id:&str)->Map<String,Value>{
        let m=self.object(v,id);
        for k in keys{if !m.contains_key(*k){self.fail("TRN_TARGET_MISSING",&format!("{id}/{k}"),"Restore this required individual target from the pending scaffold.");}}
        for k in m.keys(){if !keys.contains(&k.as_str()){self.fail("TRN_TARGET_UNKNOWN",&format!("{id}/{k}"),"Unknown target name; do not replace a required target with a spelling variant.");}}
        m
    }
    fn fields(&mut self,row:&Value,keys:&[&str],id:&str){
        for key in keys{
            let v=&row[*key];
            let present=if ["source_structure","candidate_structure"].contains(key)&&v.is_object(){v.as_object().is_some_and(|m|!m.is_empty())}
            else if ["original","translation","context","source_excerpt","entry_id","excerpt","query","interpretation","reason","resolution_basis","description","disposition","performed_at","action","check","table_id","text","subject","reference","relation","source_expression","candidate_expression","source_label","candidate_label"].contains(key){nonempty(v)}
            else{filled(v)};
            if !present{self.fail("TRN_FIELD_MISSING",&format!("{id}/{key}"),"Record this occurrence's explicit source/candidate comparison field.");}
        }
    }
    pub fn scope(&mut self,v:&Value,review:&Map<String,Value>){
        self.object(v,"scope");
        let selected=self.ids(&v["selected_units"],"scope/selected_units",true,true);
        self.report.selected_unit_count=selected.len();self.selected=selected.into_iter().collect();
        self.state(&v["selection_review"],"scope/selection_review",STATUSES);
        for id in self.selected.clone(){
            if !self.units.contains_key(&id){self.fail("TRN_UNIT_UNKNOWN",&id,"Selected ID is absent from the bound source inventory.");}
            if !review.contains_key(&id){self.fail("TRN_UNIT_MISSING",&id,"Selected source unit has no independent translation-review record.");}
        }
        for id in review.keys(){if !self.selected.contains(id){self.fail("TRN_UNIT_OUTSIDE_SCOPE",id,"A review unit must be explicitly selected in scope.");}}
        let expected:BTreeSet<_>=self.units.keys().filter(|s|!self.selected.contains(*s)).cloned().collect();
        let unselected:BTreeSet<_>=self.ids(&v["unselected_inventory_units"],"scope/unselected_inventory_units",false,true).into_iter().collect();
        if expected!=unselected{self.fail("TRN_UNSELECTED_INVENTORY_MISMATCH","scope","Keep an exact list of inventory IDs outside selected_units; do not silently drop them.");}
        let excluded=self.index(&v["excluded_units"],"scope/excluded_units");self.report.excluded_unit_count=excluded.len();
        for(id,row)in &excluded{
            if !expected.contains(id){self.fail("TRN_EXCLUSION_CONFLICT",id,"An excluded ID must be a known, unselected inventory unit.");}
            for k in ["applicability_condition","reason"]{if !nonempty(&row[k]){self.fail("TRN_EXCLUSION_REASON",id,format!("Record {k} from actual language/task scope; unselected is not an applicability decision."));}}
            self.evidence(&row["evidence"],&format!("scope/excluded_units/{id}/evidence"),true);
        }
        for id in expected.iter().filter(|s|!excluded.contains_key(*s)){self.fail("TRN_SCOPE_UNACCOUNTED",id,"Inventory unit is neither selected nor explicitly excluded with condition and evidence. Review its applicability; do not infer it from kind or keywords.");}
    }
    pub fn source_span(&mut self,sid:&str,v:&Value,unit:&Value,id:&str){
        let p=self.source_paths.get(sid).cloned();self.location(p.as_deref(),v,id);
        if !v.is_object(){return;}
        let Some(locs)=unit["source_locations"].as_array()else{return;};
        if self.sources.get(sid).is_some_and(|s|s["format"]=="docx"){
            let matched:Vec<_>=locs.iter().chain(locs.iter().flat_map(|l|list(&l["block_locations"])))
                .filter(|l|v["native_locator"].is_object()&&l["native_locator"]==v["native_locator"]).collect();
            if matched.is_empty(){self.fail("TRN_SOURCE_SPAN_OUTSIDE_UNIT",id,"This native XML occurrence is outside the recorded source unit.");}
            else if let Some(q)=string(&v["quote"]){if !matched.iter().any(|l|l["quote"].as_str().is_some_and(|s|s.contains(q))){self.fail("TRN_SOURCE_SPAN_OUTSIDE_UNIT",id,"The exact quote is outside the native text recorded for this unit.");}}
            return;
        }
        if v["page"].as_i64().is_some()&&!locs.iter().any(|l|l["page"]==v["page"]){self.fail("TRN_SOURCE_SPAN_OUTSIDE_UNIT",id,"This occurrence is not on a page belonging to its source unit.");return;}
        let same:Vec<_>=locs.iter().filter(|l|l["page"]==v["page"]).collect();
        if same.is_empty(){return;}
        if let Some(b)=box_values(&v["bbox"]){
            let parents:Vec<_>=same.iter().filter_map(|l|box_values(&l["bbox"])).collect();
            if !parents.is_empty()&&!parents.iter().any(|a|contains_box(*a,b,0.)){self.fail("TRN_SOURCE_SPAN_OUTSIDE_UNIT",id,"Occurrence bbox is outside this source unit's recorded boxes.");}return;
        }
        if same.iter().any(|l|box_values(&l["bbox"]).is_some_and(|b|b[0]<b[2]&&b[1]<b[3])){return;}
        let quotes:Vec<_>=same.iter().filter_map(|l|string(&l["quote"])).collect();
        if let Some(q)=string(&v["quote"]){
            if !quotes.is_empty()&&!quotes.iter().any(|s|norm(s).contains(&norm(q))){self.fail("TRN_SOURCE_SPAN_OUTSIDE_UNIT",id,"Quote-only occurrence is absent from this unit's recorded original quote; use the actual source span or its visible bbox.");}
        }
    }
    fn candidate_span(&mut self,v:&Value,ranges:&[(usize,usize)],id:&str){
        let p=self.candidate.clone();
        if let Some((a,b))=self.location(p.as_deref(),v,id){if !ranges.is_empty()&&!ranges.iter().any(|(x,y)|*x<=a&&a<=b&&b<=*y){self.fail("TRN_CANDIDATE_SPAN_OUTSIDE_UNIT",id,"Occurrence location must fall inside this unit's recorded candidate locations.");}}
    }
    pub fn occurrences(&mut self,v:&Value,unit:&Value,sid:&str,ranges:&[(usize,usize)],id:&str,checks:&Map<String,Value>){
        let groups=self.expected(v,KINDS,&format!("{id}/occurrences"));let mut all=HashSet::new();
        for kind in KINDS{
            let Some(rows)=groups.get(*kind)else{continue;};
            let prefix=format!("{id}/occurrences/{kind}");
            for(key,row)in self.index(rows,&prefix){
                let loc=format!("{prefix}/{key}");
                if !all.insert(key.clone()){self.fail("TRN_OCCURRENCE_DUPLICATE",&loc,"Use a distinct occurrence ID within this unit, including across categories.");}
                self.state(&row,&loc,STATUSES);self.source_span(sid,&row["source_span"],unit,&format!("{loc}/source_span"));
                self.candidate_span(&row["candidate_span"],ranges,&format!("{loc}/candidate_span"));self.fields(&row,fields(kind),&loc);
                match *kind{
                    "directions"=>for side in ["source_relation","candidate_relation"]{self.object(&row[side],&format!("{loc}/{side}"));self.fields(&row[side],&["subject","reference","relation"],&format!("{loc}/{side}"));},
                    "antecedents"|"modifiers"=>for side in ["source_candidates","candidate_candidates"]{self.ids(&row[side],&format!("{loc}/{side}"),true,true);},
                    "numbers"=>if !row["difference_type"].as_str().is_some_and(|s|["unchanged","representation_only","value_changed","structure_changed","unresolved"].contains(&s)){self.fail("TRN_NUMBER_DIFFERENCE_TYPE",&loc,"Classify the numeric representation/value/structure difference explicitly.");},
                    "formulas"=>for side in ["source_structure","candidate_structure"]{
                        let structure=self.expected(&row[side],STRUCTURE,&format!("{loc}/{side}"));
                        for key in STRUCTURE{if let Some(v)=structure.get(*key){self.ids(v,&format!("{loc}/{side}/{key}"),false,false);}}
                    },
                    "table_cells"=>for side in ["source_cell","candidate_cell"]{
                        self.object(&row[side],&format!("{loc}/{side}"));self.fields(&row[side],&["table_id","text"],&format!("{loc}/{side}"));
                        if ["row","column"].iter().any(|k|!row[side][*k].as_u64().is_some_and(|n|n>0)){self.fail("TRN_TABLE_CELL_LOCATION",&format!("{loc}/{side}"),"Record positive row and column numbers for this cell.");}
                    },_=>{}
                }
            }
        }
        for &(check,kind) in CHECK_OCC{
            let Some(row)=checks.get(check).filter(|r|r.is_object())else{continue;};let Some(group)=groups.get(kind).and_then(Value::as_array)else{continue;};
            if row["status"]=="passed"&&group.is_empty(){self.fail("TRN_OCCURRENCES_MISSING",&format!("{id}/checks/{check}"),format!("Passed {check} needs independent occurrences.{kind} records; an empty list does not prove coverage."));}
            if row["status"]=="not_applicable"&&!group.is_empty(){self.fail("TRN_APPLICABILITY_CONFLICT",&format!("{id}/checks/{check}"),"An absent category cannot also register occurrences.");}
        }
    }
    fn hit(&mut self,v:&Value,id:&str){
        self.object(v,id);self.fields(v,&["entry_id","excerpt"],id);let mut loc=v["glossary_locator"].clone();
        if loc.is_object()&&nonempty(&v["excerpt"]){loc["quote"]=v["excerpt"].clone();}
        let p=self.glossary.clone();self.location(p.as_deref(),&loc,&format!("{id}/glossary_locator"));
    }
    fn term(&mut self,row:&Value,term:&str,unit_id:&str,unit:&Value,sid:&str){
        let id=format!("units/{unit_id}/term_candidates/{term}");self.fields(row,&["original","source_excerpt"],&id);
        if !row["category"].as_str().is_some_and(|s|CATEGORIES.contains(&s)){self.fail("TRN_TERM_CATEGORY",&id,"Use a category from the scaffold's TERM_CATEGORIES.");}
        self.source_span(sid,&row["source_locator"],unit,&format!("{id}/source_locator"));
        let aliases=self.ids(&row["aliases"],&format!("{id}/aliases"),false,true);
        let modes=["full_phrase","aliases","family"];let searches=self.expected(&row["searches"],&modes,&format!("{id}/searches"));
        for mode in modes{
            let Some(s)=searches.get(mode)else{continue;};let loc=format!("{id}/searches/{mode}");self.object(s,&loc);
            let status=self.state(s,&loc,SEARCHES);let queries=self.rows(&s["queries"],&format!("{loc}/queries"));
            if (mode!="aliases"||!aliases.is_empty())&&status.as_deref()==Some("not_applicable"){self.fail("TRN_TERM_SEARCH_REQUIRED",&loc,"Full phrase and family searches always need actual queries; aliases need them when aliases exist.");}
            if status.as_deref()==Some("searched")&&queries.is_empty(){self.fail("TRN_TERM_QUERIES_MISSING",&loc,"Record actual queries, including queries with zero hits.");}
            if status.as_deref()==Some("not_applicable")&&!queries.is_empty(){self.fail("TRN_APPLICABILITY_CONFLICT",&loc,"A not-applicable search must not also claim performed queries.");}
            for(i,q)in queries.iter().enumerate(){
                let qid=format!("{loc}/queries/{i}");self.object(q,&qid);self.fields(q,&["query","interpretation"],&qid);
                for(j,hit)in self.rows(&q["hits"],&format!("{qid}/hits")).iter().enumerate(){self.hit(hit,&format!("{qid}/hits/{j}"));}
            }
        }
        let Some(decision)=row["decision"].as_str().filter(|s|["pending","adopted","unlisted","unresolved","not_a_term"].contains(s))else{self.fail("TRN_TERM_DECISION",&id,"Record one explicit term decision from the documented states.");return;};
        if decision=="pending"{self.report.pending_ids.push(id.clone());self.warning("TRN_PENDING",&id,"Term choice remains pending.");return;}
        if !nonempty(&row["decision_reason"]){self.fail("TRN_TERM_DECISION_REASON",&id,"Explain this term's contextual choice or exclusion.");}
        let matches=self.rows(&row["glossary_matches"],&format!("{id}/glossary_matches"));
        for(i,h)in matches.iter().enumerate(){self.hit(h,&format!("{id}/glossary_matches/{i}"));}
        if decision=="adopted"&&matches.is_empty(){self.fail("TRN_GLOSSARY_MATCH_MISSING",&id,"An adopted same-concept entry needs an actual glossary match and location.");}
        if decision=="not_a_term"{return;}
        if decision=="unresolved"{self.unresolved.insert((unit_id.into(),term.into()));self.report.unresolved_ids.push(id.clone());self.warning("TRN_UNRESOLVED",&id,"Term meaning/choice remains explicitly unresolved.");}
        else if !nonempty(&row["translation"]){self.fail("TRN_TERM_TRANSLATION",&id,"Record the actual chosen translation.");}
        let target=match pointer(&self.terminology,&row["terminology_pointer"]){Ok(v)=>v.clone(),Err(e)=>{self.fail("TRN_TERMINOLOGY_POINTER",&id,e.to_string());return;}};
        let section=if decision=="adopted"{"adopted"}else{"unlisted"};
        let p=row["terminology_pointer"].as_str().unwrap_or("");
        if p.strip_prefix(&format!("/{section}/")).is_some_and(|s|s=="0"||(!s.is_empty()&&!s.starts_with('0')&&s.bytes().all(|b|b.is_ascii_digit()))){self.term_mappings.insert((unit_id.into(),p.into()));}
        else{self.fail("TRN_TERMINOLOGY_POINTER",&id,format!("Point at one actual /{section}/<index> entry, not the whole ledger."));}
        self.object(&target,&format!("{id}/terminology_pointer"));
        if target["original"]!=row["original"]||target["translation"]!=row["translation"]{self.fail("TRN_TERM_MAPPING",&id,"Original and translation must agree with the pointed terminology entry.");}
        if !target["source_units"].as_array().is_some_and(|a|a.iter().any(|v|v==unit_id)){self.fail("TRN_TERM_MAPPING",&id,"The terminology entry must include this source unit ID.");}
        if decision=="unresolved"&&target["status"]!="unresolved"{self.fail("TRN_TERM_MAPPING",&id,"Unresolved term must remain unresolved in the terminology ledger.");}
        if decision=="unlisted"&&target["status"]!="resolved"{self.fail("TRN_TERM_MAPPING",&id,"A decided unlisted term needs a resolved entry with its actual basis.");}
    }
    fn term_scan(&mut self,v:&Value,candidates:&BTreeMap<String,Value>,id:&str){
        let scans=self.expected(v,CATEGORIES,&format!("{id}/term_scan"));let mut referenced=HashSet::new();
        for c in CATEGORIES{
            let Some(row)=scans.get(*c)else{continue;};let loc=format!("{id}/term_scan/{c}");
            self.object(row,&loc);let state=self.state(row,&loc,SCANS);let ids=self.ids(&row["candidate_ids"],&format!("{loc}/candidate_ids"),false,true);
            for term in &ids{if !candidates.contains_key(term){self.fail("TRN_TERM_CANDIDATE_UNKNOWN",&loc,format!("Unknown term candidate ID: {term}"));}referenced.insert(term.clone());}
            if state.as_deref()==Some("scanned")&&ids.is_empty(){self.fail("TRN_TERM_SCAN_EMPTY",&loc,"A scanned category needs its candidates.");}
            if state.as_deref()==Some("not_applicable")&&!ids.is_empty(){self.fail("TRN_APPLICABILITY_CONFLICT",&loc,"Category has candidate IDs while claiming no applicable terms.");}
        }
        for (term,row) in candidates{
            let loc=format!("{id}/term_candidates/{term}");
            if !referenced.contains(term){self.fail("TRN_TERM_CANDIDATE_UNSCANNED",&loc,"Reference every registered candidate from its actual category scans.");}
            if let Some(scan)=row["category"].as_str().and_then(|s|scans.get(s)){
                if let Some(a)=scan["candidate_ids"].as_array(){if !a.iter().any(|s|s==term){self.fail("TRN_TERM_CATEGORY_MAPPING",&loc,"Candidate is missing from its declared primary category scan.");}}
            }
        }
    }
    pub fn unit(&mut self,uid:&str,row:&Value){
        let id=format!("units/{uid}");self.object(row,&id);
        let Some(original)=self.units.get(uid).cloned()else{self.fail("TRN_UNIT_UNKNOWN",&id,"Unit ID is absent from the bound inventory.");return;};
        let sid=original["source"].as_str().unwrap_or("").to_owned();
        if row["source_id"]!=sid{self.fail("TRN_UNIT_SOURCE",&id,"Use the source identity assigned to this inventory unit.");}
        if row["kind"]!=original["kind"]{self.fail("TRN_UNIT_KIND",&id,"Preserve the inventory unit kind.");}
        let locs=self.rows(&row["source_locations"],&format!("{id}/source_locations"));
        let source=self.sources.get(&sid).cloned().unwrap_or(json!({}));
        let expected=match translation_prepare::expand(&original,&source){Ok(v)=>v,Err(e)=>{self.fail("TRN_INVENTORY_SOURCE_LOCATION",&id,e.to_string());Value::Null}};
        if locs.is_empty()||row["source_locations"]!=expected{self.fail("TRN_SOURCE_LOCATIONS_CHANGED",&id,"Retain all original inventory locations in order, including cross-page spans.");}
        let p=self.source_paths.get(&sid).cloned();
        for(i,l)in locs.iter().enumerate(){self.location(p.as_deref(),l,&format!("{id}/source_locations/{i}"));}
        let targets=self.rows(&row["candidate_locations"],&format!("{id}/candidate_locations"));if targets.is_empty(){self.fail("TRN_CANDIDATE_LOCATION_MISSING",&id,"Locate this unit's actual complete candidate text.");}
        let candidate=self.candidate.clone();let mut ranges=Vec::new();
        for(i,l)in targets.iter().enumerate(){if let Some(r)=self.location(candidate.as_deref(),l,&format!("{id}/candidate_locations/{i}")){ranges.push(r);}}
        let checks=self.expected(&row["checks"],CHECKS,&format!("{id}/checks"));
        for c in CHECKS{if let Some(v)=checks.get(*c){self.state(v,&format!("{id}/checks/{c}"),STATUSES);}}
        self.occurrences(&row["occurrences"],row,&sid,&ranges,&id,&checks);
        let candidates=self.index(&row["term_candidates"],&format!("{id}/term_candidates"));
        self.term_scan(&row["term_scan"],&candidates,&id);
        for(term,value)in candidates{self.term(&value,&term,uid,row,&sid);}
    }
    fn terminology_record(&mut self,v:&Value){
        let base=self.base.clone();let Some(p)=self.binding(v,"terminology_record",&base,None)else{return;};
        let ledger=self.read(&p,"terminology_record");self.object(&ledger,"terminology_record");self.terminology=ledger.clone();
        if ledger["schema_version"].as_u64()!=Some(1){self.fail("TRN_TERMINOLOGY_VERSION","terminology_record","Use terminology schema_version 1.");}
        let glossary=self.glossary.clone();self.binding(&ledger["glossary"],"terminology_record/glossary",p.parent().unwrap(),glossary.as_deref());
        if ledger["review_status"]!="reviewed"{self.report.pending_ids.push("terminology_record/review_status".into());self.warning("TRN_TERMINOLOGY_PENDING","terminology_record","Finish the actual terminology review before declaring its records reviewed.");}
        let covered:BTreeSet<_>=self.ids(&ledger["covered_units"],"terminology_record/covered_units",false,true).into_iter().collect();
        if !self.selected.is_subset(&covered)||covered.iter().any(|id|!self.units.contains_key(id)){self.fail("TRN_TERMINOLOGY_COVERAGE","terminology_record/covered_units","List each selected unit and only actual inventory IDs; a count cannot replace coverage IDs.");}
        for category in ["adopted","unlisted"]{
            let entries=self.rows(&ledger[category],&format!("terminology_record/{category}"));
            if category=="unlisted"&&entries.is_empty()&&!nonempty(&ledger["no_unlisted_reason"]){self.fail("TRN_UNLISTED_EMPTY_REASON","terminology_record","Explain the reviewed empty unlisted-term set from the actual searches.");}
            for(i,row)in entries.iter().enumerate(){
                let id=format!("terminology_record/{category}/{i}");self.object(row,&id);self.fields(row,&["original","translation","context"],&id);
                let ids=self.ids(&row["source_units"],&format!("{id}/source_units"),true,true);
                if ids.iter().any(|id|!self.units.contains_key(id)){self.fail("TRN_TERM_MAPPING",&id,"Terminology source_units must exist in the inventory.");}
                if category=="adopted"{if !nonempty(&row["glossary_locator"])&&!row["glossary_locator"].is_object(){self.fail("TRN_GLOSSARY_MATCH_MISSING",&id,"Record the adopted glossary entry's locator.");}continue;}
                self.fields(row,&["reason"],&id);
                match row["status"].as_str(){
                    Some("resolved")=>self.fields(row,&["resolution_basis"],&id),
                    Some("unresolved")=>{self.report.unresolved_ids.push(id.clone());self.warning("TRN_UNRESOLVED",&id,"Unlisted terminology remains unresolved.");},
                    _=>self.fail("TRN_TERM_DECISION",&id,"Use resolved or unresolved for an unlisted term.")
                }
            }
        }
    }
    fn issues(&mut self,ledger:&Value,units:&Map<String,Value>){
        let terminology=self.terminology.clone();
        for category in ["adopted","unlisted"]{
            for(i,row)in list(&terminology[category]).iter().enumerate(){
                let pointer=format!("/{category}/{i}");
                for id in list(&row["source_units"]).iter().filter_map(Value::as_str){
                    if self.selected.contains(id)&&!self.term_mappings.contains(&(id.into(),pointer.clone())){self.fail("TRN_TERMINOLOGY_CANDIDATE_MISSING",&format!("{id}:{pointer}"),"The terminology ledger claims this selected-unit term, but no per-unit candidate maps back to it.");}
                }
            }
        }
        self.state(&ledger["source_issues_review"],"source_issues_review",STATUSES);
        for(id,issue)in self.index(&ledger["source_issues"],"source_issues"){
            let loc=format!("source_issues/{id}");
            let Some(unit)=issue["unit_id"].as_str().and_then(|s|units.get(s))else{self.fail("TRN_ISSUE_UNIT",&loc,"Tie the source issue to an actual selected review unit.");continue;};
            self.object(unit,&format!("units/{}",issue["unit_id"].as_str().unwrap()));
            self.source_span(unit["source_id"].as_str().unwrap_or(""),&issue["source_locator"],unit,&format!("{loc}/source_locator"));
            self.fields(&issue,&["description","disposition"],&loc);self.ids(&issue["candidate_readings"],&format!("{loc}/candidate_readings"),true,true);self.state(&issue,&loc,STATUSES);
        }
        let mut registered=BTreeSet::new();
        for(i,row)in self.rows(&ledger["unresolved_terms"],"unresolved_terms").iter().enumerate(){
            let id=format!("unresolved_terms/{i}");self.object(row,&id);
            let Some((u,t))=string(&row["unit_id"]).zip(string(&row["candidate_id"]))else{self.fail("TRN_UNRESOLVED_TERM_MAPPING",&id,"Supply unit_id and candidate_id.");continue;};
            let pair=(u.to_owned(),t.to_owned());
            if !registered.insert(pair.clone())||!self.unresolved.contains(&pair){self.fail("TRN_UNRESOLVED_TERM_MAPPING",&id,"Use each actual unresolved term candidate once.");}
            if !nonempty(&row["reason"]){self.fail("TRN_REASON_MISSING",&id,"Explain the actual unresolved term question.");}
        }
        for (u,t) in self.unresolved.clone().difference(&registered){self.fail("TRN_UNRESOLVED_TERM_MISSING",&format!("{u}/{t}"),"Register this unresolved candidate in unresolved_terms.");}
        for(i,event)in self.rows(&ledger["events"],"events").iter().enumerate(){
            let id=format!("events/{i}");self.object(event,&id);
            let ids=self.ids(&event["unit_ids"],&format!("{id}/unit_ids"),true,true);
            if ids.iter().any(|id|!self.selected.contains(id)){self.fail("TRN_EVENT_SCOPE",&id,"Event must name actual selected units.");}
            self.fields(event,&["check","performed_at","action"],&id);
            let time=event["performed_at"].as_str().unwrap_or("");
            let valid=chrono::DateTime::parse_from_rfc3339(time).is_ok()||chrono::NaiveDateTime::parse_from_str(time,"%Y-%m-%dT%H:%M:%S%.f").is_ok()||chrono::NaiveDate::parse_from_str(time,"%Y-%m-%d").is_ok();
            if !valid{self.fail("TRN_EVENT_TIME",&id,"Use the actual action's ISO timestamp; a timestamp alone is not process evidence.");}
            self.evidence(event.get("trace_evidence").unwrap_or(&json!([])),&format!("{id}/trace_evidence"),false);
        }
    }
    pub fn load_context(&mut self,ledger:&Value,inventory:Option<&Path>,candidate:Option<&Path>){
        let bindings=Value::Object(self.object(&ledger["bindings"],"bindings"));let base=self.base.clone();
        let bound=self.binding(&bindings["source_inventory"],"bindings/source_inventory",&base,inventory);
        let inv=inventory.map(Path::to_path_buf).or(bound);
        let source=inv.as_ref().map(|p|self.read(p,"inventory")).unwrap_or(Value::Null);
        self.object(&source,"inventory");
        self.units=self.index(&source["units"],"inventory/units");self.sources=self.index(&source["sources"],"inventory/sources");self.report.inventory_unit_count=self.units.len();
        let source_base=inv.as_ref().and_then(|p|p.parent()).unwrap_or(&base);
        for(id,row)in self.sources.clone(){if let Some(p)=self.binding(&row,&format!("inventory/sources/{id}"),source_base,None){self.source_paths.insert(id,p);}}
        for(id,row)in self.units.clone(){if !row["source"].as_str().is_some_and(|s|self.sources.contains_key(s)){self.fail("TRN_UNIT_SOURCE",&format!("inventory/units/{id}"),"Inventory unit has no known source identity.");}}
        self.candidate=self.binding(&bindings["candidate"],"bindings/candidate",&base,candidate);
        if let Some(p)=candidate{self.candidate=Some(p.to_owned());}
        self.glossary=self.binding(&bindings["glossary"],"bindings/glossary",&base,None);
        if let Some(p)=self.glossary.clone(){
            if self.cache.text(&p).map(|s|s.trim().is_empty()).unwrap_or(true){self.fail("TRN_GLOSSARY_UNREADABLE","bindings/glossary","The bound glossary is empty or unreadable.");}
        }
    }
    pub fn run(&mut self,ledger:&Value,inventory:Option<&Path>,candidate:Option<&Path>){
        if ledger["schema_version"].as_u64()!=Some(1){self.fail("TRN_SCHEMA_VERSION","review","Use the schema_version 1 scaffold contract.");}
        self.report.review_status=ledger["review_status"].as_str().map(str::to_owned);
        if ledger["review_status"]=="pending"{self.report.pending_ids.push("review_status".into());self.warning("TRN_PENDING","review_status","The overall review is still pending.");}
        else if ledger["review_status"]!="reviewed"{self.fail("TRN_REVIEW_STATUS","review_status","Use pending or reviewed; reviewed does not mean all comparisons passed.");}
        self.load_context(ledger,inventory,candidate);
        let review=self.object(&ledger["units"],"units");self.report.recorded_unit_count=review.len();
        self.scope(&ledger["scope"],&review);
        let bindings=self.object(&ledger["bindings"]["sources"],"bindings/sources");
        let expected:BTreeSet<_>=self.selected.iter().filter_map(|id|self.units.get(id).and_then(|u|u["source"].as_str()).map(str::to_owned)).collect();
        for id in expected.iter().filter(|id|!bindings.contains_key(*id)){self.fail("TRN_SOURCE_BINDING_MISSING",id,"Bind every source used by selected units.");}
        let base=self.base.clone();
        for(id,b)in &bindings{
            if !self.sources.contains_key(id){self.fail("TRN_SOURCE_BINDING_UNKNOWN",id,"Source binding has no inventory source identity.");}
            let p=self.source_paths.get(id).cloned();self.binding(b,&format!("bindings/sources/{id}"),&base,p.as_deref());
        }
        self.terminology_record(&ledger["terminology_record"]);
        for(id,row)in &review{self.unit(id,row);}
        self.issues(ledger,&review);
        if let Err(e)=self.cache.verify_current(){self.fail("TRN_HASH_CHANGED","review",e.to_string());}
    }
}
pub fn validate(p:&Path,inventory:Option<&Path>,candidate:Option<&Path>)->Result<Value>{
    let base=p.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or_else(||Path::new("."));
    let mut v=Validator::new(base);
    let p=match files::canonical(p){Ok(p)=>p,Err(e)=>{v.fail("TRN_JSON_UNREADABLE","review",e.to_string());v.fail("TRN_REVIEW_SHAPE","review","The translation-review root must be an object.");return v.report.value();}};
    v.base=p.parent().unwrap().to_owned();
    let data=v.read(&p,"review");if !data.is_object(){v.fail("TRN_REVIEW_SHAPE","review","The translation-review root must be an object.");return v.report.value();}
    let inv=inventory.map(files::canonical).transpose();let can=candidate.map(files::canonical).transpose();
    match (inv,can){(Ok(inv),Ok(can))=>v.run(&data,inv.as_deref(),can.as_deref()),(Err(e),_)|(_,Err(e))=>v.fail("TRN_PATH_INVALID","review",e.to_string())};
    v.report.value()
}
