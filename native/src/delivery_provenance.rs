//! Actual source-provenance validation, an independently callable part of
//! delivery validation. PDF text, rendered pixels, and other delivery rules
//! are explicitly not certified by this operation.
use anyhow::{ensure,Context,Result};
use serde_json::{json,Value};
use std::{collections::{BTreeMap,HashMap,HashSet},fs,path::{Path,PathBuf}};
use crate::{check_ledger,docx,files,path,pdf_structure::Pdf};

struct Audit{
    base:PathBuf,
    findings:Vec<Value>,
    checked:BTreeMap<PathBuf,Value>,
    native_count:usize,
    pdf_count:usize,
    pdf_replays:Vec<Value>,
}
impl Audit{
    fn fail(&mut self,code:&str,id:&str,message:impl AsRef<str>){
        self.findings.push(json!({"code":code,"object_id":id,"message":message.as_ref(),"level":"error","line":null}));
    }
    fn record(&mut self,v:&Value,id:&str)->Option<Value>{
        let Some(raw)=v["path"].as_str() else{
            self.fail("FILE_RECORD",id,"Provide path and SHA-256 for this evidence file.");return None;
        };
        let p=self.base.join(raw);
        if !p.is_file(){self.fail("FILE_MISSING",id,format!("Missing evidence file: {}",p.display()));return None;}
        let Some(sha)=v["sha256"].as_str().filter(|s|s.len()==64&&s.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))) else{
            self.fail("FILE_HASH_REQUIRED",id,"A real lowercase SHA-256 is required.");return None;
        };
        let b=match check_ledger::bind_file(&p){
            Ok(b)=>b,Err(e)=>{self.fail("FILE_READ",id,e.to_string());return None;}
        };
        if b["sha256"]!=sha{self.fail("FILE_HASH_MISMATCH",id,format!("Evidence changed after binding: {}",p.display()));return None;}
        let key=match path(&b["path"]){
            Ok(p)=>p,Err(e)=>{self.fail("FILE_READ",id,e.to_string());return None;}
        };
        if let Some(previous)=self.checked.get(&key){
            if previous!=&b{self.fail("FILE_HASH_MISMATCH",id,"Evidence changed during this check.");return None;}
        }
        self.checked.insert(key,b.clone());Some(b)
    }
    fn rows<'a>(&mut self,v:&'a Value,id:&str,name:&str)->&'a [Value]{
        if let Some(rows)=v.as_array().filter(|rows|rows.iter().all(Value::is_object)){rows}
        else{self.fail("ARRAY_REQUIRED",id,format!("{name} must be an array of objects."));&[]}
    }
    fn index<'a>(&mut self,rows:&'a [Value],id:&str)->(Vec<&'a str>,HashMap<&'a str,&'a Value>){
        let mut order=Vec::new();let mut index=HashMap::new();
        for row in rows{
            let Some(key)=row["id"].as_str().filter(|s|!s.trim().is_empty()) else{
                self.fail("ID_REQUIRED",id,"Every entry needs a nonempty id.");continue;
            };
            if index.contains_key(key){self.fail("ID_DUPLICATE",key,"Duplicate id.");continue;}
            order.push(key);index.insert(key,row);
        }
        (order,index)
    }
    fn native(&mut self,id:&str,source:&Value,p:&Path,mut progress:impl FnMut(usize,usize,&str)){
        let actual=match docx::read_docx(p,id,|i,n,label|progress(i,n,label)){
            Ok(v)=>v,Err(e)=>{self.fail("SOURCE_DOCX",id,e.to_string());return;}
        };
        self.native_count+=1;
        for name in ["format","page_count","pages","document_part","relationships","native_blocks","media_calls","native_issues"]{
            if source.get(name)!=actual.get(name){
                self.fail("NATIVE_PROVENANCE",&format!("{id}:{name}"),
                    "Preserve the actual OOXML source inventory; pagination and image numbering are not inferred.");
            }
        }
        let rows=self.rows(&source["native_media"],id,"native_media");
        let (order,registered)=self.index(rows,&format!("{id}:media"));
        let actual_media=actual["native_media"].as_array().expect("native media from parser");
        let expected_order:Vec<_>=actual_media.iter().filter_map(|m|m["id"].as_str()).collect();
        if order!=expected_order{self.fail("NATIVE_MEDIA_SET",id,"Retain every actual embedded media part once in source inventory order.");}
        for media in actual_media{
            let mid=media["id"].as_str().unwrap();
            let empty=Value::Null;
            let entry=registered.get(mid).copied().unwrap_or(&empty);
            if media.as_object().unwrap().iter().any(|(k,v)|entry[k]!=*v){
                self.fail("NATIVE_MEDIA_PROVENANCE",mid,"Native media metadata must match the original DOCX part.");
            }
            if let Some(b)=self.record(&entry["file"],mid){
                if b["sha256"]!=media["sha256"]{self.fail("NATIVE_MEDIA_BYTES",mid,"Preserve the original embedded image bytes.");}
            }
        }
        if let Some(b)=self.record(&source["native_document"],&format!("{id}:native-document")){
            if b["sha256"]!=actual["document_part"]["sha256"]{
                self.fail("NATIVE_DOCUMENT_BYTES",id,"Preserve the original document XML bytes.");
            }
        }
        if let Some(b)=self.record(&source["native_text"],&format!("{id}:native-text")){
            let mut expected=String::new();
            for block in actual["native_blocks"].as_array().unwrap(){
                expected.push_str(&format!("[{} | {}]\n{}\n\n",block["id"].as_str().unwrap(),block["kind"].as_str().unwrap(),block["text"].as_str().unwrap()));
            }
            // The SHA-256 is bound to the actual original XML-derived text.
            if b["sha256"]!=files::digest(expected.as_bytes()){
                self.fail("NATIVE_TEXT_BYTES",id,"The native block text evidence differs from the original OOXML inventory.");
            }
        }
        let no_reviews=json!([]);
        let reviews=self.rows(source.get("native_issue_reviews").unwrap_or(&no_reviews),id,"native_issue_reviews");
        let (_,reviews)=self.index(reviews,&format!("{id}:native-reviews"));
        let issues=actual["native_issues"].as_array().unwrap();
        let ids:HashSet<_>=issues.iter().filter_map(|r|r["id"].as_str()).collect();
        for key in reviews.keys(){
            if !ids.contains(key){self.fail("NATIVE_ISSUE_UNKNOWN",key,"Review an actual native extraction issue.");}
        }
        for issue in issues{
            let key=issue["id"].as_str().unwrap();
            let empty=Value::Null;
            let review=reviews.get(key).copied().unwrap_or(&empty);
            if !matches!(review["status"].as_str(),Some("resolved"|"not_applicable")){
                self.fail("NATIVE_ISSUE_PENDING",key,"Unsupported source content requires explicit inspection before completion.");continue;
            }
            if !review["reason"].as_str().is_some_and(|s|!s.trim().is_empty()){
                self.fail("NATIVE_ISSUE_REASON",key,"Record a source-specific reason for resolving this extraction issue.");
            }
            let evidence=self.rows(&review["evidence"],key,"evidence");
            if evidence.is_empty(){self.fail("NATIVE_ISSUE_EVIDENCE",key,"Bind the actually inspected source or rendered evidence.");}
            for (i,entry) in evidence.iter().enumerate(){
                self.record(&entry["file"],&format!("{key}:evidence-{i}"));
                if !entry["locator"].as_object().is_some_and(|o|!o.is_empty()){
                    self.fail("NATIVE_ISSUE_LOCATOR",key,"Locate the actual evidence used to resolve this issue.");
                }
            }
        }
    }
    fn pdf(&mut self,id:&str,source:&Value,p:&Path){
        let pdf=match Pdf::open(p){
            Ok(v)=>v,Err(e)=>{self.fail("SOURCE_PDF",id,e.to_string());return;}
        };
        self.pdf_count+=1;
        if source["page_count"].as_u64()!=Some(pdf.len() as u64){
            self.fail("SOURCE_PAGE_COUNT",id,format!("Actual PDF has {} pages.",pdf.len()));
        }
        let pages=self.rows(&source["pages"],id,"pages");
        if pages.len()!=pdf.len()||!pages.iter().enumerate().all(|(i,v)|v["page"].as_u64()==Some((i+1) as u64)){
            self.fail("SOURCE_PAGE_SET",id,"Record every actual PDF page once in source order.");
        }
        for row in pages{
            let Some(n)=row["page"].as_u64().filter(|n|*n>=1&&*n<=pdf.len() as u64) else{continue;};
            self.record(&row["text"],&format!("{id}:page-{n}:text"));
            self.record(&row["render"],&format!("{id}:page-{n}:render"));
            if source["inventory_backend"]==crate::pdf_inventory::BACKEND{
                let ev=&row["pdf_evidence"];
                for key in ["glyphs","image_structure"]{self.record(&ev[key],&format!("{id}:page-{n}:{key}"));}
                if let Some(b)=ev.get("ocr").and_then(|v|v.get("record")).filter(|v|!v.is_null()){
                    self.record(b,&format!("{id}:page-{n}:ocr"));
                }
            }
            if let Err(e)=pdf.bounds(n){self.fail("SOURCE_PDF_GEOMETRY",id,e.to_string());}
        }
        if source["inventory_backend"]==crate::pdf_inventory::BACKEND{
            for row in self.rows(&source["image_objects"],id,"image_objects"){
                let label=format!("{id}:image-{}:{}",row["image_xref"],row["generation"]);
                for key in ["encoded_stream","raw_asset"]{
                    if let Some(b)=row.get(key).filter(|v|!v.is_null()){self.record(b,&format!("{label}:{key}"));}
                }
                for key in ["smask_stream","mask_stream"]{
                    if let Some(b)=row.get(key).and_then(|v|v.get("asset")).filter(|v|!v.is_null()){
                        self.record(b,&format!("{label}:{key}"));
                    }
                }
            }
            match crate::pdf_inventory::replay(source,p,|_,_,_|{}){
                Ok(mut report)=>{report["source_id"]=json!(id);self.pdf_replays.push(report);},
                Err(e)=>self.fail("PDF_NATIVE_REPLAY_MISMATCH",id,format!("{e:#}")),
            }
        }
        // A matching hash or same-backend replay does not prove full independent source fidelity.
        self.fail("PDF_FULL_PROVENANCE_UNMIGRATED",id,
            "Actual PDF text/block and full rendered-pixel provenance are not qualified by this native source check.");
    }
}
pub fn validate(inventory_path:&Path,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    let (raw,inventory_binding)=files::read_bound(inventory_path)?;
    let p=path(&inventory_binding["path"])?;
    let inventory=crate::parse_json(std::str::from_utf8(&raw)?)?;
    ensure!(inventory.is_object(),"Expected source inventory object");
    let mut audit=Audit{base:p.parent().context("inventory parent")?.to_owned(),findings:vec![],
        checked:BTreeMap::new(),native_count:0,pdf_count:0,pdf_replays:vec![]};
    if inventory["schema_version"].as_u64()!=Some(1){audit.fail("INVENTORY_VERSION","inventory","Expected schema_version 1.");}
    let rows=audit.rows(&inventory["sources"],"inventory","sources");
    let (order,sources)=audit.index(rows,"sources");
    if sources.is_empty(){audit.fail("SOURCES_EMPTY","inventory","At least one source is required.");}
    for (i,id) in order.iter().enumerate(){
        let source=sources[id];
        if !matches!(source["role"].as_str(),Some("main"|"supplement")){
            audit.fail("SOURCE_ROLE",id,"Choose main or supplement from the source identity.");
        }
        if let Some(b)=audit.record(source,id){
            let p=path(&b["path"])?;
            if source["format"]=="docx"{audit.native(id,source,&p,|i,n,label|progress(i,n,label));}
            else{audit.pdf(id,source,&p);}
        }
        progress(i+1,order.len(),"source provenance");
    }
    for (p,b) in audit.checked.clone(){
        if check_ledger::bind_file(&p).ok().as_ref()!=Some(&b){
            audit.fail("FILE_HASH_MISMATCH","source evidence",format!("Evidence changed during validation: {}",p.display()));
        }
    }
    if check_ledger::bind_file(&p)?!=inventory_binding{
        audit.fail("FILE_HASH_MISMATCH","inventory","Inventory changed during validation.");
    }
    let complete=audit.findings.is_empty()&&audit.native_count>0&&audit.pdf_count==0;
    let checked_files:Vec<_>=audit.checked.values().cloned().collect();
    let mut result=json!({"inventory":inventory_binding,"checked_files":checked_files,"findings":audit.findings,"native_sources_checked":audit.native_count,
        "pdf_structure_sources_checked":audit.pdf_count,"source_checks_complete":complete,"delivery_ready":false,
        "source_fidelity":"not_assessed","independent_review_required":true,
        "boundary":"Source-provenance subset only. Full delivery, inventory accounting, candidate targets, PDF pixels/text and independent review are not certified."});
    if !audit.pdf_replays.is_empty(){result["pdf_native_replays"]=json!(audit.pdf_replays);}
    Ok(result)
}
