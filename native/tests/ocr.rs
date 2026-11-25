use serde_json::json;
use std::{fs,path::Path};
use z2o_native::{ocr_backend,files};
fn fixture(root:&Path)->(std::path::PathBuf,String){
    let record=|name:&str| {
        let p=root.join(name);fs::write(&p,name.as_bytes()).unwrap();
        json!({"path":name,"sha256":files::digest(name.as_bytes())})
    };
    let payload=json!({"schema_version":1,"engine":"ppocr-v6",
        "runtime":{"platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,"version":"1.32.0","file":record("runtime.bin")},
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
    assert_eq!(description["full_workflow_ready"],true);
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


#[test]
#[ignore = "requires independently checked official CPU runtime and real PP-OCRv6 models; mandatory in dedicated CI job"]
fn actual_cpu_ocr_scan_with_verified_package(){
    use z2o_native::pipeline;
    let root=std::path::PathBuf::from(std::env::var_os("Z2O_OCR_FIXTURE_DIR").expect("actual CI fixture required"));
    let manifest=root.join("package.json");
    let digest=fs::read_to_string(root.join("package.sha256")).unwrap();
    let digest=digest.trim();
    assert_eq!(files::digest(&fs::read(&manifest).unwrap()),digest);
    let source=root.join("fixture.pdf");
    let work=root.join("ocr-evidence");
    let request=json!({"schema_version":1,"operation":"prepare_source_inventory.prepare_pdf_ocr_evidence",
        "args":[source],"kwargs":{"destination":work,"package_manifest":manifest,
            "expected_package_sha256":digest,"profile":"tiny","layout":"off","dpi":144,"mode":"auto"}});
    let req=root.join("request.json");fs::write(&req,serde_json::to_vec_pretty(&request).unwrap()).unwrap();
    // Invoke the actual binary with no Python, CUDA or other executable on
    // PATH; the tested inference backend is loaded by its absolute path.
    let output=std::process::Command::new(env!("CARGO_BIN_EXE_z2o"))
        .arg("invoke").arg(&req).env("PATH","").env("CUDA_VISIBLE_DEVICES","").env("HIP_VISIBLE_DEVICES","")
        .output().unwrap();
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    let answer:serde_json::Value=serde_json::from_slice(&output.stdout).unwrap();
    let report=pipeline::read_json(Path::new(answer["result"].as_str().unwrap())).unwrap();
    assert_eq!(report["blocked_page_count"],0,"{report}");
    assert_eq!(report["source_inventory_ready"],false);
    assert_eq!(report["pages"][0]["status"],"ocr_candidate_review_required");
    let record=pipeline::read_json(Path::new(report["pages"][0]["record"]["path"].as_str().unwrap())).unwrap();
    let result=pipeline::read_json(Path::new(record["ocr_regions"][0]["recognition"]["path"].as_str().unwrap())).unwrap();
    let text=result["plain_text"].as_str().unwrap();
    let normalized:String=text.chars().filter(|c|!c.is_whitespace()).flat_map(char::to_uppercase).collect();
    assert!(normalized.contains("NATIVEOCRTEST12345"),"actual OCR text: {text}");
    assert!(normalized.contains("海洋重力波2026"),"actual OCR text: {text}");
    assert_eq!(result["model_package"]["hardware"]["execution_provider"],"CPUExecutionProvider");
    assert_eq!(result["acceptance_ready"],false);
    let rows=result["blocks"].as_array().unwrap();
    assert!(!rows.is_empty());
    assert!(rows.iter().any(|row|row["unrotated_pdf_bbox"].is_array()),"Missing real mapped source coordinates");
    assert_eq!(files::snapshot(&source).unwrap()["sha256"],report["source_file"]["sha256"]);
    println!("Real CPU OCR hypothesis: {text}");
    // Exercise a genuine failed native worker initialization on every CPU
    // platform, not a mocked success or an assumed GPU device.
    let fake=root.join("invalid-accelerator-runtime.bin");fs::write(&fake,b"not a dynamic library").unwrap();
    let mut gpu:serde_json::Value=serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    gpu["runtime"]["file"]=json!({"path":"invalid-accelerator-runtime.bin","sha256":files::digest(b"not a dynamic library")});
    gpu["runtime"]["execution_provider"]=json!(if cfg!(target_os="windows"){"DmlExecutionProvider"}else if cfg!(target_os="macos"){"CoreMLExecutionProvider"}else{"CUDAExecutionProvider"});
    let gpu_manifest=root.join("gpu-test-invalid.json");let gpu_bytes=serde_json::to_vec(&gpu).unwrap();fs::write(&gpu_manifest,&gpu_bytes).unwrap();
    let raster=root.join("fixture.png");let image_sha=files::snapshot(&raster).unwrap()["sha256"].as_str().unwrap().to_owned();
    let request=json!({"schema_version":1,"operation":"prepare_source_inventory.recognize_ocr_png_accelerated","args":[raster],"kwargs":{
        "expected_image_sha256":image_sha,"package_manifest":manifest,"expected_package_sha256":digest,
        "gpu_package_manifest":gpu_manifest,"expected_gpu_package_sha256":files::digest(&gpu_bytes),"profile":"tiny","layout":"off","acceleration":"auto","timeout_seconds":180}});
    let req=root.join("fallback-request.json");fs::write(&req,serde_json::to_vec(&request).unwrap()).unwrap();
    let output=std::process::Command::new(env!("CARGO_BIN_EXE_z2o")).arg("invoke").arg(&req).env("PATH","").env("CUDA_VISIBLE_DEVICES","").env("HIP_VISIBLE_DEVICES","").output().unwrap();
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    let v:serde_json::Value=serde_json::from_slice(&output.stdout).unwrap();let r=&v["result"];
    assert_eq!(r["cpu_fallback_used"],true,"{r}");
    assert_eq!(r["acceleration_attempts"][0]["status"],"failed_or_unavailable");
    assert_eq!(r["acceleration_attempts"][1]["provider"],"CPUExecutionProvider");
    assert_eq!(r["acceleration_attempts"][1]["status"],"completed");
    let reading:String=r["plain_text"].as_str().unwrap().chars().filter(|c|!c.is_whitespace()).flat_map(char::to_uppercase).collect();
    assert!(reading.contains("NATIVEOCRTEST12345")&&reading.contains("海洋重力波2026"),"{r}");
    fs::write(root.join("fallback-evidence.json"),serde_json::to_vec_pretty(r).unwrap()).unwrap();
    let request=json!({"schema_version":1,"operation":"prepare_source_inventory.prepare_pdf_ocr_evidence","args":[source],"kwargs":{
        "destination":root.join("accelerated-pdf-evidence"),"package_manifest":manifest,"expected_package_sha256":digest,
        "profile":"tiny","layout":"off","dpi":144,"mode":"auto","accelerator":{"acceleration":"auto",
            "gpu_package_manifest":gpu_manifest,"expected_gpu_package_sha256":files::digest(&gpu_bytes),"timeout_seconds":180}}});
    let req=root.join("accelerated-pdf-request.json");fs::write(&req,serde_json::to_vec(&request).unwrap()).unwrap();
    let output=std::process::Command::new(env!("CARGO_BIN_EXE_z2o")).arg("invoke").arg(&req).env("PATH","").output().unwrap();
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    let answer:serde_json::Value=serde_json::from_slice(&output.stdout).unwrap();
    let report=pipeline::read_json(Path::new(answer["result"].as_str().unwrap())).unwrap();
    assert_eq!(report["blocked_page_count"],0,"{report}");
    let record=pipeline::read_json(Path::new(report["pages"][0]["record"]["path"].as_str().unwrap())).unwrap();
    let result=pipeline::read_json(Path::new(record["ocr_regions"][0]["recognition"]["path"].as_str().unwrap())).unwrap();
    assert_eq!(result["cpu_fallback_used"],true);
    assert!(result["blocks"].as_array().unwrap().iter().any(|b|b["unrotated_pdf_bbox"].is_array()));
}


#[test]
fn offline_install_checks_every_profile_and_reuses_exact_package(){
    let root=tempfile::tempdir().unwrap();let source=root.path().join("source");fs::create_dir(&source).unwrap();
    let (manifest,_)=fixture(&source);
    let mut data:serde_json::Value=serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    let medium=source.join("medium.onnx");fs::write(&medium,b"medium fixture").unwrap();
    data["profiles"]["medium"]=data["profiles"]["tiny"].clone();
    data["profiles"]["medium"]["detector"]=json!({"path":"medium.onnx","sha256":files::digest(b"medium fixture")});
    let raw=serde_json::to_vec(&data).unwrap();fs::write(&manifest,&raw).unwrap();
    let digest=files::digest(&raw);let store=root.path().join("store");
    let install=||ocr_backend::install_package(&manifest,&digest,&store,|_,_,_|{});
    let a=install().unwrap();assert_eq!(a["resource_count"],5);assert_eq!(a["runtime_loaded"],false);
    assert_eq!(a["reused_existing"],false);
    let b=install().unwrap();assert_eq!(b["reused_existing"],true);
    assert_eq!(b["package_manifest"],a["package_manifest"]);
    let installed=std::path::PathBuf::from(a["package_root"].as_str().unwrap());
    fs::write(installed.join("unexpected.dll"),b"not in manifest").unwrap();
    assert!(install().is_err());
    fs::remove_file(installed.join("unexpected.dll")).unwrap();
    fs::write(&medium,b"changed").unwrap();
    assert!(install().is_err());
    assert_eq!(fs::read(installed.join("medium.onnx")).unwrap(),b"medium fixture");
}
#[test]
fn offline_install_rejects_bad_unselected_model_before_creating_store(){
    let root=tempfile::tempdir().unwrap();let source=root.path().join("source");fs::create_dir(&source).unwrap();
    let (manifest,_)=fixture(&source);
    let mut data:serde_json::Value=serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    data["profiles"]["medium"]=data["profiles"]["tiny"].clone();
    data["profiles"]["medium"]["detector"]=json!({"path":"../escape.onnx","sha256":"0".repeat(64)});
    let raw=serde_json::to_vec(&data).unwrap();fs::write(&manifest,&raw).unwrap();
    let store=root.path().join("store");
    assert!(ocr_backend::install_package(&manifest,&files::digest(&raw),&store,|_,_,_|{}).is_err());
    assert!(!store.exists());
}

#[test]
fn explicit_table_model_resources_are_checked_before_runtime(){
    let root=tempfile::tempdir().unwrap();
    let (manifest,_)=fixture(root.path());
    let mut data:serde_json::Value=serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    fs::write(root.path().join("table.onnx"),b"table fixture").unwrap();
    fs::write(root.path().join("table.txt"),b"<td>\n</td>\n").unwrap();
    data["auxiliary"]["table_structure"]=json!({"architecture":"SLANet_plus",
        "model":{"path":"table.onnx","sha256":files::digest(b"table fixture")},
        "dictionary":{"path":"table.txt","sha256":files::digest(b"<td>\n</td>\n")}});
    let bytes=serde_json::to_vec(&data).unwrap();fs::write(&manifest,&bytes).unwrap();
    let installed=ocr_backend::install_package(&manifest,&files::digest(&bytes),&root.path().join("store"),|_,_,_|{}).unwrap();
    assert_eq!(installed["resource_count"],6);
    // An ordinary OCR package inspection never loads optional table resources.
    let inspected=ocr_backend::inspect_package(&manifest,&files::digest(&bytes),"tiny","off").unwrap();
    assert_eq!(inspected["resources"].as_object().unwrap().len(),4);
    fs::write(root.path().join("table.onnx"),b"changed").unwrap();
    assert!(ocr_backend::recognize_table_png(&root.path().join("missing.png"),&manifest,&files::digest(&bytes),"tiny","off",None).is_err());
    assert!(ocr_backend::install_package(&manifest,&files::digest(&bytes),&root.path().join("other"),|_,_,_|{}).is_err());
}


#[test]
fn ocr_quality_uses_actual_bound_raster_and_never_grants_acceptance(){
    use z2o_native::{ocr_quality,check_ledger};
    let root=tempfile::tempdir().unwrap();let (manifest,hash)=fixture(root.path());
    let model=ocr_backend::inspect_package(&manifest,&hash,"tiny","off").unwrap();
    let png=root.path().join("raster.png");image::RgbImage::from_pixel(100,100,image::Rgb([255,255,255])).save(&png).unwrap();
    let asset=check_ledger::bind_file(&png).unwrap();
    let block=|index:u32,text:&str,quad:serde_json::Value|json!({
        "index":index,"text":text,"text_score":0.98,"box_score":0.99,
        "location_valid":true,"original_pixel_quad":quad,"words":[]});
    let b0=json!([[10,10],[60,10],[60,20],[10,20]]);
    let b1=json!([[10,30],[60,30],[60,40],[10,40]]);
    let primary=json!({"schema_version":1,"status":"ocr_candidate","source_fidelity":"not_assessed","acceptance_ready":false,
        "source_image":asset,"source_image_size":[100,100],"model_package":model,
        "layout_regions":[{"index":0,"class":"Equation","original_pixel_quad":b1}],
        "blocks":[block(0,"−1.5 m",b0.clone()),block(1,"ocean",b1.clone())],"plain_text":"−1.5 m\nocean"});
    let other=json!({"schema_version":1,"status":"ocr_candidate","source_fidelity":"not_assessed","acceptance_ready":false,
        "source_image":asset,"source_image_size":[100,100],"model_package":model,
        "layout_regions":[],"blocks":[block(0,"-1.5 m",b0),block(1,"ocean",b1)],"plain_text":"-1.5 m\nocean"});
    let p=root.path().join("primary.json");let q=root.path().join("other.json");
    fs::write(&p,serde_json::to_vec(&primary).unwrap()).unwrap();
    fs::write(&q,serde_json::to_vec(&other).unwrap()).unwrap();
    let ph=files::snapshot(&p).unwrap()["sha256"].as_str().unwrap().to_owned();
    let qh=files::snapshot(&q).unwrap()["sha256"].as_str().unwrap().to_owned();
    let out=ocr_quality::audit(&p,&ph,Some((&q,&qh))).unwrap();
    assert_eq!(out["comparison_counts"]["different_text"],1);
    assert_eq!(out["comparison_counts"]["exact_text_agreement"],1);
    assert_eq!(out["same_recognition_assets"],true);
    assert_eq!(out["independence_established"],false);
    assert_eq!(out["acceptance_ready"],false);
    assert_eq!(out["tasks"].as_array().unwrap().len(),3);
    assert!(out["tasks"].as_array().unwrap().iter().all(|r|r["status"]=="pending"));
    assert!(ocr_quality::audit(&p,&ph,Some((&p,&ph))).is_err());
    fs::write(&png,b"changed").unwrap();
    assert!(ocr_quality::audit(&p,&ph,Some((&q,&qh))).is_err());
}

#[test]
fn formula_package_is_opt_in_and_all_assets_are_hash_checked(){
    let root=tempfile::tempdir().unwrap();let (manifest,_)=fixture(root.path());
    let mut data:serde_json::Value=serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    fs::write(root.path().join("formula.onnx"),b"formula fixture").unwrap();
    fs::write(root.path().join("tokenizer.json"),b"tokenizer fixture").unwrap();
    data["auxiliary"]["formula"]=json!({"architecture":"PP-FormulaNet_plus-L",
        "model":{"path":"formula.onnx","sha256":files::digest(b"formula fixture")},
        "tokenizer":{"path":"tokenizer.json","sha256":files::digest(b"tokenizer fixture")}});
    let bytes=serde_json::to_vec(&data).unwrap();fs::write(&manifest,&bytes).unwrap();
    let installed=ocr_backend::install_package(&manifest,&files::digest(&bytes),&root.path().join("store"),|_,_,_|{}).unwrap();
    assert_eq!(installed["resource_count"],6);
    let inspected=ocr_backend::inspect_package(&manifest,&files::digest(&bytes),"tiny","off").unwrap();
    assert_eq!(inspected["resources"].as_object().unwrap().len(),4);
    fs::write(root.path().join("tokenizer.json"),b"changed").unwrap();
    assert!(ocr_backend::install_package(&manifest,&files::digest(&bytes),&root.path().join("other"),|_,_,_|{}).is_err());
}
