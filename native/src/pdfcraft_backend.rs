//! Headless PdfCraft rendering and text extraction. The existing PDF object
//! index supplies independent XRef provenance, never substitutes for pixels.
//! All backends read the same immutable source bytes. No external executable.
use anyhow::{anyhow,ensure,Context,Result};
use image::{codecs::png::PngEncoder,ExtendedColorType,ImageEncoder};
use pdfcraft_render::{PageRenderer,PageInfo,RenderConfig,RenderRequest,RequestKind,Tile,device_pixels};
use serde_json::{json,Value};
use std::{fs,io::{Read,Write},path::{Path,PathBuf},sync::Arc};
use crate::{check_ledger,files,path,path_text,pdf_structure::Pdf,pipeline};

pub const PDFCRAFT_REV:&str="6e910675c68dd5092e889065cfe8d0274c1b2742";
const MAX_INPUT:u64=512*1024*1024;
const MAX_TILE:u32=4096;
const MAX_GLYPHS:usize=1_000_000;
const BOUNDARY:&str="PdfCraft native rendering and extracted glyph evidence. Raster/text parity, missing fonts, image decoding, reading order and full figure completeness require independent qualification; no visual or semantic acceptance is inferred.";
fn coordinates()->Value{
    json!({"space":"pdf_page","unit":"pt","origin":"top_left","x_axis":"right","y_axis":"down","rotation":"unrotated"})
}
fn bbox(v:&Value)->Result<[f64;4]>{
    let a=v.as_array().filter(|a|a.len()==4).context("bbox must contain four finite PDF-point numbers")?;
    let mut b=[0.;4];
    for (i,x) in a.iter().enumerate(){b[i]=x.as_f64().filter(|x|x.is_finite()).context("bbox must contain four finite PDF-point numbers")?;}
    ensure!(b[0]<b[2]&&b[1]<b[3],"bbox must be a nonempty rectangle");Ok(b)
}
fn transform(v:&Value)->Result<[f64;6]>{
    let a=v.as_array().filter(|a|a.len()==6).context("Invalid page transform")?;
    let mut m=[0.;6];for (i,x) in a.iter().enumerate(){m[i]=x.as_f64().filter(|x|x.is_finite()).context("Invalid page transform")?;}Ok(m)
}
fn point(m:[f64;6],p:[f64;2])->[f64;2]{
    [m[0]*p[0]+m[2]*p[1]+m[4],m[1]*p[0]+m[3]*p[1]+m[5]]
}
fn inverse(m:[f64;6])->Result<[f64;6]>{
    let d=m[0]*m[3]-m[1]*m[2];ensure!(d.is_finite()&&d!=0.,"Invalid page transform");
    Ok([m[3]/d,-m[1]/d,-m[2]/d,m[0]/d,(m[2]*m[5]-m[3]*m[4])/d,(m[1]*m[4]-m[0]*m[5])/d])
}
fn enclose(points:&[[f64;2]])->Result<[f64;4]>{
    ensure!(!points.is_empty(),"Empty coordinate set");
    let mut b=[f64::INFINITY,f64::INFINITY,f64::NEG_INFINITY,f64::NEG_INFINITY];
    for p in points{
        ensure!(p.iter().all(|x|x.is_finite()),"Nonfinite glyph/region coordinates");
        b[0]=b[0].min(p[0]);b[1]=b[1].min(p[1]);b[2]=b[2].max(p[0]);b[3]=b[3].max(p[1]);
    }Ok(b)
}
fn transform_box(m:[f64;6],b:[f64;4])->Result<[f64;4]>{
    enclose(&[[b[0],b[1]],[b[2],b[1]],[b[2],b[3]],[b[0],b[3]]].map(|p|point(m,p)))
}
fn png_path(p:&Path)->Result<PathBuf>{
    let p=match p.extension(){
        None=>p.with_extension("png"),
        Some(s)=>{ensure!(s.to_str().is_some_and(|s|s.eq_ignore_ascii_case("png")),"Destination extension must match PNG encoding");p.to_owned()}
    };
    Ok(p)
}
fn output(p:&Path)->Result<PathBuf>{
    let parent=p.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or_else(||Path::new("."));
    fs::create_dir_all(parent)?;
    Ok(files::canonical(parent)?.join(p.file_name().context("output filename")?))
}
fn new_file(p:&Path,bytes:&[u8])->Result<Value>{
    ensure!(!p.exists()&&!p.is_symlink(),"Keep the existing evidence file: {}",p.display());
    let mut t=tempfile::NamedTempFile::new_in(p.parent().context("output directory")?)?;
    t.write_all(bytes)?;t.as_file().sync_all()?;
    t.persist_noclobber(p).map_err(|e|anyhow!("Cannot exclusively publish {}: {}",p.display(),e.error))?;
    let binding=check_ledger::bind_file(p)?;
    ensure!(binding["sha256"]==files::digest(bytes),"Evidence bytes changed after publication");
    Ok(binding)
}
fn json_bytes(v:&Value)->Result<Vec<u8>>{let mut b=serde_json::to_vec_pretty(v)?;b.push(b'\n');Ok(b)}

