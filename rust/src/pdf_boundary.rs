//! Real glyph/image geometry and contextual raster evidence. Never claim
//! geometric em boxes are painted glyphs or that a proposed crop is complete.
use anyhow::{ensure,Context,Result};
use serde_json::{json,Value};
use std::{collections::BTreeMap,fs,path::{Path,PathBuf}};
use image::{ImageEncoder,ExtendedColorType,codecs::png::PngEncoder};
use crate::{pdfcraft_backend::Backend,files,check_ledger,path,pipeline};
const EDGES:[&str;4]=["top","right","bottom","left"];
type Rect=[f64;4];
fn coordinates()->Value{json!({"space":"pdf_page","unit":"pt","origin":"top_left","x_axis":"right","y_axis":"down","rotation":"unrotated"})}
fn rect(v:&Value)->Result<Rect>{
    let a=v.as_array().filter(|a|a.len()==4).context("bbox must contain four PDF points")?;
    let mut b=[0.;4];for (i,x) in a.iter().enumerate(){b[i]=x.as_f64().filter(|x|x.is_finite()).context("Invalid bbox")?;}
    ensure!(b[0]<b[2]&&b[1]<b[3],"Empty bbox");Ok(b)
}
fn inside(a:Rect,b:Rect)->bool{a[0]<=b[0]&&a[1]<=b[1]&&a[2]>=b[2]&&a[3]>=b[3]}
fn intersect(a:Rect,b:Rect)->Option<Rect>{let r=[a[0].max(b[0]),a[1].max(b[1]),a[2].min(b[2]),a[3].min(b[3])];(r[0]<r[2]&&r[1]<r[3]).then_some(r)}
fn union(a:Rect,b:Rect)->Rect{[a[0].min(b[0]),a[1].min(b[1]),a[2].max(b[2]),a[3].max(b[3])]}
fn expand(a:Rect,p:f64,b:Rect)->Rect{[(a[0]-p).max(b[0]),(a[1]-p).max(b[1]),(a[2]+p).min(b[2]),(a[3]+p).min(b[3])]}
fn extent(a:Rect,b:Rect)->[f64;4]{[(a[1]-b[1]).max(0.),(b[2]-a[2]).max(0.),(b[3]-a[3]).max(0.),(a[0]-b[0]).max(0.)]}
fn sides(e:[f64;4])->Value{json!({"top":e[0],"right":e[1],"bottom":e[2],"left":e[3]})}
fn affected(a:Rect,b:Rect)->Vec<&'static str>{EDGES.into_iter().zip(extent(a,b)).filter_map(|(s,n)|(n>0.).then_some(s)).collect()}
fn output(p:&Path)->Result<PathBuf>{
    let p=match p.extension(){None=>p.with_extension("png"),Some(s)=>{ensure!(s.to_str().is_some_and(|s|s.eq_ignore_ascii_case("png")),"Expected PNG destination");p.to_owned()}};
    let parent=p.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or_else(||Path::new("."));
    fs::create_dir_all(parent)?;Ok(files::canonical(parent)?.join(p.file_name().context("Output name")?))
}
fn sibling(p:&Path,suffix:&str)->Result<PathBuf>{Ok(p.with_file_name(format!("{}-{suffix}.png",p.file_stem().and_then(|s|s.to_str()).context("Output stem")?)))}
fn paint(img:&mut image::RgbaImage,b:Rect,m:[f64;6],color:[u8;4]){
    let (w,h)=img.dimensions();let mut r=[f64::INFINITY,f64::INFINITY,f64::NEG_INFINITY,f64::NEG_INFINITY];
    for [x,y] in [[b[0],b[1]],[b[2],b[1]],[b[2],b[3]],[b[0],b[3]]]{
        let p=m[0]*x+m[2]*y+m[4];let q=m[1]*x+m[3]*y+m[5];
        r[0]=r[0].min(p);r[1]=r[1].min(q);r[2]=r[2].max(p);r[3]=r[3].max(q);
    }
    let x0=r[0].floor().clamp(0.,f64::from(w-1)) as u32;let y0=r[1].floor().clamp(0.,f64::from(h-1)) as u32;
    let x1=(r[2].ceil()-1.).clamp(0.,f64::from(w-1)) as u32;let y1=(r[3].ceil()-1.).clamp(0.,f64::from(h-1)) as u32;
    for t in 0..2u32{let a=x0.saturating_add(t).min(x1);let b=y0.saturating_add(t).min(y1);let c=x1.saturating_sub(t).max(a);let d=y1.saturating_sub(t).max(b);
        for x in a..=c{img.put_pixel(x,b,image::Rgba(color));img.put_pixel(x,d,image::Rgba(color));}
        for y in b..=d{img.put_pixel(a,y,image::Rgba(color));img.put_pixel(c,y,image::Rgba(color));}
    }
}
fn inspect_data(text:&Value,structure:&Value,checked:Rect,page:Rect,margin:f64)->Result<(Value,Option<Rect>,Rect)>{
    let search=expand(checked,margin,page);let glyphs=text["glyphs"].as_array().context("Missing actual glyphs")?;
    let mut lines:BTreeMap<u64,Vec<&Value>>=BTreeMap::new();
    for g in glyphs{lines.entry(g["line_index"].as_u64().context("Missing line identity")?).or_default().push(g);}
    let mut crossing=vec![];let mut nearby=vec![];let mut images=vec![];let mut suggested=checked;
    for (id,gs) in lines{
        let mut bbox=None;let mut hit=false;let mut context=false;let mut reading=String::new();let mut outside=vec![];
        for g in gs{
            if g["space_before"]==true&&!reading.is_empty(){reading.push(' ');}
            reading.push_str(g["text"].as_str().context("Missing glyph text")?);
            let Some(b)=rect(&g["bbox"]).ok().and_then(|b|intersect(b,page))else{continue;};
            bbox=Some(bbox.map_or(b,|a|union(a,b)));hit|=intersect(b,checked).is_some();context|=intersect(b,search).is_some();
            if !inside(checked,b){outside.push(json!({"glyph_index":g["glyph_index"],"text":g["text"],"bbox":b,"crosses_crop":intersect(b,checked).is_some()}));}
        }
        let Some(b)=bbox else{continue;};if !context||reading.trim().is_empty(){continue;}
        if hit&&!inside(checked,b){
            crossing.push(json!({"line_index":id,"text":reading,"page_clipped_bbox":b,"edges":affected(checked,b),
                "outward_extent_pt":sides(extent(checked,b)),"excluded_glyphs":outside,"painted_visibility":"not_assessed"}));
            suggested=union(suggested,b);
        }else if !hit{
            nearby.push(json!({"line_index":id,"text":reading,"page_clipped_bbox":b,"edges":affected(checked,b),"painted_visibility":"not_assessed"}));
        }
    }
    if let Some(rows)=structure["occurrences"].as_array(){for r in rows{
        let Some(b)=rect(&r["bbox"]).ok().and_then(|b|intersect(b,page))else{continue;};
        if intersect(b,checked).is_some()&&!inside(checked,b){images.push(json!({"image_xref":r["image_xref"],"occurrence":r["occurrence"],"bbox":b,
            "edges":affected(checked,b),"context_effects":r["context_effects"],"visibility":"not_assessed"}));}
    }}
    let proposal=if crossing.is_empty(){None}else{
        let e=extent(checked,suggested);
        Some([(suggested[0]-if e[3]>0.{2.}else{0.}).max(page[0]),(suggested[1]-if e[0]>0.{2.}else{0.}).max(page[1]),
              (suggested[2]+if e[1]>0.{2.}else{0.}).min(page[2]),(suggested[3]+if e[2]>0.{2.}else{0.}).min(page[3])])
    };
    let complete=text["truncated"]==false&&text["warnings"].as_array().is_some_and(|a|a.is_empty())&&structure["status"]=="structurally_indexed";
    let mut edges=serde_json::Map::new();
    for edge in EDGES{
        let has=|r:&Value|r["edges"].as_array().is_some_and(|a|a.iter().any(|s|s==edge));
        let a:Vec<_>=crossing.iter().filter(|r|has(r)).collect();let b:Vec<_>=nearby.iter().filter(|r|has(r)).collect();let c:Vec<_>=images.iter().filter(|r|has(r)).collect();
        edges.insert(edge.into(),json!({"status":if !a.is_empty(){"crossing_glyph_geometry_detected"}else if !c.is_empty(){"crossing_image_geometry_detected"}else if !complete{"evidence_unresolved"}else{"no_crossing_geometry_detected"},
            "crossing_text":a,"nearby_external_text":b,"crossing_image_objects":c,"review_status":"pending"}));
    }
    let context=expand(proposal.unwrap_or(checked),margin,page);
    Ok((json!({"status":if !crossing.is_empty(){"crossing_glyph_geometry_detected"}else if !images.is_empty(){"crossing_image_geometry_detected"}else if !complete{"incomplete_geometric_evidence"}else if glyphs.is_empty(){"no_extractable_glyphs"}else{"no_crossing_geometry_detected"},
        "edges":edges,"crossing_text_count":crossing.len(),"crossing_image_count":images.len(),"text_extraction":{"backend":"PdfCraft glyphs",
            "truncated":text["truncated"],"warnings":text["warnings"],"painted_visibility":"not_assessed"},
        "structure_status":structure["status"],"suggested_bbox":proposal,"suggestion_applied":false,"figure_membership":"not_assessed",
        "visual_review":{"status":"pending","figure_completeness":"not_assessed"},"full_page_review_required":true}),proposal,context))
}
fn inspect(pdf:&mut Backend,page:u64,checked:Rect,bounds:Rect,context_path:&Path,margin:f64,dpi:u64,overwrite:bool)->Result<Value>{
    let (_,geometry,_,_)=pdf.geometry(page)?;
    let text=pdf.text(page)?;let structure=pdf.structure(page)?;
    let (mut v,proposal,context)=inspect_data(&text,&structure,checked,bounds,margin)?;
    let context_path=output(context_path)?;let marked=sibling(&context_path,"boundaries")?;
    let source=path(&pdf.source_binding()["path"])?;
    ensure!(context_path!=marked&&source!=context_path&&source!=marked,"Context must not replace original PDF");
    let r=pdf.render_with_overwrite(page,Some(context),dpi,&context_path,overwrite)?;
    let (raw,asset)=files::read_bound(&context_path)?;ensure!(asset==r["asset"],"Context changed");
    let mut img=image::load_from_memory_with_format(&raw,image::ImageFormat::Png)?.into_rgba8();
    ensure!(img.width()>0&&img.height()>0,"Empty context");
    let a=r["unrotated_page_to_pixel_transform"].as_array().filter(|a|a.len()==6).context("Missing raster transform")?;
    let mut m=[0.;6];for (i,x) in a.iter().enumerate(){m[i]=x.as_f64().filter(|n|n.is_finite()).context("Invalid raster transform")?;}
    if let Some(b)=proposal{paint(&mut img,b,m,[26,145,70,255]);}paint(&mut img,checked,m,[220,45,55,255]);
    let mut bytes=Vec::new();PngEncoder::new(&mut bytes).write_image(img.as_raw(),img.width(),img.height(),ExtendedColorType::Rgba8)?;
    pdf.verify()?;ensure!(check_ledger::bind_file(&context_path)?==asset,"Context changed during annotation");
    if overwrite{pipeline::publish_replace(&marked,&bytes)?;}else{pipeline::publish(&marked,&bytes)?;}
    let annotated=check_ledger::bind_file(&marked)?;
    ensure!(annotated["sha256"]==files::digest(&bytes),"Annotated context changed");pdf.verify()?;
    v["schema_version"]=json!(1);v["source_file"]=pdf.source_binding().clone();v["page"]=json!(page);v["coordinate_system"]=coordinates();
    v["checked_bbox"]=json!(checked);v["page_geometry"]=geometry;
    v["context"]=json!({"asset":asset,"annotated_asset":annotated,"bbox":context,"dpi":dpi,"pixel_size":[r["width"],r["height"]],
        "page_to_pixel_transform":m,"outside_crop_pt":sides(extent(checked,context)),"requested_context_margin_pt":margin,
        "legend":{"red":"checked crop","green":"suggested geometric bbox"},"orientation":"source_page_display"});
    v["evidence_boundary"]=json!("Geometric glyph/image clues, not painted ink. Hidden OCR text, clipped or occluded ink, vector and raster labels, and figure membership need independent review. No suggestion is applied.");
    Ok(v)
}
pub struct RegionOptions<'a>{
    pub page:u64,pub bbox:&'a Value,pub coordinate_system:&'a Value,pub destination:&'a Path,pub margin:f64,pub dpi:u64,
    pub expected:Option<&'a str>,pub context:Option<&'a Path>,pub context_margin:f64,pub context_dpi:u64,pub overwrite:bool,pub transparent:bool,
}
#[allow(clippy::too_many_arguments)]
pub fn inspect_crop_boundaries(source:&Path,page:u64,bbox:&Value,coords:&Value,dest:&Path,margin:f64,dpi:u64,expected:Option<&str>)->Result<Value>{
    inspect_crop_boundaries_with_overwrite(source,page,bbox,coords,dest,margin,dpi,expected,false)
}
#[allow(clippy::too_many_arguments)]
pub fn inspect_crop_boundaries_with_overwrite(source:&Path,page:u64,bbox:&Value,coords:&Value,dest:&Path,margin:f64,dpi:u64,expected:Option<&str>,overwrite:bool)->Result<Value>{
    ensure!(*coords==coordinates()&&(72..=1200).contains(&dpi)&&margin.is_finite()&&margin>0.,"Invalid coordinates, context dpi or margin");
    let mut pdf=Backend::open(source,expected)?;let (_,_,bounds,_)=pdf.geometry(page)?;
    let b=rect(bbox)?;ensure!(inside(bounds,b),"bbox must be inside actual unrotated page");
    inspect(&mut pdf,page,b,bounds,dest,margin,dpi,overwrite)
}
pub fn render_page_region(source:&Path,opt:RegionOptions<'_>)->Result<Value>{
    ensure!(*opt.coordinate_system==coordinates()&&(72..=1200).contains(&opt.dpi)&&(72..=1200).contains(&opt.context_dpi),"Invalid coordinate system or dpi");
    ensure!(opt.margin.is_finite()&&opt.margin>=0.&&opt.context_margin.is_finite()&&opt.context_margin>0.,"Invalid margins");
    let mut pdf=Backend::open(source,opt.expected)?;let (_,geometry,bounds,_)=pdf.geometry(opt.page)?;
    let b=rect(opt.bbox)?;ensure!(inside(bounds,b),"bbox must be inside actual unrotated page");
    let actual=expand(b,opt.margin,bounds);let dest=output(opt.destination)?;
    let context=match opt.context{Some(p)=>output(p)?,None=>sibling(&dest,"context")?};let marked=sibling(&context,"boundaries")?;
    let original=path(&pdf.source_binding()["path"])?;
    ensure!(dest!=context&&dest!=marked&&context!=marked&&dest!=original&&context!=original&&marked!=original,"Crop, context and source destinations must differ");
    let boundary=inspect(&mut pdf,opt.page,actual,bounds,&context,opt.context_margin,opt.context_dpi,opt.overwrite)?;
    let render=pdf.render_with_options(opt.page,Some(actual),opt.dpi,&dest,opt.overwrite,opt.transparent)?;pdf.verify()?;
    Ok(json!({"schema_version":1,"source_file":pdf.source_binding(),"page":opt.page,"requested_bbox":b,"rendered_bbox":actual,
        "coordinate_system":coordinates(),"requested_margin_pt":opt.margin,"actual_margin_pt":sides(extent(b,actual)),"dpi":opt.dpi,
        "asset":render["asset"],"method":"page_region_render","page_geometry":geometry,"orientation":"source_page_display",
        "transparent_background":opt.transparent,"pixel_format":render["pixel_format"],
        "boundary_check":boundary,"edge_reviews":EDGES.iter().map(|e|json!({"edge":e,"status":"pending","reason":"","evidence":[]})).collect::<Vec<_>>(),
        "visual_review":{"status":"pending","figure_completeness":"not_assessed"},"source_fidelity":"not_assessed","backend_render":render,
        "evidence_boundary":"Actual crop, not evidence that nothing is missing. Review all four edges independently."}))
}
pub struct BatchRegionOptions<'a>{
    pub expected:Option<&'a str>,
    pub overwrite:bool,
    pub contact_sheet_destination:Option<&'a Path>,
    pub contact_sheet_columns:Option<usize>,
}

