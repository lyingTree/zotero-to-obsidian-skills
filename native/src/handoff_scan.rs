//! Original-byte candidate selectors and actual visible link occurrences.
//! All exported offsets count Unicode code points, not UTF-8 bytes.
use anyhow::{ensure,Context,Result};
use regex::Regex;
use serde_json::{json,Value};
use std::collections::{BTreeMap,BTreeSet};
use crate::{files,syntax,validate,yaml_nodes};

const MAX_LINKS:usize=100_000;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Span{pub start:usize,pub end:usize}
impl Span{
    pub fn contains(self,b:Self)->bool{self.start<=b.start&&b.start<b.end&&b.end<=self.end}
    pub fn len(self)->usize{self.end-self.start}
}
#[derive(Clone)]
pub struct Occurrence{pub span:Span,pub locator:Value,pub href:String,pub label:String,pub image:bool,pub syntax:&'static str}
#[derive(Clone,Copy)]
struct Table{first:usize,last:usize,span:Span,body:Span}
struct Visible{prose:String,target:String,tables:Vec<Table>}
fn blank(data:&mut[u8],a:usize,b:usize){for c in &mut data[a..b]{if !matches!(*c,b'\r'|b'\n'){*c=b' ';}}}
fn delimiters(s:&str,mark:u8)->String{
    let bytes=s.as_bytes();let mut data=bytes.to_vec();let mut i=0;let mut open=None;
    while i<bytes.len(){
        if bytes[i]!=mark||syntax::escaped(s,i){i+=1;continue;}
        let a=i;while i<bytes.len()&&bytes[i]==mark{i+=1;}
        let n=i-a;
        if let Some((b,m))=open{if n==m{blank(&mut data,b,i);open=None;}}
        else{open=Some((a,n));}
    }
    String::from_utf8(data).expect("equal-byte mask")
}
fn fence(s:&str)->Option<(u8,usize,&str)>{
    let body=s.trim_start_matches(' ');if s.len()-body.len()>3{return None;}
    let first=*body.as_bytes().first()?;
    if first!=b'~'&&first!=96{return None;}
    let n=body.as_bytes().iter().take_while(|v|**v==first).count();
    (n>=3).then(||(first,n,body[n..].trim()))
}
fn comments(raw:&str,p:&mut[u8],a:usize,b:usize,state:&mut Option<&'static str>){
    let mut i=a;
    while i<b{
        if let Some(end)=*state{
            if let Some(n)=raw[i..b].find(end){let n=i+n+end.len();blank(p,i,n);i=n;*state=None;}
            else{blank(p,i,b);return;}
        }else{
            let x=raw[i..b].find("<!--").map(|n|(i+n,4,"-->"));
            let y=raw[i..b].find("%%").map(|n|(i+n,2,"%%"));
            let Some((at,n,end))=(match(x,y){(Some(a),Some(b))=>Some(if a.0<b.0{a}else{b}),(a,b)=>a.or(b)})else{return;};
            if syntax::escaped(raw,at){i=at+n;continue;}
            blank(p,at,at+n);i=at+n;*state=Some(end);
        }
    }
}
fn visible(raw:&str)->Result<Visible>{
    let rows=syntax::lines(raw);let mut p=raw.as_bytes().to_vec();let mut t=p.clone();
    let mut tables=Vec::new();let mut opened:Option<(u8,usize,bool,usize)>=None;let mut comment=None;
    let mut front=rows.first().is_some_and(|r|syntax::body(raw,*r).trim()=="---");
    for(i,r)in rows.iter().enumerate(){
        let body=syntax::body(raw,*r);
        if front{
            blank(&mut p,r.start,r.end);blank(&mut t,r.start,r.end);
            if i>0&&body.trim()=="---"{front=false;}continue;
        }
        if let Some((mark,count,rich,start))=opened{
            let close=fence(body).is_some_and(|(m,n,x)|m==mark&&n>=count&&x.is_empty());
            blank(&mut p,r.start,r.end);if !rich||close{blank(&mut t,r.start,r.end);}
            if close{
                if rich{tables.push(Table{first:start,last:i,span:Span{start:rows[start].start,end:r.body_end},body:Span{start:rows[start].end,end:r.start}});}
                opened=None;
            }continue;
        }
        comments(raw,&mut p,r.start,r.end,&mut comment);
        t[r.start..r.end].copy_from_slice(&p[r.start..r.end]);
        let safe=std::str::from_utf8(&p[r.start..r.body_end])?;
        if let Some((mark,n,lang))=fence(safe){opened=Some((mark,n,lang=="rich-table",i));blank(&mut p,r.start,r.end);blank(&mut t,r.start,r.end);continue;}
        let indent=safe.chars().take_while(|c|matches!(c,' '|'\t')).fold(0usize,|n,c|if c=='\t'{n+4-n%4}else{n+1});
        if indent>=4{blank(&mut p,r.start,r.end);blank(&mut t,r.start,r.end);}
    }
    ensure!(!front&&opened.is_none()&&comment.is_none(),"MAP_UNCLOSED_MARKUP: finish frontmatter/fences/comments");
    let prose=delimiters(&delimiters(&String::from_utf8(p)?,96),b'$');
    let mut target=delimiters(&String::from_utf8(t)?,96).into_bytes();
    for f in &tables{target[f.body.start..f.body.end].copy_from_slice(&raw.as_bytes()[f.body.start..f.body.end]);}
    Ok(Visible{prose,target:String::from_utf8(target)?,tables})
}
#[derive(Clone)]
struct Hit{span:Span,href:String,label:String,image:bool,syntax:&'static str}
fn key(s:&str)->String{
    use unicode_casefold::UnicodeCaseFold;
    s.split_whitespace().collect::<Vec<_>>().join(" ").case_fold().collect()
}
fn definitions(text:&str)->Result<(BTreeMap<String,String>,Vec<Span>)>{
    let re=Regex::new(r"^ {0,3}\[([^\]\r\n]+)\]:[ \t]*(?:<([^>\r\n]+)>|([^\s]+))")?;
    let lines=syntax::lines(text);let mut out=BTreeMap::new();let mut spans=Vec::new();
    for(i,r)in lines.iter().enumerate(){
        if let Some(c)=re.captures(syntax::body(text,*r)){
            if c[1].starts_with('^'){continue;}
            let href=c.get(2).or_else(||c.get(3)).unwrap().as_str();
            ensure!(out.insert(key(&c[1]),syntax::decode_destination(href)).is_none(),"MAP_LINK_DEFINITION_DUPLICATE");
            let end=syntax::reference_definition_end(text,&lines,i).unwrap_or(i);
            spans.push(Span{start:r.start,end:lines[end].end});
        }else if syntax::reference_definition_end(text,&lines,i).is_some(){
            anyhow::bail!("MAP_LINK_DEFINITION_UNSUPPORTED: multiline definition");
        }
    }Ok((out,spans))
}
fn add(hits:&mut Vec<Hit>,used:&mut[u8],hit:Hit)->Result<()>{
    let s=hit.span;
    if s.start>=s.end||s.end>used.len()||used[s.start..s.end].iter().any(|v|*v!=0){return Ok(());}
    ensure!(hits.len()<MAX_LINKS,"MAP_LINK_BUDGET");
    used[s.start..s.end].fill(1);hits.push(hit);Ok(())
}
fn hits(raw:&str,mask:&str,defs:&BTreeMap<String,String>,excluded:&[Span])->Result<Vec<Hit>>{
    ensure!(raw.len()==mask.len(),"Mapping mask changed byte offsets");
    let mut out=Vec::new();let mut used=vec![0;raw.len()];
    for s in excluded{used[s.start..s.end].fill(1);}
    for l in syntax::scan_links(mask){
        let label=raw.get(l.label_start..l.label_end).context("Invalid inline link span")?;
        add(&mut out,&mut used,Hit{span:Span{start:l.start,end:l.end},href:l.href,label:label.to_owned(),image:l.image,syntax:"markdown"})?;
    }
    let wiki=Regex::new(r"!?\[\[([^\]\n|]+)(?:\|([^\]\n]*))?\]\]")?;
    for c in wiki.captures_iter(mask){
        let m=c.get(0).unwrap();if syntax::escaped(mask,m.start()){continue;}
        add(&mut out,&mut used,Hit{span:Span{start:m.start(),end:m.end()},href:c[1].to_owned(),
            label:c.get(2).map_or_else(||c[1].to_owned(),|s|s.as_str().to_owned()),image:m.as_str().starts_with('!'),syntax:"wiki"})?;
    }
    let mut stack=Vec::new();let mut pairs=BTreeMap::new();
    for(i,c)in mask.char_indices(){
        if syntax::escaped(mask,i){continue;}
        if c=='['{stack.push(i);}else if c==']'{if let Some(a)=stack.pop(){pairs.insert(a,i);}}
    }
    for(&a,&b)in &pairs{
        if used[a]!=0||mask.as_bytes().get(b+1)==Some(&b'('){continue;}
        let display=&raw[a+1..b];let mut label=display;let mut end=b+1;
        if let Some(&last)=pairs.get(&end){let next=&raw[end+1..last];if !next.is_empty(){label=next;}end=last+1;}
        let Some(href)=defs.get(&key(label))else{continue;};
        let image=a>0&&mask.as_bytes()[a-1]==b'!'&&!syntax::escaped(mask,a-1);
        add(&mut out,&mut used,Hit{span:Span{start:a-usize::from(image),end},href:href.clone(),label:display.to_owned(),image,syntax:"reference_markdown"})?;
    }
    let html=Regex::new(r#"(?is)<a\b[^>]*?\bhref\s*=\s*(?:"([^"]*)"|'([^']*)')[^>]*>(.*?)</a\s*>"#)?;
    for c in html.captures_iter(mask){
        let m=c.get(0).unwrap();add(&mut out,&mut used,Hit{span:Span{start:m.start(),end:m.end()},href:c.get(1).or_else(||c.get(2)).unwrap().as_str().to_owned(),
            label:c[3].to_owned(),image:false,syntax:"html"})?;
    }
    let auto=Regex::new(r"<((?:[A-Za-z][A-Za-z0-9+.-]*:[^<>\s]+)|(?:[^<>\s@]+@[^<>\s@]+\.[^<>\s@]+))>")?;
    for c in auto.captures_iter(mask){
        let m=c.get(0).unwrap();let s=&c[1];let href=if s.contains(':'){s.to_owned()}else{format!("mailto:{s}")};
        add(&mut out,&mut used,Hit{span:Span{start:m.start(),end:m.end()},href,label:s.to_owned(),image:false,syntax:"autolink"})?;
    }
    let url=Regex::new(r"(?:https?|ftp)://[^\s<>\[\]\x60]+")?;
    for m in url.find_iter(mask){
        if used[m.start()]!=0{continue;}
        let mut s=m.as_str().trim_end_matches(['.',',',';',':','!','?']);
        while s.ends_with(')')&&s.bytes().filter(|b|*b==b')').count()>s.bytes().filter(|b|*b==b'(').count(){s=&s[..s.len()-1];}
        if !s.is_empty(){add(&mut out,&mut used,Hit{span:Span{start:m.start(),end:m.start()+s.len()},href:s.to_owned(),label:s.to_owned(),image:false,syntax:"bare_url"})?;}
    }
    for m in Regex::new(r"(?i)<(?:a|img|audio|video|source)\b")?.find_iter(mask){
        ensure!(used[m.start()]!=0,"MAP_LINK_SYNTAX_UNSUPPORTED: {}",m.as_str());
    }
    out.sort_by_key(|h|(h.span.start,h.span.end));Ok(out)
}
fn pointer(p:&str,k:&str)->String{format!("{p}/{}",k.replace('~',"~0").replace('/',"~1"))}
fn values(v:&Value,p:&str,n:usize,out:&mut Vec<(String,String)>)->Result<()>{
    ensure!(n<96&&out.len()<200_000,"MAP_TABLE_BUDGET");
    match v{
        Value::String(s)=>out.push((p.into(),s.clone())),
        Value::Array(a)=>for(i,v)in a.iter().enumerate(){values(v,&pointer(p,&i.to_string()),n+1,out)?;},
        Value::Object(m)=>for(k,v)in m{values(v,&pointer(p,k),n+1,out)?;},
        _=>{},
    }Ok(())
}
fn sheet(v:&Value,p:&str,out:&mut Vec<(String,String)>)->Result<()>{
    ensure!(v.is_object(),"MAP_TABLE_STRUCTURE");
    for k in ["title","footer"]{if let Some(x)=v.get(k){values(x,&pointer(p,k),0,out)?;}}
    for(k,field)in [("columns","name"),("rows","cells")]{
        if let Some(x)=v.get(k){
            for(i,r)in x.as_array().context("MAP_TABLE_STRUCTURE: array required")?.iter().enumerate(){
                ensure!(r.is_object(),"MAP_TABLE_STRUCTURE");
                if let Some(v)=r.get(field){values(v,&pointer(&pointer(&pointer(p,k),&i.to_string()),field),0,out)?;}
            }
        }
    }Ok(())
}
pub struct Candidate{
    pub raw:String,lf:String,lf_to_raw:Vec<usize>,visible:Visible,offsets:syntax::Offsets,
    pub anchors:BTreeMap<String,Span>,pub headings:BTreeMap<String,Vec<Span>>,pub links:Vec<Occurrence>,
}
impl Candidate{
    pub fn new(bytes:&[u8])->Result<Self>{
        ensure!(bytes.len()<=32*1024*1024,"MAP_CANDIDATE_BUDGET");
        let text=std::str::from_utf8(bytes)?;let raw=text.strip_prefix('\u{feff}').unwrap_or(text).to_owned();
        let mut lf=Vec::with_capacity(raw.len());let mut map=vec![0];let mut i=0;
        while i<raw.len(){let b=raw.as_bytes()[i];if b==b'\r'{i+=1;if raw.as_bytes().get(i)==Some(&b'\n'){i+=1;}lf.push(b'\n');}
            else{lf.push(b);i+=1;}map.push(i);}
        let visible=visible(&raw)?;let offsets=syntax::Offsets::new(&raw);
        let mut anchors=BTreeMap::new();let mut headings=BTreeMap::<String,Vec<Span>>::new();
        let a=Regex::new(r"(?m)(?:^|[ \t])(\^([A-Za-z0-9-]+)[ \t]*)(?:\r?$)")?;
        for c in a.captures_iter(&visible.prose){
            let m=c.get(1).unwrap();ensure!(anchors.insert(c[2].to_owned(),Span{start:m.start(),end:m.end()}).is_none(),"MAP_CANDIDATE_ANCHOR_DUPLICATE: {}",&c[2]);
        }
        let heading=Regex::new(r"^ {0,3}(#{1,6})[ \t]+(.+?)[ \t]*$")?;let mut stack=Vec::<(usize,String)>::new();
        for r in syntax::lines(&visible.prose){
            if let Some(c)=heading.captures(syntax::body(&visible.prose,r)){
                let level=c[1].len();let title=c[2].trim_end_matches('#').trim().to_owned();
                while stack.last().is_some_and(|(l,_)|*l>=level){stack.pop();}
                stack.push((level,title.clone()));
                let full=stack.iter().map(|(_,s)|s.as_str()).collect::<Vec<_>>().join("#");
                let span=Span{start:r.start,end:r.end};headings.entry(title.clone()).or_default().push(span);
                if full!=title{headings.entry(full).or_default().push(span);}
            }
        }
        let (defs,excluded)=definitions(&visible.prose)?;
        let mut result=Self{raw,lf:String::from_utf8(lf)?,lf_to_raw:map,visible,offsets,anchors,headings,links:vec![]};
        for h in hits(&result.raw,&result.visible.prose,&defs,&excluded)?{
            let locator=result.locator(h.span)?;
            result.links.push(Occurrence{span:h.span,locator,href:h.href,label:h.label,image:h.image,syntax:h.syntax});
        }
        for t in &result.visible.tables{
            let body=&result.raw[t.body.start..t.body.end];let rows=syntax::lines(body);
            let payload=validate::rich_payload(&rows.iter().map(|r|syntax::body(body,*r).to_owned()).collect::<Vec<_>>());
            let data=yaml_nodes::load(&payload).map_err(|e|anyhow::anyhow!("MAP_TABLE_PARSE: {}",e.message))?.value;
            ensure!(matches!(data["version"].as_u64(),Some(2|3)),"MAP_TABLE_STRUCTURE: version 2/3 required");
            let mut strings=Vec::new();sheet(&data,"",&mut strings)?;
            if data["version"]==3{
                for(i,s)in data["sheets"].as_array().filter(|v|!v.is_empty()).context("MAP_TABLE_STRUCTURE: sheets")?.iter().enumerate(){
                    sheet(s,&format!("/sheets/{i}"),&mut strings)?;
                }
            }
            for(ptr,value)in strings{
                let mask=delimiters(&delimiters(&value,96),b'$');let offsets=syntax::Offsets::new(&value);
                for h in hits(&value,&mask,&defs,&[])?{
                    let snippet=&value[h.span.start..h.span.end];let mut matches=body.match_indices(snippet);
                    // A repeated/escaped YAML spelling cannot establish an
                    // exact serialized span. Keep the actual scalar pointer
                    // and require an unambiguous enclosing fence owner.
                    let span=if let Some((at,_))=matches.next().filter(|_|matches.next().is_none()){
                        Span{start:t.body.start+at,end:t.body.start+at+snippet.len()}
                    }else{t.span};
                    let locator=json!({"kind":"rich_table_text","fence_lines":[t.first+1,t.last+1],"json_pointer":ptr,
                        "start":offsets.index(h.span.start),"end":offsets.index(h.span.end),"sha256":files::digest(snippet.as_bytes()),
                        "value_sha256":files::digest(value.as_bytes())});
                    ensure!(result.links.len()<MAX_LINKS,"MAP_LINK_BUDGET");
                    result.links.push(Occurrence{span,locator,href:h.href,label:h.label,image:h.image,syntax:h.syntax});
                }
            }
        }
        let mut seen=BTreeSet::new();
        ensure!(result.links.iter().all(|o|seen.insert(o.locator.to_string())),"MAP_LINK_LOCATOR: duplicate identity");
        result.links.sort_by(|a,b|(a.span.start,a.span.end).cmp(&(b.span.start,b.span.end)).then_with(||a.locator.to_string().cmp(&b.locator.to_string())));
        Ok(result)
    }
    pub fn locator(&self,s:Span)->Result<Value>{
        let text=self.raw.get(s.start..s.end).filter(|s|!s.is_empty()).context("Invalid character range")?;
        Ok(json!({"kind":"characters","unit":"unicode_code_points","start":self.offsets.index(s.start),"end":self.offsets.index(s.end),"sha256":files::digest(text.as_bytes())}))
    }
    pub(crate) fn visible_target(&self,span:Span)->Result<&str>{
        self.visible.target.get(span.start..span.end).context("Invalid visible candidate target")
    }
    pub(crate) fn table_records(&self)->Result<Vec<(Span,Value)>>{
        let mut out=Vec::new();
        for t in &self.visible.tables{
            let body=&self.raw[t.body.start..t.body.end];
            let rows=syntax::lines(body);
            let payload=validate::rich_payload(&rows.iter().map(|r|syntax::body(body,*r).to_owned()).collect::<Vec<_>>());
            let data=yaml_nodes::load(&payload).map_err(|e|anyhow::anyhow!("MAP_TABLE_PARSE: {}",e.message))?.value;
            out.push((t.span,data));
        }
        Ok(out)
    }
    fn raw_span(&self,a:usize,b:usize)->Span{Span{start:self.lf_to_raw[a],end:self.lf_to_raw[b]}}
    pub fn resolve(&self,v:&Value,kind:&str)->Result<Span>{
        let m=v.as_object().context("MAP_TARGET_REQUIRED")?;
        ensure!(["quote","lines","anchor"].iter().filter(|k|m.contains_key(**k)).count()==1,"MAP_TARGET_SELECTOR");
        let s=if let Some(q)=v.get("quote"){
            let q=q.as_str().filter(|s|!s.trim().is_empty()).context("MAP_TARGET_QUOTE")?;
            let mut matches=self.lf.match_indices(q);let (a,_)=matches.next().context("MAP_TARGET_QUOTE: absent")?;
            ensure!(matches.next().is_none(),"MAP_TARGET_QUOTE: ambiguous");self.raw_span(a,a+q.len())
        }else if let Some(b)=v.get("lines"){
            let b=b.as_array().filter(|b|b.len()==2).context("MAP_TARGET_LINES")?;
            let a=usize::try_from(b[0].as_u64().context("MAP_TARGET_LINES")?)?;
            let b=usize::try_from(b[1].as_u64().context("MAP_TARGET_LINES")?)?;
            let rows=syntax::lines(&self.lf);
            ensure!(a>=1&&a<=b&&b<=rows.len(),"MAP_TARGET_LINES");
            let (a,b)=(rows[a-1].start,rows[b-1].end);
            ensure!(v["sha256"]==files::digest(self.lf[a..b].as_bytes()),"MAP_TARGET_LINES_HASH");self.raw_span(a,b)
        }else{
            let anchor=self.anchors.get(v["anchor"].as_str().context("MAP_TARGET_ANCHOR")?).context("MAP_TARGET_ANCHOR: absent")?;
            let rows=syntax::lines(&self.raw);let idx=syntax::row_at(&rows,anchor.start);
            let mut start=rows[idx].start;let end=rows[idx].body_end;
            let inline=!self.raw[start..anchor.start].trim().is_empty();
            if inline&&kind=="figure"{
                let mut i=idx;while i>0&&syntax::body(&self.raw,rows[i-1]).trim().is_empty(){i-=1;}
                if i>0&&syntax::scan_links(syntax::body(&self.raw,rows[i-1])).iter().any(|l|l.image){start=rows[i-1].start;}
            }else if !inline&&idx>0{
                let previous=syntax::body(&self.raw,rows[idx-1]).trim();
                let close=fence(previous).filter(|(_,_,tail)|tail.is_empty());
                if close.is_some()||previous=="$$"{
                    let begin=(0..idx-1).rev().find(|i|{
                        let body=syntax::body(&self.raw,rows[*i]).trim();
                        if let Some((m,n,_))=close{fence(body).is_some_and(|(c,k,_)|c==m&&k>=n)}else{body=="$$"}
                    }).context("MAP_TARGET_BLOCK: missing opening delimiter")?;
                    start=rows[begin].start;
                }else{let mut i=idx;while i>0&&!syntax::body(&self.raw,rows[i-1]).trim().is_empty(){i-=1;}start=rows[i].start;}
            }Span{start,end}
        };
        ensure!(s.start<s.end&&self.visible.target.get(s.start..s.end).is_some_and(|t|!t.trim().is_empty()),"MAP_TARGET_HIDDEN");
        ensure!(!Regex::new(r"\^[A-Za-z0-9-]+")?.replace_all(&self.visible.target[s.start..s.end],"").trim().is_empty(),"MAP_TARGET_EMPTY");
        Ok(s)
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn raw_crlf_unicode_offsets_and_visible_links(){
        let raw="---\r\nx: 1\r\n---\r\n# 波动\r\n\r\n第一行 [甲](#^ref-a)\r\n第二行\r\n^p\r\n\r\n参考 ^ref-a\r\n\r\n\x60[hidden](#^bad)\x60\r\n";
        let c=Candidate::new(raw.as_bytes()).unwrap();
        let s=c.resolve(&json!({"quote":"第一行 [甲](#^ref-a)\n第二行"}),"paragraph").unwrap();
        assert_eq!(&c.raw[s.start..s.end],"第一行 [甲](#^ref-a)\r\n第二行");
        assert_eq!(c.locator(s).unwrap()["start"],raw[..s.start].chars().count());
        assert_eq!(c.links.len(),1);
        assert!(c.resolve(&json!({"anchor":"p"}),"paragraph").unwrap().contains(c.links[0].span));
    }
    #[test]
    fn rich_table_actual_scalar_pointer(){
        let raw=concat!("~~~rich-table\n",r#"{"version":2,"title":"[C](#^ref-a)","columns":[{"name":"A"}],"rows":[{"cells":{"a":"[[#^ref-a|Value]]"}}]}"#,"\n~~~\n^tbl-a\n\nReference ^ref-a\n");
        let c=Candidate::new(raw.as_bytes()).unwrap();assert_eq!(c.links.len(),2);
        assert_eq!(c.links[0].locator["json_pointer"],"/title");
        assert_eq!(c.links[1].locator["json_pointer"],"/rows/0/cells/a");
        let s=c.resolve(&json!({"anchor":"tbl-a"}),"table").unwrap();
        assert!(c.links.iter().all(|l|s.contains(l.span)));
    }
}
