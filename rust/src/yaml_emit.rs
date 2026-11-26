//! Stable block YAML for reviewed JSON-shaped data.
use anyhow::{ensure,Result};
use regex::Regex;
use serde_json::{json,Map,Value};
use std::{collections::BTreeSet,sync::OnceLock};
use crate::{properties::parse_yaml,validate};

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

fn split_gfm_cells(line:&str)->Vec<String>{
    let t=line.trim();
    let t=t.strip_prefix('|').unwrap_or(t);
    let t=t.strip_suffix('|').unwrap_or(t);
    let bytes=t.as_bytes();
    let mut out=Vec::new();
    let mut cur=String::new();
    let mut in_code=false;
    let mut in_math=false;
    let mut i=0usize;
    while i<bytes.len(){
        let ch=t[i..].chars().next().unwrap();
        if ch=='\\'&&i+1<bytes.len()&&bytes[i+1]==b'|'{
            cur.push('|');i+=2;continue;
        }
        if ch=='`'{in_code=!in_code;}
        else if ch=='$'&&!in_code&&(i==0||bytes[i-1]!=b'\\'){in_math=!in_math;}
        if ch=='|'&&!in_code&&!in_math{
            out.push(cur.trim().to_owned());
            cur.clear();
        }else{
            cur.push(ch);
        }
        i+=ch.len_utf8();
    }
    out.push(cur.trim().to_owned());
    out
}

fn is_gfm_separator(line:&str)->Option<usize>{
    static SEP:OnceLock<Regex>=OnceLock::new();
    let rx=SEP.get_or_init(||Regex::new(r"^:?-{3,}:?$").unwrap());
    let t=line.trim();
    if !t.starts_with('|'){return None;}
    let cells=split_gfm_cells(t);
    (!cells.is_empty()&&cells.iter().all(|c|rx.is_match(c.trim()))).then_some(cells.len())
}

fn is_gfm_row(line:&str,cols:usize)->Option<Vec<String>>{
    let t=line.trim();
    if !t.starts_with('|'){return None;}
    let cells=split_gfm_cells(t);
    (cells.len()==cols).then_some(cells)
}

