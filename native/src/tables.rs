//! Rich Table source checks. Only a complete recognized top-level scan can pass.
use crate::{decision,files,properties};
use anyhow::Result;
use serde_json::{json,Map,Value};
use std::path::Path;

const IDS:&[&str]=&["BODY-A83","BODY-A84","BODY-A85","BODY-A86","BODY-A87","BODY-A111"];
struct Fence{marker:char,width:usize,line:usize,table:bool,payload:Vec<String>}
fn fence(line:&str)->Option<(char,usize,&str)>{
    let trimmed=line.trim_start_matches(' ');
    if line.len()-trimmed.len()>3{return None;}
    let marker=trimmed.chars().next()?;
    if marker!='~'&&marker!='\x60'{return None;}
    let length=trimmed.chars().take_while(|c|*c==marker).count();
    if length<3{return None;}
    let tail=&trimmed[length..];
    if marker=='\x60'&&tail.contains('\x60'){return None;}
    Some((marker,length,tail.trim()))
}
fn unwrap_payload(lines:&[String])->String{
    let mut value=lines.to_vec();
    let nonempty:Vec<_>=value.iter().enumerate().filter(|(_,s)|!s.trim().is_empty()).map(|(i,_)|i).collect();
    if nonempty.len()>=2{
        let first=nonempty[0];let last=*nonempty.last().unwrap();
        let marker=|s:&str|s=="---"||s.strip_prefix("---").is_some_and(|tail|tail.starts_with(' ')&&tail.trim().starts_with('#'));
        if marker(&value[first])&&marker(&value[last]){value[last].clear();}
    }
    value.join("\n")
}
fn record(table:Fence,outcomes:&mut [Vec<Option<bool>>],closed:bool)->Value{
    let parsed=if closed{properties::parse_yaml(unwrap_payload(&table.payload).as_bytes()).ok()}else{None};
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
    let parts=properties::split(&raw)?;
    let text=std::str::from_utf8(parts.body)?;
    let head=String::from_utf8_lossy(parts.prefix).lines().count();
    let offset=if parts.payload.is_none(){0}else{head};
    let mut active:Option<Fence>=None;
    let mut math=false;
    let mut comment:Option<&str>=None;
    let mut uncertain=false;
    let mut outcomes=vec![vec![];IDS.len()];
    let mut tables=vec![];
    for (i,line) in text.lines().enumerate(){
        let line=line.trim_end_matches('\r');
        let number=offset+i+1;
        if let Some(current)=active.as_mut(){
            let closes=fence(line).is_some_and(|(marker,width,tail)|marker==current.marker&&width>=current.width&&tail.is_empty());
            if closes{
                let current=active.take().unwrap();
                if current.table{tables.push(record(current,&mut outcomes,true));}
            }else if current.table{current.payload.push(line.to_owned());}
            continue;
        }
        if math{if line.trim()=="$$"{math=false;}continue;}
        if let Some(end)=comment{
            if line.contains(end){comment=None;if line.trim_end().ends_with(end){continue;}uncertain=true;}
            continue;
        }
        let clean=line.trim_start();
        if clean.starts_with("<!--"){
            if !clean.contains("-->"){comment=Some("-->");}
            else if !clean.trim_end().ends_with("-->"){uncertain=true;}
            continue;
        }
        if clean.starts_with("%%"){
            if !clean[2..].contains("%%"){comment=Some("%%");}
            else if !clean.trim_end().ends_with("%%"){uncertain=true;}
            continue;
        }
        if clean=="$$"{math=true;continue;}
        // Inline/mixed multiline markup can change fence visibility in
        // Obsidian. Record uncertainty rather than certify such a scan.
        if line.contains("<!--")||line.contains("%%")||
            (clean.starts_with("$$")&&!clean.ends_with("$$")){uncertain=true;}
        if line.starts_with("    ")||line.starts_with('\t'){
            if clean.starts_with("~~~")||clean.starts_with("\x60\x60\x60"){uncertain=true;}
            continue;
        }
        if let Some((marker,width,tail))=fence(line){
            active=Some(Fence{marker,width,line:number,table:tail=="rich-table",payload:vec![]});
        }
    }
    if let Some(current)=active{uncertain=true;if current.table{tables.push(record(current,&mut outcomes,false));}}
    uncertain|=math||comment.is_some();
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
