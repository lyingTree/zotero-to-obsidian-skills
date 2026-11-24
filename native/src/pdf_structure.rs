//! Structural PDF evidence: page tree, actual indirect image calls, original
//! stream bytes and clipping/graphics-state flags. Does not claim visibility,
//! raster extraction, pixel reconstruction, or rendered-page parity.
use crate::{files,path_text,check_ledger,pipeline};
use anyhow::{anyhow,ensure,Context,Result};
use lopdf::{Document,Object,ObjectId,Dictionary,content::Content};
use serde_json::{json,Value};
use std::{collections::{BTreeMap,HashSet},fs,path::{Path,PathBuf}};

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
pub struct Pdf{doc:Document,pages:BTreeMap<u32,ObjectId>}
impl Pdf{
    pub fn open(p:&Path)->Result<Self>{let doc=load(p)?;let pages=doc.get_pages();Ok(Self{doc,pages})}
    /// Use the very same immutable byte snapshot as the raster/text backend.
    pub(crate) fn from_bytes(bytes:&[u8])->Result<Self>{
        ensure!(bytes.len() as u64<=MAX_PDF,"PDF exceeds structural evidence read budget");
        let doc=Document::load_mem(bytes)?;
        ensure!(!doc.is_encrypted(),"An accessible unencrypted PDF is required");
        let pages=doc.get_pages();
        ensure!(!pages.is_empty(),"PDF page tree is empty");
        Ok(Self{doc,pages})
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
    pub fn contains_bbox(&self,n:u64,bbox:&Value)->Result<bool>{
        let bounds=self.bounds(n)?;
        let Some(a)=bbox.as_array().filter(|a|a.len()==4) else{return Ok(false);};
        let mut b=[0.;4];
        for (i,v) in a.iter().enumerate(){
            let Some(x)=v.as_f64().filter(|x|x.is_finite()) else{return Ok(false);};
            b[i]=x;
        }
        Ok(b[0]<b[2]&&b[1]<b[3]&&inside(bounds,b))
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
