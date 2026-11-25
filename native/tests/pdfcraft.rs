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

#[test]
fn native_packed_pdf_samples_and_bound_mask_dependencies(){
    use z2o_native::{pdf_structure::export_image_object,source_prepare,pdf_inventory,pipeline};
    let tmp=tempfile::tempdir().unwrap();let root=tmp.path();let pdf=root.join("packed.pdf");
    let mut doc=Document::with_version("1.5");let pages=doc.new_object_id();
    // Three two-bit grayscale samples per row; two padding bits are ignored.
    let mask=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>3,"Height"=>2,"BitsPerComponent"=>2,"ColorSpace"=>"DeviceGray","Decode"=>vec![1.into(),0.into()]},vec![0b00011011,0b11100111]));
    let obj=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>3,"Height"=>2,"BitsPerComponent"=>2,"ColorSpace"=>"DeviceGray","SMask"=>mask},vec![0b00011011,0b11100111]));
    let content=doc.add_object(Stream::new(dictionary!{},b"q 30 0 0 20 10 10 cm /Img Do Q".to_vec()));
    let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>content,"Resources"=>dictionary!{"XObject"=>dictionary!{"Img"=>obj}}});
    doc.objects.insert(pages,Object::Dictionary(dictionary!{"Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1,"MediaBox"=>vec![0.into(),0.into(),100.into(),100.into()]}));
    let catalog=doc.add_object(dictionary!{"Type"=>"Catalog","Pages"=>pages});doc.trailer.set("Root",catalog);doc.save(&pdf).unwrap();
    let export=export_image_object(&pdf,1,obj.0,&root.join("packed"),None).unwrap();
    assert_eq!(export["source_object"]["bits_per_component"],2);
    let image=image::open(root.join("packed.png")).unwrap().into_luma_alpha8();
    assert_eq!(image.get_pixel(0,0).0,[0,255]);assert_eq!(image.get_pixel(1,0).0,[85,170]);assert_eq!(image.get_pixel(2,0).0,[170,85]);
    assert_eq!(image.get_pixel(0,1).0,[255,0]);assert_eq!(image.get_pixel(1,1).0,[170,85]);assert_eq!(image.get_pixel(2,1).0,[85,170]);
    let inv=source_prepare::prepare_sources(&json!({"main":pdf}),&root.join("work"),&json!({"main":"main"}),None,72,None,|_,_,_|{}).unwrap();
    let inventory=pipeline::read_json(&inv).unwrap();let source=&inventory["sources"][0];
    assert_eq!(source["image_dependency_schema"],1);
    let recorded=&source["image_objects"][0]["smask_stream"];
    assert_eq!(recorded["image_xref"],mask.0);
    assert_eq!(fs::read(recorded["asset"]["path"].as_str().unwrap()).unwrap(),vec![0b00011011,0b11100111]);
    assert_eq!(pdf_inventory::replay(source,&pdf,|_,_,_|{}).unwrap()["image_dependency_replay_complete"],true);
    fs::write(recorded["asset"]["path"].as_str().unwrap(),b"tampered").unwrap();
    assert!(pdf_inventory::replay(source,&pdf,|_,_,_|{}).is_err());
}

#[test]
fn source_preflight_rejects_ambiguous_roles_and_case_conflicting_ids(){
    use z2o_native::source_prepare;
    let tmp=tempfile::tempdir().unwrap();let root=tmp.path();let pdf=root.join("main.pdf");fixture(&pdf);
    let other=root.join("other.pdf");fixture(&other);
    for (i,(sources,roles)) in [
        (json!({"Main":pdf,"main":other}),json!({})),
        (json!({"main":pdf,"supplement":pdf}),json!({})),
        (json!({"main":pdf}),json!({"other":"supplement"})),
        (json!({"main":pdf}),json!({"main":"unknown"})),
    ].into_iter().enumerate(){
        let work=root.join(format!("bad-{i}"));
        assert!(source_prepare::prepare_sources(&sources,&work,&roles,None,72,None,|_,_,_|{}).is_err());
        assert!(!work.exists());
    }
}

fn inventory_fixture_docx(path:&Path,text:&str){
    use std::io::Write;
    let mut archive=zip::ZipWriter::new(fs::File::create(path).unwrap());
    let options=zip::write::SimpleFileOptions::default();
    archive.start_file("[Content_Types].xml",options).unwrap();
    archive.write_all(br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#).unwrap();
    archive.start_file("word/document.xml",options).unwrap();
    archive.write_all(text.as_bytes()).unwrap();
    archive.finish().unwrap();
}
const GOOD_DOCX:&str=r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Supplement evidence</w:t></w:r></w:p></w:body></w:document>"#;

fn inventory_cli(work:&Path,request_file:&Path,primary:&Path,supplement:&Path,glossary:&Path)->std::process::Output{
    let req=json!({"schema_version":1,"operation":"prepare_source_inventory.prepare_sources",
        "args":[{"a_main":primary,"z_supplement":supplement},work],
        "kwargs":{"source_roles":{"a_main":"main","z_supplement":"supplement"},"glossary_path":glossary,"dpi":72}});
    fs::write(request_file,serde_json::to_vec(&req).unwrap()).unwrap();
    std::process::Command::new(env!("CARGO_BIN_EXE_z2o")).arg("invoke").arg(request_file).output().unwrap()
}

#[test]
fn cli_preflights_later_docx_before_writing_any_evidence_and_retries(){
    let temp=tempfile::tempdir().unwrap();let dir=temp.path();
    let pdf=dir.join("primary.pdf");fixture(&pdf);
    let docx=dir.join("supplement.docx");inventory_fixture_docx(&docx,"<not-a-word-document>");
    let glossary=dir.join("terms.md");fs::write(&glossary,"wave = 波").unwrap();
    let work=dir.join("not-made");let req=dir.join("request.json");
    let out=inventory_cli(&work,&req,&pdf,&docx,&glossary);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("z_supplement: accessible DOCX required"));
    assert!(!work.exists(),"failed input preflight must not create the work tree");
    inventory_fixture_docx(&docx,GOOD_DOCX);
    let out=inventory_cli(&work,&req,&pdf,&docx,&glossary);
    assert!(out.status.success(),"{}",String::from_utf8_lossy(&out.stderr));
    let result:serde_json::Value=serde_json::from_slice(&out.stdout).unwrap();
    let inventory_path=Path::new(result["result"].as_str().unwrap());
    let inv=z2o_native::pipeline::read_json(inventory_path).unwrap();
    assert_eq!(inv["sources"][0]["role"],"main");
    assert_eq!(inv["sources"][0]["page_count"],2);
    assert_eq!(inv["sources"][1]["role"],"supplement");
    assert_eq!(inv["sources"][1]["native_blocks"][0]["text"],"Supplement evidence");
    assert_eq!(inv["units"],json!([]));
    assert_eq!(inv["sources"][0]["header_fields"]["title"]["status"],"pending");
    let ledger_path=Path::new(inv["terminology_path"].as_str().unwrap());
    // std canonicalization returns a verbatim Windows path; the inventory
    // intentionally emits the ordinary DOS spelling when either is usable.
    assert_eq!(work.join("terminology.json").canonicalize().unwrap(),ledger_path.canonicalize().unwrap());
    let ledger=z2o_native::pipeline::read_json(ledger_path).unwrap();
    assert_eq!(ledger["review_status"],"pending");
    assert_eq!(ledger["glossary"]["sha256"],files::digest("wave = 波".as_bytes()));
    let baseline=fs::read(inventory_path).unwrap();
    let rerun=inventory_cli(&work,&req,&pdf,&docx,&glossary);
    assert!(!rerun.status.success());
    assert_eq!(fs::read(inventory_path).unwrap(),baseline);
}

#[test]
fn cli_checks_later_pdf_geometry_without_writing_earlier_docx(){
    let temp=tempfile::tempdir().unwrap();let dir=temp.path();
    let docx=dir.join("first.docx");inventory_fixture_docx(&docx,GOOD_DOCX);
    let pdf=dir.join("oversized.pdf");fixture(&pdf);
    let mut document=Document::load(&pdf).unwrap();
    let page=*document.get_pages().get(&2).unwrap();
    document.get_object_mut(page).unwrap().as_dict_mut().unwrap()
        .set("MediaBox",vec![Object::Integer(0),Object::Integer(0),Object::Integer(60_000),Object::Integer(60_000)]);
    document.save(&pdf).unwrap();
    let glossary=dir.join("terms.md");fs::write(&glossary,"wave = 波").unwrap();
    let work=dir.join("not-made");let req=dir.join("request.json");
    let out=inventory_cli(&work,&req,&docx,&pdf,&glossary);
    assert!(!out.status.success());
    let error=String::from_utf8_lossy(&out.stderr);
    assert!(error.contains("z_supplement: PDF page raster preflight failed")&&error.contains("PDF page 2"),"{error}");
    assert!(!work.exists(),"raster geometry preflight must not create the work tree");
}

#[test]
fn preflight_binding_prevents_a_changed_later_attachment_from_emitting_evidence(){
    use z2o_native::source_prepare::prepare_sources;
    let temp=tempfile::tempdir().unwrap();let dir=temp.path();
    let pdf=dir.join("first.pdf");fixture(&pdf);
    let docx=dir.join("second.docx");inventory_fixture_docx(&docx,GOOD_DOCX);
    let work=dir.join("work");
    let mut changed=false;
    let result=prepare_sources(&json!({"a_main":pdf,"z_supplement":docx}),&work,&json!(null),None,72,None,|_,_,label|{
        if !changed && label=="PDF/DOCX sources"{
            inventory_fixture_docx(&docx,&GOOD_DOCX.replace("Supplement evidence","New attachment bytes"));
            changed=true;
        }
    });
    let error=format!("{:#}",result.unwrap_err());
    assert!(changed&&error.contains("z_supplement: source evidence preparation failed")&&error.contains("DOCX source changed since selection"),"{error}");
    assert!(!work.join("source-evidence/z_supplement").exists());
    assert!(!work.join("source-inventory.json").exists());
}

#[test]
fn cooperative_source_inventory_writers_do_not_share_an_incomplete_work_tree(){
    use z2o_native::source_prepare::prepare_sources;
    let tmp=tempfile::tempdir().unwrap();let root=tmp.path();
    let source=root.join("first.docx");inventory_fixture_docx(&source,GOOD_DOCX);
    let work=root.join("work");let sources=json!({"main":source});
    let mut checked=false;
    let inventory=prepare_sources(&sources,&work,&json!(null),None,72,None,|_,_,label|{
        if !checked&&label=="main native block 1"{
            let contender=prepare_sources(&sources,&work,&json!(null),None,72,None, |_,_,_|{});
            let error=format!("{:#}",contender.unwrap_err());
            assert!(error.contains("Another native process is updating this ledger"),"{error}");
            assert!(!work.join("source-inventory.json").exists());
            checked=true;
        }
    }).unwrap();
    assert!(checked&&inventory.is_file());
}

#[test]
fn legacy_pdf_selector_crosswalk_replays_actual_source_without_importing_review() {
    use z2o_native::{invoke,pipeline,source_prepare};
    let tmp=tempfile::tempdir().unwrap();
    let root=tmp.path();
    let pdf=root.join("source.pdf");
    fixture(&pdf);
    let native=source_prepare::prepare_sources(
        &json!({"main":pdf}),&root.join("native"),
        &json!({"main":"main"}),None,72,None,|_,_,_|{}).unwrap();
    let ni=pipeline::read_json(&native).unwrap();
    let local=&ni["sources"][0];
    let legacy=root.join("historical-inventory.json");
    // Construct old-format selectors from known fixture text, not an
    // assertion that native grouping equals a real legacy export.
    let mut old=json!({"schema_version":1,"sources":[{
        "id":"main","role":"main","sha256":local["sha256"],"page_count":2,"pages":[
            {"page":1,"image_index":local["pages"][0]["image_index"],"blocks":[
                {"id":"main-p1-b0","text":"Native PDF\n","bbox":[8,0,150,35]},
                {"id":"main-p1-b1","text":"evidence","bbox":[8,0,150,35]}]},
            {"page":2,"image_index":local["pages"][1]["image_index"],"blocks":[
                {"id":"main-p2-b0","text":"Native PDF evidence altered","bbox":[8,0,150,35]}]}]
    }],"human_review":{"status":"passed","reason":"historic record must not be transferred"}});
    fs::write(&legacy,serde_json::to_vec(&old).unwrap()).unwrap();
    let initial=fs::read(&legacy).unwrap();
    let new_before=fs::read(&native).unwrap();
    let mut request=json!({"schema_version":1,"operation":"validate_delivery.crosswalk_legacy_pdf_text",
        "args":[legacy,native],"kwargs":{"source_id":"main",
            "expected_legacy_inventory_sha256":files::digest(&initial),
            "expected_native_inventory_sha256":files::digest(&new_before)}});
    let result=invoke(request.clone()).unwrap();
    let result=&result["result"];
    assert_eq!(result["kind"],"pdf-legacy-text-crosswalk/v1");
    assert_eq!(result["pages"][0]["text_status"],"mechanical_text_correspondence");
    assert_eq!(result["pages"][0]["groups"][0]["old_block_ids"],json!(["main-p1-b0","main-p1-b1"]));
    assert_eq!(result["pages"][0]["groups"][0]["page_token_range"],json!([0,3]));
    assert_eq!(result["pages"][1]["text_status"],"pending");
    assert_eq!(result["pages"][0]["human_review"],"not_transferred");
    assert_eq!(result["pages"][0]["visual"],"pending");
    assert_eq!(result["delivery_ready"],false);
    assert_eq!(fs::read(&legacy).unwrap(),initial);
    assert_eq!(fs::read(&native).unwrap(),new_before);
    old["sources"][0]["pages"][0]["blocks"][1]["text"]=json!("evidence!");
    let changed=serde_json::to_vec(&old).unwrap();
    fs::write(&legacy,&changed).unwrap();
    assert!(invoke(request.clone()).unwrap_err().to_string().contains("inventory hash mismatch"));
    request["kwargs"]["expected_legacy_inventory_sha256"]=json!(files::digest(&changed));
    assert_eq!(invoke(request.clone()).unwrap()["result"]["pages"][0]["text_status"],"pending");
    old["sources"][0]["sha256"]=json!("0".repeat(64));
    let changed=serde_json::to_vec(&old).unwrap();
    fs::write(&legacy,&changed).unwrap();
    request["kwargs"]["expected_legacy_inventory_sha256"]=json!(files::digest(&changed));
    assert!(invoke(request).unwrap_err().to_string().contains("source hash or role differs"));
}

#[test]
fn legacy_pdf_image_correspondence_requires_encoded_identity_and_plain_unique_invocation() {
    use z2o_native::{invoke, pipeline, source_prepare};
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let pdf = root.join("source.pdf");
    let mut doc = Document::with_version("1.5");
    let pages = doc.new_object_id();
    let encoded = vec![20, 30, 40, 50];
    let img = doc.add_object(Stream::new(
        dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>2,"Height"=>2,
            "BitsPerComponent"=>8,"ColorSpace"=>"DeviceGray"},
        encoded.clone(),
    ));
    let content = doc.add_object(Stream::new(
        dictionary!{}, b"q 20 0 0 20 10 10 cm /Img Do Q".to_vec(),
    ));
    let page = doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>content,
        "Resources"=>dictionary!{"XObject"=>dictionary!{"Img"=>img}}});
    doc.objects.insert(pages, Object::Dictionary(dictionary!{"Type"=>"Pages",
        "Kids"=>vec![Object::Reference(page)],"Count"=>1,
        "MediaBox"=>vec![0.into(),0.into(),100.into(),100.into()]}));
    let catalog = doc.add_object(dictionary!{"Type"=>"Catalog","Pages"=>pages});
    doc.trailer.set("Root", catalog);
    doc.save(&pdf).unwrap();
    let native = source_prepare::prepare_sources(
        &json!({"main":pdf}), &root.join("native"), &json!({"main":"main"}),
        None, 72, None, |_, _, _| {},
    ).unwrap();
    let v = pipeline::read_json(&native).unwrap();
    let source = &v["sources"][0];
    // Known fixture's historical-format fields, independent of native output
    // except for the common source binding. Standalone extraction hash differs.
    let old = json!({"schema_version":1,"sources":[{
        "id":"main","role":"main","sha256":source["sha256"],"page_count":1,
        "image_objects":[{
            "image_xref":img.0,"status":"extracted","pdf_stream_sha256":files::digest(&encoded),
            "extracted_sha256":files::digest(b"standalone PNG, not the PDF stream"),
            "smask":{"image_xref":0,"status":"absent"},"image_mask":false,
            "decode":null,"explicit_mask":null,"smask_in_data":null
        }],
        "pages":[{"page":1,"blocks":[],"image_index":{
            "status":"indexed","issues":[],"skipped_forms":[],
            "bbox_meaning":"full_object_bounds_before_clipping",
            "coordinate_system":{"space":"pdf_page","unit":"pt","origin":"top_left",
                "x_axis":"right","y_axis":"down","rotation":"unrotated"},
            "page_rotation":0,"page_bounds":[0,0,100,100],
            "pdf_to_page_transform":[1,0,0,-1,0,100],
            "occurrences":[{
                "occurrence":1,"image_xref":img.0,"generation":0,"resource_name":"/Img",
                "resource_path":["/Img"],"container_xref":page.0,"operation_index":2,
                "form_xrefs":[],"form_clips":[],"context_effects":[],"fully_inside_page":true,
                "bbox":[10,70,30,90],"transform":[20,0,0,20,10,70],
                "visibility":"not_assessed"
            }]
        }}]
    }]});
    let legacy = root.join("old.json");
    let run = |record: &serde_json::Value| {
        let bytes = serde_json::to_vec(record).unwrap();
        fs::write(&legacy, &bytes).unwrap();
        let result = invoke(json!({"schema_version":1,
            "operation":"validate_delivery.crosswalk_legacy_pdf_text","args":[legacy,native],
            "kwargs":{"source_id":"main","expected_legacy_inventory_sha256":files::digest(&bytes),
                "expected_native_inventory_sha256":files::digest(&fs::read(&native).unwrap())}})).unwrap();
        assert_eq!(fs::read(&legacy).unwrap(), bytes);
        assert_eq!(result["result"]["delivery_ready"], false);
        assert_eq!(result["result"]["source_checks_complete"], false);
        assert_eq!(result["result"]["pages"][0]["image_correspondence"]["visual"], "pending");
        assert_eq!(result["result"]["pages"][0]["image_correspondence"]["human_review"], "not_transferred");
        result["result"]["pages"][0]["image_correspondence"].clone()
    };
    let same = run(&old);
    assert_eq!(same["status"], "mechanical_image_correspondence", "{same}");
    assert_eq!(same["rows"][0]["pdf_raw_stream_sha256"], files::digest(&encoded));
    for (key, replacement, reason) in [
        ("generation", json!(1), "image_generation_differs_or_is_ambiguous"),
        ("generation", serde_json::Value::Null, "invocation_generation_missing"),
        ("transform", json!([20,0,0,20,11,70]), "normalized_locator_differs"),
        ("operation_index", json!(3), "normalized_locator_differs"),
        ("context_effects", json!(["optional_content_visibility"]), "image_dependencies_or_display_context_require_review"),
    ] {
        let mut altered = old.clone();
        altered["sources"][0]["pages"][0]["image_index"]["occurrences"][0][key] = replacement;
        let row = run(&altered);
        assert_eq!(row["status"], "pending");
        assert_eq!(row["rows"][0]["reason"], reason);
    }
    let mut changed = old.clone();
    let mut duplicate = changed["sources"][0]["pages"][0]["image_index"]["occurrences"][0].clone();
    duplicate["occurrence"] = json!(2);
    changed["sources"][0]["pages"][0]["image_index"]["occurrences"].as_array_mut().unwrap().push(duplicate);
    assert_eq!(run(&changed)["rows"][0]["reason"], "invocation_missing_repeated_or_ambiguous");

    changed = old.clone();
    let obj = &mut changed["sources"][0]["image_objects"][0];
    obj["pdf_stream_sha256"] = obj["extracted_sha256"].clone();
    assert_eq!(run(&changed)["rows"][0]["reason"], "raw_stream_identity_differs_or_is_unavailable");
    changed["sources"][0]["image_objects"][0]["pdf_stream_sha256"] = serde_json::Value::Null;
    assert_eq!(run(&changed)["rows"][0]["reason"], "legacy_raw_stream_hash_missing");
    changed = old.clone();
    changed["sources"][0]["image_objects"][0].as_object_mut().unwrap().remove("explicit_mask");
    assert_eq!(run(&changed)["rows"][0]["reason"], "image_dependencies_or_display_context_require_review");
    changed = old.clone();
    changed["sources"][0]["pages"][0]["image_index"]["status"] = json!("partial");
    assert_eq!(run(&changed)["reason"], "page_image_index_missing_or_incomplete");
}

