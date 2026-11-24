use lopdf::{Document,Object,Stream,dictionary};
use serde_json::json;
use std::{fs,path::Path};
use z2o_native::{pdfcraft_backend as backend,files};
fn fixture(path:&Path){
    let mut doc=Document::with_version("1.5");
    let pages=doc.new_object_id();
    let font=doc.add_object(dictionary!{"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
    let resources=doc.add_object(dictionary!{"Font"=>dictionary!{"F1"=>font}});
    let content=doc.add_object(Stream::new(dictionary!{},b"1 0 0 rg 10 20 30 40 re f BT /F1 12 Tf 1 0 0 1 10 85 Tm (Native PDF evidence) Tj ET".to_vec()));
    let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>content,"Resources"=>resources});
    let rotated=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>content,"Resources"=>resources,"Rotate"=>90});
    doc.objects.insert(pages,Object::Dictionary(dictionary!{"Type"=>"Pages","Kids"=>vec![Object::Reference(page),Object::Reference(rotated)],"Count"=>2,"MediaBox"=>vec![0.into(),0.into(),200.into(),100.into()]}));
    let catalog=doc.add_object(dictionary!{"Type"=>"Catalog","Pages"=>pages});
    doc.trailer.set("Root",catalog);
    doc.save(path).unwrap();
}
#[test]
fn pdfcraft_headless_pixels_glyphs_and_rotated_crop(){
    let tmp=tempfile::tempdir().unwrap();let dir=tmp.path();let pdf=dir.join("fixture.pdf");
    fixture(&pdf);
    let original=fs::read(&pdf).unwrap();
    let page=backend::render_page(&pdf,1,72,&dir.join("full.png"),None).unwrap();
    assert_eq!(page["width"],200);assert_eq!(page["height"],100);
    let raster=image::open(dir.join("full.png")).unwrap().into_rgba8();
    assert_eq!(raster.get_pixel(20,60).0,[255,0,0,255]);
    let text=backend::extract_text(&pdf,1,None).unwrap();
    assert!(text["plain_text"].as_str().unwrap().contains("Native PDF evidence"));
    assert!(!text["glyphs"].as_array().unwrap().is_empty());
    assert_eq!(text["truncated"],false);
    let coords=json!({"space":"pdf_page","unit":"pt","origin":"top_left","x_axis":"right","y_axis":"down","rotation":"unrotated"});
    let crop=backend::render_region(&pdf,1,&json!([10,40,40,80]),&coords,72,&dir.join("crop"),None).unwrap();
    assert_eq!(crop["width"],30);assert_eq!(crop["height"],40);
    let cropped=image::open(dir.join("crop.png")).unwrap().into_rgba8();
    assert_eq!(cropped.get_pixel(15,20).0,[255,0,0,255]);
    let rot=backend::render_region(&pdf,2,&json!([10,40,40,80]),&coords,72,&dir.join("rot"),None).unwrap();
    assert_eq!(rot["width"],40);assert_eq!(rot["height"],30);
    let rotated=image::open(dir.join("rot.png")).unwrap().into_rgba8();
    assert_eq!(rotated.get_pixel(20,15).0,[255,0,0,255]);
    let rotated_text=backend::extract_text(&pdf,2,None).unwrap();
    assert!(rotated_text["plain_text"].as_str().unwrap().contains("Native PDF evidence"));
    assert_eq!(rotated_text["page_geometry"]["page_rotation"],90);
    let out=backend::prepare(&pdf,&dir.join("evidence"),72,None,|_,_,_|{}).unwrap();
    let evidence=z2o_native::pipeline::read_json(&out).unwrap();
    assert_eq!(evidence["page_count"],2);assert_eq!(evidence["source_inventory_ready"],false);
    for row in evidence["pages"].as_array().unwrap(){
        for binding in [&row["render"]["asset"],&row["text"],&row["glyphs"],&row["image_structure"]]{
            let p=Path::new(binding["path"].as_str().unwrap());
            assert_eq!(files::snapshot(p).unwrap()["sha256"],binding["sha256"]);
        }
    }
    assert_eq!(fs::read(&pdf).unwrap(),original);
    let changed=dir.join("changed.png");
    assert!(backend::render_page(&pdf,1,72,&changed,Some(&"0".repeat(64))).is_err());
    assert!(!changed.exists());
    assert!(backend::render_region(&pdf,1,&json!([0,0,201,100]),&coords,72,&changed,None).is_err());
    assert!(!changed.exists());
    fs::write(dir.join("full.png"),b"keep existing").unwrap();
    assert!(backend::render_page(&pdf,1,72,&dir.join("full.png"),None).is_err());
    assert_eq!(fs::read(dir.join("full.png")).unwrap(),b"keep existing");
}


