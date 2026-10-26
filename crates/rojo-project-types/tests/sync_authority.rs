use rojo_project_types::{PropertyAuthority, StructureAuthority, SyncAuthority};
use serde_json::json;

#[test]
fn exact_and_nearest_class_policies_win_independently_of_list_order() {
    for policies in [
        json!([
            {"className":"Instance","properties":"filesystem"},
            {"className":"Part","properties":"studio","structure":"twoWay"},
            {"className":"BasePart","properties":"twoWay"}
        ]),
        json!([
            {"className":"BasePart","properties":"twoWay"},
            {"className":"Part","properties":"studio","structure":"twoWay"},
            {"className":"Instance","properties":"filesystem"}
        ]),
    ] {
        let authority: SyncAuthority = serde_json::from_value(policies.clone()).unwrap();
        let part = authority.resolve("Part").unwrap().unwrap();
        assert_eq!(part.properties, PropertyAuthority::Studio);
        assert_eq!(part.structure, StructureAuthority::TwoWay);
        let mesh = authority.resolve("MeshPart").unwrap().unwrap();
        assert_eq!(mesh.class_name, "BasePart");
        assert_eq!(mesh.properties, PropertyAuthority::TwoWay);
        assert_eq!(mesh.structure, StructureAuthority::Filesystem);
        assert_eq!(
            authority.resolve("Folder").unwrap().unwrap().properties,
            PropertyAuthority::Filesystem
        );
        let roundtrip: SyncAuthority =
            serde_json::from_value(serde_json::to_value(&authority).unwrap()).unwrap();
        assert_eq!(roundtrip, authority);
    }
}

#[test]
fn malformed_authority_never_silently_grants_permissions() {
    for value in [
        json!([{"className":"Part","properties":"twoWay"}, {"className":"Part","properties":"studio"}]),
        json!([{"className":"NotARobloxClass","properties":"twoWay"}]),
        json!([{"className":"Part"}]),
        json!([{"className":"Part","properties":"twoWay","structure":"studio"}]),
        json!([{"className":"Part","properties":"twoWay","typo":true}]),
        json!([{"className":"Part","properties":"two-way"}]),
        json!([{"className":"part","properties":"twoWay"}]),
        json!(null),
    ] {
        assert!(
            serde_json::from_value::<SyncAuthority>(value.clone()).is_err(),
            "{value}"
        );
    }
}

#[test]
fn empty_and_unmatched_authority_keep_legacy_behavior_but_unknown_instances_error() {
    let empty: SyncAuthority = serde_json::from_value(json!([])).unwrap();
    assert!(empty.resolve("Part").unwrap().is_none());
    assert!(empty.resolve("NotARobloxClass").is_err());
    let scripts: SyncAuthority = serde_json::from_value(json!([
        {"className":"LuaSourceContainer","properties":"twoWay"}
    ]))
    .unwrap();
    assert!(scripts.resolve("Part").unwrap().is_none());
    for class in ["Script", "LocalScript", "ModuleScript"] {
        assert_eq!(
            scripts.resolve(class).unwrap().unwrap().properties,
            PropertyAuthority::TwoWay
        );
    }
}
