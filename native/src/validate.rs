//! Read-only Markdown structural validator. Findings concern static source
//! syntax and local anchors, never translation or application rendering.
use crate::{syntax,yaml_nodes};
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::{collections::{HashMap,HashSet},fs,path::Path,sync::OnceLock};

macro_rules! rx {($s:expr)=>{{static R:OnceLock<Regex>=OnceLock::new();R.get_or_init(||Regex::new($s).expect("static regex"))}}}
#[derive(Clone,Debug,Serialize,PartialEq,Eq)]
pub struct Finding{pub level:&'static str,pub code:&'static str,pub line:usize,pub message:String}
fn finding(out:&mut Vec<Finding>,level:&'static str,code:&'static str,line:usize,message:impl Into<String>){
    out.push(Finding{level,code,line,message:message.into()});
}
#[derive(Clone,Debug,Serialize)]
pub struct TableRecord{pub line:usize,pub closing_line:Option<usize>,pub content:Vec<String>,pub anchor:Option<String>,pub anchor_line:Option<usize>}
#[derive(Debug,Serialize)]
pub struct Inspection{pub findings:Vec<Finding>,pub table_records:Vec<TableRecord>}
pub(crate) fn anchor(line:&str)->Option<&str>{
    rx!(r"(?:^|\s)\^([A-Za-z0-9-]+)[ \t]*$").captures(line)?.get(1).map(|m|m.as_str())
}
pub(crate) fn whole_anchor(line:&str)->Option<&str>{
    rx!(r"^\^([A-Za-z0-9-]+)[ \t]*$").captures(line)?.get(1).map(|m|m.as_str())
}
pub(crate) fn heading(line:&str)->Option<(usize,&str)>{
    let c=rx!(r"^ {0,3}(#{1,6})[ \t]+(.+?)[ \t]*#*[ \t]*$").captures(line)?;
    Some((c.get(1)?.as_str().len(),c.get(2)?.as_str()))
}
pub(crate) fn entry(line:&str)->Option<(&str,usize)>{
    let c=rx!(r"^ {0,3}([1-9][0-9]*)\.[ \t]+").captures(line)?;
    Some((c.get(1)?.as_str(),c.get(0)?.end()))
}
pub(crate) fn fence(line:&str)->Option<(u8,usize,&str)>{
    let c=rx!(r"^ {0,3}(\x60{3,}|~{3,})(.*)$").captures(line)?;
    let mark=c.get(1)?.as_str();let info=c.get(2)?.as_str();
    if mark.starts_with('\x60')&&info.contains('\x60'){return None;}
    Some((mark.as_bytes()[0],mark.len(),info.trim()))
}
pub(crate) fn closing(line:&str,mark:u8,width:usize)->bool{
    let trimmed=line.trim_start_matches(' ');
    let count=trimmed.bytes().take_while(|b|*b==mark).count();
    line.len()-trimmed.len()<=3&&count>=width&&trimmed[count..].bytes().all(|b|matches!(b,b' '|b'\t'))
}
pub(crate) fn blank(text:&str)->String{text.chars().map(|c|if c=='\n'{'\n'}else{' '}).collect()}
pub(crate) fn mask_delimited(text:&str,character:char)->String{
    let runs:Vec<_>=(if character=='$'{rx!(r"\$+")}else{rx!(r"\x60+")}).find_iter(text).filter(|m|!syntax::escaped(text,m.start())).collect();
    let mut next=HashMap::new();let mut pairs=HashMap::new();
    for i in (0..runs.len()).rev(){
        let width=runs[i].as_str().len();
        if let Some(j)=next.insert(width,i){pairs.insert(i,j);}
    }
    let mut out=String::with_capacity(text.len());let mut cursor=0;let mut index=0;
    while index<runs.len(){
        let Some(&j)=pairs.get(&index)else{index+=1;continue;};
        out.push_str(&text[cursor..runs[index].start()]);out.push_str(&blank(&text[runs[index].start()..runs[j].end()]));
        cursor=runs[j].end();index=j+1;
    }
    out.push_str(&text[cursor..]);out
}
pub(crate) fn mask_paragraph_code(text:&str,mut active:Option<&'static str>)->String{
    let tokens:Vec<_>=rx!(r"<!--|%%|\x60+").find_iter(text).collect();
    let mut next=HashMap::new();let mut pairs=HashMap::new();
    for i in (0..tokens.len()).rev(){
        let t=tokens[i];
        if !t.as_str().starts_with('\x60')||syntax::escaped(text,t.start()){continue;}
        if let Some(j)=next.insert(t.as_str().len(),i){pairs.insert(i,j);}
    }
    let mut out=String::with_capacity(text.len());let mut cursor=0;let mut i=0;
    while i<tokens.len(){
        let token=tokens[i];
        if let Some(comment)=active{
            let Some(end)=text[token.start()..].find(comment).map(|p|token.start()+p+comment.len())else{break;};
            while i<tokens.len()&&tokens[i].start()<end{i+=1;}active=None;continue;
        }
        if syntax::escaped(text,token.start()){i+=1;continue;}
        if matches!(token.as_str(),"<!--"|"%%"){
            let comment=if token.as_str()=="<!--"{"-->"}else{"%%"};
            let Some(end)=text[token.end()..].find(comment).map(|p|token.end()+p+comment.len())else{break;};
            while i<tokens.len()&&tokens[i].start()<end{i+=1;}continue;
        }
        let Some(&j)=pairs.get(&i)else{i+=1;continue;};
        let end=tokens[j].end();out.push_str(&text[cursor..token.start()]);out.push_str(&blank(&text[token.start()..end]));
        cursor=end;i=j+1;
    }
    out.push_str(&text[cursor..]);out
}
pub(crate) fn mask_comments(line:&str,mut active:Option<&'static str>)->(String,Option<&'static str>){
    let mut out=String::with_capacity(line.len());let mut pos=0;
    while pos<line.len(){
        if let Some(comment)=active{
            if let Some(end)=line[pos..].find(comment).map(|i|pos+i+comment.len()){
                out.push_str(&blank(&line[pos..end]));pos=end;active=None;continue;
            }
            out.push_str(&blank(&line[pos..]));break;
        }
        if !syntax::escaped(line,pos)&&line[pos..].starts_with("<!--"){
            out.push_str("    ");active=Some("-->");pos+=4;continue;
        }
        if !syntax::escaped(line,pos)&&line[pos..].starts_with("%%"){
            out.push_str("  ");active=Some("%%");pos+=2;continue;
        }
        let c=line[pos..].chars().next().unwrap();out.push(c);pos+=c.len_utf8();
    }(out,active)
}
fn markdown_local()->&'static Regex{rx!(r#"\[(?:\\.|[^\[\]\\\n])*\]\([ \t]*(?:<#\^(?P<angle>[A-Za-z0-9-]+)>|#\^(?P<bare>[A-Za-z0-9-]+))(?:[ \t]+(?:"[^"\n]*"|'[^'\n]*'|\([^\n)]*\)))?[ \t]*\)"#)}
fn wiki_local()->&'static Regex{rx!(r"\[\[#\^(?P<id>[A-Za-z0-9-]+)(?:\|[^\]\n]*)?\]\]")}
pub(crate) fn links(text:&str,line:usize)->Vec<(String,usize)>{
    let visible=mask_delimited(&mask_delimited(text,'\x60'),'$');let mut result=Vec::new();
    for pattern in [markdown_local(),wiki_local()]{
        for c in pattern.captures_iter(&visible){
            let m=c.get(0).unwrap();if syntax::escaped(&visible,m.start()){continue;}
            let target=c.name("angle").or_else(||c.name("bare")).or_else(||c.name("id"));
            if let Some(target)=target{
                result.push((target.as_str().to_owned(),line+visible[..m.start()].bytes().filter(|b|*b==b'\n').count()));
            }
        }
    }result
}
fn visible_caption(title:&str)->String{
    let mut comment=None;let mut lines=Vec::new();
    for line in mask_paragraph_code(title,None).split('\n'){
        let(clean,next)=mask_comments(line,comment);comment=next;lines.push(clean);
    }lines.join("\n")
}
pub fn rich_payload(content:&[String])->String{
    let mut rows=content.to_vec();let nonempty:Vec<_>=rows.iter().enumerate().filter(|(_,s)|!s.trim().is_empty()).map(|(i,_)|i).collect();
    if nonempty.len()>=2{
        let a=nonempty[0];let b=*nonempty.last().unwrap();
        let marker=rx!(r"^---[ \t]*(?:#.*)?$");
        if marker.is_match(&rows[a])&&marker.is_match(&rows[b]){rows[b].clear();}
    }rows.join("\n")
}
fn load_yaml(payload:&str,start:usize,code:&'static str,findings:&mut Vec<Finding>)->Option<yaml_nodes::Document>{
    match yaml_nodes::load(payload){
        Ok(doc)=>Some(doc),
        Err(e)=>{finding(findings,"error",code,start+e.line.saturating_sub(1),format!("YAML 无法安全解析：{}",e.message.lines().next().unwrap_or("invalid YAML")));None}
    }
}
pub(crate) fn frontmatter(lines:&[&str],findings:&mut Vec<Finding>)->usize{
    if lines.first().is_none_or(|l|l.trim()!="---"){return 0;}
    let Some(end)=lines.iter().enumerate().skip(1).find(|(_,l)|matches!(l.trim(),"---"|"...")).map(|(i,_)|i)else{
        finding(findings,"error","FRONTMATTER_UNCLOSED",1,"起始 YAML 未闭合。");return lines.len();
    };
    if let Some(doc)=load_yaml(&lines[1..end].join("\n"),2,"FRONTMATTER_YAML",findings){
        if !doc.value.is_null()&&!doc.value.is_object(){finding(findings,"error","FRONTMATTER_TYPE",1,"笔记属性必须是 YAML 映射。");}
        if doc.value.get("库内参考文献").is_some(){finding(findings,"error","DEPRECATED_PROPERTY",1,"属性中仍有废弃键“库内参考文献”。");}
    }end+1
}
type LocalLink=(String,usize,String);
fn strings<'a>(value:&'a Value,out:&mut Vec<&'a str>){
    let mut stack=vec![value];
    while let Some(v)=stack.pop(){
        match v{Value::String(s)=>out.push(s),Value::Array(a)=>stack.extend(a.iter().rev()),
            Value::Object(m)=>stack.extend(m.values()),_=>{}}
    }
}
fn table_visible_links(data:&Value,start:usize,context:&str,output:&mut Vec<LocalLink>){
    let mut fields=vec![&data["title"],&data["footer"]];
    if let Some(columns)=data["columns"].as_array(){
        for column in columns{if column.is_object(){fields.push(&column["name"]);}}
    }
    if let Some(rows)=data["rows"].as_array(){
        for row in rows{if row.is_object(){fields.push(&row["cells"]);}}
    }
    for field in fields{
        let mut values=vec![];strings(field,&mut values);
        for text in values{output.extend(links(text,1).into_iter().map(|(target,_)|(target,start,context.to_owned())));}
    }
}
fn caption_self_link(data:&Value,doc:&yaml_nodes::Document,node:Option<usize>,start:usize,context:&str,own:Option<&str>,out:&mut Vec<Finding>){
    let(Some(id),Some(title))=(own,data["title"].as_str())else{return;};
    if links(&visible_caption(title),1).iter().any(|(target,_)|target==id){
        let line=doc.line(doc.field(node,"title")).map(|n|start+n).unwrap_or(start);
        finding(out,"error","TABLE_CAPTION_SELF_LINK",line,format!("[LINK-C27] {context}：title 中的表题不得链接到自身 ^{id}；保留题注文字和原锚 ID。"));
    }
}
fn sheet(data:&Value,doc:&yaml_nodes::Document,node:Option<usize>,location:usize,start:usize,context:&str,own:Option<&str>,out:&mut Vec<Finding>,output:&mut Vec<LocalLink>){
    if data["locked"]!=true{finding(out,"error","TABLE_UNLOCKED",location,format!("{context}：locked 必须为 true。"));}
    if data["theme"]!="grid"{finding(out,"error","TABLE_THEME",location,format!("{context}：theme 必须为 grid。"));}
    for(key,rule,code)in [("columns","BODY-C138","TABLE_COLUMNS_FLOW_STYLE"),("rows","BODY-C139","TABLE_ROWS_FLOW_STYLE")]{
        if data[key].as_array().is_none_or(Vec::is_empty){finding(out,"error","TABLE_STRUCTURE",location,format!("{context}：{key} 必须为非空列表。"));}
        let field=doc.field(node,key);
        if doc.flow_sequence(field){
            let line=doc.line(field).map(|n|start+n).unwrap_or(start);
            finding(out,"error",code,line,format!("[{rule}] {context}：{key} 必须使用逐条展开的块状 YAML 序列；方括号 flow 数组虽可解析，仍不满足该源码要求。"));
        }
    }
    caption_self_link(data,doc,node,start,context,own,out);table_visible_links(data,location,context,output);
}
fn rich_table(content:&[String],start:usize,own:Option<&str>,out:&mut Vec<Finding>,output:&mut Vec<LocalLink>){
    let Some(doc)=load_yaml(&rich_payload(content),start+1,"TABLE_YAML",out)else{return;};
    let data=&doc.value;
    if !data.is_object(){finding(out,"error","TABLE_TYPE",start,"Rich Table 必须是 YAML 映射。");return;}
    let version=data["version"].as_u64();
    if !matches!(version,Some(2|3)){finding(out,"error","TABLE_VERSION",start,"Rich Table version 必须为整数 2（单表）或 3（多工作表）。");}
    if version!=Some(3){sheet(data,&doc,doc.root,start,start,"Rich Table",own,out,output);return;}
    let Some(sheets)=data["sheets"].as_array().filter(|v|!v.is_empty())else{
        finding(out,"error","TABLE_STRUCTURE",start,"Rich Table v3 的 sheets 必须为非空列表。");return;
    };
    table_visible_links(data,start,"Rich Table 工作簿",output);
    caption_self_link(data,&doc,doc.root,start,"Rich Table 工作簿",own,out);
    let nodes=doc.seq(doc.field(doc.root,"sheets")).unwrap_or(&[]);
    for(i,value)in sheets.iter().enumerate(){
        let node=nodes.get(i).copied();
        let location=doc.line(node).map(|n|start+n).unwrap_or(start);
        let mut context=format!("Rich Table 工作表 {}",i+1);
        if !value.is_object(){finding(out,"error","TABLE_SHEET_TYPE",location,format!("{context}必须是 YAML 映射。"));continue;}
        let labels:Vec<_>=["name","id"].iter().filter_map(|k|value[*k].as_str().filter(|s|!s.is_empty()).map(|s|format!("{k}={s}"))).collect();
        if !labels.is_empty(){context.push_str(&format!("（{}）",labels.join("，")));}
        sheet(value,&doc,node,location,start,&context,own,out,output);
    }
}
fn following_anchor(lines:&[&str],closing:usize,prefix:&str)->Option<(usize,String)>{
    let mut following=closing+1;
    while following<lines.len()&&lines[following].trim().is_empty(){following+=1;}
    let a=whole_anchor(lines.get(following)?)?;
    a.starts_with(prefix).then(||(following,a.to_owned()))
}
fn object_anchor(lines:&[&str],closing:usize,prefix:&str,out:&mut Vec<Finding>,required:bool){
    let Some((following,_))=following_anchor(lines,closing,prefix)else{
        if required{finding(out,"error","TABLE_ANCHOR",closing+1,"Rich Table 闭合围栏下一行须为顶格独占的 ^tbl-…。");}
        return;
    };
    if following!=closing+1{finding(out,"error","ANCHOR_GAP",following+1,"对象闭合行与其锚点之间不得有空行。");}
    if lines.get(following+1).is_some_and(|l|!l.trim().is_empty()){finding(out,"error","ANCHOR_SPACING",following+1,"对象锚点与后续内容之间须留一空行。");}
}
fn caption_figure(line:&str,id:&str,number:usize,out:&mut Vec<Finding>){
    if !links(line,number).iter().any(|(a,_)|a==id){return;}
    let label=markdown_local().replace_all(line,|c:&regex::Captures<'_>|c.get(0).unwrap().as_str()[1..].split("](").next().unwrap().to_owned());
    let label=wiki_local().replace_all(&label,|c:&regex::Captures<'_>|{
        let raw=c.get(0).unwrap().as_str();raw[2..raw.len()-2].split_once('|').map(|(_,s)|s).unwrap_or("").to_owned()
    });
    let mut plain=String::new();let mut pos=0;
    for m in rx!(r"\*\*|__").find_iter(&label){
        if m.start()>0&&label.as_bytes()[m.start()-1]==b'\\'{continue;}
        plain.push_str(&label[pos..m.start()]);pos=m.end();
    }
    plain.push_str(&label[pos..]);
    if rx!(r"^(?:图[^\s：:]+[：:]|图(?:形|文)摘要(?:\s|$)|(?:Figure|Fig\.)\s+[A-Za-z0-9])").is_match(plain.trim()){
        finding(out,"error","FIGURE_CAPTION_SELF_LINK",number,format!("[LINK-C27] 图题不得链接到自身 ^{id}；保留题注文字和原锚 ID。"));
    }
}
fn finish_entry(number:&str,first:usize,si:bool,rows:&[&str],out:&mut Vec<Finding>){
    let anchors:Vec<_>=rows.iter().filter_map(|l|anchor(l)).collect();
    if anchors.len()!=1{
        finding(out,"error","BIBLIOGRAPHY_ANCHOR",first,"每条书目须有且仅有一个末尾参考文献块 ID。");return;
    }
    if rows.iter().rev().find(|s|!s.trim().is_empty()).and_then(|l|anchor(l)).is_none(){
        finding(out,"error","BIBLIOGRAPHY_ANCHOR",first,"参考文献块 ID 须放在该书目条目的末尾。");return;
    }
    let id=anchors[0];
    let parsed=rx!(r"(?i)^(s?ref)-?([1-9][0-9]{0,8})$").captures(id).or_else(||rx!(r"(?i)^((?:s?ref)(?:-[A-Za-z][A-Za-z0-9]*)+)-([1-9][0-9]{0,8})$").captures(id));
    let Some(c)=parsed else{
        let recommended=if si{"sref-N 或 sref-src<source_token>-N"}else{"ref-N"};
        finding(out,"warning","BIBLIOGRAPHY_PLUGIN_COMPAT",first,format!("块 ID ^{id} 保留，但 Citation Preview 1.3.0 不识别此形式。新建书目采用 {recommended}；不要据此重命名已有锚点或改写链接。"));return;
    };
    if &c[2]!=number{
        finding(out,"warning","BIBLIOGRAPHY_NUMBER",first,format!("书目显示序号 {number} 与已有锚点 ^{id} 的数字不同；保留 ID 和链接，按书目身份及原有映射核对，不自动重编号。"));
    }
}
fn bibliography(lines:&[String],out:&mut Vec<Finding>){
    let mut supplement=false;let mut level=None;let mut current:Option<(&str,usize,bool)>=None;let mut rows:Vec<&str>=vec![];
    let finish=|current:Option<(&str,usize,bool)>,rows:&[&str],out:&mut Vec<Finding>|{
        if let Some((number,first,si))=current{finish_entry(number,first,si,rows,out);}
    };
    for(i,line)in lines.iter().enumerate(){
        if rx!(r"^ {0,3}(?:-{3,}|\*{3,}|_{3,})[ \t]*$").is_match(line){
            finish(current,&rows,out);current=None;rows.clear();level=None;continue;
        }
        if let Some((n,title))=heading(line){
            finish(current,&rows,out);current=None;rows.clear();
            if n<=2{supplement=rx!(r"^(?:[0-9]+[ \t]+)?补充材料$").is_match(title);}
            if level.is_some_and(|v|n<=v){level=None;}
            if rx!(r"^(?:[0-9]+(?:\.[0-9]+)*[ \t]+)?参考文献$").is_match(title){level=Some(n);}
            continue;
        }
        if level.is_none(){continue;}
        if let Some((number,_))=entry(line){
            finish(current,&rows,out);current=Some((number,i+1,supplement));rows.clear();rows.push(line);continue;
        }
        if current.is_some(){rows.push(line);}
    }finish(current,&rows,out);
}

