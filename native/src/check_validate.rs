//! Validate real rule declarations and evidence bindings. A valid ledger
//! establishes record consistency, never the truth of a reviewer's claim.
use crate::{check_ledger,files,path_text,parse_json,syntax};
use anyhow::{anyhow,ensure,Context,Result};
use serde::Serialize;
use serde_json::{json,Value};
use std::{collections::{BTreeMap,BTreeSet,HashMap,HashSet},fs,path::{Path,PathBuf}};

const STATUSES:&[&str]=&["passed","failed","blocked","not_verified","not_applicable"];
const TYPES:&[&str]=&["unclassified","execution_process","offline_outcome","live_environment"];
const ENV:&[&str]=&["actual_ui","native_cache","actual_cli"];
const EVIDENCE:&[&str]=&["source_location","offline_result","execution_trace","environment_observation"];
const BOUNDARY:&str="Only atomic-ID coverage, record structure, existing evidence locations and current file hashes are checked. Status, scope, applicability and observation claims are supplied by the recorder. Independent comparison against the source files is still required for file acceptance. Unobserved process history is not inferred.";
fn nonempty(v:&Value)->bool{v.as_str().is_some_and(|s|!s.trim().is_empty())}
fn supported(v:&Value,set:&[&str])->bool{v.as_str().is_some_and(|s|set.contains(&s))}
fn list(v:&Value)->&[Value]{v.as_array().map(Vec::as_slice).unwrap_or(&[])}
fn resolved(s:&Value)->bool{matches!(s.as_str(),Some("passed"|"not_applicable"))}
#[derive(Serialize)]
pub struct Finding{code:String,object_id:String,message:String,level:&'static str}
#[derive(Default,Serialize)]
pub struct Report{
    pub findings:Vec<Finding>,pub catalog_count:usize,pub expected_count:usize,pub recorded_count:usize,
    pub status_counts:BTreeMap<String,usize>,pub pending_ids:Vec<String>,pub blocked_ids:Vec<String>,
    pub unclassified_ids:Vec<String>,pub offline_ids:Vec<String>,pub offline_unresolved_ids:Vec<String>,
    pub offline_failed_ids:Vec<String>,pub process_unverified_ids:Vec<String>,pub live_unverified_ids:Vec<String>,
}
impl Report{
    fn add(&mut self,code:&str,id:&str,message:impl Into<String>){self.findings.push(Finding{code:code.into(),object_id:id.into(),message:message.into(),level:"error"});}
    pub fn to_value(&self)->Result<Value>{
        let mut v=serde_json::to_value(self)?;
        let consistent=self.findings.is_empty();
        v["structurally_consistent"]=json!(consistent);
        v["recorded_checks_complete"]=json!(consistent&&self.expected_count>0&&self.pending_ids.is_empty()&&self.blocked_ids.is_empty()&&self.unclassified_ids.is_empty());
        v["offline_records_ready"]=json!(consistent&&self.expected_count>0&&self.offline_unresolved_ids.is_empty()&&self.unclassified_ids.is_empty());
        for k in ["source_fidelity","execution_truth","live_behavior"]{v[k]=json!("not_assessed");}
        v["independent_review_required"]=json!(true);v["evidence_boundary"]=json!(BOUNDARY);Ok(v)
    }
}
fn bound_path(v:&Value,base:&Path)->Result<PathBuf>{
    let s=v.as_str().filter(|s|!s.trim().is_empty()).context("A nonempty file path is required.")?;
    let p=PathBuf::from(s);files::canonical(&if p.is_absolute(){p}else{base.join(p)})
}
#[derive(Default)]
struct Cache{
    hashes:HashMap<PathBuf,String>,texts:HashMap<PathBuf,String>,
    page_counts:HashMap<PathBuf,usize>,dimensions:HashMap<PathBuf,(u32,u32)>,
    locators:HashMap<(PathBuf,String),Option<String>>,
}
impl Cache{
    fn hash(&mut self,p:&Path)->Result<String>{
        if let Some(v)=self.hashes.get(p){return Ok(v.clone());}
        let v=files::snapshot(p)?["sha256"].as_str().context("missing digest")?.to_owned();
        self.hashes.insert(p.to_owned(),v.clone());Ok(v)
    }
    fn text(&mut self,p:&Path)->Result<&str>{
        if !self.texts.contains_key(p){
            let data=fs::read(p)?;let digest=files::digest(&data);
            if let Some(expected)=self.hashes.get(p){ensure!(*expected==digest,"Evidence file changed during validation");}
            self.hashes.insert(p.to_owned(),digest);
            self.texts.insert(p.to_owned(),String::from_utf8(data)?);
        }Ok(&self.texts[p])
    }
    fn pages(&mut self,p:&Path)->Result<usize>{
        if let Some(n)=self.page_counts.get(p){return Ok(*n);}
        // This parser checks actual PDF page tree membership only; it is
        // not the rendering or PDF image provenance implementation.
        ensure!(fs::metadata(p)?.len()<=512*1024*1024,"PDF evidence exceeds the read budget");
        let doc=lopdf::Document::load(p)?;
        ensure!(!doc.is_encrypted(),"Encrypted PDF evidence is not supported");
        let n=doc.get_pages().len();
        ensure!(n>0,"PDF page tree is empty");
        self.page_counts.insert(p.to_owned(),n);Ok(n)
    }
    fn dimensions(&mut self,p:&Path)->Result<(u32,u32)>{
        if let Some(n)=self.dimensions.get(p){return Ok(*n);}
        let size=image::ImageReader::open(p)?.with_guessed_format()?.into_dimensions()?;
        self.dimensions.insert(p.to_owned(),size);Ok(size)
    }
    fn locator(&mut self,p:&Path,v:&Value)->Result<Option<String>>{
        let key=(p.to_owned(),serde_json::to_string(v)?);
        if let Some(e)=self.locators.get(&key){return Ok(e.clone());}
        let out=self.locator_inner(p,v)?;
        self.locators.insert(key,out.clone());Ok(out)
    }
    fn locator_inner(&mut self,p:&Path,v:&Value)->Result<Option<String>>{
        let msg=|s:&str|Ok(Some(s.to_owned()));
        if !v.as_object().is_some_and(|m|!m.is_empty()){return msg("Evidence needs a structured locator, not a free-form claim.");}
        let bounds=||v["start"].as_u64().zip(v["end"].as_u64()).filter(|(a,b)|*a>0&&a<=b);
        match v["kind"].as_str(){
            Some("whole_file")=>if fs::metadata(p)?.len()>0{Ok(None)}else{msg("The referenced whole file is empty.")},
            Some("lines")=>{
                let Some((_,end))=bounds()else{return msg("Line locations require integer 1 <= start <= end.");};
                if end>syntax::lines(self.text(p)?).len() as u64{msg("The evidence line range is outside the actual file.")}else{Ok(None)}
            }
            Some("pdf_pages")=>{
                let Some((_,end))=bounds()else{return msg("PDF page locations require integer 1 <= start <= end.");};
                if end>self.pages(p)? as u64{msg("The evidence page range is outside the actual PDF.")}else{Ok(None)}
            }
            Some("image_region")=>{
                let Some(boxes)=v["bbox"].as_array().filter(|v|v.len()==4)else{return msg("An image region needs four pixel coordinates.");};
                let ints:Option<Vec<i64>>=boxes.iter().map(Value::as_i64).collect();
                let Some(b)=ints else{return msg("Image-region coordinates must be integers.");};
                let(w,h)=self.dimensions(p)?;
                if b[0]>=0&&b[0]<b[2]&&b[2]<=w as i64&&b[1]>=0&&b[1]<b[3]&&b[3]<=h as i64{Ok(None)}
                else{msg("The evidence image region is outside the actual image.")}
            }
            Some("json_pointer")=>{
                let Some(pointer)=v["pointer"].as_str().filter(|s|s.is_empty()||s.starts_with('/'))else{return msg("A JSON pointer must be empty (root) or start with /.");};
                let doc=parse_json(self.text(p)?)?;let mut value=&doc;
                if !pointer.is_empty(){
                    for part in pointer[1..].split('/'){
                        let mut iter=part.chars();let mut segment=String::new();
                        while let Some(c)=iter.next(){
                            if c!='~'{segment.push(c);continue;}
                            match iter.next(){Some('0')=>segment.push('~'),Some('1')=>segment.push('/'),_=>return msg("Invalid JSON pointer escape.")}
                        }
                        let next=if let Some(map)=value.as_object(){map.get(&segment)}
                        else if let Some(rows)=value.as_array(){
                            if segment=="0"||(!segment.starts_with('0')&&!segment.is_empty()&&segment.bytes().all(|b|b.is_ascii_digit())){
                                segment.parse::<usize>().ok().and_then(|n|rows.get(n))
                            }else{None}
                        }else{None};
                        let Some(next)=next else{return msg("The JSON pointer does not identify an existing value.");};value=next;
                    }
                }Ok(None)
            }
            _=>Ok(Some(format!("Unsupported evidence locator kind: {}",v["kind"]))),
        }
    }
}
fn verify_binding(v:&Value,id:&str,base:&Path,cache:&mut Cache,report:&mut Report)->Option<PathBuf>{
    if !v.is_object(){report.add("CHECK_FILE_BINDING_INVALID",id,"A file binding must be an object.");return None;}
    let found=(||->Result<(PathBuf,String)>{
        let p=bound_path(&v["path"],base)?;
        let hash=cache.hash(&p)?;Ok((p,hash))
    })();
    let(p,actual)=match found{Ok(pair)=>pair,Err(e)=>{report.add("CHECK_FILE_MISSING",id,e.to_string());return None;}};
    let Some(expected)=v["sha256"].as_str().filter(|s|s.len()==64&&s.bytes().all(|b|b.is_ascii_digit()||matches!(b,b'a'..=b'f')))else{
        report.add("CHECK_HASH_MISSING",id,"A recorded SHA-256 digest is required.");return None;};
    if actual!=expected{report.add("CHECK_HASH_CHANGED",id,format!("File bytes changed: {}",p.display()));return None;}Some(p)
}
pub fn scope_valid(v:&Value)->bool{
    v.as_array().is_some_and(|a|!a.is_empty()&&a.iter().all(|item|item.is_object()&&nonempty(&item["object_id"])&&
        (nonempty(&item["locator"])||item["locator"].as_object().is_some_and(|m|!m.is_empty()))))
}
fn observation_valid(e:&Value,typ:&str,environment:&Value)->bool{
    let o=&e["observation"];
    let (kind,basis)=if typ=="execution_process"{("execution_trace",Some("direct_event"))}else{("environment_observation",environment.as_str())};
    e["evidence_type"]==kind&&o.is_object()&&o["observed"]==true&&o["basis"].as_str()==basis&&basis.is_some()&&
        nonempty(&o["observer"])&&o["observed_at"].as_str().is_some_and(|s|chrono::DateTime::parse_from_rfc3339(s).is_ok())&&
        (typ!="live_environment"||nonempty(&o["environment"]))
}
fn tally(ids:&[String])->Vec<(String,usize)>{
    let mut count=HashMap::new();let mut order=Vec::new();
    for id in ids{
        if !count.contains_key(id){order.push(id.clone());}*count.entry(id.clone()).or_insert(0)+=1;
    }order.into_iter().map(|id|{let n=count[&id];(id,n)}).collect()
}
pub fn validate(path:&Path,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    let mut report=Report::default();
    let (raw,binding)=match files::read_bound(path){Ok(v)=>v,Err(e)=>{report.add("CHECK_LEDGER_UNREADABLE","checks",e.to_string());return report.to_value();}};
    let ledger=match std::str::from_utf8(&raw).map_err(anyhow::Error::from).and_then(parse_json){
        Ok(v)=>v,Err(e)=>{report.add("CHECK_LEDGER_UNREADABLE","checks",e.to_string());return report.to_value();}};
    if !ledger.is_object(){report.add("CHECK_LEDGER_INVALID","checks","The ledger must be a JSON object.");return report.to_value();}
    if ledger["schema_version"].as_u64()!=Some(1){report.add("CHECK_SCHEMA_VERSION","checks","Unsupported or missing schema_version.");}
    let ledger_path=PathBuf::from(binding["path"].as_str().unwrap());
    let base=ledger_path.parent().context("ledger directory")?;
    let catalog=match bound_path(&ledger["skills_root"],base).and_then(|p|check_ledger::catalog(&p,&mut progress)){
        Ok(v)=>v,Err(e)=>{report.add("CHECK_RULE_CATALOG_INVALID","rules",e.to_string());return report.to_value();}};
    let rules:HashMap<_,_>=catalog.rules.iter().map(|r|(r.id.as_str(),r)).collect();
    report.catalog_count=rules.len();
    let mode=ledger["rule_selection"]["mode"].as_str().unwrap_or("");
    let selected=match check_ledger::selected(&catalog,mode,&ledger["rule_selection"]["additional_ids"]){
        Ok(v)=>v,Err(e)=>{report.add("CHECK_SELECTION_INVALID","checks",e.to_string());check_ledger::selected(&catalog,"acceptance",&json!([]))?}};
    report.expected_count=selected.len();
    let mut catalog_ids=Vec::new();
    if !ledger["catalog"].is_array(){report.add("CHECK_CATALOG_INVALID","catalog","catalog must retain all actual atomic rule declarations.");}
    for(i,item)in list(&ledger["catalog"]).iter().enumerate(){
        let Some(id)=item["id"].as_str().filter(|s|!s.trim().is_empty())else{report.add("CHECK_CATALOG_ID_INVALID",&format!("catalog:{i}"),"Every catalog entry needs an atomic ID.");continue;};
        catalog_ids.push(id.to_owned());
        if rules.get(id).is_none_or(|r|r.entry()!=*item){report.add("CHECK_CATALOG_CHANGED",id,"The catalog entry differs from the actual rule declaration.");}
    }
    for(id,n)in tally(&catalog_ids){if n>1{report.add("CHECK_CATALOG_ID_DUPLICATE",&id,"The full rule catalog contains a duplicate ID.");}}
    let seen:HashSet<_>=catalog_ids.iter().map(String::as_str).collect();
    let missing:BTreeSet<_>=rules.keys().copied().filter(|s|!seen.contains(s)).collect();
    for id in missing{report.add("CHECK_CATALOG_ID_MISSING",id,"The full rule catalog lacks this current atomic declaration.");}
    let mut cache=Cache::default();
    if !ledger["rule_files"].is_array(){report.add("CHECK_RULE_FILES_INVALID","rules","rule_files must list actual rule file bindings.");}
    let mut seen_paths=Vec::new();
    for (i,item) in list(&ledger["rule_files"]).iter().enumerate(){
        verify_binding(item,&format!("rules:{i}"),base,&mut cache,&mut report);
        if let Ok(p)=bound_path(&item["path"],base){seen_paths.push(path_text(&p)?);}
    }
    for (p,n) in tally(&seen_paths){if n>1{report.add("CHECK_RULE_FILE_DUPLICATE",&p,"A rule file was bound more than once.");}}
    let expected:HashSet<_>=catalog.bindings.iter().filter_map(|r|r["path"].as_str()).collect();
    if seen_paths.iter().map(String::as_str).collect::<HashSet<_>>()!=expected{
        report.add("CHECK_RULE_FILES_CHANGED","rules","The complete current skill/reference file set differs from the recorded set.");
    }
    if !ledger["bindings"].is_object(){report.add("CHECK_BINDINGS_INVALID","bindings","File bindings must be an object.");}
    for label in ["sources","candidates"]{
        let entries=list(&ledger["bindings"][label]);
        if entries.is_empty(){report.add("CHECK_BINDING_MISSING",label,format!("At least one actual {label} file binding is required."));continue;}
        let mut ids=Vec::new();
        for (i,item) in entries.iter().enumerate(){
            let identity=item["id"].as_str().filter(|s|!s.trim().is_empty());
            if let Some(id)=identity{ids.push(id.to_owned());}else{report.add("CHECK_BINDING_ID_MISSING",&format!("{label}:{i}"),"File bindings need nonempty IDs.");}
            let id=identity.map(str::to_owned).unwrap_or_else(||i.to_string());
            verify_binding(item,&format!("{label}:{id}"),base,&mut cache,&mut report);
        }
        for(id,n)in tally(&ids){if n>1{report.add("CHECK_BINDING_ID_DUPLICATE",&format!("{label}:{id}"),"Duplicate file binding ID.");}}
    }
    let empty=serde_json::Map::new();
    let evidence=ledger["evidence"].as_object().unwrap_or_else(||{report.add("CHECK_EVIDENCE_REGISTRY_INVALID","evidence","evidence must be an ID-to-file-location object.");&empty});
    let mut valid=HashSet::new();
    for(id,item)in evidence{
        let label=format!("evidence:{id}");
        if id.trim().is_empty(){report.add("CHECK_EVIDENCE_ID_INVALID",&label,"Evidence IDs must be nonempty.");}
        let Some(p)=verify_binding(item,&label,base,&mut cache,&mut report)else{continue;};
        if !supported(&item["evidence_type"],EVIDENCE){report.add("CHECK_EVIDENCE_TYPE_INVALID",&label,"An explicit supported evidence_type is required.");continue;}
        match cache.locator(&p,&item["locator"]){
            Ok(None)=>{valid.insert(id.as_str());},
            Ok(Some(e))=>report.add("CHECK_EVIDENCE_LOCATION_INVALID",&label,e),
            Err(e)=>report.add("CHECK_EVIDENCE_LOCATION_INVALID",&label,e.to_string()),
        }
    }
    if !ledger["checks"].is_array(){report.add("CHECK_RECORDS_INVALID","checks","checks must be one flat list of atomic records.");}
    let checks=list(&ledger["checks"]);report.recorded_count=checks.len();let mut seen_ids=Vec::new();
    for(i,item)in checks.iter().enumerate(){
        progress(i+1,checks.len(),"checks");
        let Some(id)=item["id"].as_str().filter(|s|!s.trim().is_empty())else{report.add("CHECK_ID_INVALID",&format!("checks:{}",i+1),"An atomic check ID is required.");continue;};
        seen_ids.push(id.to_owned());
        let rule=rules.get(id);
        if rule.is_none(){report.add("CHECK_ID_UNKNOWN",id,"This ID is not a current atomic rule declaration.");}
        else if !selected.contains(id){report.add("CHECK_ID_UNSELECTED",id,"Add this constraint to additional_ids or use rule_selection=all before tracking it.");}
        else if rule.is_some_and(|r|item["rule"]!=r.location||item["objective"]!=r.objective||item["skill"]!=r.skill){report.add("CHECK_RULE_CHANGED",id,"The recorded objective, skill or source location differs from the current rule.");}
        if !scope_valid(&item["scope"]){report.add("CHECK_SCOPE_EMPTY",id,"List actual covered object_id and locator pairs; a skill-wide status is not a scope.");}
        let status=&item["status"];
        if !supported(status,STATUSES){report.add("CHECK_STATUS_INVALID",id,format!("Unknown status: {status}"));}
        else{*report.status_counts.entry(status.as_str().unwrap().into()).or_default()+=1;}
        if !item["reason"].is_string(){report.add("CHECK_REASON_MISSING",id,"Every record needs a reason string.");}
        else if matches!(status.as_str(),Some("blocked"|"not_verified"|"not_applicable"))&&!nonempty(&item["reason"]){report.add("CHECK_REASON_MISSING",id,"Pending, blocked and inapplicable records require a specific reason.");}
        if status=="not_applicable"&&!nonempty(&item["applicability_condition"]){report.add("CHECK_NA_CONDITION_MISSING",id,"Record the actual condition that makes this check inapplicable.");}
        let typ=&item["requirement_type"];let env=&item["environment_kind"];
        if !supported(typ,TYPES){report.add("CHECK_REQUIREMENT_TYPE_INVALID",id,"Use an explicit supported requirement_type.");}
        else if typ=="unclassified"{
            report.unclassified_ids.push(id.into());
            if matches!(status.as_str(),Some("passed"|"failed"|"not_applicable")){report.add("CHECK_TYPE_UNCLASSIFIED",id,"Classify the evidence boundary explicitly before recording a resolved result.");}
        }
        if rule.is_some_and(|r|r.typ!="unclassified"&&typ!=r.typ.as_str()){report.add("CHECK_TYPE_CONFLICT",id,"The record's type conflicts with the source rule declaration.");}
        if typ=="live_environment"{
            if !supported(env,ENV){report.add("CHECK_ENVIRONMENT_KIND_INVALID",id,"A live-environment record needs an explicit actual_ui, native_cache or actual_cli context.");}
            if rule.and_then(|r|r.environment.as_deref()).is_some_and(|s|env!=s){report.add("CHECK_ENVIRONMENT_CONFLICT",id,"The environment kind conflicts with the source rule declaration.");}
        }else if !env.is_null(){report.add("CHECK_ENVIRONMENT_KIND_INVALID",id,"Only live-environment records may declare an environment kind.");}
        if typ=="offline_outcome"{
            report.offline_ids.push(id.into());if !resolved(status){report.offline_unresolved_ids.push(id.into());}
            if status=="failed"{report.offline_failed_ids.push(id.into());}
        }else if typ=="execution_process"&&!resolved(status){report.process_unverified_ids.push(id.into());}
        else if typ=="live_environment"&&!resolved(status){report.live_unverified_ids.push(id.into());}
        let refs=if item["evidence"].as_array().is_some_and(|a|a.iter().all(nonempty)){list(&item["evidence"])}else{
            report.add("CHECK_EVIDENCE_REFS_INVALID",id,"evidence must be a list of individual evidence IDs.");&[]
        };
        let mut actual_refs=Vec::new();
        for r in refs{
            let r=r.as_str().unwrap();
            if !evidence.contains_key(r){report.add("CHECK_EVIDENCE_UNKNOWN",id,format!("Unknown evidence ID: {r}"));}
            else if !valid.contains(r){report.add("CHECK_EVIDENCE_UNUSABLE",id,format!("Evidence has no valid current file/location binding: {r}"));}
            else{actual_refs.push(&evidence[r]);}
        }
        if matches!(status.as_str(),Some("passed"|"failed"|"not_applicable")){
            if actual_refs.is_empty(){report.add("CHECK_RESULT_EVIDENCE_MISSING",id,"Passed, failed and inapplicable results require an existing, hash-bound evidence location.");}
            if status!="not_applicable"&&matches!(typ.as_str(),Some("execution_process"|"live_environment"))&&
                !actual_refs.iter().any(|r|observation_valid(r,typ.as_str().unwrap(),env)){
                let code=if typ=="execution_process"{"CHECK_PROCESS_NOT_OBSERVED"}else if env=="actual_ui"{"CHECK_UI_NOT_OBSERVED"}else{"CHECK_ENVIRONMENT_NOT_OBSERVED"};
                report.add(code,id,"A static result or self-report cannot replace the required direct observation; retain not_verified if it was not observed.");
            }
        }
        if status=="not_verified"{report.pending_ids.push(id.into());}else if status=="blocked"{report.blocked_ids.push(id.into());}
    }
    for(id,n)in tally(&seen_ids){if n>1{report.add("CHECK_ID_DUPLICATE",&id,format!("Atomic ID occurs {n} times; keep one record with multiple scope objects if necessary."));}}
    let seen:HashSet<_>=seen_ids.iter().map(String::as_str).collect();
    for id in selected.iter().filter(|id|!seen.contains(id.as_str())).collect::<BTreeSet<_>>(){
        report.add("CHECK_ID_MISSING",id,"The full-workflow ledger lacks this atomic rule ID.");
    }
    // Recheck inputs rather than silently validating records against a mixture
    // of file versions if an editor or another process modified evidence.
    for(p,hash)in &cache.hashes{
        if files::snapshot(p).ok().and_then(|b|b["sha256"].as_str().map(str::to_owned)).as_ref()!=Some(hash){
            report.add("CHECK_HASH_CHANGED",&path_text(p)?,"File bytes changed during evidence validation.");
        }
    }
    if files::snapshot(&ledger_path)?["sha256"]!=binding["sha256"]{report.add("CHECK_HASH_CHANGED","checks","Ledger changed during validation.");}
    report.to_value()
}
