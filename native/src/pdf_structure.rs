//! Structural PDF evidence: page tree, actual indirect image calls, original
//! stream bytes and clipping/graphics-state flags. Does not claim visibility,
//! raster extraction, pixel reconstruction, or rendered-page parity.
use crate::{files,path_text,check_ledger,pipeline};
use anyhow::{anyhow,ensure,Context,Result};
use lopdf::{Document,Object,ObjectId,Dictionary,content::Content};
use serde_json::{json,Value};
use std::{collections::{BTreeMap,HashSet},fs,path::{Path,PathBuf},sync::Mutex};

const MAX_PDF:u64=512*1024*1024;
const MAX_OPS:usize=1_000_000;
const MAX_FORM:usize=64;
const MAX_CONTENT:usize=128*1024*1024;
type Matrix=[f64;6];
const ID:Matrix=[1.,0.,0.,1.,0.,0.];
fn load(p:&Path)->Result<Document>{
    ensure!(fs::metadata(p)?.len()<=MAX_PDF,"PDF exceeds structural evidence read budget");
    let doc=Document::load(p)?;
    ensure!(!doc.is_encrypted(),"An accessible unencrypted PDF is required");
    ensure!(!doc.get_pages().is_empty(),"PDF page tree is empty");Ok(doc)
}
fn deref<'a>(doc:&'a Document,mut v:&'a Object)->Result<&'a Object>{
    let mut seen=HashSet::new();
    for _ in 0..128{
        if let Object::Reference(id)=v{ensure!(seen.insert(*id),"PDF reference cycle");v=doc.get_object(*id)?;}
        else{return Ok(v);}
    }Err(anyhow!("PDF reference chain exceeds limit"))
}
fn number(v:&Object)->Result<f64>{
    let n=match v{Object::Integer(n)=>*n as f64,Object::Real(n)=>*n as f64,_=>return Err(anyhow!("Expected PDF number"))};
    ensure!(n.is_finite(),"PDF number must be finite");Ok(n)
}
fn array<const N:usize>(doc:&Document,obj:&Object)->Result<[f64;N]>{
    let v=deref(doc,obj)?.as_array()?;ensure!(v.len()==N,"Expected {N} PDF numbers");
    let mut out=[0.;N];for i in 0..N{out[i]=number(deref(doc,&v[i])?)?;}Ok(out)
}
fn inherited<'a>(doc:&'a Document,mut id:ObjectId,key:&[u8])->Result<Option<&'a Object>>{
    let mut seen=HashSet::new();
    for _ in 0..256{
        ensure!(seen.insert(id),"PDF page-tree cycle");
        let d=doc.get_object(id)?.as_dict()?;
        if let Ok(value)=d.get(key){return Ok(Some(deref(doc,value)?));}
        match d.get(b"Parent"){Ok(Object::Reference(p))=>id=*p,Ok(_)=>return Err(anyhow!("Invalid PDF page parent")),Err(_)=>return Ok(None)}
    }Err(anyhow!("PDF page-tree depth exceeds limit"))
}
fn ordered(b:[f64;4])->[f64;4]{[b[0].min(b[2]),b[1].min(b[3]),b[0].max(b[2]),b[1].max(b[3])]}
fn mul(a:Matrix,b:Matrix)->Matrix{
    [a[0]*b[0]+a[1]*b[2],a[0]*b[1]+a[1]*b[3],a[2]*b[0]+a[3]*b[2],a[2]*b[1]+a[3]*b[3],
        a[4]*b[0]+a[5]*b[2]+b[4],a[4]*b[1]+a[5]*b[3]+b[5]]
}
fn inverse(a:Matrix)->Result<Matrix>{
    let det=a[0]*a[3]-a[1]*a[2];ensure!(det.is_finite()&&det!=0.,"Singular Form transformation");
    let m=[a[3]/det,-a[1]/det,-a[2]/det,a[0]/det,(a[2]*a[5]-a[3]*a[4])/det,(a[1]*a[4]-a[0]*a[5])/det];
    ensure!(m.iter().all(|n|n.is_finite()),"Nonfinite inverse matrix");Ok(m)
}
fn point(m:Matrix,x:f64,y:f64)->[f64;2]{[m[0]*x+m[2]*y+m[4],m[1]*x+m[3]*y+m[5]]}
fn bbox(m:Matrix)->Result<[f64;4]>{
    let mut b=[f64::INFINITY,f64::INFINITY,f64::NEG_INFINITY,f64::NEG_INFINITY];
    for (x,y) in [(0.,0.),(0.,1.),(1.,0.),(1.,1.)]{
        let p=point(m,x,y);b[0]=b[0].min(p[0]);b[1]=b[1].min(p[1]);b[2]=b[2].max(p[0]);b[3]=b[3].max(p[1]);
    }
    ensure!(b.iter().all(|v|v.is_finite())&&b[0]<b[2]&&b[1]<b[3],"Degenerate image transformation");Ok(b)
}
fn inside(outer:[f64;4],inner:[f64;4])->bool{outer[0]<=inner[0]&&outer[1]<=inner[1]&&inner[2]<=outer[2]&&inner[3]<=outer[3]}
pub struct Pdf{
    doc:Document,
    pages:BTreeMap<u32,ObjectId>,
    invoked_xrefs:Mutex<BTreeMap<u64,HashSet<u32>>>,
}
impl Pdf{
    pub fn open(p:&Path)->Result<Self>{let doc=load(p)?;let pages=doc.get_pages();Ok(Self{doc,pages,invoked_xrefs:Mutex::default()})}
    /// Use the very same immutable byte snapshot as the raster/text backend.
    pub(crate) fn from_bytes(bytes:&[u8])->Result<Self>{
        ensure!(bytes.len() as u64<=MAX_PDF,"PDF exceeds structural evidence read budget");
        let doc=Document::load_mem(bytes)?;
        ensure!(!doc.is_encrypted(),"An accessible unencrypted PDF is required");
        let pages=doc.get_pages();
        ensure!(!pages.is_empty(),"PDF page tree is empty");
        Ok(Self{doc,pages,invoked_xrefs:Mutex::default()})
    }
    pub(crate) fn geometry_record(&self,n:u64)->Result<Value>{Ok(self.geometry(n)?.value())}
    pub fn len(&self)->usize{self.pages.len()}
    fn page(&self,n:u64)->Result<ObjectId>{let n=u32::try_from(n)?;self.pages.get(&n).copied().context("page_number must name an actual PDF page")}
    fn geometry(&self,n:u64)->Result<Geometry>{
        let id=self.page(n)?;let doc=&self.doc;
        let media=ordered(array::<4>(doc,inherited(doc,id,b"MediaBox")?.context("Missing PDF MediaBox")?)?);
        let crop=match inherited(doc,id,b"CropBox")?{Some(b)=>ordered(array::<4>(doc,b)?),None=>media};
        let clipped=[media[0].max(crop[0]),media[1].max(crop[1]),media[2].min(crop[2]),media[3].min(crop[3])];
        let d=doc.get_object(id)?.as_dict()?;
        let unit=match d.get(b"UserUnit"){Ok(v)=>number(deref(doc,v)?)?,Err(_)=>1.};
        let w=(clipped[2]-clipped[0])*unit;let h=(clipped[3]-clipped[1])*unit;
        ensure!(w>0.&&h>0.&&unit>0.&&w.is_finite()&&h.is_finite(),"Invalid PDF page geometry");
        let rotation=match inherited(doc,id,b"Rotate")?{Some(v)=>number(v)?,None=>0.};
        ensure!(rotation.fract()==0.&&rotation%90.==0.,"Invalid PDF page rotation");
        let rotation=rotation.rem_euclid(360.) as i32;
        let rotated=match rotation{90=>[0.,1.,-1.,0.,h,0.],180=>[-1.,0.,0.,-1.,w,h],270=>[0.,-1.,1.,0.,0.,w],_=>ID};
        Ok(Geometry{bounds:[0.,0.,w,h],media,crop,rotation,unit,matrix:[unit,0.,0.,-unit,-clipped[0]*unit,clipped[3]*unit],rotated})
    }
    pub fn bounds(&self,n:u64)->Result<[f64;4]>{Ok(self.geometry(n)?.bounds)}
    /// Collect the image resources actually reachable from the page/Form resource
    /// dictionaries. This is a resource lookup, not evidence of a painted image.
    pub fn resource_image_xrefs(&self,n:u64)->Result<HashSet<u32>>{
        fn visit(doc:&Document,res:&Dictionary,seen:&mut HashSet<ObjectId>,
                 images:&mut HashSet<u32>,depth:usize,budget:&mut usize)->Result<()>{
            ensure!(depth<=MAX_FORM,"PDF Form resource depth exceeds limit");
            let Some(xobjects)=dictionary(doc,res,b"XObject")? else{return Ok(());};
            for (_,value) in xobjects.iter(){
                *budget+=1;ensure!(*budget<=MAX_OPS,"PDF resource traversal exceeds budget");
                let id=match value{Object::Reference(id)=>Some(*id),_=>None};
                if let Some(id)=id{if !seen.insert(id){continue;}}
                let stream=deref(doc,value)?.as_stream()?;
                let subtype=deref(doc,stream.dict.get(b"Subtype")?)?.as_name()?;
                match subtype{
                    b"Image"=>{if let Some(id)=id{images.insert(id.0);}},
                    b"Form"=>{if let Some(nested)=dictionary(doc,&stream.dict,b"Resources")?{
                        visit(doc,nested,seen,images,depth+1,budget)?;
                    }},
                    _=>{},
                }
            }
            Ok(())
        }
        let mut images=HashSet::new();
        let mut seen=HashSet::new();
        let mut budget=0;
        if let Some(resources)=inherited(&self.doc,self.page(n)?,b"Resources")?{
            visit(&self.doc,resources.as_dict()?,&mut seen,&mut images,0,&mut budget)?;
        }
        Ok(images)
    }
    /// Native evidence requires an actual image paint call, not a resource
    /// dictionary entry. Keep the legacy resource-only lookup separate.
    /// An occurrence still does not prove visibility, clipping or fidelity.
    pub(crate) fn matches_image_locator(&self,n:u64,xref:u32,native:bool)->Result<bool>{
        if !native{return Ok(self.resource_image_xrefs(n)?.contains(&xref));}
        let mut cache=self.invoked_xrefs.lock().map_err(|_|anyhow!("PDF image locator cache unavailable"))?;
        if let Some(found)=cache.get(&n){return Ok(found.contains(&xref));}
        let index=self.index(n)?;
        let rows=index["occurrences"].as_array().context("Missing PDF image occurrence index")?;
        let mut found=HashSet::new();
        for row in rows{found.insert(u32::try_from(row["image_xref"].as_u64().context("Actual image XRef")?)?);}
        let matches=found.contains(&xref);
        // Bound the cache independently of the number of pages and sources.
        const MAX_CACHED_XREFS:usize=16_384;
        if cache.values().map(HashSet::len).sum::<usize>()+found.len()<=MAX_CACHED_XREFS && cache.len()<256{
            cache.insert(n,found);
        }
        Ok(matches)
    }
    pub fn contains_bbox(&self,n:u64,bbox:&Value)->Result<bool>{
        Ok(valid_pdf_bbox(bbox,&json!(self.bounds(n)?)))
    }

