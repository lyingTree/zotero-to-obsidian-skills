//! Actual visible object inventory for the read-only mechanical audit.
use crate::{grid,properties,syntax,validate};
use anyhow::Result;
use regex::Regex;
use serde_json::{json,Value};
use std::{collections::{BTreeMap,HashMap},sync::OnceLock};
macro_rules! rx{($pat:expr)=>{{static RE:OnceLock<Regex>=OnceLock::new();RE.get_or_init(||Regex::new($pat).expect("inventory regex"))}}}
#[derive(Default,Debug)]
pub struct Inventory{
    pub anchors:BTreeMap<String,usize>,pub counts:BTreeMap<String,usize>,pub dimensions:BTreeMap<String,Value>,
    pub resources:Vec<(String,usize,&'static str)>,pub targets:Vec<(&'static str,String,usize)>,
    pub headings:HashMap<String,usize>,pub events:BTreeMap<usize,Vec<(&'static str,usize)>>,
    pub grid_findings:Vec<grid::GridFinding>,
}
impl Inventory{
    fn event(&mut self,kind:&'static str,start:usize,end:usize){self.events.entry(end).or_default().push((kind,start));}
    fn count(&mut self,kind:&str){*self.counts.entry(kind.into()).or_default()+=1;}
    fn scan_links(&mut self,line:&str,number:usize){
        let(line,_)=validate::mask_comments(line,None);
        let line=validate::mask_delimited(&validate::mask_delimited(&line,'\x60'),'$');
        for link in syntax::scan_links(&line){
            if link.image{self.resources.push((link.href.clone(),number,"markdown"));}
            let decoded=percent_encoding::percent_decode_str(&link.href).decode_utf8_lossy();
            if let Some(target)=decoded.strip_prefix("#^").filter(|s|rx!(r"^[A-Za-z0-9-]+$").is_match(s)){
                self.targets.push(("block",target.into(),number));
            }
            if link.href.starts_with('#')&&!link.href.starts_with("#^")&&!link.image{
                self.targets.push(("heading",percent_encoding::percent_decode_str(&link.href[1..]).decode_utf8_lossy().into_owned(),number));
            }
        }
        for cap in rx!(r"!\[\[([^\]\n|]+)(?:\|[^\]\n]*)?\]\]").captures_iter(&line){
            if !syntax::escaped(&line,cap.get(0).unwrap().start()){
                self.resources.push((cap[1].into(),number,"wiki"));
            }
        }
        for cap in rx!(r"\[\[#\^([A-Za-z0-9-]+)(?:\|[^\]\n]*)?\]\]").captures_iter(&line){
            if !syntax::escaped(&line,cap.get(0).unwrap().start()){self.targets.push(("block",cap[1].into(),number));}
        }
        for cap in rx!(r"\[\[#([^|\]\n]+)(?:\|[^\]\n]*)?\]\]").captures_iter(&line){
            if !cap[1].starts_with('^')&&!syntax::escaped(&line,cap.get(0).unwrap().start()){self.targets.push(("heading",cap[1].into(),number));}
        }
    }
}
fn visible(text:&str,keep_tables:bool)->Vec<String>{
    let mut out=Vec::new();let mut fenced:Option<(u8,usize,String)>=None;let mut comment=None;
    let mut front=text.split('\n').next().is_some_and(|s|s.trim()=="---");
    for(index,line)in text.split('\n').enumerate(){
        if front{out.push(validate::blank(line));if index>0&&matches!(line.trim(),"---"|"..."){front=false;}continue;}
        if let Some((mark,width,language))=&fenced{
            let closing=validate::closing(line,*mark,*width);
            out.push(if language=="rich-table"&&keep_tables{line.to_owned()}else{validate::blank(line)});
            if closing{fenced=None;}continue;
        }
        let (safe,next)=validate::mask_comments(line,comment);comment=next;
        if let Some((mark,width,language))=validate::fence(&safe){
            fenced=Some((mark,width,language.into()));
            out.push(if keep_tables&&language=="rich-table"{safe}else{validate::blank(&safe)});continue;
        }
        if validate::indentation(&safe)>=4{out.push(validate::blank(&safe));continue;}
        out.push(validate::mask_delimited(&safe,'\x60'));
    }out
}
fn strings<'a>(v:&'a Value,out:&mut Vec<&'a str>){
    match v{Value::String(s)=>out.push(s),Value::Array(a)=>for x in a{strings(x,out);},
        Value::Object(m)=>for x in m.values(){strings(x,out);},_=>{}}
}
pub fn inspect(text:&str)->Result<Inventory>{
    let lines:Vec<_>=text.split('\n').collect();
    let visible=visible(text,false);let tables=self::visible(text,true);
    let mut inv=Inventory::default();
    for key in ["figures","equations","bibliography","tables","images"]{inv.counts.insert(key.into(),0);}
    let mut prior:Vec<(usize,String)>=vec![];let mut images=HashMap::new();
    let mut fence:Option<(u8,usize,usize)>=None;let mut equation=None;let mut bibliography=None;
    for(idx,(line,table_line))in visible.iter().zip(&tables).enumerate(){
        let number=idx+1;
        if let Some((mark,width,opening))=fence{
            if !validate::closing(table_line,mark,width){continue;}
            fence=None;inv.event("table",opening+1,number);
            // A malformed table has already been diagnosed by structural validation.
            let payload=lines[opening+1..idx].iter().map(|s|s.to_string()).collect::<Vec<_>>();
            let data=properties::parse_yaml(validate::rich_payload(&payload).as_bytes())?;
            if !data.is_object(){continue;}
            let sheets=if data["version"]==3{data["sheets"].as_array().map(|s|s.iter().collect::<Vec<_>>()).unwrap_or_default()}else{vec![&data]};
            if data["version"]==3{
                for key in ["title","footer"]{let mut values=vec![];strings(&data[key],&mut values);for v in values{inv.scan_links(v,opening+1);}}
            }
            let mut sizes=vec![];
            for sheet in sheets{
                if !sheet.is_object(){continue;}
                inv.grid_findings.extend(grid::check_merges(&format!("line {}",opening+1),None,sheet));
                let mut fields=vec![&sheet["title"],&sheet["footer"]];
                for (field,key)in[("columns","name"),("rows","cells")]{
                    if let Some(a)=sheet[field].as_array(){fields.extend(a.iter().filter(|x|x.is_object()).map(|x|&x[key]));}
                }
                for field in fields{let mut values=vec![];strings(field,&mut values);for v in values{inv.scan_links(v,opening+1);}}
                if let(Some(rows),Some(cols))=(sheet["rows"].as_array(),sheet["columns"].as_array()){
                    sizes.push(json!({"rows":rows.len(),"columns":cols.len()}));
                }
            }
            if let Some(anchor)=visible.get(idx+1).and_then(|s|validate::whole_anchor(s)){inv.dimensions.insert(anchor.into(),json!(sizes));}
            continue;
        }
        if let Some((mark,width,"rich-table"))=validate::fence(table_line){inv.count("tables");fence=Some((mark,width,idx));continue;}
        if line.trim()=="$$"{
            if let Some(start)=equation.take(){inv.count("equations");inv.event("formula",start,number);}else{equation=Some(number);}
            continue;
        }
        if equation.is_some(){continue;}
        if rx!(r"^ {0,3}\$\$.+\$\$[ \t]*$").is_match(line){inv.count("equations");inv.event("formula",number,number);continue;}
        let line=validate::mask_delimited(line,'$');
        let anchor=validate::anchor(&line);
        if let Some(id)=anchor{
            inv.anchors.insert(id.into(),number);
            if id.starts_with("fig-"){
                let standalone=validate::whole_anchor(&line).is_some();
                let cap=if standalone{prior.last().map(|(_,s)|s.as_str()).unwrap_or("")}
                    else{line.rfind(&format!("^{id}")).map(|p|line[..p].trim()).unwrap_or("")};
                let img=if standalone&&prior.len()>=2{prior[prior.len()-2].0}else{prior.last().map(|p|p.0).unwrap_or(0)};
                if !cap.trim().is_empty()&&validate::heading(cap).is_none(){
                    if let Some(&start)=images.get(&img){inv.count("figures");inv.event("figure",start,number);}
                }
            }
        }
        if let Some((level,title))=validate::heading(&line){
            inv.event(if level==1{"header"}else{"heading"},number,number);
            let title=rx!(r"\\([^\w\s])").replace_all(title,|c:&regex::Captures|c[1].to_owned()).into_owned();
            *inv.headings.entry(title.clone()).or_default()+=1;
            if bibliography.is_some_and(|b|level<=b){bibliography=None;}
            if rx!(r"^(?:[0-9]+(?:\.[0-9]+)*[ \t]+)?(?:参考文献|References|Bibliography)(?:[ \t]*[：:])?$").is_match(title.as_str()){bibliography=Some(level);}
        }else if rx!(r"^ {0,3}(?:-{3,}|\*{3,}|_{3,})[ \t]*$").is_match(&line){bibliography=None;}
        else if bibliography.is_some()&&validate::entry(&line).is_some(){inv.count("bibliography");}
        if !line.trim().is_empty(){
            if syntax::standalone_image(&line){let p=prior.last().map(|p|p.0).unwrap_or(0);images.insert(number,*images.get(&p).unwrap_or(&number));}
            if validate::entry(&line).is_some()&&anchor.is_some_and(|a|rx!(r"(?i)^(?:s?ref)(?:-[A-Za-z][A-Za-z0-9]*)*-?[1-9][0-9]{0,8}$").is_match(a)){
                inv.event("reference",number,number);inv.event("bibliography",number,number);
            }
            if rx!(r"^(?:- |[0-9]+\. )").is_match(&line){inv.event("list",number,number);}
            if line.starts_with("> "){inv.event("metadata",number,number);}
            if line.starts_with("> 译注："){inv.event("translator_note",number,number);}
            if line.starts_with("## [Zotero原条目](zotero://select/"){inv.event("zotero_link",number,number);}
            inv.event("paragraph",number,number);
            if prior.len()==2{prior.remove(0);}prior.push((number,line.clone()));
        }
        inv.scan_links(&line,number);
    }
    inv.counts.insert("images".into(),inv.resources.len());Ok(inv)
}
