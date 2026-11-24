//! Native conservative Markdown normalizer. Internal spans use UTF-8 byte
//! offsets; the published change ledger uses Unicode code-point offsets.
use crate::{files,figures,syntax::{self,Offsets,Line}};
use anyhow::{ensure,Result,Context};
use fancy_regex::Regex as Fancy;
use regex::Regex;
use serde_json::{json,Value};
use std::{collections::{HashMap,HashSet},fs,io::Write,path::Path,sync::OnceLock};

macro_rules! rx {($s:expr)=>{{static R:OnceLock<Regex>=OnceLock::new();R.get_or_init(||Regex::new($s).expect("static regex"))}}}
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
fn prefix(s:&str)->bool{rx!(r"(?i)(?:第|图|表|公式|方程|章节|文献|Fig\.?|Figure|Table|Eq\.?)\s*$").is_match(s)}
fn allowed(text:&str,offsets:&Offsets,a:usize,b:usize,n:&str,u:&str)->Result<bool>{
    let before=&text[offsets.previous(a,36)..a];let after=&text[b..offsets.next(b,32)];
    if prefix(before)||rx!(r"[0-9]\s*(?:[-−–—~～至到×±]|[eE][+-]?)\s*$").is_match(before)||
        rx!(r"^(?:[/^²³·]|\s*[×±]|每|/)").is_match(after)||
        fx!(r"^[ \t]+(?:min|[kcm]?m|[kmg]?g|s|h|Hz|Pa)(?=[⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻^/·\s.,，。；;]|$)").is_match(after)?||
        before.trim_end().ends_with(['×','±','·','/'])||(u=="年"&&after.starts_with('级'))||
        rx!(r"(?i)(?:UTC\s*|[0-9][,，]|[±×])$").is_match(before)||rx!(r"(?i)^\s*UTC\b").is_match(after){return Ok(false);}
    if u=="节"&&rx!(r"^(?:课|目|点|能|选|[、.．])").is_match(after){return Ok(false);}
    if u=="年"&&n.len()==4&&n.bytes().all(|b|b.is_ascii_digit())&&n.parse::<u32>().is_ok_and(|y|(1500..=2199).contains(&y)){return Ok(false);}
    Ok(true)
}
fn math(n:&str,u:&str)->String{
    if matches!(u,"%"|"％"){return format!("{}{n}\\%$",'$');}
    let cmd=if u.chars().any(|c|('\u{3400}'..='\u{9fff}').contains(&c)){"text"}else{"mathrm"};
    format!("{}{n}\\,\\{cmd}{{{u}}}$",'$')
}
#[derive(Clone,Debug)]
struct Span{a:usize,b:usize,kind:&'static str}
#[derive(Clone,Debug)]
struct Edit{a:usize,b:usize,rule:&'static str,after:String}
struct Pass<'a>{text:&'a str,rows:Vec<Line>,offsets:Offsets,spans:Vec<Span>,edits:Vec<Edit>,issues:Vec<Value>}
impl<'a> Pass<'a>{
    fn new(text:&'a str)->Self{Self{text,rows:syntax::lines(text),offsets:Offsets::new(text),spans:vec![],edits:vec![],issues:vec![]}}
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
            if allowed(text,&self.offsets,m.start(),m.end(),n,u)?{self.edit(m.start(),m.end(),"quantity",math(n,u));}
            else{self.issue("AMBIGUOUS_QUANTITY",m.start(),"编号、日期、范围或复合单位需核对。","review");}
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
            if rx!(r"\d\s*$").is_match(&text[self.offsets.previous(start,8).max(a)..start])&&rx!(r"^\s*\d").is_match(&text[finish..b]){
                self.issue("AMBIGUOUS_RANGE",start,"保留数值之间的横线，待确认是范围还是减法。","review");
            }else{self.edit(start,finish,"dash","-");}
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
                let closer=if raw==r"\("{r"\)"}else{r"\]"};end=text[b..].find(closer).map(|i|b+i+closer.len());
                kind=if raw==r"\("{"math"}else{"display"};
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
            }else if raw=="<"{
                if rx!(r"^(?:/?[A-Za-z!]|https?://)").is_match(&text[b..]){
                    end=rx!(r#"^(?:[^>"']|"[^"]*"|'[^']*')*>"#).find(&text[b..]).map(|m|b+m.end());
                    if let Some(c)=fx!(r"(?i)^<(code|pre)(?=[\s>])").captures(&text[a..])?{
                        if let Some(e)=end{
                            let tag=c.get(1).unwrap().as_str().to_ascii_lowercase();
                            let closer=if tag=="code"{rx!(r"(?i)</code\s*>")}else{rx!(r"(?i)</pre\s*>")};
                            end=closer.find(&text[e..]).map(|m|e+m.end());kind="html_code";
                        }
                    }
                }
            }
            if end.is_none(){self.issue(&format!("UNCLOSED_{}",kind.to_uppercase()),a,"未找到可确认的闭合边界，保留余下原文。","review");end=Some(text.len());}
            pos=self.protect(a,end.unwrap(),kind);
        }
        let spans=self.spans.clone();let mut cursor=0;
        for s in &spans{if cursor<s.a{self.prose(cursor,s.a)?;}cursor=s.b;}
        if cursor<text.len(){self.prose(cursor,text.len())?;}
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
                    let whole=syntax::whole(text,self.rows[first]).trim()==&text[s.a..s.b];
                    roles.insert(first,if whole{"display_whole"}else{"protected"}.into());
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
            let stage=if matches!(e.rule,"split_percent"|"split_quantity"|"quantity"){"math_formatting"}
                else if matches!(e.rule,"block_spacing"|"cjk_space"|"image_hard_break"){"whitespace_formatting"}else{"punctuation_formatting"};
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
fn pass(text:&str)->Result<(String,Vec<Value>,Vec<Value>)>{let mut pass=Pass::new(text);pass.scan()?;Ok(pass.finish())}
fn stable(text:&str)->Result<(String,Vec<Value>,Vec<Value>)>{
    let(mut candidate,mut changes,mut issues)=pass(text)?;
    for _ in 0..3{
        let(verified,more,_)=pass(&candidate)?;
        if verified==candidate{return Ok((candidate,changes,issues));}
        changes.extend(more);candidate=verified;
    }
    issues.push(json!({"code":"INTERACTING_RULES","line":1,"start":0,"message":"规则互相影响，保留整份输入。"}));
    Ok((text.into(),vec![],issues))
}
pub fn normalize(text:&str)->Result<Value>{
    let(candidate,changes,unresolved)=stable(text)?;
    let(verified,_,_)=stable(&candidate)?;
    Ok(json!({"schema_version":1,"engine":"normalize_note","text":candidate,"changes":changes,"unresolved":unresolved,
        "input_sha256":files::digest(text.as_bytes()),"output_sha256":files::digest(candidate.as_bytes()),
        "idempotent":candidate==verified,"source_fidelity":"not_assessed"}))
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
