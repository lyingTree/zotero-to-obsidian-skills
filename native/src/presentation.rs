//! Native read-only presentation diagnostics. No automatic repair or fidelity claim.
use crate::{syntax,validate,properties};
use anyhow::Result;
use fancy_regex::{Regex,Captures};
use serde_json::Value;
use std::{collections::{HashMap,HashSet},sync::OnceLock,path::Path,fs};
macro_rules! re{($pat:expr)=>{{static RE:OnceLock<Regex>=OnceLock::new();RE.get_or_init(||Regex::new($pat).expect("validated presentation regex"))}}}
fn all<'a>(r:&Regex,s:&'a str)->Result<Vec<fancy_regex::Match<'a>>>{Ok(r.find_iter(s).collect::<std::result::Result<Vec<_>,_>>()?)}
fn captures<'a>(r:&Regex,s:&'a str)->Result<Vec<Captures<'a>>>{Ok(r.captures_iter(s).collect::<std::result::Result<Vec<_>,_>>()?)}
fn found(r:&Regex,s:&str)->Result<bool>{Ok(r.is_match(s)?)}
fn whole(r:&Regex,s:&str)->Result<bool>{Ok(r.find(s)?.is_some_and(|m|m.start()==0&&m.end()==s.len()))}
fn start(r:&Regex,s:&str)->Result<bool>{Ok(r.find(s)?.is_some_and(|m|m.start()==0))}
fn group<'a>(c:&Captures<'a>,n:&str)->&'a str{c.name(n).map(|m|m.as_str()).unwrap_or("")}
fn replace(r:&Regex,s:&str,replacement:&str)->Result<String>{
    let mut out=String::with_capacity(s.len());let mut pos=0;
    for m in all(r,s)?{out.push_str(&s[pos..m.start()]);out.push_str(replacement);pos=m.end();}
    out.push_str(&s[pos..]);Ok(out)
}
fn excerpt_chars(s:&str,n:usize)->String{s.chars().take(n).collect()}
#[derive(Clone,Debug,PartialEq,Eq,Hash,serde::Serialize)]
pub struct Prompt{pub rule_id:&'static str,pub line:usize,pub code:&'static str,pub excerpt:String,pub message:String,pub level:&'static str}
fn add(out:&mut Vec<Prompt>,rule:&'static str,line:usize,code:&'static str,excerpt:impl Into<String>,message:impl Into<String>){
    out.push(Prompt{rule_id:rule,line,code,excerpt:excerpt.into(),message:message.into(),level:"review"});
}
fn split_lines(s:&str)->Vec<&str>{
    // Python splitlines recognizes Unicode line separators, but the raw
    // characters inside each line are preserved.
    syntax::lines(s).iter().map(|l|s[l.start..l.end].trim_end_matches(['\r','\n','\u{b}','\u{c}','\u{1c}','\u{1d}','\u{1e}','\u{85}','\u{2028}','\u{2029}'])).collect()
}
fn replace_links(text:&str,labels:bool)->String{
    let mut out=String::with_capacity(text.len());let mut start=0;
    for link in syntax::scan_links(text){
        out.push_str(&text[start..link.start]);
        if labels&&!link.image{out.push_str(&text[link.label_start..link.label_end]);}
        start=link.end;
    }
    out.push_str(&text[start..]);out
}
fn visible(text:&str)->Result<String>{
    let text=validate::mask_delimited(text,'\x60');
    let mut active=None;let mut rows=Vec::new();
    for line in split_lines(&text){let (line,c)=validate::mask_comments(line,active);active=c;rows.push(line);}
    let value=replace_links(&rows.join("\n"),true);
    let mut result=String::new();let mut pos=0;
    for c in captures(wiki(),&value)?{
        let m=c.get(0).unwrap();result.push_str(&value[pos..m.start()]);
        if !(m.start()>0&&value.as_bytes()[m.start()-1]==b'!'&&!syntax::escaped(&value,m.start()-1)){
            let inner=c.get(1).unwrap().as_str();result.push_str(inner.split_once('|').map(|(_,s)|s).unwrap_or(inner));
        }
        pos=m.end();
    }
    result.push_str(&value[pos..]);replace(re!(r"<[^>]*>"),&result,"")
}
fn scan_math(text:&str,line:usize,display:bool,out:&mut Vec<Prompt>)->Result<()>{
    for m in all(math_number_range(),text)?{
        add(out,"BODY-C185",line,"MATH_RANGE_OR_SUBTRACTION",m.as_str(),r"对照来源判断这是范围还是减法；范围使用 \text{-}，真实减法保留运算符。");
    }
    for m in all(bare_degree_command(),text)?{
        add(out,"BODY-C183",line,"DEGREE_COMMAND_MISSING_BACKSLASH",m.as_str(),r"核对来源角度；^circ 会被解析为字母幂，角度单位应使用 ^\circ 或 ^{\circ}。此提示不自动改写来源。");
    }
    if !display{return Ok(());}
    let mut tail=text.trim_end().to_owned();
    for _ in 0..2{
        tail=replace(re!(r"\s*\\tag\*?\{[^{}]*\}\s*$"),&tail,"")?.trim_end().to_owned();
        tail=replace(re!(r"(?:\\end\{(?:aligned|align\*?|gathered|gather\*?|split|equation\*?)\}\s*)+$"),&tail,"")?.trim_end().to_owned();
    }
    tail=replace(re!(r"\\\\\s*$"),&tail,"")?.trim_end().to_owned();
    if found(re!(r"(?<!\\)[,;，；。.]$"),&tail)?&&!(tail.ends_with('.')&&(tail.ends_with("..")||found(re!(r"\\(?:left|right|middle)\s*\.$"),&tail)?)){
        let count=tail.chars().count();let excerpt=tail.chars().skip(count.saturating_sub(40)).collect::<String>();
        add(out,"BODY-C202",line,"DISPLAY_TRAILING_PUNCTUATION",excerpt,"核对显示公式末尾的标点；正文标点归入正文，数学分隔符保留。");
    }Ok(())
}
struct PanelLink<'a>{start:usize,end:usize,href:String,label:&'a str}
fn scan_panels(text:&str,line:usize,out:&mut Vec<Prompt>)->Result<()>{
    let masked=validate::mask_delimited(&validate::mask_delimited(text,'\x60'),'$');
    let mut links:Vec<_>=syntax::scan_links(&masked).into_iter().filter(|l|!l.image).map(|l|PanelLink{
        start:l.start,end:l.end,href:l.href,label:&masked[l.label_start..l.label_end]}).collect();
    for c in captures(wiki(),&masked)?{
        let m=c.get(0).unwrap();
        if syntax::escaped(&masked,m.start())||(m.start()>0&&masked.as_bytes()[m.start()-1]==b'!'){continue;}
        if let Some((target,label))=c.get(1).unwrap().as_str().split_once('|'){
            links.push(PanelLink{start:m.start(),end:m.end(),href:target.to_owned(),label});
        }
    }
    links.sort_by_key(|l|l.start);
    let offsets=syntax::Offsets::new(&masked);let original:Vec<_>=text.char_indices().map(|(i,_)|i).chain(std::iter::once(text.len())).collect();
    for pair in links.windows(2){
        let(l,r)=(&pair[0],&pair[1]);
        let Some(a)=panel_label().captures(l.label)?.filter(|c|c.get(0).is_some_and(|m|m.start()==0&&m.end()==l.label.len()))else{continue;};
        let Some(b)=panel_label().captures(r.label)?.filter(|c|c.get(0).is_some_and(|m|m.start()==0&&m.end()==r.label.len()))else{continue;};
        if group(&a,"prefix").is_empty()&&group(&b,"prefix").is_empty(){continue;}
        if group(&a,"object").to_ascii_lowercase()!=group(&b,"object").to_ascii_lowercase()||l.href!=r.href||r.start<l.end{continue;}
        if !whole(panel_join(),&masked[l.end..r.start])?{continue;}
        add(out,"LINK-C13",line,"SPLIT_PANEL_CALL",&text[original[offsets.index(l.start)]..original[offsets.index(r.end)]],
            "同一对象的同组多面板调用应整体链接；核对来源调用边界后处理本次范围内的新建调用。");
    }Ok(())
}
fn calendar(number:&str,unit:&str)->Result<bool>{
    if unit!="年"{return Ok(false);}
    let number=number.replace(r"\text{-}","-").replace(r"\mathrm{-}","-");
    let digits="零〇一二三四五六七八九";let vals=b"00123456789";
    let value:String=number.chars().map(|c|digits.chars().position(|d|d==c).map(|i|vals[i] as char).unwrap_or(c)).collect();
    let value=value.trim();let mut prev=0;let mut years=Vec::new();
    for m in all(re!(r"\s*[-\x{2010}-\x{2015}至到]\s*"),value)?{years.push(&value[prev..m.start()]);prev=m.end();}
    years.push(&value[prev..]);
    Ok(years.iter().all(|y|y.parse::<u32>().is_ok_and(|n|(1500..=2199).contains(&n))))
}
fn scan(text:&str,line:usize,out:&mut Vec<Prompt>)->Result<()>{
    if text.trim().is_empty(){return Ok(());}
    let mut active=None;let mut rows=Vec::new();
    for s in split_lines(text){let (s,c)=validate::mask_comments(s,active);active=c;rows.push(s);}
    let text=rows.join("\n");
    scan_panels(&text,line,out)?;
    let math_visible=visible(&text)?;
    for math in all(inline_math(),&math_visible)?{
        let number=&math.as_str()[1..math.as_str().len()-1];
        scan_math(number,line,false,out)?;
    }
    for math in all(inline_math(),&math_visible)?{
        let tail=&math_visible[math.end()..];
        let unit=adjacent_unit().captures(tail)?.filter(|c|c.get(0).unwrap().start()==0);
        let unit=match unit{Some(u)=>Some(u),None=>adjacent_english_duration().captures(tail)?.filter(|c|c.get(0).unwrap().start()==0)};
        if let Some(unit)=unit{
            let number=&math.as_str()[1..math.as_str().len()-1];
            if !calendar(number,group(&unit,"unit"))?{
                add(out,"BODY-C169",line,"QUANTITY_SPLIT_MATH",&math_visible[math.start()..math.end()+unit.get(0).unwrap().end()],
                    "数值已在数学环境内而单位仍在外；核对量的含义后将完整量值写入同一数学环境。");
            }
        }
    }
    let naked=replace_links(&validate::mask_delimited(&text,'\x60'),false);
    let naked=replace(wiki(),&naked,"")?;
    let naked=replace(re!(r"(?i)<(?:https?://|mailto:)[^>]+>"),&naked,"")?;
    let naked=replace(inline_math(),&naked,"")?;
    for (r,rule,code,msg) in [(email(),"BODY-C260","NAKED_EMAIL","核对邮箱来源后建立 mailto 链接。"),
        (doi(),"BODY-C269","NAKED_DOI","核对 DOI 身份后建立对应链接。"),
        (url(),"BODY-C54","NAKED_URL","核对网址边界后建立 Markdown 链接。")]{
        for m in all(r,&naked)?{add(out,rule,line,code,m.as_str(),msg);}
    }
    let v=visible(&text)?;
    if found(re!(r"^ {0,3}>\s"),&v)?{
        if all(header_label(),&v)?.len()>1{add(out,"HDR-C30",line,"HEADER_ITEMS_SAME_LINE",v.trim(),"题头元信息的一行包含多个带标签的项目；每个日期类别与 DOI 分别占一条引用行。");}
        if found(date_label(),&v)?{for m in all(date_placeholder(),&v)?{add(out,"HDR-C42",line,"HEADER_DATE_PLACEHOLDER",m.as_str(),"日期仍是占位文本；放大实际源页的日期区域核对可见值。若可见来源确实只有占位，保存该区域与具体观察后记录非问题依据。");}}
    }
    for m in all(non_ascii_dash(),&v)?{add(out,"BODY-C173",line,"VISIBLE_DASH",m.as_str(),"核对该可见连接线是否应使用 ASCII -；不得修改链接目标。");}
    let prose=replace(inline_math(),&v,"ⓜ")?;
    for m in all(percent(),&prose)?{add(out,"BODY-C184",line,"PERCENT_OUTSIDE_MATH",m.as_str(),r"百分量整体置于数学环境，百分号写作 \%。");}
    for r in [quantity(),english_duration(),chinese_quantity(),century_duration()]{
        for c in captures(r,&prose)?{
            if calendar(group(&c,"number"),group(&c,"unit"))?{continue;}
            add(out,"BODY-C169",line,"QUANTITY_OUTSIDE_MATH",c.get(0).unwrap().as_str(),"按完整原句核对这是量值而非日期/编号；适用时数值与单位放入同一数学环境，保留原有近似限定。");
        }
    }
    for c in captures(prose_number_range(),&prose)?{
        let m=c.get(0).unwrap();
        if prose[..m.start()].trim_end().ends_with(['第','图','表','式','节','章','页']){continue;}
        if prose[m.end()..].trim_start().starts_with('年')&&calendar(&format!("{}-{}",group(&c,"first"),group(&c,"last")),"年")?{continue;}
        add(out,"BODY-C180",line,"NUMERIC_RANGE_OUTSIDE_MATH",m.as_str(),r"核对这是否为量值范围；量值范围整体放入数学环境，并以 \text{-} 保留范围关系。日期、UTC时刻和文献/对象编号按各自规则保留。");
    }
    let prefix=numbered_heading_prefix().find(&prose)?;
    for m in all(space(),&prose)?{
        if prefix.is_some_and(|p|m.end()<=p.end()){continue;}
        add(out,"BODY-C69",line,"CJK_SPACE",m.as_str(),"核对并移除中文与数字、拉丁字母或数学环境之间的多余空格。");
    }Ok(())
}

