//! Source-bound two-pass renderer. Invalid input cannot yield partial Markdown.
use crate::{files,figures,yaml_emit};
use regex::Regex;
use serde_json::{json,Map,Value};
use std::{collections::{HashMap,HashSet,BTreeSet},sync::OnceLock,fmt};

#[derive(Debug)]
pub struct Invalid{pub field:String,pub message:String}
impl fmt::Display for Invalid{fn fmt(&self,f:&mut fmt::Formatter<'_>)->fmt::Result{write!(f,"{}: {}",self.field,self.message)}}
impl std::error::Error for Invalid{}
type R<T>=Result<T,Invalid>;
fn bad(path:&str,msg:impl Into<String>)->Invalid{Invalid{field:path.into(),message:msg.into()}}
fn check(ok:bool,path:&str,msg:&str)->R<()>{if ok{Ok(())}else{Err(bad(path,msg))}}
fn fields<'a>(v:&'a Value,p:&str,required:&str,optional:&str)->R<&'a Map<String,Value>>{
    let obj=v.as_object().ok_or_else(||bad(p,"expected an object"))?;
    let missing=required.split_whitespace().filter(|key|!obj.contains_key(*key)).min();
    if let Some(key)=missing{return Err(bad(&format!("{p}/{key}"),"required field is missing"));}
    for key in obj.keys(){
        if !required.split_whitespace().chain(optional.split_whitespace()).any(|s|s==key){
            return Err(bad(&format!("{p}/{key}"),"unexpected or protected field"));
        }
    }
    Ok(obj)
}
pub fn line<'a>(v:&'a Value,p:&str)->R<&'a str>{
    let s=v.as_str().filter(|s|!s.trim().is_empty()&&s.trim()==*s).ok_or_else(||bad(p,"expected nonempty text without outer whitespace"))?;
    check(!s.chars().any(|c|c<'\u{20}'||matches!(c,'\u{7f}'|'\u{85}'|'\u{2028}'|'\u{2029}')),p,"expected one line without controls")?;Ok(s)
}
fn ident(s:&str)->bool{
    s.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)&&s.bytes().all(|b|b.is_ascii_alphanumeric()||b"_.-".contains(&b))
}
fn ref_kind(k:&str)->bool{matches!(k,"reference"|"bibliography")}
fn spec(k:&str)->Option<(&'static str,&'static str)>{
    Some(match k{
        "header"=>("original_title authors affiliations translated","notes"),"heading"=>("level","numbered"),
        "list"=>("style count",""),"metadata"=>("notes",""),"paragraph"|"translator_note"=>("","links literals"),
        "figure"=>("images label anchor","links literals"),"formula"=>("latex anchor",""),"table"=>("data anchor",""),
        "reference"|"bibliography"=>("source_text","anchor"),"zotero_link"=>("uri",""),_=>return None,
    })
}
fn plan_units(plan:&Value)->R<&Vec<Value>>{
    fields(plan,"/plan","schema_version units","properties")?;
    check(plan["schema_version"].as_u64()==Some(1),"/plan/schema_version","expected integer 1")?;
    if let Some(v)=plan.get("properties"){check(v.is_object(),"/plan/properties","expected a YAML property mapping")?;}
    let units=plan["units"].as_array().filter(|u|!u.is_empty()).ok_or_else(||bad("/plan/units","expected a nonempty ordered list"))?;
    let mut seen=HashSet::new();let mut supplement=false;
    for (i,u) in units.iter().enumerate(){
        let p=format!("/plan/units/{i}");
        let k=u["kind"].as_str().ok_or_else(||bad(&format!("{p}/kind"),"unknown or missing unit kind"))?;
        let (required,optional)=spec(k).ok_or_else(||bad(&format!("{p}/kind"),"unknown or missing unit kind"))?;
        fields(u,&p,&format!("unit_id kind scope source {required}"),optional)?;
        let uid=line(&u["unit_id"],&format!("{p}/unit_id"))?;
        check(ident(uid)&&seen.insert(uid),&format!("{p}/unit_id"),"invalid or duplicate unit ID")?;
        fields(&u["source"],&format!("{p}/source"),"source_id locator","")?;
        line(&u["source"]["source_id"],&format!("{p}/source/source_id"))?;
        let loc=&u["source"]["locator"];
        check(loc.as_str().is_some_and(|s|!s.trim().is_empty())||loc.as_object().is_some_and(|m|!m.is_empty()),&format!("{p}/source/locator"),"expected a nonempty locator")?;
        let scope=line(&u["scope"],&format!("{p}/scope"))?;
        check(scope=="main"||scope.strip_prefix("supplement:").is_some_and(ident),&format!("{p}/scope"),"invalid source scope")?;
        if k=="header"{check(i==0&&scope=="main",&p,"header must be the first main unit")?;}
        if k=="zotero_link"{check(i+1==units.len()&&scope=="main",&p,"Zotero link must be the final main-scope footer")?;}
        check(!supplement||scope!="main"||k=="zotero_link",&format!("{p}/scope"),"main content must precede supplements")?;
        supplement|=scope!="main";
        if k=="list"{
            check(u["count"].as_u64().is_some_and(|n|n>0&&n<=1_000_000),&format!("{p}/count"),"expected a positive source item count within allocation limit")?;
            check(matches!(u["style"].as_str(),Some("ordered"|"unordered")),&format!("{p}/style"),"expected ordered or unordered")?;
        }
        if let Some(notes)=u.get("notes"){
            let notes=notes.as_array().filter(|v|k!="metadata"||!v.is_empty()).ok_or_else(||bad(&format!("{p}/notes"),"expected a list of verified metadata lines"))?;
            for(j,n)in notes.iter().enumerate(){line(n,&format!("{p}/notes/{j}"))?;}
        }
        if k=="header"{check(u["translated"].is_boolean(),&format!("{p}/translated"),"expected a boolean")?;}
    }Ok(units)
}
fn slots(u:&Value)->Value{
    match u["kind"].as_str().unwrap(){
        "header" if u["translated"]==true=>json!({"translated_title":""}),
        "heading"|"paragraph"|"translator_note"=>json!({"text":""}),
        "list"=>json!({"items":vec!["";u["count"].as_u64().unwrap() as usize]}),
        "figure"=>json!({"caption":""}),_=>json!({}),
    }
}
pub fn content_template(plan:&Value)->R<Value>{
    Ok(Value::Object(plan_units(plan)?.iter().map(|u|(u["unit_id"].as_str().unwrap().to_owned(),slots(u))).collect()))
}
pub fn literal(s:&str)->String{
    let mut out=String::with_capacity(s.len());let mut run=0;
    for c in s.chars(){
        if c=='%'{run+=1;continue;}
        if run==1{out.push('%');}else if run>1{for _ in 0..run{out.push_str("\\%");}}run=0;
        if "\\\x60*_[]<>$~&#^|=".contains(c){out.push('\\');}out.push(c);
    }
    if run==1{out.push('%');}else if run>1{for _ in 0..run{out.push_str("\\%");}}
    let digits=out.bytes().take_while(u8::is_ascii_digit).count();
    if digits>0&&out.as_bytes().get(digits).is_some_and(|b|*b==b'.'||*b==b')')&&out[digits+1..].chars().next().is_none_or(char::is_whitespace){out.insert(digits,'\\');}
    else if out.starts_with(['-','+','!'])&&(out[1..].chars().next().is_none_or(char::is_whitespace)||
        (out.starts_with('-')&&out.chars().filter(|c|!c.is_whitespace()).count()>=3&&out.chars().all(|c|c=='-'||c.is_whitespace()))){out.insert(0,'\\');}
    out
}
fn percent(s:&str,allow:impl Fn(u8)->bool)->String{
    let mut out=String::new();for b in s.bytes(){if allow(b){out.push(b as char);}else{out.push_str(&format!("%{b:02X}"));}}out
}
fn fragment(s:&str)->String{
    let mut out=String::new();for c in s.chars(){if "%<>#\\&".contains(c){out.push_str(&format!("%{:02X}",c as u32));}else{out.push(c);}}out
}
fn marker(s:&str)->bool{!s.is_empty()&&s.bytes().all(|b|b.is_ascii_alphanumeric())}
fn header_item(v:&Value,field:&str,p:&str)->R<String>{
    if v.is_string(){return Ok(literal(line(v,p)?));}
    if field=="affiliations"{
        fields(v,p,"name number","")?;
        let number=line(&v["number"],&format!("{p}/number"))?;
        check(marker(number),&format!("{p}/number"),"expected the source affiliation marker")?;
        return Ok(format!("{}{{}}^{{{number}}}{}{}",'$','$',literal(line(&v["name"],&format!("{p}/name"))?)));
    }
    fields(v,p,"name","numbers corresponding email")?;
    let mut name=literal(line(&v["name"],&format!("{p}/name"))?);let mut markers=Vec::new();
    if let Some(numbers)=v.get("numbers"){
        for n in numbers.as_array().ok_or_else(||bad(&format!("{p}/numbers"),"expected a list of source affiliation markers"))?{
            markers.push(n.as_str().filter(|s|marker(s)).ok_or_else(||bad(&format!("{p}/numbers"),"expected a list of source affiliation markers"))?);
        }
    }
    if let Some(c)=v.get("corresponding"){if c.as_bool().ok_or_else(||bad(&format!("{p}/corresponding"),"expected a boolean"))?{markers.push("\\ast");}}
    if let Some(e)=v.get("email"){
        let email=line(e,&format!("{p}/email"))?;
        let(a,b)=email.split_once('@').ok_or_else(||bad(&format!("{p}/email"),"invalid email"))?;
        check(!a.is_empty()&&!b.is_empty()&&!b.contains('@')&&!email.chars().any(|c|c.is_whitespace()||matches!(c,'<'|'>')),&format!("{p}/email"),"invalid email")?;
        name=format!("[{name}](mailto:{})",percent(email,|b|b.is_ascii_alphanumeric()||b"@.+-_~".contains(&b)));
    }
    if !markers.is_empty(){name.push_str(&format!("{}{{}}^{{{}}}{}",'$',markers.join(","),'$'));}Ok(name)
}
type TextMap=HashMap<String,String>;
fn citation(v:&Value,p:&str,targets:&TextMap,numbers:&HashMap<String,usize>)->R<String>{
    fields(v,p,"citation","")?;
    let parts=v["citation"].as_array().filter(|a|!a.is_empty()&&a.len()%2==1).ok_or_else(||bad(p,"expected reference IDs alternating with separators"))?;
    let mut out=String::from("<sup>\\[");
    for(i,v)in parts.iter().enumerate(){
        let s=v.as_str().ok_or_else(||bad(p,"expected a reference ID or separator"))?;
        if i%2==1{check(matches!(s,","|"-"),p,"expected ',' or '-' between reference IDs")?;out.push_str(s);}
        else{
            let n=numbers.get(s).ok_or_else(||bad(p,"citation must resolve to a bibliography unit"))?;
            let t=targets.get(s).ok_or_else(||bad(p,"citation target missing"))?;
            out.push_str(&format!("[{n}](<#{t}>)"));
        }
    }out.push_str("\\]</sup>");Ok(out)
}
fn inline(u:&Value,value:&str,p:&str,targets:&TextMap,numbers:&HashMap<String,usize>,ambiguous:&HashSet<String>)->R<String>{
    static TOKENS:OnceLock<Regex>=OnceLock::new();static BLOCK:OnceLock<Regex>=OnceLock::new();
    let token=TOKENS.get_or_init(||Regex::new(r"\{\{(link|literal):([A-Za-z0-9][A-Za-z0-9_.-]*)\}\}").unwrap());
    let block=BLOCK.get_or_init(||Regex::new(r"^(?:#{1,6}(?:\s|$)|>|[-+*]\s|\d+[.)]\s|[-*_]{3,}\s*$|\^[-A-Za-z0-9]+$|\x60{3,}|~{3,}|\$\$|<[/!A-Za-z])").unwrap());
    let mut rules=HashMap::new();
    for(typ,field)in [("link","links"),("literal","literals")]{
        let Some(entries)=u.get(field)else{continue;};
        for(name,spec)in entries.as_object().ok_or_else(||bad(p,format!("{typ} registrations must be an object")))?{
            check(ident(name),p,"invalid placeholder name")?;
            let rendered=if typ=="literal"{
                let raw=line(spec,p)?;check(!block.is_match(raw),p,"protected inline text contains a block delimiter")?;raw.to_owned()
            }else if spec.get("citation").is_some(){citation(spec,p,targets,numbers)?}
            else{
                fields(spec,p,"target label","")?;let target=line(&spec["target"],p)?;
                let dest=targets.get(target).ok_or_else(||bad(p,format!("unresolved link target {target}")))?;
                check(!ambiguous.contains(dest),p,&format!("link target resolves to multiple headings: {target}"))?;
                format!("[{}](<#{dest}>)",literal(line(&spec["label"],p)?))
            };rules.insert(format!("{typ}:{name}"),rendered);
        }
    }
    let mut cursor=0;let mut out=String::new();let mut used=HashSet::new();
    for caps in token.captures_iter(value){
        let m=caps.get(0).unwrap();let key=format!("{}:{}",&caps[1],&caps[2]);
        let replacement=rules.get(&key).filter(|_|used.insert(key.clone())).ok_or_else(||bad(p,format!("unregistered or repeated placeholder {}",m.as_str())))?;
        out.push_str(&literal(&value[cursor..m.start()]));out.push_str(replacement);cursor=m.end();
    }
    check(used.len()==rules.len(),p,"missing registered placeholder(s)")?;
    out.push_str(&literal(&value[cursor..]));Ok(out)
}
pub fn canonical(v:&Value)->String{files::digest(&serde_json::to_vec(v).expect("JSON value serialization"))}
fn render(plan:&Value,content:&Value)->R<(String,Vec<Value>)>{
    let units=plan_units(plan)?;
    let values=content.as_object().ok_or_else(||bad("/content","expected a mapping of unit IDs"))?;
    let expected:BTreeSet<&str>=units.iter().map(|u|u["unit_id"].as_str().unwrap()).collect();
    for id in &expected{check(values.contains_key(*id),&format!("/content/{id}"),"missing unit")?;}
    for id in values.keys(){check(expected.contains(id.as_str()),&format!("/content/{id}"),"unexpected unit")?;}
    let mut targets=TextMap::new();let mut headings=TextMap::new();let mut numbers=HashMap::new();
    let mut counts:HashMap<&str,usize>=HashMap::new();let mut levels:Vec<usize>=vec![];let mut occupied=HashSet::new();
    let multi_si=units.iter().filter_map(|u|u["scope"].as_str().filter(|s|*s!="main")).collect::<HashSet<_>>().len()>1;
    for(i,u)in units.iter().enumerate(){
        let uid=u["unit_id"].as_str().unwrap();let k=u["kind"].as_str().unwrap();let scope=u["scope"].as_str().unwrap();
        let p=format!("/plan/units/{i}");let cp=format!("/content/{uid}");
        let template=slots(u);let required=if k=="figure"{String::new()}else{template.as_object().unwrap().keys().cloned().collect::<Vec<_>>().join(" ")};
        let v=fields(&values[uid],&cp,&required,if k=="figure"{"caption"}else{""})?;
        for(key,value)in v{
            if k=="list"{
                let items=value.as_array().filter(|v|v.len() as u64==u["count"].as_u64().unwrap()).ok_or_else(||bad(&format!("{cp}/items"),"supply exactly one translation per source list item"))?;
                for(j,item)in items.iter().enumerate(){line(item,&format!("{cp}/items/{j}"))?;}
            }else if k!="figure"||value!=""{line(value,&format!("{cp}/{key}"))?;}
        }
        if k=="heading"{
            let level=u["level"].as_u64().filter(|n|(2..=6).contains(n)).ok_or_else(||bad(&p,"heading requires level 2–6 and boolean numbered"))? as usize;
            let numbered=u.get("numbered").map(|v|v.as_bool().ok_or_else(||bad(&p,"heading requires level 2–6 and boolean numbered"))).transpose()?.unwrap_or(true);
            let mut name=v["text"].as_str().unwrap().to_owned();
            if numbered{
                let depth=level-1;check(depth<=levels.len()+1,&format!("{p}/level"),"numbered heading skips a parent")?;
                levels.resize(depth,0);levels[depth-1]+=1;
                name=format!("{} {name}",levels.iter().map(usize::to_string).collect::<Vec<_>>().join("."));
            }
            headings.insert(uid.to_owned(),format!("{} {}","#".repeat(level),literal(&name)));targets.insert(uid.to_owned(),fragment(&name));continue;
        }
        let mut anchor=u.get("anchor").cloned();
        if ref_kind(k){
            let count=counts.entry(scope).or_default();*count+=1;numbers.insert(uid.to_owned(),*count);
            let prefix=if scope=="main"{"ref".to_owned()}else if multi_si{format!("sref-src{}",&scope["supplement:".len()..])}else{"sref".into()};
            if anchor.is_none(){anchor=Some(json!(format!("{prefix}-{count}")));}
        }
        if let Some(anchor)=anchor{
            let a=line(&anchor,&format!("{p}/anchor"))?;let a=a.strip_prefix('^').unwrap_or(a);
            check(!a.is_empty()&&a.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-')&&occupied.insert(a.to_owned()),&format!("{p}/anchor"),"invalid or duplicate block ID")?;
            let prefix=a.split('-').next().unwrap();
            let ns=match prefix{"fig"=>Some("figure"),"eq"=>Some("formula"),"tbl"=>Some("table"),"ref"|"sref"=>Some("reference"),_=>None};
            check(ns.is_none_or(|s|s==if ref_kind(k){"reference"}else{k}),&format!("{p}/anchor"),"anchor belongs to a different object kind")?;
            targets.insert(uid.to_owned(),format!("^{a}"));
        }
    }
    let mut heading_counts=HashMap::new();
    for uid in headings.keys(){*heading_counts.entry(targets[uid].clone()).or_insert(0usize)+=1;}
    let ambiguous=heading_counts.into_iter().filter(|(_,n)|*n>1).map(|(s,_)|s).collect();
    let mut out=String::new();let mut mapping=Vec::with_capacity(units.len());let mut offset=0;let mut current_line=1;
    if let Some(props)=plan.get("properties"){
        let yaml=yaml_emit::dump(props).map_err(|e|bad("/plan/properties",e.to_string()))?;
        out.push_str(&format!("---\n{yaml}---\n\n"));offset=out.chars().count();current_line+=out.bytes().filter(|b|*b==b'\n').count();
    }
    let mut previous="";
    for(i,u)in units.iter().enumerate(){
        let uid=u["unit_id"].as_str().unwrap();let k=u["kind"].as_str().unwrap();let p=format!("/plan/units/{i}");let cp=format!("/content/{uid}");let v=&values[uid];
        let block=match k{
            "header"=>{
                let mut title=format!("# {}",literal(line(&u["original_title"],&format!("{p}/original_title"))?));
                if u["translated"]==true{title.push('\n');title.push_str(&literal(v["translated_title"].as_str().unwrap()));}
                let mut blocks=vec![title];
                for(field,sep)in [("authors","; "),("affiliations","  \n")]{
                    let items=u[field].as_array().ok_or_else(||bad(&format!("{p}/{field}"),"expected a list of source lines"))?;
                    if !items.is_empty(){blocks.push(items.iter().enumerate().map(|(j,item)|header_item(item,field,&format!("{p}/{field}/{j}"))).collect::<R<Vec<_>>>()?.join(sep));}
                }
                if let Some(notes)=u["notes"].as_array().filter(|a|!a.is_empty()){blocks.push(notes.iter().map(|n|format!("> {}",literal(n.as_str().unwrap()))).collect::<Vec<_>>().join("\n"));}
                blocks.join("\n\n")
            }
            "metadata"=>u["notes"].as_array().unwrap().iter().map(|n|format!("> {}",literal(n.as_str().unwrap()))).collect::<Vec<_>>().join("\n"),
            "list"=>v["items"].as_array().unwrap().iter().enumerate().map(|(j,item)|format!("{}{}",if u["style"]=="ordered"{format!("{}. ",j+1)}else{"- ".into()},literal(item.as_str().unwrap()))).collect::<Vec<_>>().join("\n"),
            "heading"=>headings[uid].clone(),
            "paragraph"|"translator_note"=>{
                let text=inline(u,v["text"].as_str().unwrap(),&format!("{cp}/text"),&targets,&numbers,&ambiguous)?;
                if k=="translator_note"{format!("> 译注：{text}")}else{text}
            }
            "formula"=>{
                let latex=u["latex"].as_str().filter(|s|!s.trim().is_empty()&&!s.contains("$$")&&!s.chars().any(|c|(c<'\u{20}'&&!matches!(c,'\n'|'\t'))||matches!(c,'\u{7f}'|'\u{85}'|'\u{2028}'|'\u{2029}'))).ok_or_else(||bad(&format!("{p}/latex"),"expected raw LF LaTeX without display delimiters"))?;
                format!("$$\n{latex}\n$$\n{}",targets[uid])
            }
            "table"=>yaml_emit::rich_table(&u["data"],&targets[uid],true,"\n").map_err(|e|bad(&p,e.to_string()))?.trim_end_matches('\n').to_owned(),
            "figure"=>{
                let images=u["images"].as_array().filter(|v|!v.is_empty()).ok_or_else(||bad(&format!("{p}/images"),"expected source image list"))?;
                for(j,image)in images.iter().enumerate(){
                    let ip=format!("{p}/images/{j}");fields(image,&ip,"target alt","")?;line(&image["target"],&format!("{ip}/target"))?;
                    if image["alt"]!=""{line(&image["alt"],&format!("{ip}/alt"))?;}
                }
                let cap=inline(u,v["caption"].as_str().unwrap_or(""),&format!("{cp}/caption"),&targets,&numbers,&ambiguous)?;
                let label=literal(line(&u["label"],&format!("{p}/label"))?);
                let caption=if cap.is_empty(){format!("**{label}**")}else{format!("**{label}** {cap}")};
                figures::build(&u["images"],&caption,&targets[uid],"\n").map_err(|e|bad(&p,e.to_string()))?.trim_end_matches('\n').to_owned()
            }
            "reference"|"bibliography"=>format!("{}. {} {}",numbers[uid],literal(line(&u["source_text"],&format!("{p}/source_text"))?),targets[uid]),
            "zotero_link"=>{
                fn uri_regex()->&'static Regex{static URI:OnceLock<Regex>=OnceLock::new();URI.get_or_init(||Regex::new(r"^zotero://select/(?:library|groups/[1-9][0-9]*)/items/[A-Za-z0-9]+$").unwrap())}
                let uri=line(&u["uri"],&format!("{p}/uri"))?;
                check(uri_regex().is_match(uri),&format!("{p}/uri"),"expected a Zotero item selection URI")?;
                format!("---\n\n## [Zotero原条目]({uri})")
            }
            _=>unreachable!(),
        };
        let sep=if previous.is_empty(){""}else if (previous=="heading"&&(matches!(k,"paragraph"|"translator_note"|"metadata"|"list")||ref_kind(k)))||(ref_kind(previous)&&ref_kind(k))||(previous=="metadata"&&k=="metadata"){"\n"}else{"\n\n"};
        offset+=sep.len();current_line+=sep.len();out.push_str(sep);
        let lines=block.bytes().filter(|b|*b==b'\n').count();let chars=block.chars().count();
        mapping.push(json!({"unit_id":uid,"kind":k,"scope":u["scope"],"source":u["source"],"start_line":current_line,"end_line":current_line+lines,
            "start_offset":offset,"end_offset":offset+chars,"sha256":files::digest(block.as_bytes())}));
        offset+=chars;current_line+=lines;previous=k;out.push_str(&block);
    }
    out.push('\n');Ok((out,mapping))
}
pub fn render_note(plan:&Value,content:&Value)->Value{
    let ph=canonical(plan);let ch=canonical(content);
    match render(plan,content){
        Ok((text,mapping))=>json!({"schema_version":1,"status":"rendered","output_sha256":files::digest(text.as_bytes()),"text":text,
            "mapping":mapping,"findings":[],"plan_sha256":ph,"content_sha256":ch,"source_fidelity":"not_assessed"}),
        Err(e)=>json!({"schema_version":1,"status":"blocked","text":null,"mapping":[],"findings":[{"code":"RENDER_INVALID","field":e.field,"message":e.message}],
            "plan_sha256":ph,"content_sha256":ch,"output_sha256":null,"source_fidelity":"not_assessed"}),
    }
}


/// The public helpers have the same checked behavior as the renderer's
/// internal primitives, with safe allocation bounds for standalone calls.
pub fn keys(value:&Value,path:&str,required:&str,optional:&str)->R<()>{
    fields(value,path,required,optional)?;Ok(())
}
pub fn digest(value:&str)->String{files::digest(value.as_bytes())}
pub fn template_fields(unit:&Value)->R<Value>{
    let kind=unit["kind"].as_str().ok_or_else(||bad("/unit/kind","expected a unit kind"))?;
    check(spec(kind).is_some(),"/unit/kind","unknown unit kind")?;
    if kind=="header"{check(unit["translated"].is_boolean(),"/unit/translated","expected a boolean")?;}
    if kind=="list"{check(unit["count"].as_u64().is_some_and(|n|n>0&&n<=1_000_000),"/unit/count","positive bounded item count required")?;}
    Ok(slots(unit))
}