    pub fn index(&self,n:u64)->Result<Value>{
        let id=self.page(n)?;let geometry=self.geometry(n)?;
        let empty=Dictionary::new();
        let resources=match inherited(&self.doc,id,b"Resources")?{Some(v)=>v.as_dict()?,None=>&empty};
        let mut scan=Scan{pdf:self,geometry,occurrences:vec![],issues:vec![],objects:BTreeMap::new(),skipped:vec![],ops:0};
        // The actual decoded page content retains paint order. A parser error
        // is recorded as unresolved, not an empty successfully indexed page.
        match self.doc.get_page_content(id){
            Ok(bytes)=>{if let Err(e)=scan.walk(&bytes,resources,id,ID,&[],&[],&[],&[]){scan.issue("CONTENT_UNRESOLVED",id,0,e.to_string());}},
            Err(e)=>scan.issue("CONTENT_UNRESOLVED",id,0,e.to_string())
        }
        let mut result=scan.geometry.value();
        result["status"]=json!(if scan.issues.is_empty(){"structurally_indexed"}else if scan.occurrences.is_empty(){"unresolved"}else{"partial"});
        result["occurrences"]=json!(scan.occurrences);result["objects"]=json!(scan.objects.into_values().collect::<Vec<_>>());
        result["issues"]=json!(scan.issues);result["skipped_forms"]=json!(scan.skipped);
        result["bbox_meaning"]=json!("full_object_bounds_before_clipping");
        result["figure_completeness"]=json!("not_assessed");
        result["pixel_reconstruction"]=json!("not_assessed");result["render_status"]=json!("not_rendered");
        result["evidence_boundary"]=json!("Structural object calls only; page appearance, clipping, masks, text and complete figure boundaries require independent rendering and comparison.");
        Ok(result)
    }
    pub fn raw_object(&self,xref:u32,generation:u16)->Result<&lopdf::Stream>{
        let stream=self.doc.get_object((xref,generation))?.as_stream()?;
        ensure!(stream.dict.get(b"Subtype")?.as_name()?==b"Image","xref does not identify an image object");
        Ok(stream)
    }
}
#[derive(Clone,Copy)]
struct Geometry{bounds:[f64;4],media:[f64;4],crop:[f64;4],rotation:i32,unit:f64,matrix:Matrix,rotated:Matrix}
impl Geometry{fn value(&self)->Value{json!({"coordinate_system":{"space":"pdf_page","unit":"pt","origin":"top_left","x_axis":"right","y_axis":"down","rotation":"unrotated"},
    "page_rotation":self.rotation,"page_bounds":self.bounds,"media_box_pdf_coordinates":self.media,"crop_box_pdf_coordinates":self.crop,
    "user_unit":self.unit,"pdf_to_page_transform":self.matrix,"page_to_rotated_page_transform":self.rotated})}}
