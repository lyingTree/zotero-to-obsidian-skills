//! arXiv/LaTeX source assistance. The TeX text is an independent evidence
//! source for equations, tables and bibliography; PDF pixels remain the
//! authority for visible layout and figure membership.
use anyhow::{anyhow,ensure,Context,Result};
use flate2::read::GzDecoder;
use regex::Regex;
use serde_json::{json,Value};
use std::{
    collections::{BTreeMap,BTreeSet},
    fs,
    io::{Cursor,Read,Write},
    path::{Path,PathBuf},
    process::Command,
    sync::OnceLock,
};
use crate::{check_ledger,files};

const MAX_ARCHIVE:u64=64*1024*1024;
const MAX_MEMBER:u64=32*1024*1024;
const MAX_FILES:usize=4096;
const TEXT_EXT:&[&str]=&["tex","ltx","bbl","bib","sty","cls","clo"];
const MATH_ENVS:&[&str]=&["equation","equation*","align","align*","gather","gather*","multline","multline*","eqnarray","eqnarray*","displaymath"];
const TABLE_ENVS:&[&str]=&["table","table*","longtable","tabular","tabular*","tabularx","tabulary","array"];
const FIGURE_ENVS:&[&str]=&["figure","figure*","wrapfigure","subfigure","subfigure*"];

fn decode_tex_text(bytes:&[u8],name:&str)->Result<String>{
    Ok(crate::mojibake::decode_text_bytes(bytes,name)?.text)
}
fn gunzip_bounded(raw:&[u8])->Result<Vec<u8>>{
    let mut decoded=Vec::new();
    GzDecoder::new(Cursor::new(raw)).take(MAX_ARCHIVE.saturating_add(1)).read_to_end(&mut decoded)?;
    ensure!(decoded.len() as u64<=MAX_ARCHIVE,"Decompressed TeX archive exceeds size budget");
    Ok(decoded)
}