pub struct Backend{
    source_path:PathBuf,
    source:Value,
    structural:Pdf,
    renderer:PageRenderer,
    pages:Vec<PageInfo>,
    inspection_warnings:Vec<String>,
}
impl Backend{
    pub fn open(p:&Path,expected:Option<&str>)->Result<Self>{
        let p=files::canonical(p)?;
        let before=check_ledger::bind_file(&p)?;
        if let Some(expected)=expected{ensure!(before["sha256"]==expected,"The source PDF hash differs from the selected inventory");}
        let meta=fs::metadata(&p)?;
        ensure!(meta.is_file()&&meta.len()<=MAX_INPUT,"PDF exceeds native evidence input budget");
        let mut data=Vec::with_capacity(meta.len() as usize);
        fs::File::open(&p)?.take(MAX_INPUT+1).read_to_end(&mut data)?;
        ensure!(data.len() as u64<=MAX_INPUT&&files::digest(&data)==before["sha256"],"Source PDF changed during loading");
        let structural=Pdf::from_bytes(&data)?;
        let bytes=Arc::new(data);
        // PdfCraft's read-only inspector gives the renderer's own rotated page
        // dimensions. Cross-check them against the independent structural parser.
        let info=pdfcraft_render::inspect(bytes.clone(),None)?;
        ensure!(!info.encrypted,"An accessible unencrypted PDF is required");
        let renderer=PageRenderer::new(bytes,RenderConfig{reject_oversize:true,system_fonts:false,..Default::default()});
        ensure!(renderer.page_count()==structural.len()&&info.pages.len()==structural.len(),
            "PdfCraft and the structural parser disagree about actual page count");
        let engine=Self{source_path:p,source:before,structural,renderer,pages:info.pages,inspection_warnings:info.warnings};
        engine.verify()?;Ok(engine)
    }
    pub fn len(&self)->usize{self.pages.len()}
    pub(crate) fn source_binding(&self)->&Value{&self.source}
    pub(crate) fn structure(&self,page:u64)->Result<Value>{
        let mut result=self.structural.index(page)?;
        self.verify()?;
        result["source_file"]=self.source.clone();
        result["page"]=json!(page);Ok(result)
    }