fn name(v:&Object)->Result<String>{Ok(format!("/{}",String::from_utf8_lossy(v.as_name()?)))}
fn dictionary<'a>(doc:&'a Document,res:&'a Dictionary,key:&[u8])->Result<Option<&'a Dictionary>>{
    match res.get(key){Ok(v)=>Ok(Some(deref(doc,v)?.as_dict()?)),Err(_)=>Ok(None)}
}
fn append_unique(v:&mut Vec<String>,s:&str){if !v.iter().any(|x|x==s){v.push(s.to_owned());}}
#[derive(Clone)]
struct Clip{xref:u32,raw:[f64;4],bounds:[f64;4],inverse:Matrix}
struct Scan<'a>{pdf:&'a Pdf,geometry:Geometry,occurrences:Vec<Value>,issues:Vec<Value>,objects:BTreeMap<(u32,u16),Value>,skipped:Vec<Value>,ops:usize}
impl Scan<'_>{
    fn issue(&mut self,code:&str,container:ObjectId,index:usize,reason:String){
        self.issues.push(json!({"code":code,"container_xref":container.0,"operation_index":index,"reason":reason}));
    }
    fn walk(&mut self,bytes:&[u8],resources:&Dictionary,container:ObjectId,mut ctm:Matrix,forms:&[ObjectId],names:&[String],effects:&[String],clips:&[Clip])->Result<()>{
        ensure!(forms.len()<=MAX_FORM&&bytes.len()<=MAX_CONTENT,"PDF content/form limit exceeded");
        let content=Content::decode(bytes)?;let pdf=self.pdf;let doc=&pdf.doc;
        let mut state=effects.to_vec();let mut stack=Vec::new();
        for (index,op) in content.operations.iter().enumerate(){
            self.ops+=1;ensure!(self.ops<=MAX_OPS,"Page content exceeds operation limit");
            match op.operator.as_str(){
                "q"=>{stack.push((ctm,state.clone()));continue;}
                "Q"=>{(ctm,state)=stack.pop().context("Unbalanced PDF graphics-state restore")?;continue;}
                "cm"=>{
                    ensure!(op.operands.len()==6,"Malformed transformation");
                    let mut m=[0.;6];for(i,v)in op.operands.iter().enumerate(){m[i]=number(v)?;}
                    ctm=mul(m,ctm);continue;
                }
                "W"|"W*"=>{append_unique(&mut state,"page_clipping_path");continue;}
                "gs"=>{
                    if let Some(v)=op.operands.first(){
                        let gs=dictionary(doc,resources,b"ExtGState")?.context("Missing extended graphics state")?;
                        let dict=deref(doc,gs.get(v.as_name()?)?)?.as_dict()?;
                        for key in [b"SMask".as_slice(),b"ca",b"CA",b"BM",b"TR",b"TR2"]{
                            if dict.get(key).is_ok(){append_unique(&mut state,&format!("graphics_state:/{}",String::from_utf8_lossy(key)));}
                        }
                    }else{return Err(anyhow!("Malformed extended graphics state"));}
                    continue;
                }
                "BI"|"ID"|"EI"|"INLINE IMAGE"=>{self.issue("INLINE_IMAGE",container,index,"Inline image has no indirect xref; render the page region.".into());continue;}
                "Tf"=>{
                    if let Some(v)=op.operands.first(){
                        if let Some(fonts)=dictionary(doc,resources,b"Font")?{
                            if let Ok(font)=fonts.get(v.as_name()?){
                                if deref(doc,font)?.as_dict()?.get(b"Subtype").ok().and_then(|v|v.as_name().ok())==Some(b"Type3".as_slice()){
                                    self.issue("TYPE3_CONTENT",container,index,"Type 3 glyph image calls require rendering.".into());
                                }
                            }
                        }
                    }continue;
                }
                "scn"|"SCN"=>{
                    if let Some(v)=op.operands.last().and_then(|v|v.as_name().ok()){
                        if dictionary(doc,resources,b"Pattern")?.is_some_and(|p|p.get(v).is_ok()){
                            self.issue("PATTERN_CONTENT",container,index,"Pattern painting may repeat images; requires rendering.".into());
                        }
                    }continue;
                }
                "Do"=>{}
                _=>continue,
            }
            ensure!(op.operands.len()==1,"Malformed PDF XObject call");
            let resource=op.operands[0].as_name()?;
            let xobjects=dictionary(doc,resources,b"XObject")?.context("Missing XObject resources")?;
            let reference=xobjects.get(resource)?;
            let id=match reference{Object::Reference(id)=>*id,_=>return Err(anyhow!("XObject is not an indirect object"))};
            let stream=doc.get_object(id)?.as_stream()?;
            let subtype=stream.dict.get(b"Subtype")?.as_name()?;
            let mut child_names=names.to_vec();child_names.push(name(&op.operands[0])?);
            if subtype==b"Form"{
                let res=(||->Result<()>{
                    let raw=array::<4>(doc,stream.dict.get(b"BBox")?)?;let bounds=ordered(raw);
                    if bounds[0]==bounds[2]||bounds[1]==bounds[3]{
                        self.skipped.push(json!({"form_xref":id.0,"container_xref":container.0,"operation_index":index,"resource_path":child_names,"code":"EMPTY_FORM_BBOX","pdf_bbox":raw,"bbox":bounds}));return Ok(());
                    }
                    ensure!(!forms.contains(&id),"Recursive Form XObject cycle");
                    let matrix=match stream.dict.get(b"Matrix"){Ok(v)=>array::<6>(doc,v)?,Err(_)=>ID};
                    let child=mul(matrix,ctm);let mut child_forms=forms.to_vec();child_forms.push(id);
                    let mut child_clips=clips.to_vec();child_clips.push(Clip{xref:id.0,raw,bounds,inverse:inverse(child)?});
                    let mut child_state=state.clone();if stream.dict.get(b"Group").is_ok(){append_unique(&mut child_state,"form_transparency_group");}
                    let r=match stream.dict.get(b"Resources"){Ok(v)=>deref(doc,v)?.as_dict()?,Err(_)=>resources};
                    let decoded=stream.get_plain_content()?;
                    self.walk(&decoded,r,id,child,&child_forms,&child_names,&child_state,&child_clips)
                })();
                if let Err(e)=res{self.issue("FORM_UNRESOLVED",container,index,format!("form {}: {e:#}",id.0));}
                continue;
            }
            ensure!(subtype==b"Image","Unsupported XObject subtype");
            let flip=[1.,0.,0.,-1.,0.,1.];let raw=mul(flip,ctm);let transform=mul(raw,self.geometry.matrix);let b=bbox(transform)?;
            let mut effects=state.clone();
            if let Some(colors)=dictionary(doc,resources,b"ColorSpace")?{
                if [b"DefaultRGB".as_slice(),b"DefaultGray",b"DefaultCMYK"].iter().any(|k|colors.get(k).is_ok()){append_unique(&mut effects,"default_colorspace_override");}
            }
            for clip in clips{
                let transformed=bbox(mul(raw,clip.inverse))?;
                let bounds=[clip.bounds[0]-0.0001,clip.bounds[1]-0.0001,clip.bounds[2]+0.0001,clip.bounds[3]+0.0001];
                if !inside(bounds,transformed){append_unique(&mut effects,"form_bbox_clipping");break;}
            }
            if stream.dict.get(b"OC").is_ok(){append_unique(&mut effects,"optional_content_visibility");}
            let fully=inside(self.geometry.bounds,b);if !fully{append_unique(&mut effects,"page_boundary_clipping");}
            let mask=|key:&[u8]|stream.dict.get(key).ok().map(|v|match v{Object::Reference(id)=>json!({"xref":id.0,"generation":id.1}),_=>json!({"direct_or_other":true})});
            self.objects.entry(id).or_insert_with(||json!({"image_xref":id.0,"generation":id.1,"status":"raw_stream_indexed",
                "width":stream.dict.get(b"Width").ok().and_then(|v|number(v).ok()),"height":stream.dict.get(b"Height").ok().and_then(|v|number(v).ok()),
                "encoded_stream_sha256":files::digest(&stream.content),"encoded_stream_bytes":stream.content.len(),
                "smask":mask(b"SMask"),"mask":mask(b"Mask"),"figure_completeness":"not_assessed","pixel_reconstruction":"not_assessed"}));
            self.occurrences.push(json!({"occurrence":self.occurrences.len()+1,"image_xref":id.0,"generation":id.1,
                "resource_name":child_names.last(),"resource_path":child_names,"container_xref":container.0,"operation_index":index,
                "form_xrefs":forms.iter().map(|p|p.0).collect::<Vec<_>>(),"bbox":b,"transform":transform,"fully_inside_page":fully,
                "form_clips":clips.iter().map(|v|json!({"form_xref":v.xref,"bbox":v.bounds,"pdf_bbox":v.raw,"pdf_to_form_transform":v.inverse})).collect::<Vec<_>>(),
                "context_effects":effects,"visibility":"not_assessed"}));
        }
        ensure!(stack.is_empty(),"Unbalanced PDF graphics-state save");Ok(())
    }
}
pub fn index_page(p:&Path,page:u64)->Result<Value>{
    let before=check_ledger::bind_file(p)?;
    let pdf=Pdf::open(p)?;let mut v=pdf.index(page)?;
    ensure!(check_ledger::bind_file(p)?==before,"Source PDF changed during structural indexing");
    v["source_file"]=before;v["page"]=json!(page);Ok(v)
}
pub fn prepare_structure(p:&Path,dest:&Path,mut progress:impl FnMut(usize,usize,&str))->Result<PathBuf>{
    let before=check_ledger::bind_file(p)?;
    let pdf=Pdf::open(p)?;let mut pages=Vec::new();
    for n in 1..=pdf.len(){pages.push(pdf.index(n as u64)?);progress(n,pdf.len(),"PDF structural image calls");}
    ensure!(check_ledger::bind_file(p)?==before,"Source PDF changed during indexing");
    let parent=dest.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or_else(||Path::new("."));
    fs::create_dir_all(parent)?;
    let dest=files::canonical(parent)?.join(dest.file_name().context("destination filename")?);
    ensure!(path_text(&dest)?!=before["path"].as_str().unwrap(),"Output must not replace the original PDF");
    let v=json!({"schema_version":1,"source":before,"page_count":pdf.len(),"pages":pages,
        "source_inventory_ready":false,"render_status":"not_rendered","source_fidelity":"not_assessed",
        "boundary":"Structural PDF evidence only; not a full source-inventory.json and not a rendering or extraction pass."});
    let mut bytes=serde_json::to_vec_pretty(&v)?;bytes.push(b'\n');pipeline::publish(&dest,&bytes)?;Ok(dest)
}


