//! Bounded task selection from a replayed native translation writer. All
//! decisions remain explicitly supplied; the validator is the only gate.
use crate::{translation_writer::{Writer,range},translation_validate as tv,files};
use anyhow::{ensure,Context,Result};
use serde_json::{json,Value,Map};
use std::collections::{BTreeMap,HashSet};
const BOUNDARY:&str="Record readiness only. Every existing check, category and occurrence group remains required. Source meaning, applicability and the truth of review actions are not assessed. No atomic acceptance ID is awarded a pass.";
const GUIDANCE:&[(&str,&str)]=&[("alignment","定位本来源单元的完整对应译文。"),("ambiguity","判定本单元的歧义是否已有明确处理。"),("negation","核对每处否定的适用范围。"),("strength","核对每条断言的强度。"),("number_changes","核对每次数值表达的变化。"),("terminology","核对每个术语的语境含义。"),("units","核对每个量的单位，包括用文字表达的时间尺度。"),("symbols","核对每个符号的含义。"),("formulas","核对每条公式的结构。"),("citation_labels","核对每次引文调用的标号。"),("completeness","核对每条实质命题，包括具体动作本身，是否完整保留。"),("directions","核对每处方位关系的方向。"),("citation_claims","核对每次引文实际支持的论断。"),("table_text","核对每个表格单元格的文字含义。"),("conditions","核对每处条件的作用范围。"),("antecedents","核对每处指代的先行对象。"),("modifier_scope","核对每个修饰语的修饰对象。")];
fn rows(v:&Value)->&[Value]{v.as_array().map(Vec::as_slice).unwrap_or(&[])}
fn fields(r:&tv::Report,limit:Option<usize>)->Value{
    let n=limit.unwrap_or(usize::MAX);
    let mut v=json!({"findings":r.findings.iter().take(n).collect::<Vec<_>>(),"pending_ids":r.pending_ids.iter().take(n).collect::<Vec<_>>(),
        "failed_ids":r.failed_ids.iter().take(n).collect::<Vec<_>>(),"unresolved_ids":r.unresolved_ids.iter().take(n).collect::<Vec<_>>(),"status_counts":r.status_counts});
    if limit.is_some(){
        let mut codes=BTreeMap::new();for f in &r.findings{*codes.entry(f.code.clone()).or_insert(0usize)+=1;}
        v["diagnostic_totals"]=json!({"findings":r.findings.len(),"pending_ids":r.pending_ids.len(),"failed_ids":r.failed_ids.len(),"unresolved_ids":r.unresolved_ids.len(),"codes":codes});
        v["display_limit"]=json!(n);
    }v
}
pub fn unit_report(writer:&mut Writer,id:&str,require:bool)->Result<Value>{
    writer.verify()?;let r=writer.unit_report(id)?;
    let mut v=fields(&r,None);v.as_object_mut().unwrap().extend(json!({"unit_id":id,"unit_records_ready":r.part_ready(),"source_fidelity":"not_assessed","execution_truth":"not_assessed","boundary":BOUNDARY}).as_object().unwrap().clone());
    ensure!(!require||r.part_ready(),"Finish records for {id}: {}",v["findings"]);
    Ok(v)
}
fn source_context(writer:&Writer,id:&str)->Result<Value>{
    let unit=rows(&writer.inventory["units"]).iter().find(|v|v["id"]==id).context("unknown source unit")?;
    let sid=unit["source"].as_str().context("source ID")?;
    let source=rows(&writer.inventory["sources"]).iter().find(|v|v["id"]==sid).context("unknown source")?;
    let locs=crate::translation_prepare::expand(unit,source)?;
    let pages:HashSet<_>=rows(&locs).iter().filter_map(|v|v["page"].as_u64()).collect();
    let records=rows(&source["pages"]).iter().filter(|v|v["page"].as_u64().is_some_and(|n|pages.contains(&n))).map(|v|
        Value::Object(["page","render","text"].iter().filter_map(|k|v.get(*k).map(|x|((*k).into(),x.clone()))).collect())).collect::<Vec<_>>();
    Ok(json!({"unit_id":id,"kind":unit["kind"],"source_id":sid,"source_file":writer.source_bindings[sid],"source_locations":locs,"source_page_records":records}))
}
fn candidate_context(writer:&Writer,id:&str)->Value{
    let text=&writer.candidate_text;let mut result=Vec::new();
    for(i,loc)in rows(&writer.data["units"][id]["candidate_locations"]).iter().enumerate(){
        result.push(match range(text,loc){
            Ok((a,b))=>json!({"locator":loc,"char_range":[text[..a].chars().count(),text[..b].chars().count()],"text":&text[a..b],"location_index":i}),
            Err(e)=>json!({"locator":loc,"location_error":e.to_string()}),
        });
    }json!(result)
}
fn term_summary(v:&Value)->Value{
    let mut out=Map::new();
    for k in ["id","original","category","decision","translation","decision_reason"]{out.insert(k.into(),v[k].clone());}
    let modes=v["searches"].as_object().map(|m|m.iter().map(|(mode,s)|{
        let queries=rows(&s["queries"]).iter().map(|q|json!({"query":q["query"],"entry_ids":rows(&q["hits"]).iter().map(|h|h["entry_id"].clone()).collect::<Vec<_>>(),"interpretation":q["interpretation"]})).collect::<Vec<_>>();
        (mode.clone(),json!({"status":s["status"],"reason":s["reason"],"queries":queries}))
    }).collect()).unwrap_or_default();
    out.insert("searches".into(),Value::Object(modes));Value::Object(out)
}
fn common()->Value{json!({"source_fidelity":"not_assessed","execution_truth":"not_assessed","boundary":BOUNDARY})}
pub fn next(writer:&mut Writer,scope_limit:usize)->Result<Value>{
    ensure!(scope_limit>0,"scope_limit must be a positive integer");writer.verify()?;
    let scope=writer.scope_report()?;
    let mut out=common();
    if !scope.part_ready(){
        let missing:HashSet<_>=scope.findings.iter().filter(|f|f.code=="TRN_SCOPE_UNACCOUNTED").map(|f|f.object_id.as_str()).collect();
        let unaccounted:Vec<_>=rows(&writer.inventory["units"]).iter().filter_map(|u|u["id"].as_str().filter(|id|missing.contains(*id))).collect();
        let context=unaccounted.iter().take(scope_limit).map(|id|source_context(writer,id)).collect::<Result<Vec<_>>>()?;
        out.as_object_mut().unwrap().extend(fields(&scope,Some(12)).as_object().unwrap().clone());
        out.as_object_mut().unwrap().extend(json!({"stage":"scope","selected_units":writer.selected,"unaccounted_count":unaccounted.len(),"unaccounted_units":context,
            "next_action":"Read each unit's source and task role. If it needs translation, prepare the correct selected-unit scope before reviewing units. Otherwise explicitly exclude that unit with its actual condition and reason. Do not infer exclusion from kind or keywords. Finish the explicit scope decision as well."}).as_object().unwrap().clone());
        return Ok(out);
    }
    let mut completed=Vec::new();
    for id in &writer.selected{
        let report=writer.unit_report(id)?;
        if report.part_ready(){completed.push(id.clone());continue;}
        let row=&writer.data["units"][id];
        let checks=GUIDANCE.iter().map(|(name,question)|json!({"check":name,"question":question,"status":row["checks"][*name]["status"],"reason":row["checks"][*name]["reason"]})).collect::<Vec<_>>();
        let scans=tv::CATEGORIES.iter().map(|name|json!({"category":name,"status":row["term_scan"][*name]["status"],"candidate_ids":row["term_scan"][*name]["candidate_ids"],"reason":row["term_scan"][*name]["reason"]})).collect::<Vec<_>>();
        let occurrences=tv::KINDS.iter().map(|name|json!({"group":name,"recorded_count":row["occurrences"][*name].as_array().map(Vec::len),"comparison_fields":tv::fields(name)})).collect::<Vec<_>>();
        out.as_object_mut().unwrap().extend(source_context(writer,id)?.as_object().unwrap().clone());
        out.as_object_mut().unwrap().extend(fields(&report,Some(12)).as_object().unwrap().clone());
        out.as_object_mut().unwrap().extend(json!({"stage":"unit","unit_id":id,"completed_preceding_units":completed,"selected_unit_count":writer.selected.len(),
            "candidate_file":writer.data["bindings"]["candidate"],"candidate_locations":candidate_context(writer,id),"checks":checks,"term_scan":scans,
            "glossary_file":writer.data["bindings"]["glossary"],"term_candidates":rows(&row["term_candidates"]).iter().map(term_summary).collect::<Vec<_>>(),
            "occurrence_groups":occurrences,"next_action":"Compare the complete source/candidate pair, record each required check and category, each applicable occurrence and actual term query; explain absence separately. Require the unit record gate before continuing."}).as_object().unwrap().clone());
        return Ok(out);
    }
    let directory=tempfile::Builder::new().prefix("translation-record-preview-").tempdir()?;
    let report=writer.save(&directory.path().join("review.json"),Some(&directory.path().join("terminology.json")))?;
    let ready=report["records_ready"]==true;
    let mut codes=BTreeMap::new();
    for f in rows(&report["findings"]){if let Some(c)=f["code"].as_str(){*codes.entry(c.to_owned()).or_insert(0usize)+=1;}}
    for k in ["findings","pending_ids","failed_ids","unresolved_ids"]{out[k]=json!(rows(&report[k]).iter().take(12).collect::<Vec<_>>());}
    out["status_counts"]=report["status_counts"].clone();
    out["diagnostic_totals"]=json!({"findings":rows(&report["findings"]).len(),"pending_ids":rows(&report["pending_ids"]).len(),
        "failed_ids":rows(&report["failed_ids"]).len(),"unresolved_ids":rows(&report["unresolved_ids"]).len(),"codes":codes});
    out["display_limit"]=json!(12);out["stage"]=json!(if ready{"save_final_records"}else{"global_records"});
    out["completed_units"]=json!(completed);out["preview_records_ready"]=json!(ready);
    out["next_action"]=json!(if ready{"Save new final review and terminology files, check the actual report, then bind those exact files. Temporary preview files are not a delivery."}
        else{"Finish each reported global scope, source-issue or terminology record. Preserve each explicit decision and actual evidence. Then request the next review task."});
    Ok(out)
}