#[test]
fn same_resolution_explicit_mask_is_lossless_bound_and_replayed(){
    use z2o_native::{files,pdf_inventory,pdf_structure::export_image_object,pipeline,source_prepare};
    let temporary=tempfile::tempdir().unwrap();
    let root=temporary.path();
    let pdf=root.join("explicit.pdf");
    let mut doc=Document::with_version("1.5");
    let pages=doc.new_object_id();
    // Padding noise must not turn into samples in either row.
    let bits=[0b0101_1111,0b1010_0001];
    let encoded=craft_pdf::filters::encode_flate(&bits);
    let mask=doc.add_object(Stream::new(dictionary!{
        "Type"=>"XObject","Subtype"=>"Image","Width"=>3,"Height"=>2,"ImageMask"=>true,
        "BitsPerComponent"=>1,"Decode"=>vec![1.into(),0.into()],"Filter"=>"FlateDecode"
    },encoded.clone()));
    let pixels=[20u8,100,200, 200,30,80, 224,2,47, 9,15,203, 23,156,1, 231,5,2];
    let item=doc.add_object(Stream::new(dictionary!{
        "Type"=>"XObject","Subtype"=>"Image","Width"=>3,"Height"=>2,"ColorSpace"=>"DeviceRGB",
        "BitsPerComponent"=>8,"Mask"=>mask
    },pixels.to_vec()));
    let body=doc.add_object(Stream::new(dictionary!{},b"q 30 0 0 20 10 10 cm /Image Do Q".to_vec()));
    let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>body,
        "Resources"=>dictionary!{"XObject"=>dictionary!{"Image"=>item}}});
    doc.objects.insert(pages,Object::Dictionary(dictionary!{"Type"=>"Pages","Kids"=>vec![Object::Reference(page)],
        "Count"=>1,"MediaBox"=>vec![0.into(),0.into(),100.into(),100.into()]}));
    let catalog=doc.add_object(dictionary!{"Type"=>"Catalog","Pages"=>pages});
    doc.trailer.set("Root",catalog);
    doc.save(&pdf).unwrap();
    let frozen=fs::read(&pdf).unwrap();
    let output=root.join("asset.png");
    let r=export_image_object(&pdf,1,item.0,&output,Some(&files::digest(&frozen))).unwrap();
    assert_eq!(r["method"],"explicit_mask_composited_png");
    let record=&r["source_object"]["explicit_mask"];
    assert_eq!(record["image_xref"],mask.0);
    assert_eq!(record["generation"],mask.1);
    assert_eq!(record["painted_sample"],1);
    assert_eq!(record["pdf_stream_sha256"],files::digest(&encoded));
    let image=image::open(output).unwrap().into_rgba8();
    let alpha=[0u8,255,0,255,0,255];
    assert_eq!(record["decoded_alpha_sha256"],files::digest(&alpha));
    for ((actual,rgb),alpha) in image.pixels().zip(pixels.chunks_exact(3)).zip(alpha){
        assert_eq!(&actual.0[..3],rgb);
        assert_eq!(actual.0[3],alpha);
    }
    assert_eq!(fs::read(&pdf).unwrap(),frozen);
    let inv=source_prepare::prepare_sources(&json!({"main":pdf}),&root.join("work"),
        &json!({"main":"main"}),None,72,None,|_,_,_|{}).unwrap();
    let inventory=pipeline::read_json(&inv).unwrap();
    let source=&inventory["sources"][0];
    let row=&source["image_objects"][0];
    assert_eq!(row["mask_stream"]["encoded_stream_sha256"],files::digest(&encoded));
    assert_eq!(row["extraction"]["source_object"]["explicit_mask"],*record);
    let replay=pdf_inventory::replay(source,&pdf,|_,_,_|{}).unwrap();
    assert_eq!(replay["image_dependency_replay_complete"],true);
    assert_eq!(replay["independent_visual_parity"],"not_assessed");

    let mut old=source.clone();
    let historical=&mut old["image_objects"][0];
    historical.as_object_mut().unwrap().remove("raw_asset");
    historical["extraction"]=json!({"status":"page_render_required","reason":"PAGE_RENDER_REQUIRED: Mask"});
    let old_replay=pdf_inventory::replay(&old,&pdf,|_,_,_|{}).unwrap();
    assert!(old_replay["pending_review"].as_array().unwrap().iter().any(|r|r["image_xref"]==item.0));
    assert_eq!(old_replay["source_checks_complete"],false);

    fs::write(row["mask_stream"]["asset"]["path"].as_str().unwrap(),b"tampered").unwrap();
    assert!(pdf_inventory::replay(source,&pdf,|_,_,_|{}).is_err());
}