// Use the already-pinned PdfCraft core codecs, bounded by actual declared
// image geometry and a cumulative work budget. Unsupported display effects
// must go through the actual page renderer.
use craft_pdf::filters::{self,Filter,Params};
use image::{ImageDecoder,ImageEncoder,ExtendedColorType,codecs::{jpeg::JpegDecoder,png::PngEncoder}};
use std::io::Cursor;
fn img_value<'a>(doc:&'a Document,d:&'a Dictionary,key:&[u8])->Result<Option<&'a Object>>{
    match d.get(key){Ok(v)=>{let v=deref(doc,v)?;if matches!(v,Object::Null){Ok(None)}else{Ok(Some(v))}},Err(_)=>Ok(None)}
}
fn img_int(doc:&Document,d:&Dictionary,key:&[u8],default:Option<i64>)->Result<i64>{
    match img_value(doc,d,key)?{Some(Object::Integer(n))=>Ok(*n),Some(_)=>Err(anyhow!("Invalid integer {}",String::from_utf8_lossy(key))),
        None=>default.context("Missing image integer")}
}
fn img_chain(doc:&Document,d:&Dictionary)->Result<Vec<(Filter,Params)>>{
    let names=match img_value(doc,d,b"Filter")?{
        None=>vec![],Some(Object::Name(n))=>vec![Filter::from_name(n)],
        Some(Object::Array(a))=>a.iter().map(|v|Ok(Filter::from_name(deref(doc,v)?.as_name()?))).collect::<Result<Vec<_>>>()?,
        _=>return Err(anyhow!("Invalid filter chain")),
    };
    ensure!(names.len()<=filters::MAX_CHAIN,"Filter chain exceeds limit");
    let mut parms=Vec::new();
    match img_value(doc,d,b"DecodeParms")?{
        None=>parms.resize(names.len(),None),
        Some(Object::Dictionary(d))=>{ensure!(names.len()==1,"DecodeParms/filter mismatch");parms.push(Some(d));},
        Some(Object::Array(a))=>{
            ensure!(a.len()==names.len(),"DecodeParms/filter mismatch");
            for v in a{match deref(doc,v)?{Object::Null=>parms.push(None),Object::Dictionary(d)=>parms.push(Some(d)),_=>return Err(anyhow!("Invalid DecodeParms"))}}
        },
        _=>return Err(anyhow!("Invalid DecodeParms")),
    }
    let mut out=Vec::new();
    for (f,d) in names.into_iter().zip(parms){
        let mut p=Params::default();
        if let Some(d)=d{
            ensure!(f.is_decodable(),"PAGE_RENDER_REQUIRED: image codec DecodeParms");
            ensure!(d.iter().all(|(k,_)|matches!(k.as_slice(),b"Predictor"|b"Colors"|b"BitsPerComponent"|b"Columns"|b"EarlyChange")),
                "PAGE_RENDER_REQUIRED: unknown DecodeParms");
            p.predictor=img_int(doc,d,b"Predictor",Some(1))?;
            p.colors=img_int(doc,d,b"Colors",Some(1))?;
            p.bits_per_component=img_int(doc,d,b"BitsPerComponent",Some(8))?;
            p.columns=img_int(doc,d,b"Columns",Some(1))?;
            p.early_change=img_int(doc,d,b"EarlyChange",Some(1))?;
            ensure!(matches!(p.predictor,1|2|10..=15)&&matches!(p.colors,1|3)&&matches!(p.bits_per_component,1|2|4|8)&&
                (1..=20_000_000).contains(&p.columns)&&matches!(p.early_change,0|1),"PAGE_RENDER_REQUIRED: unsupported predictor");
        }
        out.push((f,p));
    }
    Ok(out)
}
// An active predictor describes the actual image sample rows, so only a
// final Flate or LZW stage may carry it. Decoding the codec separately lets
// us check the bytes *before* the predictor: its stream-level API permits a
// partial final row or a dangling PNG row tag, neither valid for this image.
fn decode_image_rows(
    chain: &[(Filter, Params)],
    encoded: &[u8],
    width: u32,
    height: u32,
    channels: usize,
    bits: usize,
    packed_size: usize,
) -> Result<Vec<u8>> {
    let mut work = 512u64 * 1024 * 1024;
    let h = height as usize;
    let png_max = packed_size.checked_add(h).context("Sample budget overflow")?;
    if chain.iter().all(|(_, p)| p.predictor == 1) {
        return filters::decode_within(chain, encoded, png_max, &mut work)
            .context("PAGE_RENDER_REQUIRED: cannot decode image samples");
    }

    let ((filter, p), prefix) = chain.split_last().context("Missing predictor filter")?;
    ensure!(
        matches!(filter, Filter::Flate | Filter::Lzw)
            && p.predictor != 1
            && prefix.iter().all(|(_, p)| p.predictor == 1),
        "PAGE_RENDER_REQUIRED: predictor requires a final Flate or LZW stage"
    );
    ensure!(
        (p.columns, p.colors, p.bits_per_component) == (i64::from(width), channels as i64, bits as i64),
        "PAGE_RENDER_REQUIRED: predictor layout differs from image"
    );
    let expected = if p.predictor == 2 { packed_size } else { png_max };
    let data = if prefix.is_empty() {
        std::borrow::Cow::Borrowed(encoded)
    } else {
        // A wrapper may produce a complete compressed stream, larger than
        // the predicted samples, but no larger than our encoded byte budget.
        std::borrow::Cow::Owned(
            filters::decode_within(prefix, encoded, 128 * 1024 * 1024, &mut work)
                .context("PAGE_RENDER_REQUIRED: cannot decode predictor filter prefix")?,
        )
    };
    let unpredicted = [(filter.clone(), Params { predictor: 1, ..p.clone() })];
    let predicted = filters::decode_within(&unpredicted, &data, expected, &mut work)
        .context("PAGE_RENDER_REQUIRED: cannot decode predictor rows within image byte budget")?;
    ensure!(
        predicted.len() == expected,
        "PAGE_RENDER_REQUIRED: predictor row byte count differs from image"
    );
    craft_stream_codecs::pay(&mut work, expected as u64)
        .context("PAGE_RENDER_REQUIRED: predictor work exceeds budget")?;
    let codec = if *filter == Filter::Flate { "FlateDecode" } else { "LZWDecode" };
    craft_stream_codecs::predictor::decode(p, predicted, codec, Some(h))
        .map_err(|failure| failure.error)
        .context("PAGE_RENDER_REQUIRED: invalid image predictor row")
}
struct Samples{
    w:u32,h:u32,channels:usize,bits:usize,bytes:Vec<u8>,jpeg:bool,preserve_jpeg:bool,filters:Vec<String>,
    pdf_colorspace:&'static str,indexed_palette:Option<Value>,
}
fn device_space(value:&Object)->Option<(&'static str,usize)>{
    match value.as_name().ok()?{
        b"DeviceRGB"=>Some(("/DeviceRGB",3)),b"DeviceGray"=>Some(("/DeviceGray",1)),_=>None,
    }
}
struct Palette{base:&'static str,channels:usize,hival:u8,lookup:Vec<u8>,lookup_format:&'static str,encoded_sha:Option<String>}
fn indexed_palette(doc:&Document,color:&Object)->Result<Palette>{
    let a=color.as_array().context("PAGE_RENDER_REQUIRED: ICC, CMYK or resource-defined ColorSpace")?;
    ensure!(a.len()==4&&deref(doc,&a[0])?.as_name().ok()==Some(b"Indexed"),"PAGE_RENDER_REQUIRED: unsupported ColorSpace array");
    let (base,channels)=device_space(deref(doc,&a[1])?).context("PAGE_RENDER_REQUIRED: Indexed base is not DeviceRGB/DeviceGray")?;
    let hival=u8::try_from(deref(doc,&a[2])?.as_i64().context("PAGE_RENDER_REQUIRED: Indexed hival must be an integer")?)
        .context("PAGE_RENDER_REQUIRED: Indexed hival is outside 0..255")?;
    let expected=(usize::from(hival)+1)*channels;
    let (lookup,lookup_format,encoded_sha)=match deref(doc,&a[3])?{
        Object::String(data,_)=>{
            ensure!(data.len()==expected,"PAGE_RENDER_REQUIRED: Indexed lookup length differs from hival and base");
            (data.clone(),"string",None)
        }
        Object::Stream(stream)=>{
            ensure!(stream.content.len()<=1024*1024,"PAGE_RENDER_REQUIRED: Indexed lookup encoded byte budget");
            let chain=img_chain(doc,&stream.dict)?;
            ensure!(chain.iter().all(|(f,p)|f.is_decodable()&&p.predictor==1),
                "PAGE_RENDER_REQUIRED: unsupported Indexed lookup stream filter or predictor");
            let mut work=8u64*1024*1024;
            let bytes=filters::decode_within(&chain,&stream.content,expected,&mut work)
                .context("PAGE_RENDER_REQUIRED: cannot decode Indexed lookup stream")?;
            (bytes,"stream",Some(files::digest(&stream.content)))
        }
        _=>return Err(anyhow!("PAGE_RENDER_REQUIRED: Indexed lookup must be a string or stream")),
    };
    ensure!(lookup.len()==expected,"PAGE_RENDER_REQUIRED: Indexed lookup length differs from hival and base");
    Ok(Palette{base,channels,hival,lookup,lookup_format,encoded_sha})
}
// PDF rows are independently byte-padded and samples are MSB-first.
// Keep original integer indices for a palette; device-color samples scale to PNG channels.
fn unpack_samples(raw:&[u8],w:u32,h:u32,channels:usize,bits:usize,scale:bool)->Result<Vec<u8>>{
    ensure!(matches!(bits,1|2|4|8),"Unsupported packed PDF samples");
    let samples=(w as usize).checked_mul(channels).context("Image row size overflow")?;
    let stride=samples.checked_mul(bits).context("Image row bits overflow")?.div_ceil(8);
    ensure!(raw.len()==stride.checked_mul(h as usize).context("Image size overflow")?,"Packed PDF sample size differs");
    if bits==8{return Ok(raw.to_vec());}
    let max=(1u16<<bits)-1;
    let mut result=Vec::with_capacity(samples.checked_mul(h as usize).context("Image sample count overflow")?);
    for row in raw.chunks_exact(stride){
        for n in 0..samples{
            let pos=n*bits;let shift=8-bits-pos%8;
            let value=(u16::from(row[pos/8])>>shift)&max;
            result.push(if scale{(value*255/max) as u8}else{value as u8});
        }
    }
    Ok(result)
}
fn image_samples(doc:&Document,stream:&lopdf::Stream,mask:bool)->Result<Samples>{
    let d=&stream.dict;
    let w=u32::try_from(img_int(doc,d,b"Width",None)?)?;let h=u32::try_from(img_int(doc,d,b"Height",None)?)?;
    ensure!(w>0&&h>0&&u64::from(w)*u64::from(h)<=20_000_000,"Image exceeds pixel budget");
    let colorspace=img_value(doc,d,b"ColorSpace")?.context("Missing ColorSpace")?;
    let (pdf_colorspace,channels,palette)=if let Some((base,channels))=device_space(colorspace){
        (base,channels,None)
    }else{
        ensure!(!mask,"PAGE_RENDER_REQUIRED: non-gray SMask");
        let palette=indexed_palette(doc,colorspace)?;
        ("/Indexed",palette.channels,Some(palette))
    };
    ensure!(!mask||channels==1,"PAGE_RENDER_REQUIRED: non-gray SMask");
    let sample_channels=if palette.is_some(){1}else{channels};
    let bits=usize::try_from(img_int(doc,d,b"BitsPerComponent",None)?)?;
    ensure!(matches!(bits,1|2|4|8),"PAGE_RENDER_REQUIRED: unsupported sample depth");
    for key in [b"Matte".as_slice(),b"OC",b"Alternates",b"F",b"FFilter",b"FDecodeParms"]{
        ensure!(img_value(doc,d,key)?.is_none(),"PAGE_RENDER_REQUIRED: {}",String::from_utf8_lossy(key));
    }
    if mask{ensure!(img_value(doc,d,b"Mask")?.is_none(),"PAGE_RENDER_REQUIRED: nested Mask on SMask");}
    ensure!(img_int(doc,d,b"SMaskInData",Some(0))?==0&&matches!(img_value(doc,d,b"ImageMask")?,None|Some(Object::Boolean(false))),
        "PAGE_RENDER_REQUIRED: SMaskInData or ImageMask");
    let mut invert=vec![false;channels];
    let max_sample=((1u32<<bits)-1) as f64;
    let mut index_decode=[0.,max_sample];
    if let Some(v)=img_value(doc,d,b"Decode")?{
        if palette.is_some(){
            index_decode=array::<2>(doc,v).context("PAGE_RENDER_REQUIRED: invalid Indexed Decode array")?;
        }else{
            let a=v.as_array()?;
            ensure!(a.len()==2*channels,"PAGE_RENDER_REQUIRED: invalid Decode array length");
            for (channel,pair) in a.chunks_exact(2).enumerate(){
                let low=number(deref(doc,&pair[0])?)?;let high=number(deref(doc,&pair[1])?)?;
                ensure!((low,high)==(0.,1.)||(low,high)==(1.,0.),"PAGE_RENDER_REQUIRED: non-identity/non-inverting Decode");
                invert[channel]=low==1.;
            }
        }
    }
    if mask{if let Some(v)=img_value(doc,d,b"SMask")?{ensure!(v.as_name().ok()==Some(b"None".as_slice()),"PAGE_RENDER_REQUIRED: nested SMask");}}
    ensure!(stream.content.len()<=128*1024*1024,"Encoded image exceeds byte budget");
    let chain=img_chain(doc,d)?;
    let names=chain.iter().map(|(f,_)|f.name().to_owned()).collect::<Vec<_>>();
    let size=(w as usize)*(h as usize)*sample_channels;
    let stride=(w as usize).checked_mul(sample_channels).and_then(|n|n.checked_mul(bits)).context("Image row bits overflow")?.div_ceil(8);
    let packed_size=stride.checked_mul(h as usize).context("Image byte budget overflow")?;
    let jpeg=chain.len()==1&&chain[0].0==Filter::Dct;
    let mut bytes=if jpeg{
        ensure!(palette.is_none(),"PAGE_RENDER_REQUIRED: Indexed images with DCT are not supported");
        ensure!(bits==8&&img_value(doc,d,b"DecodeParms")?.is_none(),"PAGE_RENDER_REQUIRED: JPEG depth or DecodeParms");
        let mut dec=JpegDecoder::new(Cursor::new(&stream.content))?;
        ensure!(dec.dimensions()==(w,h),"JPEG dimensions differ from PDF dictionary");
        ensure!(dec.icc_profile()?.is_none()&&dec.orientation()?==image::metadata::Orientation::NoTransforms,
            "PAGE_RENDER_REQUIRED: JPEG ICC profile or orientation");
        let expected=if channels==1{image::ColorType::L8}else{image::ColorType::Rgb8};
        ensure!(dec.color_type()==expected&&dec.total_bytes()==size as u64,"JPEG colors differ from PDF dictionary");
        let mut data=vec![0u8;size];dec.read_image(&mut data)?;data
    }else{
        ensure!(chain.iter().all(|(f,_)|f.is_decodable()),"PAGE_RENDER_REQUIRED: unsupported image codec");
        ensure!((bits==8&&palette.is_none())||chain.iter().all(|(_,p)|p.predictor!=2),
            "PAGE_RENDER_REQUIRED: packed or Indexed TIFF predictor");
        let raw=decode_image_rows(&chain,&stream.content,w,h,sample_channels,bits,packed_size)?;
        if bits==8{raw}else{unpack_samples(&raw,w,h,sample_channels,bits,palette.is_none())?}
    };
    ensure!(bytes.len()==size,"Decoded sample size differs from actual PDF image dimensions");
    if invert.iter().any(|b|*b){
        for pixel in bytes.chunks_exact_mut(channels){
            for (sample,reverse) in pixel.iter_mut().zip(&invert){if *reverse{*sample=255-*sample;}}
        }
    }
    let indexed_palette=palette.map(|palette|{
        let [low,high]=index_decode;
        // ISO 32000-2: default Decode for a palette is [0, 2^bpc - 1], not [0, hival].
        // Decode precedes nearest-integer lookup and clipping to [0, hival].
        let mut indices=[0usize;256];
        for (sample,index) in indices.iter_mut().take(max_sample as usize+1).enumerate(){
            *index=(low+(sample as f64/max_sample)*(high-low)).round().clamp(0.,f64::from(palette.hival)) as usize;
        }
        bytes=bytes.iter().flat_map(|&sample|{
            let start=indices[usize::from(sample)]*palette.channels;
            palette.lookup[start..start+palette.channels].iter().copied()
        }).collect();
        json!({"base":palette.base,"hival":palette.hival,"decode":index_decode,
            "lookup_format":palette.lookup_format,"lookup_sha256":files::digest(&palette.lookup),
            "encoded_lookup_stream_sha256":palette.encoded_sha})
    });
    let preserve_jpeg=jpeg&&!invert.iter().any(|b|*b);
    Ok(Samples{w,h,channels,bits,bytes,jpeg,preserve_jpeg,filters:names,pdf_colorspace,indexed_palette})
}
// ISO 32000-1 §8.9.6.2–3: in a one-bit image mask, Decode [0 1]
// paints bit 0, and [1 0] paints bit 1. Equal resolution and nearest sample
// semantics are required here; rendering a page remains the fallback.
fn explicit_mask_samples(pdf:&Pdf,id:ObjectId,w:u32,h:u32)->Result<(Vec<u8>,Value)>{
    let doc=&pdf.doc;
    let stream=pdf.raw_object(id.0,id.1).context("PAGE_RENDER_REQUIRED: explicit Mask stream")?;
    let d=&stream.dict;
    ensure!(img_value(doc,d,b"ImageMask")?==Some(&Object::Boolean(true)),
        "PAGE_RENDER_REQUIRED: explicit Mask must declare ImageMask true");
    ensure!(img_int(doc,d,b"Width",None)?==i64::from(w)&&img_int(doc,d,b"Height",None)?==i64::from(h),
        "PAGE_RENDER_REQUIRED: explicit Mask resolution differs from base image");
    ensure!(img_int(doc,d,b"BitsPerComponent",Some(1))?==1,"PAGE_RENDER_REQUIRED: explicit Mask is not one-bit");
    for key in [b"ColorSpace".as_slice(),b"Mask",b"SMask",b"SMaskInData",b"Matte",b"OC",b"Alternates",b"F",b"FFilter",b"FDecodeParms"]{
        ensure!(img_value(doc,d,key)?.is_none(),"PAGE_RENDER_REQUIRED: unsupported explicit Mask {}",String::from_utf8_lossy(key));
    }
    ensure!(matches!(img_value(doc,d,b"Interpolate")?,None|Some(Object::Boolean(false))),
        "PAGE_RENDER_REQUIRED: explicit Mask requests interpolation");
    let decode=match img_value(doc,d,b"Decode")?{
        None=>[0.,1.],
        Some(v)=>array::<2>(doc,v).context("PAGE_RENDER_REQUIRED: malformed explicit Mask Decode")?,
    };
    let painted=match decode{
        [0.,1.]=>0,
        [1.,0.]=>1,
        _=>return Err(anyhow!("PAGE_RENDER_REQUIRED: unsupported explicit Mask Decode")),
    };
    ensure!(stream.content.len()<=128*1024*1024,"Explicit Mask exceeds encoded byte budget");
    let chain=img_chain(doc,d)?;
    ensure!(chain.iter().all(|(f,p)|f.is_decodable()&&p.predictor!=2),
        "PAGE_RENDER_REQUIRED: unsupported explicit Mask filter or predictor");
    let packed_size=(w as usize).div_ceil(8).checked_mul(h as usize).context("Explicit Mask size overflow")?;
    let raw=decode_image_rows(&chain,&stream.content,w,h,1,1,packed_size)?;
    let mut alpha=unpack_samples(&raw,w,h,1,1,false)?;
    for value in &mut alpha{*value=if *value==painted{255}else{0};}
    let lo=*alpha.iter().min().context("Empty explicit Mask")?;
    let hi=*alpha.iter().max().context("Empty explicit Mask")?;
    let record=json!({"image_xref":id.0,"generation":id.1,"status":if lo==255{"opaque"}else{"nonopaque"},
        "width":w,"height":h,"bits_per_component":1,"decode":decode,"painted_sample":painted,
        "alpha_min":lo,"alpha_max":hi,"pdf_stream_sha256":files::digest(&stream.content),
        "decoded_alpha_sha256":files::digest(&alpha),
        "filters":chain.iter().map(|(f,_)|f.name()).collect::<Vec<_>>()});
    Ok((alpha,record))
}
fn image_png(s:&Samples,alpha:Option<&[u8]>)->Result<Vec<u8>>{
    let mut out=Vec::new();
    if let Some(alpha)=alpha{
        ensure!(alpha.len()==(s.w as usize)*(s.h as usize),"Image alpha dimensions differ");
        let mut pixels=Vec::with_capacity(alpha.len()*(s.channels+1));
        for (p,a) in s.bytes.chunks_exact(s.channels).zip(alpha){pixels.extend_from_slice(p);pixels.push(*a);}
        let ty=if s.channels==1{ExtendedColorType::La8}else{ExtendedColorType::Rgba8};
        PngEncoder::new(&mut out).write_image(&pixels,s.w,s.h,ty)?;
    }else{
        let ty=if s.channels==1{ExtendedColorType::L8}else{ExtendedColorType::Rgb8};
        PngEncoder::new(&mut out).write_image(&s.bytes,s.w,s.h,ty)?;
    }
    Ok(out)
}
/// Actual occurrence/XRef validation, strict bounded sample decoding, original
/// JPEG preservation and non-premultiplied alpha composition.
pub fn export_image_object(source:&Path,page:u64,xref:u32,dest:&Path,expected:Option<&str>)->Result<Value>{
    ensure!(xref>0,"Invalid image_xref");
    let before=check_ledger::bind_file(source)?;
    if let Some(hash)=expected{ensure!(before["sha256"]==hash,"Source PDF hash differs from selected inventory");}
    let pdf=Pdf::open(source)?;let index=pdf.index(page)?;
    export_indexed(&pdf,source,&before,page,xref,dest,&index)
}
pub(crate) fn export_indexed(pdf:&Pdf,source:&Path,before:&Value,page:u64,xref:u32,dest:&Path,index:&Value)->Result<Value>{
    let doc=&pdf.doc;
    let issues=index["issues"].as_array().context("Missing index issues")?;
    ensure!(index["status"]=="structurally_indexed"||(!issues.is_empty()&&issues.iter().all(|r|r["code"]=="FORM_UNRESOLVED")),
        "PAGE_RENDER_REQUIRED: unresolved page image calls");
    let locations:Vec<_>=index["occurrences"].as_array().context("Missing occurrences")?.iter()
        .filter(|r|r["image_xref"].as_u64()==Some(u64::from(xref))).cloned().collect();
    ensure!(!locations.is_empty(),"image_xref is not actually invoked on the selected PDF page");
    ensure!(locations.iter().all(|r|r["context_effects"].as_array().is_some_and(|a|a.is_empty())),"PAGE_RENDER_REQUIRED: clipping or graphics state");
    let generation=locations[0]["generation"].as_u64().context("Missing generation")?;
    ensure!(locations.iter().all(|r|r["generation"].as_u64()==Some(generation)),"Ambiguous image generation");
    let stream=pdf.raw_object(xref,u16::try_from(generation)?)?;
    ensure!(img_value(doc,doc.catalog()?,b"OCProperties")?.is_none(),"PAGE_RENDER_REQUIRED: optional-content configuration");
    let s=image_samples(doc,stream,false)?;let geo=pdf.geometry(page)?;
    for l in &locations{
        let a=l["transform"].as_array().filter(|a|a.len()==6).context("Missing transform")?;
        let mut m=[0.;6];for (i,x) in a.iter().enumerate(){m[i]=x.as_f64().filter(|n|n.is_finite()).context("Invalid transform")?;}
        let m=mul(m,geo.rotated);
        ensure!(m[0]>0.&&m[3]>0.&&m[1].abs()<=0.0001&&m[2].abs()<=0.0001,"PAGE_RENDER_REQUIRED: rotated, mirrored or skewed appearance");
        let (sx,sy)=(m[0]/f64::from(s.w),m[3]/f64::from(s.h));
        ensure!((sx-sy).abs()<=0.0001*sx.abs().max(sy.abs()),"PAGE_RENDER_REQUIRED: nonuniform image scale");
    }
    let mut alpha=None;let mut mask=json!({"image_xref":0,"status":"absent"});
    let mut explicit=None;
    let soft=img_value(doc,&stream.dict,b"SMask")?.filter(|v|v.as_name().ok()!=Some(b"None".as_slice()));
    if img_value(doc,&stream.dict,b"Mask")?.is_some(){
        ensure!(soft.is_none(),"PAGE_RENDER_REQUIRED: combined explicit Mask and SMask");
        ensure!(matches!(img_value(doc,&stream.dict,b"Interpolate")?,None|Some(Object::Boolean(false))),
            "PAGE_RENDER_REQUIRED: explicit Mask base requests interpolation");
        let id=match stream.dict.get(b"Mask")?{
            Object::Reference(id)=>*id,
            _=>return Err(anyhow!("PAGE_RENDER_REQUIRED: Mask must be an indirect same-resolution one-bit image; color-key masks are not supported")),
        };
        ensure!(id!=(xref,u16::try_from(generation)?),"Recursive explicit Mask");
        let (hard_alpha,record)=explicit_mask_samples(pdf,id,s.w,s.h)?;
        alpha=Some(hard_alpha);explicit=Some(record);
    }
    if soft.is_some(){
        let id=match stream.dict.get(b"SMask")?{Object::Reference(id)=>*id,_=>return Err(anyhow!("PAGE_RENDER_REQUIRED: SMask must be indirect"))};
        ensure!(id!=(xref,u16::try_from(generation)?),"Recursive SMask");
        let ms=pdf.raw_object(id.0,id.1)?;let a=image_samples(doc,ms,true)?;
        ensure!((a.w,a.h)==(s.w,s.h),"PAGE_RENDER_REQUIRED: differing SMask dimensions");
        let lo=*a.bytes.iter().min().context("Empty SMask")?;let hi=*a.bytes.iter().max().context("Empty SMask")?;
        mask=json!({"image_xref":id.0,"generation":id.1,"status":if lo==255{"opaque"}else{"nonopaque"},
            "width":a.w,"height":a.h,"alpha_min":lo,"alpha_max":hi,"pdf_stream_sha256":files::digest(&ms.content),
            "decoded_alpha_sha256":files::digest(&a.bytes),"filters":a.filters});
        if lo!=255{alpha=Some(a.bytes);}
    }
    let png=if s.preserve_jpeg&&alpha.is_none(){None}else{Some(image_png(&s,alpha.as_deref())?)};
    let (data,ext,method)=match png.as_ref(){
        Some(b)=>(b.as_slice(),"png",if explicit.is_some(){"explicit_mask_composited_png"}else if alpha.is_some(){"smask_composited_png"}else{"lossless_sample_png"}),
        None=>(stream.content.as_slice(),"jpeg","original_extracted_bytes"),
    };
    let target=match dest.extension(){
        None=>dest.with_extension(ext),
        Some(x)=>{let x=x.to_str().context("Invalid extension")?.to_ascii_lowercase();ensure!(x==ext||(ext=="jpeg"&&x=="jpg"),"Destination extension must match actual encoding");dest.to_owned()}
    };
    let parent=target.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or_else(||Path::new("."));
    fs::create_dir_all(parent)?;
    let target=files::canonical(parent)?.join(target.file_name().context("Missing output filename")?);
    ensure!(path_text(&target)?!=before["path"],"Cannot replace source PDF");
    ensure!(check_ledger::bind_file(source)?==*before,"Source PDF changed during extraction");
    pipeline::publish(&target,data)?;
    let asset=check_ledger::bind_file(&target)?;
    ensure!(asset["sha256"]==files::digest(data)&&check_ledger::bind_file(source)?==*before,"Source or output changed");
    let mut record=json!({"schema_version":1,"source_file":before,"page":page,"image_xref":xref,"source_locations":locations,
        "source_index_status":index["status"],"source_index_issues":issues,"page_rotation":geo.rotation,
        "page_to_rotated_page_transform":geo.rotated,"coordinate_system":index["coordinate_system"],
        "source_object":{"image_xref":xref,"generation":generation,"width":s.w,"height":s.h,"bits_per_component":s.bits,
            "pdf_colorspace":s.pdf_colorspace,"filters":s.filters,
            "original_encoded_extension":if s.jpeg{Some("jpeg")}else{None},"pdf_stream_sha256":files::digest(&stream.content),
            "decoded_samples_sha256":files::digest(&s.bytes),"smask":mask,"figure_completeness":"not_assessed"},
        "asset":asset,"output_extension":ext,"method":method,"orientation":"native_image_samples",
        "visual_review":{"status":"pending","figure_completeness":"not_assessed"},"source_fidelity":"not_assessed",
        "evidence_boundary":"Actual invoked image samples, not proof of visible ink or complete figure membership. Overlay, text, vector and unsupported display effects require page rendering and independent review."});
    if let Some(palette)=s.indexed_palette{record["source_object"]["indexed_palette"]=palette;}
    if let Some(mask)=explicit{record["source_object"]["explicit_mask"]=mask;}
    Ok(record)
}