fn trim_rgba_margins(img:&image::RgbaImage,padding_px:u32)->image::RgbaImage{
    let (w,h)=img.dimensions();
    if w==0||h==0{return img.clone();}
    let (mut min_x,mut min_y,mut max_x,mut max_y)=(w,h,0u32,0u32);
    let mut found=false;
    for (x,y,p) in img.enumerate_pixels(){
        let fg=p[3]>8&&!(p[0]>=246&&p[1]>=246&&p[2]>=246);
        if fg{
            found=true;
            min_x=min_x.min(x);min_y=min_y.min(y);
            max_x=max_x.max(x);max_y=max_y.max(y);
        }
    }
    if !found{return img.clone();}
    let x0=min_x.saturating_sub(padding_px);
    let y0=min_y.saturating_sub(padding_px);
    let x1=(max_x+padding_px+1).min(w);
    let y1=(max_y+padding_px+1).min(h);
    image::imageops::crop_imm(img,x0,y0,(x1-x0).max(1),(y1-y0).max(1)).to_image()
}

fn glyph_5x7(ch:char)->[u8;7]{
    match ch{
        '#'=>[0b01010,0b11111,0b01010,0b01010,0b11111,0b01010,0b00000],
        '0'=>[0b01110,0b10001,0b10011,0b10101,0b11001,0b10001,0b01110],
        '1'=>[0b00100,0b01100,0b00100,0b00100,0b00100,0b00100,0b01110],
        '2'=>[0b01110,0b10001,0b00001,0b00110,0b01000,0b10000,0b11111],
        '3'=>[0b11110,0b00001,0b00001,0b01110,0b00001,0b00001,0b11110],
        '4'=>[0b00010,0b00110,0b01010,0b10010,0b11111,0b00010,0b00010],
        '5'=>[0b11111,0b10000,0b11110,0b00001,0b00001,0b10001,0b01110],
        '6'=>[0b01110,0b10000,0b11110,0b10001,0b10001,0b10001,0b01110],
        '7'=>[0b11111,0b00001,0b00010,0b00100,0b01000,0b01000,0b01000],
        '8'=>[0b01110,0b10001,0b10001,0b01110,0b10001,0b10001,0b01110],
        '9'=>[0b01110,0b10001,0b10001,0b01111,0b00001,0b00001,0b01110],
        _ =>[0;7],
    }
}