fn wiki()->&'static Regex{re!(r###"\[\[([^\]\n]+)\]\]"###)}
fn inline_math()->&'static Regex{re!(r###"(?<!\\)\$(?!\$)(?:\\.|[^$\n])*(?<!\\)\$"###)}
fn email()->&'static Regex{re!(r###"(?<![\w@])[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+"###)}
fn doi()->&'static Regex{re!(r###"(?i)\b10\.\d{4,9}/[^\s<>]+"###)}
fn url()->&'static Regex{re!(r###"(?i)(?:https?://|www\.)[^\s<>]+"###)}
fn percent()->&'static Regex{re!(r###"\d+(?:\.\d+)?\s*[%％]"###)}
fn non_ascii_dash()->&'static Regex{re!(r###"[\x{2010}-\x{2015}]"###)}
fn space()->&'static Regex{re!(r###"[\x{3400}-\x{9fff}] +(?=[0-9A-Za-zⓜ])|[0-9A-Za-zⓜ] +(?=[\x{3400}-\x{9fff}])"###)}
fn numbered_heading_prefix()->&'static Regex{re!(r###"^ {0,3}#{1,6} +[0-9]+(?:\.[0-9]+)* +"###)}
fn heading()->&'static Regex{re!(r###"^ {0,3}#{1,6} +\S"###)}
fn panel_label()->&'static Regex{re!(r###"(?i)(?P<prefix>图)?\s*(?P<object>S?\d+)(?P<panel>[a-z])"###)}
fn panel_join()->&'static Regex{re!(r###"(?:\s|[,，、和及与\-\x{2010}-\x{2015}]|以及)*"###)}
fn math_number_range()->&'static Regex{re!(r###"(?<![0-9A-Za-z_.])\d+(?:\.\d+)?\s*-\s*\d+(?:\.\d+)?(?![0-9.])"###)}
fn bare_degree_command()->&'static Regex{re!(r###"(?<!\\)\^\s*(?:\{\s*)?circ\b"###)}
fn quantity()->&'static Regex{re!(r###"(?<![0-9A-Za-z_.])(?P<number>\d+(?:\.\d+)?(?:\s*[-\x{2010}-\x{2015}]\s*\d+(?:\.\d+)?)?)\s*(?P<unit>小时|分钟|秒|千米|公里|厘米|毫米|米|年|km|cm|mm|m|h|s)(?![A-Za-z])"###)}
fn english_duration()->&'static Regex{re!(r###"(?<![0-9A-Za-z_.])(?P<number>\d+(?:\.\d+)?(?:\s*[-\x{2010}-\x{2015}]\s*\d+(?:\.\d+)?)?)\s*(?:[-\x{2010}-\x{2015}]\s*)?(?P<unit>(?i:seconds?|minutes?|hours?|days?|weeks?|months?|years?|decades?|century|centuries))(?![A-Za-z])"###)}
fn adjacent_english_duration()->&'static Regex{re!(r###"\s*(?:[-\x{2010}-\x{2015}]\s*)?(?P<unit>(?i:seconds?|minutes?|hours?|days?|weeks?|months?|years?|decades?|century|centuries))(?![A-Za-z])"###)}
fn letter_list_item()->&'static Regex{re!(r###"(?i)^ {0,3}(?:\*\*|__)?(?:\((?P<parenthesized>[a-z])\)|(?P<plain>[a-z])[).、])(?:\*\*|__)?\s+\S"###)}
fn figure_caption_no_space()->&'static Regex{re!(r###"^ {0,3}\*\*图(?P<number>[A-Za-z0-9](?:\\.|[^\r\n：*])*)：\*\*(?P<body>\S)"###)}
fn chinese_quantity()->&'static Regex{re!(r###"(?P<number>[数几]?[零〇一二三四五六七八九十百千万亿两半]+(?:[-\x{2010}-\x{2015}至到][数几]?[零〇一二三四五六七八九十百千万亿两半]+)?)\s*(?P<unit>小时|分钟|秒|千米|公里|厘米|毫米|米|年)"###)}
fn century_duration()->&'static Regex{re!(r###"(?<![第0-9零〇一二三四五六七八九十百千万亿两半数几])(?P<number>(?:[数几]?(?:[零〇一二三四五六七八九十百千万亿两半]+|\d+)个(?:多|余)?|[数几]个?))\s*(?P<unit>世纪)"###)}
fn prose_number_range()->&'static Regex{re!(r###"(?<![0-9A-Za-z_.:])(?P<first>[+-]?\d+(?:\.\d+)?)\s*(?:至|到)\s*(?P<last>[+-]?\d+(?:\.\d+)?)(?![0-9.:])"###)}
fn adjacent_unit()->&'static Regex{re!(r###"\s*(?P<unit>小时|分钟|秒|千米|公里|厘米|毫米|米|年|km|cm|mm|m|h|s)(?![A-Za-z])"###)}
fn header_label()->&'static Regex{re!(r###"(?i)(?<![A-Za-z])(?:Received|Accepted|Published(?:\s+online)?|Publication\s+date|DOI)\s*[:：]|(?:收稿|接收|接受|录用|在线发表|发表|出版)(?:日期)?\s*[:：]"###)}
fn date_label()->&'static Regex{re!(r###"(?i)(?<![A-Za-z])(?:Received|Accepted|Published(?:\s+online)?|Publication\s+date)\s*[:：]|(?:收稿|接收|接受|录用|在线发表|发表|出版)(?:日期)?\s*[:：]"###)}
fn date_placeholder()->&'static Regex{re!(r###"(?i)(?<![A-Za-z])(?:x{1,4}(?:[\s./-]+x{1,4}){1,2}|(?:dd|yyyy)[\s./-]+(?:mm|month)[\s./-]+(?:yyyy|dd))(?![A-Za-z])"###)}
fn front_label()->&'static Regex{re!(r###"(?i)(?:研究亮点|研究要点|主要亮点|亮点|要点|图形摘要|图文摘要|图像摘要|Graphical\s+Abstract|(?:Research\s+)?Highlights)(?:\s*(?:与|和|及|、|/|&|\+|and)\s*(?:研究亮点|研究要点|主要亮点|亮点|要点|图形摘要|图文摘要|图像摘要|Graphical\s+Abstract|(?:Research\s+)?Highlights))*"###)}

