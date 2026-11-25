//! Native drafting state machine. Every call reloads and validates the
//! frozen inventory; no source semantics or review results are inferred.
use crate::{check_ledger,check_write,files,path,path_text,pipeline,parse_json};
use anyhow::{anyhow,ensure,Context,Result};
use serde_json::{json,Map,Value};
use std::{collections::{HashMap,HashSet},fs,io::Write,path::{Path,PathBuf}};

const BOUNDARY:&str="Draft coverage and file integrity only. Inventoried blocks, media calls, explicit exclusions and written fragments do not establish source completeness, translation fidelity, correct grouping or visual review. No acceptance ID receives a pass.";
fn text<'a>(v:&'a Value,label:&str)->Result<&'a str>{v.as_str().filter(|s|!s.trim().is_empty()).with_context(||format!("{label} must be nonempty text"))}
fn obj<'a>(v:&'a Value,label:&str)->Result<&'a Map<String,Value>>{v.as_object().with_context(||format!("Queue {label} must be an object"))}
fn rows<'a>(v:&'a Value,label:&str)->Result<&'a Vec<Value>>{v.as_array().filter(|a|a.iter().all(Value::is_object)).with_context(||format!("{label} must be an array of objects"))}
fn ids<'a>(v:&'a Value,label:&str)->Result<Vec<&'a str>>{
    let list=v.as_array().with_context(||format!("{label} must be a sequence of IDs"))?;
    let mut seen=HashSet::new();let mut result=Vec::with_capacity(list.len());
    for v in list{
        let s=v.as_str().filter(|s|!s.is_empty()).with_context(||format!("{label} contains an invalid ID"))?;
        ensure!(seen.insert(s),"{label} contains duplicate IDs");result.push(s);
    }Ok(result)
}
fn array_set<'a>(v:&'a Value,label:&str)->Result<HashSet<&'a str>>{Ok(ids(v,label)?.into_iter().collect())}
fn file(p:&Path)->Result<Value>{check_ledger::bind_file(p)}
fn binding(v:&Value,base:&Path)->Result<Value>{
    ensure!(v.is_object()&&v["path"].is_string(),"Every source needs a file path and SHA-256");
    let p=path(&v["path"])?;let p=if p.is_absolute(){p}else{base.join(p)};
    let current=file(&p)?;ensure!(current["sha256"]==v["sha256"],"Stale source binding: {}",p.display());Ok(current)
}
fn page_key(source:&str,page:u64)->String{serde_json::to_string(&json!([source,page])).expect("page key")}
fn serialize(v:&Value)->Result<Vec<u8>>{let mut bytes=serde_json::to_vec_pretty(v)?;bytes.push(b'\n');Ok(bytes)}
fn new_file(p:&Path,bytes:&[u8])->Result<()>{
    let parent=p.parent().context("output parent")?;
    fs::create_dir_all(parent)?;
    ensure!(!p.exists()&&!p.is_symlink(),"Keep existing output: {}",p.display());
    let mut temp=tempfile::Builder::new().prefix(".pending-draft-").tempfile_in(parent)?;
    temp.write_all(bytes)?;temp.as_file().sync_all()?;
    temp.persist_noclobber(p).map_err(|e|anyhow!("Cannot exclusively save {}: {}",p.display(),e.error))?;
    ensure!(fs::read(p)?==bytes,"Draft file readback differs");Ok(())
}
fn output(p:&Path)->Result<PathBuf>{
    let parent=p.parent().filter(|s|!s.as_os_str().is_empty()).unwrap_or_else(||Path::new("."));
    fs::create_dir_all(parent)?;
    Ok(files::canonical(parent)?.join(p.file_name().context("output filename")?))
}
fn same_set(values:&Value,expected:&HashSet<&str>,label:&str)->Result<()>{
    ensure!(&array_set(values,label)?==expected,"{label} does not cover the actual source entries");Ok(())
}
fn add_block(blocks:&mut Map<String,Value>,id:&str,mut v:Value)->Result<()>{
    ensure!(!id.trim().is_empty(),"source block ID must be nonempty text");
    ensure!(!blocks.contains_key(id),"Duplicate source block ID: {id}");
    v["id"]=json!(id);blocks.insert(id.to_owned(),v);Ok(())
}
fn extract(inventory:&Value,base:&Path,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    let sources=rows(&inventory["sources"],"inventory.sources")?;
    ensure!(!sources.is_empty(),"A drafting queue needs actual sources");
    let mut order=Vec::new();let mut source_map=Map::new();let mut blocks=Map::new();let mut pages=Map::new();let mut issues=Map::new();
    for (i,source) in sources.iter().enumerate(){
        let id=text(&source["id"],"source ID")?;
        ensure!(!source_map.contains_key(id),"Duplicate source ID: {id}");
        let bound=binding(source,base)?;
        let extension=Path::new(bound["path"].as_str().unwrap()).extension().and_then(|s|s.to_str()).unwrap_or("").to_lowercase();
        let format=source["format"].as_str().filter(|s|!s.is_empty()).unwrap_or(&extension);
        ensure!(matches!(format,"pdf"|"docx"),"Unsupported queue source format: {format}");
        order.push(id.to_owned());let mut block_ids=Vec::new();let mut page_keys=Vec::new();
        if format=="docx"{
            let empty=json!([]);
            let calls=rows(source.get("media_calls").unwrap_or(&empty),"DOCX media_calls")?;
            let mut by_block:HashMap<&str,Vec<&Value>>=HashMap::new();
            for call in calls{if let Some(id)=call["block_id"].as_str(){by_block.entry(id).or_default().push(call);}}
            for block in rows(&source["native_blocks"],"DOCX native_blocks")?{
                let bid=text(&block["id"],"DOCX block ID")?;
                add_block(&mut blocks,bid,json!({"source":id,"kind":block.get("kind").cloned().unwrap_or(json!("native")),
                    "text":block.get("text").cloned().unwrap_or(json!("")),"native_locator":block["locator"],
                    "media_calls":by_block.get(bid).cloned().unwrap_or_default(),"native_record":block}))?;
                block_ids.push(bid.to_owned());
            }
            for(j,issue)in rows(source.get("native_issues").unwrap_or(&empty),"DOCX native_issues")?.iter().enumerate(){
                issues.insert(format!("{id}:native-issue:{}",j+1),json!({"source":id,"original":issue,"decision":null}));
            }
        }else{
            let ps=rows(&source["pages"],"PDF pages")?;
            ensure!(!ps.is_empty()&&source["page_count"].as_u64()==Some(ps.len() as u64),"PDF page_count and page records disagree");
            for(j,page)in ps.iter().enumerate(){
                let n=(j+1)as u64;ensure!(page["page"].as_u64()==Some(n),"Retain every actual PDF page once in physical order");
                let key=page_key(id,n);let mut page_ids=Vec::new();
                for block in rows(&page["blocks"],"PDF blocks")?{
                    let bid=text(&block["id"],"PDF block ID")?;
                    add_block(&mut blocks,bid,json!({"source":id,"page":n,"kind":"text","text":block.get("text").cloned().unwrap_or(json!("")),
                        "bbox":block["bbox"],"original_block":block}))?;
                    page_ids.push(bid.to_owned());
                }
                let image_index=page.get("image_index").cloned().unwrap_or(json!({}));
                ensure!(image_index.is_object(),"PDF image_index must be an object");
                let empty=json!([]);
                for(k,call)in rows(image_index.get("occurrences").unwrap_or(&empty),"PDF image occurrences")?.iter().enumerate(){
                    let bid=format!("{id}-p{n}-image-call-{}",k+1);
                    add_block(&mut blocks,&bid,json!({"source":id,"page":n,"kind":"image_call","text":"","bbox":call["bbox"],"image_call":call}))?;
                    page_ids.push(bid);
                }
                let mut frozen_page=json!({"source":id,"page":n,"block_ids":page_ids,"render":page["render"],"text":page["text"],
                    "image_index":image_index,"order_review":null,"additional_object_review":null});
                if source["inventory_backend"]==crate::pdf_inventory::BACKEND{
                    ensure!(page["pdf_evidence"].is_object(),"Retain actual native PDF evidence");
                    frozen_page["pdf_evidence"]=page["pdf_evidence"].clone();
                }
                pages.insert(key.clone(),frozen_page);
                page_keys.push(key);block_ids.extend(page_ids);
            }
        }
        source_map.insert(id.to_owned(),json!({"path":bound["path"],"sha256":bound["sha256"],"format":format,"role":source["role"],
            "block_ids":block_ids,"page_keys":page_keys,"native_media":source.get("native_media").cloned().unwrap_or(json!([]))}));
        progress(i+1,sources.len(),"索引实际来源");
    }
    Ok(json!({"source_order":order,"sources":source_map,"blocks":blocks,"pages":pages,"issues":issues}))
}
pub fn prepare(inventory:&Path,queue:&Path,progress:impl FnMut(usize,usize,&str))->Result<PathBuf>{
    let inventory=files::canonical(inventory)?;
    let queue=output(queue)?;
    let stem=queue.file_stem().and_then(|s|s.to_str()).context("queue stem")?;
    let snapshot=queue.with_file_name(format!("{stem}-source-snapshot.json"));
    let parts=queue.with_file_name(format!("{stem}-parts"));
    let _lock=check_write::lock_target(&queue)?;
    ensure!(!queue.exists()&&!snapshot.exists()&&!queue.is_symlink()&&!snapshot.is_symlink(),"Resume the existing queue; do not overwrite its snapshot");
    ensure!(queue!=inventory&&snapshot!=inventory,"A queue must not replace its source inventory");
    let(raw,inv_binding)=files::read_bound(&inventory)?;
    let inv=parse_json(std::str::from_utf8(&raw)?)?;
    ensure!(inv.is_object(),"The inventory must be a JSON object");
    let mut data=extract(&inv,inventory.parent().unwrap(),progress)?;
    for source in obj(&data["sources"],"sources")?.values(){
        let p=Path::new(source["path"].as_str().unwrap());
        ensure!(queue!=p&&snapshot!=p,"A queue must not replace a source");
        binding(source,inventory.parent().unwrap())?;
    }
    ensure!(files::snapshot(&inventory)?["sha256"]==inv_binding["sha256"],"Source inventory changed during preparation");
    data.as_object_mut().unwrap().extend(json!({"schema_version":1,"boundary":BOUNDARY,
        "inventory_origin":path_text(&inventory)?,"inventory_snapshot":{"path":path_text(&snapshot)?,"sha256":files::digest(&raw)},
        "parts_root":path_text(&parts)?,"units":{},"exclusions":{},"history":[]}).as_object().unwrap().clone());
    new_file(&snapshot,&raw)?;new_file(&queue,&serialize(&data)?)?;Ok(queue)
}
struct Queue{path:PathBuf,data:Value,hash:String,baseline:Value}
impl Queue{
    fn open(p:&Path)->Result<Self>{
        check_write::no_link(p)?;
        let p=files::canonical(p)?;let(raw,b)=files::read_bound(&p)?;
        let data=parse_json(std::str::from_utf8(&raw)?)?;
        ensure!(data["schema_version"].as_u64()==Some(1),"Unsupported drafting queue schema");
        let snapshot=binding(&data["inventory_snapshot"],p.parent().unwrap())?;
        let original=path(&data["inventory_origin"])?;
        let snapshot_path=path(&snapshot["path"])?;
        let stem=p.file_stem().and_then(|s|s.to_str()).context("queue stem")?;
        ensure!(snapshot_path==p.with_file_name(format!("{stem}-source-snapshot.json"))&&
            path(&data["parts_root"])?==p.with_file_name(format!("{stem}-parts")),"Queue snapshot or parts path was replaced");
        let baseline=extract(&pipeline::read_json(&snapshot_path)?,original.parent().context("inventory origin directory")?,|_,_,_|{})?;
        let q=Self{path:p,data,hash:b["sha256"].as_str().unwrap().to_owned(),baseline};
        q.validate()?;Ok(q)
    }
    fn validate(&self)->Result<()>{
        let d=&self.data;let b=&self.baseline;
        for name in ["sources","blocks","pages","units","exclusions","issues"]{obj(&d[name],name)?;}
        ensure!(d["source_order"]==b["source_order"],"Do not drop or reorder sources from the frozen inventory");
        let sources=obj(&d["sources"],"sources")?;let blocks=obj(&d["blocks"],"blocks")?;let pages=obj(&d["pages"],"pages")?;
        let baseline_sources=obj(&b["sources"],"sources")?;let baseline_pages=obj(&b["pages"],"pages")?;
        ensure!(sources.keys().eq(baseline_sources.keys()),"Source identities differ from frozen inventory");
        ensure!(pages.keys().eq(baseline_pages.keys()),"Keep every frozen source page");
        for (id,v) in obj(&b["blocks"],"baseline blocks")?{ensure!(blocks.get(id)==Some(v),"Keep the frozen source entry unchanged: {id}");}
        let frozen=obj(&b["blocks"],"baseline blocks")?;
        for(id,entry)in blocks{
            ensure!(entry["id"]==id.as_str(),"Source block identity changed");
            let sid=text(&entry["source"],"block source")?;
            ensure!(sources.contains_key(sid),"Block refers to an unknown source");
            if !frozen.contains_key(id){
                let n=entry["page"].as_u64().context("Additional entries must refer to an actual PDF page")?;
                ensure!(entry["added_from_page"]==true&&pages.contains_key(&page_key(sid,n)),"Additional entries must refer to an actual PDF page");
                text(&entry["kind"],"source object kind")?;text(&entry["registration_reason"],"registration reason")?;
                ensure!(entry["text"].is_string(),"Visible transcription must be text");
            }
        }
        let mut source_entries:HashMap<&str,HashSet<&str>>=HashMap::new();
        let mut page_entries:HashMap<String,HashSet<&str>>=HashMap::new();
        for(id,block)in blocks{
            let sid=block["source"].as_str().unwrap();source_entries.entry(sid).or_default().insert(id);
            if let Some(n)=block["page"].as_u64(){page_entries.entry(page_key(sid,n)).or_default().insert(id);}
        }
        let empty=HashSet::new();
        for(id,expected)in baseline_sources{
            let actual=&sources[id];
            for field in ["path","sha256","format","role","page_keys","native_media"]{ensure!(actual[field]==expected[field],"Keep the frozen source identity: {id}");}
            same_set(&actual["block_ids"],source_entries.get(id.as_str()).unwrap_or(&empty),"The source entry list")?;
            if actual["format"]=="docx"{ensure!(actual["block_ids"]==expected["block_ids"],"Keep native DOCX XML order");}
        }
        for(key,expected)in baseline_pages{
            let page=&pages[key];
            for field in ["source","page","render","text","image_index","pdf_evidence"]{ensure!(page[field]==expected[field],"Keep frozen page evidence unchanged");}
            let entries=page_entries.get(key).unwrap_or(&empty);
            same_set(&page["block_ids"],entries,"The page entry list")?;
            if !page["order_review"].is_null(){
                same_set(&page["order_review"]["block_ids"],entries,"The confirmed reading order")?;
                text(&page["order_review"]["reason"],"reading order reason")?;
            }
            if !page["additional_object_review"].is_null(){text(&page["additional_object_review"]["reason"],"additional object review")?;}
        }
        let issues=obj(&d["issues"],"issues")?;let original=obj(&b["issues"],"baseline issues")?;
        ensure!(issues.keys().eq(original.keys()),"Keep every native source issue visible");
        for(id,expected)in original{
            let issue=&issues[id];ensure!(issue["source"]==expected["source"]&&issue["original"]==expected["original"],"Keep each original native issue unchanged");
            if !issue["decision"].is_null(){
                let v=&issue["decision"];
                ensure!(matches!(v["status"].as_str(),Some("resolved"|"excluded"|"blocked")),"Keep an explicit valid native issue disposition");
                text(&v["reason"],"native issue decision reason")?;
                ensure!(!rows(&v["evidence"],"native issue evidence")?.is_empty(),"A native issue disposition requires evidence");
            }
        }
        let mut assigned=HashSet::new();
        for(id,unit)in obj(&d["units"],"units")?{
            ensure!(unit["id"]==id.as_str(),"Keep the actual drafting unit identity");
            text(&unit["kind"],"draft unit kind")?;text(&unit["grouping_reason"],"grouping reason")?;
            rows(&unit["revisions"],"draft revisions")?;
            let list=ids(&unit["block_ids"],"unit block IDs")?;
            ensure!(!list.is_empty(),"A unit must refer to actual source entries: {id}");
            for bid in list{
                let block=blocks.get(bid).with_context(||format!("A unit must refer to actual source entries: {id}"))?;
                ensure!(block["source"]==unit["source"],"Keep different sources in different drafting units");
                ensure!(assigned.insert(bid),"A source entry cannot be assigned to multiple units");
            }
        }
        for(id,row)in obj(&d["exclusions"],"exclusions")?{
            ensure!(blocks.contains_key(id),"An exclusion must name an actual source entry");
            ensure!(!assigned.contains(id.as_str()),"A source entry cannot be both assigned and excluded");
            text(&row["reason"],"individual exclusion reason")?;
        }
        rows(&d["history"],"history")?;
        Ok(())
    }
    fn page(&self,sid:&str,n:u64)->Result<String>{
        let key=page_key(sid,n);ensure!(self.data["pages"].get(&key).is_some(),"Choose an actual PDF source page");Ok(key)
    }
    fn assigned(&self)->HashMap<String,String>{
        let mut result=HashMap::new();
        for(id,unit)in self.data["units"].as_object().unwrap(){for bid in unit["block_ids"].as_array().unwrap(){result.insert(bid.as_str().unwrap().into(),id.clone());}}result
    }
    fn ordered_blocks(&self)->Vec<String>{
        let mut out=Vec::new();
        for id in self.data["source_order"].as_array().unwrap(){
            let s=&self.data["sources"][id.as_str().unwrap()];
            if s["format"]=="docx"{out.extend(s["block_ids"].as_array().unwrap().iter().map(|v|v.as_str().unwrap().to_owned()));continue;}
            for k in s["page_keys"].as_array().unwrap(){
                let p=&self.data["pages"][k.as_str().unwrap()];
                let v=if p["order_review"].is_null(){&p["block_ids"]}else{&p["order_review"]["block_ids"]};
                out.extend(v.as_array().unwrap().iter().map(|v|v.as_str().unwrap().to_owned()));
            }
        }out
    }
    fn unit_order(&self)->Vec<String>{
        let units=self.data["units"].as_object().unwrap();let mut seen=HashSet::new();let mut out=Vec::new();
        for event in self.data["history"].as_array().unwrap(){
            if event["action"]=="group"{if let Some(id)=event["target"].as_str().filter(|s|units.contains_key(*s)){if seen.insert(id){out.push(id.to_owned());}}}
        }
        for id in units.keys(){if seen.insert(id){out.push(id.clone());}}out
    }
    fn status(&self)->Value{
        let d=&self.data;let assigned=self.assigned();
        let undisposed:Vec<_>=self.ordered_blocks().into_iter().filter(|id|!assigned.contains_key(id)&&d["exclusions"].get(id).is_none()).collect();
        let missing:Vec<_>=self.unit_order().into_iter().filter(|id|d["units"][id]["revisions"].as_array().unwrap().is_empty()).collect();
        let mut pages=Vec::new();let mut issues=Vec::new();
        for sid in d["source_order"].as_array().unwrap(){
            let sid=sid.as_str().unwrap();
            for k in d["sources"][sid]["page_keys"].as_array().unwrap(){
                let k=k.as_str().unwrap();let p=&d["pages"][k];
                if p["order_review"].is_null()||p["additional_object_review"].is_null(){pages.push(k.to_owned());}
            }
            let mut current:Vec<_>=d["issues"].as_object().unwrap().iter().filter(|(_,r)|r["source"]==sid).collect();
            current.sort_by_key(|(id,_)|id.rsplit(':').next().and_then(|n|n.parse::<usize>().ok()).unwrap_or(usize::MAX));
            for(id,r)in current{if r["decision"].is_null()||r["decision"]["status"]=="blocked"{issues.push(id.clone());}}
        }
        let n=d["units"].as_object().unwrap().len();
        json!({"draft_ready":n>0&&undisposed.is_empty()&&missing.is_empty()&&pages.is_empty()&&issues.is_empty(),
            "source_entry_count":d["blocks"].as_object().unwrap().len(),"unit_count":n,"written_unit_count":n-missing.len(),
            "excluded_entry_count":d["exclusions"].as_object().unwrap().len(),"unassigned_entries":undisposed,"unwritten_units":missing,
            "unreviewed_pdf_pages":pages,"unresolved_source_issues":issues,"source_fidelity":"not_assessed","boundary":BOUNDARY})
    }
    fn context(&self,entries:&[String])->Value{
        let d=&self.data;let mut sources=Map::new();let mut pages=Vec::new();let mut seen=HashSet::new();
        for id in entries{
            let block=&d["blocks"][id];let sid=block["source"].as_str().unwrap();
            let source=&d["sources"][sid];
            sources.entry(sid.to_owned()).or_insert_with(||{
                Value::Object(["path","sha256","format","role","native_media"].into_iter().map(|k|(k.to_owned(),source[k].clone())).collect())
            });
            if let Some(n)=block["page"].as_u64(){
                let k=page_key(sid,n);
                if seen.insert(k.clone()){
                    let page=&d["pages"][&k];
                    let mut row=Value::Object(["source","page","render","text"].into_iter().map(|k|(k.to_owned(),page[k].clone())).collect());
                    if let Some(v)=page.get("pdf_evidence"){row["pdf_evidence"]=v.clone();}
                    pages.push(row);
                }
            }
        }json!({"source_files":sources,"source_pages":pages})
    }
    fn next_task(&self)->Value{
        let d=&self.data;let assigned=self.assigned();
        for sid in d["source_order"].as_array().unwrap(){
            let sid=sid.as_str().unwrap();let source=&d["sources"][sid];
            let mut events:Vec<(bool,String)>=Vec::new();
            if source["format"]=="docx"{events.extend(source["block_ids"].as_array().unwrap().iter().map(|v|(false,v.as_str().unwrap().to_owned())));}
            else{for k in source["page_keys"].as_array().unwrap(){
                let k=k.as_str().unwrap();events.push((true,k.to_owned()));let p=&d["pages"][k];
                let v=if p["order_review"].is_null(){&p["block_ids"]}else{&p["order_review"]["block_ids"]};
                events.extend(v.as_array().unwrap().iter().map(|v|(false,v.as_str().unwrap().to_owned())));
            }}
            for(page,id)in events{
                if page{
                    let p=&d["pages"][&id];if !p["order_review"].is_null()&&!p["additional_object_review"].is_null(){continue;}
                    let entries:Vec<_>=p["block_ids"].as_array().unwrap().iter().map(|id|d["blocks"][id.as_str().unwrap()].clone()).collect();
                    return json!({"stage":"inventory_page","page":p,"source_file":{"path":source["path"],"sha256":source["sha256"]},"entries":entries,"boundary":BOUNDARY});
                }
                if d["exclusions"].get(&id).is_some(){continue;}
                let mut task=if let Some(unit)=assigned.get(&id){
                    let u=&d["units"][unit];if !u["revisions"].as_array().unwrap().is_empty(){continue;}
                    let ids:Vec<_>=u["block_ids"].as_array().unwrap().iter().map(|v|v.as_str().unwrap().to_owned()).collect();
                    let mut v=self.context(&ids);v["stage"]=json!("write_unit");v["unit"]=u.clone();
                    v["entries"]=json!(ids.iter().map(|id|d["blocks"][id].clone()).collect::<Vec<_>>());v
                }else{
                    let mut v=self.context(std::slice::from_ref(&id));v["stage"]=json!("group_or_exclude");v["entry"]=d["blocks"][&id].clone();v
                };
                task["boundary"]=json!(BOUNDARY);return task;
            }
        }
        let mut status=self.status();
        if let Some(id)=status["unresolved_source_issues"].as_array().unwrap().first().and_then(Value::as_str){
            return json!({"stage":"source_issue","issue_id":id,"issue":d["issues"][id],"boundary":BOUNDARY});
        }
        status["stage"]=json!(if status["draft_ready"]==true{"assemble"}else{"no_draft_units"});status
    }
    fn commit(&mut self,action:&str,target:&str)->Result<Value>{
        self.validate()?;
        self.data["history"].as_array_mut().unwrap().push(json!({"action":action,"target":target}));
        let bytes=serialize(&self.data)?;
        check_write::replace_bytes(&self.path,&self.hash,&bytes)?;
        self.hash=files::digest(&bytes);Ok(Value::Null)
    }
    fn group(&mut self,id:&str,block_ids:&Value,kind:&str,reason:&str)->Result<Value>{
        ensure!(!self.data["units"].as_object().unwrap().contains_key(id),"Keep existing units; use a new unit ID");
        let list=ids(block_ids,"grouped source entries")?;
        ensure!(!list.is_empty(),"Group one or more actual source entries");
        let assigned=self.assigned();let mut source=None;
        for bid in list{
            let block=self.data["blocks"].get(bid).context("Group one or more actual source entries")?;
            ensure!(!assigned.contains_key(bid)&&self.data["exclusions"].get(bid).is_none(),"A source entry already has a disposition");
            let s=block["source"].as_str().unwrap();
            if let Some(previous)=source{ensure!(previous==s,"Do not group different source documents");}else{source=Some(s);}
        }
        let source=source.unwrap().to_owned();
        self.data["units"][id]=json!({"id":id,"source":source,"block_ids":block_ids,"kind":kind,"grouping_reason":reason,"revisions":[]});
        self.commit("group",id)
    }
    fn exclude(&mut self,id:&str,reason:&str)->Result<Value>{
        ensure!(self.data["blocks"].get(id).is_some(),"Choose an actual source entry");
        ensure!(self.data["exclusions"].get(id).is_none()&&!self.assigned().contains_key(id),"A source entry already has a disposition");
        self.data["exclusions"][id]=json!({"reason":reason});self.commit("exclude",id)
    }
    fn write_unit(&mut self,id:&str,markdown:&str,assets:&Value)->Result<Value>{
        let unit=self.data["units"].get(id).context("Group the actual source entries before writing")?;
        ensure!(!markdown.trim().is_empty(),"draft fragment must be nonempty text");
        let mut bound=Vec::new();for asset in assets.as_array().context("assets must be an array")?{bound.push(file(&path(asset)?)?);}
        let token=files::digest(id.as_bytes());let version=unit["revisions"].as_array().unwrap().len()+1;
        let parts=path(&self.data["parts_root"])?;
        if parts.exists(){ensure!(parts.is_dir()&&!parts.is_symlink(),"Refusing a linked parts directory");}
        let p=parts.join(format!("{}-v{version:04}.md",&token[..24]));
        new_file(&p,markdown.as_bytes())?;
        let mut revision=file(&p)?;revision["assets"]=json!(bound);
        self.data["units"][id]["revisions"].as_array_mut().unwrap().push(revision.clone());
        self.commit("write_unit",id)?;Ok(revision)
    }
    fn issue(&mut self,id:&str,status:&str,reason:&str,evidence:&Value)->Result<Value>{
        ensure!(self.data["issues"].get(id).is_some(),"Choose an actual source issue");
        ensure!(matches!(status,"resolved"|"excluded"|"blocked"),"Issue status must be resolved, excluded or blocked");
        let mut bindings=Vec::new();
        for p in evidence.as_array().context("evidence_paths must be an array")?{bindings.push(file(&path(p)?)?);}
        ensure!(!bindings.is_empty(),"A source issue decision needs actual evidence files");
        self.data["issues"][id]["decision"]=json!({"status":status,"reason":reason,"evidence":bindings});
        self.commit("issue_decision",id)
    }
    #[allow(clippy::too_many_arguments)]
    fn add_page_object(&mut self,id:&str,sid:&str,n:u64,kind:&str,reason:&str,bbox:&Value,content:&str)->Result<Value>{
        ensure!(self.data["blocks"].get(id).is_none(),"Keep source entry IDs unique");
        let key=self.page(sid,n)?;
        if !bbox.is_null(){
            let b=valid_box(bbox).context("bbox needs four finite numeric coordinates and must be nonempty")?;
            let page=&self.data["pages"][&key];let source=&self.data["sources"][sid];
            let bound=binding(source,self.path.parent().unwrap())?;
            let bounds=if let Some(boxes)=page["image_index"].get("page_bounds"){
                valid_box(boxes).context("Frozen PDF page bounds must be finite and nonempty")?
            }else{pdf_bounds(Path::new(bound["path"].as_str().unwrap()),n)?};
            ensure!(bounds[0]<=b[0]&&bounds[1]<=b[1]&&b[2]<=bounds[2]&&b[3]<=bounds[3],"bbox must lie wholly inside the actual PDF page");
        }
        self.data["blocks"][id]=json!({"id":id,"source":sid,"page":n,"kind":kind,"text":content,"bbox":bbox,"registration_reason":reason,"added_from_page":true});
        let page=&mut self.data["pages"][&key];page["block_ids"].as_array_mut().unwrap().push(json!(id));
        page["order_review"]=Value::Null;page["additional_object_review"]=Value::Null;
        self.data["sources"][sid]["block_ids"].as_array_mut().unwrap().push(json!(id));
        self.commit("add_page_object",id)
    }
    fn assemble(&self,p:&Path,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
        self.validate()?;let status=self.status();ensure!(status["draft_ready"]==true,"Continue drafting before assembly: {status}");
        let p=output(p)?;let mapping=p.with_file_name(format!("{}.draft-map.json",p.file_name().unwrap().to_string_lossy()));
        let d=&self.data;let base=self.path.parent().unwrap();
        let mut protected=HashSet::from([self.path.clone(),path(&d["inventory_snapshot"]["path"])?]);
        binding(&d["inventory_snapshot"],base)?;
        for source in obj(&d["sources"],"sources")?.values(){let b=binding(source,base)?;protected.insert(path(&b["path"])?);}
        for issue in obj(&d["issues"],"issues")?.values(){
            for evidence in rows(&issue["decision"]["evidence"],"native issue evidence")?{
                let b=binding(evidence,base)?;protected.insert(path(&b["path"])?);
            }
        }
        let order=self.ordered_blocks();let ranks:HashMap<_,_>=order.iter().enumerate().map(|(i,id)|(id.as_str(),i)).collect();
        let mut units:Vec<_>=obj(&d["units"],"units")?.values().collect();
        units.sort_by_key(|u|u["block_ids"].as_array().unwrap().iter().map(|id|ranks[id.as_str().unwrap()]).min().unwrap());
        let mut pieces=Vec::new();let mut maps=Vec::new();let mut line=1usize;
        for(i,unit)in units.iter().enumerate(){
            let revision=unit["revisions"].as_array().unwrap().last().context("Missing draft revision")?;
            let b=binding(revision,base)?;let path=path(&b["path"])?;protected.insert(path.clone());
            for asset in rows(&revision["assets"],"fragment assets")?{let b=binding(asset,base)?;protected.insert(crate::path(&b["path"])?);}
            let raw=fs::read(&path)?;ensure!(files::digest(&raw)==b["sha256"].as_str().unwrap(),"Fragment changed during assembly");
            let value=std::str::from_utf8(&raw)?.trim_end_matches(['\r','\n']);
            ensure!(!value.trim().is_empty(),"Empty fragment: {}",unit["id"]);
            let lines=value.bytes().filter(|b|*b==b'\n').count()+1;
            maps.push(json!({"unit_id":unit["id"],"source":unit["source"],"block_ids":unit["block_ids"],"candidate_lines":[line,line+lines-1],"fragment":revision,"source_fidelity":"not_assessed"}));
            pieces.push(value.to_owned());line+=lines+1;progress(i+1,units.len(),"组装已保存片段");
        }
        ensure!(!protected.contains(&p)&&!protected.contains(&mapping),"An assembly must not overwrite a protected file");
        ensure!(!p.exists()&&!mapping.exists()&&!p.is_symlink()&&!mapping.is_symlink(),"Assemble a new version; keep previous candidates");
        let content=format!("{}\n",pieces.join("\n\n"));
        let queue=file(&self.path)?;ensure!(queue["sha256"]==self.hash,"The queue changed in another script; reload before writing");
        let report=json!({"candidate":{"path":path_text(&p)?,"sha256":files::digest(content.as_bytes())},"inventory_snapshot":d["inventory_snapshot"],
            "queue":queue,"units":maps,"excluded_entries":d["exclusions"],"draft_ready":true,"source_fidelity":"not_assessed","boundary":BOUNDARY});
        new_file(&p,content.as_bytes())?;new_file(&mapping,&serialize(&report)?)?;Ok(report)
    }
}
fn valid_box(v:&Value)->Option<[f64;4]>{
    let a=v.as_array().filter(|a|a.len()==4)?;
    let b=[a[0].as_f64()?,a[1].as_f64()?,a[2].as_f64()?,a[3].as_f64()?];
    (b.iter().all(|x|x.is_finite())&&b[0]<b[2]&&b[1]<b[3]).then_some(b)
}
// Only the unrotated page extent is needed by the queue's explicit object
// registration. Full PDF extraction/rendering remains a separate gate.
fn pdf_bounds(p:&Path,page:u64)->Result<[f64;4]>{
    use lopdf::{Document,Object,ObjectId};
    ensure!(fs::metadata(p)?.len()<=512*1024*1024,"PDF exceeds geometry read budget");
    let doc=Document::load(p)?;ensure!(!doc.is_encrypted(),"Encrypted PDF geometry is not supported");
    let n=u32::try_from(page)?;let pages=doc.get_pages();let id=*pages.get(&n).context("Choose an actual PDF source page")?;
    fn deref<'a>(doc:&'a Document,mut obj:&'a Object)->Result<&'a Object>{
        let mut seen=HashSet::new();
        for _ in 0..128{if let Object::Reference(id)=obj{ensure!(seen.insert(*id),"PDF object reference cycle");obj=doc.get_object(*id)?;}else{return Ok(obj);}}
        Err(anyhow!("PDF reference chain exceeds limit"))
    }
    fn number(v:&Object)->Option<f64>{match v{Object::Integer(n)=>Some(*n as f64),Object::Real(n)=>Some(*n as f64),_=>None}}
    fn inherited<'a>(doc:&'a Document,mut id:ObjectId,key:&[u8])->Result<Option<&'a Object>>{
        let mut seen=HashSet::new();
        for _ in 0..256{
            ensure!(seen.insert(id),"PDF page tree cycle");
            let dict=doc.get_object(id)?.as_dict()?;
            if let Ok(v)=dict.get(key){return Ok(Some(deref(doc,v)?));}
            match dict.get(b"Parent"){Ok(Object::Reference(parent))=>id = *parent,Ok(_)=>return Err(anyhow!("Invalid PDF page parent")),Err(_)=>return Ok(None)}
        }Err(anyhow!("PDF page tree exceeds depth limit"))
    }
    let read_box=|key:&[u8]|->Result<Option<[f64;4]>>{
        let Some(v)=inherited(&doc,id,key)?else{return Ok(None);};
        let a=v.as_array()?;ensure!(a.len()==4,"Invalid PDF page rectangle");
        let mut b=[0.0;4];for i in 0..4{b[i]=number(deref(&doc,&a[i])?).context("Invalid PDF page rectangle coordinate")?;}
        ensure!(b.iter().all(|n|n.is_finite())&&b[0]<b[2]&&b[1]<b[3],"Invalid PDF page rectangle");Ok(Some(b))
    };
    let media=read_box(b"MediaBox")?.context("Missing PDF MediaBox")?;
    let crop=read_box(b"CropBox")?.unwrap_or(media);
    let width=media[2].min(crop[2])-media[0].max(crop[0]);
    let height=media[3].min(crop[3])-media[1].max(crop[1]);
    let dict=doc.get_object(id)?.as_dict()?;
    let unit=match dict.get(b"UserUnit"){Ok(v)=>number(deref(&doc,v)?).context("Invalid PDF UserUnit")?,Err(_)=>1.0};
    ensure!(width>0.0&&height>0.0&&unit.is_finite()&&unit>0.0&& (width*unit).is_finite()&&(height*unit).is_finite(),"Invalid PDF page extent");
    Ok([0.0,0.0,width*unit,height*unit])
}
/// Read-only snapshot for workflow routing. No lock files or evidence are
/// written; the caller compares the original hash before using the result.
pub fn resume_snapshot(p:&Path)->Result<Value>{
    let q=Queue::open(p)?;
    let status=q.status();
    if status["draft_ready"]==true{
        for page in q.data["pages"].as_object().context("queue pages")?.values(){crate::pdf_inventory::verify_page_files(page)?;}
        for unit in q.data["units"].as_object().context("queue units")?.values(){
            let revision=unit["revisions"].as_array().and_then(|a|a.last()).context("Missing saved revision")?;
            binding(revision,q.path.parent().unwrap()).context("The saved draft fragment or resource changed")?;
            for asset in rows(&revision["assets"],"fragment assets")?{
                binding(asset,q.path.parent().unwrap()).context("The saved draft fragment or resource changed")?;
            }
        }
    }
    let next=q.next_task();
    verify_task_pages(&next)?;
    let b=file(&q.path)?;
    ensure!(b["sha256"]==q.hash,"Draft queue changed during read-only resumption");
    Ok(json!({"data":q.data,"binding":b,"status":status,"next_task":next}))
}
pub fn open(p:&Path)->Result<Value>{
    let q=Queue::open(p)?;Ok(json!({"data":q.data,"sha256":q.hash,"path":path_text(&q.path)?}))
}
fn verify_task_pages(task:&Value)->Result<()>{
    if let Some(page)=task.get("page"){crate::pdf_inventory::verify_page_files(page)?;}
    if let Some(pages)=task["source_pages"].as_array(){for page in pages{crate::pdf_inventory::verify_page_files(page)?;}}
    Ok(())
}
pub fn operate(p:&Path,action:&str,params:&Value,expected:&Value,progress:impl FnMut(usize,usize,&str))->Result<Value>{
    check_write::no_link(p)?;let p=files::canonical(p)?;
    let _lock=check_write::lock_target(&p)?;
    let mut q=Queue::open(&p)?;
    if !expected.is_null(){ensure!(expected==q.hash.as_str(),"The queue changed in another script; reload before writing");}
    let s=|k:&str|text(&params[k],k);
    let n=||params["page"].as_u64().filter(|n|*n>0).context("page must be a positive integer");
    match action{
        "status"=>Ok(q.status()),"next_task"=>{let task=q.next_task();verify_task_pages(&task)?;Ok(task)},
        "review_pdf_page_order"=>{
            let sid=s("source_id")?;let page=n()?;let reason=s("reason")?;let key=q.page(sid,page)?;
            crate::pdf_inventory::verify_page_files(&q.data["pages"][&key])?;
            let current=array_set(&q.data["pages"][&key]["block_ids"],"actual page entries")?;
            ensure!(array_set(&params["ordered_block_ids"],"confirmed page order")?==current,"Page order must include every text, image-call and added entry exactly once");
            q.data["pages"][&key]["order_review"]=json!({"block_ids":params["ordered_block_ids"],"reason":reason});q.commit("review_page_order",&key)
        }
        "review_additional_objects"=>{
            let key=q.page(s("source_id")?,n()?)?;crate::pdf_inventory::verify_page_files(&q.data["pages"][&key])?;q.data["pages"][&key]["additional_object_review"]=json!({"reason":s("reason")?});q.commit("review_additional_objects",&key)
        }
        "group"=>q.group(s("unit_id")?,&params["block_ids"],s("kind")?,s("reason")?),
        "exclude"=>q.exclude(s("block_id")?,s("reason")?),
        "write_unit"=>q.write_unit(s("unit_id")?,s("markdown")?,&params["assets"]),
        "record_issue_decision"=>q.issue(s("issue_id")?,s("status")?,s("reason")?,&params["evidence_paths"]),
        "add_page_object"=>q.add_page_object(s("block_id")?,s("source_id")?,n()?,s("kind")?,s("reason")?,&params["bbox"],params["text"].as_str().context("Visible transcription must be text")?),
        "assemble"=>q.assemble(&path(&params["output_path"])?,progress),
        _=>Err(anyhow!("unknown draft queue operation {action}"))
    }
}