fn draw_badge(canvas:&mut image::RgbaImage,x0:u32,y0:u32,label:&str){
    let scale=2u32;
    let char_w=6u32*scale;
    let bw=label.chars().count() as u32*char_w+8;
    let bh=7*scale+8;
    let (cw,ch)=canvas.dimensions();
    for dy in 0..bh{
        for dx in 0..bw{
            let x=x0+dx;let y=y0+dy;
            if x<cw&&y<ch{canvas.put_pixel(x,y,image::Rgba([205,36,44,255]));}
        }
    }
    for (ci,c) in label.chars().enumerate(){
        let rows=glyph_5x7(c);
        for (ry,bits) in rows.iter().enumerate(){
            for rx in 0..5u32{
                if (bits>>(4-rx))&1==1{
                    for sy in 0..scale{
                        for sx in 0..scale{
                            let px=x0+4+(ci as u32)*char_w+rx*scale+sx;
                            let py=y0+4+(ry as u32)*scale+sy;
                            if px<cw&&py<ch{canvas.put_pixel(px,py,image::Rgba([255,255,255,255]));}
                        }
                    }
                }
            }
        }
    }
}

fn build_contact_sheet(tiles:&[(usize,image::RgbaImage)],columns_opt:Option<usize>,dest:&Path,overwrite:bool)->Result<Value>{
    let dest=output(dest)?;
    let n=tiles.len();
    let cols=columns_opt.unwrap_or(if n<=1{1}else if n<=4{2}else{3}).clamp(1,4);
    let rows=n.div_ceil(cols).max(1);
    let (slot_w,slot_h,pad)=(560u32,420u32,16u32);
    let total_w=pad+(slot_w+pad)*cols as u32;
    let total_h=pad+(slot_h+pad)*rows as u32;
    let mut canvas=image::RgbaImage::from_pixel(total_w,total_h,image::Rgba([248,248,250,255]));
    for (slot_idx,(item_num,tile)) in tiles.iter().enumerate(){
        let col=(slot_idx%cols) as u32;
        let row=(slot_idx/cols) as u32;
        let sx=pad+col*(slot_w+pad);
        let sy=pad+row*(slot_h+pad);
        for yy in 0..slot_h{
            for xx in 0..slot_w{
                let c=if ((xx/12)+(yy/12))%2==0{242u8}else{228u8};
                canvas.put_pixel(sx+xx,sy+yy,image::Rgba([c,c,c,255]));
            }
        }
        let (tw,th)=tile.dimensions();
        if tw>0&&th>0{
            let max_w=f64::from(slot_w-16);
            let max_h=f64::from(slot_h-16);
            let scale=(max_w/f64::from(tw)).min(max_h/f64::from(th)).min(2.0);
            let nw=((f64::from(tw)*scale).round() as u32).max(1).min(slot_w-12);
            let nh=((f64::from(th)*scale).round() as u32).max(1).min(slot_h-12);
            let resized=image::imageops::resize(tile,nw,nh,image::imageops::FilterType::Triangle);
            let ox=sx+(slot_w-nw)/2;
            let oy=sy+(slot_h-nh)/2;
            image::imageops::overlay(&mut canvas,&resized,i64::from(ox),i64::from(oy));
        }
        for t in 0..2u32{
            for x in sx..sx+slot_w{
                canvas.put_pixel(x,sy+t,image::Rgba([215,40,48,255]));
                canvas.put_pixel(x,sy+slot_h-1-t,image::Rgba([215,40,48,255]));
            }
            for y in sy..sy+slot_h{
                canvas.put_pixel(sx+t,y,image::Rgba([215,40,48,255]));
                canvas.put_pixel(sx+slot_w-1-t,y,image::Rgba([215,40,48,255]));
            }
        }
        draw_badge(&mut canvas,sx+4,sy+4,&format!("#{item_num}"));
    }
    let mut bytes=Vec::new();
    PngEncoder::new(&mut bytes).write_image(canvas.as_raw(),total_w,total_h,ExtendedColorType::Rgba8)?;
    if overwrite{pipeline::publish_replace(&dest,&bytes)?;}else{pipeline::publish(&dest,&bytes)?;}
    Ok(json!({"asset":check_ledger::bind_file(&dest)?,"width":total_w,"height":total_h,"columns":cols,"tiles":n}))
}