fn sheets(data:&Value)->Vec<&Value>{
    if !data.is_object(){return vec![];}
    if data["version"]==3{data["sheets"].as_array().map(|s|s.iter().collect()).unwrap_or_default()}else{vec![data]}
}
fn table_scan(data:&Value,line:usize,out:&mut Vec<Prompt>)->Result<()>{
    for sheet in sheets(data){
        if !sheet.is_object(){continue;}
        for field in ["title","footer"]{if let Some(s)=sheet[field].as_str(){scan(s,line,out)?;}}
        if let Some(columns)=sheet["columns"].as_array(){for c in columns{if let Some(s)=c["name"].as_str(){scan(s,line,out)?;}}}
        if let Some(rows)=sheet["rows"].as_array(){for row in rows{if let Some(cells)=row["cells"].as_object(){for v in cells.values(){if let Some(s)=v.as_str(){scan(s,line,out)?;}}}}}
    }
    for (i,sheet) in sheets(data).into_iter().enumerate(){
        let Some(title)=sheet["title"].as_str()else{continue;};
        let location=format!("第{}张工作表",i+1);
        let title=visible(title)?.trim().to_owned();if title.is_empty(){continue;}
        let plain=replace(re!(r"(?<!\\)(?:\*\*|__)"),&title,"")?.trim_start().to_owned();
        let mut label=None;
        for r in [re!(r"^(?P<kind>表)(?P<number>[^：:\r\n]+)(?=[:：])"),re!(r"^(?P<kind>图)(?P<number>[^\r\n]+?)(?=内表格)"),re!(r"^(?P<kind>表|图)(?P<number>[A-Za-z0-9](?:\\.|[A-Za-z0-9.()_ -])*)")]{
            if let Some(c)=r.captures(&plain)?{label=Some(c);break;}
        }
        let strong=re!(r"^(?P<marker>\*\*|__)(?P<label>.+?)\k<marker>").captures(&title)?;
        let Some(label)=label else{
            if strong.is_none(){add(out,"BODY-C18",line,"TABLE_TITLE_BOLD",&title,format!("{location}的 title 未见完整粗体题注标签；先对照来源确认标签范围。"));}
            add(out,"BODY-C106",line,"TABLE_TITLE_LABEL",&title,format!("{location}的 title 未识别表号或图内表格标记；先对照来源确定对象类型与原编号，不自动补造或重排编号。"));continue;
        };
        let kind=group(&label,"kind");let number=group(&label,"number").trim_end();
        let is_embedded=kind=="图"&&plain[label.get(0).unwrap().end()..].starts_with("内表格");
        let required=format!("{kind}{number}{}",if is_embedded{"内表格"}else{""});
        if strong.as_ref().is_none_or(|s|!group(s,"label").starts_with(&required)){
            add(out,"BODY-C18",line,"TABLE_TITLE_BOLD",&title,format!("{location}的 title 题注标签按模板使用闭合的粗体。"));
        }
        if kind=="表"&&!title.starts_with(&format!("**表{number}：**")){
            add(out,"BODY-C106",line,"TABLE_TITLE_LABEL",&title,format!("{location}的独立表 title 使用 **表N：** 标签；N 保留实际表号。"));
        }
        if kind=="图"&&!is_embedded{
            add(out,"BODY-C263",line,"EMBEDDED_TABLE_TITLE",&title,format!("{location}的图内数据表可编辑副本用“图N内表格”标明来源；N 取原图编号。"));
        }
    }Ok(())
}
#[derive(Clone)]
struct Event{kind:&'static str,line:usize,text:String}
fn event(events:&mut Vec<Event>,kind:&'static str,line:usize,text:&str){events.push(Event{kind,line,text:text.to_owned()});}
fn heading_spacing(events:&[Event],out:&mut Vec<Prompt>){
    for(i,e)in events.iter().enumerate(){
        if e.kind!="heading"{continue;}
        if i>0&&events[i-1].kind!="blank"{add(out,"BODY-C67",e.line,"HEADING_GAP_MISSING",&e.text,"标题前应留一空行；父标题直接接下级标题时也按该间距处理。");}
        if i>1&&events[i-1].kind=="blank"&&events[i-2].kind=="blank"{add(out,"BODY-C67",e.line,"HEADING_GAP_EXTRA",&e.text,"核对标题前的多余空行，保留一空行。");}
        let mut following=i+1;while following<events.len()&&events[following].kind=="blank"{following+=1;}
        if following>i+1&&following<events.len()&&events[following].kind=="text"{add(out,"BODY-C68",e.line,"HEADING_BODY_GAP",&e.text,"标题后的普通正文块紧接下一行；下级标题和显示公式按各自间距处理。");}
    }
}
fn paragraph_spacing(events:&[Event],out:&mut Vec<Prompt>){
    let mut start=0;
    while start<events.len(){
        if events[start].kind!="blank"{start+=1;continue;}
        let mut end=start+1;while end<events.len()&&events[end].kind=="blank"{end+=1;}
        if end-start>1&&start>0&&end<events.len()&&events[start-1].kind=="text"&&events[end].kind=="text"{
            add(out,"BODY-C66",events[start].line,"PARAGRAPH_GAP_EXTRA",excerpt_chars(&events[end].text,80),"相邻可见正文段落之间存在连续空行；核对块边界后仅保留一空行。");
        }start=end;
    }
}
fn letter_lists(events:&[Event],out:&mut Vec<Prompt>)->Result<()>{
    let mut group:Vec<&Event>=vec![];
    let finish=|group:&mut Vec<&Event>,out:&mut Vec<Prompt>|{
        if group.len()>=2{add(out,"BODY-C104",group[0].line,"LETTER_LIST_NOT_MARKDOWN",&group[0].text,"这些字母标签仍是普通段落；对照来源确认列表后使用 Markdown 列表标记，并保留原字母标签。硬换行不产生列表结构。");}
        group.clear();
    };
    for e in events{
        if e.kind=="blank"{continue;}
        if e.kind!="text"||!start(letter_list_item(),&visible(&e.text)?)?{finish(&mut group,out);continue;}
        group.push(e);
    }finish(&mut group,out);Ok(())
}
fn front_material(events:&[Event],out:&mut Vec<Prompt>)->Result<()>{
    let mut abstract_seen=false;let mut body_seen=false;let mut supplement_level:Option<usize>=None;
    for e in events{
        if !matches!(e.kind,"heading"|"text"){continue;}
        let heading=if e.kind=="heading"{validate::heading(&e.text)}else{None};
        let mut label=heading.map(|(_,s)|s.trim()).unwrap_or(e.text.trim()).to_owned();
        if let Some((level,_))=heading{
            label=replace(re!(r"^[0-9]+(?:\.[0-9]+)*\s+"),&label,"")?;
            if supplement_level.is_some_and(|s|level<=s){supplement_level=None;}
            if level<=2&&found(re!(r"(?i)^(?:补充材料|补充信息|Supplement(?:ary|al)?\s+(?:Materials?|Information))(?:$|[\s：:（(])"),&label)?{supplement_level=Some(level);}
            if supplement_level.is_some(){continue;}
            if level==2&&matches!(label.to_lowercase().as_str(),"摘要"|"abstract"){abstract_seen=true;continue;}
        }else{
            let Some(c)=re!(r"^(?P<mark>\*\*|__)(?P<label>.+?)\k<mark>[：:]?$").captures(&label)?else{continue;};
            label=group(&c,"label").trim().to_owned();
        }
        if let Some(c)=re!(r"^(?:\*\*|__)(.*?)(?:\*\*|__)$").captures(&label)?{label=c.get(1).unwrap().as_str().to_owned();}
        label=label.trim().trim_end_matches(['：',':']).trim().to_owned();
        if supplement_level.is_some()||!abstract_seen{continue;}
        if !whole(front_label(),&label)?{
            if heading.is_some_and(|(level,_)|level<=2){body_seen=true;}continue;
        }
        let rule=if body_seen{"BODY-C16"}else{"BODY-C10"};
        let position=if body_seen{"正文开始之后"}else{"主文摘要与正文之间"};
        add(out,rule,e.line,"FRONT_MATERIAL_AFTER_ABSTRACT",&e.text,format!("显式前置信息标签“{label}”位于{position}；对照来源核对其位置。来源的前置材料放在摘要之前，摘要之后直接接正文；不新增来源中没有的对象。"));
    }Ok(())
}
fn figure_groups(events:&[Event],out:&mut Vec<Prompt>){
    let mut index=0;
    while index<events.len(){
        let e=&events[index];
        if e.kind!="text"||!syntax::standalone_image(&e.text){index+=1;continue;}
        let mut end=index;
        while end+1<events.len(){let f=&events[end+1];if f.kind!="text"||f.line!=events[end].line+1||!syntax::standalone_image(&f.text){break;}end+=1;}
        let mut cursor=end+1;while cursor<events.len()&&events[cursor].kind=="blank"{cursor+=1;}
        let caption=events.get(cursor).filter(|c|c.kind=="text"&&crate::figures::bold_label(&c.text));
        let Some(caption)=caption else{
            add(out,"BODY-C93",events[end].line,"FIGURE_CAPTION_MISSING",&events[end].text,"独占行图像组后没有题注；对照来源确认图件身份。文献图（含无编号图）紧接来源支持的粗体题注或标签，不编造说明。");
            add(out,"LINK-C126",events[end].line,"FIGURE_ANCHOR_MISSING",&events[end].text,"图像组未形成图片、题注、独占行图锚的完整块；文献图的图锚不以是否有正文调用为前提。");index=end+1;continue;
        };
        if caption.line!=events[end].line+1{add(out,"BODY-C93",caption.line,"FIGURE_CAPTION_GAP",&caption.text,"所属题注紧接最后一行图片；图块内不留空行。");}
        cursor+=1;while cursor<events.len()&&events[cursor].kind=="blank"{cursor+=1;}
        match events.get(cursor).filter(|a|a.kind=="anchor"){
            None=>add(out,"LINK-C126",caption.line,"FIGURE_ANCHOR_MISSING",&caption.text,"所属题注后缺独占一行的图锚；未编号图同样需要完整图块。"),
            Some(a) if a.line!=caption.line+1=>add(out,"LINK-C71",a.line,"FIGURE_ANCHOR_GAP",&a.text,"图锚紧接题注下一行，图块内不留空行。"),
            _=>{}
        }index=end+1;
    }
}
pub fn lint(text:&str)->Result<Vec<Prompt>>{
    let lines=split_lines(text);let mut out=Vec::new();
    let start=if lines.first().is_some_and(|s|s.trim_start_matches('\u{feff}')=="---"){
        let Some(end)=lines.iter().skip(1).position(|s|*s=="---")else{return Ok(out);};end+2
    }else{0};
    let mut fence:Option<(u8,usize,String,usize)>=None;let mut payload=Vec::<String>::new();
    let mut math:Option<(usize,Vec<String>)>=None;let mut comment=None;let mut events=Vec::new();let mut code_lines:HashMap<usize,String>=HashMap::new();
    let mut list_indent=None;let mut last_visible:Option<(&str,String)>=None;
    let logical=syntax::lines(text);let mut reference_end=None;
    for position in start..lines.len(){
        if reference_end.is_some_and(|end|position<=end){continue;}
        let raw=lines[position];
        if let Some((marker,len,language,first))=fence.as_ref(){
            if !validate::closing(raw,*marker,*len){payload.push(raw.to_owned());continue;}
            if language=="rich-table"{
                if let Ok(data)=properties::parse_yaml(validate::rich_payload(&payload).as_bytes()){table_scan(&data,*first,&mut out)?;}
            }fence=None;payload.clear();continue;
        }
        if let Some((first,parts))=math.as_mut(){
            if let Some(s)=raw.trim().strip_prefix("$$"){
                parts.push(s.to_owned());scan_math(&parts.join("\n"),*first,true,&mut out)?;math=None;
            }else{parts.push(raw.to_owned());}continue;
        }
        let indent=validate::indentation(raw);let continuation=list_indent.is_some_and(|n|indent>=n);
        if !raw.trim().is_empty()&&!continuation{list_indent=None;}
        if comment.is_none()&&indent>=4&&!(continuation&&list_indent.is_some_and(|n|indent<n+4)){
            event(&mut events,"code",position+1,raw);last_visible=Some(("code",raw.trim().to_owned()));continue;
        }
        if comment.is_none(){
            if let Some((_,end))=validate::entry(raw){list_indent=Some(end);}
            if let Some((marker,len,language))=validate::fence(raw){
                fence=Some((marker,len,language.to_owned(),position+1));event(&mut events,"code",position+1,raw);
                last_visible=Some(("code",raw.trim().to_owned()));list_indent=None;continue;
            }
        }
        if !code_lines.contains_key(&position){
            let mut end=position+1;
            if !raw.trim().is_empty()&&!found(heading(),raw)?{
                while end<lines.len()&&!lines[end].trim().is_empty()&&validate::fence(lines[end]).is_none()&&!found(heading(),lines[end])?&&validate::entry(lines[end]).is_none(){end+=1;}
            }
            let paragraph=validate::mask_paragraph_code(&lines[position..end].join("\n"),comment);
            for(i,s)in paragraph.split('\n').enumerate(){code_lines.insert(position+i,s.to_owned());}
        }
        let safe=code_lines.remove(&position).unwrap_or_else(||raw.to_owned());let active=comment;
        let (line,new_comment)=validate::mask_comments(&safe,comment);comment=new_comment;
        if line.trim().is_empty(){
            if raw.trim().is_empty()&&active.is_none(){event(&mut events,"blank",position+1,"");}
            else if !raw.trim().is_empty()&&safe.trim().is_empty(){event(&mut events,"code",position+1,raw);last_visible=Some(("code",raw.trim().to_owned()));}
            continue;
        }
        if comment.is_none()&&events.last().is_none_or(|e|e.kind!="text")&&position<logical.len(){
            if let Some(end)=syntax::reference_definition_end(text,&logical,position){reference_end=Some(end);continue;}
        }
        if let Some((marker,len,language))=validate::fence(&line){
            fence=Some((marker,len,language.to_owned(),position+1));event(&mut events,"code",position+1,&line);
            last_visible=Some(("code",line.trim().to_owned()));continue;
        }
        if let Some(tail)=line.trim().strip_prefix("$$"){
            if let Some(end)=tail.rfind("$$"){scan_math(&tail[..end],position+1,true,&mut out)?;}
            else{math=Some((position+1,vec![tail.to_owned()]));}
            event(&mut events,"math",position+1,&line);last_visible=Some(("math",line.trim().to_owned()));continue;
        }
        if found(re!(r"^\^[A-Za-z0-9-]+\s*$"),&line)?{
            if line.starts_with("^tbl-fig-")&&!found(re!(r"^\^tbl-fig-[A-Za-z0-9]+(?:-[A-Za-z0-9]+)+\s*$"),&line)?{
                add(&mut out,"LINK-C133",position+1,"EMBEDDED_TABLE_SOURCE_TOKEN",&line,"新建图内表锚应含来源 token 与图原编号 token；已核实的既有 ID 保留，仅按本次新建范围核对。");
            }
            event(&mut events,"anchor",position+1,&line);last_visible=Some(("anchor",line.trim().to_owned()));continue;
        }
        let kind=if found(heading(),&line)?{"heading"}else{"text"};
        event(&mut events,kind,position+1,&line);let v=visible(&line)?;
        if let Some(c)=figure_caption_no_space().captures(&v)?{
            let rule=if group(&c,"number").to_ascii_lowercase().starts_with('s'){"BODY-C96"}else{"BODY-C95"};
            add(&mut out,rule,position+1,"FIGURE_CAPTION_SPACE",&line,"粗体图号标签闭合后保留模板中的空格，再接题注正文。");
        }
        if syntax::standalone_image(&line)&&!raw.ends_with("  "){add(&mut out,"BODY-C92",position+1,"IMAGE_HARD_BREAK",line.trim(),"图片独占行的末尾使用两个空格硬换行；所属题注紧接下一行。");}
        if found(re!(r"^ {0,3}##\s+Zotero原条目(?:\s+#+)?\s*$"),&v)?&&last_visible.as_ref().is_none_or(|(k,s)|*k!="text"||s!="---"){
            add(&mut out,"LINK-C120",position+1,"ZOTERO_SEPARATOR_MISSING",&line,"文献内容与末尾 Zotero 原条目标题之间应有独立的 --- 分隔线。");
        }
        scan(&line,position+1,&mut out)?;
        if kind=="text"&&found(re!(r"译注\s*[:：]"),&v)?&&!found(re!(r"^ {0,3}>[ \t]+译注\s*[:：]"),&line)?{
            add(&mut out,"BODY-C40",position+1,"TRANSLATOR_NOTE_INLINE",&line,"核对显式译注标签；实际译注单独写为 > 译注：…，保留原文句子和说明内容，不静默删注。");
        }
        last_visible=Some((kind,line.trim().to_owned()));
    }
    heading_spacing(&events,&mut out);paragraph_spacing(&events,&mut out);letter_lists(&events,&mut out)?;front_material(&events,&mut out)?;figure_groups(&events,&mut out);
    let mut seen=HashSet::new();out.retain(|p|seen.insert(p.clone()));Ok(out)
}
pub fn lint_file(p:&Path)->Result<Vec<Prompt>>{
    let text=fs::read_to_string(p)?;lint(text.strip_prefix('\u{feff}').unwrap_or(&text))
}
