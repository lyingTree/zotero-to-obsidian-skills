//! Pending translation scaffolds and source location expansion. No generated
//! decision, terminology hit, or source location asserts semantic correctness.
use crate::{docx,files,pipeline};
use anyhow::{anyhow,ensure,Context,Result};
use serde_json::{json,Map,Value};
use std::{collections::{HashMap,HashSet},fs,io::Write,path::{Path,PathBuf}};

const CHECKS:&[&str]=&["alignment","ambiguity","negation","strength","number_changes","terminology","units","symbols","formulas","citation_labels","completeness","directions","citation_claims","table_text","conditions","antecedents","modifier_scope"];
const CATEGORIES:&[&str]=&["concept","method","estimator","statistic","metric","procedure","model","parameter","classification","abbreviation"];
const OCCURRENCES:&[&str]=&["claims","directions","negations","conditions","antecedents","modifiers","strength","numbers","units","symbols","formulas","citation_labels","citation_claims","table_cells"];
fn pending()->Value{json!({"status":"pending","reason":null,"evidence":[]})}
fn nonempty(v:&Value)->bool{v.as_str().is_some_and(|s|!s.trim().is_empty())}
pub fn bind_file(p:&Path)->Result<Value>{
    let v=files::snapshot(p)?;Ok(json!({"path":v["path"],"sha256":v["sha256"]}))
}
pub fn bind_evidence(p:&Path,loc:&Value)->Result<Value>{
    ensure!(loc.as_object().is_some_and(|m|!m.is_empty()),"Supply a structured evidence locator");
    let mut record=bind_file(p)?;record["locator"]=loc.clone();Ok(record)
}
pub fn occurrence(id:&str,kind:&str,span:&Value)->Result<Value>{
    ensure!(!id.trim().is_empty(),"Supply a nonempty occurrence ID");
    let fields:&[&str]=match kind{
        "claims"=>&["source_claim","candidate_claim"],"directions"=>&["source_relation","candidate_relation"],
        "negations"=>&["source_scope","candidate_scope"],"conditions"=>&["source_condition","candidate_condition"],
        "antecedents"|"modifiers"=>&["source_candidates","candidate_candidates"],
        "strength"=>&["source_strength","candidate_strength"],"numbers"=>&["source_expression","candidate_expression","difference_type"],
        "units"=>&["source_expression","candidate_expression","source_quantity","candidate_quantity"],
        "symbols"=>&["source_expression","candidate_expression","source_definition","candidate_definition"],
        "formulas"=>&["source_expression","candidate_expression","source_structure","candidate_structure"],
        "citation_labels"=>&["source_label","candidate_label","source_target","candidate_target"],
        "citation_claims"=>&["source_claim","candidate_claim","source_target","candidate_target"],
        "table_cells"=>&["source_cell","candidate_cell"],_=>anyhow::bail!("Unknown occurrence kind: {kind}"),
    };
    ensure!(span.as_object().is_some_and(|m|!m.is_empty()),"Supply the actual source span");
    let mut row=pending();row["id"]=json!(id);row["source_span"]=span.clone();row["candidate_span"]=Value::Null;
    for key in fields{row[*key]=Value::Null;}
    Ok(row)
}
pub fn term(id:&str,original:&str,category:&str,excerpt:&str,locator:&Value)->Result<Value>{
    ensure!([id,original,excerpt].iter().all(|s|!s.trim().is_empty()),"term_id, original and source_excerpt must be nonempty");
    ensure!(CATEGORIES.contains(&category),"Unknown term category: {category}");
    ensure!(locator.as_object().is_some_and(|m|!m.is_empty()),"Supply the actual source occurrence locator");
    let searches:Map<String,Value>=["full_phrase","aliases","family"].iter().map(|mode|{
        let mut r=pending();r["queries"]=json!([]);((*mode).to_owned(),r)
    }).collect();
    Ok(json!({"id":id,"original":original,"category":category,"source_excerpt":excerpt,"source_locator":locator,
        "aliases":[],"translation":null,"searches":searches,"decision":"pending","decision_reason":null,
        "glossary_matches":[],"terminology_pointer":null}))
}
struct Index<'a>{ordered:Vec<&'a str>,rows:HashMap<&'a str,&'a Value>}
fn index<'a>(v:&'a Value,label:&str)->Result<Index<'a>>{
    let list=v.as_array().with_context(||format!("{label} must be a list"))?;
    let mut out=Index{ordered:Vec::with_capacity(list.len()),rows:HashMap::new()};
    for row in list{
        ensure!(row.is_object(),"{label} must contain objects");
        let id=row["id"].as_str().filter(|s|!s.trim().is_empty()).with_context(||format!("{label} requires unique nonempty IDs"))?;
        ensure!(out.rows.insert(id,row).is_none(),"{label} requires unique nonempty IDs");
        out.ordered.push(id);
    }Ok(out)
}
fn box_values(v:&Value)->Option<[f64;4]>{
    let a=v.as_array().filter(|a|a.len()==4)?;
    let b=[a[0].as_f64()?,a[1].as_f64()?,a[2].as_f64()?,a[3].as_f64()?];
    (b.iter().all(|v|v.is_finite())&&b[0]<b[2]&&b[1]<b[3]).then_some(b)
}
fn unique_ids<'a>(v:&'a Value,label:&str)->Result<Vec<&'a str>>{
    let list=v.as_array().filter(|v|!v.is_empty()).with_context(||format!("{label} must be a unique nonempty string list"))?;
    let mut seen=HashSet::new();let mut out=Vec::with_capacity(list.len());
    for v in list{
        let s=v.as_str().filter(|s|!s.trim().is_empty()).with_context(||format!("{label} must be a unique nonempty string list"))?;
        ensure!(seen.insert(s),"{label} must be a unique nonempty string list");out.push(s);
    }Ok(out)
}
fn expand_native(unit:&Value,source:&Value,locs:&[Value])->Result<Value>{
    let id=unit["id"].as_str().unwrap_or("unknown");
    let blocks=index(&source["native_blocks"],&format!("{id}: native source blocks"))?;
    let mut out=Vec::with_capacity(locs.len());
    for loc in locs{
        let errors=docx::evidence_errors(source,loc);
        ensure!(errors.is_empty(),"{id}: {}",errors.join(" "));
        let mut v=loc.clone();let mut resolved=Vec::new();
        for key in loc["blocks"].as_array().unwrap(){
            let key=key.as_str().unwrap();let block=blocks.rows[key];
            resolved.push(json!({"block_id":key,"basis":"native","native_locator":block["locator"],"quote":block["text"]}));
        }
        v["block_locations"]=json!(resolved);
        if !nonempty(&v["quote"]){v["quote"]=json!(docx::source_text(source,loc)?);}
        out.push(v);
    }Ok(json!(out))
}
pub fn expand(unit:&Value,source:&Value)->Result<Value>{
    ensure!(unit.is_object()&&source.is_object(),"Supply inventory unit and source objects");
    let id=unit["id"].as_str().unwrap_or("unknown");
    ensure!(unit["source"].is_string()&&unit["source"]==source["id"],"{id}: unit must belong to the supplied source");
    let locations=unit["evidence"].as_array().filter(|v|!v.is_empty()).with_context(||format!("{id}: source locations are required"))?;
    if source["format"]=="docx"{return expand_native(unit,source,locations);}
    let mut pages:Option<HashMap<u64,&Value>>=None;
    let mut out=Vec::with_capacity(locations.len());
    // Cache indexes per actual source page instead of rescanning for each span.
    let mut page_blocks:HashMap<u64,Index<'_>>=HashMap::new();
    for loc in locations{
        ensure!(loc.as_object().is_some_and(|m|!m.is_empty()),"{id}: source locations must be objects");
        let page=loc["page"].as_u64().filter(|n|*n>0).with_context(||format!("{id}: each source location needs a positive PDF page"))?;
        let mut row=loc.clone();
        if let Some(raw)=loc.get("blocks"){
            if raw.as_array().is_some_and(Vec::is_empty)&&(box_values(&loc["bbox"]).is_some()||nonempty(&loc["quote"])){
                out.push(row);continue;
            }
            let keys=unique_ids(raw,&format!("{id}: block IDs"))?;
            if pages.is_none(){
                let raw_pages=source["pages"].as_array().with_context(||format!("{id}: source.pages is required to resolve block IDs"))?;
                let mut p=HashMap::new();
                for v in raw_pages{
                    let n=v["page"].as_u64().filter(|n|*n>0).with_context(||format!("{id}: source page records require unique positive page numbers"))?;
                    ensure!(p.insert(n,v).is_none(),"{id}: source page records require unique positive page numbers");
                }pages=Some(p);
            }
            let page_record=pages.as_ref().unwrap().get(&page).copied().with_context(||format!("{id}: page {page} has no source block index"))?;
            if let std::collections::hash_map::Entry::Vacant(entry)=page_blocks.entry(page){
                entry.insert(index(&page_record["blocks"],&format!("{id}: page {page} blocks"))?);
            }
            let blocks=&page_blocks[&page].rows;
            let mut resolved=Vec::with_capacity(keys.len());let mut quotes=Vec::new();
            let mut combined=[f64::INFINITY,f64::INFINITY,f64::NEG_INFINITY,f64::NEG_INFINITY];
            for key in keys{
                let block=blocks.get(key).with_context(||format!("{id}: unknown block {key} on page {page}"))?;
                let b=box_values(&block["bbox"]).with_context(||format!("{id}: block {key} has no valid original bbox"))?;
                for i in 0..4{combined[i]=if i<2{combined[i].min(b[i])}else{combined[i].max(b[i])};}
                let mut r=json!({"block_id":key,"page":page,"bbox":block["bbox"]});
                if nonempty(&block["text"]){r["quote"]=block["text"].clone();quotes.push(block["text"].as_str().unwrap());}
                resolved.push(r);
            }
            row["block_locations"]=json!(resolved);
            if box_values(&row["bbox"]).is_none(){row["bbox"]=json!(combined);}
            if !nonempty(&row["quote"])&&!quotes.is_empty(){row["quote"]=json!(quotes.join("\n"));}
        }
        ensure!(box_values(&row["bbox"]).is_some()||nonempty(&row["quote"]),"{id}: each source location needs a valid bbox, nonempty quote, or resolvable blocks");
        out.push(row);
    }Ok(json!(out))
}
fn bound(p:&Path,expected:&Value)->Result<Value>{
    let actual=bind_file(p)?;
    ensure!(actual["sha256"]==expected["sha256"],"source bytes differ from the inventory: {}",p.display());
    Ok(actual)
}
fn verify_record(row:&Value)->Result<()>{
    let path=row["path"].as_str().context("bound path missing")?;
    bound(Path::new(path),row)?;Ok(())
}
fn exclusive_json(output:&Path,value:&Value)->Result<PathBuf>{
    // Same-directory exclusive publication; do not permit a retry to overwrite
    // or silently replace a previous human-reviewed record.
    let parent=output.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or_else(||Path::new("."));
    fs::create_dir_all(parent)?;
    let output=files::canonical(parent)?.join(output.file_name().context("review output file name")?);
    ensure!(!output.exists()&&!output.is_symlink(),"Keep the existing review record: {}",output.display());
    let mut bytes=serde_json::to_vec_pretty(value)?;bytes.push(b'\n');
    let mut tmp=tempfile::Builder::new().prefix(".pending-review-").tempfile_in(output.parent().unwrap())?;
    tmp.write_all(&bytes)?;tmp.as_file().sync_all()?;
    tmp.persist_noclobber(&output).map_err(|e|anyhow!("Cannot exclusively publish review record {}: {}",output.display(),e.error))?;
    ensure!(pipeline::read_json(&output)?==*value,"Review record did not round-trip");
    Ok(output)
}
pub fn prepare(inventory_path:&Path,candidate_path:&Path,output_path:&Path,unit_ids:&Value,
               glossary_path:Option<&Path>,targets:&Value,mut progress:impl FnMut(usize,usize,&str))->Result<PathBuf>{
    ensure!(!output_path.exists()&&!output_path.is_symlink(),"Keep the existing review record: {}",output_path.display());
    let selected=unique_ids(unit_ids,"Selected source unit IDs")?;
    let inventory_path=files::canonical(inventory_path)?;
    // Bind the exact bytes parsed, not a later replacement at the same path.
    let (raw,inventory_binding)=files::read_bound(&inventory_path)?;
    let inventory=crate::parse_json(std::str::from_utf8(&raw)?)?;
    ensure!(inventory.is_object(),"The source inventory must be an object");
    let sources=index(&inventory["sources"],"sources")?;
    let units=index(&inventory["units"],"units")?;
    ensure!(selected.iter().all(|s|units.rows.contains_key(s)),"Unknown source units");
    let targets=if targets.is_null(){Map::new()}else{targets.as_object().context("Candidate locations must be an object")?.clone()};
    let selection:HashSet<&str>=selected.iter().copied().collect();
    ensure!(targets.keys().all(|s|selection.contains(s.as_str())),"Candidate locations may refer only to selected source units");
    let candidate=bind_file(candidate_path)?;
    let glossary=match glossary_path{Some(p)=>bind_file(p)?,None=>Value::Null};
    let mut bindings=Map::new();let mut records=Map::new();
    for (i,id) in selected.iter().enumerate(){
        let unit=units.rows[id];
        let sid=unit["source"].as_str().filter(|s|sources.rows.contains_key(s)).with_context(||format!("{id}: source identity is missing"))?;
        let source=sources.rows[sid];
        let locations=expand(unit,source)?;
        if !bindings.contains_key(sid){
            let p=source["path"].as_str().filter(|s|!s.is_empty()).with_context(||format!("{sid}: inventory source path/hash is missing"))?;
            ensure!(source["sha256"].is_string(),"{sid}: inventory source path/hash is missing");
            let p=PathBuf::from(p);
            let p=if p.is_absolute(){p}else{inventory_path.parent().unwrap().join(p)};
            bindings.insert(sid.to_owned(),bound(&p,source)?);
        }
        let candidate_locations=targets.get(*id).cloned().unwrap_or_else(||json!([]));
        ensure!(candidate_locations.as_array().is_some_and(|a|a.iter().all(|v|v.as_object().is_some_and(|m|!m.is_empty()))),
            "{id}: candidate locations must be a list of locator objects");
        let checks:Map<String,Value>=CHECKS.iter().map(|n|((*n).into(),pending())).collect();
        let occurrences:Map<String,Value>=OCCURRENCES.iter().map(|n|((*n).into(),json!([]))).collect();
        let terms:Map<String,Value>=CATEGORIES.iter().map(|n|{let mut r=pending();r["candidate_ids"]=json!([]);((*n).into(),r)}).collect();
        records.insert((*id).to_owned(),json!({"source_id":sid,"kind":unit["kind"],"source_locations":locations,
            "candidate_locations":candidate_locations,"checks":checks,"occurrences":occurrences,"term_scan":terms,"term_candidates":[]}));
        progress(i+1,selected.len(),id);
    }
    let unselected:Vec<_>=units.ordered.iter().filter(|id|!selection.contains(*id)).collect();
    let payload=json!({"schema_version":1,"evidence_boundary":"Pending scaffold only; neither semantic correctness nor review actions are inferred.",
        "review_status":"pending","bindings":{"source_inventory":inventory_binding,"sources":bindings,"candidate":candidate,"glossary":glossary},
        "scope":{"selected_units":selected,"unselected_inventory_units":unselected,"selection_review":pending(),"excluded_units":[]},
        "units":records,"terminology_record":null,"unresolved_terms":[],"source_issues":[],"source_issues_review":pending(),"events":[]});
    verify_record(&payload["bindings"]["source_inventory"])?;
    verify_record(&payload["bindings"]["candidate"])?;
    if !payload["bindings"]["glossary"].is_null(){verify_record(&payload["bindings"]["glossary"])?;}
    for record in payload["bindings"]["sources"].as_object().unwrap().values(){verify_record(record)?;}
    let result=exclusive_json(output_path,&payload)?;
    Ok(result)
}
