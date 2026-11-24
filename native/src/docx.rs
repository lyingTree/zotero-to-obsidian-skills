//! Native OOXML source provenance. No invented page geometry, figure numbers,
//! OCR, external content, or semantic review decisions.
use crate::{files,path_text};
use anyhow::{anyhow,ensure,Context,Result};
use regex::Regex;
use roxmltree::{Document,Node};
use serde_json::{json,Map,Value};
use sha2::{Digest,Sha256};
use std::{collections::{HashMap,HashSet},fs::{self,File},io::{Read,Write},path::{Path,PathBuf},sync::OnceLock};
use zip::ZipArchive;

pub const DOCUMENT:&str="word/document.xml";
const TYPES:&str="[Content_Types].xml";
const RELS:&str="word/_rels/document.xml.rels";
const W:&str="http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R:&str="http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const A:&str="http://schemas.openxmlformats.org/drawingml/2006/main";
const V:&str="urn:schemas-microsoft-com:vml";
const A14:&str="http://schemas.microsoft.com/office/drawing/2010/main";
const MAX_XML:u64=256*1024*1024;
const MAX_MEDIA:u64=2*1024*1024*1024;
const MAX_TOTAL_MEDIA:u64=16*1024*1024*1024;
const MAX_NODES:u32=2_000_000;
const MAX_DEPTH:usize=256;
macro_rules! rx{($p:expr)=>{{static RE:OnceLock<Regex>=OnceLock::new();RE.get_or_init(||Regex::new($p).unwrap())}}}

