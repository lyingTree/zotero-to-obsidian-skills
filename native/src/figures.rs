//! Source-bound figure captions and stable anchors; no image identity inference.
use crate::string;
use anyhow::{anyhow,ensure,Result};
use regex::Regex;
use serde_json::{json,Value};

pub fn new_anchor(source:&str,number:&str,occupied:&Value)->Result<String>{
    ensure!([source,number].iter().all(|s|!s.is_empty()&&s.bytes().all(|b|b.is_ascii_alphanumeric())),
        "tokens must be registered ASCII alphanumeric values");
    let occupied=occupied.as_array().ok_or_else(||anyhow!("occupied_ids must be an array"))?;
    ensure!(occupied.iter().all(Value::is_string),"occupied IDs must be strings");
    let anchor=format!("^tbl-fig-{source}-{number}");
    ensure!(!occupied.iter().any(|v|v.as_str()==Some(anchor.as_str())||v.as_str()==Some(&anchor[1..])),
        "new anchor is already occupied: {anchor}");
    Ok(anchor)
}
pub fn bold_label(text:&str)->bool{
    Regex::new(r"^ {0,3}\*\*(?:\\[^\r\n]|[^*\\\r\n])+\*\*").expect("static regex").is_match(text)
}
pub fn build(images:&Value,caption:&str,anchor:&str,newline:&str)->Result<String>{
    let images=images.as_array().filter(|v|!v.is_empty()).ok_or_else(||anyhow!("images must be a nonempty array"))?;
    let id=anchor.strip_prefix('^').unwrap_or(anchor);
    ensure!(!id.is_empty()&&id.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-'),"invalid block ID");
    ensure!(["\n","\r\n"].contains(&newline),"newline must be LF or CRLF");
    ensure!(caption==caption.trim()&&!caption.contains(['\r','\n'])&&bold_label(caption),
        "caption must be one line with its explicit bold source label");
    let mut lines=Vec::with_capacity(images.len()+2);
    for item in images{
        let fields=item.as_object().ok_or_else(||anyhow!("image must be a mapping"))?;
        ensure!(fields.len()==2&&fields.contains_key("target")&&fields.contains_key("alt"),"each image must contain exactly target and alt");
        let target=string(&fields["target"],"image target")?;
        let alt=fields["alt"].as_str().ok_or_else(||anyhow!("image alt must be a string"))?;
        ensure!(target==target.trim()&&!target.contains(['<','>','\r','\n']),"supply an unwrapped single-line image target");
        ensure!(!alt.contains(['\r','\n']),"image alt must be one line");
        let alt=alt.replace('\\',"\\\\").replace('[',"\\[").replace(']',"\\]").replace('&',"&amp;");
        let target=target.replace('%',"%25").replace('#',"%23").replace('?',"%3F").replace('\\',"\\\\").replace('&',"&amp;");
        lines.push(format!("![{alt}](<{target}>)  "));
    }
    lines.push(caption.to_owned());lines.push(format!("^{id}"));
    Ok(lines.join(newline)+newline)
}
fn number(v:&Value,label:&str)->Result<String>{
    let text=string(v,label)?;
    ensure!(text==text.trim()&&!Regex::new(r"[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]").expect("static regex").is_match(&text),
        "{label} must be a literal number without controls or outer whitespace");
    Ok(text)
}
fn escape(s:&str)->String{
    let mut result=String::with_capacity(s.len());
    for c in s.chars(){if "\\\x60*_[]<>$~%&".contains(c){result.push('\\');}result.push(c);}
    result
}
pub fn supplement(original:&Value,caption:&str,display:&Value,reason:&Value)->Result<Value>{
    ensure!(!caption.is_empty()&&caption==caption.trim(),"caption must not have outer whitespace");
    if original.is_null(){
        let reason=string(reason,"original_number_reason")?;
        let display=number(display,"display_number")?;
        ensure!(display.to_lowercase().starts_with('s'),"generated SI navigation number requires S prefix");
        return Ok(json!({"original_number":null,"original_number_reason":reason,
            "display_number":display,"display_number_origin":"generated_navigation",
            "caption":format!("**图{}：** {caption}（来源无图号）",escape(&display))}));
    }
    let original=number(original,"original_number")?;
    ensure!(reason.is_null(),"absence reason requires unnumbered source figure");
    let expected=if original.to_lowercase().starts_with('s'){original.clone()}else{format!("S{original}")};
    let display=if display.is_null(){expected.clone()}else{number(display,"display_number")?};
    ensure!(display==expected,"preserve the original number; only add one S prefix when absent");
    let note=if display==original{String::new()}else{format!("（来源原号：{}）",escape(&original))};
    Ok(json!({"original_number":original,"display_number":display,
        "display_number_origin":if display==original{"source_literal"}else{"normalized_source"},
        "caption":format!("**图{}：** {caption}{note}",escape(&display))}))
}
