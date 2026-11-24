use serde_json::json;
use std::{fs,path::Path};
use z2o_native::{ocr_backend,files};
fn fixture(root:&Path)->(std::path::PathBuf,String){
    let record=|name:&str| {
        let p=root.join(name);fs::write(&p,name.as_bytes()).unwrap();
        json!({"path":name,"sha256":files::digest(name.as_bytes())})
    };
    let payload=json!({"schema_version":1,"engine":"ppocr-v6",
        "runtime":{"platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,"version":"1.28.2","file":record("runtime.bin")},
        "profiles":{"tiny":{"detector":record("det.onnx"),"recognizer":record("rec.onnx"),"dictionary":record("dict.txt")}}});
    let manifest=root.join("package.json");let raw=serde_json::to_vec_pretty(&payload).unwrap();
    fs::write(&manifest,&raw).unwrap();(manifest,files::digest(&raw))
}
#[test]
fn inspect_without_loading_gpu_or_runtime(){
    let root=tempfile::tempdir().unwrap();let (manifest,hash)=fixture(root.path());
    let p=ocr_backend::inspect_package(&manifest,&hash,"tiny","off").unwrap();
    assert_eq!(p["hardware"]["gpu_required"],false);assert_eq!(p["hardware"]["execution_provider"],"CPUExecutionProvider");
    assert_eq!(p["qualified"],false);assert_eq!(p["resources"].as_object().unwrap().len(),4);
    assert!(ocr_backend::inspect_package(&manifest,&"0".repeat(64),"tiny","off").is_err());
    fs::write(root.path().join("det.onnx"),b"changed").unwrap();
    assert!(ocr_backend::inspect_package(&manifest,&hash,"tiny","off").is_err());
}
#[test]
fn resource_escape_and_architecture_mismatch_block_before_dlopen(){
    let root=tempfile::tempdir().unwrap();let (manifest,_)=fixture(root.path());
    let original:serde_json::Value=serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    for (key,value) in [("architecture",json!("wrong-arch")),("version",json!("1.27.0"))]{
        let mut changed=original.clone();changed["runtime"][key]=value;
        let raw=serde_json::to_vec(&changed).unwrap();fs::write(&manifest,&raw).unwrap();
        assert!(ocr_backend::inspect_package(&manifest,&files::digest(&raw),"tiny","off").is_err());
    }
    let mut changed=original;changed["profiles"]["tiny"]["detector"]["path"]=json!("../escape.onnx");
    let raw=serde_json::to_vec(&changed).unwrap();fs::write(&manifest,&raw).unwrap();
    assert!(ocr_backend::inspect_package(&manifest,&files::digest(&raw),"tiny","off").is_err());
}
#[test]
fn invalid_runtime_is_reported_not_required_for_non_ocr_operations(){
    let root=tempfile::tempdir().unwrap();let (manifest,hash)=fixture(root.path());
    let package=ocr_backend::Package::load(&manifest,&hash,"tiny","off").unwrap();
    assert!(ocr_backend::Engine::open(package).is_err());
    let description=z2o_native::describe();
    assert_eq!(description["system_python_required"],false);
    assert_eq!(description["ocr_backend"]["gpu_required"],false);
    assert_eq!(description["full_workflow_ready"],false);
}

#[test]
fn native_and_scanned_pdf_pages_resume_without_runtime(){
    use lopdf::{Document,Object,Stream,dictionary};
    use z2o_native::{ocr_pdf,pipeline};
    let root=tempfile::tempdir().unwrap();let pdf=root.path().join("mixed.pdf");
    let mut doc=Document::with_version("1.5");let pages=doc.new_object_id();
    let font=doc.add_object(dictionary!{"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
    let contents=doc.add_object(Stream::new(dictionary!{},b"BT /F1 16 Tf 1 0 0 1 10 60 Tm (Native text) Tj ET".to_vec()));
    let p1=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>contents,"Resources"=>dictionary!{"Font"=>dictionary!{"F1"=>font}}});
    let img=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>1,"Height"=>1,"BitsPerComponent"=>8,"ColorSpace"=>"DeviceRGB"},vec![180,40,40]));
    let scan=doc.add_object(Stream::new(dictionary!{},b"q 200 0 0 100 0 0 cm /Scan Do Q".to_vec()));
    let p2=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>scan,"Resources"=>dictionary!{"XObject"=>dictionary!{"Scan"=>img}}});
    doc.objects.insert(pages,Object::Dictionary(dictionary!{"Type"=>"Pages","Kids"=>vec![Object::Reference(p1),Object::Reference(p2)],"Count"=>2,"MediaBox"=>vec![0.into(),0.into(),200.into(),100.into()]}));
    let catalog=doc.add_object(dictionary!{"Type"=>"Catalog","Pages"=>pages});doc.trailer.set("Root",catalog);doc.save(&pdf).unwrap();
    let dest=root.path().join("ocr-work");let regions=json!({});let coord=serde_json::Value::Null;
    let run=||ocr_pdf::prepare(&pdf,&dest,ocr_pdf::Options{package_manifest:None,package_sha:None,
        profile:"auto",layout:"auto",mode:"auto",dpi:72,expected_source_sha:None,
        regions:&regions,coordinate_system:&coord},|_,_,_|{});
    let a=pipeline::read_json(&run().unwrap()).unwrap();
    assert_eq!(a["pages"][0]["status"],"native_text_candidate");
    assert_eq!(a["pages"][1]["status"],"blocked");
    assert_eq!(a["blocked_page_count"],1);assert_eq!(a["source_inventory_ready"],false);
    let b=pipeline::read_json(&run().unwrap()).unwrap();
    assert_eq!(b["pages"][0]["record"],a["pages"][0]["record"]);
    assert_eq!(b["pages"][0]["resumed"],true);
    assert_ne!(b["pages"][1]["record"],a["pages"][1]["record"]);
    let record=pipeline::read_json(Path::new(a["pages"][0]["record"]["path"].as_str().unwrap())).unwrap();
    let evidence=Path::new(record["native_text"]["path"].as_str().unwrap());
    fs::write(evidence,b"tampered").unwrap();
    assert!(run().is_err());
}