pub(crate) fn indentation(line:&str)->usize{
    let mut n=0;
    for c in line.chars(){match c{' '=>n+=1,'\t'=>n+=4-n%4,_=>break,}}n
}
pub fn inspect(text:&str)->Inspection{
    let text=text.strip_prefix('\u{feff}').unwrap_or(text).replace("\r\n","\n").replace('\r',"\n");
    let lines:Vec<_>=text.split('\n').collect();let mut findings=vec![];let mut tables=vec![];
    let first=frontmatter(&lines,&mut findings);
    let mut visible=vec![String::new();lines.len()];let mut local_links:Vec<LocalLink>=vec![];
    let mut active_fence:Option<(u8,usize,&str,usize)>=None;let mut fence_content=vec![];
    let mut equation=None;let mut comment=None;let mut eq_closings=HashSet::new();let mut table_closings=HashSet::new();
    let mut code_lines:Vec<Option<String>>=vec![None;lines.len()];let mut list_indent:Option<usize>=None;
    // Post-comment fences can be derived from masked strings; own language
    // separately so the active state never borrows a temporary mask.
    for index in first..lines.len(){
        let raw=lines[index];
        if let Some((marker,width,language,opening))=active_fence{
            if closing(raw,marker,width){
                if language=="rich-table"{
                    table_closings.insert(index);
                    let own=following_anchor(&lines,index,"tbl-");
                    tables.push(TableRecord{line:opening+1,closing_line:Some(index+1),content:fence_content.clone(),
                        anchor:own.as_ref().map(|(_,s)|s.clone()),anchor_line:own.as_ref().map(|(i,_)|i+1)});
                    rich_table(&fence_content,opening+1,own.as_ref().map(|(_,s)|s.as_str()),&mut findings,&mut local_links);
                    object_anchor(&lines,index,"tbl-",&mut findings,true);
                }
                active_fence=None;fence_content.clear();continue;
            }
            fence_content.push(raw.to_owned());continue;
        }
        if equation.is_some(){
            if raw.trim()=="$$"{
                eq_closings.insert(index);object_anchor(&lines,index,"eq-",&mut findings,false);equation=None;
            }continue;
        }
        let indent=indentation(raw);let continuation=list_indent.is_some_and(|n|indent>=n);
        if !raw.trim().is_empty()&&!continuation{list_indent=None;}
        if comment.is_none()&&indent>=4&&!(continuation&&list_indent.is_some_and(|n|indent<n+4)){continue;}
        if comment.is_none(){if let Some((_,end))=entry(raw){list_indent=Some(end);}}
        if comment.is_none(){
            if let Some((marker,width,language))=fence(raw){active_fence=Some((marker,width,language,index));list_indent=None;continue;}
        }
        if code_lines[index].is_none(){
            let mut end=index+1;
            if !raw.trim().is_empty()&&heading(raw).is_none(){
                while end<lines.len()&&!lines[end].trim().is_empty()&&
                    fence(lines[end]).is_none()&&heading(lines[end]).is_none()&&entry(lines[end]).is_none(){end+=1;}
            }
            let masked=mask_paragraph_code(&lines[index..end].join("\n"),comment);
            for (i,line) in masked.split('\n').enumerate(){code_lines[index+i]=Some(line.to_owned());}
        }
        let code_safe=code_lines[index].as_deref().unwrap_or(raw);
        if comment.is_none()&&rx!(r"^ {0,3}\$\$.+\$\$[ \t]*$").is_match(code_safe){
            eq_closings.insert(index);object_anchor(&lines,index,"eq-",&mut findings,false);continue;
        }
        let(line,next)=mask_comments(&mask_delimited(code_safe,'$'),comment);comment=next;
        if let Some((marker,width,language))=fence(&line){
            // A fence reached after masking only needs the Rich Table flag.
            // Use static strings so no reference to the mask survives.
            let kind=if language=="rich-table"{"rich-table"}else{"other"};
            active_fence=Some((marker,width,kind,index));continue;
        }
        if line.trim()=="$$"{equation=Some(index);continue;}
        visible[index]=line;
    }
    if let Some((_,_,language,opening))=active_fence{
        if language=="rich-table"{tables.push(TableRecord{line:opening+1,closing_line:None,content:fence_content,anchor:None,anchor_line:None});}
        finding(&mut findings,"error","FENCE_UNCLOSED",opening+1,"代码围栏未闭合。");
    }
    if let Some(index)=equation{finding(&mut findings,"error","MATH_UNCLOSED",index+1,"显示公式未闭合。");}
    if comment.is_some(){finding(&mut findings,"warning","COMMENT_UNCLOSED",lines.len(),"注释未闭合；后续内容可能不可见。");}
    let mut anchors=HashMap::new();
    for(index,line)in visible.iter().enumerate(){
        if let Some(id)=anchor(line){
            if let Some(first)=anchors.get(id){
                finding(&mut findings,"error","DUPLICATE_ANCHOR",index+1,format!("块 ID ^{id} 重复，首次定义在第 {first} 行。"));
            }else{anchors.insert(id,index+1);}
            let previous=index.checked_sub(1);
            let whole=whole_anchor(line).is_some();
            if id.starts_with("eq-"){
                if previous.is_none_or(|i|!eq_closings.contains(&i)){
                    finding(&mut findings,"error","EQUATION_ANCHOR",index+1,"公式锚点须顶格独占显示公式闭合行的下一行。");
                }else if !whole{
                    finding(&mut findings,"error","EQUATION_ANCHOR",index+1,"公式锚点须顶格独占一行。");
                }
            }
            if id.starts_with("tbl-")&&(previous.is_none_or(|i|!table_closings.contains(&i))||!whole){
                finding(&mut findings,"error","TABLE_ANCHOR",index+1,"表锚须顶格独占 Rich Table 闭合围栏的下一行。");
            }
            if id.starts_with("fig-"){
                let caption=if whole{previous}else{Some(index)};
                if let Some(i)=caption{caption_figure(&visible[i],id,i+1,&mut findings);}
            }
        }
        local_links.extend(links(line,index+1).into_iter().map(|(target,line)|(target,line,String::new())));
    }
    for(target,line,context)in local_links{
        if !anchors.contains_key(target.as_str()){
            let prefix=if context.is_empty(){String::new()}else{format!("{context}：")};
            finding(&mut findings,"error","MISSING_ANCHOR",line,format!("{prefix}本篇链接目标 ^{target} 未定义。"));
        }
    }
    bibliography(&visible,&mut findings);
    findings.sort_by(|a,b|(a.line,a.code,&a.message).cmp(&(b.line,b.code,&b.message)));
    Inspection{findings,table_records:tables}
}
pub fn validate_note(text:&str)->Vec<Finding>{inspect(text).findings}
pub fn validate_file(path:&Path)->Vec<Finding>{
    match fs::read_to_string(path){
        Ok(text)=>validate_note(&text),
        Err(error)=>vec![Finding{level:"error",code:"FILE_READ",line:1,message:format!("无法读取文件：{error}")}],
    }
}