/// Original geometry helper, exposed through actual file and one-based page
/// instead of passing a Python-specific Fitz page object across the ABI.
pub fn pdf_page_rect(source:&Path,page:u64)->Result<Value>{
    let before=check_ledger::bind_file(source)?;
    let pdf=Pdf::open(source)?;
    let rect=pdf.bounds(page)?;
    ensure!(check_ledger::bind_file(source)?==before,"Source PDF changed during page geometry extraction");
    Ok(json!(rect))
}
fn finite_rect(v:&Value)->Option<[f64;4]>{
    let a=v.as_array().filter(|a|a.len()==4)?;
    let mut b=[0.;4];
    for(i,v)in a.iter().enumerate(){b[i]=v.as_f64().filter(|x|x.is_finite())?;}
    (b[0]<b[2]&&b[1]<b[3]).then_some(b)
}
pub fn valid_pdf_bbox(bbox:&Value,page_rect:&Value)->bool{
    finite_rect(bbox).zip(finite_rect(page_rect)).is_some_and(|(b,p)|inside(p,b))
}
#[cfg(test)]
mod geometry_helpers_tests{
    use super::*;
    #[test]
    fn reject_invalid_and_out_of_page_boxes(){
        let page=json!([0,0,100,200]);
        assert!(valid_pdf_bbox(&json!([0,0,100,200]),&page));
        for b in [json!([0,0,101,200]),json!([1,0,0,200]),json!([0,0,0,200]),json!([false,0,99,200]),json!([0,0,null,200]),json!([0,0,1])]{assert!(!valid_pdf_bbox(&b,&page));}
        assert!(!valid_pdf_bbox(&page,&json!([0,0,0,0])));
    }
}


