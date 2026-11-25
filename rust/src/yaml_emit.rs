//! Stable block YAML for reviewed JSON-shaped data.
use anyhow::{ensure,Result};
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;
use crate::properties::parse_yaml;

fn quote(s:&str)->String{
    static IMPLICIT:OnceLock<Regex>=OnceLock::new();
    let implicit=IMPLICIT.get_or_init(||Regex::new(r"(?i)^(?:yes|no|true|false|on|off|null|~|[-+]?(?:[0-9][0-9_]*(?:[.:][0-9_]+)*(?:[eE][-+]?[0-9]+)?|0[xob][0-9a-f_]+|\.[0-9_]+|\.inf|\.nan)|[0-9]{4}-[0-9]{1,2}-[0-9]{1,2}.*)$").unwrap());
    if s.chars().any(|c|c.is_control()||matches!(c,'\u{2028}'|'\u{2029}'|'\u{feff}')){
        return serde_json::to_string(s).expect("string serialization");
    }
    let plain=!s.is_empty()&&s.trim()==s&&
        !s.chars().next().is_some_and(|c|matches!(c,'-'|'?'|':'|'['|']'|'{'|'}'|','|'#'|'&'|'*'|'!'|'|'|'>'|'\''|'"'|'%'|'@'|'\x60'))&&
        !s.contains(": ")&&!s.contains(" #")&&!s.ends_with(':')&&!implicit.is_match(s)&&
        !matches!(s,"---"|"...");
    if plain{return s.into();}
    format!("'{}'",s.replace('\'',"''"))
}
fn scalar(v:&Value)->Option<String>{
    match v{
        Value::Null=>Some("null".into()),Value::Bool(b)=>Some(b.to_string()),Value::Number(n)=>Some(n.to_string()),
        Value::Array(a) if a.is_empty()=>Some("[]".into()),Value::Object(o) if o.is_empty()=>Some("{}".into()),
        Value::String(s)=>Some(quote(s)),_=>None,
    }
}
fn indent(out:&mut String,depth:usize){out.extend(std::iter::repeat_n(' ',depth));}
fn append(v:&Value,depth:usize,out:&mut String,inline:bool){
    if let Some(s)=scalar(v){if !inline{indent(out,depth);}out.push_str(&s);out.push('\n');return;}
    match v{
        Value::Object(map)=>for(i,(key,value))in map.iter().enumerate(){
            if !inline||i!=0{indent(out,depth);}out.push_str(&quote(key));out.push(':');
            if let Some(s)=scalar(value){out.push(' ');out.push_str(&s);out.push('\n');}
            else{out.push('\n');append(value,if value.is_array(){depth}else{depth+2},out,false);}
        },
        Value::Array(items)=>for(i,value)in items.iter().enumerate(){
            if !inline||i!=0{indent(out,depth);}out.push_str("- ");append(value,depth+2,out,true);
        },
        _=>unreachable!(),
    }
}
pub fn dump(v:&Value)->Result<String>{
    let mut result=String::new();append(v,0,&mut result,false);
    ensure!(parse_yaml(result.as_bytes())?==*v,"YAML serialization changed a supplied value");
    Ok(result)
}
pub fn rich_table(data:&Value,anchor:&str,wrapped:bool,newline:&str)->Result<String>{
    let id=anchor.strip_prefix('^').unwrap_or(anchor);
    ensure!(id.strip_prefix("tbl-").is_some_and(|s|!s.is_empty()&&s.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-')),"anchor must be a supplied tbl- block ID");
    ensure!(matches!(newline,"\n"|"\r\n"),"newline must be LF or CRLF");
    let sheets=match data["version"].as_u64(){
        Some(2)=>vec![data],
        Some(3)=>data["sheets"].as_array().map(|a|a.iter().collect()).unwrap_or_default(),
        _=>anyhow::bail!("version must be integer 2 or 3"),
    };
    ensure!(!sheets.is_empty(),"a v3 workbook requires a nonempty sheets list");
    for(i,sheet)in sheets.iter().enumerate(){
        ensure!(sheet.is_object(),"sheet {} must be a mapping",i+1);
        ensure!(sheet["locked"]==true,"sheet {} must explicitly supply locked=True",i+1);
        ensure!(sheet["theme"]=="grid","sheet {} must explicitly supply theme='grid'",i+1);
        for name in ["columns","rows"]{ensure!(sheet[name].as_array().is_some_and(|v|!v.is_empty()),"sheet {} must supply nonempty {name}",i+1);}
    }
    let payload=dump(data)?;
    let payload=if wrapped{format!("---\n{payload}---\n")}else{payload};
    let mut longest=0;let mut count=0;
    for c in payload.chars(){if c=='\x60'{count+=1;longest=longest.max(count);}else{count=0;}}
    let fence="\x60".repeat(3.max(longest+1));
    let block=format!("{fence}rich-table\n{payload}{fence}\n^{id}\n");
    Ok(if newline=="\n"{block}else{block.replace('\n',newline)})
}