fn open(p:&Path)->Result<ZipArchive<File>>{
    ZipArchive::new(File::open(p)?).context("An accessible DOCX package is required")
}
fn member(archive:&mut ZipArchive<File>,name:&str,max:u64)->Result<Vec<u8>>{
    let f=archive.by_name(name).with_context(||format!("Missing OOXML member: {name}"))?;
    ensure!(f.size()<=max,"OOXML member exceeds safe read budget: {name}");
    let mut b=Vec::with_capacity(usize::try_from(f.size())?);
    f.take(max+1).read_to_end(&mut b)?;
    ensure!(b.len() as u64<=max,"OOXML member exceeded safe read budget: {name}");Ok(b)
}
fn xml_text(data:&[u8],name:&str)->Result<String>{
    // UTF-8 and both standard UTF-16 XML byte orders. XML declarations and
    // namespace processing are validated by the real XML parser.
    let (utf16,start)=if data.starts_with(&[0xff,0xfe]){(Some(true),2)}
        else if data.starts_with(&[0xfe,0xff]){(Some(false),2)}
        else if data.starts_with(&[b'<',0,b'?',0]){(Some(true),0)}
        else if data.starts_with(&[0,b'<',0,b'?']){(Some(false),0)}
        else{(None,0)};
    let text=if let Some(little)=utf16{
        ensure!((data.len()-start)%2==0,"{name}: invalid UTF-16 length");
        let units=data[start..].chunks_exact(2).map(|b|if little{u16::from_le_bytes([b[0],b[1]])}else{u16::from_be_bytes([b[0],b[1]])}).collect::<Vec<_>>();
        String::from_utf16(&units).with_context(||format!("{name}: invalid UTF-16 XML"))?
    }else{std::str::from_utf8(data).with_context(||format!("{name}: unsupported or invalid XML encoding"))?.trim_start_matches('\u{feff}').to_owned()};
    ensure!(!rx!(r"(?i)<!\s*(?:DOCTYPE|ENTITY)\b").is_match(&text),"{name}: DTD/entity declarations are not supported");
    Ok(text)
}
fn parse<'a>(text:&'a str,name:&str)->Result<Document<'a>>{
    let opt=roxmltree::ParsingOptions{allow_dtd:false,nodes_limit:MAX_NODES};
    let doc=Document::parse_with_options(text,opt).with_context(||format!("{name}: invalid OOXML"))?;
    let mut stack=vec![(doc.root_element(),0usize)];
    while let Some((node,depth))=stack.pop(){
        ensure!(depth<=MAX_DEPTH,"{name}: XML nesting exceeds safe limit");
        stack.extend(node.children().filter(|n|n.is_element()).map(|n|(n,depth+1)));
    }Ok(doc)
}
fn tag(n:Node<'_,'_>,ns:&str,name:&str)->bool{n.is_element()&&n.tag_name().namespace()==Some(ns)&&n.tag_name().name()==name}
fn local<'a,'i>(n:Node<'a,'i>)->&'i str{n.tag_name().name()}
fn child<'a,'i>(n:Node<'a,'i>,name:&str)->Option<Node<'a,'i>>{n.children().find(|n|tag(*n,W,name))}
fn kids<'a,'i>(n:Node<'a,'i>)->impl Iterator<Item=Node<'a,'i>>{n.children().filter(|n|n.is_element())}
fn attr<'a,'i>(n:Node<'a,'i>,ns:&str,name:&str)->Option<&'a str>{n.attribute((ns,name))}
fn locator(indices:&[usize])->Value{json!({"part":DOCUMENT,"element_path":indices})}
fn at(indices:&[usize],i:usize)->Vec<usize>{let mut v=indices.to_vec();v.push(i);v}
pub fn valid_locator(v:&Value)->bool{
    v.as_object().is_some_and(|o|o.len()==2)&&v["part"]==DOCUMENT&&v["element_path"].as_array().is_some_and(|a|a.iter().all(|x|x.as_u64().is_some()))
}
fn target_part(target:&str)->Option<String>{
    if target.contains(['\\','?','#'])||target.starts_with("//")||rx!(r"^[A-Za-z][A-Za-z0-9+.-]*:").is_match(target){return None;}
    let raw=percent_encoding::percent_decode_str(target).decode_utf8().ok()?;
    if raw.contains(['\0','\\']){return None;}
    let mut parts=if raw.starts_with('/') {vec![]}else{vec!["word"]};
    for p in raw.split('/'){match p{""|"."=>{},".."=>{parts.pop()?;},_=>parts.push(p)}}
    Some(parts.join("/"))
}
fn unsupported(n:&str)->Option<&'static str>{Some(match n{
    "oMath"|"oMathPara"=>"math","anchor"=>"floating_object","txbxContent"=>"text_box","object"=>"embedded_object","chart"=>"chart",
    "altChunk"|"AlternateContent"=>"alternate_content","footnoteReference"=>"footnote","endnoteReference"=>"endnote",
    "del"|"ins"|"moveFrom"|"moveTo"=>"tracked_change","fldChar"|"instrText"|"fldSimple"=>"field","sym"=>"symbol",
    "numPr"=>"generated_list_number","pict"=>"legacy_drawing",_=>return None,
})}
fn ignored(n:Node<'_,'_>,container:&str)->bool{
    if n.tag_name().namespace()!=Some(W){return false;}
    let l=local(n);
    matches!(l,"bookmarkStart"|"bookmarkEnd"|"proofErr"|"permStart"|"permEnd")||
        match container{"table"=>matches!(l,"tblPr"|"tblGrid"),"row"=>matches!(l,"trPr"|"tblPrEx"),"cell"=>l=="tcPr",_=>false}
}
fn xml_escape(s:&str,attribute:bool)->String{
    let mut out=String::new();
    for ch in s.chars(){match ch{
        '&'=>out.push_str("&amp;"),'<'=>out.push_str("&lt;"),'>'=>out.push_str("&gt;"),
        '"' if attribute=>out.push_str("&quot;"),'\r' if attribute=>out.push_str("&#13;"),
        '\n' if attribute=>out.push_str("&#10;"),'\t' if attribute=>out.push_str("&#09;"),_=>out.push(ch),
    }}out
}
// Serialize a pending subtree with its own namespace declarations. The original
// package XML is separately preserved byte-for-byte as the primary evidence.
fn subtree(n:Node<'_,'_>)->String{
    let mut ns:Vec<(String,String)>=vec![];
    let add=|ns:&mut Vec<(String,String)>,u:Option<&str>|{
        let Some(u)=u else{return;};
        if ns.iter().any(|(uri,_)|uri==u){return;}
        let p=match u{
            "http://www.w3.org/XML/1998/namespace"=>"xml".into(),"http://www.w3.org/1999/xhtml"=>"html".into(),
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#"=>"rdf".into(),"http://schemas.xmlsoap.org/wsdl/"=>"wsdl".into(),
            "http://www.w3.org/2001/XMLSchema"=>"xs".into(),"http://www.w3.org/2001/XMLSchema-instance"=>"xsi".into(),
            "http://purl.org/dc/elements/1.1/"=>"dc".into(),_=>format!("ns{}",ns.len()),
        };ns.push((u.into(),p));
    };
    for x in n.descendants().filter(|x|x.is_element()){
        add(&mut ns,x.tag_name().namespace());
        for a in x.attributes(){add(&mut ns,a.namespace());}
    }
    let prefix:HashMap<_,_>=ns.iter().map(|(u,p)|(u.as_str(),p.as_str())).collect();
    fn qualified(ns:Option<&str>,name:&str,prefix:&HashMap<&str,&str>)->String{
        match ns.and_then(|s|prefix.get(s)){Some(p)=>format!("{p}:{name}"),None=>name.to_owned()}
    }
    fn write(n:Node<'_,'_>,prefix:&HashMap<&str,&str>,root:bool,out:&mut String){
        let name=qualified(n.tag_name().namespace(),n.tag_name().name(),prefix);
        out.push('<');out.push_str(&name);
        if root{
            let mut decl:Vec<_>=prefix.iter().filter(|(_,p)|**p!="xml").collect();decl.sort_by_key(|(_,p)|**p);
            for (uri,p)in decl{out.push_str(&format!(" xmlns:{p}=\"{}\"",xml_escape(uri,true)));}
        }
        for a in n.attributes(){out.push_str(&format!(" {}=\"{}\"",qualified(a.namespace(),a.name(),prefix),xml_escape(a.value(),true)));}
        if n.children().any(|c|c.is_element()||c.is_text()){
            out.push('>');for c in n.children(){if c.is_element(){write(c,prefix,false,out);}else if c.is_text(){out.push_str(&xml_escape(c.text().unwrap_or(""),false));}}
            out.push_str("</");out.push_str(&name);out.push('>');
        }else{out.push_str(" />");}
    }
    let mut out=String::new();write(n,&prefix,true,&mut out);
    // ElementTree's standalone serialization also contains its immediate tail.
    let mut tail=n.next_sibling();
    while let Some(c)=tail{if c.is_element(){break;}if c.is_text(){out.push_str(&xml_escape(c.text().unwrap_or(""),false));}tail=c.next_sibling();}
    out
}
fn xml_text_value(n:Node<'_,'_>)->String{
    n.children().take_while(|c|!c.is_element()).filter_map(|c|if c.is_text(){c.text()}else{None}).collect::<Vec<_>>().concat()
}
fn effect(n:Node<'_,'_>)->bool{
    n.descendants().skip(1).filter(|n|n.is_element()).any(|n|!(tag(n,A,"extLst")||tag(n,A,"ext")||tag(n,A14,"useLocalDpi")))
}
struct Scan<'a>{
    id:&'a str,relations:HashMap<String,Value>,media:HashMap<String,Value>,
    issues:Vec<Value>,calls:Vec<Value>,blocks:Vec<Value>,
}
impl<'a> Scan<'a>{
    fn issue(&mut self,kind:&str,loc:&Value,detail:String,block:Option<&str>)->String{
        let id=format!("{}-native-issue-{}",self.id,self.issues.len()+1);
        let mut row=json!({"id":id,"kind":kind,"locator":loc,"detail":detail,"status":"pending"});
        if let Some(b)=block{row["block_id"]=json!(b);}
        self.issues.push(row);id
    }
    fn paragraph(&mut self,node:Node<'_,'_>,indices:&[usize],block:&str)->Value{
        fn visit(scan:&mut Scan<'_>,n:Node<'_,'_>,p:&[usize],block:&str,text:&mut String,events:&mut Vec<Value>,own:&mut Vec<String>){
            let l=local(n);let loc=locator(p);
            if let Some(kind)=unsupported(l){
                let pending=scan.issue(kind,&loc,format!("Inspect original OOXML {l}; no reading order or rendered meaning is inferred."),Some(block));
                events.push(json!({"kind":"pending_object","issue":pending,"locator":loc,"xml":subtree(n)}));
                if !matches!(l,"anchor"|"pict"){return;}
            }
            if tag(n,W,"t")||tag(n,W,"noBreakHyphen")||tag(n,W,"tab")||tag(n,W,"br")||tag(n,W,"cr"){
                let (kind,value)=match l{"t"=>("text",xml_text_value(n)),"noBreakHyphen"=>(l,"\u{2011}".to_owned()),"tab"=>(l,"\t".into()),_=>(l,"\n".into())};
                text.push_str(&value);events.push(json!({"kind":kind,"text":value,"locator":loc}));return;
            }
            if tag(n,W,"hyperlink"){
                let id=attr(n,R,"id");let relationship=id.and_then(|id|scan.relations.get(id));
                events.push(json!({"kind":"hyperlink","locator":loc,"relationship_id":id,"anchor":attr(n,W,"anchor"),
                    "target":relationship.map(|r|&r["Target"])}));
                scan.issue("hyperlink_target",&loc,"Inspect this source hyperlink target as well as its visible text; the target is not inferred from that text.".into(),Some(block));
            }
            if tag(n,A,"blip")||tag(n,V,"imagedata"){
                let relation_id=attr(n,R,"embed").filter(|s|!s.is_empty()).or_else(||attr(n,R,"link").filter(|s|!s.is_empty())).or_else(||attr(n,R,"id"));
                let id=format!("{}-image-call-{}",scan.id,scan.calls.len()+1);
                let mut call=json!({"id":id,"block_id":block,"locator":loc,"relationship_id":relation_id,
                    "media_id":null,"status":"resolved","figure_number":null});
                match relation_id.and_then(|id|scan.relations.get(id)){
                    None=>call["status"]=json!("missing_relationship"),
                    Some(r) if r["TargetMode"]=="External"=>{call["status"]=json!("external_relationship");call["external_target"]=r["Target"].clone();},
                    Some(r) if !r["Type"].as_str().unwrap_or("").ends_with("/image")=>call["status"]=json!("non_image_relationship"),
                    Some(r)=>match r["resolved_part"].as_str().and_then(|part|scan.media.get(part)){
                        Some(m)=>call["media_id"]=m["id"].clone(),
                        None=>{call["status"]=json!("missing_media");call["target_part"]=r["resolved_part"].clone();}
                    },
                }
                let status=call["status"].as_str().unwrap().to_owned();
                scan.calls.push(call);own.push(id.clone());events.push(json!({"kind":"image","media_call":id,"locator":loc}));
                if status!="resolved"{scan.issue(&status,&loc,format!("Image call {id} cannot be resolved to embedded bytes; no external content was fetched."),Some(block));}
                if effect(n){scan.issue("image_effect",&loc,"DrawingML image effects require a visible-source comparison.".into(),Some(block));}
                return;
            }
            if l=="srcRect"&&n.attributes().any(|a|!matches!(a.value(),"0"|"0.0")){
                scan.issue("image_crop",&loc,"OOXML applies an image crop; native media bytes alone do not reproduce that visible crop.".into(),Some(block));
            }
            if l=="xfrm"&&["rot","flipH","flipV"].iter().any(|s|n.attribute(*s).is_some()){
                scan.issue("image_transform",&loc,"OOXML applies rotation or reflection; compare a rendered source before delivering pixels.".into(),Some(block));
            }
            for(i,c)in kids(n).enumerate(){visit(scan,c,&at(p,i),block,text,events,own);}
        }
        let mut text=String::new();let mut events=vec![];let mut own=vec![];
        visit(self,node,indices,block,&mut text,&mut events,&mut own);
        let style=child(node,"pPr").and_then(|n|child(n,"pStyle")).and_then(|n|attr(n,W,"val"));
        json!({"text":text,"inline_content":events,"media_calls":own,"style_id":style})
    }
    fn pending(&mut self,n:Node<'_,'_>,indices:&[usize],container:&str,kind:&str,block:&str)->Value{
        let loc=locator(indices);
        let id=self.issue(kind,&loc,format!("Unsupported {} content in a {container}; inspect the original XML before resolving source coverage.",local(n)),Some(block));
        json!({"issue":id,"container":container,"locator":loc,"xml":subtree(n)})
    }
    fn append(&mut self,n:Node<'_,'_>,indices:&[usize]){
        let l=local(n);
        if n.tag_name().namespace()==Some(W)&&matches!(l,"sectPr"|"bookmarkStart"|"bookmarkEnd"|"proofErr"){return;}
        let id=format!("{}-docx-b{}",self.id,self.blocks.len()+1);
        let mut base=json!({"id":id,"kind":l,"order":self.blocks.len(),"locator":locator(indices)});
        if tag(n,W,"p"){
            base["kind"]=json!("paragraph");let p=self.paragraph(n,indices,&id);
            base.as_object_mut().unwrap().extend(p.as_object().unwrap().clone());
        }else if tag(n,W,"tbl"){
            let mut rows=vec![];let mut calls=vec![];let mut unparsed=vec![];let mut table_text=vec![];
            for(i,r)in kids(n).enumerate(){
                let rp=at(indices,i);
                if !tag(r,W,"tr"){if !ignored(r,"table"){unparsed.push(self.pending(r,&rp,"table","table_content_container",&id));}continue;}
                let mut cells=vec![];let mut row_text=vec![];
                for(j,c)in kids(r).enumerate(){
                    let cp=at(&rp,j);
                    if !tag(c,W,"tc"){if !ignored(c,"row"){unparsed.push(self.pending(c,&cp,"row","table_content_container",&id));}continue;}
                    let mut paragraphs=vec![];let mut cell_text=vec![];
                    for(k,e)in kids(c).enumerate(){
                        let ep=at(&cp,k);
                        if tag(e,W,"p"){
                            let mut p=self.paragraph(e,&ep,&id);p["locator"]=locator(&ep);
                            cell_text.push(p["text"].as_str().unwrap().to_owned());
                            calls.extend(p["media_calls"].as_array().unwrap().iter().cloned());paragraphs.push(p);
                        }else if tag(e,W,"tbl"){unparsed.push(self.pending(e,&ep,"cell","nested_table",&id));}
                        else if !ignored(e,"cell"){unparsed.push(self.pending(e,&ep,"cell","table_content_container",&id));}
                    }
                    let pr=child(c,"tcPr");
                    let span=match pr.and_then(|n|child(n,"gridSpan")){Some(n)=>attr(n,W,"val"),None=>Some("1")};
                    let merge=pr.and_then(|n|child(n,"vMerge")).map(|n|attr(n,W,"val").unwrap_or("continue"));
                    let text=cell_text.join("\n");row_text.push(text.clone());
                    cells.push(json!({"locator":locator(&cp),"paragraphs":paragraphs,"text":text,"grid_span":span,"vertical_merge":merge}));
                }
                table_text.push(row_text.join("\t"));rows.push(json!({"cells":cells}));
            }
            base["kind"]=json!("table");base["rows"]=json!(rows);base["media_calls"]=json!(calls);base["text"]=json!(table_text.join("\n"));
            if !unparsed.is_empty(){base["unparsed_content"]=json!(unparsed);}
        }else{
            self.issue("body_container",&locator(indices),format!("Unsupported body container {l}; inspect original XML before deciding reading order."),Some(&id));
            base["kind"]=json!("unsupported");base["text"]=json!("");base["media_calls"]=json!([]);base["xml"]=json!(subtree(n));
        }
        self.blocks.push(base);
    }
}

