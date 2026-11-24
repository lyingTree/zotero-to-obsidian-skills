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