/// Retain the actual invoked object and optional mask streams in a source-
/// scoped immutable object store. Native standalone images are supplementary
/// candidates, not replacements for the raw streams or rendered page.
fn object_store_dir(p:&Path)->Result<PathBuf>{
    match fs::symlink_metadata(p){
        Ok(m)=>{
            ensure!(m.is_dir()&&!m.file_type().is_symlink(),"Refusing linked/non-directory image object store");
            #[cfg(windows)]{
                use std::os::windows::fs::MetadataExt;
                ensure!(m.file_attributes()&0x400==0,"Refusing reparse-point image object store");
            }
        },
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>fs::create_dir_all(p)?,
        Err(e)=>return Err(e.into())
    }
    files::canonical(p)
}
fn store_stream(pdf:&Pdf,id:(u32,u16),root:&Path,source:&Value)->Result<Value>{
    let s=pdf.raw_object(id.0,id.1)?;
    let hash=files::digest(&s.content);
    let mut row=json!({"image_xref":id.0,"generation":id.1,"encoded_stream_sha256":hash,
        "encoded_stream_bytes":s.content.len(),"source_fidelity":"not_assessed"});
    if s.content.len()>128*1024*1024{row["status"]=json!("blocked_budget");return Ok(row);}
    let p=root.join(format!("xref-{:06}-g{}.bin",id.0,id.1));
    ensure!(path_text(&p)?!=source["path"],"Image evidence must not replace source PDF");
    if p.exists(){crate::check_write::no_link(&p)?;}
    pipeline::publish(&p,&s.content)?;
    crate::check_write::no_link(&p)?;
    let actual=check_ledger::bind_file(&p)?;
    ensure!(actual["sha256"]==hash,"Stored stream differs from actual object");
    row["status"]=json!("raw_stream_stored");
    row["asset"]=actual;Ok(row)
}
/// Original ImageObjectIndexer.index_page ABI: explicit PDF path and page,
/// optional object_directory replaces the Python context manager. Preserve
/// occurrence identities even for unsupported filters/display effects, and
/// bind all emitted bytes. Repeated pages/objects reuse identical streams.
pub fn index_page_with_objects(source:&Path,page:u64,object_directory:Option<&Path>,expected_sha:Option<&str>,
    mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    let before=check_ledger::bind_file(source)?;
    if let Some(expected)=expected_sha{ensure!(before["sha256"]==expected,"Original PDF hash differs from expected binding");}
    let pdf=Pdf::open(source)?;
    let mut result=pdf.index(page)?;
    result["source_file"]=before.clone();result["page"]=json!(page);
    let Some(directory)=object_directory else{
        ensure!(check_ledger::bind_file(source)?==before,"Source PDF changed while indexing");
        return Ok(result);
    };
    let root=object_store_dir(directory)?;
    let root=object_store_dir(&root.join(format!("source-{}",before["sha256"].as_str().context("Source digest")?)))?;
    let _lock=crate::check_write::lock_target(&root.join("object-store"))?;
    let objects=result["objects"].as_array().context("Actual indexed image objects required")?.clone();
    let mut streams=BTreeMap::<(u32,u16),Value>::new();
    let mut stored=Vec::with_capacity(objects.len());
    for(i,mut row)in objects.into_iter().enumerate(){
        let xref=u32::try_from(row["image_xref"].as_u64().context("Actual image XRef")?)?;
        let generation=u16::try_from(row["generation"].as_u64().context("Actual generation")?)?;
        let id=(xref,generation);
        if !streams.contains_key(&id){streams.insert(id,store_stream(&pdf,id,&root,&before)?);}
        let raw=&streams[&id];
        ensure!(raw["encoded_stream_sha256"]==row["encoded_stream_sha256"],"Actual encoded object differs from index");
        row["encoded_stream"]=raw.clone();
        // Soft and explicit masks are independent object dependencies;
        // storing their encoded bytes does not assert decoded alpha fidelity.
        for key in ["smask","mask"]{
            let locator=row[key].clone();
            if let Some(x)=locator["xref"].as_u64(){
                let mid=(u32::try_from(x)?,u16::try_from(locator["generation"].as_u64().context("Mask generation")?)?);
                ensure!(mid!=id,"Recursive image mask");
                if !streams.contains_key(&mid){
                    match store_stream(&pdf,mid,&root,&before){
                        Ok(v)=>{streams.insert(mid,v);},
                        Err(e)=>{streams.insert(mid,json!({"image_xref":mid.0,"generation":mid.1,"status":"unresolved","reason":format!("{e:#}")}));},
                    }
                }
                row[format!("{key}_stream")]=streams[&mid].clone();
            }else if locator["direct_or_other"]==true{
                row[format!("{key}_stream")]=json!({"status":"unresolved_direct_or_other","review_required":true});
            }
        }
        let name=root.join(format!("xref-{xref:06}-g{generation}"));
        match export_indexed(&pdf,source,&before,page,xref,&name,&result){
            Ok(v)=>{row["raw_asset"]=v["asset"].clone();row["extraction"]=v;},
            Err(e)=>{row["extraction"]=json!({"status":"standalone_extraction_unavailable","reason":format!("{e:#}"),"review_required":true});},
        }
        stored.push(row);progress(i+1,result["objects"].as_array().unwrap().len(),"actual image objects and masks");
    }
    // Revalidate all retained files at the point of returning the index.
    for r in streams.values(){if let Some(b)=r.get("asset"){ensure!(check_ledger::bind_file(&crate::path(&b["path"])?)?==*b,"Image stream changed during indexing");}}
    for r in &stored{if let Some(b)=r.get("raw_asset"){ensure!(check_ledger::bind_file(&crate::path(&b["path"])?)?==*b,"Standalone object changed during indexing");}}
    ensure!(check_ledger::bind_file(source)?==before,"Source PDF changed during image object extraction");
    result["objects"]=json!(stored);result["object_directory"]=json!(path_text(&root)?);
    result["object_streams"]=json!(streams.into_values().collect::<Vec<_>>());
    result["source_fidelity"]=json!("not_assessed");
    result["image_evidence_ready"]=json!(false);
    Ok(result)
}
