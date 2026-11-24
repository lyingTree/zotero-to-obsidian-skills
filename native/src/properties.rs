//! Byte-preserving frontmatter processing; ambiguous destructive edits fail closed.
use crate::{decision,files};
use anyhow::{anyhow,bail,ensure,Result};
use serde_json::{json,Map,Value};
use std::path::Path;

const BOM:&[u8]=b"\xef\xbb\xbf";
const DEPRECATED:&str="库内参考文献";

fn lines(data:&[u8])->Vec<&[u8]>{
    let mut result=vec![];
    let mut start=0;
    let mut i=0;
    while i<data.len(){
        if data[i]==b'\r'||data[i]==b'\n'{
            i+=1;
            if data[i-1]==b'\r'&&i<data.len()&&data[i]==b'\n'{i+=1;}
            result.push(&data[start..i]);start=i;
        }else{i+=1;}
    }
    if start<data.len(){result.push(&data[start..]);}
    result
}
fn trimmed(line:&[u8])->&[u8]{
    let mut end=line.len();
    while end>0&&matches!(line[end-1],b'\r'|b'\n'|b' '|b'\t'){end-=1;}
    &line[..end]
}
pub struct Parts<'a>{pub prefix:&'a[u8],pub body:&'a[u8],pub payload:Option<&'a[u8]>}
pub fn split(data:&[u8])->Result<Parts<'_>>{
    let offset=if data.starts_with(BOM){BOM.len()}else{0};
    let rows=lines(&data[offset..]);
    let Some(first)=rows.first().filter(|line|trimmed(line)==b"---") else {
        return Ok(Parts{prefix:&data[..offset],body:&data[offset..],payload:None});
    };
    let start=offset+first.len();
    let mut cursor=start;
    for row in rows.iter().skip(1){
        let value=trimmed(row);
        if value==b"---"||value==b"..."{
            return Ok(Parts{prefix:&data[..cursor+row.len()],body:&data[cursor+row.len()..],payload:Some(&data[start..cursor])});
        }
        cursor+=row.len();
    }
    bail!("unclosed beginning YAML frontmatter")
}
pub fn parse_yaml(data:&[u8])->Result<Value>{
    // serde-saphyr rejects duplicate keys by default. Non-JSON YAML keys and
    // values fail closed rather than being silently changed by reserialization.
    let text=std::str::from_utf8(data)?;
    Ok(serde_saphyr::from_str::<crate::StrictValue>(text)?.0)
}
fn mapping(payload:Option<&[u8]>)->Result<Map<String,Value>>{
    parse_yaml(payload.ok_or_else(||anyhow!("missing beginning YAML frontmatter"))?)?
        .as_object().cloned().ok_or_else(||anyhow!("frontmatter must be a mapping of string keys"))
}
fn managed(item_type:&str)->Result<Vec<&'static str>>{
    ensure!(["journalArticle","conferencePaper","thesis","preprint"].contains(&item_type),
        "unsupported or unconfirmed Zotero itemType");
    let mut keys=vec!["类型","年份","关键词"];
    if item_type=="journalArticle"{keys.push("期刊名");}Ok(keys)
}
fn valid_types(values:&Map<String,Value>,keys:&[&str])->bool{
    keys.iter().all(|key|{
        let Some(value)=values.get(*key)else{return true;};
        match *key{
            "年份"=>value.is_null()||value.is_i64()||value.is_u64(),
            "关键词"=>value.as_array().is_some_and(|words|words.iter().all(Value::is_string)),
            _=>value.is_null()||value.is_string(),
        }
    })
}
fn python_json(value:&Value)->Result<String>{
    // Match json.dumps(..., ensure_ascii=False) separators for managed scalars/lists.
    if let Some(items)=value.as_array(){
        let values=items.iter().map(python_json).collect::<Result<Vec<_>>>()?;
        return Ok(format!("[{}]",values.join(", ")));
    }
    Ok(serde_json::to_string(value)?)
}
fn newline(data:&[u8])->&[u8]{if data.windows(2).any(|w|w==b"\r\n"){b"\r\n"}else{b"\n"}}
pub fn build(data:&[u8],item_type:&str,confirmed:&Value)->Result<Vec<u8>>{
    let parts=split(data)?;
    let keys=managed(item_type)?;
    let fields=confirmed.as_object().ok_or_else(||anyhow!("confirmed fields must be an object"))?;
    ensure!(parts.payload.is_none(),"existing YAML: use repair_properties");
    ensure!(fields.len()==keys.len()&&keys.iter().all(|key|fields.contains_key(*key)),"confirmed fields must exactly match template");
    ensure!(valid_types(fields,&keys),"managed property value type differs from template");
    let year=&fields["年份"];
    ensure!(year.is_null()||year.as_u64().is_some_and(|v|(1..=9999).contains(&v)),"invalid confirmed year");
    let nl=newline(parts.body);
    let mut result=Vec::with_capacity(data.len()+256);
    result.extend_from_slice(parts.prefix);
    result.extend_from_slice(b"---");result.extend_from_slice(nl);
    for key in keys{
        result.extend_from_slice(format!("{key}: {}",python_json(&fields[key])?).as_bytes());
        result.extend_from_slice(nl);
    }
    result.extend_from_slice(b"---");result.extend_from_slice(nl);
    result.extend_from_slice(parts.body);
    let after=split(&result)?;
    ensure!(after.body==parts.body&&mapping(after.payload)?==*fields,"generated properties did not round-trip");
    Ok(result)
}

