use super::{
    ResourceScope, form_matrix, form_resource_scope, image_metadata_for_object, page_geometry,
    page_resource_scope,
};
use crate::{ExtractError, PageNumber};
use lopdf::{Document, Object, ObjectId, Stream, dictionary};
use std::collections::HashMap;

#[test]
fn page_geometry_reads_inherited_media_box() {
    let mut document = Document::new();
    let page_tree_id: ObjectId = (1, 0);
    let page_id: ObjectId = (2, 0);

    document.objects.insert(
        page_tree_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => 1,
            "MediaBox" => vec![0.into(), 0.into(), 500.into(), 700.into()],
        }),
    );
    document.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Parent" => Object::Reference(page_tree_id),
        }),
    );

    let page_number = PageNumber::new(1);
    assert!(page_number.is_ok());
    let Ok(page_number) = page_number else { return };
    let geometry = page_geometry(&document, page_number, page_id, 8);
    assert!(geometry.is_ok());

    let Ok(geometry) = geometry else {
        return;
    };
    assert!((geometry.width - 500.0).abs() < f64::EPSILON);
    assert!((geometry.height - 700.0).abs() < f64::EPSILON);
}

#[test]
fn page_geometry_errors_on_malformed_media_box() {
    let mut document = Document::new();
    let page_id: ObjectId = (1, 0);

    document.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "MediaBox" => vec![0.into(), 0.into(), 500.into()],
        }),
    );

    let page_number = PageNumber::new(1);
    assert!(page_number.is_ok());
    let Ok(page_number) = page_number else { return };
    let geometry = page_geometry(&document, page_number, page_id, 8);
    assert!(geometry.is_err());

    let Err(error) = geometry else { return };
    assert!(matches!(error, ExtractError::MalformedPageGeometry { .. }));
}

#[test]
fn page_geometry_detects_parent_cycle() {
    let mut document = Document::new();
    let page_id: ObjectId = (1, 0);

    document.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 200.into()],
            "Parent" => Object::Reference(page_id),
        }),
    );

    let page_number = PageNumber::new(1).expect("page number");
    let error = page_geometry(&document, page_number, page_id, 8).expect_err("must fail");
    assert!(matches!(error, ExtractError::PageTreeCycleDetected { .. }));
}

#[test]
fn page_geometry_enforces_parent_depth_limit() {
    let mut document = Document::new();
    let root_id: ObjectId = (1, 0);
    let page_id: ObjectId = (2, 0);

    document.objects.insert(
        root_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 200.into()],
        }),
    );
    document.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Parent" => Object::Reference(root_id),
        }),
    );

    let page_number = PageNumber::new(1).expect("page number");
    let error = page_geometry(&document, page_number, page_id, 0).expect_err("must fail");
    assert!(matches!(error, ExtractError::PageTreeDepthExceeded { .. }));
}

#[test]
fn page_geometry_applies_crop_and_rotate_to_output_dimensions() {
    let mut document = Document::new();
    let page_id: ObjectId = (1, 0);

    document.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "MediaBox" => vec![0.into(), 0.into(), 300.into(), 200.into()],
            "CropBox" => vec![10.into(), 20.into(), 210.into(), 120.into()],
            "Rotate" => 90,
        }),
    );

    let page_number = PageNumber::new(1).expect("page number");
    let geometry = page_geometry(&document, page_number, page_id, 8).expect("geometry");
    assert!((geometry.width - 100.0).abs() < f64::EPSILON);
    assert!((geometry.height - 200.0).abs() < f64::EPSILON);

    let bbox = crate::BBox::new(10.0, 20.0, 30.0, 40.0).expect("bbox");
    let normalized = geometry
        .normalize_bbox(bbox, 3, page_number)
        .expect("normalized bbox");
    assert!(normalized.width() > 0.0);
    assert!(normalized.height() > 0.0);
}

#[test]
fn page_geometry_rejects_non_right_angle_rotation() {
    let mut document = Document::new();
    let page_id: ObjectId = (1, 0);

    document.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "MediaBox" => vec![0.into(), 0.into(), 300.into(), 200.into()],
            "Rotate" => 135,
        }),
    );

    let page_number = PageNumber::new(1).expect("page number");
    let error = page_geometry(&document, page_number, page_id, 8).expect_err("invalid rotation");
    assert!(matches!(error, ExtractError::MalformedPageGeometry { .. }));
}

#[test]
fn page_resource_scope_and_form_scope_resolve_xobjects() {
    let mut document = Document::new();
    let page_id: ObjectId = (1, 0);
    let resources_id: ObjectId = (2, 0);
    let image_id: ObjectId = (3, 0);
    let form_id: ObjectId = (4, 0);
    let form_resources_id: ObjectId = (5, 0);

    document.objects.insert(
        image_id,
        Object::Stream(Stream::new(
            dictionary! {
                "Subtype" => "Image",
                "Width" => 10,
                "Height" => 20,
            },
            vec![],
        )),
    );

    document.objects.insert(
        form_resources_id,
        Object::Dictionary(dictionary! {
            "XObject" => dictionary! {
                "NestedImage" => Object::Reference(image_id),
            },
        }),
    );

    document.objects.insert(
        form_id,
        Object::Stream(Stream::new(
            dictionary! {
                "Subtype" => "Form",
                "Resources" => Object::Reference(form_resources_id),
                "Matrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 2.into(), 3.into()],
            },
            vec![],
        )),
    );

    document.objects.insert(
        resources_id,
        Object::Dictionary(dictionary! {
            "XObject" => dictionary! {
                "Im1" => Object::Reference(image_id),
                "Fm1" => Object::Reference(form_id),
            },
        }),
    );

    document.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Resources" => Object::Reference(resources_id),
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
        }),
    );

    let scope = std::sync::Arc::new(page_resource_scope(&document, page_id).expect("scope"));
    assert!(scope.resolve_xobject(b"Im1").is_some());
    assert!(scope.resolve_xobject(b"Fm1").is_some());

    let form_stream = document
        .get_object(form_id)
        .expect("form object")
        .as_stream()
        .expect("form stream");
    let mut cache = HashMap::new();
    let nested_scope = form_resource_scope(
        &document,
        std::sync::Arc::clone(&scope),
        form_id,
        form_stream,
        &mut cache,
    );
    assert!(nested_scope.resolve_xobject(b"NestedImage").is_some());

    // Recursion into the same form must not deep-clone the parent chain:
    // we assert the parent `Arc` is shared between the two child scopes.
    let cached_scope = form_resource_scope(
        &document,
        std::sync::Arc::clone(&scope),
        form_id,
        form_stream,
        &mut cache,
    );
    assert!(cached_scope.resolve_xobject(b"NestedImage").is_some());
    assert!(std::sync::Arc::strong_count(&scope) >= 3);

    let metadata = image_metadata_for_object(&document, image_id, b"Im1").expect("metadata");
    assert_eq!(metadata.width_px, 10);
    assert_eq!(metadata.height_px, 20);

    let matrix = form_matrix(form_stream).expect("matrix");
    let expected = [1.0, 0.0, 0.0, 1.0, 2.0, 3.0];
    for (actual, expected) in matrix.into_iter().zip(expected.into_iter()) {
        assert!((actual - expected).abs() < f64::EPSILON);
    }

    let empty = ResourceScope::default();
    assert!(empty.resolve_xobject(b"missing").is_none());
}
