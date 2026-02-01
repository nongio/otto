use std::collections::HashMap;
use std::convert::TryInto;

use xdg_desktop_portal_otto::portal::{
    build_streams_value_from_descriptors, StreamDescriptor, SOURCE_TYPE_MONITOR,
};
use zbus::zvariant::{OwnedValue, Value};

#[test]
fn streams_value_encodes_required_metadata() {
    let mapping_id = "mapping".to_string();
    let descriptor = StreamDescriptor {
        node_id: 42,
        stream_id: "screen-1".to_string(),
        mapping_id: Some(mapping_id.clone()),
        width: Some(1920),
        height: Some(1080),
        position: Some((0, 0)),
        scale_factor: Some(1.5),
        refresh_millihz: Some(60_000),
        stride: Some(7680),
        fourcc: Some(875713112),
        modifier: Some(0),
        buffer_kind: Some("DMA".to_string()),
    };

    let owned = build_streams_value_from_descriptors(&[descriptor.clone()])
        .expect("should build streams value");

    let value: Value = owned.into();
    let entries: Vec<(u32, HashMap<String, OwnedValue>)> = value
        .try_into()
        .expect("streams value should decode into tuple array");

    assert_eq!(entries.len(), 1);
    let (node_id, props) = &entries[0];
    assert_eq!(*node_id, 42);

    let source_type =
        u32::try_from(props.get("source_type").unwrap().try_clone().unwrap()).unwrap();
    assert_eq!(source_type, SOURCE_TYPE_MONITOR);

    let id_value: Value = props.get("id").unwrap().try_clone().unwrap().into();
    let id = match id_value {
        Value::Str(s) => s.to_string(),
        other => panic!("unexpected id value: {other:?}"),
    };
    assert_eq!(id, descriptor.stream_id);

    let mapping_value: Value = props.get("mapping_id").unwrap().try_clone().unwrap().into();
    let mapping = match mapping_value {
        Value::Str(s) => s.to_string(),
        other => panic!("unexpected mapping value: {other:?}"),
    };
    assert_eq!(mapping, mapping_id);

    let size_value: Value = props.get("size").unwrap().try_clone().unwrap().into();
    let size: (i32, i32) = size_value.try_into().unwrap();
    assert_eq!(
        size,
        (
            descriptor.width.unwrap() as i32,
            descriptor.height.unwrap() as i32
        )
    );
    let position_value: Value = props.get("position").unwrap().try_clone().unwrap().into();
    let position: (i32, i32) = position_value.try_into().unwrap();
    assert_eq!(position, descriptor.position.unwrap());
}
