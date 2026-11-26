//! Shared source-preserving Markdown lexical helpers. All internal offsets
//! are byte positions on valid UTF-8 boundaries; reports convert to code points.
use regex::Regex;
use std::{collections::BTreeMap,sync::OnceLock};

#[derive(Clone,Copy,Debug)]
pub struct Line { pub start:usize,pub end:usize,pub body_end:usize }
pub fn lines(text:&str)->Vec<Line>{
    let mut result=Vec::new();let mut start=0;
    let mut chars=text.char_indices().peekable();
    while let Some((i,ch))=chars.next(){
        if matches!(ch,'\r'|'\n'|'\u{b}'|'\u{c}'|'\u{1c}'|'\u{1d}'|'\u{1e}'|'\u{85}'|'\u{2028}'|'\u{2029}'){
            let mut end=i+ch.len_utf8();
            if ch=='\r'&&chars.peek().is_some_and(|(_,c)|*c=='\n'){chars.next();end+=1;}
            let body_end=if matches!(ch,'\r'|'\n'){i}else{end};
            result.push(Line{start,end,body_end});start=end;
        }
    }
    if start<text.len(){result.push(Line{start,end:text.len(),body_end:text.len()});}
    result
}
pub fn row_at(rows:&[Line],at:usize)->usize{rows.partition_point(|r|r.start<=at).saturating_sub(1)}
pub fn body(text:&str,row:Line)->&str{&text[row.start..row.body_end]}
pub fn whole(text:&str,row:Line)->&str{&text[row.start..row.end]}
pub fn ch_at(text:&str,p:usize)->Option<char>{text.get(p..)?.chars().next()}
pub fn escaped(text:&str,p:usize)->bool{text.as_bytes()[..p].iter().rev().take_while(|b|**b==b'\\').count()%2==1}
pub fn balanced(text:&str,pos:usize,open:char,close:char)->Option<usize>{
    let mut depth=0usize;
    for (i,c) in text.get(pos..)?.char_indices(){
        let i=pos+i;
        if !matches!(c,x if x==open||x==close)||escaped(text,i){continue;}
        if c==open{depth+=1;}else{depth=depth.checked_sub(1)?;}
        if depth==0{return Some(i+c.len_utf8());}
    }None
}
fn skip_space(text:&str,mut p:usize)->usize{while let Some(c)=ch_at(text,p){if !c.is_whitespace(){break;}p+=c.len_utf8();}p}
fn skip_escape(text:&str,p:usize)->usize{let next=p+1;next+ch_at(text,next).map(char::len_utf8).unwrap_or(0)}
pub struct Link{pub start:usize,pub end:usize,pub label_start:usize,pub label_end:usize,pub image:bool,pub href:String}
pub(crate) fn image_destination_span(text:&str,link:&Link)->Option<(usize,usize,bool)>{
    let close=link.label_end;
    if ch_at(text,close+1)!=Some('('){return None;}
    let mut cursor=skip_space(text,close+2);
    let angle=ch_at(text,cursor)==Some('<');
    cursor+=usize::from(angle);
    let destination_start=cursor;
    let mut depth=0usize;
    while let Some(c)=ch_at(text,cursor){
        if c=='\\'&&cursor+1<text.len(){cursor=skip_escape(text,cursor);continue;}
        if angle{
            if matches!(c,'>'|'\r'|'\n'){break;}
        }else{
            if c.is_whitespace()||(c==')'&&depth==0){break;}
            if c=='('{depth+=1;}else if c==')'{depth-=1;}
        }
        cursor+=c.len_utf8();
    }
    Some((destination_start,cursor,angle))
}
pub fn scan_links(text:&str)->Vec<Link>{
    let mut stack=Vec::new();let mut pairs=BTreeMap::new();let mut p=0;
    while let Some(c)=ch_at(text,p){
        if c=='\\'{p=skip_escape(text,p);continue;}
        if c=='['{stack.push(p);}
        else if c==']'{if let Some(a)=stack.pop(){pairs.insert(a,p);}}
        p+=c.len_utf8();
    }
    let mut result=Vec::new();let mut consumed=0;
    for(open,close)in pairs{
        if open<consumed||ch_at(text,close+1)!=Some('('){continue;}
        let mut cursor=skip_space(text,close+2);
        let angle=ch_at(text,cursor)==Some('<');
        cursor+=usize::from(angle);
        let destination_start=cursor;
        let mut depth=0usize;
        while let Some(c)=ch_at(text,cursor){
            if c=='\\'&&cursor+1<text.len(){cursor=skip_escape(text,cursor);continue;}
            if angle{
                if matches!(c,'>'|'\r'|'\n'){break;}
            }else{
                if c.is_whitespace()||(c==')'&&depth==0){break;}
                if c=='('{depth+=1;}else if c==')'{depth-=1;}
            }
            cursor+=c.len_utf8();
        }
        if cursor>=text.len()||(angle&&ch_at(text,cursor)!=Some('>'))||depth!=0{continue;}
        let href=decode_destination(&text[destination_start..cursor]);
        cursor+=usize::from(angle);let whitespace=cursor;cursor=skip_space(text,cursor);
        if cursor<text.len()&&ch_at(text,cursor)!=Some(')'){
            let Some(c)=ch_at(text,cursor)else{continue;};
            if cursor==whitespace||!matches!(c,'\''|'"'|'('){continue;}
            let end=if c=='(' {')'}else{c};cursor+=1;
            while let Some(c)=ch_at(text,cursor){if c==end&&!escaped(text,cursor){break;}cursor+=c.len_utf8();}
            if cursor>=text.len(){continue;}
            cursor=skip_space(text,cursor+1);
        }
        if ch_at(text,cursor)!=Some(')'){continue;}
        let image=open>0&&text.as_bytes()[open-1]==b'!'&&!escaped(text,open-1);
        let start=open-usize::from(image);consumed=cursor+1;
        result.push(Link{start,end:consumed,label_start:open+1,label_end:close,image,href});
    }result
}
pub(crate) struct ReferenceDefinition<'a>{
    pub label:&'a str,
    pub href:String,
    pub end:usize,
}
pub(crate) fn reference_definition<'a>(text:&'a str,rows:&[Line],start:usize)->Option<ReferenceDefinition<'a>>{
    static DEFINITION:OnceLock<Regex>=OnceLock::new();
    static DESTINATION:OnceLock<Regex>=OnceLock::new();
    let captures=DEFINITION.get_or_init(||Regex::new(r"^ {0,3}\[(?P<label>(?:\\.|[^\]\\\r\n])+)\]:[ \t]*(?P<payload>.*)$").unwrap()).captures(body(text,rows[start]))?;
    let label=captures.name("label")?.as_str();
    if label.trim().is_empty()||label.starts_with('^'){return None;}
    let mut end=start;let mut payload=captures.name("payload")?.as_str().trim_start();
    if payload.is_empty()&&end+1<rows.len(){end+=1;payload=body(text,rows[end]).trim_start();}
    let destination=DESTINATION.get_or_init(||Regex::new(r"^(?:<(?:\\.|[^\\<>\r\n])*>|(?:\\.|[^\s])+)").unwrap()).find(payload)?;
    let target=destination.as_str();let synthetic=format!("[reference]({target})");
    let mut links=scan_links(&synthetic);
    if links.len()!=1||links[0].end!=synthetic.len(){return None;}
    let href=links.pop()?.href;
    let mut tail=payload[destination.end()..].trim_start();
    let same_line=!tail.is_empty();let dest_end=end;
    if tail.is_empty()&&end+1<rows.len(){end+=1;tail=body(text,rows[end]).trim_start();}
    let definition=|end|ReferenceDefinition{label,href:href.clone(),end};
    let fallback=||(!same_line).then(||definition(dest_end));
    let Some(first)=tail.chars().next().filter(|c|matches!(c,'\''|'"'|'('))else{return fallback();};
    let closing=if first=='(' {')'}else{first};let mut parts=Vec::new();let mut cursor=1;
    loop{
        if let Some((i,_))=tail[cursor..].char_indices().map(|(i,c)|(i+cursor,c)).find(|(i,c)|*c==closing&&!escaped(tail,*i)){
            if !tail[i+1..].trim().is_empty(){return fallback();}
            parts.push(&tail[..i+1]);
            let synthetic=format!("[reference]({target} {})",parts.join("\n"));
            let links=scan_links(&synthetic);
            return if links.len()==1&&links[0].end==synthetic.len(){Some(definition(end))}else{fallback()};
        }
        parts.push(tail);end+=1;
        if end>=rows.len()||body(text,rows[end]).trim().is_empty(){return fallback();}
        tail=body(text,rows[end]);cursor=0;
    }
}
pub fn reference_definition_end(text:&str,rows:&[Line],start:usize)->Option<usize>{
    reference_definition(text,rows,start).map(|definition|definition.end)
}
pub fn standalone_image(line:&str)->bool{
    static IMAGE:OnceLock<Regex>=OnceLock::new();
    let Some(c)=IMAGE.get_or_init(||Regex::new(r"^ {0,3}(?:!\[\[(?P<wiki>[^\]\r\n]+)\]\]|!\[(?:\\.|[^\]\\\r\n])*\]\((?P<markdown><[^>\r\n]*>|[^()\r\n]*)\))(?:\\)?[ \t]*$").unwrap()).captures(line)else{return false;};
    if c.name("markdown").is_some(){return true;}
    let target=c.name("wiki").unwrap().as_str().split('|').next().unwrap().split('#').next().unwrap();
    let ext=target.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    target.contains('.')&&matches!(ext.as_str(),"avif"|"bmp"|"gif"|"jpeg"|"jpg"|"png"|"svg"|"webp")
}
pub struct Offsets{points:Vec<usize>}
impl Offsets{
    pub fn new(text:&str)->Self{let mut points:Vec<_>=text.char_indices().map(|(i,_)|i).collect();points.push(text.len());Self{points}}
    pub fn index(&self,byte:usize)->usize{self.points.binary_search(&byte).expect("UTF-8 boundary")}
    pub fn previous(&self,byte:usize,count:usize)->usize{self.points[self.index(byte).saturating_sub(count)]}
    pub fn next(&self,byte:usize,count:usize)->usize{self.points[(self.index(byte)+count).min(self.points.len()-1)]}
    pub fn remap(&self,other:&Offsets,other_byte:usize)->usize{self.points[other.index(other_byte)]}
}

pub fn decode_destination(text:&str)->String{
    let mut unescaped=String::with_capacity(text.len());
    let mut chars=text.chars().peekable();
    while let Some(c)=chars.next(){
        if c=='\\'&&chars.peek().is_some_and(|x|x.is_ascii_punctuation()){
            unescaped.push(chars.next().unwrap());
        }else{unescaped.push(c);}
    }
    html_escape::decode_html_entities(&unescaped).into_owned()
}