pub fn render_page_regions_batch(source:&Path,regions:&Value,opt:BatchRegionOptions<'_>)->Result<Value>{
    let rows=regions.as_array().filter(|a|!a.is_empty()).context("regions must be a nonempty array")?;
    ensure!(rows.len()<=32,"Batch crop size exceeds 32 regions per call");
    let mut pdf=Backend::open(source,opt.expected)?;
    let original=path(&pdf.source_binding()["path"])?;
    let mut geom_cache=BTreeMap::<u64,(Value,Rect)>::new();
    let mut items=Vec::with_capacity(rows.len());
    let mut tiles=Vec::with_capacity(rows.len());

    for (idx,row) in rows.iter().enumerate(){
        let page=row["page_number"].as_u64().context("region.page_number must be an integer")?;
        if let Some(cs)=row.get("coordinate_system").filter(|v|!v.is_null()){
            ensure!(*cs==coordinates(),"Invalid region.coordinate_system");
        }
        let (_,bounds)=if let Some(g)=geom_cache.get(&page){g.clone()}else{
            let (_,geom,b,_)=pdf.geometry(page)?;
            geom_cache.insert(page,(geom.clone(),b));
            (geom,b)
        };
        let b=rect(&row["bbox"])?;
        ensure!(inside(bounds,b),"region.bbox must be inside actual unrotated page");
        let margin=row.get("margin_pt").and_then(Value::as_f64).unwrap_or(6.0);
        ensure!(margin.is_finite()&&margin>=0.,"Invalid region.margin_pt");
        let dpi=row.get("dpi").and_then(Value::as_u64).unwrap_or(300);
        ensure!((72..=1200).contains(&dpi),"Invalid region.dpi");
        let transparent=row.get("transparent").and_then(Value::as_bool).unwrap_or(true);
        let auto_trim=row.get("auto_trim").and_then(Value::as_bool).unwrap_or(false);
        let dest=output(&path(&row["destination"])?)?;
        ensure!(dest!=original,"Crop destination cannot overwrite source PDF");

        let actual=expand(b,margin,bounds);
        let (mut bytes,mut meta)=pdf.raster(page,Some(actual),dpi,transparent)?;
        let mut rgba=image::load_from_memory_with_format(&bytes,image::ImageFormat::Png)?.into_rgba8();
        if auto_trim{
            rgba=trim_rgba_margins(&rgba,6);
            bytes.clear();
            PngEncoder::new(&mut bytes).write_image(rgba.as_raw(),rgba.width(),rgba.height(),ExtendedColorType::Rgba8)?;
            meta["trimmed_width"]=json!(rgba.width());
            meta["trimmed_height"]=json!(rgba.height());
        }
        if opt.overwrite{pipeline::publish_replace(&dest,&bytes)?;}else{pipeline::publish(&dest,&bytes)?;}
        let asset=check_ledger::bind_file(&dest)?;
        items.push(json!({
            "index":idx+1,
            "label":row.get("label").cloned().unwrap_or(Value::Null),
            "page":page,
            "requested_bbox":b,
            "rendered_bbox":actual,
            "dpi":dpi,
            "transparent_background":transparent,
            "auto_trim":auto_trim,
            "asset":asset,
            "alpha_min":meta["alpha_min"],
            "alpha_max":meta["alpha_max"],
        }));
        tiles.push((idx+1,rgba));
    }
    pdf.verify()?;

    let contact_sheet=if let Some(cs_dest)=opt.contact_sheet_destination{
        ensure!(path(&json!(cs_dest.to_string_lossy().as_ref()))?!=original,"Contact sheet cannot overwrite source PDF");
        build_contact_sheet(&tiles,opt.contact_sheet_columns,cs_dest,opt.overwrite)?
    }else{Value::Null};

    Ok(json!({
        "schema_version":1,
        "status":"ok",
        "source_file":pdf.source_binding(),
        "count":items.len(),
        "items":items,
        "contact_sheet":contact_sheet,
        "source_fidelity":"not_assessed",
        "evidence_boundary":"Batch crops rendered with optional auto_trim and numbered contact_sheet for visual inspection.",
    }))
}

