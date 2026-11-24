//! Stateless native translation decisions. Rebuild from the immutable
//! prepared file and explicit ordered commands, not caller-supplied conclusions
//! about file identity, search results or candidate locations.
use crate::{files,path,path_text,parse_json,pipeline,translation_prepare as prep,translation_validate as tv,glossary,syntax};
use anyhow::{anyhow,ensure,Context,Result};
use serde_json::{json,Value,Map};
use std::{collections::{BTreeMap,BTreeSet,HashMap,HashSet},fs,io::Write,path::{Path,PathBuf}};

fn text<'a>(v:&'a Value,label:&str)->Result<&'a str>{v.as_str().filter(|s|!s.trim().is_empty()).with_context(||format!("TRW_REASON at {label}: supply nonempty text"))}
fn rows<'a>(v:&'a Value,label:&str)->Result<&'a Vec<Value>>{v.as_array().with_context(||format!("{label} must be an array"))}
fn ids(v:&Value,label:&str)->Result<Vec<String>>{
    let mut result=Vec::new();let mut seen=HashSet::new();
    for id in rows(v,label)?{let id=text(id,label)?;ensure!(seen.insert(id),"TRW_IDS at {label}: IDs must be unique");result.push(id.to_owned());}Ok(result)
}
fn absolute(p:&Path,base:&Path)->PathBuf{if p.is_absolute(){p.to_owned()}else{base.join(p)}}
fn bind(v:&Value,base:&Path)->Result<Value>{
    ensure!(v.is_object(),"TRW_BINDING: missing real file binding");
    let p=absolute(&path(&v["path"])?,base);let actual=prep::bind_file(&p)?;
    ensure!(actual["sha256"]==v["sha256"],"TRW_STALE_BINDING: File bytes changed: {}",p.display());Ok(actual)
}
fn bound_path(v:&Value,base:&Path)->Result<PathBuf>{path(&bind(v,base)?["path"])}
fn state(status:&Value,reason:&Value,evidence:Value,allowed:&[&str])->Result<Value>{
    ensure!(status.as_str().is_some_and(|s|allowed.contains(&s)),"TRW_STATUS: unknown explicit status");
    text(reason,"reason")?;Ok(json!({"status":status,"reason":reason,"evidence":evidence}))
}
fn with_locator(binding:&Value,locator:&Value)->Value{let mut v=binding.clone();v["locator"]=locator.clone();v}
fn verify_tree(v:&Value,base:&Path)->Result<()>{
    let mut pending=vec![v];let mut cache=HashMap::<PathBuf,String>::new();
    while let Some(v)=pending.pop(){
        if let Some(o)=v.as_object(){
            if o.contains_key("path")&&o.contains_key("sha256"){
                let p=files::canonical(&absolute(&path(&v["path"])?,base))?;
                let hash=if let Some(h)=cache.get(&p){h.clone()}else{let h=files::snapshot(&p)?["sha256"].as_str().unwrap().to_owned();cache.insert(p.clone(),h.clone());h};
                ensure!(v["sha256"]==hash,"TRW_STALE_BINDING: File bytes changed: {}",p.display());
            }
            pending.extend(o.values());
        }else if let Some(a)=v.as_array(){pending.extend(a);}
    }Ok(())
}
fn index(v:&Value,label:&str)->Result<BTreeMap<String,Value>>{
    let mut out=BTreeMap::new();
    for row in rows(v,label)?{
        ensure!(row.is_object(),"{label} needs objects");let id=text(&row["id"],label)?;
        ensure!(out.insert(id.to_owned(),row.clone()).is_none(),"{label} duplicate ID: {id}");
    }Ok(out)
}
fn matches(text:&str,quote:&str,start:usize,end:usize)->Vec<(usize,usize)>{
    if start>end||end>text.len(){return vec![];}
    let mut out=Vec::new();let mut cursor=start;
    while cursor<end{
        let Some(i)=text[cursor..end].find(quote)else{break;};
        let at=cursor+i;out.push((at,at+quote.len()));
        cursor=at+text[at..].chars().next().unwrap().len_utf8();
    }out
}
fn choose<T:Clone>(items:&[T],which:&Value,id:&str)->Result<T>{
    ensure!(!items.is_empty(),"TRW_LOCATION at {id}: exact selection is absent");
    if which.is_null(){ensure!(items.len()==1,"TRW_LOCATION_AMBIGUOUS at {id}: select an explicit occurrence");return Ok(items[0].clone());}
    let n=which.as_u64().filter(|n|*n>0&&*n<=items.len() as u64).context("TRW_LOCATION: selected occurrence does not exist")?;
    Ok(items[n as usize-1].clone())
}
pub fn range(text:&str,loc:&Value)->Result<(usize,usize)>{
    ensure!(loc.as_object().is_some_and(|m|!m.is_empty()),"TRW_LOCATION: Use an actual text locator.");
    ensure!(loc["kind"].is_null()||matches!(loc["kind"].as_str(),Some("lines"|"quote")),"TRW_LOCATION: unsupported candidate locator");
    let mut start=0;let mut end=text.len();
    let line_location=loc.get("lines").is_some()||loc["kind"]=="lines";
    if line_location{
        let mut l=loc.clone();l.as_object_mut().unwrap().remove("quote");
        (start,end)=tv::text_range(text,&l)?;
    }
    if !loc["quote"].is_null(){
        let quote=self::text(&loc["quote"],"quote")?;
        return choose(&matches(text,quote,start,end),&loc["occurrence"],"candidate");
    }
    ensure!(line_location,"TRW_LOCATION: Supply lines or exact unique quote");Ok((start,end))
}
fn bounds(v:&Value)->Option<[f64;4]>{
    let a=v.as_array().filter(|a|a.len()==4)?;
    let b=[a[0].as_f64()?,a[1].as_f64()?,a[2].as_f64()?,a[3].as_f64()?];
    (b.iter().all(|x|x.is_finite())&&b[0]<b[2]&&b[1]<b[3]).then_some(b)
}
fn within(a:[f64;4],b:[f64;4])->bool{a[0]<=b[0]&&b[2]<=a[2]&&a[1]<=b[1]&&b[3]<=a[3]}
#[derive(Clone)]
struct Selection{locator:Value,reading:String,raw:Option<String>,basis:Value}
fn hit(v:&Value)->Value{json!({"entry_id":v["id"],"glossary_locator":{"lines":[v["line"],v["line"]]},"excerpt":v["raw_line"]})}
pub struct Writer{
    pub prepared:Value,pub data:Value,pub commands:Vec<Value>,pub summary:Value,pub inventory:Value,
    pub source_bindings:BTreeMap<String,Value>,pub candidate_path:PathBuf,pub inventory_path:PathBuf,pub glossary_path:Option<PathBuf>,
    pub candidate_text:String,pub selected:Vec<String>,pub base:PathBuf,
    sources:BTreeMap<String,Value>,units:BTreeMap<String,Value>,cache:tv::Cache,index:Option<glossary::Glossary>
}
impl Writer{
    pub fn open(p:&Path)->Result<Self>{
        let (raw,prepared)=files::read_bound(p)?;let data=parse_json(std::str::from_utf8(&raw)?)?;
        ensure!(data["schema_version"].as_u64()==Some(1),"TRW_SCHEMA: use actual schema-version-1 prepared ledger");
        let p=path(&prepared["path"])?;let base=p.parent().unwrap().to_owned();
        verify_tree(&data,&base)?;
        let inventory_path=bound_path(&data["bindings"]["source_inventory"],&base)?;
        let candidate_path=bound_path(&data["bindings"]["candidate"],&base)?;
        let glossary_path=if data["bindings"]["glossary"].is_null(){None}else{Some(bound_path(&data["bindings"]["glossary"],&base)?)};
        let inventory=pipeline::read_json(&inventory_path)?;
        let sources=index(&inventory["sources"],"inventory sources")?;let units=index(&inventory["units"],"inventory units")?;
        let selected=ids(&data["scope"]["selected_units"],"selected units")?;
        ensure!(!selected.is_empty(),"TRW_TARGETS: no selected units");
        let selected_set:BTreeSet<_>=selected.iter().cloned().collect();
        let review_units=data["units"].as_object().context("TRW_TARGETS: review units")?;
        ensure!(review_units.keys().cloned().collect::<BTreeSet<_>>()==selected_set,"TRW_TARGETS: preserve every selected unit");
        ensure!(ids(&data["scope"]["unselected_inventory_units"],"unselected units")?.into_iter().collect::<BTreeSet<_>>()==units.keys().filter(|id|!selected_set.contains(*id)).cloned().collect(),"TRW_TARGETS: preserve unselected units");
        let mut bindings=BTreeMap::new();
        for(id,source)in &sources{bindings.insert(id.clone(),bind(source,inventory_path.parent().unwrap())?);}
        for id in &selected{
            let original=units.get(id).context("TRW_UNIT: unknown inventory unit")?;let unit=&review_units[id];
            let sid=text(&original["source"],"source identity")?;
            let source=sources.get(sid).context("TRW_UNIT: missing source")?;
            ensure!(unit["source_id"]==sid&&unit["kind"]==original["kind"],"TRW_UNIT: preserve source identity and kind");
            ensure!(unit["source_locations"]==prep::expand(original,source)?,"TRW_SOURCE_LOCATION: retain all source locations");
            ensure!(bind(&data["bindings"]["sources"][sid],&base)?==bindings[sid],"TRW_BINDING: source differs from inventory");
            for(key,expected)in [("checks",tv::CHECKS),("term_scan",tv::CATEGORIES),("occurrences",tv::KINDS)]{
                let actual=unit[key].as_object().context("TRW_TARGETS: missing targets")?;
                ensure!(actual.len()==expected.len()&&expected.iter().all(|k|actual.contains_key(*k)),"TRW_TARGETS: restore every required target");
            }
            rows(&unit["term_candidates"],"term candidates")?;
        }
        let summary=data["writer_metadata"]["terminology_summary"].as_object().map(|m|Value::Object(m.clone()))
            .unwrap_or(json!({"review_status":"pending","covered_units":[],"no_unlisted_reason":null}));
        let candidate_text=fs::read_to_string(&candidate_path)?;
        let mut result=Self{prepared,data,commands:vec![],summary,inventory,source_bindings:bindings,candidate_path,inventory_path,glossary_path,
            candidate_text,selected,base,sources,units,cache:tv::Cache::default(),index:None};
        result.verify()?;Ok(result)
    }
    pub fn verify(&mut self)->Result<()>{
        bind(&self.prepared,&self.base)?;verify_tree(&self.data,&self.base)?;
        for binding in self.source_bindings.values(){bind(binding,&self.base)?;}
        self.check_hits()
    }
    fn glossary(&mut self)->Result<&glossary::Glossary>{
        let p=self.glossary_path.as_ref().context("TRW_GLOSSARY: actual glossary required")?;
        let current=prep::bind_file(p)?;ensure!(current==self.data["bindings"]["glossary"],"TRW_STALE_BINDING: glossary changed");
        if self.index.is_none(){self.index=Some(glossary::Glossary::new(p)?);}
        let index=self.index.as_ref().unwrap();ensure!(index.binding==current,"TRW_STALE_BINDING: glossary index changed");Ok(index)
    }
    fn check_hits(&mut self)->Result<()>{
        let units=self.data["units"].as_object().unwrap().clone();
        for unit in units.values(){
            for term in rows(&unit["term_candidates"],"term_candidates")?{
                let mut hits=HashMap::new();
                for search in term["searches"].as_object().context("TRW_TARGETS: searches")?.values(){
                    for query in rows(&search["queries"],"queries")?{
                        let result=self.glossary()?.search(text(&query["query"],"query")?)?;
                        let expected:Vec<_>=rows(&result,"search result")?.iter().map(hit).collect();
                        ensure!(query["hits"]==json!(expected),"TRW_GLOSSARY_HIT: Stored hits differ from actual glossary query");
                        for h in expected{hits.insert(text(&h["entry_id"],"entry_id")?.to_owned(),h);}
                    }
                }
                for h in rows(&term["glossary_matches"],"glossary matches")?{
                    ensure!(hits.get(text(&h["entry_id"],"entry_id")?)==Some(h),"TRW_GLOSSARY_HIT: chosen match must be an actual query result");
                }
            }
        }Ok(())
    }
    fn inventory_evidence(&self,id:Option<&str>)->Result<Value>{
        let pointer=if let Some(id)=id{
            let i=rows(&self.inventory["units"],"units")?.iter().position(|v|v["id"]==id).context("TRW_UNIT: unknown inventory ID")?;
            format!("/units/{i}")
        }else{"/units".into()};
        Ok(json!([with_locator(&prep::bind_file(&self.inventory_path)?,&json!({"kind":"json_pointer","pointer":pointer}))]))
    }
    fn source_evidence(&self,id:&str)->Result<Vec<Value>>{
        let unit=&self.data["units"][id];let b=&self.source_bindings[text(&unit["source_id"],"source ID")?];
        Ok(rows(&unit["source_locations"],"source locations")?.iter().map(|l|with_locator(b,l)).collect())
    }
    fn evidence(&self,id:&str,source:Option<&Selection>,candidate:Option<&Selection>)->Result<Value>{
        let row=&self.data["units"][id];let b=&self.source_bindings[text(&row["source_id"],"source ID")?];
        let mut result=match source{Some(s)=>vec![with_locator(b,&s.locator)],None=>self.source_evidence(id)?};
        let locations=match candidate{Some(s)=>vec![s.locator.clone()],None=>rows(&row["candidate_locations"],"candidate locations")?.clone()};
        result.extend(locations.iter().map(|l|with_locator(&self.data["bindings"]["candidate"],l)));Ok(json!(result))
    }
    fn ranges(&self,id:&str)->Result<Vec<(usize,usize)>>{
        let locations=rows(&self.data["units"][id]["candidate_locations"],"candidate locations")?;
        ensure!(!locations.is_empty(),"TRW_LOCATION: Bind this unit to actual candidate locations once");
        locations.iter().map(|v|range(&self.candidate_text,v)).collect()
    }
    fn source(&mut self,id:&str,selector:&Value)->Result<Selection>{
        let quote=text(&selector["quote"],"source quote")?.to_owned();
        let row=self.data["units"][id].clone();
        let sid=text(&row["source_id"],"source ID")?;
        let p=path(&self.source_bindings[sid]["path"])?;
        let native=self.sources[sid]["format"]=="docx";
        let mut parents=rows(&row["source_locations"],"source locations")?.clone();
        let block=selector["block_id"].as_str();
        if let Some(id)=block{
            parents=parents.iter().flat_map(|p|p["block_locations"].as_array().into_iter().flatten()).filter(|p|p["block_id"]==id).cloned().collect();
            ensure!(!parents.is_empty(),"TRW_LOCATION: block ID does not belong to source unit");
        }
        if native{
            ensure!(selector["page"].is_null()&&selector["bbox"].is_null()&&selector["observation_basis"].is_null(),"TRW_LOCATION: native XML cannot invent PDF coordinates");
        }else{
            if !selector["page"].is_null(){let page=selector["page"].as_u64().filter(|n|*n>0).context("TRW_LOCATION: invalid page")?;
                parents.retain(|p|p["page"].as_u64()==Some(page));}
            if !selector["bbox"].is_null(){
                ensure!(!selector["page"].is_null(),"TRW_LOCATION: explicit bbox needs page");
                let b=bounds(&selector["bbox"]).context("TRW_LOCATION: invalid bbox")?;
                self.cache.location(&p,&json!({"page":selector["page"],"bbox":selector["bbox"]}))?;
                parents.retain(|p|bounds(&p["bbox"]).is_some_and(|a|within(a,b)));
            }
            if !selector["observation_basis"].is_null(){
                text(&selector["observation_basis"],"observation basis")?;
                ensure!(!selector["page"].is_null()&&!selector["bbox"].is_null(),"TRW_VISIBLE_BASIS: visible transcription needs page and bbox");
                let parent=choose(&parents,&selector["occurrence"],id)?;
                return Ok(Selection{locator:json!({"page":parent["page"],"bbox":selector["bbox"],"quote":quote,"observation_basis":selector["observation_basis"]}),reading:quote,raw:None,basis:selector["observation_basis"].clone()})
            }
        }
        let mut found=Vec::new();
        for parent in &parents{
            let s=parent["quote"].as_str().unwrap_or("");
            for(i,_)in matches(s,&quote,0,s.len()).iter().enumerate(){found.push((parent.clone(),i+1));}
        }
        let (parent,position)=choose(&found,&selector["occurrence"],id)?;
        let mut locator=if native{json!({"basis":"native","native_locator":parent["native_locator"],"quote":quote,"occurrence_in_recorded_span":position})}
        else{json!({"page":parent["page"],"quote":quote,"occurrence_in_recorded_span":position})};
        if !native&&parent.get("bbox").is_some(){locator["bbox"]=if selector["bbox"].is_null(){parent["bbox"].clone()}else{selector["bbox"].clone()};}
        if let Some(block)=block{locator["block_id"]=json!(block);}
        self.cache.location(&p,&locator)?;
        Ok(Selection{locator,reading:quote.clone(),raw:Some(quote),basis:Value::Null})
    }
    fn candidate(&self,id:&str,selector:&Value)->Result<Selection>{
        let quote=text(&selector["quote"],"candidate quote")?;let text_=&self.candidate_text;
        let ranges=self.ranges(id)?;
        let limits=if selector["lines"].is_null(){(0,text_.len())}else{range(text_,&json!({"lines":selector["lines"]}))?};
        let positions:BTreeSet<_>=ranges.iter().flat_map(|(a,b)|matches(text_,quote,(*a).max(limits.0),(*b).min(limits.1))).collect();
        let (begin,end)=choose(&positions.into_iter().collect::<Vec<_>>(),&selector["occurrence"],id)?;
        let reading=if selector["comparison_text"].is_null(){ensure!(selector["interpretation_basis"].is_null(),"TRW_RENDERED_BASIS: basis requires comparison text");quote}
        else{text(&selector["interpretation_basis"],"interpretation_basis")?;text(&selector["comparison_text"],"comparison_text")?};
        let start_line=text_.as_bytes()[..begin].iter().filter(|b|**b==b'\n').count()+1;
        let end_line=text_.as_bytes()[..end-1].iter().filter(|b|**b==b'\n').count()+1;
        let chars=[text_[..begin].chars().count(),text_[..end].chars().count()];
        let mut loc=json!({"lines":[start_line,end_line],"quote":quote,"char_range":chars});
        let (a,b)=range(text_,&json!({"lines":[start_line,end_line]}))?;
        if !ranges.iter().any(|(x,y)|*x<=a&&b<=*y){
            if matches(text_,quote,0,text_.len()).len()==1{loc=json!({"quote":quote,"source_lines":[start_line,end_line],"char_range":chars});}
            else{
                let (a,b)=ranges.iter().copied().find(|(a,b)|*a<=begin&&end<=*b&&matches(text_,&text_[*a..*b],0,text_.len()).len()==1)
                    .context("TRW_LOCATION: repeated inline fragment needs explicit lines or a unique containing source span")?;
                loc=json!({"quote":&text_[a..b],"selected_quote":quote,"source_lines":[start_line,end_line],"char_range":chars});
            }
        }
        Ok(Selection{locator:loc,reading:reading.into(),raw:Some(quote.into()),basis:selector["interpretation_basis"].clone()})
    }
    pub fn apply(&mut self,command:&Value)->Result<Value>{
        let command_obj=command.as_object().context("TRW_DRAFT: invalid short call")?;
        let op=text(&command["op"],"op")?;
        let allowed:&[&str]=match op{
            "scope"=>&["status","reason"],"exclude"=>&["unit_id","condition","reason"],
            "source_issues_review"=>&["status","reason"],"terminology_review"=>&["status","covered_units","no_unlisted_reason"],
            "review"=>&["status"],"unit"=>&["unit_id","candidate_locations"],
            "check"=>&["unit_id","name","status","reason"],"scan"=>&["unit_id","category","status","candidate_ids","reason"],
            "occurrence"=>&["unit_id","kind","occurrence_id","source","candidate","status","reason","comparison"],
            "term"=>&["unit_id","term_id","original","category","source","aliases"],
            "issue"=>&["unit_id","issue_id","source","description","candidate_readings","disposition","status","reason"],
            "search"=>&["unit_id","term_id","mode","query","interpretation"],
            "search_status"=>&["unit_id","term_id","mode","status","reason"],
            "decide"=>&["unit_id","term_id","decision","translation","reason","matches","resolution_basis"],
            _=>return Err(anyhow!("TRW_DRAFT: unknown operation {op}"))
        };
        ensure!(command_obj.keys().all(|k|k=="op"||allowed.contains(&k.as_str())),"TRW_DRAFT: unknown or protected command field");
        match op{
            "scope"=>{self.data["scope"]["selection_review"]=state(&command["status"],&command["reason"],self.inventory_evidence(None)?,tv::STATUSES)?;}
            "exclude"=>{
                let id=text(&command["unit_id"],"unit ID")?;
                ensure!(rows(&self.data["scope"]["unselected_inventory_units"],"unselected units")?.iter().any(|v|v==id),"TRW_UNIT: only an existing unselected unit can be excluded");
                ensure!(!rows(&self.data["scope"]["excluded_units"],"excluded units")?.iter().any(|v|v["id"]==id),"TRW_DUPLICATE: exclusion already recorded");
                text(&command["condition"],"condition")?;text(&command["reason"],"reason")?;
                let evidence=self.inventory_evidence(Some(id))?;
                self.data["scope"]["excluded_units"].as_array_mut().unwrap().push(json!({"id":id,"applicability_condition":command["condition"],"reason":command["reason"],"evidence":evidence}));
            }
            "source_issues_review"=>{
                let mut evidence=Vec::new();for id in &self.selected{evidence.extend(self.source_evidence(id)?);}
                self.data["source_issues_review"]=state(&command["status"],&command["reason"],json!(evidence),tv::STATUSES)?;
            }
            "terminology_review"=>{
                ensure!(matches!(command["status"].as_str(),Some("pending"|"reviewed")),"TRW_STATUS: pending or reviewed");
                let covered=ids(&command["covered_units"],"covered units")?;
                ensure!(covered.iter().all(|id|self.selected.contains(id)),"TRW_UNIT: terminology scope outside selected units");
                if !command["no_unlisted_reason"].is_null(){text(&command["no_unlisted_reason"],"no_unlisted_reason")?;}
                self.summary=json!({"review_status":command["status"],"covered_units":covered,"no_unlisted_reason":command["no_unlisted_reason"]});
            }
            "review"=>{
                ensure!(matches!(command["status"].as_str(),Some("pending"|"reviewed")),"TRW_STATUS: use pending or reviewed");
                self.data["review_status"]=command["status"].clone();
            }
            _=>{
                let id=text(&command["unit_id"],"unit ID")?.to_owned();
                ensure!(self.selected.contains(&id),"TRW_UNIT: use selected prepared unit");
                if op=="unit"{
                    let row=&self.data["units"][&id];
                    let supplied=&command["candidate_locations"];
                    let locs=if supplied.is_null(){row["candidate_locations"].clone()}else{supplied.clone()};
                    ensure!(locs.as_array().is_some_and(|a|!a.is_empty()),"TRW_LOCATION: bind complete candidate unit");
                    ensure!(rows(&row["candidate_locations"],"candidate locations")?.is_empty()||row["candidate_locations"]==locs,"TRW_LOCATION_REBOUND: keep prepared candidate mapping");
                    for loc in rows(&locs,"candidate locations")?{range(&self.candidate_text,loc)?;}
                    let p=path(&self.source_bindings[text(&row["source_id"],"source identity")?]["path"])?;
                    for l in rows(&row["source_locations"],"source locations")?{self.cache.location(&p,l)?;}
                    self.data["units"][&id]["candidate_locations"]=locs;
                }else{
                    self.ranges(&id)?;
                    match op{
                        "check"=>{
                            let name=text(&command["name"],"check")?;ensure!(tv::CHECKS.contains(&name),"TRW_CHECK: unknown check");
                            self.data["units"][&id]["checks"][name]=state(&command["status"],&command["reason"],self.evidence(&id,None,None)?,tv::STATUSES)?;
                        }
                        "scan"=>{
                            let category=text(&command["category"],"category")?;ensure!(tv::CATEGORIES.contains(&category),"TRW_CATEGORY: unknown category");
                            let ids=ids(&command["candidate_ids"],"candidate IDs")?;
                            let terms=index(&self.data["units"][&id]["term_candidates"],"terms")?;
                            ensure!(ids.iter().all(|s|terms.contains_key(s)),"TRW_TERM_MAPPING: register actual term candidates first");
                            ensure!(!(command["status"]=="scanned"&&ids.is_empty())&&!(command["status"]=="not_applicable"&&!ids.is_empty()),"TRW_APPLICABILITY: inconsistent term scan");
                            let mut r=state(&command["status"],&command["reason"],self.evidence(&id,None,None)?,tv::SCANS)?;
                            r["candidate_ids"]=json!(ids);self.data["units"][&id]["term_scan"][category]=r;
                        }
                        "term"=>{
                            let tid=text(&command["term_id"],"term ID")?;let original=text(&command["original"],"original term")?;let category=text(&command["category"],"category")?;
                            ensure!(!rows(&self.data["units"][&id]["term_candidates"],"terms")?.iter().any(|v|v["id"]==tid),"TRW_DUPLICATE: term ID");
                            let selection=self.source(&id,&command["source"])?;
                            ensure!(selection.reading.contains(original),"TRW_TERM_SOURCE: locate original term in selected excerpt");
                            let mut term=prep::term(tid,original,category,&selection.reading,&selection.locator)?;
                            term["aliases"]=if command["aliases"].is_null(){json!([])}else{json!(ids(&command["aliases"],"aliases")?)};
                            term["source_selection"]=json!({"raw_text":selection.raw,"comparison_text":selection.reading,"observation_basis":selection.basis});
                            self.data["units"][&id]["term_candidates"].as_array_mut().unwrap().push(term);
                        }
                        "occurrence"=>{
                            let kind=text(&command["kind"],"occurrence kind")?;ensure!(tv::KINDS.contains(&kind),"TRW_OCCURRENCE_KIND: unknown kind");
                            let oid=text(&command["occurrence_id"],"occurrence ID")?;
                            ensure!(!self.data["units"][&id]["occurrences"].as_object().unwrap().values().flat_map(|v|v.as_array().into_iter().flatten()).any(|v|v["id"]==oid),"TRW_DUPLICATE: occurrence ID");
                            let source=self.source(&id,&command["source"])?;let candidate=self.candidate(&id,&command["candidate"])?;
                            let mut compare=command["comparison"].as_object().context("TRW_COMPARISON_FIELD: explicit comparison object required")?.clone();
                            let pair=match kind{
                                "claims"|"citation_claims"=>Some(("source_claim","candidate_claim")),
                                "numbers"|"units"|"symbols"|"formulas"=>Some(("source_expression","candidate_expression")),
                                "citation_labels"=>Some(("source_label","candidate_label")),_=>None
                            };
                            if let Some((a,b))=pair{
                                for (k,s) in [(a,&source),(b,&candidate)]{
                                    ensure!(compare.get(k).is_none_or(|v|v==&json!(s.reading)),"TRW_COMPARISON_TEXT: use explicitly selected text");
                                    compare.insert(k.into(),json!(s.reading));
                                }
                            }
                            if kind=="table_cells"{
                                for (k,s) in [("source_cell",&source),("candidate_cell",&candidate)]{
                                    let cell=compare.get_mut(k).and_then(Value::as_object_mut).context("TRW_COMPARISON_FIELD: actual table ID, row and column required")?;
                                    ensure!(cell.get("text").is_none_or(|v|v==&json!(s.reading)),"TRW_COMPARISON_TEXT: cell text differs");
                                    cell.insert("text".into(),json!(s.reading));
                                }
                            }
                            let expected=tv::fields(kind);
                            ensure!(compare.len()==expected.len()&&expected.iter().all(|k|compare.contains_key(*k)),"TRW_COMPARISON_FIELD: supply all independent comparison fields");
                            let mut row=prep::occurrence(oid,kind,&source.locator)?;
                            row["candidate_span"]=candidate.locator.clone();
                            row.as_object_mut().unwrap().extend(compare);
                            row.as_object_mut().unwrap().extend(state(&command["status"],&command["reason"],self.evidence(&id,Some(&source),Some(&candidate))?,tv::STATUSES)?.as_object().unwrap().clone());
                            for(side,s)in [("source",&source),("candidate",&candidate)]{
                                row[format!("{side}_selection")]=json!({"raw_text":s.raw,"comparison_text":s.reading,"interpretation_basis":s.basis});
                            }
                            let mut validator=self.validator()?;
                            let mut groups=Map::new();for k in tv::KINDS{groups.insert((*k).into(),if *k==kind{json!([row])}else{json!([])});}
                            validator.occurrences(&Value::Object(groups),&self.data["units"][&id],text(&self.data["units"][&id]["source_id"],"source ID")?,&self.ranges(&id)?,&id,&Map::new());
                            if let Some(e)=validator.report.findings.iter().find(|f|f.level=="error"){return Err(anyhow!("{} at {}: {}",e.code,e.object_id,e.message));}
                            self.data["units"][&id]["occurrences"][kind].as_array_mut().unwrap().push(row);
                        }
                        "issue"=>{
                            let issue=text(&command["issue_id"],"issue ID")?;
                            ensure!(!rows(&self.data["source_issues"],"issues")?.iter().any(|v|v["id"]==issue),"TRW_DUPLICATE: issue ID");
                            let source=self.source(&id,&command["source"])?;
                            let readings=ids(&command["candidate_readings"],"readings")?;ensure!(!readings.is_empty(),"TRW_ISSUE: actual readings required");
                            text(&command["description"],"description")?;text(&command["disposition"],"disposition")?;
                            let mut row=json!({"id":issue,"unit_id":id,"source_locator":source.locator,"description":command["description"],"candidate_readings":readings,"disposition":command["disposition"]});
                            row.as_object_mut().unwrap().extend(state(&command["status"],&command["reason"],self.evidence(&id,Some(&source),None)?,tv::STATUSES)?.as_object().unwrap().clone());
                            self.data["source_issues"].as_array_mut().unwrap().push(row);
                        }
                        "search"|"search_status"|"decide"=>self.term_command(&id,op,command)?,
                        _=>unreachable!()
                    }
                }
            }
        }
        self.commands.push(command.clone());Ok(Value::Null)
    }
    fn term_command(&mut self,id:&str,op:&str,c:&Value)->Result<()>{
        let tid=text(&c["term_id"],"term ID")?;
        let i=rows(&self.data["units"][id]["term_candidates"],"terms")?.iter().position(|v|v["id"]==tid).context("TRW_TERM_MAPPING: unknown term")?;
        let mut term=self.data["units"][id]["term_candidates"][i].clone();
        if op=="decide"{
            let decision=text(&c["decision"],"decision")?;
            ensure!(["pending","adopted","unlisted","unresolved","not_a_term"].contains(&decision),"TRW_TERM_DECISION: unknown decision");
            text(&c["reason"],"reason")?;
            if matches!(decision,"adopted"|"unlisted"){text(&c["translation"],"translation")?;}
            if decision=="unlisted"{text(&c["resolution_basis"],"resolution basis")?;}
            let ids=if c["matches"].is_null(){vec![]}else{ids(&c["matches"],"matches")?};
            let mut hits=HashMap::new();
            for s in term["searches"].as_object().unwrap().values(){for q in rows(&s["queries"],"queries")?{for h in rows(&q["hits"],"hits")?{hits.insert(text(&h["entry_id"],"entry ID")?,h.clone());}}}
            ensure!(ids.iter().all(|id|hits.contains_key(id.as_str()))&&(decision!="adopted"||!ids.is_empty()),"TRW_GLOSSARY_HIT: choose only actual search result IDs");
            // Resolve the selected hits before mutating the term that owns their borrowed IDs.
            let glossary_matches=json!(ids.iter().map(|id|hits[id.as_str()].clone()).collect::<Vec<_>>());
            term["decision"]=c["decision"].clone();term["translation"]=c["translation"].clone();term["decision_reason"]=c["reason"].clone();
            term["glossary_matches"]=glossary_matches;term["resolution_basis"]=c["resolution_basis"].clone();
        }else{
            let mode=text(&c["mode"],"search mode")?;
            ensure!(["full_phrase","aliases","family"].contains(&mode),"TRW_SEARCH_MODE: unknown mode");
            let aliases=rows(&term["aliases"],"aliases")?;
            if op=="search"{
                ensure!(mode!="aliases"||!aliases.is_empty(),"TRW_ALIASES: register alias before searching");
                let query=text(&c["query"],"query")?;let interpretation=text(&c["interpretation"],"interpretation")?;
                let index=self.glossary()?;let result=index.search(query)?;
                let hits=rows(&result,"hits")?.iter().map(hit).collect::<Vec<_>>();
                let b=index.binding.clone();let p=path(&b["path"])?;let lines=syntax::lines(&fs::read_to_string(p)?).len();
                let evidence=json!([with_locator(&b,&json!({"lines":[1,lines]}))]);
                let search=&mut term["searches"][mode];
                search.as_object_mut().unwrap().extend(state(&json!("searched"),&c["interpretation"],evidence,&["searched"])?.as_object().unwrap().clone());
                search["queries"].as_array_mut().unwrap().push(json!({"query":query,"hits":hits,"interpretation":interpretation}));
            }else{
                ensure!(c["status"]!="searched","TRW_SEARCH_ACTION: perform an actual query");
                if c["status"]=="not_applicable"{ensure!(mode=="aliases"&&aliases.is_empty(),"TRW_APPLICABILITY: full phrase and family remain required");}
                ensure!(rows(&term["searches"][mode]["queries"],"queries")?.is_empty(),"TRW_SEARCH_ACTION: do not erase performed queries");
                let r=state(&c["status"],&c["reason"],self.evidence(id,None,None)?,&["pending","not_applicable","blocked","unresolved"])?;
                term["searches"][mode].as_object_mut().unwrap().extend(r.as_object().unwrap().clone());
            }
        }
        self.data["units"][id]["term_candidates"][i]=term;Ok(())
    }
    pub fn validator(&self)->Result<tv::Validator>{
        let mut v=tv::Validator::new(&self.base);
        v.units=self.units.clone();v.sources=self.sources.clone();
        for (id,b) in &self.source_bindings{v.source_paths.insert(id.clone(),path(&b["path"])?);}
        v.candidate=Some(self.candidate_path.clone());v.glossary=self.glossary_path.clone();
        Ok(v)
    }
    pub fn compile(&self)->Result<(Value,Value)>{
        let mut payload=self.data.clone();
        let mut result=json!({"schema_version":1,"glossary":payload["bindings"]["glossary"],"adopted":[],"unlisted":[]});
        result.as_object_mut().unwrap().extend(self.summary.as_object().context("terminology summary")?.clone());
        let mut unresolved=Vec::new();
        // Preserve selected-unit order from the immutable preparation.
        for uid in &self.selected{
            for term in payload["units"][uid]["term_candidates"].as_array_mut().unwrap(){
                // Own the decision across the subsequent mutation of this term.
                let decision=text(&term["decision"],"term decision")?.to_owned();
                if matches!(decision.as_str(),"pending"|"not_a_term"){continue;}
                let section=if decision=="adopted"{"adopted"}else{"unlisted"};
                let rows=result[section].as_array_mut().unwrap();
                term["terminology_pointer"]=json!(format!("/{section}/{}",rows.len()));
                let mut entry=json!({"original":term["original"],"translation":term["translation"],"context":term["source_excerpt"],"source_units":[uid]});
                if decision=="adopted"{
                    let line=term["glossary_matches"].as_array().and_then(|a|a.first()).and_then(|v|v["glossary_locator"]["lines"][0].as_u64()).context("TRW_GLOSSARY_HIT: chosen glossary match required")?;
                    entry["glossary_locator"]=json!(format!("{}#L{line}",self.glossary_path.as_ref().context("glossary path")?.display()));
                }else{
                    entry["status"]=json!(if decision=="unresolved"{"unresolved"}else{"resolved"});entry["reason"]=term["decision_reason"].clone();entry["resolution_basis"]=term["resolution_basis"].clone();
                }
                if decision=="unresolved"{unresolved.push(json!({"unit_id":uid,"candidate_id":term["id"],"reason":term["decision_reason"]}));}
                rows.push(entry);
            }
        }
        payload["unresolved_terms"]=json!(unresolved);
        payload["writer_metadata"]=json!({"format":"explicit-translation-decisions-v1","terminology_summary":self.summary,"source_fidelity":"not_assessed","execution_truth":"not_assessed"});
        Ok((payload,result))
    }
    pub fn scope_report(&self)->Result<tv::Report>{
        let mut v=self.validator()?;
        v.scope(&self.data["scope"],self.data["units"].as_object().unwrap());Ok(v.report)
    }
    pub fn unit_report(&self,id:&str)->Result<tv::Report>{
        ensure!(self.selected.iter().any(|s|s==id),"Choose an actual selected unit");
        let (payload,terminology)=self.compile()?;
        let mut v=self.validator()?;v.terminology=terminology;v.unit(id,&payload["units"][id]);Ok(v.report)
    }
    pub fn draft(&mut self,p:&Path)->Result<PathBuf>{
        self.verify()?;
        let v=json!({"schema_version":1,"format":"translation-review-decisions-v1","prepared":self.prepared,"decisions":self.commands,
            "boundary":"Explicit decisions only. Resume requires the original unchanged prepared inputs; no cross-version inheritance."});
        write_new(p,&v)
    }
    pub fn from_draft(p:&Path)->Result<Self>{
        let d=pipeline::read_json(p)?;
        ensure!(d["schema_version"].as_u64()==Some(1)&&d["format"]=="translation-review-decisions-v1","TRW_DRAFT: use a writer decision draft");
        let prepared=bind(&d["prepared"],p.parent().unwrap_or(Path::new(".")))?;
        let mut writer=Self::open(&path(&prepared["path"])?)?;
        ensure!(writer.prepared==prepared,"TRW_STALE_BINDING: prepared file changed");
        for command in rows(&d["decisions"],"decisions")?{writer.apply(command)?;}
        writer.verify()?;Ok(writer)
    }
    pub fn save(&mut self,p:&Path,term:Option<&Path>)->Result<Value>{
        self.verify()?;
        let dest=planned(p)?;
        ensure!(!dest.exists()&&!dest.is_symlink(),"Keep the existing record: {}",dest.display());
        let term=term.map(planned).transpose()?;
        if let Some(q)=&term{ensure!(q!=&dest&&!q.exists()&&!q.is_symlink(),"Review and terminology need separate new files");}
        let (mut payload,terminology)=self.compile()?;
        if term.is_none(){
            // Match the existing writer: compiling pointers is tied to actually
            // publishing the new terminology record, never a silent rebind.
            let metadata=payload["writer_metadata"].clone();
            payload=self.data.clone();payload["writer_metadata"]=metadata;
        }
        // Normal prepared records use absolute bindings. Refuse a relocation
        // that would reinterpret a relative path as another file.
        verify_tree(&payload,&self.base)?;verify_tree(&payload,dest.parent().unwrap())?;
        if let Some(q)=term{
            write_new(&q,&terminology)?;payload["terminology_record"]=prep::bind_file(&q)?;
        }
        self.verify()?;verify_tree(&payload,dest.parent().unwrap())?;
        let out=write_new(&dest,&payload)?;
        tv::validate(&out,Some(&self.inventory_path),Some(&self.candidate_path))
    }
}
fn planned(p:&Path)->Result<PathBuf>{
    let parent=p.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or_else(||Path::new("."));
    fs::create_dir_all(parent)?;Ok(files::canonical(parent)?.join(p.file_name().context("output filename")?))
}
fn write_new(p:&Path,v:&Value)->Result<PathBuf>{
    let p=planned(p)?;
    let mut bytes=serde_json::to_vec_pretty(v)?;bytes.push(b'\n');
    let mut tmp=tempfile::Builder::new().prefix(".pending-translation-").tempfile_in(p.parent().unwrap())?;
    tmp.write_all(&bytes)?;tmp.as_file().sync_all()?;
    tmp.persist_noclobber(&p).map_err(|e|anyhow!("Keep the existing record or inspect failed output: {}",e.error))?;
    ensure!(pipeline::read_json(&p)?==*v,"Translation record did not round-trip");Ok(p)
}
pub fn replay(prepared:&Path,commands:&Value)->Result<Writer>{
    let mut writer=Writer::open(prepared)?;
    let list=rows(commands,"decisions")?;
    for(i,command)in list.iter().enumerate(){writer.apply(command)?;if (i+1)%128==0||i+1==list.len(){eprintln!("[{}/{}] translation decisions",i+1,list.len());}}
    writer.verify()?;Ok(writer)
}
