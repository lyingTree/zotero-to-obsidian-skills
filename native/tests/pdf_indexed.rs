use lopdf::{dictionary,Dictionary,Document,Object,ObjectId,Stream,StringFormat};
use serde_json::{json,Value};
use std::{fs,path::Path};
use z2o_native::{files,invoke,pdf_inventory,pipeline,source_prepare};

fn stream(doc:&mut Document,dict:Dictionary,bytes:Vec<u8>)->ObjectId{
    doc.add_object(Stream::new(dict,bytes))
}
fn indexed(base:&str,hival:i64,lookup:Object)->Object{
    Object::Array(vec![Object::Name(b"Indexed".to_vec()),Object::Name(base.as_bytes().to_vec()),hival.into(),lookup])
}
fn image(width:i32,height:i32,bits:i32,space:Object)->Dictionary{
    dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>width,"Height"=>height,"BitsPerComponent"=>bits,"ColorSpace"=>space}
}
fn write_pdf(mut doc:Document,path:&Path,objects:&[(&str,ObjectId)],ops:&str){
    let pages=doc.new_object_id();
    let mut resources=Dictionary::new();
    for (key,id) in objects{resources.set(*key,Object::Reference(*id));}
    let contents=stream(&mut doc,Dictionary::new(),ops.as_bytes().to_vec());
    let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>contents,"Resources"=>dictionary!{"XObject"=>resources}});
    doc.objects.insert(pages,Object::Dictionary(dictionary!{"Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1,
        "MediaBox"=>vec![0.into(),0.into(),100.into(),100.into()]}));
    let catalog=doc.add_object(dictionary!{"Type"=>"Catalog","Pages"=>pages});doc.trailer.set("Root",catalog);doc.save(path).unwrap();
}
fn export(pdf:&Path,xref:u32,output:&Path)->anyhow::Result<Value>{
    Ok(invoke(json!({"schema_version":1,"operation":"pdf_image_objects.export_image_object",
        "args":[pdf],"kwargs":{"page_number":1,"image_xref":xref,"destination":output}}))?["result"].clone())
}

#[test]
fn lossless_indexed_flate_exports_exact_packed_rgb_and_provenance(){
    let temp=tempfile::tempdir().unwrap();let root=temp.path();let pdf=root.join("packed-indexed.pdf");
    let mut doc=Document::with_version("1.5");
    let rgb=vec![10,20,30,40,50,60,70,80,90];
    let compressed_palette=craft_pdf::filters::encode_flate(&rgb);
    let palette=stream(&mut doc,dictionary!{"Filter"=>"FlateDecode"},compressed_palette.clone());
    let color_space=doc.add_object(indexed("DeviceRGB",2,Object::Reference(palette)));
    let mut d=image(3,2,2,Object::Reference(color_space));d.set("Filter","FlateDecode");
    // Rows represent [0, 1, 2] and [3, 2, 1]; nonzero padding is ignored.
    let raw=[0b00011011,0b11100111];
    let obj=stream(&mut doc,d,craft_pdf::filters::encode_flate(&raw));
    write_pdf(doc,&pdf,&[("Image",obj)],"q 30 0 0 20 10 10 cm /Image Do Q");
    let original=fs::read(&pdf).unwrap();

    let png=root.join("rgb.png");let record=export(&pdf,obj.0,&png).unwrap();
    let actual=image::open(&png).unwrap().into_rgb8();
    // Index 3 clips to 2, while index 2 still selects entry 2; mapping the
    // entire sample domain to hival would incorrectly make index 2 select 1.
    let expected=[10,20,30,40,50,60,70,80,90,70,80,90,70,80,90,40,50,60];
    assert_eq!(actual.dimensions(),(3,2));assert_eq!(actual.as_raw(),&expected);
    let object=&record["source_object"];
    assert_eq!(record["source_fidelity"],"not_assessed");
    assert_eq!(record["visual_review"]["status"],"pending");
    assert_eq!(object["pdf_colorspace"],"/Indexed");
    assert_eq!(object["bits_per_component"],2);
    assert_eq!(object["decoded_samples_sha256"],files::digest(&expected));
    assert_eq!(object["indexed_palette"],json!({"base":"/DeviceRGB","hival":2,"decode":[0.,3.],
        "lookup_format":"stream","lookup_sha256":files::digest(&rgb),"encoded_lookup_stream_sha256":files::digest(&compressed_palette)}));

    let inventory=source_prepare::prepare_sources(&json!({"main":pdf}),&root.join("inventory"),&json!({"main":"main"}),
        None,72,None,|_,_,_|{}).unwrap();
    let data=pipeline::read_json(&inventory).unwrap();
    let source=&data["sources"][0];
    assert_eq!(source["image_objects"][0]["extraction"]["source_object"],*object);
    assert_eq!(pdf_inventory::replay(source,&pdf,|_,_,_|{}).unwrap()["same_backend_replay_complete"],true);
    let mut changed=source.clone();
    changed["image_objects"][0]["extraction"]["source_object"]["indexed_palette"]["lookup_sha256"]=json!("0".repeat(64));
    assert!(pdf_inventory::replay(&changed,&pdf,|_,_,_|{}).is_err());
    assert_eq!(fs::read(&pdf).unwrap(),original);
}

