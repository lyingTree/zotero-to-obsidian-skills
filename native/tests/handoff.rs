use lopdf::{Document,Object,Stream,dictionary};
use serde_json::{json,Value};
use std::{fs,path::{Path,PathBuf}};
use z2o_native::{files,handoff_native as maps,mapping_compare,pipeline};
fn pdf(path:&Path){
    let mut d=Document::with_version("1.5");let pages=d.new_object_id();
    let font=d.add_object(dictionary!{"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
    let stream=d.add_object(Stream::new(dictionary!{},b"BT /F1 12 Tf 1 0 0 1 10 760 Tm (Body [1]) Tj 1 0 0 1 10 730 Tm (Reference entry) Tj ET".to_vec()));
    let page=d.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>stream,"Resources"=>dictionary!{"Font"=>dictionary!{"F1"=>font}}});
    d.objects.insert(pages,Object::Dictionary(dictionary!{"Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1,"MediaBox"=>vec![0.into(),0.into(),600.into(),800.into()]}));
    let catalog=d.add_object(dictionary!{"Type"=>"Catalog","Pages"=>pages});d.trailer.set("Root",catalog);d.save(path).unwrap();
}
fn binding(p:&Path)->Value{let b=files::snapshot(p).unwrap();json!({"path":b["path"],"sha256":b["sha256"]})}
fn save(p:&Path,v:&Value){fs::write(p,serde_json::to_vec_pretty(v).unwrap()).unwrap();}
struct Fixture{root:tempfile::TempDir,inventory:PathBuf,candidate:PathBuf,paths:Value,deliveries:Value,expected:Value,run:PathBuf}
impl Fixture{
    fn new()->Self{
        let root=tempfile::tempdir().unwrap();let p=root.path();
        let main=p.join("main.pdf");let si=p.join("supplement.pdf");pdf(&main);pdf(&si);
        let candidate=p.join("candidate.md");
        fs::write(&candidate,"---\r\nkind: test\r\n---\r\n# 海洋\r\n\r\n正文 [1](#^ref-a)\r\n^body\r\n\r\nReference entry ^ref-a\r\n\r\n补充材料 ^si-body\r\n").unwrap();
        let mut a=binding(&main);a["id"]=json!("main");a["role"]=json!("main");
        a["pages"]=json!([{"page":1,"blocks":[{"id":"b1","bbox":[10,10,100,20],"text":"Body [1]"},{"id":"b2","bbox":[10,30,100,40],"text":"Reference entry"}]}]);
        let mut b=binding(&si);b["id"]=json!("si");b["role"]=json!("supplement");
        b["pages"]=json!([{"page":1,"blocks":[{"id":"s1","bbox":[10,10,100,20],"text":"Body [1]"}]}]);
        let loc=|block:&str,text:&str|json!([{"page":1,"blocks":[block],"quote":text}]);
        let inv=json!({"schema_version":1,"sources":[a,b],"units":[
            {"id":"body","kind":"paragraph","source":"main","evidence":loc("b1","Body")},
            {"id":"call","kind":"citation","source":"main","references":["reference"],"display_text":"[1]","evidence":loc("b1","[1]")},
            {"id":"reference","kind":"reference","source":"main","bibliography_id":"main-bib","number":"1","original_text":"Reference entry","evidence":loc("b2","Reference entry")},
            {"id":"si-body","kind":"paragraph","source":"si","evidence":loc("s1","Body")}
        ],"excluded_blocks":[],"excluded_pages":[]});
        let inventory=p.join("inventory.json");save(&inventory,&inv);
        let deliveries=json!([{"unit":"body","target":{"anchor":"body"}},{"unit":"call","target":{"quote":"[1](#^ref-a)"}},
            {"unit":"reference","target":{"anchor":"ref-a"}},{"unit":"si-body","target":{"anchor":"si-body"}}]);
        let paths=json!({"source_map":p.join("source-map.json"),"si_map":p.join("si-map.json"),"link_map":p.join("link-map.json")});
        let expected=json!({"main":{"path":a["path"],"sha256":a["sha256"],"role":"main"},"si":{"path":b["path"],"sha256":b["sha256"],"role":"supplement"}});
        let run=p.join("run.json");save(&run,&json!({"schema_version":1,"records":{"source_inventory":inventory,"source_map":paths["source_map"],"si_map":paths["si_map"],"link_map":paths["link_map"]},
            "config":{"candidate_path":candidate},"identity":{"sources":[a,b]},"scope":{"source_roles":{"main":"main","si":"supplement"}}}));
        Self{root,inventory,candidate,paths,deliveries,expected,run}
    }
    fn write(&self)->Value{maps::write(&self.inventory,&self.candidate,&self.deliveries,&self.paths,&self.expected,|_,_,_|{}).unwrap()}
    fn validate(&self)->Value{maps::validate(&self.inventory,&self.candidate,&self.paths,&self.expected,|_,_,_|{}).unwrap()}
    fn map(&self,name:&str)->PathBuf{PathBuf::from(self.paths[name].as_str().unwrap())}
}
#[test]
fn native_generation_validation_and_actual_rerun(){
    let f=Fixture::new();let r=f.write();assert_eq!(r["records_ready"],true,"{r}");
    let source=pipeline::read_json(&f.map("source_map")).unwrap();let link=pipeline::read_json(&f.map("link_map")).unwrap();
    assert_eq!(source["blocks"].as_array().unwrap().len(),2);assert_eq!(link["internal_links"][0]["origin_unit"],"call");
    assert_eq!(link["internal_links"][0]["target_unit"],"reference");
    assert_eq!(link["internal_links"][0]["source_display_text"],"[1]");
    assert_eq!(source["review_status"],"pending");assert_eq!(f.validate()["records_ready"],true);
    let old=fs::read(f.map("source_map")).unwrap();assert_eq!(f.write()["records_ready"],false);assert_eq!(fs::read(f.map("source_map")).unwrap(),old);
    let p=mapping_compare::record_mapping_rerun(&f.run,&f.deliveries,&f.root.path().join("rerun"),|_,_,_|{}).unwrap();
    let r=pipeline::read_json(&p).unwrap();assert_eq!(r["actual_rerun_performed"],true);
    assert_eq!(r["mapping_equal"],true);assert_eq!(r["acceptance_status_written"],false);
    assert_eq!(fs::read(f.map("source_map")).unwrap(),old);
    let mut changed=link;changed["internal_links"][0]["source_display_text"]=json!("fabricated");save(&f.map("link_map"),&changed);
    assert_eq!(f.validate()["records_ready"],false);
    assert!(mapping_compare::record_mapping_rerun(&f.run,&f.deliveries,&f.root.path().join("bad-rerun"),|_,_,_|{}).is_err());
    assert!(!f.root.path().join("bad-rerun").exists());
}
#[test]
fn preflight_preserves_complete_map_group(){
    let f=Fixture::new();let mut d=f.deliveries.clone();d.as_array_mut().unwrap().pop();
    let r=maps::write(&f.inventory,&f.candidate,&d,&f.paths,&f.expected,|_,_,_|{}).unwrap();
    assert_eq!(r["records_ready"],false);
    for name in ["source_map","si_map","link_map"]{assert!(!f.map(name).exists());}
    let mut inv=pipeline::read_json(&f.inventory).unwrap();inv["sources"][0]["pages"][0]["blocks"].as_array_mut().unwrap().push(json!({"id":"unaccounted","text":"lost","bbox":[10,50,100,60]}));
    save(&f.inventory,&inv);
    let r=f.write();assert_eq!(r["records_ready"],false);assert!(r["findings"][0]["message"].as_str().unwrap().contains("MAP_BLOCK_UNACCOUNTED"));
    assert!(!f.map("link_map").exists());
}
#[test]
fn actual_rerun_records_selector_change(){
    let f=Fixture::new();let r=f.write();assert_eq!(r["records_ready"],true,"{r}");
    let mut d=f.deliveries.clone();d[0]["target"]=json!({"quote":"正文 [1](#^ref-a)\n^body"});
    let p=mapping_compare::record_mapping_rerun(&f.run,&d,&f.root.path().join("changed"),|_,_,_|{}).unwrap();
    let r=pipeline::read_json(&p).unwrap();assert_eq!(r["actual_rerun_performed"],true);
    assert_eq!(r["mapping_equal"],false);assert!(!r["differences"].as_array().unwrap().is_empty());
}
