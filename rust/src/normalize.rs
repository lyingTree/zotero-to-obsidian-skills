//! Native conservative Markdown normalizer. Internal spans use UTF-8 byte
//! offsets; the published change ledger uses Unicode code-point offsets.
use crate::{files,figures,syntax::{self,Offsets,Line}};
use anyhow::{ensure,Result,Context};
use fancy_regex::Regex as Fancy;
use regex::Regex;
use serde_json::{json,Value};
use std::{collections::{HashMap,HashSet},fs,io::Write,path::Path,sync::OnceLock};

fn compile_rx(s:&str)->Regex{Regex::new(s).expect("static regex")}
macro_rules! rx {($s:expr)=>{{static R:OnceLock<Regex>=OnceLock::new();R.get_or_init(||compile_rx($s))}}}
macro_rules! fx {($s:expr)=>{{static R:OnceLock<Fancy>=OnceLock::new();R.get_or_init(||Fancy::new($s).expect("static regex"))}}}
const UNITS:&str=r"小时|分钟|秒钟|千米|公里|厘米|毫米|米|秒|节|年|milliseconds?|seconds?|minutes?|hours?|days?|weeks?|months?|years?|min|km|cm|mm|ms|kg|mg|kPa|Pa|Hz|m|h|s|g|[%％]";
const NUMBER:&str=r"[+−-]?(?:[0-9]+(?:\.[0-9]+)?|\.[0-9]+)";
fn quantities()->&'static Fancy{fx!(&format!(r"(?<![A-Za-z0-9_.:/+−-])(?P<n>{NUMBER})[ \t]*(?P<u>{UNITS})(?![A-Za-z0-9_])"))}
fn wrapped()->&'static Fancy{fx!(&format!(r"\$(?P<n>{NUMBER})\$[ \t]*(?P<u>{UNITS})(?![A-Za-z0-9_])"))}
fn split_percent()->&'static Fancy{fx!(r"(?<![A-Za-z0-9_.:/+−-])(?P<a>[+−-]?[0-9]+\.)\$(?P<b>[0-9]+)\\?[%％]\$")}
fn token()->&'static Fancy{fx!(concat!(
    r"(?m)^ {0,3}#{1,6}[ \t]+[^\r\n]*|^ {0,3}>[^\r\n]*|^ {0,3}(?:\x60{3,}|~{3,})[^\r\n]*|^(?: {4}|\t)[^\r\n]*|",
    r"^ {0,3}\[(?:\\.|[^\]\\\r\n])+\]:[^\r\n]*|",
    r"\\\\[^\s<>]+|<!--|%%|\\[\s\S]|\x60+|\$\$|\$|!?\[|<|",
    r"(?i:(?<![A-Za-z0-9+.-])(?:[a-z][a-z0-9+.-]*://|www\.)[^\s<>]+)|",
    r"(?<![\w.])10\.\d{4,9}/[^\s<>]+|",
    r"(?:[A-Za-z]:[\\/]|(?:\.{0,2}/|~/)|(?<![\w.-])[\w.\-]+[/\\])[^\s<>]*|",
    r"(?i:(?<![\w.—-])[\w.—-]+\.(?:png|jpe?g|svg|gif|webp|pdf|md|csv|docx?|xlsx?|py|txt)\b)|",
    r"\^[\w-]+|(?<![\w.+-])[\w.+-]+@[\w.-]+\.[A-Za-z]+"
))}
fn prefix(s:&str)->bool{rx!(r"(?i)(?:第|图|表|公式|方程|章节|文献|引理|定理|命题|推论|定义|例|Fig\.?|Figure|Table|Eq\.?|Section|Lemma|Theorem)\s*$").is_match(s)}
enum QuantityCheck{Convert,Skip,Ambiguous}
fn classify_quantity(text:&str,offsets:&Offsets,a:usize,b:usize,n:&str,u:&str)->Result<QuantityCheck>{
    let before=&text[offsets.previous(a,36)..a];let after=&text[b..offsets.next(b,32)];
    if u=="节"&&rx!(r"^(?:课|目|点|能|选|[、.．])").is_match(after){return Ok(QuantityCheck::Skip);}
    if prefix(before)||(u=="年"&&after.starts_with('级')){return Ok(QuantityCheck::Skip);}
    if u=="年"&&n.len()==4&&n.bytes().all(|b|b.is_ascii_digit())&&n.parse::<u32>().is_ok_and(|y|(1500..=2199).contains(&y)){
        return Ok(QuantityCheck::Skip);
    }
    if rx!(r"(?i)(?:UTC\s*)$").is_match(before)||rx!(r"(?i)^\s*UTC\b").is_match(after)||rx!(r"[A-Za-z_]\s*=\s*$").is_match(before){
        return Ok(QuantityCheck::Skip);
    }
    if rx!(r"[0-9]\s*(?:[-−–—~～至到×±]|[eE][+-]?)\s*$").is_match(before)||
        rx!(r"^(?:[/^²³·]|\s*[×±]|每|/)").is_match(after)||
        fx!(r"^[ \t]+(?:min|[kcm]?m|[kmg]?g|s|h|Hz|Pa)(?=[⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻^/·\s.,，。；;]|$)").is_match(after)?||
        before.trim_end().ends_with(['×','±','·','/'])||
        rx!(r"[0-9][,，]$|[±×]$").is_match(before){
        return Ok(QuantityCheck::Ambiguous);
    }
    Ok(QuantityCheck::Convert)
}
fn allowed(text:&str,offsets:&Offsets,a:usize,b:usize,n:&str,u:&str)->Result<bool>{
    Ok(matches!(classify_quantity(text,offsets,a,b,n,u)?,QuantityCheck::Convert))
}
fn math(n:&str,u:&str)->String{
    if matches!(u,"%"|"％"){return format!("{}{n}\\%$",'$');}
    let cmd=if u.chars().any(|c|('\u{3400}'..='\u{9fff}').contains(&c)){"text"}else{"mathrm"};
    format!("{}{n}\\,\\{cmd}{{{u}}}$",'$')
}
fn is_cjk(c:char)->bool{('\u{3400}'..='\u{9fff}').contains(&c)}
fn is_latin_math(c:char)->bool{c.is_ascii_alphanumeric()||c=='$'}
fn is_transparent_tag(s:&str)->bool{rx!(r"(?i)^</?(?:sup|sub)(?:[\s>][^>]*)?>$").is_match(s)}
fn strip_edge_tags_start(mut s:&str)->&str{
    while let Some(m)=rx!(r"(?i)^</?(?:sup|sub)(?:[\s>][^>]*)?>").find(s){s=&s[m.end()..];}
    s
}
fn strip_edge_tags_end(mut s:&str)->&str{
    while let Some(m)=rx!(r"(?i)</?(?:sup|sub)(?:[\s>][^>]*)?>$").find(s){s=&s[..m.start()];}
    s
}
fn link_visible_label(text:&str,links:&HashMap<usize,syntax::Link>,span:&Span)->Option<(usize,usize)>{
    if span.kind!="link"||text[span.a..].starts_with("!["){return None;}
    if let Some(l)=links.get(&span.a){
        if !l.image{return Some((l.label_start,l.label_end));}
        return None;
    }
    if text[span.a..span.b].starts_with("[[")&&text[span.a..span.b].ends_with("]]"){
        let x=span.a+2;let y=span.b-2;
        if let Some(i)=text[x..y].find('|'){return Some((x+i+1,y));}
        return Some((x,y));
    }
    None
}
fn visible_left_char(text:&str,spans:&[Span],links:&HashMap<usize,syntax::Link>,mut idx:usize)->Option<(char,bool)>{
    let mut crossed=false;
    loop{
        if let Ok(i)=spans.binary_search_by_key(&idx,|s|s.b){
            let s=&spans[i];
            if s.kind=="literal"&&is_transparent_tag(&text[s.a..s.b]){crossed=true;idx=s.a;continue;}
            if let Some((la,lb))=link_visible_label(text,links,s){
                let ch=strip_edge_tags_end(&text[la..lb]).chars().next_back();
                return ch.map(|c|(c,true));
            }
            if s.kind=="math"&&crossed{return text[..idx].chars().next_back().map(|c|(c,true));}
            return None;
        }
        let ch=text[..idx].chars().next_back()?;
        if matches!(ch,'\r'|'\n'){return None;}
        return Some((ch,crossed));
    }
}
fn visible_right_char(text:&str,spans:&[Span],links:&HashMap<usize,syntax::Link>,mut idx:usize)->Option<(char,bool)>{
    let mut crossed=false;
    loop{
        if let Ok(i)=spans.binary_search_by_key(&idx,|s|s.a){
            let s=&spans[i];
            if s.kind=="literal"&&is_transparent_tag(&text[s.a..s.b]){crossed=true;idx=s.b;continue;}
            if let Some((la,lb))=link_visible_label(text,links,s){
                let ch=strip_edge_tags_start(&text[la..lb]).chars().next();
                return ch.map(|c|(c,true));
            }
            if s.kind=="math"&&crossed{return text[idx..].chars().next().map(|c|(c,true));}
            return None;
        }
        let ch=text[idx..].chars().next()?;
        if matches!(ch,'\r'|'\n'){return None;}
        return Some((ch,crossed));
    }
}

