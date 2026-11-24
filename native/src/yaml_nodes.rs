//! YAML source locations and collection styles from a real parser, not regex
//! approximations of YAML syntax. Resolved values are independently decoded
//! by the budgeted serde-saphyr deserializer.
use granit_parser::{Event,Parser,ScalarStyle,StructureStyle};
use serde_json::Value;
use regex::Regex;
use std::{collections::{HashMap,HashSet},sync::OnceLock};
use crate::properties;

#[derive(Debug)]
pub struct Failure{pub line:usize,pub message:String}
fn failure(line:usize,message:impl Into<String>)->Failure{Failure{line:line.max(1),message:message.into()}}
#[derive(Debug)]
enum Kind{Scalar(String,bool),Map(Vec<usize>),Seq(Vec<usize>,bool),Alias(usize)}
#[derive(Debug)]
struct Node{line:usize,kind:Kind}
#[derive(Debug)]
pub struct Document{pub value:Value,nodes:Vec<Node>,anchors:HashMap<usize,usize>,pub root:Option<usize>}
fn date_valid(text:&str)->bool{
    static DATE:OnceLock<Regex>=OnceLock::new();
    let re=DATE.get_or_init(||Regex::new(r"^(\d{4})-(\d{2})-(\d{2})(?:$|[Tt \t]\d{1,2}:\d{2}:\d{2})").unwrap());
    let Some(c)=re.captures(text)else{return true;};
    let (Ok(y),Ok(m),Ok(d))=(c[1].parse::<u32>(),c[2].parse::<u32>(),c[3].parse::<u32>())else{return false;};
    let leap=y%4==0&&(y%100!=0||y%400==0);
    let days=match m{1|3|5|7|8|10|12=>31,4|6|9|11=>30,2=>if leap{29}else{28},_=>0};
    y>0&&d>0&&d<=days
}
impl Document{
    fn resolve(&self,mut id:usize)->Option<usize>{
        let mut seen=HashSet::new();
        while let Some(Node{kind:Kind::Alias(anchor),..})=self.nodes.get(id){
            if !seen.insert(id){return None;}
            id=*self.anchors.get(anchor)?;
        }Some(id)
    }
    pub fn line(&self,id:Option<usize>)->Option<usize>{self.nodes.get(self.resolve(id?)?).map(|n|n.line)}
    pub fn seq(&self,id:Option<usize>)->Option<&[usize]>{
        match &self.nodes.get(self.resolve(id?)?)?.kind{Kind::Seq(items,_)=>Some(items),_=>None}
    }
    pub fn flow_sequence(&self,id:Option<usize>)->bool{
        id.and_then(|id|self.resolve(id)).and_then(|id|self.nodes.get(id)).is_some_and(|n|matches!(&n.kind,Kind::Seq(v,true) if !v.is_empty()))
    }
    fn field_inner(&self,id:usize,name:&str,seen:&mut HashSet<usize>)->Option<usize>{
        let id=self.resolve(id)?;
        if !seen.insert(id){return None;}
        let Kind::Map(items)=&self.nodes[id].kind else{return None;};
        let mut merges=Vec::new();
        for pair in items.chunks_exact(2){
            let key=self.resolve(pair[0])?;
            let Kind::Scalar(value,merge)=&self.nodes[key].kind else{continue;};
            if *merge{merges.push(pair[1]);}
            else if value==name{return self.resolve(pair[1]);}
        }
        for merge in merges{
            let merge=self.resolve(merge)?;
            if let Some(children)=self.seq(Some(merge)){
                for child in children{if let Some(found)=self.field_inner(*child,name,seen){return Some(found);}}
            }else if let Some(found)=self.field_inner(merge,name,seen){return Some(found);}
        }None
    }
    pub fn field(&self,id:Option<usize>,name:&str)->Option<usize>{self.field_inner(id?,name,&mut HashSet::new())}
}
fn append(doc:&mut Document,stack:&[usize],node:Node,anchor:usize)->Result<usize,Failure>{
    let id=doc.nodes.len();let line=node.line;
    if id>=500_000||stack.len()>=128{return Err(failure(line,"YAML source-node budget exceeded"));}
    doc.nodes.push(node);
    if anchor>0{doc.anchors.insert(anchor,id);}
    if let Some(parent)=stack.last(){
        match &mut doc.nodes[*parent].kind{
            Kind::Map(items)|Kind::Seq(items,_)=>items.push(id),_=>return Err(failure(line,"invalid YAML event hierarchy"))
        }
    }else if doc.root.replace(id).is_some(){return Err(failure(line,"multiple YAML document roots"));}
    Ok(id)
}
pub fn load(payload:&str)->Result<Document,Failure>{
    let mut doc=Document{value:Value::Null,nodes:vec![],anchors:HashMap::new(),root:None};
    let mut stack=Vec::new();let mut documents=0;
    for next in Parser::new_from_str(payload){
        let(event,span)=next.map_err(|e|failure(1,format!("{e}")))?;
        let line=span.start.line();
        match event{
            Event::DocumentStart(..)=>{documents+=1;if documents>1{return Err(failure(line,"multiple YAML documents"));}}
            Event::MappingStart(_,anchor,tag)=>{
                if tag.as_ref().is_some_and(|t|t.is_custom()){return Err(failure(line,"unsupported custom YAML tag"));}
                let id=append(&mut doc,&stack,Node{line,kind:Kind::Map(vec![])},anchor)?;stack.push(id);
            }
            Event::SequenceStart(style,anchor,tag)=>{
                if tag.as_ref().is_some_and(|t|t.is_custom()){return Err(failure(line,"unsupported custom YAML tag"));}
                let id=append(&mut doc,&stack,Node{line,kind:Kind::Seq(vec![],style==StructureStyle::Flow)},anchor)?;stack.push(id);
            }
            Event::MappingEnd|Event::SequenceEnd=>{stack.pop();}
            Event::Scalar(value,style,anchor,tag)=>{
                if tag.as_ref().is_some_and(|t|t.is_custom()){return Err(failure(line,"unsupported custom YAML tag"));}
                let implicit=style==ScalarStyle::Plain&&tag.is_none();
                let timestamp=tag.as_ref().is_some_and(|t|t.is_yaml_core_schema_tag("timestamp"));
                if (implicit||timestamp)&&!date_valid(&value){return Err(failure(line,"invalid calendar date"));}
                let merge=value=="<<"&&(implicit||tag.as_ref().is_some_and(|t|t.is_yaml_core_schema_tag("merge")));
                append(&mut doc,&stack,Node{line,kind:Kind::Scalar(value.into_owned(),merge)},anchor)?;
            }
            Event::Alias(anchor)=>{append(&mut doc,&stack,Node{line,kind:Kind::Alias(anchor)},0)?;}
            _=>{}
        }
    }
    // The decoded parser independently checks duplicate keys, number types,
    // aliases and merge expansion. Here literal duplicates also get the actual
    // second-key source mark, not a guessed document line.
    for node in &doc.nodes{
        let Kind::Map(items)=&node.kind else{continue;};
        if items.len()%2!=0{return Err(failure(node.line,"incomplete YAML mapping"));}
        let mut seen=HashSet::new();
        for pair in items.chunks_exact(2){
            let Some(key)=doc.resolve(pair[0])else{return Err(failure(doc.nodes[pair[0]].line,"unresolved YAML key alias"));};
            if let Kind::Scalar(value,false)=&doc.nodes[key].kind{
                if !seen.insert(value){return Err(failure(doc.nodes[pair[0]].line,format!("duplicate key: {value}")));}
            }
        }
    }
    doc.value=properties::parse_yaml(payload.as_bytes()).map_err(|e|failure(1,e.to_string()))?;
    Ok(doc)
}