#[test]
fn strict_native_image_extraction_preserves_bytes_and_alpha(){
    use image::ImageEncoder;
    use z2o_native::pdf_structure::export_image_object;
    let tmp=tempfile::tempdir().unwrap();let root=tmp.path();let pdf=root.join("image.pdf");
    let mut doc=Document::with_version("1.5");let pages=doc.new_object_id();
    let rgb=[20u8,100,200,200,30,80];let mut jpeg=Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg,95).write_image(&rgb,2,1,image::ExtendedColorType::Rgb8).unwrap();
    let j=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>2,"Height"=>1,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8,"Filter"=>"DCTDecode"},jpeg.clone()));
    let m=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>2,"Height"=>1,"ColorSpace"=>"DeviceGray","BitsPerComponent"=>8,"Decode"=>vec![1.into(),0.into()]},vec![255,127]));
    let f=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>2,"Height"=>1,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8,"Filter"=>"FlateDecode","SMask"=>m},craft_pdf::filters::encode_flate(&rgb)));
    let unused=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>2,"Height"=>1,"ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8},rgb.to_vec()));
    let content=doc.add_object(Stream::new(dictionary!{},b"q 20 0 0 10 5 50 cm /J Do Q q 20 0 0 10 40 50 cm /F Do Q".to_vec()));
    let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>content,"Resources"=>dictionary!{"XObject"=>dictionary!{"J"=>j,"F"=>f,"Unused"=>unused}}});
    doc.objects.insert(pages,Object::Dictionary(dictionary!{"Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1,"MediaBox"=>vec![0.into(),0.into(),100.into(),100.into()]}));
    let catalog=doc.add_object(dictionary!{"Type"=>"Catalog","Pages"=>pages});doc.trailer.set("Root",catalog);doc.save(&pdf).unwrap();
    let original=fs::read(&pdf).unwrap();
    let store=root.join("object-store");
    let indexed=z2o_native::pdf_structure::index_page_with_objects(&pdf,1,Some(&store),None,|_,_,_|{}).unwrap();
    assert_eq!(indexed["objects"].as_array().unwrap().len(),2);
    assert_eq!(indexed["object_streams"].as_array().unwrap().len(),3);
    let found=indexed["objects"].as_array().unwrap().iter().find(|r|r["image_xref"]==f.0).unwrap();
    assert_eq!(found["smask_stream"]["image_xref"],m.0);
    assert_eq!(fs::read(found["smask_stream"]["asset"]["path"].as_str().unwrap()).unwrap(),vec![255,127]);
    let repeated=z2o_native::pdf_structure::index_page_with_objects(&pdf,1,Some(&store),None,|_,_,_|{}).unwrap();
    assert_eq!(repeated,indexed);
    assert!(z2o_native::pdf_structure::index_page_with_objects(&pdf,1,Some(&store),Some(&"0".repeat(64)),|_,_,_|{}).is_err());
    let a=export_image_object(&pdf,1,j.0,&root.join("jpeg"),None).unwrap();
    assert_eq!(a["method"],"original_extracted_bytes");assert_eq!(fs::read(root.join("jpeg.jpeg")).unwrap(),jpeg);
    let b=export_image_object(&pdf,1,f.0,&root.join("alpha"),None).unwrap();
    assert_eq!(b["method"],"smask_composited_png");assert_eq!(b["source_object"]["smask"]["alpha_min"],0);
    let img=image::open(root.join("alpha.png")).unwrap().into_rgba8();
    assert_eq!(img.get_pixel(0,0).0,[20,100,200,0]);assert_eq!(img.get_pixel(1,0).0,[200,30,80,128]);
    assert!(export_image_object(&pdf,1,unused.0,&root.join("unused"),None).is_err());
    assert!(export_image_object(&pdf,1,f.0,&root.join("wrong.jpg"),None).is_err());
    assert!(export_image_object(&pdf,1,j.0,&root.join("changed"),Some(&"0".repeat(64))).is_err());
    assert_eq!(fs::read(&pdf).unwrap(),original);
    fs::write(root.join("jpeg.jpeg"),b"keep").unwrap();
    assert!(export_image_object(&pdf,1,j.0,&root.join("jpeg"),None).is_err());
    assert_eq!(fs::read(root.join("jpeg.jpeg")).unwrap(),b"keep");
}
#[test]
fn native_crop_with_context_keeps_four_pending_edges(){
    use z2o_native::pdf_boundary::{self,RegionOptions};
    let tmp=tempfile::tempdir().unwrap();let root=tmp.path();let pdf=root.join("fixture.pdf");fixture(&pdf);
    let original=fs::read(&pdf).unwrap();let dest=root.join("crop.png");
    let coords=json!({"space":"pdf_page","unit":"pt","origin":"top_left","x_axis":"right","y_axis":"down","rotation":"unrotated"});
    let bbox=json!([5,5,45,25]);
    let result=pdf_boundary::render_page_region(&pdf,RegionOptions{page:1,bbox:&bbox,coordinate_system:&coords,destination:&dest,margin:0.,
        dpi:72,expected:None,context:None,context_margin:20.,context_dpi:72}).unwrap();
    assert_eq!(result["rendered_bbox"],json!([5.,5.,45.,25.]));
    assert_eq!(result["boundary_check"]["suggestion_applied"],false);
    assert_eq!(result["boundary_check"]["full_page_review_required"],true);
    assert!(result["edge_reviews"].as_array().unwrap().iter().all(|e|e["status"]=="pending"));
    for b in [&result["asset"],&result["boundary_check"]["context"]["asset"],&result["boundary_check"]["context"]["annotated_asset"]]{
        assert_eq!(files::snapshot(Path::new(b["path"].as_str().unwrap())).unwrap()["sha256"],b["sha256"]);
    }
    assert_eq!(image::open(&dest).unwrap().width(),40);assert_eq!(fs::read(&pdf).unwrap(),original);
    assert!(pdf_boundary::inspect_crop_boundaries(&pdf,1,&json!([0,0,201,100]),&coords,&root.join("bad.png"),20.,72,None).is_err());
}


#[test]
fn mixed_inventory_pdf_uses_real_lines_and_independent_source_replay(){
    use z2o_native::{source_prepare,pdf_inventory,draft_queue,pipeline,delivery_provenance};
    let tmp=tempfile::tempdir().unwrap();let root=tmp.path();
    let pdf=root.join("fixture.pdf");fixture(&pdf);
    let sources=json!({"main":pdf});
    let inv=source_prepare::prepare_sources(&sources,&root.join("work"),&json!({"main":"main"}),None,72,None,|_,_,_|{}).unwrap();
    let inventory=pipeline::read_json(&inv).unwrap();let row=&inventory["sources"][0];
    assert_eq!(row["inventory_backend"],pdf_inventory::BACKEND);assert_eq!(row["page_count"],2);
    assert!(row["pages"][0]["blocks"].as_array().unwrap().iter().any(|b|b["text"].as_str().unwrap().contains("Native PDF evidence")));
    assert!(row["pages"][0]["blocks"][0]["id"].as_str().unwrap().contains("-pc-line-"));
    assert_eq!(row["pages"][0]["pdf_evidence"]["reading_order_review"],"pending");
    assert_eq!(row["pages"][1]["pdf_evidence"]["render_metadata"]["page_geometry"]["page_rotation"],90);
    let report=pdf_inventory::replay(row,&pdf,|_,_,_|{}).unwrap();
    assert_eq!(report["same_backend_replay_complete"],true);assert_eq!(report["source_checks_complete"],false);
    let audit=delivery_provenance::validate(&inv,|_,_,_|{}).unwrap();
    assert_eq!(audit["pdf_native_replays"][0]["same_backend_replay_complete"],true);
    assert_eq!(audit["delivery_ready"],false);
    let queue=root.join("draft.json");draft_queue::prepare(&inv,&queue,|_,_,_|{}).unwrap();
    let task=draft_queue::resume_snapshot(&queue).unwrap();
    assert_eq!(task["next_task"]["stage"],"inventory_page");
    assert_eq!(task["next_task"]["page"]["pdf_evidence"]["reading_order_review"],"pending");
    let mut altered=row.clone();altered["pages"][0]["blocks"][0]["text"]=json!("invented");
    assert!(pdf_inventory::replay(&altered,&pdf,|_,_,_|{}).is_err());
    let p=Path::new(row["pages"][0]["pdf_evidence"]["glyphs"]["path"].as_str().unwrap());
    fs::write(p,b"tampered").unwrap();
    assert!(draft_queue::resume_snapshot(&queue).is_err());
    assert!(pdf_inventory::replay(row,&pdf,|_,_,_|{}).is_err());
}
#[test]
fn scan_without_ocr_resources_preserves_page_and_requires_review(){
    use z2o_native::{source_prepare,pipeline,pdf_inventory,draft_queue};
    let tmp=tempfile::tempdir().unwrap();let root=tmp.path();let pdf=root.join("scan.pdf");
    let mut doc=Document::with_version("1.5");let pages=doc.new_object_id();
    let obj=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>1,"Height"=>1,"BitsPerComponent"=>8,"ColorSpace"=>"DeviceRGB"},vec![20,30,40]));
    let content=doc.add_object(Stream::new(dictionary!{},b"q 100 0 0 100 0 0 cm /Img Do Q".to_vec()));
    let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>content,"Resources"=>dictionary!{"XObject"=>dictionary!{"Img"=>obj}}});
    doc.objects.insert(pages,Object::Dictionary(dictionary!{"Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1,"MediaBox"=>vec![0.into(),0.into(),100.into(),100.into()]}));
    let catalog=doc.add_object(dictionary!{"Type"=>"Catalog","Pages"=>pages});doc.trailer.set("Root",catalog);doc.save(&pdf).unwrap();
    let inv=source_prepare::prepare_sources(&json!({"main":pdf}),&root.join("work"),&json!({"main":"main"}),None,72,None,|_,_,_|{}).unwrap();
    let v=pipeline::read_json(&inv).unwrap();let s=&v["sources"][0];let page=&s["pages"][0];
    assert_eq!(page["pdf_evidence"]["ocr"]["status"],"ocr_resources_required");assert_eq!(page["blocks"],json!([]));
    assert_eq!(s["image_objects"][0]["image_xref"],obj.0);
    assert_eq!(fs::read(s["image_objects"][0]["encoded_stream"]["path"].as_str().unwrap()).unwrap(),vec![20,30,40]);
    assert_eq!(pdf_inventory::replay(s,&pdf,|_,_,_|{}).unwrap()["same_backend_replay_complete"],true);
    let queue=root.join("queue.json");draft_queue::prepare(&inv,&queue,|_,_,_|{}).unwrap();
    let state=draft_queue::resume_snapshot(&queue).unwrap();
    assert_eq!(state["status"]["draft_ready"],false);assert_eq!(state["next_task"]["page"]["pdf_evidence"]["ocr"]["status"],"ocr_resources_required");
    let mut changed=s.clone();changed["image_objects"]=json!([]);
    assert!(pdf_inventory::replay(&changed,&pdf,|_,_,_|{}).is_err());
}