#[derive(Clone,Copy,Debug,Default)]
pub struct Options{pub strip_display_trailing_punctuation:bool,pub normalize_line_endings:bool}
#[derive(Clone,Debug)]
struct Span{a:usize,b:usize,kind:&'static str}
#[derive(Clone,Debug)]
struct Edit{a:usize,b:usize,rule:&'static str,after:String}
struct Pass<'a>{text:&'a str,rows:Vec<Line>,offsets:Offsets,options:Options,display_whole_rows:HashSet<usize>,display_end_rows:HashSet<usize>,heading_renames:HashMap<String,String>,spans:Vec<Span>,edits:Vec<Edit>,issues:Vec<Value>}
impl<'a> Pass<'a>{
    fn new(text:&'a str,options:Options)->Self{Self{text,rows:syntax::lines(text),offsets:Offsets::new(text),options,display_whole_rows:HashSet::new(),display_end_rows:HashSet::new(),heading_renames:HashMap::new(),spans:vec![],edits:vec![],issues:vec![]}}
    fn clean_cjk_scalar(s:&str)->Result<Option<String>>{
        let mut mask:Vec<(usize,usize)>=Vec::new();
        let mut i=0usize;
        while i<s.len(){
            let rest=&s[i..];
            if rest.starts_with('`'){
                let width=rest.bytes().take_while(|c|*c==b'`').count();
                let mut cur=i+width;
                while let Some(pos)=s[cur..].find(&"`".repeat(width)){
                    let cand=cur+pos;
                    if s[cand..].bytes().take_while(|c|*c==b'`').count()==width{
                        mask.push((i,cand+width));
                        cur=cand+width;break;
                    }
                    cur=cand+1;
                }
                i=cur;continue;
            }
            if rest.starts_with('$')&&!rest.starts_with("$$")&&!syntax::escaped(s,i){
                let mut cur=i+1;
                while let Some(pos)=s[cur..].find('$'){
                    let cand=cur+pos;
                    if !syntax::escaped(s,cand){
                        mask.push((i+1,cand));
                        cur=cand+1;break;
                    }
                    cur=cand+1;
                }
                i=cur;continue;
            }
            i+=rest.chars().next().map_or(1,char::len_utf8);
        }
        let spaces=fx!(r"(?<=[\u{3400}-\u{9fff}]) +(?=[0-9A-Za-z$])|(?<=[0-9A-Za-z$]) +(?=[\u{3400}-\u{9fff}])");
        let mut cuts=Vec::new();
        let mut pos=0usize;
        while let Some(m)=spaces.find_from_pos(s,pos)?{
            let a=m.start();let b=m.end();pos=b;
            if mask.iter().any(|&(ma,mb)|a>=ma&&b<=mb){continue;}
            cuts.push((a,b));
        }
        if cuts.is_empty(){return Ok(None);}
        let mut out=String::with_capacity(s.len());
        let mut cur=0usize;
        for (a,b) in cuts{out.push_str(&s[cur..a]);cur=b;}
        out.push_str(&s[cur..]);
        Ok(Some(out))
    }
    fn clean_rich_table_cjk_space(&mut self,a:usize,b:usize)->Result<()>{
        let raw=&self.text[a..b];
        let eol=if raw.contains("\r\n"){"\r\n"}else{"\n"};
        let rows:Vec<&str>=raw.split('\n').map(|l|l.strip_suffix('\r').unwrap_or(l)).collect();
        if rows.len()<3{return Ok(());}
        let inner:Vec<String>=rows[1..rows.len()-1].iter().map(|s|(*s).to_owned()).collect();
        let non_empty:Vec<&str>=inner.iter().map(|s|s.trim()).filter(|s|!s.is_empty()).collect();
        let wrapped=non_empty.first().is_some_and(|s|*s=="---")&&non_empty.last().is_some_and(|s|matches!(*s,"---"|"..."));
        let Ok(mut val)=crate::properties::parse_yaml(crate::validate::rich_payload(&inner).as_bytes()) else{return Ok(());};
        if !val.is_object(){return Ok(());}
        let mut changed=false;
        fn walk_sheet(sheet:&mut Value,changed:&mut bool)->Result<()>{
            for key in ["title","footer"]{
                if let Some(s)=sheet.get(key).and_then(Value::as_str){
                    if let Some(clean)=Pass::clean_cjk_scalar(s)?{sheet[key]=json!(clean);*changed=true;}
                }
            }
            if let Some(cols)=sheet.get_mut("columns").and_then(Value::as_array_mut){
                for col in cols{
                    if let Some(s)=col.get("name").and_then(Value::as_str){
                        if let Some(clean)=Pass::clean_cjk_scalar(s)?{col["name"]=json!(clean);*changed=true;}
                    }
                }
            }
            if let Some(rows)=sheet.get_mut("rows").and_then(Value::as_array_mut){
                for row in rows{
                    if let Some(cells)=row.get_mut("cells").and_then(Value::as_object_mut){
                        for v in cells.values_mut(){
                            if let Some(s)=v.as_str(){
                                if let Some(clean)=Pass::clean_cjk_scalar(s)?{*v=json!(clean);*changed=true;}
                            }
                        }
                    }
                }
            }
            Ok(())
        }
        if val["version"]==3{
            for key in ["title","footer"]{
                if let Some(s)=val.get(key).and_then(Value::as_str){
                    if let Some(clean)=Self::clean_cjk_scalar(s)?{val[key]=json!(clean);changed=true;}
                }
            }
            if let Some(sheets)=val.get_mut("sheets").and_then(Value::as_array_mut){
                for sheet in sheets{if sheet.is_object(){walk_sheet(sheet,&mut changed)?;}}
            }
        }else{
            walk_sheet(&mut val,&mut changed)?;
        }
        if !changed{return Ok(());}
        let Ok(dumped)=crate::yaml_emit::dump(&val) else{return Ok(());};
        let payload=if wrapped{format!("---\n{dumped}---\n")}else{dumped};
        let rebuilt=format!("{}\n{}{}",rows[0],payload,rows[rows.len()-1]);
        let rebuilt=if eol=="\n"{rebuilt}else{rebuilt.replace('\n',eol)};
        if rebuilt!=raw{self.edit(a,b,"table_cjk_space",rebuilt);}
        Ok(())
    }
    fn heading_cjk_space(&mut self,a:usize,b:usize)->Result<()>{
        let line=&self.text[a..b];
        let body_start=if let Some(m)=rx!(r"^ {0,3}#{1,6} +(?:[0-9]+(?:\.[0-9]+)* +)?").find(line){
            a+m.end()
        }else{
            return Ok(());
        };
        if body_start>=b{return Ok(());}
        let sub=&self.text[body_start..b];
        let mut mask_ranges:Vec<(usize,usize)>=Vec::new();
        let mut i=0usize;
        while i<sub.len(){
            let rest=&sub[i..];
            if rest.starts_with('`'){
                let width=rest.bytes().take_while(|c|*c==b'`').count();
                let mut cur=i+width;
                while let Some(pos)=sub[cur..].find(&"`".repeat(width)){
                    let cand=cur+pos;
                    if sub[cand..].bytes().take_while(|c|*c==b'`').count()==width{
                        mask_ranges.push((body_start+i,body_start+cand+width));
                        cur=cand+width;break;
                    }
                    cur=cand+1;
                }
                i=cur;continue;
            }
            if rest.starts_with('$')&&!rest.starts_with("$$")&&!syntax::escaped(sub,i){
                let mut cur=i+1;
                while let Some(pos)=sub[cur..].find('$'){
                    let cand=cur+pos;
                    if !syntax::escaped(sub,cand){
                        mask_ranges.push((body_start+i,body_start+cand+1));
                        cur=cand+1;break;
                    }
                    cur=cand+1;
                }
                i=cur;continue;
            }
            i+=rest.chars().next().map_or(1,char::len_utf8);
        }
        let mut local_edits:Vec<(usize,usize)>=Vec::new();
        let links=syntax::scan_links(sub);
        for l in links{
            if l.label_end+1<l.end{mask_ranges.push((body_start+l.label_end+1,body_start+l.end));}
            let la=body_start+l.start;let lb=body_start+l.end;
            if !l.image{
                let inner=strip_edge_tags_end(strip_edge_tags_start(&self.text[body_start+l.label_start..body_start+l.label_end]));
                if let Some(rc)=inner.chars().next(){
                    let before_len=self.text[body_start..la].trim_end_matches(' ').len();
                    let space_a=body_start+before_len;
                    if space_a<la{
                        if let Some(lc)=self.text[body_start..space_a].chars().next_back(){
                            if (is_cjk(lc)&&is_latin_math(rc))||(is_latin_math(lc)&&is_cjk(rc)){
                                local_edits.push((space_a,la));
                            }
                        }
                    }
                }
                if let Some(lc)=inner.chars().next_back(){
                    let after_spaces=self.text[lb..b].bytes().take_while(|c|*c==b' ').count();
                    if after_spaces>0{
                        let space_b=lb+after_spaces;
                        if let Some(rc)=self.text[space_b..b].chars().next(){
                            if (is_cjk(lc)&&is_latin_math(rc))||(is_latin_math(lc)&&is_cjk(rc)){
                                local_edits.push((lb,space_b));
                            }
                        }
                    }
                }
            }
        }
        mask_ranges.sort_by_key(|r|r.0);
        let spaces=fx!(r"(?<=[\u{3400}-\u{9fff}]) +(?=[0-9A-Za-z$])|(?<=[0-9A-Za-z$]) +(?=[\u{3400}-\u{9fff}])");
        let mut pos=body_start;
        while let Some(m)=spaces.find_from_pos(&self.text[..b],pos)?{
            let s=m.start();let e=m.end();pos=e;
            if mask_ranges.iter().any(|&(ma,mb)|s>=ma&&e<=mb){continue;}
            local_edits.push((s,e));
        }
        if !local_edits.is_empty(){
            local_edits.sort_unstable();
            local_edits.dedup();
            let mut new_line=String::with_capacity(b-a);
            let mut cur=a;
            for &(ea,eb) in &local_edits{
                if ea<cur{continue;}
                self.edit(ea,eb,"cjk_space","");
                new_line.push_str(&self.text[cur..ea]);
                cur=eb;
            }
            new_line.push_str(&self.text[cur..b]);
            let h_rx=rx!(r"^ {0,3}(#{1,6})[ \t]+(.+?)[ \t]*#*[ \t]*$");
            if let (Some(co),Some(cn))=(h_rx.captures(line.trim_end_matches(['\r','\n'])),h_rx.captures(new_line.trim_end_matches(['\r','\n']))){
                let old_t=co.get(2).unwrap().as_str();
                let new_t=cn.get(2).unwrap().as_str();
                if old_t!=new_t{self.heading_renames.insert(old_t.to_owned(),new_t.to_owned());}
            }
        }
        Ok(())
    }
    fn issue(&mut self,code:&str,at:usize,message:&str,severity:&str){
        let line=if self.rows.is_empty(){1}else if at==self.text.len(){self.rows.len()+1}else{syntax::row_at(&self.rows,at)+1};
        self.issues.push(json!({"code":code,"line":line,"start":self.offsets.index(at),"message":message,"severity":severity}));
    }
    fn protect(&mut self,a:usize,b:usize,kind:&'static str)->usize{if b>a{self.spans.push(Span{a,b,kind});}b}
    fn edit(&mut self,a:usize,b:usize,rule:&'static str,after:impl Into<String>){self.edits.push(Edit{a,b,rule,after:after.into()});}
    fn prose(&mut self,a:usize,b:usize)->Result<()>{
        if a>=b{return Ok(());}
        let text=self.text;let end=&text[..b];let mut pos=a;
        while let Some(c)=quantities().captures_from_pos(end,pos)?{
            let m=c.get(0).unwrap();let n=c.name("n").unwrap().as_str();let u=c.name("u").unwrap().as_str();
            match classify_quantity(text,&self.offsets,m.start(),m.end(),n,u)?{
                QuantityCheck::Convert=>self.edit(m.start(),m.end(),"quantity",math(n,u)),
                QuantityCheck::Ambiguous=>self.issue("AMBIGUOUS_QUANTITY",m.start(),"编号、日期、范围或复合单位需核对。","review"),
                QuantityCheck::Skip=>{}
            }
            pos=m.end();
        }
        let ambiguous=fx!(r"(?<![\w.])\d+(?:\.\d+)?[ \t]+[A-Za-zµμ°]+\b");let mut pos=a;
        while let Some(m)=ambiguous.find_from_pos(end,pos)?{
            if !quantities().find_from_pos(text,m.start())?.is_some_and(|q|q.start()==m.start()){
                self.issue("UNCLASSIFIED_UNIT",m.start(),"保留未识别的数值与拉丁文字组合。","review");
            }pos=m.end();
        }
        for m in rx!(r"[\u{2010}-\u{2015}]").find_iter(&text[a..b]){
            let start=a+m.start();let finish=a+m.end();
            match syntax::classify_dash(&text[a..start],m.as_str(),&text[finish..b]){
                syntax::Dash::Numeric=>self.issue("AMBIGUOUS_RANGE",start,"保留数值之间的横线，待确认是范围还是减法。","review"),
                syntax::Dash::Hyphen=>self.edit(start,finish,"dash","-"),
                syntax::Dash::Preserve=>{}
            }
        }
        let spaces=fx!(r"(?<=[\u{3400}-\u{9fff}]) +(?=[0-9A-Za-z$])|(?<=[0-9A-Za-z$]) +(?=[\u{3400}-\u{9fff}])");
        let mut pos=self.offsets.previous(a,1);let end=&text[..self.offsets.next(b,1)];
        while let Some(m)=spaces.find_from_pos(end,pos)?{
            let start=m.start();let finish=m.end();pos=finish;
            let line_start=self.rows[syntax::row_at(&self.rows,start)].start;
            if rx!(r"^ {0,3}#{1,6} +[0-9]+(?:\.[0-9]+)*$").is_match(&text[line_start..start]){continue;}
            if a<=start&&finish<=b{self.edit(start,finish,"cjk_space","");}
        }Ok(())
    }
    fn scan(&mut self)->Result<()>{
        let text=self.text;
        let links:HashMap<_,_>=syntax::scan_links(text).into_iter().map(|l|(l.start,l)).collect();
        let mut labels=Vec::new();let mut pos=0;
        if self.rows.first().is_some_and(|l|syntax::body(text,*l).trim_start_matches('\u{feff}')=="---"){
            let end=self.rows.iter().skip(1).find(|l|matches!(syntax::body(text,**l),"---"|"...")).map(|r|r.end);
            if end.is_none(){self.issue("UNCLOSED_YAML",0,"前置 YAML 缺少结束标记。","review");}
            pos=self.protect(0,end.unwrap_or(text.len()),"yaml");
        }
        while let Some(m)=token().find_from_pos(text,pos)?{
            let a=m.start();let b=m.end();let raw=m.as_str();let row=syntax::row_at(&self.rows,a);
            let at_start=a==self.rows[row].start;let mut kind="literal";let mut end=Some(b);
            let fence=if at_start{rx!(r"^ {0,3}(\x60{3,}|~{3,})(.*)$").captures(raw)}else{None};
            if at_start&&rx!(r"^ {0,3}>").is_match(raw){
                let mut last=row;while last+1<self.rows.len()&&!syntax::body(text,self.rows[last+1]).trim().is_empty(){last+=1;}
                end=Some(self.rows[last].body_end);kind="quote";
            }else if let Some(c)=fence{
                let mark=c.get(1).unwrap().as_str();let info=c.get(2).unwrap().as_str();let marker=mark.as_bytes()[0];
                end=self.rows.iter().skip(row+1).find(|r|{
                    let line=syntax::body(text,**r);let trim=line.trim_start_matches(' ');let n=trim.bytes().take_while(|v|*v==marker).count();
                    line.len()-trim.len()<=3&&n>=mark.len()&&trim[n..].bytes().all(|b|matches!(b,b' '|b'\t'))
                }).map(|r|r.body_end);
                kind=if info.to_lowercase().contains("rich-table"){"table"}else{"fence"};
                if kind=="table"{self.issue("RICH_TABLE_PROTECTED",a,"表格按原字节边界保留，结构与字段由表格验收器核对。","info");}
            }else if at_start&&rx!(r"^ {0,3}\[(?:\\.|[^\]\\\r\n])+\]:").is_match(raw){
                let last=syntax::reference_definition_end(text,&self.rows,row).unwrap_or(row);end=Some(self.rows[last].end);
            }else if raw.starts_with('\x60'){
                let width=raw.len();let mut close=None;let mut i=b;
                while let Some(next)=text[i..].find('\x60'){
                    let start=i+next;let n=text[start..].bytes().take_while(|b|*b==b'\x60').count();
                    if n==width{close=Some(start+n);break;}i=start+n;
                }end=close;kind="code";
            }else if matches!(raw,r"\("|r"\["){
                if raw==r"\["&&(text[b..].starts_with('[')||text[..a].ends_with("<sup>")){
                    end=Some(b);kind="literal";
                }else{
                    let closer=if raw==r"\("{r"\)"}else{r"\]"};end=text[b..].find(closer).map(|i|b+i+closer.len());
                    kind=if raw==r"\("{"math"}else{"display"};
                }
            }else if matches!(raw,"<!--"|"%%"|"$$"|"$"){
                if raw=="$"{
                    let start=self.offsets.previous(a,128);
                    if let Some(s)=rx!(r"[+−-]?[0-9]+\.$").find(&text[start..a]){
                        let repair_start=start+s.start();
                        let repair=split_percent().captures_from_pos(text,repair_start)?.filter(|c|c.get(0).unwrap().start()==repair_start);
                        if let Some(c)=repair.filter(|_|repair_start>=pos){
                            let rm=c.get(0).unwrap();
                            if prefix(&text[self.offsets.previous(repair_start,36)..repair_start]){
                                self.issue("AMBIGUOUS_SPLIT_DECIMAL",a,"编号后紧接百分数，保留原文边界。","review");
                                let trailing=text[rm.end()..].bytes().take_while(|v|matches!(*v,b' '|b'\t')).count();
                                pos=self.protect(repair_start,rm.end()+trailing,"math");continue;
                            }
                            self.edit(repair_start,rm.end(),"split_percent",format!("{}{}{}\\%$",'$',c.name("a").unwrap().as_str(),c.name("b").unwrap().as_str()));
                            pos=self.protect(repair_start,rm.end(),"math");continue;
                        }
                        self.issue("AMBIGUOUS_SPLIT_DECIMAL",a,"小数点紧接数学边界，无法唯一确定数值或百分号归属。","review");
                    }
                }
                let delimiter=if raw=="<!--"{"-->"}else{raw};let mut from=b;end=None;
                while let Some(i)=text[from..].find(delimiter){
                    let i=from+i;if !syntax::escaped(text,i){end=Some(i+delimiter.len());break;}from=i+delimiter.len();
                }
                kind=match raw{"$"=>"math","$$"=>"display",_=>"comment"};
                if raw=="$"{
                    if let Some(c)=wrapped().captures_from_pos(text,a)?.filter(|c|c.get(0).unwrap().start()==a){
                        let wm=c.get(0).unwrap();let n=c.name("n").unwrap().as_str();let u=c.name("u").unwrap().as_str();
                        if allowed(text,&self.offsets,a,wm.end(),n,u)?{end=Some(wm.end());self.edit(a,wm.end(),"split_quantity",math(n,u));}
                    }
                }
            }else if raw.ends_with('['){
                end=syntax::balanced(text,b-1,'[',']');
                if let Some(e)=end{
                    if syntax::ch_at(text,e)==Some('('){
                        end=links.get(&a).map(|l|l.end);if end.is_some()&&raw=="["{labels.push((b,e-1));}
                    }else if syntax::ch_at(text,e)==Some('['){end=syntax::balanced(text,e,'[',']');}
                    else if raw=="["&&text[a..].starts_with("[["){
                        if let Some(i)=text[b..e].find('|'){let x=b+i+1;let y=e.saturating_sub(2);if x<=y{labels.push((x,y));}}
                    }
                }kind="link";
            }else if raw=="<"&&rx!(r"^(?:/?[A-Za-z!]|https?://)").is_match(&text[b..]){
                end=rx!(r#"^(?:[^>"']|"[^"]*"|'[^']*')*>"#).find(&text[b..]).map(|m|b+m.end());
                if let (Some(e),Some(c))=(end,fx!(r"(?i)^<(code|pre)(?=[\s>])").captures(&text[a..])?){
                    let tag=c.get(1).unwrap().as_str().to_ascii_lowercase();
                    let closer=if tag=="code"{rx!(r"(?i)</code\s*>")}else{rx!(r"(?i)</pre\s*>")};
                    end=closer.find(&text[e..]).map(|m|e+m.end());kind="html_code";
                }
            }
            if end.is_none(){self.issue(&format!("UNCLOSED_{}",kind.to_uppercase()),a,"未找到可确认的闭合边界，保留余下原文。","review");end=Some(text.len());}
            let mut end_val=end.unwrap();
            if kind=="display" && raw=="$$"{
                let first_row=syntax::row_at(&self.rows,a);
                let last_row=syntax::row_at(&self.rows,end_val.saturating_sub(1));
                let row_end=self.rows[last_row].body_end;
                let before_open=self.text[self.rows[first_row].start..a].trim().is_empty();
                let after_close=&self.text[end_val..row_end];
                let close_dollar_end=end_val;
                if before_open && !after_close.is_empty() && rx!(r"^[ \t]*[.,;，。；]+[ \t]*$").is_match(after_close){
                    self.edit(end_val,row_end,"display_closing_punctuation","");
                    if first_row==last_row{self.display_whole_rows.insert(last_row);}
                    self.display_end_rows.insert(last_row);
                    end_val=row_end;
                }
                if self.options.strip_display_trailing_punctuation && end.is_some() && close_dollar_end>=a+4{
                    let inner_start=a+2;
                    let inner_end=close_dollar_end-2;
                    if inner_end>inner_start{
                        let inner=&self.text[inner_start..inner_end];
                        let mut cut=inner.trim_end().len();
                        for _ in 0..2{
                            if let Some(m)=rx!(r"\s*\\tag\*?\{[^{}]*\}\s*$").find(&inner[..cut]){cut=m.start();}
                            if let Some(m)=rx!(r"(?:\\end\{(?:aligned|align\*?|gathered|gather\*?|split|equation\*?)\}\s*)+$").find(&inner[..cut]){cut=m.start();}
                            cut=inner[..cut].trim_end().len();
                        }
                        if let Some(m)=rx!(r"\\\\\s*$").find(&inner[..cut]){cut=inner[..m.start()].trim_end().len();}
                        let body=&inner[..cut];
                        if let Some(ch)=body.chars().next_back(){
                            if matches!(ch,','|';'|'，'|'；'|'。'|'.'){
                                let p_start=cut-ch.len_utf8();
                                let escaped=syntax::escaped(body,p_start);
                                let keep_dot=ch=='.'&&(body.ends_with("..")||rx!(r"\\(?:left|right|middle)\s*\.$").is_match(body));
                                if !escaped && !keep_dot{
                                    self.edit(inner_start+p_start,inner_start+cut,"display_trailing_punctuation","");
                                }
                            }
                        }
                    }
                }
            }
            pos=self.protect(a,end_val,kind);
        }
        let spans=self.spans.clone();let mut cursor=0;
        let check_wrapper_spaces=|edits:&mut Vec<Edit>,ga:usize,gb:usize|{
            if ga>=gb{return;}
            for m in rx!(r" +").find_iter(&text[ga..gb]){
                let sa=ga+m.start();let sb=ga+m.end();
                let Some((lc,lw))=visible_left_char(text,&spans,&links,sa)else{continue;};
                let Some((rc,rw))=visible_right_char(text,&spans,&links,sb)else{continue;};
                if (lw!=rw)&&!matches!(lc,' '|'\t')&&!matches!(rc,' '|'\t')&&((is_cjk(lc)&&is_latin_math(rc))||(is_latin_math(lc)&&is_cjk(rc))){
                    edits.push(Edit{a:sa,b:sb,rule:"cjk_space",after:String::new()});
                }
            }
        };
        for s in &spans{
            if cursor<s.a{
                self.prose(cursor,s.a)?;
                check_wrapper_spaces(&mut self.edits,cursor,s.a);
            }
            if s.kind=="literal" && rx!(r"^ {0,3}#{1,6}[ \t]+\S").is_match(&self.text[s.a..s.b]){
                self.heading_cjk_space(s.a,s.b)?;
            }else if s.kind=="table"{
                self.clean_rich_table_cjk_space(s.a,s.b)?;
            }
            cursor=s.b;
        }
        if cursor<text.len(){
            self.prose(cursor,text.len())?;
            check_wrapper_spaces(&mut self.edits,cursor,text.len());
        }
        if !self.heading_renames.is_empty(){
            let renames=self.heading_renames.clone();
            for s in &spans{
                if s.kind!="link"||text[s.a..].starts_with("!["){continue;}
                if let Some(l)=links.get(&s.a){
                    if !l.image && l.href.starts_with('#') && !l.href.starts_with("#^"){
                        let decoded=percent_encoding::percent_decode_str(&l.href[1..]).decode_utf8_lossy();
                        if let Some(new_h)=renames.get(decoded.as_ref()){
                            if let Some((ds,de,angle))=syntax::image_destination_span(text,l){
                                let encoded=if !angle && new_h.contains(char::is_whitespace){
                                    format!("#{}",new_h.replace(' ',"%20"))
                                }else{format!("#{new_h}")};
                                self.edit(ds,de,"heading_link_sync",encoded);
                            }
                        }
                    }
                }else if text[s.a..s.b].starts_with("[[#")&&text[s.a..s.b].ends_with("]]"){
                    let inner_start=s.a+3;
                    let inner_end=text[inner_start..s.b-2].find('|').map_or(s.b-2,|i|inner_start+i);
                    let decoded=percent_encoding::percent_decode_str(&text[inner_start..inner_end]).decode_utf8_lossy();
                    if let Some(new_h)=renames.get(decoded.as_ref()){
                        self.edit(inner_start,inner_end,"heading_link_sync",new_h.clone());
                    }
                }
            }
        }
        for(a,b)in labels{
            let s=&text[a..b];
            if !rx!(r"[\x60$\[\]<>\\/|]").is_match(s)&&!rx!(r"(?i)(?:https?://|www\.)[^\s<>]+").is_match(s)&&!rx!(r"(?i)\b10\.\d{4,9}/[^\s<>]+").is_match(s){self.prose(a,b)?;}
        }
        self.layout();Ok(())
    }
    fn layout(&mut self){
        let text=self.text;
        let mut protected=HashSet::new();let mut roles=HashMap::new();
        for s in &self.spans{
            let first=syntax::row_at(&self.rows,s.a);let last=syntax::row_at(&self.rows,s.b-1);
            if matches!(s.kind,"yaml"|"table"|"fence"|"display")||first!=last{
                protected.extend(first..=last);
                roles.insert(first,format!("{}_start",s.kind));roles.insert(last,format!("{}_end",s.kind));
                if first==last&&s.kind=="display"{
                    let whole=self.display_whole_rows.contains(&first)||syntax::whole(text,self.rows[first]).trim()==&text[s.a..s.b];
                    roles.insert(first,if whole{"display_whole"}else{"protected"}.into());
                }else if self.display_end_rows.contains(&last){
                    roles.insert(last,"display_end".into());
                }
            }
        }
        let occupied:Vec<_>=self.rows.iter().enumerate().filter(|(_,r)|!syntax::whole(text,**r).trim().is_empty()).map(|(i,_)|i).collect();
        let role=|i:usize|->String{
            if protected.contains(&i){return roles.get(&i).cloned().unwrap_or_else(||"protected".into());}
            let s=syntax::body(text,self.rows[i]);
            if s.starts_with("# "){return "h1".into();}
            if rx!(r"^ {0,3}#{1,6} +\S").is_match(s){return "heading".into();}
            if syntax::standalone_image(s){return "image".into();}
            if let Some(c)=rx!(r"(?i)^\^(fig|eq|tbl|tab)[-\w]*$").captures(s){return format!("anchor_{}",c.get(1).unwrap().as_str().to_lowercase());}
            if rx!(r"(?i)^\*\*(?:图|Fig(?:ure)?\.?)\s*[\w.-]*[：:.]").is_match(s){return "caption".into();}
            if s.starts_with([' ','\t','>','|']){"other".into()}else{"text".into()}
        };
        let roles:Vec<_>=occupied.iter().map(|i|role(*i)).collect();
        let author=rx!(r"(?i)^(?:\*\*)?(?:作者|Authors?)\s*[:：]|\(mailto:|\$\{\}\^");
        let title=rx!(r"^(?:\*\*)?(?:中译名|中文译名|中文题名|译名|中文标题|译题)\s*[:：]");
        for j in 0..occupied.len().saturating_sub(1){
            let i=occupied[j];let k=occupied[j+1];
            let mut left=roles[j].as_str();let mut right=roles[j+1].as_str();
            if protected.contains(&i)&&!(left.ends_with("_end")||left.ends_with("_whole"))||protected.contains(&k)&&!(right.ends_with("_start")||right.ends_with("_whole")){continue;}
            if right=="text"&&left=="image"&&roles.get(j+2).is_some_and(|r|r=="anchor_fig")&&figures::bold_label(syntax::body(text,self.rows[k]).trim()){right="caption";}
            if left=="text"&&right=="anchor_fig"&&j>0&&roles[j-1]=="image"&&figures::bold_label(syntax::body(text,self.rows[i]).trim()){left="caption";}
            if left=="image"&&matches!(right,"image"|"caption"){
                let raw=syntax::body(text,self.rows[i]);let stripped=raw.trim_end_matches([' ','\t']);
                if !stripped.ends_with('\\')&&raw!=format!("{stripped}  "){self.edit(self.rows[i].start+stripped.len(),self.rows[i].body_end,"image_hard_break","  ");}
            }
            let blank=if matches!(right,"heading"|"h1")||left.starts_with("anchor_"){Some(true)}
                else if left=="h1"&&right=="text"{
                    let line=syntax::whole(text,self.rows[k]);
                    if title.is_match(line){Some(false)}else if author.is_match(line){Some(true)}
                    else if j+2<occupied.len()&&author.is_match(syntax::whole(text,self.rows[occupied[j+2]]))&&rx!(r"[\u{3400}-\u{9fff}]").is_match(line){Some(false)}
                    else{self.issue("AMBIGUOUS_HEADER",self.rows[k].start,"题名之后行的角色不明，保留间距。","review");None}
                }
                else if left=="text"&&right=="text"&&j>0&&roles[j-1]=="h1"&&author.is_match(syntax::whole(text,self.rows[k])){Some(true)}
                else if left=="image"&&matches!(right,"image"|"caption")||matches!((left,right),("caption","anchor_fig")|("display_end","anchor_eq")|("display_whole","anchor_eq")|("table_end","anchor_tab")|("table_end","anchor_tbl")){Some(false)}
                else if matches!(right,"image"|"display_start"|"display_whole")||matches!(left,"display_end"|"display_whole"){Some(true)}
                else if left=="heading"&&right=="text"{Some(false)}else{None};
            if let Some(blank)=blank{
                if let Some(eol)=rx!(r"\r\n|\r|\n").find(syntax::whole(text,self.rows[i])){
                    self.edit(self.rows[i].end,self.rows[k].start,"block_spacing",if blank{eol.as_str()}else{""});
                }
            }
        }
    }
    fn finish(mut self)->(String,Vec<Value>,Vec<Value>){
        self.edits.sort_by(|a,b|a.a.cmp(&b.a).then(b.b.cmp(&a.b)));
        let mut output=String::with_capacity(self.text.len());let mut changes=Vec::new();let mut cursor=0;let mut shift=0isize;let mut line=1usize;let mut column=1usize;
        for e in self.edits{
            if e.a<cursor||self.text[e.a..e.b]==e.after{continue;}
            let untouched=&self.text[cursor..e.a];advance(&mut line,&mut column,untouched);
            output.push_str(untouched);output.push_str(&e.after);
            let a=self.offsets.index(e.a);let b=self.offsets.index(e.b);
            let stage=if matches!(e.rule,"split_percent"|"split_quantity"|"quantity"|"display_closing_punctuation"|"display_trailing_punctuation"){"math_formatting"}
                else if matches!(e.rule,"block_spacing"|"cjk_space"|"image_hard_break"|"table_cjk_space"|"line_ending"){"whitespace_formatting"}
                else if e.rule=="heading_link_sync"{"internal_links"}else{"punctuation_formatting"};
            changes.push(json!({"start":a as isize+shift,"end":b as isize+shift,"before":&self.text[e.a..e.b],"after":e.after,
                "line":line,"column":column,"stage":stage,"rule":e.rule}));
            advance(&mut line,&mut column,&e.after);
            shift+=e.after.chars().count() as isize-(b-a) as isize;cursor=e.b;
        }
        output.push_str(&self.text[cursor..]);(output,changes,self.issues)
    }
}
fn advance(line:&mut usize,column:&mut usize,s:&str){
    let mut last=None;
    for m in rx!(r"\r\n|\r|\n").find_iter(s){*line+=1;last=Some(m.end());}
    if let Some(end)=last{*column=1+s[end..].chars().count();}else{*column+=s.chars().count();}
}
fn pass_line_endings(text:&str)->(String,Vec<Value>){
    let mut out=String::with_capacity(text.len());
    let mut changes=Vec::new();
    let mut line=1usize;let mut col=1usize;let mut shift=0isize;let mut cp=0usize;
    let mut plain_start=0usize;
    let mut it=text.char_indices().peekable();
    while let Some((i,ch))=it.next(){
        if ch=='\r'{
            let mut end=i+1;
            let before=if it.peek().is_some_and(|(_,c)|*c=='\n'){it.next();end+=1;"\r\n"}else{"\r"};
            let untouched=&text[plain_start..i];
            cp+=untouched.chars().count();
            advance(&mut line,&mut col,untouched);
            out.push_str(untouched);
            out.push('\n');
            let blen=before.chars().count();
            changes.push(json!({
                "start":cp as isize+shift,"end":(cp+blen) as isize+shift,
                "before":before,"after":"\n","line":line,"column":col,
                "stage":"whitespace_formatting","rule":"line_ending"
            }));
            advance(&mut line,&mut col,"\n");
            cp+=blen;
            shift+=1isize-blen as isize;
            plain_start=end;
        }
    }
    out.push_str(&text[plain_start..]);
    (out,changes)
}
fn pass(text:&str,options:Options)->Result<(String,Vec<Value>,Vec<Value>)>{let mut pass=Pass::new(text,options);pass.scan()?;Ok(pass.finish())}
fn stable(text:&str,options:Options)->Result<(String,Vec<Value>,Vec<Value>,bool)>{
    let owned;
    let mut prefix_changes=Vec::new();
    let work=if options.normalize_line_endings&&text.contains('\r'){
        let (lf,cr_edits)=pass_line_endings(text);
        owned=lf;
        prefix_changes=cr_edits;
        owned.as_str()
    }else{text};
    let(mut candidate,mut changes,mut issues)=pass(work,options)?;
    if candidate==work&&changes.is_empty(){
        prefix_changes.extend(changes);
        return Ok((candidate,prefix_changes,issues,true));
    }
    if !prefix_changes.is_empty(){
        prefix_changes.extend(changes);
        changes=prefix_changes;
    }
    for _ in 0..3{
        let(verified,more,_)=pass(&candidate,options)?;
        if verified==candidate{return Ok((candidate,changes,issues,more.is_empty()));}
        changes.extend(more);candidate=verified;
    }
    issues.push(json!({"code":"INTERACTING_RULES","line":1,"start":0,"message":"规则互相影响，保留整份输入。"}));
    Ok((text.into(),vec![],issues,false))
}
pub fn normalize(text:&str)->Result<Value>{
    normalize_with_options(text,Options::default())
}
pub fn normalize_with_options(text:&str,options:Options)->Result<Value>{
    Ok(normalize_with_replay_proof(text,options)?.0)
}
pub(crate) fn normalize_with_replay_proof(text:&str,options:Options)->Result<(Value,bool)>{
    let(candidate,changes,unresolved,known_fixed)=stable(text,options)?;
    // The last pass already proved that a verified fixed point needs no edits.
    // Keep a separate probe for the exceptional interaction/zero-net-edit case.
    let (idempotent,empty_replay)=if known_fixed {(true,true)}else{
        let(verified,replay_changes,_,_)=stable(&candidate,options)?;
        (candidate==verified,replay_changes.is_empty())
    };
    let report=json!({"schema_version":1,"engine":"normalize_note","text":candidate,"changes":changes,"unresolved":unresolved,
        "input_sha256":files::digest(text.as_bytes()),"output_sha256":files::digest(candidate.as_bytes()),
        "idempotent":idempotent,"source_fidelity":"not_assessed"});
    Ok((report,idempotent&&empty_replay))
}
pub fn normalize_file(source:&Path,destination:&Path)->Result<Value>{
    ensure!(files::canonical(source)?!=dunce::canonicalize(destination).unwrap_or_else(|_|destination.to_path_buf()),"use a new candidate path");
    ensure!(fs::symlink_metadata(destination).is_err_and(|e|e.kind()==std::io::ErrorKind::NotFound),"destination already exists or is not accessible");
    let text=fs::read_to_string(source)?;
    let report=normalize(&text)?;
    ensure!(report["idempotent"]==true,"normalization did not reach a verified fixed point");
    let payload=report["text"].as_str().context("missing normalized text")?.as_bytes();
    let parent=destination.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut temporary=tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(payload)?;temporary.as_file().sync_all()?;
    temporary.persist_noclobber(destination)?;
    ensure!(fs::read(destination)?==payload,"normalization readback mismatch");
    Ok(report)
}

#[cfg(test)]
mod tests{
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn citation_brackets_heading_cjk_wrapper_spaces_and_display_punct(){
        let input=concat!(
            "### 2.1 Mayer 路径同调与 A 操作\n",
            "结果参见 <sup>\\[[22](<#^ref-22>)\\]</sup> 之后，另有 **粗体**。\n",
            "采用 [BERT](#ref-1) 模型与 <sup>[22](#ref-22)</sup> 方法，且 <sup>[$2$](#eq-1)</sup> 成立。\n\n",
            "> > 引用 [BERT](#ref-1) 模型\n",
            "> 其长度为 5 m。\n\n",
            "$$\nx = 1,\n$$.\n"
        );
        let res=normalize(input).unwrap();
        let out=res["text"].as_str().unwrap();
        assert!(out.contains("### 2.1 Mayer路径同调与A操作\n"),"{out}");
        assert!(out.contains("采用[BERT](#ref-1)模型与<sup>[22](#ref-22)</sup>方法，且<sup>[$2$](#eq-1)</sup>成立。"),"{out}");
        assert!(out.contains("> 其长度为 5 m。"),"{out}");
        assert!(out.contains("$$\nx = 1,\n$$\n"),"{out}");
        assert!(!out.contains("$$."),"{out}");
        assert!(res["unresolved"].as_array().unwrap().is_empty(),"{res}");

        let opted=normalize_with_options("$$\nx = 1,\n$$.\n",Options{strip_display_trailing_punctuation:true,..Options::default()}).unwrap();
        assert_eq!(opted["text"].as_str().unwrap(),"$$\nx = 1\n$$\n");
        assert_eq!(normalize("[持续60 min](https://x/a(b)—c) [[图—1|值25%]]").unwrap()["text"],r"[持续$60\,\mathrm{min}$](https://x/a(b)—c) [[图—1|值$25\%$]]");
    }

    #[test]
    fn heading_link_sync_table_cjk_space_and_quantity_filter(){
        let md=concat!(
            "## 4 持久 Mayer 路径同调\n\n",
            "见 [第4节](<#4 持久 Mayer 路径同调>)、",
            "[编码链接](#4%20%E6%8C%81%E4%B9%85%20Mayer%20%E8%B7%AF%E5%BE%84%E5%90%8C%E8%B0%83) 与 ",
            "[[#4 持久 Mayer 路径同调|别名]]。\n\n",
            "```rich-table\n",
            "id: tbl-1\n",
            "columns:\n",
            "  - id: c_000001\n",
            "    name: Foo 条目\n",
            "rows:\n",
            "  - id: r_000001\n",
            "    cells:\n",
            "      c_000001: 值 A 与 B 对比\n",
            "```\n\n",
            "在第2节与图3中，2024年N=3（UTC时间）下测量了 5–7 km。\n"
        );
        let res=normalize(md).unwrap();
        let out=res["text"].as_str().unwrap();
        assert!(out.contains("## 4 持久Mayer路径同调\n"),"{out}");
        assert!(out.contains("[第4节](<#4 持久Mayer路径同调>)"),"{out}");
        assert!(out.contains("[编码链接](#4%20持久Mayer路径同调)"),"{out}");
        assert!(out.contains("[[#4 持久Mayer路径同调|别名]]"),"{out}");
        assert!(out.contains("name: Foo条目"),"{out}");
        assert!(out.contains("c_000001: 值A与B对比"),"{out}");
        let ambig:Vec<_>=res["unresolved"].as_array().unwrap().iter().filter(|r|r["code"]=="AMBIGUOUS_QUANTITY").collect();
        assert_eq!(ambig.len(),1,"only 5-7 km should remain ambiguous: {ambig:?}");
    }

    #[test]
    fn pipeline_write_normalized_to_and_vault_guard(){
        use crate::pipeline::{process_with_options,ProcessOptions};
        let dir=tempdir().unwrap();
        let src=dir.path().join("source.txt");fs::write(&src,b"src").unwrap();
        let cand=dir.path().join("note.md");
        fs::write(&cand,"---\r\ntype: note\r\n---\r\n\r\n# Title\r\n\r\n### 1.2 Foo 条目\r\n每60 min观测。\r\n\r\n**表1：** 测试表\r\n| 列A | 列B |\r\n| --- | --- |\r\n| 值1 | 值2 |\r\n^tbl-1\r\n").unwrap();
        let out_root=dir.path().join("out");
        let direct=dir.path().join("direct.md");
        let contract=json!({"schema_version":1,"sources":{"main":{"path":src,"sha256":files::digest(b"src")}},
            "counts":{"figures":0,"equations":0,"tables":1,"bibliography":0}});
        let r=process_with_options(Some(&cand),&Value::Null,&Value::Null,&out_root,&contract,Some(dir.path()),
            ProcessOptions{write_normalized_to:Some(&direct),strip_display_trailing_punctuation:true,force_lf:true,convert_markdown_tables:true},|_,_,_|{}).unwrap();
        assert_eq!(r["status"],"mechanical_pass","{r}");
        assert_eq!(r["published_candidate"]["written"],true);
        assert_eq!(r["summary"]["tables_converted"],1);
        let norm_json:Value=serde_json::from_str(&fs::read_to_string(r["outputs"]["normalization.json"]["path"].as_str().unwrap()).unwrap()).unwrap();
        assert!(norm_json.get("text").is_none(),"normalization.json should omit full text");
        let direct_text=fs::read_to_string(&direct).unwrap();
        assert!(!direct_text.contains('\r'),"force_lf should emit LF only");
        assert!(direct_text.contains("### 1.2 Foo条目"));
        assert!(direct_text.contains("```rich-table"),"{direct_text}");

        let vault=dir.path().join("vault");
        fs::create_dir_all(vault.join(".obsidian")).unwrap();
        let blocked_target=vault.join("03_Literature/note.md");
        assert!(process_with_options(Some(&cand),&Value::Null,&Value::Null,&out_root,&contract,Some(dir.path()),
            ProcessOptions{write_normalized_to:Some(&blocked_target),..ProcessOptions::default()},|_,_,_|{}).is_err());
    }
}
