use rbx_dom_weak::{types::Variant, ustr};

const FIXTURE: &str = r#"<roblox version="4"><Item class="StyleSheet" referent="R"><Properties><string name="Name">Theme</string><BinaryString name="AttributesSerialize">AgAAAAsAAABBY2NlbnRDb2xvcgIFAAAAJFBpbmsLAAAATW90aW9uSG92ZXIkCtcjPAAAAAAAAAAAAwAAAAAAAAAA</BinaryString></Properties></Item></roblox>"#;

#[test]
fn tweeninfo_attributes_decode_and_roundtrip_without_losing_other_attributes() {
    let model = rbx_xml::from_reader(FIXTURE.as_bytes(), rbx_xml::DecodeOptions::new()).unwrap();
    let theme = model.get_by_ref(model.root().children()[0]).unwrap();
    let attributes = theme.properties.get(&ustr("Attributes")).unwrap();
    assert!(
        matches!(attributes, Variant::Attributes(_)),
        "Attributes must decode, not fall back to an opaque blob"
    );
    let json = serde_json::to_value(attributes).unwrap();
    let Variant::Attributes(values) = attributes else {
        unreachable!()
    };
    match values.get("AccentColor").unwrap() {
        Variant::String(value) => assert_eq!(value, "$Pink"),
        Variant::BinaryString(value) => assert_eq!(AsRef::<[u8]>::as_ref(value), b"$Pink"),
        other => panic!("Unexpected AccentColor: {other:?}"),
    }
    let tween = &json["Attributes"]["MotionHover"]["TweenInfo"];
    assert_eq!(tween["easingStyle"], 3);
    assert_eq!(tween["easingDirection"], 0);
    assert_eq!(tween["repeatCount"], 0);
    assert_eq!(tween["reverses"], false);
    assert_eq!(tween["delayTime"], 0.0);
    assert!((tween["time"].as_f64().unwrap() - 0.01).abs() < 0.00001);
    let mut binary = Vec::new();
    rbx_binary::to_writer(&mut binary, &model, &[theme.referent()]).unwrap();
    let decoded = rbx_binary::from_reader(binary.as_slice()).unwrap();
    let actual = decoded
        .descendants()
        .find(|instance| instance.name == "Theme")
        .unwrap();
    assert_eq!(actual.properties.get(&ustr("Attributes")), Some(attributes));

    let mut xml = Vec::new();
    rbx_xml::to_writer(
        &mut xml,
        &model,
        &[theme.referent()],
        rbx_xml::EncodeOptions::new(),
    )
    .unwrap();
    let decoded = rbx_xml::from_reader(xml.as_slice(), rbx_xml::DecodeOptions::new()).unwrap();
    assert_eq!(
        decoded
            .get_by_ref(decoded.root().children()[0])
            .unwrap()
            .properties
            .get(&ustr("Attributes")),
        Some(attributes)
    );
}

#[test]
fn tweeninfo_attribute_payload_roundtrips_all_fields_and_rejects_truncation() {
    use rbx_dom_weak::types::Attributes;
    let value = serde_json::json!({"TweenInfo": {"time": 1.25, "delayTime": 0.5, "easingStyle": 10, "easingDirection": 2, "repeatCount": -1, "reverses": true}});
    let tween: Variant = serde_json::from_value(value).unwrap();
    let mut attributes = Attributes::new();
    attributes.insert("Motion".into(), tween);
    let mut bytes = Vec::new();
    attributes.to_writer(&mut bytes).unwrap();
    assert_eq!(
        Attributes::from_reader(bytes.as_slice()).unwrap(),
        attributes
    );
    for length in 1..bytes.len() {
        assert!(
            Attributes::from_reader(&bytes[..length]).is_err(),
            "truncation at {length} was accepted"
        );
    }
    // Four-byte count, four-byte key length, six-byte key, then type ID.
    bytes[14] = 0xff;
    assert!(Attributes::from_reader(bytes.as_slice()).is_err());
}

#[test]
fn syncback_keeps_tweeninfo_and_other_attributes_in_binary_and_xml_models() {
    use librojo::{snapshot_from_vfs, syncback_loop, InstanceContext, Project, RojoTree};
    use rbx_dom_weak::{types::Attributes, InstanceBuilder, WeakDom};

    let tween: Variant = serde_json::from_value(serde_json::json!({"TweenInfo": {"time": 1.25, "delayTime": 0.5, "easingStyle": 10, "easingDirection": 2, "repeatCount": -1, "reverses": true}})).unwrap();
    for extension in ["rbxm", "rbxmx"] {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join(format!("Theme.{extension}"));
        let initial =
            rbx_xml::from_reader(FIXTURE.as_bytes(), rbx_xml::DecodeOptions::new()).unwrap();
        let mut contents = Vec::new();
        if extension == "rbxm" {
            rbx_binary::to_writer(&mut contents, &initial, initial.root().children()).unwrap();
        } else {
            contents.extend_from_slice(FIXTURE.as_bytes());
        }
        std::fs::write(&source, contents).unwrap();
        let path = directory.path().join("default.project.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({"name":"Theme", "tree":{"$path":source}}))
                .unwrap(),
        )
        .unwrap();
        let vfs = memofs::Vfs::new_default().unwrap();
        vfs.set_watch_enabled(false);
        let project = Project::load_exact(&vfs, &path, None).unwrap();
        let initial = snapshot_from_vfs(&InstanceContext::default(), &vfs, &path)
            .unwrap()
            .unwrap();
        let mut old = RojoTree::new(initial);
        let mut attributes = Attributes::new();
        attributes.insert("MotionHover".into(), tween.clone());
        attributes.insert("AccentColor".into(), Variant::String("$Blue".into()));
        let new = WeakDom::new(
            InstanceBuilder::new("StyleSheet")
                .with_name("Theme")
                .with_property("Attributes", attributes),
        );
        let changes = syncback_loop(&vfs, &mut old, new, &project).unwrap();
        changes.write_to_vfs(directory.path(), &vfs).unwrap();
        let after = snapshot_from_vfs(&InstanceContext::default(), &vfs, &path)
            .unwrap()
            .unwrap();
        let Variant::Attributes(attributes) = &after.properties[&ustr("Attributes")] else {
            panic!("{extension}: attributes became opaque")
        };
        assert_eq!(attributes.get("MotionHover"), Some(&tween), "{extension}");
        match attributes.get("AccentColor").unwrap() {
            Variant::String(value) => assert_eq!(value, "$Blue"),
            Variant::BinaryString(value) => assert_eq!(AsRef::<[u8]>::as_ref(value), b"$Blue"),
            other => panic!("{extension}: incorrect ordinary attribute: {other:?}"),
        }
    }
}
