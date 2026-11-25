//! Rich Table source checks. Only a complete recognized top-level scan can pass.
use crate::{decision,files,validate,yaml_nodes};
use anyhow::Result;
use serde_json::{json,Map,Value};
use std::path::Path;

const IDS:&[&str]=&["BODY-A83","BODY-A84","BODY-A85","BODY-A86","BODY-A87","BODY-A111"];
fn record(table:&validate::TableRecord,outcomes:&mut [Vec<Option<bool>>])->Value{
    let parsed=if table.closing_line.is_some(){yaml_nodes::load(&validate::rich_payload(&table.content)).ok().map(|doc|doc.value)}else{None};
    outcomes[0].push(Some(parsed.is_some()));
    let mut details=vec![];
    let Some(value)=parsed else{
        for row in outcomes.iter_mut().skip(1){row.push(None);}
        return json!({"line":table.line,"sheets":details,"error":"invalid or unclosed Rich Table YAML"});
    };
    let sheets=match value["version"].as_u64(){
        Some(2)=>Some(vec![&value]),
        Some(3)=>value["sheets"].as_array().map(|a|a.iter().collect()),
        _=>None
    };
    let Some(sheets)=sheets.filter(|s|!s.is_empty()&&s.iter().all(|v|v.is_object())) else{
        for row in outcomes.iter_mut().skip(1){row.push(None);}
        return json!({"line":table.line,"sheets":details,"error":"unsupported or incomplete sheet structure"});
    };
    if value["version"]==3{
        if let Some(footer)=value.get("footer"){outcomes[5].push(Some(footer.is_string()));}
    }
    for (i,sheet) in sheets.iter().enumerate(){
        let columns=sheet["columns"].as_array();
        let rows=sheet["rows"].as_array();
        outcomes[1].push(Some(sheet["locked"]==true));
        outcomes[2].push(Some(sheet["theme"]=="grid"));
        outcomes[3].push(Some(columns.is_some_and(|v|!v.is_empty())));
        outcomes[4].push(Some(rows.is_some_and(|v|!v.is_empty())));
        if let Some(footer)=sheet.get("footer"){outcomes[5].push(Some(footer.is_string()));}
        details.push(json!({"index":i+1,"columns":columns.map(Vec::len),"rows":rows.map(Vec::len)}));
    }
    json!({"line":table.line,"sheets":details})
}
pub fn audit(candidate:&Path)->Result<Value>{
    let (raw,binding)=files::read_bound(candidate)?;
    let text=std::str::from_utf8(&raw)?;
    let inspection=validate::inspect(text);
    let uncertain=inspection.findings.iter().any(|f|matches!(f.code,"FENCE_UNCLOSED"|"MATH_UNCLOSED"|"COMMENT_UNCLOSED"));
    let mut outcomes=vec![vec![];IDS.len()];
    let tables:Vec<_>=inspection.table_records.iter().map(|recorded|record(recorded,&mut outcomes)).collect();
    let mut checks=Map::new();
    for (id,values) in IDS.iter().zip(outcomes){
        let status=if values.contains(&Some(false)){Some(false)}
            else if uncertain||values.is_empty()||values.contains(&None){None}
            else{Some(true)};
        checks.insert((*id).to_owned(),decision(id,status,"evaluate recognized Rich Table sheets; incomplete scan stays unresolved"));
    }
    Ok(json!({"stage":"tables","inputs":[binding],"checks":checks,"tables":tables,
        "source_fidelity":"not_assessed","scan_uncertain":uncertain}))
}
