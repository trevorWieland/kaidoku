use crate::{BBox, ExtractError, PageNumber};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ImageMetadata {
    pub(super) name: String,
    pub(super) width_px: u32,
    pub(super) height_px: u32,
    pub(super) color_space: Option<String>,
    pub(super) bits_per_component: Option<u8>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ResourceScope {
    xobjects: HashMap<Vec<u8>, ObjectId>,
}

impl ResourceScope {
    #[must_use]
    pub(super) fn resolve_xobject(&self, name: &[u8]) -> Option<ObjectId> {
        self.xobjects.get(name).copied()
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PageGeometry {
    crop_min_x: f64,
    crop_min_y: f64,
    rotate_deg: i32,
    pub(super) width: f64,
    pub(super) height: f64,
}

impl PageGeometry {
    pub(super) fn normalize_bbox(
        self,
        bbox: BBox,
        precision: u8,
        page_number: PageNumber,
    ) -> Result<BBox, ExtractError> {
        let mut x = bbox.x() - self.crop_min_x;
        let mut y = bbox.y() - self.crop_min_y;
        let mut width = bbox.width();
        let mut height = bbox.height();

        if !x.is_finite() || !y.is_finite() || !width.is_finite() || !height.is_finite() {
            return Err(ExtractError::InvalidFallbackGeometry {
                page_number: page_number.get(),
                reason: "non-finite bbox values during page normalization".to_string(),
            });
        }

        if width < 0.0 || height < 0.0 {
            return Err(ExtractError::InvalidFallbackGeometry {
                page_number: page_number.get(),
                reason: "negative bbox dimensions during page normalization".to_string(),
            });
        }

        match self.rotate_deg {
            0 => {}
            90 => {
                let next_x = y;
                let next_y = self.height - (x + width);
                x = next_x;
                y = next_y;
                std::mem::swap(&mut width, &mut height);
            }
            180 => {
                x = self.width - (x + width);
                y = self.height - (y + height);
            }
            270 => {
                let next_x = self.width - (y + height);
                let next_y = x;
                x = next_x;
                y = next_y;
                std::mem::swap(&mut width, &mut height);
            }
            _ => {
                return Err(ExtractError::MalformedPageGeometry {
                    page_number: page_number.get(),
                    page_object_number: 0,
                    page_object_generation: 0,
                    reason: format!("unsupported page rotation {}", self.rotate_deg),
                });
            }
        }

        BBox::new(x, y, width, height)
            .map_err(ExtractError::from)
            .map(|normalized| normalized.quantized(precision))
    }
}

pub(super) fn page_resource_scope(
    document: &Document,
    page_id: ObjectId,
) -> Result<ResourceScope, ExtractError> {
    let (resource_dict, resource_ids) =
        document
            .get_page_resources(page_id)
            .map_err(|error| ExtractError::PdfParse {
                reason: error.to_string(),
            })?;

    let mut scope = ResourceScope::default();

    for resource_id in &resource_ids {
        if let Ok(dict) = document.get_dictionary(*resource_id) {
            collect_xobjects(document, dict, &mut scope);
        }
    }

    if let Some(dict) = resource_dict {
        collect_xobjects(document, dict, &mut scope);
    }

    Ok(scope)
}

pub(super) fn form_resource_scope(
    document: &Document,
    parent: &ResourceScope,
    form_stream: &Stream,
) -> ResourceScope {
    let mut merged = parent.clone();

    if let Ok(resources_obj) = form_stream.dict.get_deref(b"Resources", document)
        && let Ok(resources_dict) = resources_obj.as_dict()
    {
        collect_xobjects(document, resources_dict, &mut merged);
    }

    merged
}

pub(super) fn image_metadata_for_object(
    document: &Document,
    object_id: ObjectId,
    name_hint: &[u8],
) -> Option<ImageMetadata> {
    let object = document.get_object(object_id).ok()?;
    let stream = object.as_stream().ok()?;
    let subtype = stream.dict.get(b"Subtype").ok()?.as_name().ok()?;
    if subtype != b"Image" {
        return None;
    }

    let width_px = stream
        .dict
        .get(b"Width")
        .ok()
        .and_then(|value| value.as_i64().ok())
        .and_then(|value| u32::try_from(value).ok())?;

    let height_px = stream
        .dict
        .get(b"Height")
        .ok()
        .and_then(|value| value.as_i64().ok())
        .and_then(|value| u32::try_from(value).ok())?;

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

    Some(ImageMetadata {
        name: String::from_utf8_lossy(name_hint).to_string(),
        width_px,
        height_px,
        color_space,
        bits_per_component,
    })
}

pub(super) fn form_matrix(stream: &Stream) -> Option<[f64; 6]> {
    let matrix = stream.dict.get(b"Matrix").ok()?.as_array().ok()?;
    if matrix.len() != 6 {
        return None;
    }

    let mut values = [0.0_f64; 6];
    for (index, value) in matrix.iter().enumerate() {
        values[index] = f64::from(object_to_f32(value).ok()?);
    }

    Some(values)
}

pub(super) fn page_geometry(
    document: &Document,
    page_number: PageNumber,
    page_id: ObjectId,
    max_depth: usize,
) -> Result<PageGeometry, ExtractError> {
    let inherited = inherited_page_attributes(document, page_number, page_id, max_depth)?;

    let media_box = inherited
        .media_box
        .ok_or(ExtractError::MalformedPageGeometry {
            page_number: page_number.get(),
            page_object_number: page_id.0,
            page_object_generation: page_id.1,
            reason: "missing inherited MediaBox".to_string(),
        })?;

    let crop_box = inherited.crop_box.unwrap_or(media_box);
    let crop_width = (crop_box[2] - crop_box[0]).abs();
    let crop_height = (crop_box[3] - crop_box[1]).abs();
    if crop_width <= 0.0 || crop_height <= 0.0 {
        return Err(ExtractError::MalformedPageGeometry {
            page_number: page_number.get(),
            page_object_number: page_id.0,
            page_object_generation: page_id.1,
            reason: format!(
                "invalid CropBox dimensions [{:.3}, {:.3}, {:.3}, {:.3}]",
                crop_box[0], crop_box[1], crop_box[2], crop_box[3]
            ),
        });
    }

    let rotate_deg = normalize_rotate(inherited.rotate_deg.unwrap_or(0));

    let (width, height) = if rotate_deg == 90 || rotate_deg == 270 {
        (crop_height, crop_width)
    } else {
        (crop_width, crop_height)
    };

    Ok(PageGeometry {
        crop_min_x: crop_box[0],
        crop_min_y: crop_box[1],
        rotate_deg,
        width,
        height,
    })
}

fn normalize_rotate(raw: i32) -> i32 {
    let normalized = raw.rem_euclid(360);
    normalized - (normalized % 90)
}

#[derive(Debug, Clone, Copy, Default)]
struct InheritedPageAttributes {
    media_box: Option<[f64; 4]>,
    crop_box: Option<[f64; 4]>,
    rotate_deg: Option<i32>,
}

fn inherited_page_attributes(
    document: &Document,
    page_number: PageNumber,
    page_id: ObjectId,
    max_depth: usize,
) -> Result<InheritedPageAttributes, ExtractError> {
    let mut attrs = InheritedPageAttributes::default();
    let mut cursor = Some(page_id);
    let mut visited = HashSet::new();
    let mut depth = 0usize;

    while let Some(current_id) = cursor {
        if depth > max_depth {
            return Err(ExtractError::PageTreeDepthExceeded {
                page_number: page_number.get(),
                depth,
                limit: max_depth,
            });
        }

        if !visited.insert(current_id) {
            return Err(ExtractError::PageTreeCycleDetected {
                page_number: page_number.get(),
                object_number: current_id.0,
                object_generation: current_id.1,
            });
        }

        let dict = document.get_dictionary(current_id).map_err(|error| {
            ExtractError::MalformedPageGeometry {
                page_number: page_number.get(),
                page_object_number: current_id.0,
                page_object_generation: current_id.1,
                reason: error.to_string(),
            }
        })?;

        if attrs.media_box.is_none() {
            attrs.media_box = dict
                .get_deref(b"MediaBox", document)
                .ok()
                .and_then(parse_box_array);
        }

        if attrs.crop_box.is_none() {
            attrs.crop_box = dict
                .get_deref(b"CropBox", document)
                .ok()
                .and_then(parse_box_array);
        }

        if attrs.rotate_deg.is_none() {
            attrs.rotate_deg = dict
                .get(b"Rotate")
                .ok()
                .and_then(|value| value.as_i64().ok())
                .and_then(|value| i32::try_from(value).ok());
        }

        cursor = dict.get(b"Parent").and_then(Object::as_reference).ok();
        depth = depth.saturating_add(1);

        if attrs.media_box.is_some() && attrs.crop_box.is_some() && attrs.rotate_deg.is_some() {
            break;
        }
    }

    Ok(attrs)
}

fn parse_box_array(value: &Object) -> Option<[f64; 4]> {
    let array = value.as_array().ok()?;
    if array.len() != 4 {
        return None;
    }

    let mut values = [0.0_f64; 4];
    for (index, value) in array.iter().enumerate() {
        values[index] = f64::from(object_to_f32(value).ok()?);
    }

    let min_x = values[0].min(values[2]);
    let max_x = values[0].max(values[2]);
    let min_y = values[1].min(values[3]);
    let max_y = values[1].max(values[3]);

    Some([min_x, min_y, max_x, max_y])
}

fn collect_xobjects(document: &Document, resources: &Dictionary, scope: &mut ResourceScope) {
    let Ok(xobject) = resources.get_deref(b"XObject", document) else {
        return;
    };
    let Ok(xobject_dict) = xobject.as_dict() else {
        return;
    };

    for (name, value) in xobject_dict {
        let Ok((object_id, object)) = document.dereference(value) else {
            continue;
        };
        let Some(object_id) = object_id else {
            continue;
        };

        let Ok(stream) = object.as_stream() else {
            continue;
        };

        if stream
            .dict
            .get(b"Subtype")
            .and_then(Object::as_name)
            .is_ok()
        {
            scope.xobjects.insert(name.clone(), object_id);
        }
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

fn object_to_f32(value: &Object) -> Result<f32, ExtractError> {
    value
        .as_float()
        .map_err(|error| ExtractError::ContentDecode {
            reason: error.to_string(),
        })
}

#[cfg(test)]
mod resources_tests;