#[test]
fn indexed_gray_decode_rounds_and_applies_independent_soft_mask(){
    let tmp=tempfile::tempdir().unwrap();let root=tmp.path();let pdf=root.join("gray-indexed.pdf");
    let mut doc=Document::with_version("1.5");
    let mask=stream(&mut doc,image(4,1,8,Object::Name(b"DeviceGray".to_vec())),vec![255,192,127,0]);
    let mut d=image(4,1,2,indexed("DeviceGray",2,Object::String(vec![10,100,200],StringFormat::Hexadecimal)));
    // Affine Decode yields 2.5, 1.5, 0.5 and -0.5 from samples 0-3.
    // Round to nearest then clip to 0-2: indices 2, 2, 1, 0.
    d.set("Decode",vec![Object::Real(2.5),Object::Real(-0.5)]);d.set("SMask",Object::Reference(mask));
    let obj=stream(&mut doc,d,vec![0b00011011]);
    write_pdf(doc,&pdf,&[("Gray",obj)],"q 40 0 0 10 10 10 cm /Gray Do Q");
    let png=root.join("gray.png");let report=export(&pdf,obj.0,&png).unwrap();
    assert_eq!(report["method"],"smask_composited_png");
    assert_eq!(report["source_object"]["indexed_palette"]["lookup_format"],"string");
    assert_eq!(report["source_object"]["smask"]["image_xref"],mask.0);
    assert_eq!(image::open(&png).unwrap().into_luma_alpha8().as_raw(),&[200,255,200,192,100,127,10,0]);
}

#[test]
fn malformed_indexed_palettes_and_unsupported_display_never_publish_an_asset(){
    let tmp=tempfile::tempdir().unwrap();let root=tmp.path();let pdf=root.join("bad-indexed.pdf");
    let mut doc=Document::with_version("1.5");
    let table=||Object::String(vec![10,20,30,40,50,60],StringFormat::Hexadecimal);
    let short=indexed("DeviceRGB",2,table());
    let cmyk=indexed("DeviceCMYK",1,table());
    let oversized=indexed("DeviceRGB",256,table());
    let mut d=image(1,1,8,indexed("DeviceRGB",1,table()));d.set("Filter","DCTDecode");
    let jpeg=stream(&mut doc,d,vec![0]);
    let mut d=image(1,1,8,indexed("DeviceRGB",1,table()));d.set("Decode",vec![0.into(),1.into(),2.into()]);
    let decode=stream(&mut doc,d,vec![0]);
    let bad_mask=stream(&mut doc,image(1,1,8,indexed("DeviceGray",1,Object::String(vec![100,200],StringFormat::Hexadecimal))),vec![0]);
    let mut d=image(1,1,8,indexed("DeviceRGB",1,table()));d.set("SMask",Object::Reference(bad_mask));
    let mask=stream(&mut doc,d,vec![0]);
    let objects=[
        ("Short",stream(&mut doc,image(1,1,8,short),vec![0])),
        ("Cmyk",stream(&mut doc,image(1,1,8,cmyk),vec![0])),
        ("Oversized",stream(&mut doc,image(1,1,8,oversized),vec![0])),
        ("Jpeg",jpeg),("Decode",decode),("Mask",mask),
    ];
    write_pdf(doc,&pdf,&objects,"q 1 0 0 1 1 1 cm /Short Do /Cmyk Do /Oversized Do /Jpeg Do /Decode Do /Mask Do Q");
    for ((name,id),reason) in objects.into_iter().zip(["lookup length","Indexed base","Indexed hival","with DCT","Indexed Decode","non-gray SMask"]){
        let out=root.join(format!("{name}.png"));let error=export(&pdf,id.0,&out).unwrap_err();
        assert!(format!("{error:#}").contains(reason),"{name}: {error:#}");assert!(!out.exists());
    }
}
