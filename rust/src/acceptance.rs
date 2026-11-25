//! Read-only mechanical acceptance. Every check binds to actual note bytes,
//! declared source files and the running native engine, never model assertions.
use crate::{files,inventory,presentation,properties,validate,normalize,ENGINE_VERSION};
use anyhow::{ensure,Context,Result};
use regex::Regex;
use serde_json::{json,Map,Value};
use std::{collections::{BTreeMap,BTreeSet,HashMap},fs,path::{Path,PathBuf,Component},sync::OnceLock};

fn compile_rx(pat:&str)->Regex{Regex::new(pat).expect("acceptance regex")}
macro_rules! rx{($pat:expr)=>{{static R:OnceLock<Regex>=OnceLock::new();R.get_or_init(||compile_rx($pat))}}}
const KINDS:&[&str]=&["header","heading","list","metadata","paragraph","translator_note","figure","formula","table","reference","bibliography","zotero_link"];
const FIELDS:&[&str]=&["schema_version","sources","required_units","required_anchors","counts","table_dimensions","resources","plan_sha256"];
fn hash(v:&Value)->String{files::digest(&serde_json::to_vec(v).expect("JSON value"))}
fn text(v:&Value)->bool{v.as_str().is_some_and(|s|!s.trim().is_empty())}
fn digest(v:&Value)->bool{v.as_str().is_some_and(|s|rx!(r"^[a-f0-9]{64}$").is_match(s))}
fn n(v:&Value)->Option<usize>{v.as_u64().and_then(|v|usize::try_from(v).ok())}
fn scope(v:&Value)->bool{v.as_str().is_some_and(|s|rx!(r"^(?:main|supplement:[A-Za-z0-9][A-Za-z0-9_.-]*)$").is_match(s))}
fn extras(v:&Value,allowed:&[&str])->Value{
    json!(v.as_object().map(|m|m.keys().filter(|k|!allowed.contains(&k.as_str())).collect::<Vec<_>>()).unwrap_or_default())
}
fn stage(code:&str)->&'static str{
    if code=="SOURCE_FIDELITY"{"translation_review"}
    else if ["GAP","SPACING","HARD_BREAK"].iter().any(|s|code.contains(*s)){"spacing"}
    else if code.starts_with("RESOURCE")||code.starts_with("REMOTE_RESOURCE"){"resources"}
    else if code.starts_with("BIBLIOGRAPHY")||code.starts_with("CITATION"){"citations"}
    else if code.contains("ANCHOR")||code=="INTERNAL_TARGET"{"internal_links"}
    else if ["QUANTITY","PERCENT","MATH","FORMULA","DEGREE","EQUATION"].iter().any(|s|code.contains(*s)){"math_formatting"}
    else if code.contains("FIGURE")||code.contains("CROP"){"figures"}
    else if code.contains("TABLE")||code.contains("RICH"){"tables"}
    else if code.starts_with("HEADING"){"headings"}
    else if ["FRONTMATTER","HEADER","AUTHOR","AFFILIATION"].iter().any(|s|code.starts_with(*s)){"metadata"}
    else if ["UNIT_","RENDER","OUTPUT_HASH","UNMAPPED","UNBOUND"].iter().any(|s|code.starts_with(*s)){"assembly"}
    else if ["SOURCE","CONTRACT","REQUIRED","PLAN_HASH","OBJECT_COUNT","SCHEMA","UNIQUE_UNIT"].iter().any(|s|code.starts_with(*s)){"source_coverage"}
    else{"markdown_structure"}
}
// Resolve existing path prefixes, including dangling links, without assuming
// missing targets are inside the vault. Limit link traversals and reject cycles.
pub(crate) fn resolve(p:&Path)->Result<PathBuf>{
    fn inner(p:&Path,depth:usize)->Result<PathBuf>{
        ensure!(depth<40,"Too many symbolic links");
        let absolute=if p.is_absolute(){p.to_path_buf()}else{std::env::current_dir()?.join(p)};
        let mut result=PathBuf::new();
        for c in absolute.components(){
            match c{
                Component::Prefix(v)=>result.push(v.as_os_str()),
                Component::RootDir=>result.push(c.as_os_str()),
                Component::CurDir=>{},
                Component::ParentDir=>{result.pop();},
                Component::Normal(name)=>{
                    result.push(name);
                    match fs::symlink_metadata(&result){
                        Ok(m) if m.file_type().is_symlink()=>{
                            let target=fs::read_link(&result)?;
                            result=inner(&if target.is_absolute(){target}else{result.parent().context("symbolic link parent")?.join(target)},depth+1)?;
                        }
                        Ok(_)=>{result=files::canonical(&result)?;}
                        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},
                        Err(e)=>return Err(e.into()),
                    }
                }
            }
        }Ok(result)
    }inner(p,0)
}
struct Audit{
    path:PathBuf,root:PathBuf,checks:Vec<Value>,prompts:Vec<Value>,hashes:HashMap<PathBuf,String>,
}
impl Audit{
    fn check(&mut self,code:&str,location:&str,expected:Value,actual:Value,passed:Option<bool>)->bool{
        let ok=passed.unwrap_or(expected==actual);
        self.checks.push(json!({"code":code,"location":location,"expected":expected,"actual":actual,"status":if ok{"pass"}else{"fail"}}));ok
    }
    fn shape(&mut self,ok:bool,location:&str,expected:&str)->bool{
        if !ok{self.check("SCHEMA",location,json!(expected),json!("missing or invalid"),Some(false));}ok
    }
    fn hash_file(&mut self,p:&Path)->Result<String>{
        let p=resolve(p)?;
        if let Some(v)=self.hashes.get(&p){return Ok(v.clone());}
        let observed=files::snapshot(&p)?["sha256"].as_str().context("file digest")?.to_owned();
        self.hashes.insert(p,observed.clone());Ok(observed)
    }
    fn canonical_format(&mut self,raw:&str)->Result<()>{
        let result=normalize::normalize(raw)?;
        let mut blocked=false;
        for issue in result["unresolved"].as_array().context("normalizer unresolved")?{
            let code=issue["code"].as_str().context("normalizer issue code")?;
            if code.starts_with("UNCLOSED_")||code=="INTERACTING_RULES"{
                blocked=true;self.check(code,&format!("line {}",issue["line"]),json!("complete, stable parse"),issue["message"].clone(),Some(false));
            }else if issue["severity"]!="info"{let mut v=issue.clone();v["status"]=json!("pending");self.prompts.push(v);}
        }
        let changes=result["changes"].as_array().context("normalizer changes")?;
        if changes.is_empty(){
            self.check("CANONICAL_FORMAT","candidate",json!("verified fixed point"),json!(if blocked{"parse incomplete"}else{"verified fixed point"}),Some(!blocked&&result["idempotent"]==true));
        }else{
            for change in changes{
                self.check("CANONICAL_FORMAT",&format!("line {}, column {}",change["line"],change["column"]),change["after"].clone(),change["before"].clone(),Some(false));
                let check=self.checks.last_mut().unwrap();
                for k in ["rule","stage","line","column"]{check[k]=change[k].clone();}
            }
        }Ok(())
    }
    fn resource(&mut self,href:&str,line:usize,syntax:&str,hashes:&Map<String,Value>){
        let loc=format!("line {line}: {href}");
        let attempt=(||->Result<()>{
            let name=if syntax=="markdown"{
                let scheme=rx!(r"^[A-Za-z][A-Za-z0-9+.-]*:").find(href).map(|m|m.as_str());
                if scheme.is_some_and(|s|s.eq_ignore_ascii_case("http:")||s.eq_ignore_ascii_case("https:")){
                    if let Some((_,host))=href.split_once("://"){
                        if !host.split(['/', '?','#']).next().unwrap_or("").is_empty(){
                            self.prompts.push(json!({"code":"REMOTE_RESOURCE","location":loc,"status":"pending"}));
                            if let Some(v)=hashes.get(href){self.check("RESOURCE_HASH",&loc,v.clone(),json!("remote resource"),Some(false));}
                            return Ok(());
                        }
                    }
                }
                if scheme.is_some()||href.starts_with("//"){
                    self.check("RESOURCE_BOUNDARY",&loc,json!("within resource root"),json!("None"),Some(false));return Ok(());
                }
                percent_encoding::percent_decode_str(href.split(['?','#']).next().unwrap_or("")).decode_utf8()?.into_owned()
            }else{href.split('#').next().unwrap_or("").to_owned()};
            let path=Path::new(&name);
            let invalid=name.is_empty()||name.contains('\\')||name.contains('\0')||path.is_absolute()||
                rx!(r"^[A-Za-z]:").is_match(&name)||name.starts_with("//");
            let resolved=if invalid{None}else{Some(resolve(&self.root.join(path))?)};
            let actual=resolved.as_ref().map(|p|p.to_string_lossy().into_owned()).unwrap_or("None".into());
            let within=resolved.as_ref().is_some_and(|p|p.starts_with(&self.root));
            if !self.check("RESOURCE_BOUNDARY",&loc,json!("within resource root"),json!(actual),Some(within)){return Ok(());}
            let p=resolved.unwrap();
            if !self.check("RESOURCE_FILE",&loc,json!(true),json!(p.is_file()),None){return Ok(());}
            let actual=self.hash_file(&p)?;
            if let Some(expected)=hashes.get(href){self.check("RESOURCE_HASH",&loc,expected.clone(),json!(actual),None);}
            else{self.check("RESOURCE_FINGERPRINT",&loc,json!("readable SHA-256"),json!(actual),Some(true));}
            Ok(())
        })();
        if let Err(e)=attempt{self.check("RESOURCE_READ",&loc,json!("readable path"),json!(e.to_string()),Some(false));}
    }
    fn sources(&mut self,sources:&Value)->Map<String,Value>{
        if !self.shape(sources.as_object().is_some_and(|s|!s.is_empty()),"sources","nonempty source mapping"){return Map::new();}
        let sources=sources.as_object().unwrap();
        for(id,source)in sources{
            let loc=format!("sources.{id}");
            if !self.shape(!id.trim().is_empty()&&source.is_object()&&text(&source["path"])&&digest(&source["sha256"]),&loc,"source ID, file path and SHA-256"){continue;}
            let path=self.path.parent().unwrap_or(Path::new(".")).join(source["path"].as_str().unwrap());
            let observed=self.hash_file(&path).unwrap_or_else(|e|e.to_string());
            self.check("SOURCE_HASH",&loc,source["sha256"].clone(),json!(observed),None);
        }sources.clone()
    }
    fn units(&mut self,units:&Value,sources:&Map<String,Value>,rendering:&Value,raw:&str,contract:&Value,inv:&inventory::Inventory){
        if !self.shape(units.is_array(),"required_units","array"){return;}
        let units=units.as_array().unwrap();
        let mut expected=Vec::new();let mut seen=BTreeSet::new();let mut duplicate=BTreeSet::new();
        for(i,u)in units.iter().enumerate(){
            let loc=format!("required_units[{i}]");
            if !self.shape(u.is_object()&&text(&u["unit_id"]),&loc,"unit with ID"){continue;}
            self.check("UNIT_FIELDS",&loc,json!([]),extras(u,&["unit_id","kind","source","scope","anchor"]),None);
            if let Some(v)=u.get("scope"){self.shape(scope(v),&loc,"main or named supplement scope");}
            let s=&u["source"];
            let ok=u["kind"].as_str().is_some_and(|k|KINDS.contains(&k))&&s.is_object()&&
                s["source_id"].as_str().is_some_and(|id|sources.contains_key(id))&&
                (text(&s["locator"])||s["locator"].as_object().is_some_and(|m|!m.is_empty()));
            self.shape(ok,&loc,"kind and locator in known source");
            let id=u["unit_id"].as_str().unwrap();if !seen.insert(id){duplicate.insert(id);}expected.push(id);
        }
        self.check("UNIQUE_UNIT_IDS","required_units",json!([]),json!(duplicate),None);
        if rendering.is_null()&&units.is_empty(){return;}
        if !self.shape(rendering.is_object()&&rendering["mapping"].is_array(),"rendering","rendered unit mapping"){return;}
        self.check("RENDER_STATUS","rendering.status",json!("rendered"),rendering["status"].clone(),None);
        self.check("RENDER_SCHEMA","rendering.schema_version",json!(1),rendering["schema_version"].clone(),Some(rendering["schema_version"].as_u64()==Some(1)));
        self.check("OUTPUT_HASH","rendering.output_sha256",json!(files::digest(raw.as_bytes())),rendering["output_sha256"].clone(),None);
        if let Some(h)=contract.get("plan_sha256"){self.check("PLAN_HASH","rendering.plan_sha256",h.clone(),rendering["plan_sha256"].clone(),None);}
        let mapping=rendering["mapping"].as_array().unwrap();
        self.check("UNIT_ORDER","rendering.mapping",json!(expected),json!(mapping.iter().map(|e|&e["unit_id"]).collect::<Vec<_>>()),None);
        let lines:Vec<_>=raw.split('\n').collect();let mut offsets=vec![0];for line in lines.iter().take(lines.len().saturating_sub(1)){offsets.push(offsets.last().unwrap()+line.chars().count()+1);}
        let mut prev=0;
        if lines.first().is_some_and(|s|s.trim()=="---")&&mapping.first().is_some_and(|e|e.is_object()&&e["start_line"]!=1){
            if let Some(close)=lines.iter().enumerate().skip(1).find_map(|(i,s)|matches!(s.trim(),"---"|"...").then_some(i)){prev=close+1;}
        }
        for(i,entry)in mapping.iter().enumerate(){
            let loc=format!("rendering.mapping[{i}]");
            if !self.shape(entry.is_object(),&loc,"mapping entry"){continue;}
            if let Some(u)=units.get(i).filter(|u|u.is_object()){
                for (field,code)in[("source","UNIT_SOURCE"),("kind","UNIT_KIND"),("scope","UNIT_SCOPE")]{if let Some(v)=u.get(field){self.check(code,&loc,v.clone(),entry[field].clone(),None);}}
            }
            let (start,end)=(n(&entry["start_line"]),n(&entry["end_line"]));
            let valid=start.zip(end).is_some_and(|(a,b)|prev<a&&a<=b&&b<=lines.len());
            if !self.check("UNIT_SPAN",&loc,json!(format!("{prev} < start <= end <= {}",lines.len())),json!([entry["start_line"],entry["end_line"]]),Some(valid)){continue;}
            let (start,end)=(start.unwrap(),end.unwrap());
            self.check("UNMAPPED_CONTENT",&loc,json!(false),json!(lines[prev..start-1].iter().any(|s|!s.trim().is_empty())),None);prev=end;
            let snippet=lines[start-1..end].join("\n");
            self.check("UNIT_NONEMPTY",&loc,json!(true),json!(!snippet.trim().is_empty()),None);
            if let Some(u)=units.get(i).filter(|u|u.is_object()){
                if let Some(v)=u.get("anchor"){
                    if self.shape(text(v),&loc,"block anchor"){
                        let id=v.as_str().unwrap();let id=id.strip_prefix('^').unwrap_or(id);let actual=inv.anchors.get(id);
                        self.check("UNIT_ANCHOR",&loc,json!(format!("{id} on lines {start}..{end}")),json!(actual),Some(actual.is_some_and(|v|start<=*v&&*v<=end)));
                    }
                }
                if let Some(kind)=u["kind"].as_str().filter(|k|KINDS.contains(k)){
                    let has=inv.events.range(start..=end).any(|(_,es)|es.iter().any(|(k,a)|*k==kind&&*a>=start));
                    self.check("UNIT_OBJECT",&loc,json!(kind),json!(if has{"matching active syntax"}else{"missing syntax"}),Some(has));
                }
            }
            if entry.get("start_offset").is_some()||entry.get("end_offset").is_some(){
                let expected=json!([offsets[start-1],offsets[end-1]+lines[end-1].chars().count()]);let actual=json!([entry["start_offset"],entry["end_offset"]]);
                self.check("UNIT_OFFSETS",&loc,expected,actual,None);
            }
            self.check("UNIT_HASH",&loc,json!(files::digest(snippet.as_bytes())),entry["sha256"].clone(),None);
        }
        self.check("UNMAPPED_CONTENT","after mapping",json!(false),json!(lines[prev..].iter().any(|s|!s.trim().is_empty())),None);
    }
    fn expectations(&mut self,c:&Value,inv:&inventory::Inventory)->Map<String,Value>{
        let empty_array=json!([]);let empty_map=json!({});
        let anchors=c.get("required_anchors").unwrap_or(&empty_array);
        if self.shape(anchors.is_array(),"required_anchors","array of bare IDs"){
            for(i,id)in anchors.as_array().unwrap().iter().enumerate(){
                if self.shape(id.as_str().is_some_and(|s|rx!(r"^[A-Za-z0-9-]+$").is_match(s)),&format!("required_anchors[{i}]"),"bare anchor ID"){
                    let id=id.as_str().unwrap();self.check("REQUIRED_ANCHOR",id,json!(true),json!(inv.anchors.contains_key(id)),None);
                }
            }
        }
        let counts=c.get("counts").unwrap_or(&empty_map);
        if self.shape(counts.is_object(),"counts","object counts"){
            for(k,v)in counts.as_object().unwrap(){if self.shape(inv.counts.contains_key(k)&&n(v).is_some(),&format!("counts.{k}"),"supported nonnegative count"){
                self.check("OBJECT_COUNT",&format!("counts.{k}"),v.clone(),json!(inv.counts[k]),None);
            }}
        }
        let dims=c.get("table_dimensions").unwrap_or(&empty_map);
        if self.shape(dims.is_object(),"table_dimensions","anchor to sheet dimensions"){
            for(k,v)in dims.as_object().unwrap(){
                let valid=v.as_array().is_some_and(|a|!a.is_empty()&&a.iter().all(|r|r.as_object().is_some_and(|m|m.len()==2&&n(&r["rows"]).is_some()&&n(&r["columns"]).is_some())));
                if self.shape(valid,&format!("table_dimensions.{k}"),"nonempty sheet dimensions"){
                    self.check("TABLE_DIMENSIONS",k,v.clone(),inv.dimensions.get(k).cloned().unwrap_or(Value::Null),None);
                }
            }
        }
        let hashes=c.get("resources").unwrap_or(&empty_map);
        if !self.shape(hashes.is_object(),"resources","href to SHA-256"){return Map::new();}
        for(k,v)in hashes.as_object().unwrap(){
            self.shape(!k.trim().is_empty()&&digest(v),&format!("resources.{k}"),"href and SHA-256");
            self.check("RESOURCE_EMBEDDED",k,json!(true),json!(inv.resources.iter().any(|(href,_,_)|href==k)),None);
        }hashes.as_object().unwrap().clone()
    }
}
pub fn accept(path:&Path,contract:&Value,resource_root:Option<&Path>,rendering:&Value)->Result<Value>{
    let mut audit=Audit{path:path.to_path_buf(),root:resource_root.unwrap_or_else(||path.parent().unwrap_or(Path::new("."))).to_path_buf(),
        checks:vec![],prompts:vec![],hashes:HashMap::new()};
    let mut fingerprints=json!({"candidate_sha256":null,"contract_sha256":if contract.is_null(){Value::Null}else{json!(hash(contract))},
        "rendering_sha256":if rendering.is_null(){Value::Null}else{json!(hash(rendering))}});
    let mut counts=BTreeMap::new();let mut coverage="not_assessed";
    let result=(||->Result<()>{
        let raw=fs::read(path)?;fingerprints["candidate_sha256"]=json!(files::digest(&raw));
        let raw=std::str::from_utf8(&raw)?;
        audit.canonical_format(raw)?;
        let text=raw.strip_prefix('\u{feff}').unwrap_or(raw).replace("\r\n","\n").replace('\r',"\n");
        audit.root=resolve(&audit.root)?;
        let first=text.split('\n').next().unwrap_or("").trim();
        audit.check("FRONTMATTER_PRESENT","line 1",json!("---"),json!(first),None);
        let findings=validate::validate_note(&text);
        if first=="---"&&!findings.iter().any(|f|f.code.starts_with("FRONTMATTER")){
            let lines:Vec<_>=text.split('\n').collect();
            if let Some(end)=lines.iter().enumerate().skip(1).find_map(|(i,l)|matches!(l.trim(),"---"|"...").then_some(i)){
                let v=properties::parse_yaml(lines[1..end].join("\n").as_bytes())?;
                let typename=match v{Value::Object(_)=>"dict",Value::Array(_)=>"list",Value::Null=>"NoneType",Value::Bool(_)=>"bool",Value::Number(_)=>"number",Value::String(_)=>"str"};
                audit.check("FRONTMATTER_MAPPING","line 1",json!("mapping"),json!(typename),Some(v.is_object()));
            }
        }
        audit.check("NOTE_STRUCTURE","candidate",json!(0),json!(findings.iter().filter(|f|f.level=="error").count()),None);
        for f in findings{
            if f.level=="error"{audit.check(f.code,&format!("line {}",f.line),json!("satisfies structural rule"),json!(f.message),Some(false));}
            else{let mut v=serde_json::to_value(f)?;v["status"]=json!("pending");audit.prompts.push(v);}
        }
        for p in presentation::lint(&text)?{let mut v=serde_json::to_value(p)?;v["status"]=json!("pending");audit.prompts.push(v);}
        let inv=inventory::inspect(&text)?;
        counts=inv.counts.clone();
        for f in &inv.grid_findings{audit.check(f.code,&f.identity,json!("valid Rich Table grid"),json!(f.message),Some(false));}
        for(kind,target,line)in &inv.targets{
            let hits=if *kind=="block"{usize::from(inv.anchors.contains_key(target))}else{*inv.headings.get(target).unwrap_or(&0)};
            audit.check("INTERNAL_TARGET",&format!("line {line}: {target}"),json!(1),json!(hits),None);
        }
        let mut hashes=Map::new();
        if !contract.is_null()&&audit.shape(contract.is_object(),"contract","mapping"){
            audit.check("CONTRACT_FIELDS","contract",json!([]),extras(contract,FIELDS),None);
            audit.check("CONTRACT_VERSION","contract",json!(1),contract["schema_version"].clone(),Some(contract["schema_version"].as_u64()==Some(1)));
            let sources=audit.sources(&contract["sources"]);
            if let Some(v)=contract.get("plan_sha256"){audit.shape(digest(v),"plan_sha256","SHA-256");audit.shape(!rendering.is_null(),"rendering","mapping bound to plan digest");}
            audit.units(contract.get("required_units").unwrap_or(&json!([])),&sources,rendering,raw,contract,&inv);
            hashes=audit.expectations(contract,&inv);
            let has_expectation=["required_units","required_anchors","counts","table_dimensions","resources"].iter().any(|k|contract.get(*k).is_some_and(|v|match v{Value::Array(a)=>!a.is_empty(),Value::Object(m)=>!m.is_empty(),Value::Null=>false,_=>true}));
            if has_expectation{coverage="verified_against_contract";}
        }else if !rendering.is_null(){audit.check("UNBOUND_MAPPING","rendering",json!("source-bound required units"),json!("no contract"),Some(false));}
        for(href,line,syntax)in &inv.resources{audit.resource(href,*line,syntax,&hashes);}
        Ok(())
    })();
    if let Err(e)=result{audit.check("INPUT_INVALID","candidate or supplied evidence",json!("readable supported data"),json!(e.to_string()),Some(false));}
    let failed=audit.checks.iter().any(|c|c["status"]=="fail");
    if !contract.is_null()&&failed{coverage="failed";}
    let binary=files::snapshot(&std::env::current_exe()?)?;
    let rules=hash(&json!({"engine":"rust","version":ENGINE_VERSION,"binary_sha256":binary["sha256"]}));
    fingerprints["resource_root"]=json!(audit.root.to_string_lossy());
    let evidence:Vec<_>=audit.checks.iter().filter(|v|v["code"].as_str().is_some_and(|s|s.starts_with("RESOURCE_")||s=="SOURCE_HASH"))
        .map(|v|json!([v["code"],v["location"],v["actual"]])).collect();
    fingerprints["file_evidence_sha256"]=json!(hash(&json!(evidence)));let input_hash=hash(&fingerprints);
    audit.prompts.push(json!({"code":"SOURCE_FIDELITY","status":"pending","message":"Compare meaning, translation, object identities and source inventory with originals."}));
    for item in audit.checks.iter_mut().chain(audit.prompts.iter_mut()){
        if item.get("stage").is_none(){item["stage"]=json!(stage(item["code"].as_str().unwrap_or("")));}
        item["input_sha256"]=json!(input_hash);item["rules_sha256"]=json!(rules);
    }
    Ok(json!({"schema_version":1,"mechanical_status":if failed{"fail"}else{"pass"},"source_coverage":coverage,
        "source_fidelity":"not_assessed","inputs":fingerprints,"input_sha256":input_hash,"rules_sha256":rules,
        "counts":counts,"checks":audit.checks,"review_prompts":audit.prompts}))
}