fn read_bound(source:&Path,expected:Option<&str>)->Result<(PathBuf,Vec<u8>,Value)>{
    let p=files::canonical(source)?;
    let before=check_ledger::bind_file(&p)?;
    if let Some(hash)=expected{ensure!(before["sha256"]==hash,"TeX source hash differs from the selected inventory");}
    ensure!(fs::metadata(&p)?.len()<=MAX_ARCHIVE,"TeX source exceeds size budget");
    let raw=fs::read(&p)?;
    ensure!(raw.len() as u64<=MAX_ARCHIVE&&files::digest(&raw)==before["sha256"],"TeX source changed during read");
    Ok((p,raw,before))
}
fn safe_name(name:&str)->Result<String>{
    let name=name.replace('\\',"/");
    let mut parts=vec![];
    for part in name.split('/'){
        ensure!(!part.is_empty()&&part!="."&&part!="..","Unsafe TeX archive member: {name}");
        parts.push(part);
    }
    ensure!(!name.starts_with('/')&&!name.contains(':'),"Absolute TeX archive member: {name}");
    Ok(parts.join("/"))
}
fn text_extension(name:&str)->bool{
    name.rsplit_once('.').is_some_and(|(_,e)|TEXT_EXT.contains(&e.to_ascii_lowercase().as_str()))
}
fn zip_files(raw:&[u8])->Result<BTreeMap<String,String>>{
    let mut archive=zip::ZipArchive::new(Cursor::new(raw)).context("Read arXiv ZIP source")?;
    ensure!(archive.len()<=MAX_FILES,"TeX archive exceeds file-count budget");
    let mut out=BTreeMap::new();
    for i in 0..archive.len(){
        let mut f=archive.by_index(i)?;
        if f.is_dir()||!text_extension(f.name()){continue;}
        let name=safe_name(f.name())?;
        ensure!(f.size()<=MAX_MEMBER,"TeX archive member exceeds size budget: {name}");
        let mut bytes=Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut bytes)?;
        ensure!(bytes.len() as u64<=MAX_MEMBER,"TeX archive member exceeds size budget: {name}");
        let text=decode_tex_text(&bytes,&name)?;
        out.insert(name,text);
    }
    Ok(out)
}
fn octal(bytes:&[u8])->Result<u64>{
    let text=String::from_utf8_lossy(bytes).trim_matches(['\0',' ']).trim().to_owned();
    if text.is_empty(){return Ok(0);}
    let digits=text.trim_start_matches('0');
    if digits.is_empty(){return Ok(0);}
    u64::from_str_radix(digits,8).map_err(|_|anyhow!("Invalid tar size field"))
}
fn tar_files(raw:&[u8])->Result<BTreeMap<String,String>>{
    let mut out=BTreeMap::new();
    let mut at=0usize;
    while at+512<=raw.len(){
        let header=&raw[at..at+512];
        if header.iter().all(|b|*b==0){break;}
        let name=String::from_utf8_lossy(&header[0..100]).trim_matches('\0').trim().to_owned();
        let prefix=String::from_utf8_lossy(&header[345..500]).trim_matches('\0').trim().to_owned();
        let size=usize::try_from(octal(&header[124..136])?)?;
        let kind=header[156];
        let data_start=at+512;
        ensure!(data_start.checked_add(size).is_some_and(|n|n<=raw.len()),"Truncated tar archive");
        let data=&raw[data_start..data_start+size];
        at=data_start+size.div_ceil(512)*512;
        if !matches!(kind,0|b'0'){continue;}
        let full=if prefix.is_empty(){name}else{format!("{prefix}/{name}")};
        if !text_extension(&full){continue;}
        let full=safe_name(&full)?;
        ensure!(out.len()<MAX_FILES,"TeX archive exceeds file-count budget");
        let text=decode_tex_text(data,&full)?;
        out.insert(full,text);
    }
    Ok(out)
}
fn looks_like_tar(raw:&[u8])->bool{
    raw.len()>=512 && (&raw[257..262]==b"ustar" || tar_files(raw).is_ok_and(|m|!m.is_empty()))
}
fn source_files(name:&str,raw:&[u8])->Result<BTreeMap<String,String>>{
    ensure!(raw.len() as u64<=MAX_ARCHIVE,"TeX source exceeds size budget");
    let head=String::from_utf8_lossy(&raw[..raw.len().min(64)]).trim_start().to_ascii_lowercase();
    ensure!(!raw.starts_with(b"%PDF-")&&!head.starts_with("<!doctype")&&!head.starts_with("<html"),
        "File is PDF or HTML rather than TeX source");
    let lower=name.to_ascii_lowercase();
    if raw.starts_with(b"PK\x03\x04")||lower.ends_with(".zip"){return zip_files(raw);}
    if raw.starts_with(&[0x1f,0x8b])||lower.ends_with(".tar.gz")||lower.ends_with(".tgz")||lower.ends_with(".gz"){
        let decoded=gunzip_bounded(raw)?;
        if lower.ends_with(".tar.gz")||lower.ends_with(".tgz")||looks_like_tar(&decoded){
            return tar_files(&decoded);
        }
        let stem=name.strip_suffix(".gz").unwrap_or("main.tex");
        let tex_name=if stem.to_ascii_lowercase().ends_with(".tex")||stem.to_ascii_lowercase().ends_with(".ltx"){
            safe_name(stem)?
        }else{"main.tex".to_owned()};
        return Ok(BTreeMap::from([(tex_name,decode_tex_text(&decoded,stem)?)]));
    }
    if lower.ends_with(".tar")||looks_like_tar(raw){return tar_files(raw);}
    if lower.ends_with(".tex")||lower.ends_with(".ltx")||head.contains("\\documentclass")||head.contains("\\begin{"){
        let text=decode_tex_text(raw,name)?;
        return Ok(BTreeMap::from([(safe_name(name)?,text)]));
    }
    Err(anyhow!("Unsupported arXiv/TeX source format; use .tex, .zip, .tar, .tar.gz or .gz"))
}
fn main_file(files:&BTreeMap<String,String>)->Result<String>{
    let candidates:Vec<_>=files.iter().filter(|(_,v)|v.contains("\\documentclass")).map(|(k,_)|k).collect();
    if let Some(name)=candidates.iter().min_by_key(|n|n.len()){return Ok((**name).clone());}
    files.iter().filter(|(k,_)|k.to_ascii_lowercase().ends_with(".tex"))
        .max_by_key(|(_,v)|v.len()).map(|(k,_)|k.clone())
        .context("TeX archive contains no .tex file")
}
fn join_relative(base:&str,include:&str)->Option<String>{
    let include=include.replace('\\',"/");
    let mut parts:Vec<&str>=base.split('/').collect();parts.pop();
    for part in include.split('/'){
        match part{""|"."=>{},".."=>{parts.pop()?;},_=>parts.push(part)}
    }
    (!parts.is_empty()).then(||parts.join("/"))
}
fn expand_inputs(name:&str,text:&str,files:&BTreeMap<String,String>,seen:&mut BTreeSet<String>,depth:usize)->Result<String>{
    ensure!(depth<=12,"TeX input nesting exceeds depth budget");
    if !seen.insert(name.to_owned()){return Ok(String::new());}
    let bbl_files:Vec<&String>=files.keys().filter(|k|k.to_ascii_lowercase().ends_with(".bbl")).collect();
    let main_bbl=name.rsplit_once('.').map(|(stem,_)|format!("{stem}.bbl"));
    let mut out=String::with_capacity(text.len());
    for line in text.lines(){
        let uncommented=strip_comment(line);
        if let Some(arg)=command_arg(&uncommented,"\\bibliography"){
            let mut candidates=Vec::new();
            for item in arg.split(',').map(str::trim).filter(|s|!s.is_empty()){
                if let Some(rel)=join_relative(name,&format!("{item}.bbl")){candidates.push(rel);}
            }
            if let Some(mb)=&main_bbl{candidates.push(mb.clone());}
            if bbl_files.len()==1{candidates.push(bbl_files[0].clone());}
            if let Some(resolved)=candidates.into_iter().find(|c|files.contains_key(c)&&!seen.contains(c)){
                out.push_str(line);
                out.push('\n');
                out.push_str(&expand_inputs(&resolved,&files[&resolved],files,seen,depth+1)?);
                out.push('\n');
                continue;
            }
        }
        let mut cursor=0usize;
        loop{
            let active=&uncommented[cursor.min(uncommented.len())..];
            let next=["\\input{","\\include{"].into_iter()
                .filter_map(|marker|active.find(marker).map(|at|(cursor+at,marker)))
                .min_by_key(|(at,_)|*at);
            let Some((at,marker))=next else{out.push_str(&line[cursor..]);break;};
            out.push_str(&line[cursor..at]);
            let rest=&line[at+marker.len()..];
            let Some(end)=rest.find('}') else{out.push_str(&line[at..]);break;};
            let target=rest[..end].trim();
            let Some(relative)=join_relative(name,target) else{out.push_str(&line[at..at+marker.len()+end+1]);cursor=at+marker.len()+end+1;continue;};
            let resolved=[relative.clone(),format!("{relative}.tex")].into_iter()
                .find(|candidate|files.contains_key(candidate));
            if let Some(resolved)=resolved{
                out.push_str(&expand_inputs(&resolved,&files[&resolved],files,seen,depth+1)?);
            }else{
                out.push_str(&line[at..at+marker.len()+end+1]);
            }
            cursor=at+marker.len()+end+1;
        }
        out.push('\n');
    }
    Ok(out)
}
fn strip_comment(line:&str)->String{
    let mut escaped=false;
    for (i,c) in line.char_indices(){
        if c=='%'&&!escaped{return line[..i].to_owned();}
        escaped=c=='\\'&&!escaped;
        if c!='\\'{escaped=false;}
    }
    line.to_owned()
}
fn env_name(line:&str,prefix:&str)->Option<String>{
    let at=line.find(prefix)?+prefix.len();
    let rest=&line[at..];
    let end=rest.find('}')?;
    let name=rest[..end].trim();
    (!name.is_empty()&&name.chars().all(|c|c.is_ascii_alphanumeric()||matches!(c,'*'|'-')))
        .then(||name.to_owned())
}
fn command_arg<'a>(line:&'a str,command:&str)->Option<&'a str>{
    let at=line.find(command)?+command.len();
    let mut rest=line[at..].trim_start();
    if rest.starts_with('['){
        let end=rest.find(']')?;
        rest=rest[end+1..].trim_start();
    }
    let rest=rest.strip_prefix('{')?;
    let end=rest.find('}')?;
    Some(rest[..end].trim())
}
fn plain_text(latex:&str)->String{
    let mut out=String::new();let mut i=0usize;let bytes=latex.as_bytes();
    while i<bytes.len(){
        let c=latex[i..].chars().next().unwrap();
        if c=='\\'{
            let rest=&latex[i+1..];
            let name_len=rest.find(|c:char|!c.is_ascii_alphabetic()).unwrap_or(rest.len());
            let name=&rest[..name_len];
            i+=1+name_len;
            if name.is_empty(){continue;}
            if ["begin","end","label","cite","ref","eqref","includegraphics","bibliography","bibliographystyle"].contains(&name){
                if latex[i..].starts_with('{'){let end=latex[i+1..].find('}').map(|n|i+1+n).unwrap_or(i);i=end.saturating_add(1);}
                if name=="cite"||name=="ref"||name=="eqref"{out.push_str("[引用]");}
                continue;
            }
            if matches!(name,"frac"|"dfrac"|"tfrac"){continue;}
            if matches!(name,"alpha"|"beta"|"gamma"|"delta"|"pi"|"theta"|"lambda"|"sigma"|"mu"){out.push(c);continue;}
            if matches!(name,"times"|"cdot"){out.push('×');continue;}
            continue;
        }
        if matches!(c,'{'|'}'|'$'|'&'){i+=c.len_utf8();continue;}
        if c=='~'{out.push(' ');i+=1;continue;}
        out.push(c);i+=c.len_utf8();
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn kind(env:&str,command:&str)->&'static str{
    if MATH_ENVS.contains(&env){return "equation";}
    if TABLE_ENVS.contains(&env){return "table";}
    if FIGURE_ENVS.contains(&env){return "figure";}
    if command.starts_with("\\section")||command.starts_with("\\subsection")||command.starts_with("\\subsubsection"){return "section";}
    if command.starts_with("\\bibitem"){return "reference";}
    "paragraph"
}
pub fn parse(name:&str,text:&str)->Result<Value>{
    let text=text.trim_start_matches('\u{feff}').replace("\r\n","\n").replace('\r',"\n");
    let mut blocks=Vec::new();let mut equations=Vec::new();let mut tables=Vec::new();let mut figures=Vec::new();
    let mut references=Vec::new();
    let mut citations=BTreeSet::new();let mut labels=BTreeSet::new();let mut graphics=BTreeSet::new();
    let mut env:Option<(String,usize,String)>=None;let mut math_at:Option<(String,usize,String)>=None;
    let mut paragraph=String::new();let mut paragraph_start=1usize;
    let mut bib_entry:Option<(String,usize,String)>=None;
    let mut in_bib=false;
    let push=|kind:&str,body:&str,start:usize,end:usize,env_name:&str,blocks:&mut Vec<Value>|{
        let body=body.trim();
        if body.is_empty(){return;}
        let index=blocks.len()+1;
        blocks.push(json!({"id":format!("__SOURCE__-tex-b{index}"),"kind":kind,"order":index-1,"text":body,
            "plain_text":plain_text(body),"locator":{"part":name,"line_start":start,"line_end":end},"environment":env_name}));
    };
    let flush_bib=|bib:&mut Option<(String,usize,String)>,end_line:usize,blocks:&mut Vec<Value>,references:&mut Vec<Value>|{
        if let Some((key,start,body))=bib.take(){
            let trimmed=body.trim();
            if !trimmed.is_empty(){
                push("reference",trimmed,start,end_line,"bibitem",blocks);
                references.push(json!({"key":key,"line_start":start,"line_end":end_line,"raw":trimmed}));
            }
        }
    };
    let begin=text.find("\\begin{document}").map(|i|i+"\\begin{document}".len());
    let body_start=begin.unwrap_or(0);
    let body_end=begin.and_then(|at|text[at..].find("\\end{document}").map(|i|at+i)).unwrap_or(text.len());
    let preamble=&text[..begin.map(|i|i-"\\begin{document}".len()).unwrap_or(0)];
    for (index,raw) in preamble.lines().enumerate(){
        let clean=strip_comment(raw);let trimmed=clean.trim();
        if trimmed.contains("\\title")||trimmed.contains("\\author")||trimmed.contains("\\date"){
            push("metadata",raw,index+1,index+1,"preamble",&mut blocks);
        }
    }
    let body_start_line=text[..body_start].lines().count().max(1);
    for (index,raw) in text[body_start..body_end].lines().enumerate(){
        let line_no=body_start_line+index;let line=strip_comment(raw);let trimmed=line.trim();
        if trimmed.starts_with("\\begin{thebibliography}"){
            if !paragraph.trim().is_empty(){push("paragraph",&paragraph,paragraph_start,line_no.saturating_sub(1),"",&mut blocks);paragraph.clear();}
            in_bib=true;paragraph_start=line_no+1;continue;
        }
        if trimmed.contains("\\end{thebibliography}"){
            flush_bib(&mut bib_entry,line_no.saturating_sub(1),&mut blocks,&mut references);
            in_bib=false;paragraph_start=line_no+1;continue;
        }
        if let Some(name)=env_name(trimmed,"\\begin{"){
            if env.is_none()&&(name=="abstract"||MATH_ENVS.contains(&name.as_str())||TABLE_ENVS.contains(&name.as_str())||FIGURE_ENVS.contains(&name.as_str())){
                if !paragraph.trim().is_empty(){push("paragraph",&paragraph,paragraph_start,line_no.saturating_sub(1),"",&mut blocks);paragraph.clear();}
                flush_bib(&mut bib_entry,line_no.saturating_sub(1),&mut blocks,&mut references);
                env=Some((name.clone(),line_no,String::new()));
            }
        }
        if math_at.is_none()&&env.is_none(){
            if trimmed.starts_with("\\[")||trimmed.starts_with("$$"){
                if !paragraph.trim().is_empty(){push("paragraph",&paragraph,paragraph_start,line_no.saturating_sub(1),"",&mut blocks);paragraph.clear();}
                let marker=if trimmed.starts_with("\\["){"bracket"}else{"dollar"};
                math_at=Some((marker.to_owned(),line_no,String::new()));
            }
        }
        if let Some((name,start,body))=env.as_mut(){
            body.push_str(raw);body.push('\n');
            if trimmed.contains(&format!("\\end{{{name}}}")){
                let env_name=name.clone();let start=*start;let body=std::mem::take(body);env=None;
                let k=if env_name=="abstract"{"abstract"}else{kind(&env_name,"")};
                push(k,&body,start,line_no,&env_name,&mut blocks);
                let record=json!({"kind":k,"environment":env_name,"line_start":start,"line_end":line_no,"raw":body.trim()});
                match k{
                    "equation"=>equations.push(record),
                    "table"=>tables.push(record),
                    "figure"=>{
                        if let Some(g)=command_arg(body.trim(),"\\includegraphics"){graphics.insert(g.to_owned());}
                        figures.push(record);
                    }
                    _=>{}
                }
                paragraph_start=line_no+1;continue;
            }
            continue;
        }
        if let Some((marker,start,body))=math_at.as_mut(){
            body.push_str(raw);body.push('\n');
            let close=marker.split('@').next().unwrap();
            let end_marker=if close=="dollar"{"$$"}else{"\\]"};
            let closes=if line_no==*start{
                let after=trimmed.find(end_marker).map(|i|i+end_marker.len()).unwrap_or(0);
                after>0&&trimmed[after..].trim().is_empty()
            }else{trimmed.contains(end_marker)};
            if closes{
                let marker=marker.clone();let start=*start;let body=std::mem::take(body);math_at=None;
                push("equation",&body,start,line_no,&marker,&mut blocks);
                equations.push(json!({"kind":"equation","environment":marker,"line_start":start,"line_end":line_no,"raw":body.trim()}));
                paragraph_start=line_no+1;continue;
            }
            continue;
        }
        if trimmed.starts_with("\\bibitem"){
            if !paragraph.trim().is_empty(){push("paragraph",&paragraph,paragraph_start,line_no.saturating_sub(1),"",&mut blocks);paragraph.clear();}
            flush_bib(&mut bib_entry,line_no.saturating_sub(1),&mut blocks,&mut references);
            let key=command_arg(trimmed,"\\bibitem").unwrap_or("").to_owned();
            bib_entry=Some((key,line_no,format!("{raw}\n")));
            paragraph_start=line_no+1;continue;
        }
        if let Some((_,_,ref mut body))=bib_entry.as_mut(){
            if trimmed.is_empty(){
                flush_bib(&mut bib_entry,line_no.saturating_sub(1),&mut blocks,&mut references);
            }else{
                body.push_str(raw);body.push('\n');
            }
            continue;
        }
        let _=in_bib;
        if trimmed.is_empty(){
            if !paragraph.trim().is_empty(){push("paragraph",&paragraph,paragraph_start,line_no.saturating_sub(1),"",&mut blocks);paragraph.clear();}
            paragraph_start=line_no+1;continue;
        }
        if trimmed.starts_with("\\section")||trimmed.starts_with("\\subsection")||trimmed.starts_with("\\subsubsection")||trimmed.starts_with("\\chapter"){
            if !paragraph.trim().is_empty(){push("paragraph",&paragraph,paragraph_start,line_no.saturating_sub(1),"",&mut blocks);paragraph.clear();}
            push("section",raw,line_no,line_no,"",&mut blocks);paragraph_start=line_no+1;continue;
        }
        for command in ["\\cite","\\citep","\\citet","\\parencite"]{
            if let Some(arg)=command_arg(&line,command){
                for key in arg.split(','){let key=key.trim();if !key.is_empty(){citations.insert(key.to_owned());}}
            }
        }
        if let Some(arg)=command_arg(&line,"\\label"){labels.insert(arg.to_owned());}
        if let Some(arg)=command_arg(&line,"\\includegraphics"){graphics.insert(arg.to_owned());}
        if paragraph.is_empty(){paragraph_start=line_no;}
        paragraph.push_str(raw);paragraph.push('\n');
    }
    flush_bib(&mut bib_entry,text.lines().count(),&mut blocks,&mut references);
    if !paragraph.trim().is_empty(){push("paragraph",&paragraph,paragraph_start,text.lines().count(),"",&mut blocks);}
    Ok(json!({"schema_version":1,"format":"tex","part":name,"blocks":blocks,
        "equations":equations,"tables":tables,"figures":figures,"references":references,
        "citations":citations,"labels":labels,"graphics":graphics,
        "source_fidelity":"not_assessed",
        "boundary":"TeX text is source-authored evidence for equations, tables and references. It does not prove the compiled PDF layout, figure graphics, page order or that the archive is the version of record."}))
}
fn adapt_ids(mut parsed:Value,id:&str)->Value{
    if let Some(rows)=parsed["blocks"].as_array_mut(){
        for row in rows{
            let n=row["id"].as_str().and_then(|s|s.rsplit("-tex-b").next()).unwrap_or("0");
            row["id"]=json!(format!("{id}-tex-b{n}"));
        }
    }
    parsed
}
pub fn inspect(source:&Path,id:&str,expected:Option<&str>)->Result<Value>{
    ensure!(id.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)&&
        id.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-')),"Invalid source ID");
    let (p,raw,binding)=read_bound(source,expected)?;
    let mut files=source_files(p.file_name().and_then(|s|s.to_str()).context("TeX filename")?,&raw)?;
    ensure!(!files.is_empty(),"TeX source contains no readable text files");
    let main=main_file(&files)?;
    let mut seen=BTreeSet::new();
    let expanded=expand_inputs(&main,&files[&main],&files,&mut seen,0)?;
    files.insert(main.clone(),expanded.clone());
    let mut parsed=parse(&main,&expanded)?;
    parsed=adapt_ids(parsed,id);
    parsed["source_file"]=binding;
    parsed["archive_files"]=json!(files.keys().collect::<Vec<_>>());
    parsed["main_file"]=json!(main);
    parsed["expanded_inputs"]=json!(seen.into_iter().collect::<Vec<_>>());
    Ok(parsed)
}
fn render_clean_markdown(parsed:&Value)->String{
    let mut out=String::from("<!-- Derived from TeX source for reading/translation aid. Verify visible page order, headers & figures against PDF renders. -->\n\n");
    let mut refs_header=false;
    for b in parsed["blocks"].as_array().into_iter().flatten(){
        let id=b["id"].as_str().unwrap_or("");
        let kind=b["kind"].as_str().unwrap_or("paragraph");
        let text=b["text"].as_str().unwrap_or("").trim();
        if text.is_empty(){continue;}
        match kind{
            "metadata"=>{
                if let Some(t)=command_arg(text,"\\title"){
                    out.push_str(&format!("# {t}\n\n"));
                }else{
                    out.push_str(&format!("<!-- {id} -->\n{text}\n\n"));
                }
            }
            "abstract"=>{
                out.push_str(&format!("## Abstract <!-- {id} -->\n\n{text}\n\n"));
            }
            "section"=>{
                let prefix=if text.starts_with("\\subsubsection"){"####"}
                    else if text.starts_with("\\subsection"){"###"}
                    else{"##"};
                let heading=command_arg(text,"\\subsubsection*")
                    .or_else(||command_arg(text,"\\subsubsection"))
                    .or_else(||command_arg(text,"\\subsection*"))
                    .or_else(||command_arg(text,"\\subsection"))
                    .or_else(||command_arg(text,"\\section*"))
                    .or_else(||command_arg(text,"\\section"))
                    .or_else(||command_arg(text,"\\chapter*"))
                    .or_else(||command_arg(text,"\\chapter"))
                    .unwrap_or(text);
                out.push_str(&format!("{prefix} {heading} <!-- {id} -->\n\n"));
            }
            "equation"|"table"|"figure"=>{
                out.push_str(&format!("<!-- {id} ({kind}) -->\n{text}\n\n"));
            }
            "reference"=>{
                if !refs_header{out.push_str("## References\n\n");refs_header=true;}
                let single=text.lines().map(str::trim).filter(|s|!s.is_empty()).collect::<Vec<_>>().join(" ");
                out.push_str(&format!("- <!-- {id} --> {single}\n"));
            }
            _=>{
                let joined=text.lines().map(str::trim_end).collect::<Vec<_>>().join(" ");
                out.push_str(&format!("<!-- {id} --> {joined}\n\n"));
            }
        }
    }
    out
}
pub fn detect_arxiv_id(value:&Value,extra_text:Option<&str>)->Option<String>{
    static PREFIXED:OnceLock<Regex>=OnceLock::new();
    static BARE:OnceLock<Regex>=OnceLock::new();
    let prefixed=PREFIXED.get_or_init(||Regex::new(r"(?i)(?:arxiv\s*[:./]\s*|arxiv\.org/(?:abs|pdf|e-print)/)([0-9]{4}\.[0-9]{4,5}(?:v[0-9]+)?|[a-z-]+(?:\.[A-Z]{2})?/[0-9]{7}(?:v[0-9]+)?)").unwrap());
    let bare=BARE.get_or_init(||Regex::new(r"^([0-9]{4}\.[0-9]{4,5}(?:v[0-9]+)?|[a-z-]+(?:\.[A-Z]{2})?/[0-9]{7}(?:v[0-9]+)?)$").unwrap());
    let check_str=|s:&str,allow_bare:bool|->Option<String>{
        if let Some(c)=prefixed.captures(s){
            let id=c.get(1)?.as_str().trim_end_matches(".pdf");
            if id.len()<=40{return Some(id.to_owned());}
        }
        if allow_bare{
            let t=s.trim().trim_end_matches(".pdf");
            if t.len()<=40 && bare.is_match(t){return Some(t.to_owned());}
        }
        None
    };
    if let Some(s)=value.as_str(){
        if let Some(id)=check_str(s,true){return Some(id);}
    }
    let data=if value.get("raw").and_then(|r|r.get("data")).is_some(){&value["raw"]["data"]}
        else if value.get("data").is_some(){&value["data"]}
        else{value};
    for key in ["DOI","doi","url","archiveID","extra","libraryCatalog"]{
        if let Some(s)=data.get(key).and_then(Value::as_str){
            if let Some(id)=check_str(s,key=="archiveID"){return Some(id);}
        }
    }
    if let Some(t)=extra_text{
        if let Some(id)=check_str(t,false){return Some(id);}
    }
    fn walk(v:&Value,f:&dyn Fn(&str,bool)->Option<String>)->Option<String>{
        match v{
            Value::String(s)=>f(s,false),
            Value::Array(a)=>a.iter().find_map(|x|walk(x,f)),
            Value::Object(m)=>m.values().find_map(|x|walk(x,f)),
            _=>None,
        }
    }
    walk(value,&check_str)
}
pub fn summarize_prepared(parsed:&Value,arxiv_id:Option<&str>)->Value{
    json!({
        "schema_version":1,
        "status":"ok",
        "format":"tex",
        "arxiv_id":arxiv_id,
        "main_file":parsed["main_file"],
        "source_file":parsed["source_file"],
        "clean_markdown":parsed.get("clean_markdown").cloned().unwrap_or(Value::Null),
        "structure_json":parsed.get("structure_json").cloned().unwrap_or(Value::Null),
        "native_text":parsed.get("native_text").cloned().unwrap_or(Value::Null),
        "native_document":parsed.get("native_document").cloned().unwrap_or(Value::Null),
        "counts":{
            "blocks":parsed["blocks"].as_array().map_or(0,Vec::len),
            "equations":parsed["equations"].as_array().map_or(0,Vec::len),
            "tables":parsed["tables"].as_array().map_or(0,Vec::len),
            "figures":parsed["figures"].as_array().map_or(0,Vec::len),
            "references":parsed["references"].as_array().map_or(0,Vec::len),
            "citations":parsed["citations"].as_array().map_or(0,Vec::len),
            "graphics":parsed["graphics"].as_array().map_or(0,Vec::len),
        },
        "source_fidelity":parsed["source_fidelity"],
        "boundary":parsed["boundary"],
    })
}
pub fn fetch_and_prepare(target:&Value,destination:&Path,source_id:&str,pdf_path:Option<&Path>,timeout_seconds:u64,compact:bool)->Result<Value>{
    ensure!(source_id.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)&&
        source_id.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-')),"Invalid source ID");
    ensure!(!destination.is_symlink(),"Refusing a symlink TeX evidence directory");
    let mut arxiv_id=detect_arxiv_id(target,None);
    if arxiv_id.is_none(){
        if let Some(pdf)=pdf_path{
            if let Ok(mut b)=crate::pdfcraft_backend::Backend::open(pdf,None){
                if let Ok(t)=b.text(1){
                    arxiv_id=detect_arxiv_id(&Value::Null,t["plain_text"].as_str());
                }
            }
        }
    }
    let Some(arxiv_id)=arxiv_id else{
        return Ok(json!({"schema_version":1,"status":"not_arxiv","fallback":"pdf_layout"}));
    };
    static SAFE_ID:OnceLock<Regex>=OnceLock::new();
    let safe_rx=SAFE_ID.get_or_init(||Regex::new(r"^(?:[0-9]{4}\.[0-9]{4,5}(?:v[0-9]+)?|[a-z-]+(?:\.[A-Z]{2})?/[0-9]{7}(?:v[0-9]+)?)$").unwrap());
    ensure!(arxiv_id.len()<=40&&safe_rx.is_match(&arxiv_id),"Invalid or unsafe arXiv ID: {arxiv_id}");
    fs::create_dir_all(destination)?;
    let dir=files::canonical(destination)?;
    let structure_path=dir.join("tex-structure.json");
    let clean_path=dir.join("clean-source.md");
    if structure_path.is_file()&&clean_path.is_file(){
        let mut loaded=crate::pipeline::read_json(&structure_path)?;
        loaded["structure_json"]=check_ledger::bind_file(&structure_path)?;
        loaded["clean_markdown"]=check_ledger::bind_file(&clean_path)?;
        return Ok(if compact{summarize_prepared(&loaded,Some(&arxiv_id))}else{loaded});
    }
    let safe_file=arxiv_id.replace('/',"_");
    let archive_path=dir.join(format!("arxiv-{safe_file}.eprint"));
    if !archive_path.is_file(){
        let url=format!("https://arxiv.org/e-print/{arxiv_id}");
        let tmp=tempfile::Builder::new().prefix(".pending-arxiv-").tempfile_in(&dir)?;
        let tmp_path=tmp.path().to_path_buf();
        drop(tmp);
        let timeout=timeout_seconds.clamp(5,300).to_string();
        let curl=if cfg!(windows){"curl.exe"}else{"curl"};
        let output=Command::new(curl)
            .args(["--silent","--show-error","--fail","--location","--max-redirs","5","--max-time",&timeout,
                "--max-filesize","67108864","-A","z2o-rust/2.0","-o"])
            .arg(&tmp_path).arg(&url).output();
        let ok=match output{
            Ok(out) if out.status.success()=>true,
            Ok(out)=>{
                let _=fs::remove_file(&tmp_path);
                let msg=String::from_utf8_lossy(&out.stderr).trim().to_owned();
                return Ok(json!({"schema_version":1,"status":"unavailable","arxiv_id":arxiv_id,"reason":msg,"fallback":"pdf_layout"}));
            }
            Err(e)=>{
                let _=fs::remove_file(&tmp_path);
                return Ok(json!({"schema_version":1,"status":"unavailable","arxiv_id":arxiv_id,"reason":format!("{e:#}"),"fallback":"pdf_layout"}));
            }
        };
        if ok{
            let head=fs::read(&tmp_path).unwrap_or_default();
            let prefix=String::from_utf8_lossy(&head[..head.len().min(64)]).trim_start().to_ascii_lowercase();
            if head.starts_with(b"%PDF-")||prefix.starts_with("<!doctype")||prefix.starts_with("<html")||head.is_empty(){
                let _=fs::remove_file(&tmp_path);
                return Ok(json!({"schema_version":1,"status":"unavailable","arxiv_id":arxiv_id,
                    "reason":"arXiv returned PDF, HTML or empty payload instead of TeX source","fallback":"pdf_layout"}));
            }
            if let Err(e)=fs::rename(&tmp_path,&archive_path){
                let _=fs::remove_file(&tmp_path);
                return Err(e.into());
            }
        }
    }
    match prepare(&archive_path,source_id,&dir,None){
        Ok(parsed)=>Ok(if compact{summarize_prepared(&parsed,Some(&arxiv_id))}else{parsed}),
        Err(e)=>Ok(json!({"schema_version":1,"status":"unavailable","arxiv_id":arxiv_id,
            "archive":check_ledger::bind_file(&archive_path).ok(),
            "reason":format!("{e:#}"),"fallback":"pdf_layout"})),
    }
}
pub fn prepare(source:&Path,id:&str,directory:&Path,expected:Option<&str>)->Result<Value>{
    let mut parsed=inspect(source,id,expected)?;
    let source_binding=parsed["source_file"].clone();
    ensure!(!directory.is_symlink(),"Refusing a symlink TeX evidence directory");
    fs::create_dir_all(directory)?;
    let directory=files::canonical(directory)?;
    let raw=fs::read(path_text_source(&source_binding)?)?;
    let files=source_files(path_text_source(&source_binding)?.file_name().and_then(|s|s.to_str()).context("TeX filename")?,&raw)?;
    let mut text=String::new();
    for row in parsed["blocks"].as_array().context("TeX blocks")?{
        text+=&format!("[{} | {} | L{}-{}]\n{}\n\n",row["id"].as_str().unwrap(),row["kind"].as_str().unwrap(),
            row["locator"]["line_start"],row["locator"]["line_end"],row["text"].as_str().unwrap());
    }
    let clean=render_clean_markdown(&parsed);
    let mut source_tex=Vec::new();
    for(name,body)in &files{source_tex.extend_from_slice(format!("%% ==== {name} ====\n{body}\n\n").as_bytes());}
    parsed["native_document"]=write_new(&directory,"source.tex",&source_tex)?;
    parsed["native_text"]=write_new(&directory,"native-blocks.txt",text.as_bytes())?;
    parsed["clean_markdown"]=write_new(&directory,"clean-source.md",clean.as_bytes())?;
    let mut json_bytes=serde_json::to_vec_pretty(&parsed)?;json_bytes.push(b'\n');
    parsed["structure_json"]=write_new(&directory,"tex-structure.json",&json_bytes)?;
    Ok(parsed)
}
fn path_text_source(binding:&Value)->Result<PathBuf>{
    crate::path(&binding["path"])
}
fn write_new(directory:&Path,name:&str,bytes:&[u8])->Result<Value>{
    let p=directory.join(name);
    ensure!(!p.exists()&&!p.is_symlink(),"Keep existing TeX evidence: {}",p.display());
    let mut t=tempfile::Builder::new().prefix(".pending-tex-").tempfile_in(directory)?;
    t.write_all(bytes)?;t.as_file().sync_all()?;
    t.persist_noclobber(&p).map_err(|e|anyhow!("Cannot publish {}: {}",p.display(),e.error))?;
    let b=check_ledger::bind_file(&p)?;
    ensure!(b["sha256"]==files::digest(bytes),"TeX evidence changed after publication");Ok(b)
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn parses_sections_equations_tables_and_references(){
        let text=r#"\documentclass{article}
\begin{document}
\section{Introduction}
We cite \cite{alpha,beta} and see Fig.~\ref{fig:one}.

\begin{equation}\label{eq:one}
E = mc^2
\end{equation}

\begin{table}
\begin{tabular}{cc}
a & b \\
\end{tabular}
\end{table}

\begin{figure}
\includegraphics[width=\linewidth]{figs/one.pdf}
\end{figure}

\bibitem{alpha} Alpha.
\end{document}
"#;
        let p=parse("main.tex",text).unwrap();
        let kinds:Vec<_>=p["blocks"].as_array().unwrap().iter().map(|b|b["kind"].as_str().unwrap()).collect();
        assert!(kinds.contains(&"section"));
        assert!(kinds.contains(&"equation"));
        assert!(kinds.contains(&"table"));
        assert!(kinds.contains(&"figure"));
        assert_eq!(p["equations"].as_array().unwrap().len(),1);
        assert!(p["citations"].as_array().unwrap().contains(&json!("alpha")));
        assert!(p["graphics"].as_array().unwrap().contains(&json!("figs/one.pdf")));
        assert_eq!(p["blocks"][1]["locator"]["part"],"main.tex");
    }
    #[test]
    fn targz_source_extracts_main_tex_and_inputs(){
        let files=[("paper/main.tex",&b"\\documentclass{article}\\begin{document}\\input{part}\\end{document}"[..]),
            ("paper/part.tex",&b"\\section{Remote}\\begin{equation}a=b\\end{equation}"[..])];
        let mut tar=Vec::new();
        for(name,body)in files{
            let mut header=[0u8;512];
            header[..name.len()].copy_from_slice(name.as_bytes());
            let size=format!("{:011o}",body.len());
            header[124..124+size.len()].copy_from_slice(size.as_bytes());
            header[156]=b'0';
            tar.extend_from_slice(&header);
            tar.extend_from_slice(body);
            tar.resize(tar.len().div_ceil(512)*512,0);
        }
        tar.extend_from_slice(&[0u8;1024]);
        let mut gz=Vec::new();
        let mut enc=flate2::write::GzEncoder::new(&mut gz,flate2::Compression::fast());
        enc.write_all(&tar).unwrap();enc.finish().unwrap();
        let dir=tempfile::tempdir().unwrap();let p=dir.path().join("source.tar.gz");fs::write(&p,gz).unwrap();
        let r=inspect(&p,"arxiv",None).unwrap();
        assert_eq!(r["main_file"],"paper/main.tex");
        assert!(r["blocks"].as_array().unwrap().iter().any(|b|b["text"].as_str().unwrap().contains("Remote")));
    }
    #[test]
    fn detects_arxiv_id_and_expands_bbl_and_plain_gz(){
        let item=json!({"raw":{"data":{"DOI":"10.48550/arXiv.2611.26982","url":"https://arxiv.org/abs/2611.26982v1"}}});
        assert_eq!(detect_arxiv_id(&item,None).as_deref(),Some("2611.26982"));
        assert_eq!(detect_arxiv_id(&json!("2501.01234v2"),None).as_deref(),Some("2501.01234v2"));
        assert_eq!(detect_arxiv_id(&json!({"title":"No arxiv"}),Some("preprint header")),None);

        let tex=concat!(
            "\\documentclass{article}\\title{Sample Paper}\n",
            "\\begin{document}\n",
            "\\begin{abstract}First sentence.\nSecond sentence.\\end{abstract}\n",
            "\\section{Intro}We compare methods~\\cite{refA}.\n",
            "\\section*{Code Availability}Public repo.\n",
            "\\bibliography{sample}\n",
            "\\end{document}\n"
        );
        let mut gz=Vec::new();
        let mut enc=flate2::write::GzEncoder::new(&mut gz,flate2::Compression::fast());
        enc.write_all(tex.as_bytes()).unwrap();enc.finish().unwrap();
        let dir=tempfile::tempdir().unwrap();
        let p=dir.path().join("arxiv-2611.26982.eprint");
        fs::write(&p,gz).unwrap();
        let prep=prepare(&p,"main",&dir.path().join("out"),None).unwrap();
        assert!(prep["clean_markdown"]["path"].as_str().unwrap().ends_with("clean-source.md"));
        let md=fs::read_to_string(prep["clean_markdown"]["path"].as_str().unwrap()).unwrap();
        assert!(md.contains("# Sample Paper")&&md.contains("## Abstract")&&md.contains("## Intro"));
        assert!(md.contains("## Code Availability")&&!md.contains("\\section*{Code Availability}"),"{md}");

        // .tar.gz with .bbl expansion & multi-line \bibitem
        let bbl="\\begin{thebibliography}{1}\n\\bibitem[1]{refA} Author A.\nJournal 2025.\n\\end{thebibliography}\n";
        let mut tar=Vec::new();
        for (name,body) in [("main.tex",tex.as_bytes()),("main.bbl",bbl.as_bytes())]{
            let mut h=[0u8;512];h[..name.len()].copy_from_slice(name.as_bytes());
            let sz=format!("{:011o}",body.len());h[124..124+sz.len()].copy_from_slice(sz.as_bytes());h[156]=b'0';
            tar.extend_from_slice(&h);tar.extend_from_slice(body);tar.resize(tar.len().div_ceil(512)*512,0);
        }
        tar.extend_from_slice(&[0u8;1024]);
        let mut gz2=Vec::new();
        let mut enc2=flate2::write::GzEncoder::new(&mut gz2,flate2::Compression::fast());
        enc2.write_all(&tar).unwrap();enc2.finish().unwrap();
        let p2=dir.path().join("bundle.eprint");fs::write(&p2,gz2).unwrap();
        let out2=dir.path().join("out2");
        let prep2=prepare(&p2,"main",&out2,None).unwrap();
        assert_eq!(prep2["references"].as_array().unwrap().len(),1);
        assert!(prep2["references"][0]["raw"].as_str().unwrap().contains("Journal 2025"));

        // Idempotent offline fetch reuse when already prepared
        let cached=fetch_and_prepare(&json!("2611.26982"),&out2,"main",None,10,true).unwrap();
        assert_eq!(cached["status"],"ok");
        assert_eq!(cached["counts"]["references"],1);
    }
    #[test]
    fn decodes_gbk_tex_and_zip_sources_without_mojibake(){
        use zip::write::SimpleFileOptions;
        let dir=tempfile::tempdir().unwrap();
        let src="\\documentclass{article}\n\\begin{document}\n\\section{海洋内波动力学的非线性数值模拟}\n本文研究不同层结下内孤立波的能量耗散特征。\n\\end{document}\n";
        let (gbk,_,err)=encoding_rs::GB18030.encode(src);
        assert!(!err);

        let tex_path=dir.path().join("gbk-paper.tex");
        fs::write(&tex_path,&gbk).unwrap();
        let r=inspect(&tex_path,"gbk",None).unwrap();
        let joined:String=r["blocks"].as_array().unwrap().iter().filter_map(|b|b["text"].as_str()).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("海洋内波动力学的非线性数值模拟"),"{joined}");
        assert!(!crate::mojibake::analyze_text(&joined).is_garbled);

        let zip_path=dir.path().join("gbk-paper.zip");
        {
            let f=fs::File::create(&zip_path).unwrap();
            let mut z=zip::ZipWriter::new(f);
            z.start_file("main.tex",SimpleFileOptions::default()).unwrap();
            z.write_all(&gbk).unwrap();
            z.finish().unwrap();
        }
        let rz=inspect(&zip_path,"gbkzip",None).unwrap();
        let zjoined:String=rz["blocks"].as_array().unwrap().iter().filter_map(|b|b["text"].as_str()).collect::<Vec<_>>().join("\n");
        assert!(zjoined.contains("内孤立波的能量耗散特征"),"{zjoined}");
        assert!(!crate::mojibake::analyze_text(&zjoined).is_garbled);
    }
}
