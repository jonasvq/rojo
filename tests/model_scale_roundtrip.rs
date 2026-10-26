use librojo::{build_project, write_project, BuildFormat, Project};
use memofs::{InMemoryFs, Vfs};
use rbx_dom_weak::{
    types::{Ref, Variant, Vector3},
    InstanceBuilder, WeakDom,
};
use serde_json::json;

#[test]
fn stored_model_scale_preserves_geometry_and_references_across_json_xml_and_binary() {
    let pole = Ref::new();
    let dom = WeakDom::new(
        InstanceBuilder::new("DataModel").with_child(
            InstanceBuilder::new("Model")
                .with_name("Flag")
                .with_property("Scale", 2.0_f32)
                .with_property("PrimaryPart", pole)
                .with_child(
                    InstanceBuilder::new("Part")
                        .with_referent(pole)
                        .with_name("Pole")
                        .with_property("Size", Vector3::new(6.0, 0.2, 0.2)),
                ),
        ),
    );
    for source in ["model.json", "rbxmx", "rbxm"] {
        let mut bytes = Vec::new();
        match source {
            "model.json" => bytes=json!({"className":"Model","properties":{"Scale":2},"attributes":{"Rojo_Target_PrimaryPart":"pole"},"children":[{"name":"Pole","className":"Part","attributes":{"Rojo_Id":"pole"},"properties":{"Size":[6,0.2,0.2]}}]}).to_string().into_bytes(),
            "rbxmx" => rbx_xml::to_writer(&mut bytes,&dom,dom.root().children(),rbx_xml::EncodeOptions::new()).unwrap(),
            _ => rbx_binary::to_writer(&mut bytes,&dom,dom.root().children()).unwrap(),
        }
        let vfs = Vfs::new(InMemoryFs::new());
        vfs.set_watch_enabled(false);
        vfs.create_dir_all("/project").unwrap();
        vfs.write(format!("/project/Flag.{source}"), bytes).unwrap();
        vfs.write(
            "/project/default.project.json",
            json!({"name":"Root","tree":{"$path":format!("Flag.{source}")}}).to_string(),
        )
        .unwrap();
        let project =
            Project::load_exact(&vfs, "/project/default.project.json".as_ref(), None).unwrap();
        let tree = build_project(&vfs, &project).unwrap();
        for format in [BuildFormat::Rbxm, BuildFormat::Rbxmx] {
            let mut output = Vec::new();
            write_project(&mut output, &tree, format).unwrap();
            let decoded = match format {
                BuildFormat::Rbxm => rbx_binary::from_reader(output.as_slice()).unwrap(),
                _ => {
                    rbx_xml::from_reader(output.as_slice(), rbx_xml::DecodeOptions::new()).unwrap()
                }
            };
            let model = decoded.get_by_ref(decoded.root().children()[0]).unwrap();
            let pole = decoded.get_by_ref(model.children()[0]).unwrap();
            assert_eq!(
                model.properties[&"Scale".into()],
                Variant::Float32(2.0),
                "{source} {format:?}"
            );
            assert_eq!(
                pole.properties[&"Size".into()],
                Variant::Vector3(Vector3::new(6.0, 0.2, 0.2)),
                "{source} {format:?}"
            );
            assert_eq!(
                model.properties[&"PrimaryPart".into()],
                Variant::Ref(pole.referent()),
                "{source} {format:?}"
            );
        }
    }
}