#[cfg(test)]
mod tests{
    use super::*;
    use lopdf::{Document,Object,Stream,dictionary};
    #[test]
    fn batch_render_trims_and_builds_contact_sheet(){
        let tmp=tempfile::tempdir().unwrap();
        let pdf=tmp.path().join("figs.pdf");
        let mut doc=Document::with_version("1.5");
        let pages=doc.new_object_id();
        let img=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>2,"Height"=>2,"BitsPerComponent"=>8,"ColorSpace"=>"DeviceRGB"},vec![20,40,180,20,40,180,20,40,180,20,40,180]));
        let res=dictionary!{"XObject"=>dictionary!{"Im"=>img}};
        let c1=doc.add_object(Stream::new(dictionary!{},b"q 60 0 0 40 50 180 cm /Im Do Q".to_vec()));
        let p1=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>c1,"Resources"=>res});
        doc.objects.insert(pages,Object::Dictionary(dictionary!{"Type"=>"Pages","Kids"=>vec![Object::Reference(p1)],"Count"=>1,"MediaBox"=>vec![0.into(),0.into(),300.into(),300.into()]}));
        let catalog=doc.add_object(dictionary!{"Type"=>"Catalog","Pages"=>pages});
        doc.trailer.set("Root",catalog);
        doc.save(&pdf).unwrap();

        let fig1=tmp.path().join("out/fig1.png");
        let fig2=tmp.path().join("out/fig2.png");
        let sheet=tmp.path().join("out/contact.png");
        let regions=json!([
            {"page_number":1,"bbox":[40.0,65.0,125.0,130.0],"destination":fig1,"dpi":150,"auto_trim":true,"transparent":true,"label":"fig1"},
            {"page_number":1,"bbox":[40.0,65.0,125.0,130.0],"destination":fig2,"dpi":150,"auto_trim":false,"transparent":false,"label":"fig2"}
        ]);
        let r=render_page_regions_batch(&pdf,&regions,BatchRegionOptions{
            expected:None,overwrite:true,contact_sheet_destination:Some(&sheet),contact_sheet_columns:Some(2),
        }).unwrap();
        assert_eq!(r["count"],2);
        assert!(fig1.is_file()&&fig2.is_file()&&sheet.is_file());
        assert_eq!(r["items"][0]["transparent_background"],true);
        assert_eq!(r["items"][1]["transparent_background"],false);
        assert_eq!(r["contact_sheet"]["columns"],2);
    }
    #[test]
    fn external_text_does_not_expand_crop(){
        let g=|i:u64,l:u64,t:&str,b:Rect|json!({"glyph_index":i,"line_index":l,"text":t,"bbox":b});
        let t=json!({"glyphs":[g(0,1,"a",[20.,20.,25.,25.]),g(1,1,"b",[25.,20.,35.,25.]),g(2,2,"outside",[60.,20.,70.,25.])],"truncated":false,"warnings":[]});
        let s=json!({"status":"structurally_indexed","occurrences":[]});
        let (v,b,_)=inspect_data(&t,&s,[10.,10.,30.,30.],[0.,0.,100.,100.],50.).unwrap();
        assert_eq!(b,Some([10.,10.,37.,30.]));assert_eq!(v["crossing_text_count"],1);
        assert_eq!(v["suggestion_applied"],false);assert_eq!(v["edges"]["right"]["nearby_external_text"].as_array().unwrap().len(),1);
    }
}