fn hash_member(z:&mut ZipArchive<File>,part:&str,mut sink:Option<&mut dyn Write>)->Result<(String,u64)>{
    let mut f=z.by_name(part)?;
    ensure!(f.size()<=MAX_MEDIA,"Embedded media exceeds safe read budget: {part}");
    let mut buffer=vec![0u8;1024*1024];let mut hash=Sha256::new();let mut size=0u64;
    loop{
        let n=f.read(&mut buffer)?;if n==0{break;}
        size+=n as u64;ensure!(size<=MAX_MEDIA,"Embedded media exceeded safe read budget: {part}");
        hash.update(&buffer[..n]);
        if let Some(s)=sink.as_mut(){s.write_all(&buffer[..n])?;}
    }
    Ok((format!("{:x}",hash.finalize()),size))
}
fn extension(part:&str)->&str{
    let name=part.rsplit('/').next().unwrap_or(part);
    match name.rsplit_once('.') {Some((a,b)) if !a.is_empty()&&!b.is_empty()=>b,_=>""}
}
fn scan(z:&mut ZipArchive<File>,id:&str,mut progress:impl FnMut(usize,usize,&str))->Result<(Value,Vec<u8>)>{
    ensure!(rx!(r"^[A-Za-z][A-Za-z0-9_-]*$").is_match(id),"A stable task-local source ID is required");
    ensure!(z.len()<=100_000,"DOCX ZIP member count exceeds safe limit");
    let mut names=HashSet::new();let mut parts=vec![];
    for i in 0..z.len(){let f=z.by_index(i)?;let name=f.name().to_owned();ensure!(names.insert(name.clone()),"DOCX contains duplicate ZIP member names");parts.push(name);}
    ensure!(names.contains(DOCUMENT)&&names.contains(TYPES),"An accessible DOCX WordprocessingML package is required");
    let document_bytes=member(z,DOCUMENT,MAX_XML)?;
    let document=xml_text(&document_bytes,DOCUMENT)?;let doc=parse(&document,DOCUMENT)?;
    let root=doc.root_element();ensure!(tag(root,W,"document"),"Only transitional WordprocessingML documents are supported by this adapter");
    let bodies=kids(root).enumerate().filter(|(_,n)|tag(*n,W,"body")).collect::<Vec<_>>();
    ensure!(bodies.len()==1,"DOCX must have exactly one document body");let (body_index,body)=bodies[0];
    let types=xml_text(&member(z,TYPES,MAX_XML)?,TYPES)?;let types=parse(&types,TYPES)?;
    let mut defaults=HashMap::new();let mut overrides=HashMap::new();
    for n in kids(types.root_element()){
        if local(n)=="Default"{defaults.insert(n.attribute("Extension").unwrap_or("").to_ascii_lowercase(),n.attribute("ContentType").unwrap_or("").to_owned());}
        if local(n)=="Override"{overrides.insert(n.attribute("PartName").unwrap_or("").trim_start_matches('/').to_owned(),n.attribute("ContentType").unwrap_or("").to_owned());}
    }
    let content_type=|part:&str|overrides.get(part).or_else(||defaults.get(&extension(part).to_ascii_lowercase())).map(String::as_str);
    let mut relations=vec![];let mut relation_map=HashMap::new();
    if names.contains(RELS){
        let s=xml_text(&member(z,RELS,MAX_XML)?,RELS)?;let doc=parse(&s,RELS)?;
        for node in kids(doc.root_element()).filter(|n|local(*n)=="Relationship"){
            let rid=node.attribute("Id").unwrap_or("");let kind=node.attribute("Type").unwrap_or("");
            let target=node.attribute("Target").unwrap_or("");let mode=node.attribute("TargetMode").unwrap_or("");
            ensure!(!rid.is_empty()&&!relation_map.contains_key(rid),"DOCX document relationships require unique nonempty IDs");
            let resolved=if mode=="External"{None}else{target_part(target)};
            let row=json!({"Id":rid,"Type":kind,"Target":target,"TargetMode":mode,"resolved_part":resolved});
            relation_map.insert(rid.to_owned(),row.clone());relations.push(row);
        }
    }
    let mut media_parts:Vec<_>=parts.iter().filter(|p|!p.ends_with('/')&&(p.starts_with("word/media/")||content_type(p).is_some_and(|s|s.starts_with("image/")))).collect();
    media_parts.sort();
    let mut media=vec![];let mut media_by_part=HashMap::new();let mut total=0u64;
    for(i,part)in media_parts.iter().enumerate(){
        let (hash,size)=hash_member(z,part,None)?;
        total=total.checked_add(size).context("embedded-media size overflow")?;ensure!(total<=MAX_TOTAL_MEDIA,"DOCX media total exceeds safe budget");
        let row=json!({"id":format!("{id}-media-{}",i+1),"part":part,"sha256":hash,"bytes":size,
            "content_type":content_type(part).unwrap_or("application/octet-stream")});
        media_by_part.insert((*part).to_owned(),row.clone());media.push(row);
    }
    let mut scan=Scan{id,relations:relation_map,media:media_by_part,issues:vec![],calls:vec![],blocks:vec![]};
    let count=kids(body).count();
    for(i,n)in kids(body).enumerate(){scan.append(n,&[body_index,i]);progress(i+1,count,&format!("{id} native block {}",i+1));}
    let mut previous=vec![];let mut next=vec![String::new();scan.blocks.len()];let mut latest="";
    for block in &scan.blocks{
        previous.push(latest.to_owned());let text=block["text"].as_str().unwrap_or("");if !text.trim().is_empty(){latest=text;}
    }
    latest="";
    for(i,block)in scan.blocks.iter().enumerate().rev(){next[i]=latest.to_owned();let text=block["text"].as_str().unwrap_or("");if !text.trim().is_empty(){latest=text;}}
    let block_index:HashMap<_,_>=scan.blocks.iter().enumerate().map(|(i,b)|(b["id"].as_str().unwrap(),i)).collect();
    let mut used=HashSet::new();
    for call in &mut scan.calls{
        let i=block_index[call["block_id"].as_str().unwrap()];
        call["nearby_text"]=json!({"same_block":scan.blocks[i]["text"],"previous":previous[i],"next":next[i]});
        if let Some(s)=call["media_id"].as_str(){used.insert(s.to_owned());}
    }
    for m in &media{
        if !used.contains(m["id"].as_str().unwrap()){
            scan.issue("unresolved_media_use",&json!({"part":m["part"]}),"This embedded media part has no resolved body image call; inspect headers, footers, drawings or unused package content.".into(),None);
        }
    }
    for r in &relations{
        let role=r["Type"].as_str().unwrap_or("").rsplit('/').next().unwrap_or("");
        if matches!(role,"header"|"footer"|"footnotes"|"endnotes"|"comments"|"oleObject"|"chart"|"aFChunk"){
            scan.issue("additional_part",&json!({"part":r["resolved_part"]}),format!("Inspect related {role} part before declaring source coverage; relationship {}.",r["Id"].as_str().unwrap_or("")),None);
        }
    }
    Ok((json!({"format":"docx","page_count":null,"pages":[],"document_part":{"part":DOCUMENT,"sha256":files::digest(&document_bytes)},
        "relationships":relations,"native_blocks":scan.blocks,"media_calls":scan.calls,"native_media":media,"native_issues":scan.issues,
        "native_boundary":"XML order and embedded bytes are provenance, not figure numbering, pagination, displayed appearance or semantic acceptance."}),document_bytes))
}
pub fn read_docx(path:&Path,id:&str,progress:impl FnMut(usize,usize,&str))->Result<Value>{
    let p=files::canonical(path)?;let before=files::snapshot(&p)?;
    let (result,_)=scan(&mut open(&p)?,id,progress)?;
    ensure!(files::snapshot(&p)?==before,"DOCX source changed during inventory");Ok(result)
}
fn exclusive(directory:&Path,name:&str,data:&[u8])->Result<Value>{
    let path=directory.join(name);
    let mut f=tempfile::Builder::new().prefix(".pending-").tempfile_in(directory)?;
    f.write_all(data)?;f.as_file().sync_all()?;
    f.persist_noclobber(&path).map_err(|e|anyhow!("Cannot exclusively save {}: {}",path.display(),e.error))?;
    let observed=files::snapshot(&path)?;ensure!(observed["sha256"]==files::digest(data),"Native evidence readback mismatch");
    Ok(json!({"path":observed["path"],"sha256":observed["sha256"]}))
}
pub fn prepare_docx_source(path:&Path,id:&str,directory:&Path,progress:impl FnMut(usize,usize,&str))->Result<Value>{
    let p=files::canonical(path)?;let before=files::snapshot(&p)?;let mut z=open(&p)?;
    let (mut result,xml)=scan(&mut z,id,progress)?;
    ensure!(!directory.is_symlink(),"Refusing a symlink evidence directory");
    fs::create_dir_all(directory)?;let directory=files::canonical(directory)?;
    let mut media_names=Vec::new();
    for(i,m)in result["native_media"].as_array().unwrap().iter().enumerate(){
        let ext=extension(m["part"].as_str().unwrap());
        let ext=if rx!(r"^[A-Za-z0-9]{1,12}$").is_match(ext){ext}else{"bin"};
        media_names.push(format!("media-{:04}.{ext}",i+1));
    }
    for name in ["document.xml","native-blocks.txt"].into_iter().chain(media_names.iter().map(String::as_str)){
        match fs::symlink_metadata(directory.join(name)){Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},Err(e)=>return Err(e.into()),Ok(_)=>return Err(anyhow!("Evidence file already exists: {}",directory.join(name).display()))}
    }
    result["native_document"]=exclusive(&directory,"document.xml",&xml)?;
    for(m,name)in result["native_media"].as_array_mut().unwrap().iter_mut().zip(media_names){
        let mut temp=tempfile::Builder::new().prefix(".pending-").tempfile_in(&directory)?;
        let (hash,len)=hash_member(&mut z,m["part"].as_str().unwrap(),Some(&mut temp))?;
        ensure!(m["sha256"]==hash&&m["bytes"]==json!(len),"Embedded media changed during evidence preservation");
        temp.as_file().sync_all()?;let path=directory.join(name);
        temp.persist_noclobber(&path).map_err(|e|anyhow!("Cannot exclusively save {}: {}",path.display(),e.error))?;
        let observed=files::snapshot(&path)?;ensure!(observed["sha256"]==hash,"Native media readback mismatch");
        m["file"]=json!({"path":observed["path"],"sha256":observed["sha256"]});
    }
    let mut text=String::new();
    for b in result["native_blocks"].as_array().unwrap(){text+=&format!("[{} | {}]\n{}\n\n",b["id"].as_str().unwrap(),b["kind"].as_str().unwrap(),b["text"].as_str().unwrap());}
    result["native_text"]=exclusive(&directory,"native-blocks.txt",text.as_bytes())?;
    ensure!(files::snapshot(&p)?==before,"DOCX source changed during preservation; no inventory result was issued");
    Ok(result)
}
fn indexed<'a>(source:&'a Value,key:&str)->HashMap<&'a str,&'a Value>{
    source[key].as_array().into_iter().flatten().filter_map(|v|v["id"].as_str().map(|id|(id,v))).collect()
}
pub fn evidence_errors(source:&Value,location:&Value)->Vec<String>{
    let mut errors:Vec<String>=vec![];
    if !location.is_object(){return vec!["A native source location must be an object.".into()];}
    if location["basis"]!="native"{errors.push("Declare basis=native for an OOXML location.".into());}
    if ["page","bbox","image_xref","coordinate_system"].iter().any(|k|location.get(k).is_some()){
        errors.push("A native OOXML location must not invent page-space geometry.".into());
    }
    let locator=&location["native_locator"];
    if !valid_locator(locator){errors.push("Use the recorded document part and zero-based XML element_path.".into());}
    let blocks=indexed(source,"native_blocks");
    let Some(references)=location["blocks"].as_array().filter(|a|!a.is_empty()&&a.iter().all(|v|v.as_str().is_some_and(|s|blocks.contains_key(s))))else{
        errors.push("Name the actual native source block IDs.".into());return errors;
    };
    let refs:Vec<_>=references.iter().map(|r|r.as_str().unwrap()).collect();
    if refs.iter().collect::<HashSet<_>>().len()!=refs.len(){errors.push("Do not repeat source block IDs within one location.".into());}
    if !location["media_call"].is_null(){
        let calls=indexed(source,"media_calls");
        match location["media_call"].as_str().and_then(|s|calls.get(s)){
            Some(c) if c["status"]=="resolved"=>{if locator!=&c["locator"]||references.len()!=1||references[0]!=c["block_id"]{
                errors.push("Use this image call's exact native locator and parent block.".into());}},
            _=>errors.push("The image call must resolve to an embedded media part.".into()),
        }
    }else if references.len()!=1||locator!=&blocks[refs[0]]["locator"]{errors.push("Each native text/table location must identify exactly one original block.".into());}
    if !location["quote"].is_null(){
        let text=refs.iter().map(|r|blocks[r]["text"].as_str().unwrap_or("")).collect::<Vec<_>>().join("\n");
        if !location["quote"].as_str().is_some_and(|s|!s.is_empty()&&text.contains(s)){errors.push("The declared quote must occur in its native source block.".into());}
    }
    errors
}
pub fn source_text(source:&Value,location:&Value)->Result<String>{
    let errors=evidence_errors(source,location);ensure!(errors.is_empty(),"{}",errors.join(" "));
    let blocks=indexed(source,"native_blocks");
    Ok(location["blocks"].as_array().unwrap().iter().map(|r|blocks[r.as_str().unwrap()]["text"].as_str().unwrap_or("")).collect::<Vec<_>>().join("\n"))
}
pub struct NativeDocument{pub inventory:Value,text:HashMap<Vec<u64>,String>}
fn indices(locator:&Value)->Result<Vec<u64>>{
    ensure!(valid_locator(locator),"Use the exact native part and XML child-index path, without page/bbox fields");
    Ok(locator["element_path"].as_array().unwrap().iter().map(|v|v.as_u64().unwrap()).collect())
}
impl NativeDocument{
    pub fn new(p:&Path)->Result<Self>{
        let inventory=read_docx(p,"native",|_,_,_|{})?;
        let blocks=indexed(&inventory,"native_blocks");let mut text=HashMap::new();
        for b in blocks.values(){text.insert(indices(&b["locator"])?,b["text"].as_str().unwrap_or("").to_owned());}
        for c in inventory["media_calls"].as_array().unwrap(){
            let b=blocks.get(c["block_id"].as_str().unwrap()).context("image call parent block")?;
            text.insert(indices(&c["locator"])?,b["text"].as_str().unwrap_or("").to_owned());
        }
        Ok(Self{inventory,text})
    }
    pub fn resolve(&self,locator:&Value)->Result<&str>{self.text.get(&indices(locator)?).map(String::as_str).context("The native locator does not identify an actual body block or image call")}
}
pub fn figure_location(source:&Value,location:&Value)->Result<Value>{
    let errors=evidence_errors(source,location);ensure!(errors.is_empty(),"{}",errors.join(" "));
    let calls=indexed(source,"media_calls");
    let call=location["media_call"].as_str().and_then(|id|calls.get(id)).context("A native figure location needs an explicit media_call")?;
    let media=indexed(source,"native_media");let item=call["media_id"].as_str().and_then(|id|media.get(id)).context("missing embedded media")?;
    let file=&item["file"];ensure!(file.as_object().is_some_and(|m|m.len()==2&&m.contains_key("path")&&m.contains_key("sha256")),"The original embedded media needs its actual file binding");
    Ok(json!({"native_locator":call["locator"],"media_call":call["id"],"relationship_id":call["relationship_id"],"source_media":file}))
}
