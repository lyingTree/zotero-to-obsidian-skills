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