pub fn convert_markdown_tables(text:&str)->Result<(String,usize)>{
    static ANCHOR_ALL:OnceLock<Regex>=OnceLock::new();
    static ANCHOR_LINE:OnceLock<Regex>=OnceLock::new();
    static CAPTION_LINE:OnceLock<Regex>=OnceLock::new();
    let rx_all=ANCHOR_ALL.get_or_init(||Regex::new(r"(?m)(?:^|\s)\^(tbl-[A-Za-z0-9-]+)[ \t]*$").unwrap());
    let rx_anchor=ANCHOR_LINE.get_or_init(||Regex::new(r"^\s*\^(tbl-[A-Za-z0-9-]+)[ \t]*$").unwrap());
    let rx_cap=CAPTION_LINE.get_or_init(||Regex::new(r"(?i)^\*\*(?:表|Table)\s*([A-Za-z0-9.-]+)?[：:.]").unwrap());

    let lf=text.replace("\r\n","\n").replace('\r',"\n");
    let mut used_anchors:BTreeSet<String>=rx_all.captures_iter(&lf).map(|c|c[1].to_owned()).collect();
    let lines:Vec<&str>=lf.split('\n').collect();
    let mut out:Vec<String>=Vec::with_capacity(lines.len());
    let mut fence:Option<(u8,usize)>=None;
    let mut front=lines.first().is_some_and(|l|l.trim_start_matches('\u{feff}').trim()=="---");
    let mut comment:Option<&'static str>=None;
    let mut auto_seq=1usize;
    let mut converted=0usize;
    let mut i=0usize;

    while i<lines.len(){
        let raw=lines[i];
        if front{
            out.push(raw.to_owned());
            if i>0&&matches!(raw.trim(),"---"|"..."){front=false;}
            i+=1;continue;
        }
        if let Some((mark,width))=fence{
            out.push(raw.to_owned());
            if validate::closing(raw,mark,width){fence=None;}
            i+=1;continue;
        }
        let (safe,next_comment)=validate::mask_comments(raw,comment);
        if let Some((mark,width,_))=validate::fence(&safe){
            fence=Some((mark,width));comment=next_comment;
            out.push(raw.to_owned());i+=1;continue;
        }
        if validate::indentation(&safe)>=4{
            comment=next_comment;out.push(raw.to_owned());i+=1;continue;
        }

        if comment.is_none() && safe.trim().starts_with('|') && i+1<lines.len(){
            if let Some(sep_cols)=is_gfm_separator(lines[i+1]){
                if let Some(headers)=is_gfm_row(raw,sep_cols){
                    let mut j=i+2;
                    let mut data_rows=Vec::new();
                    while j<lines.len(){
                        if let Some(r)=is_gfm_row(lines[j],sep_cols){
                            data_rows.push(r);
                            j+=1;
                        }else{break;}
                    }
                    if !data_rows.is_empty(){
                        let mut title:Option<String>=None;
                        let mut cap_num:Option<String>=None;
                        let mut trim_out=out.len();
                        while trim_out>0 && out[trim_out-1].trim().is_empty(){trim_out-=1;}
                        if trim_out>0 && out.len()-trim_out<=2{
                            if let Some(c)=rx_cap.captures(out[trim_out-1].trim()){
                                title=Some(out[trim_out-1].trim().to_owned());
                                cap_num=c.get(1).map(|m|m.as_str().to_owned());
                                out.truncate(trim_out-1);
                            }
                        }

                        let mut k=j;
                        if k<lines.len() && lines[k].trim().is_empty() && k+1<lines.len() && rx_anchor.is_match(lines[k+1]){
                            k+=1;
                        }
                        let anchor=if k<lines.len() && rx_anchor.is_match(lines[k]){
                            let cap=rx_anchor.captures(lines[k]).unwrap();
                            k+=1;
                            cap[1].to_owned()
                        }else{
                            k=j;
                            let mut cand=cap_num.as_deref()
                                .map(|n|format!("tbl-{}",n.to_ascii_lowercase().replace('.',"-").chars().filter(|c|c.is_ascii_alphanumeric()||*c=='-').collect::<String>()))
                                .filter(|s|s.len()>4)
                                .unwrap_or_else(||format!("tbl-auto-{auto_seq}"));
                            while used_anchors.contains(&cand){
                                auto_seq+=1;
                                cand=format!("tbl-auto-{auto_seq}");
                            }
                            used_anchors.insert(cand.clone());
                            auto_seq+=1;
                            cand
                        };

                        let cols:Vec<Value>=headers.iter().enumerate().map(|(idx,h)|json!({
                            "id":format!("c_{:06}",idx+1),
                            "name":h
                        })).collect();
                        let rows_json:Vec<Value>=data_rows.iter().enumerate().map(|(ridx,cells)|{
                            let mut cmap=Map::new();
                            for (cidx,val) in cells.iter().enumerate(){
                                cmap.insert(format!("c_{:06}",cidx+1),json!(val));
                            }
                            json!({"id":format!("r_{:06}",ridx+1),"cells":Value::Object(cmap)})
                        }).collect();
                        let mut data=json!({
                            "version":2,
                            "columns":cols,
                            "rows":rows_json,
                            "theme":"grid",
                            "locked":true,
                        });
                        if let Some(t)=title{data["title"]=json!(t);}
                        let block=rich_table(&data,&anchor,true,"\n")?;
                        while out.last().is_some_and(|l|l.trim().is_empty()){out.pop();}
                        if !out.is_empty(){out.push(String::new());}
                        for bline in block.trim_end_matches('\n').split('\n'){
                            out.push(bline.to_owned());
                        }
                        if k<lines.len()&&!lines[k].trim().is_empty(){
                            out.push(String::new());
                        }
                        converted+=1;
                        comment=None;
                        i=k;
                        continue;
                    }
                }
            }
        }
        comment=next_comment;
        out.push(raw.to_owned());
        i+=1;
    }
    Ok((out.join("\n"),converted))
}

#[cfg(test)]
mod tests{
    use super::convert_markdown_tables;

    #[test]
    fn converts_gfm_table_with_bold_caption_math_and_anchor(){
        let md=concat!(
            "正文开始。\n\n",
            "**表1：** 关键指标对比\n",
            "| 方法 | 值域 | 备注 |\n",
            "| :--- | :---: | ---: |\n",
            "| Mayer $\\Delta_k$ | $0.95$ | 无 |\n",
            "| Hodge | $0.88$ | baseline |\n",
            "^tbl-1\n\n",
            "后续段落。\n"
        );
        let (out,count)=convert_markdown_tables(md).unwrap();
        assert_eq!(count,1);
        assert!(out.contains("```rich-table"),"{out}");
        assert!(out.contains("version: 2"),"{out}");
        assert!(out.contains("title: '**表1：** 关键指标对比'"),"{out}");
        assert!(out.contains("Mayer $\\Delta_k$"),"{out}");
        assert!(out.contains("```\n^tbl-1"),"{out}");
        assert!(out.contains("后续段落。"),"{out}");
    }
}