pub fn repair(data:&[u8])->Result<Vec<u8>>{
    let parts=split(data)?;
    let mut expected=mapping(parts.payload)?;
    if expected.remove(DEPRECATED).is_none(){return Ok(data.to_vec());}
    let payload=parts.payload.unwrap();
    let source=std::str::from_utf8(payload)?;
    // Only a literal block-mapping entry can be surgically isolated. Reject
    // directives, merge keys and alias/anchor syntax for this mutation.
    ensure!(!source.lines().any(|l|l.trim_start().starts_with(['%','{','?'])||l.trim_start().starts_with("<<:")),
        "cannot isolate deprecated field in complex mapping");
    let rows=lines(payload);
    let key_pattern=[format!("{DEPRECATED}:"),format!("'{DEPRECATED}':"),format!("\"{DEPRECATED}\":")];
    let mut matches=vec![];
    for (i,row) in rows.iter().enumerate(){
        let line=std::str::from_utf8(row)?;
        if key_pattern.iter().any(|prefix|line.strip_prefix(prefix).is_some_and(|rest|rest.is_empty()||rest.starts_with(char::is_whitespace))){matches.push(i);}
    }
    ensure!(matches.len()==1,"deprecated key is not one literal top-level entry");
    let start=matches[0];
    let mut end=start+1;
    // Keep separators and independent comments. A comment between the key
    // and indented value is ambiguous, so refuse instead of deleting it.
    while end<rows.len(){
        let row=rows[end];
        if trimmed(row).is_empty(){
            let next=rows[end+1..].iter().find(|r|!trimmed(r).is_empty());
            if next.is_some_and(|r|r.starts_with(b" ")||r.starts_with(b"\t")){end+=1;continue;}
            break;
        }
        if row.starts_with(b" ")||row.starts_with(b"\t")||row.starts_with(b"- "){
            let line=std::str::from_utf8(row)?;
            ensure!(!line.trim_start().starts_with('#'),"deprecated field intersects independent comments");
            end+=1;continue;
        }
        break;
    }
    let removed=rows[start..end].concat();
    let removed_text=std::str::from_utf8(&removed)?;
    // Alias-bearing field removal could invalidate another field. Block
    // conservatively even if the token appears in a literal string.
    ensure!(!removed_text.contains(['&','*']),"cannot isolate YAML anchors or aliases");
    let remaining=[rows[..start].concat(),rows[end..].concat()].concat();
    let remaining=if remaining.iter().all(u8::is_ascii_whitespace){
        [b"{}".as_slice(),newline(payload)].concat()
    }else{remaining};
    let opening=parts.prefix.len()-payload.len();
    let payload_start=if data.starts_with(BOM){BOM.len()}else{0};
    let prefix_len=payload_start+lines(&data[payload_start..])[0].len();
    ensure!(opening>=prefix_len,"frontmatter range inconsistent");
    let closing_start=prefix_len+payload.len();
    let mut result=Vec::with_capacity(data.len());
    result.extend_from_slice(&data[..prefix_len]);result.extend_from_slice(&remaining);
    result.extend_from_slice(&data[closing_start..]);
    let after=split(&result)?;
    ensure!(after.body==parts.body&&mapping(after.payload)?==expected,"removal changed other YAML values or body");
    Ok(result)
}
fn put(checks:&mut Map<String,Value>,id:&str,result:Option<bool>,reason:&str){
    checks.insert(id.into(),decision(id,result,reason));
}
pub fn audit(before:&Path,after:&Path,mode:&str,item_type:&str)->Result<Value>{
    ensure!(["new","repair"].contains(&mode),"mode must be new or repair");
    let keys=managed(item_type)?;
    let (original,a)=files::read_bound(before)?;let (candidate,b)=files::read_bound(after)?;
    let old=split(&original);let new=split(&candidate);
    let mut checks=Map::new();
    put(&mut checks,"PROP-A08",old.as_ref().ok().zip(new.as_ref().ok()).map(|(x,y)|x.body==y.body),
        "compare actual body bytes outside beginning YAML");
    let payload=new.as_ref().ok().and_then(|p|p.payload);
    let parse=payload.ok_or_else(||anyhow!("beginning YAML missing or unclosed")).and_then(parse_yaml);
    let values=mapping(payload);
    // Parser rejects duplicate keys. A generic parse error cannot alone prove
    // that a duplicate occurred (or that ordinary YAML syntax is invalid).
    let parse_ok=parse.is_ok();
    let duplicate=parse.as_ref().err().is_some_and(|e|{
        let s=e.to_string().to_lowercase();s.contains("duplicate")&&s.contains("key")
    });
    put(&mut checks,"PROP-A01",if duplicate{None}else{Some(parse_ok)},"actual YAML parsing");
    put(&mut checks,"PROP-A02",if duplicate{Some(false)}else if values.is_ok(){Some(true)}else{None},"unique-key YAML mapping");
    put(&mut checks,"PROP-A04",values.as_ref().ok().map(|v|!v.contains_key(DEPRECATED)),"check the actual parsed top-level key set");
    let key_match=values.as_ref().ok().and_then(|v|{
        if mode=="new"{Some(v.len()==keys.len()&&keys.iter().all(|key|v.contains_key(*key)))}
        else{let mut previous=mapping(old.as_ref().ok()?.payload).ok()?;previous.remove(DEPRECATED);Some(previous.keys().eq(v.keys()))}
    });
    put(&mut checks,"PROP-A05",key_match,"compare keys with selected template or original properties");
    put(&mut checks,"PROP-A06",values.as_ref().ok().map(|v|valid_types(v,&keys)),"check actual managed value types");
    if mode=="repair"{
        let exact=repair(&original).ok().map(|value|value==candidate);
        put(&mut checks,"PROP-C10",exact,"compare complete result with surgical removal");
        put(&mut checks,"PROP-C18",exact,"compare original writing outside deprecated field");
    }
    Ok(json!({"stage":"properties","mode":mode,"item_type":item_type,"inputs":[a,b],"checks":checks,"source_fidelity":"not_assessed"}))
}
pub fn audit_unchanged(before:&Path,after:&Path,stage:&str)->Result<Value>{
    let id=match stage{"header"=>"HDR-A14","library"=>"LIB-A08",_=>bail!("stage must be header or library")};
    let (a,first)=files::read_bound(before)?;let (b,second)=files::read_bound(after)?;
    let result=split(&a).ok().zip(split(&b).ok()).map(|(a,b)|a.prefix==b.prefix);
    Ok(json!({"stage":stage,"inputs":[first,second],
        "checks":{(id):decision(id,result,"compare complete beginning property region byte-for-byte")},"source_fidelity":"not_assessed"}))
}
pub fn audit_write(candidate:&Path,written:&Path)->Result<Value>{
    let (a,first)=files::read_bound(candidate)?;let (b,second)=files::read_bound(written)?;
    ensure!(first["path"]!=second["path"],"readback needs independent candidate and actual target files");
    let absent=split(&b).ok().and_then(|p|mapping(p.payload).ok()).map(|m|!m.contains_key(DEPRECATED));
    Ok(json!({"stage":"write","inputs":[first,second],
        "checks":{"WRITE-A04":decision("WRITE-A04",Some(files::digest(&a)==files::digest(&b)),"actual candidate and readback SHA-256"),
        "WRITE-A05":decision("WRITE-A05",absent,"parsed actual written YAML")},"source_fidelity":"not_assessed"}))
}