    pub(crate) fn verify(&self)->Result<()>{
        ensure!(check_ledger::bind_file(&self.source_path)?==self.source,"Source PDF changed during evidence preparation");
        Ok(())
    }
    fn geometry(&self,page:u64)->Result<(usize,Value,[f64;4],[f64;6])>{
        let n=usize::try_from(page).ok().filter(|n|*n>=1&&*n<=self.len())
            .context("page_number must name an actual PDF page, starting at 1")?;
        let geometry=self.structural.geometry_record(page)?;
        let b=bbox(&geometry["page_bounds"])?;
        let m=transform(&geometry["page_to_rotated_page_transform"])?;
        let display=transform_box(m,b)?;
        let info=&self.pages[n-1];
        let near=|a:f64,b:f64|a.is_finite()&&b.is_finite()&&(a-b).abs()<=0.002+1e-6*a.abs().max(b.abs());
        ensure!(geometry["page_rotation"].as_u64()==Some(u64::from(info.rotation))&&
            near(display[2]-display[0],f64::from(info.width))&&near(display[3]-display[1],f64::from(info.height)),
            "PdfCraft and structural PDF geometry differ on page {page}; no coordinate evidence produced");
        Ok((n-1,geometry,b,m))
    }
    pub fn text(&mut self,page:u64)->Result<Value>{
        let (index,geometry,_,m)=self.geometry(page)?;
        let inv=inverse(m)?;
        let result=self.renderer.render(RenderRequest{page:index,kind:RequestKind::Text,..Default::default()});
        if let Some(e)=result.error{anyhow::bail!("PdfCraft text extraction: {e}");}
        let text=result.text.context("PdfCraft returned no text-layer result")?;
        ensure!(text.glyphs.len()<=MAX_GLYPHS&&text.glyphs.len()==text.line_of.len()&&text.glyphs.len()==text.space_before.len(),
            "PdfCraft glyph budget or line structure mismatch");
        let mut rows=Vec::with_capacity(text.glyphs.len());
        for (i,g) in text.glyphs.iter().enumerate(){
            let displayed=g.quad.map(|p|p.map(f64::from));
            let quad=displayed.map(|p|point(inv,p));
            let b=enclose(&quad)?;
            let origin=point(inv,[0.,0.]);let end=point(inv,g.direction.map(f64::from));
            let direction=[end[0]-origin[0],end[1]-origin[1]];
            ensure!(direction.iter().all(|v|v.is_finite()),"Invalid glyph direction");
            rows.push(json!({"glyph_index":i,"text":g.text,"bbox":b,"quad":quad,
                "display_bbox":g.rect,"display_quad":displayed,"direction":direction,
                "line_index":text.line_of[i],"space_before":text.space_before[i]}));
        }
        let warnings:Vec<_>=result.warnings.iter().map(|w|format!("{w:?}")).collect();
        self.verify()?;
        Ok(json!({"schema_version":1,"backend":"pdfcraft-render","backend_revision":PDFCRAFT_REV,
            "source_file":self.source,"page":page,"page_geometry":geometry,"coordinate_system":coordinates(),
            "plain_text":text.plain_text(),"glyphs":rows,"truncated":text.truncated,
            "warnings":warnings,"inspection_warnings":self.inspection_warnings,
            "source_fidelity":"not_assessed","evidence_boundary":BOUNDARY}))
    }
    fn raster(&mut self,page:u64,region:Option<[f64;4]>,dpi:u64)->Result<(Vec<u8>,Value)>{
        ensure!((72..=1200).contains(&dpi),"dpi must be an integer between 72 and 1200");
        let (index,geometry,bounds,m)=self.geometry(page)?;
        let scale=dpi as f32/72.;
        let w=device_pixels(self.pages[index].width,scale);let h=device_pixels(self.pages[index].height,scale);
        let (clip,tile,px)=if let Some(b)=region{
            ensure!(bounds[0]<=b[0]&&bounds[1]<=b[1]&&b[2]<=bounds[2]&&b[3]<=bounds[3],"bbox must be fully inside the unrotated PDF page");
            let display=transform_box(m,b)?;
            let factor=f64::from(scale);
            let coordinate=|x:f64,max:u32,upper:bool|->u32{
                let y=x*factor;let y=if upper{y.ceil()}else{y.floor()};
                y.clamp(0.,f64::from(max)) as u32
            };
            let x0=coordinate(display[0],w,false);let y0=coordinate(display[1],h,false);
            let x1=coordinate(display[2],w,true);let y1=coordinate(display[3],h,true);
            ensure!(x1>x0&&y1>y0,"Crop has no raster pixels");
            ensure!(x1-x0<=MAX_TILE&&y1-y0<=MAX_TILE,"PdfCraft tile exceeds 4096 pixels on one side; lower DPI or select a smaller region");
            (b,Some(Tile{x:x0,y:y0,w:x1-x0,h:y1-y0}),[x0,y0,x1,y1])
        }else{
            ensure!(w as f32<=pdfcraft_render::MAX_SIDE&&h as f32<=pdfcraft_render::MAX_SIDE&&
                u64::from(w)*u64::from(h)<=pdfcraft_render::MAX_PIXELS as u64,
                "Requested whole-page raster exceeds PdfCraft limits; no silent downscaling");
            (bounds,None,[0,0,w,h])
        };
        let result=self.renderer.render(RenderRequest{page:index,kind:RequestKind::Pixels,tile,scale,tag:0,preview:false});
        if let Some(e)=result.error{anyhow::bail!("PdfCraft raster: {e}");}
        ensure!(result.width==px[2]-px[0]&&result.height==px[3]-px[1],
            "PdfCraft returned unexpected raster dimensions; no silent downscaling or tile clipping");
        let rgba=result.rgba.as_ref();
        let length=(result.width as usize).checked_mul(result.height as usize).and_then(|n|n.checked_mul(4)).context("Raster size overflow")?;
        ensure!(rgba.len()==length&&rgba.chunks_exact(4).all(|p|p[3]==255),"PdfCraft white-background raster is not opaque RGBA8");
        let mut bytes=Vec::new();
        PngEncoder::new(&mut bytes).write_image(rgba,result.width,result.height,ExtendedColorType::Rgba8)?;
        let warnings:Vec<_>=result.warnings.iter().map(|w|format!("{w:?}")).collect();
        let f=f64::from(scale);
        let pixel_matrix=[m[0]*f,m[1]*f,m[2]*f,m[3]*f,m[4]*f-f64::from(px[0]),m[5]*f-f64::from(px[1])];
        let value=json!({"schema_version":1,"backend":"pdfcraft-render","backend_revision":PDFCRAFT_REV,
            "source_file":self.source,"page":page,"page_geometry":geometry,"coordinate_system":coordinates(),
            "requested_bbox":clip,"dpi":dpi,"width":result.width,"height":result.height,
            "pixel_bounds_in_rotated_page":px,"unrotated_page_to_pixel_transform":pixel_matrix,
            "orientation":"source_page_display","method":if tile.is_some(){"pdfcraft_tile_render"}else{"pdfcraft_page_render"},
            "warnings":warnings,"inspection_warnings":self.inspection_warnings,
            "visual_review":{"status":"pending","figure_completeness":"not_assessed"},"source_fidelity":"not_assessed",
            "evidence_boundary":BOUNDARY});
        Ok((bytes,value))
    }
    pub fn render(&mut self,page:u64,region:Option<[f64;4]>,dpi:u64,destination:&Path)->Result<Value>{
        let (bytes,mut result)=self.raster(page,region,dpi)?;
        let dest=output(&png_path(destination)?)?;
        ensure!(dest!=self.source_path,"Cannot replace the source PDF");
        self.verify()?;
        // Reuse exactly matching output bytes; do not follow or replace links.
        pipeline::publish(&dest,&bytes)?;
        let asset=check_ledger::bind_file(&dest)?;
        ensure!(asset["sha256"]==files::digest(&bytes),"Rendered evidence changed after publication");
        self.verify()?;result["asset"]=asset;Ok(result)
    }
}
pub fn extract_text(p:&Path,page:u64,expected:Option<&str>)->Result<Value>{Backend::open(p,expected)?.text(page)}
pub fn render_page(p:&Path,page:u64,dpi:u64,dest:&Path,expected:Option<&str>)->Result<Value>{
    Backend::open(p,expected)?.render(page,None,dpi,dest)
}
pub fn render_region(p:&Path,page:u64,region:&Value,coordinate_system:&Value,dpi:u64,dest:&Path,expected:Option<&str>)->Result<Value>{
    ensure!(*coordinate_system==coordinates(),"Declare unrotated PDF-point coordinates explicitly");
    Backend::open(p,expected)?.render(page,Some(bbox(region)?),dpi,dest)
}
/// Produce actual page images, text and per-glyph locations; retain structural
/// image-call evidence without promoting it into legacy PyMuPDF block IDs.
pub fn prepare(p:&Path,root:&Path,dpi:u64,expected:Option<&str>,mut progress:impl FnMut(usize,usize,&str))->Result<PathBuf>{
    ensure!((72..=600).contains(&dpi),"inventory dpi must be between 72 and 600");
    let mut backend=Backend::open(p,expected)?;
    let root=output(root)?;
    fs::create_dir(&root).context("Use a fresh PDF evidence directory; keep existing evidence")?;
    let mut pages=Vec::with_capacity(backend.len());
    for i in 1..=backend.len(){
        let page=i as u64;
        let render=backend.render(page,None,dpi,&root.join(format!("page-{i:06}.png")))?;
        let text=backend.text(page)?;
        let raw=text["plain_text"].as_str().context("actual page text")?;
        let text_file=new_file(&root.join(format!("page-{i:06}.txt")),raw.as_bytes())?;
        let glyph_file=new_file(&root.join(format!("page-{i:06}-glyphs.json")),&json_bytes(&text)?)?;
        let index=backend.structural.index(page)?;
        let index_file=new_file(&root.join(format!("page-{i:06}-objects.json")),&json_bytes(&index)?)?;
        pages.push(json!({"page":page,"render":render,"text":text_file,"glyphs":glyph_file,
            "image_structure":index_file,"text_truncated":text["truncated"],"text_warnings":text["warnings"],
            "image_structure_status":index["status"]}));
        progress(i,backend.len(),"PdfCraft page rendering, text and XRef evidence");
    }
    backend.verify()?;
    let result=json!({"schema_version":1,"backend":"pdfcraft-render","backend_revision":PDFCRAFT_REV,
        "source_file":backend.source,"page_count":backend.len(),"dpi":dpi,"pages":pages,
        "source_inventory_ready":false,"source_fidelity":"not_assessed","evidence_boundary":BOUNDARY,
        "boundary":"Actual PdfCraft artifacts; legacy source-unit/block contract, figure boundaries and complete source validation are not yet qualified."});
    let dest=root.join("pdfcraft-evidence.json");
    new_file(&dest,&json_bytes(&result)?)?;Ok(dest)
}
