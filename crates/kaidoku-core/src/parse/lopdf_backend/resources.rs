use crate::{ExtractError, PageNumber};
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ImageMetadata {
    pub(super) name: String,
    pub(super) width_px: u32,
    pub(super) height_px: u32,
    pub(super) color_space: Option<String>,
    pub(super) bits_per_component: Option<u8>,
}

pub(super) fn build_image_catalog(
    document: &Document,
    page_id: ObjectId,
) -> Result<HashMap<Vec<u8>, ImageMetadata>, ExtractError> {
    let (resource_dict, resource_ids) =
        document
            .get_page_resources(page_id)
            .map_err(|error| ExtractError::PdfParse {
                reason: error.to_string(),
            })?;

    let mut catalog = HashMap::new();

    if let Some(dict) = resource_dict {
        collect_image_metadata(document, dict, &mut catalog);
    }

    for resource_id in &resource_ids {
        if let Ok(dict) = document.get_dictionary(*resource_id) {
            collect_image_metadata(document, dict, &mut catalog);
        }
    }

    Ok(catalog)
}

fn collect_image_metadata(
    document: &Document,
    resources: &Dictionary,
    out: &mut HashMap<Vec<u8>, ImageMetadata>,
) {
    let Ok(xobject) = resources.get_deref(b"XObject", document) else {
        return;
    };
    let Ok(xobject_dict) = xobject.as_dict() else {
        return;
    };

    for (name, value) in xobject_dict {
        let Ok((_, object)) = document.dereference(value) else {
            continue;
        };
        let Ok(stream) = object.as_stream() else {
            continue;
        };
        let Ok(subtype) = stream.dict.get(b"Subtype").and_then(Object::as_name) else {
            continue;
        };
        if subtype != b"Image" {
            continue;
        }

        let Some(width_px) = stream
            .dict
            .get(b"Width")
            .ok()
            .and_then(|value| value.as_i64().ok())
            .and_then(|value| u32::try_from(value).ok())
        else {
            continue;
        };

        let Some(height_px) = stream
            .dict
            .get(b"Height")
            .ok()
            .and_then(|value| value.as_i64().ok())
            .and_then(|value| u32::try_from(value).ok())
        else {
            continue;
        };

        let color_space = stream
            .dict
            .get(b"ColorSpace")
            .ok()
            .and_then(extract_color_space);

        let bits_per_component = stream
            .dict
            .get(b"BitsPerComponent")
            .ok()
            .and_then(|value| value.as_i64().ok())
            .and_then(|value| u8::try_from(value).ok());

        out.insert(
            name.clone(),
            ImageMetadata {
                name: String::from_utf8_lossy(name).to_string(),
                width_px,
                height_px,
                color_space,
                bits_per_component,
            },
        );
    }
}

fn extract_color_space(value: &Object) -> Option<String> {
    match value {
        Object::Name(name) => Some(String::from_utf8_lossy(name).to_string()),
        Object::Array(array) => array
            .first()
            .and_then(|first| first.as_name().ok())
            .map(|name| String::from_utf8_lossy(name).to_string()),
        _ => None,
    }
}

pub(super) fn page_dimensions(
    document: &Document,
    page_number: PageNumber,
    page_id: ObjectId,
) -> Result<(f64, f64), ExtractError> {
    let media_box =
        inherited_media_box(document, page_id).ok_or(ExtractError::MalformedPageGeometry {
            page_number: page_number.get(),
            page_object_number: page_id.0,
            page_object_generation: page_id.1,
            reason: "missing inherited MediaBox".to_string(),
        })?;

    let width = (media_box[2] - media_box[0]).abs();
    let height = (media_box[3] - media_box[1]).abs();
    if width > 0.0 && height > 0.0 {
        return Ok((width, height));
    }

    Err(ExtractError::MalformedPageGeometry {
        page_number: page_number.get(),
        page_object_number: page_id.0,
        page_object_generation: page_id.1,
        reason: format!(
            "invalid MediaBox dimensions [{:.3}, {:.3}, {:.3}, {:.3}]",
            media_box[0], media_box[1], media_box[2], media_box[3]
        ),
    })
}

fn inherited_media_box(document: &Document, page_id: ObjectId) -> Option<[f64; 4]> {
    let mut cursor = Some(page_id);
    while let Some(current_id) = cursor {
        let dict = document.get_dictionary(current_id).ok()?;
        if let Ok(media_box) = dict.get_deref(b"MediaBox", document) {
            let array = media_box.as_array().ok()?;
            if array.len() != 4 {
                return None;
            }

            let mut values = [0.0_f64; 4];
            for (index, value) in array.iter().enumerate() {
                values[index] = f64::from(object_to_f32(value).ok()?);
            }
            return Some(values);
        }

        cursor = dict.get(b"Parent").and_then(Object::as_reference).ok();
    }

    None
}

fn object_to_f32(value: &Object) -> Result<f32, ExtractError> {
    value
        .as_float()
        .map_err(|error| ExtractError::ContentDecode {
            reason: error.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::page_dimensions;
    use crate::{ExtractError, PageNumber};
    use lopdf::{Document, Object, ObjectId, dictionary};

    #[test]
    fn page_dimensions_reads_inherited_media_box() {
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
        let dimensions = page_dimensions(&document, page_number, page_id);
        assert!(dimensions.is_ok());

        let Ok((width, height)) = dimensions else {
            return;
        };
        assert!((width - 500.0).abs() < f64::EPSILON);
        assert!((height - 700.0).abs() < f64::EPSILON);
    }

    #[test]
    fn page_dimensions_errors_on_malformed_media_box() {
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
        let dimensions = page_dimensions(&document, page_number, page_id);
        assert!(dimensions.is_err());

        let Err(error) = dimensions else { return };
        assert!(matches!(error, ExtractError::MalformedPageGeometry { .. }));
    }
}
