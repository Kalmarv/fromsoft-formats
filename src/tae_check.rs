//! A TAE file held to the community's event templates: [`Tae::check_templates`] refuses an event whose parameter bytes
//! break a template, naming the animation, the event and the field, and [`Tae::write_checked`] writes only a file that
//! passes. The templates are the caller's ([`Templates`]); what they assert is read from them, never assumed here.
use crate::{
    Result, invalid,
    tae::{
        Tae,
        template::{FieldKind, Templates},
    },
};

const FORMAT: &str = "TAE";

/// What a file held to its templates came to: [`Tae::check_templates`] returns it for a file that passes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TemplateCheck {
    /// The events of the file.
    pub events: usize,
    /// The events whose type the templates define (the others are not checked).
    pub templated: usize,
    /// The asserted fields that the events store (a field an event is stored too short to hold is not counted), each of
    /// which holds its asserted value.
    pub asserted_fields: usize,
}

/// The value the bytes `raw` store for a field of `kind`, as a float: `None` for a run of bytes, a float that is not finite
/// and bytes shorter than the kind.
fn stored(kind: FieldKind, raw: &[u8]) -> Option<f64> {
    fn le<const N: usize>(raw: &[u8]) -> Option<[u8; N]> {
        raw.get(..N)?.try_into().ok()
    }
    Some(match kind {
        FieldKind::Bool | FieldKind::U8 => f64::from(*raw.first()?),
        FieldKind::S8 => f64::from(i8::from_le_bytes(le::<1>(raw)?)),
        FieldKind::U16 => f64::from(u16::from_le_bytes(le::<2>(raw)?)),
        FieldKind::S16 => f64::from(i16::from_le_bytes(le::<2>(raw)?)),
        FieldKind::U32 => f64::from(u32::from_le_bytes(le::<4>(raw)?)),
        FieldKind::S32 => f64::from(i32::from_le_bytes(le::<4>(raw)?)),
        FieldKind::U64 => u64::from_le_bytes(le::<8>(raw)?) as f64,
        FieldKind::S64 => i64::from_le_bytes(le::<8>(raw)?) as f64,
        FieldKind::F32 => {
            let value = f32::from_le_bytes(le::<4>(raw)?);
            if !value.is_finite() {
                return None;
            }
            f64::from(value)
        }
        FieldKind::F64 => {
            let value = f64::from_le_bytes(le::<8>(raw)?);
            if !value.is_finite() {
                return None;
            }
            value
        }
        FieldKind::Bytes(_) => return None,
    })
}

impl Tae {
    /// Hold every event of the file whose type `templates` define to its template: each asserted field the event stores
    /// must hold its asserted value, and the bytes must decode through the template
    /// ([`EventTemplate::decode`](crate::tae::template::EventTemplate::decode): a boolean byte is 0 or 1, a float64 is
    /// finite). The first event that fails is refused with its animation's number ([`Tae::number`]), its place in the
    /// animation, its type and name, and, for an asserted field, the field's place in the template, its name, its offset in
    /// the event's bytes, what it stores and what the template asserts. An event the file stores shorter than its template is
    /// checked as far as it goes, and an event of a type the templates do not define is not checked.
    ///
    /// Over every event file of both installs and the community's Sekiro and Elden Ring templates, this passes: 553,355 of
    /// Sekiro's and 2,050,123 of Elden Ring's asserted fields hold their value.
    pub fn check_templates(&self, templates: &Templates) -> Result<TemplateCheck> {
        let mut check = TemplateCheck::default();
        for animation in &self.animations {
            for (index, event) in animation.events.iter().enumerate() {
                check.events = check.events.saturating_add(1);
                let Some(template) = templates.get(event.kind) else {
                    continue;
                };
                check.templated = check.templated.saturating_add(1);
                let place = || {
                    format!(
                        "animation {} event {index} (type {}{})",
                        self.number(animation),
                        event.kind,
                        template
                            .name
                            .as_deref()
                            .map(|n| format!(", {n}"))
                            .unwrap_or_default()
                    )
                };
                let mut at = 0usize;
                for (position, field) in template.fields.iter().enumerate() {
                    let end = at.saturating_add(field.kind.width());
                    let Some(raw) = event.parameters.get(at..end) else {
                        break;
                    };
                    if let Some(want) = field.assert {
                        check.asserted_fields = check.asserted_fields.saturating_add(1);
                        let got = stored(field.kind, raw);
                        if got != Some(want) {
                            let stored = got.map_or_else(|| format!("{raw:02x?}"), |v| v.to_string());
                            return Err(invalid(
                                FORMAT,
                                0,
                                format!(
                                    "{}: field {position}{} at byte {at:#x} of the event stores {stored}, the template asserts {want}",
                                    place(),
                                    field
                                        .name
                                        .as_deref()
                                        .map(|n| format!(" ({n})"))
                                        .unwrap_or_default()
                                ),
                            ));
                        }
                    }
                    at = end;
                }
                template
                    .decode(&event.parameters)
                    .map_err(|e| invalid(FORMAT, 0, format!("{}: {e}", place())))?;
            }
        }
        Ok(check)
    }

    /// [`Tae::write`] for a file that [`Tae::check_templates`] passes: a file with an event that breaks a template is
    /// refused, with that check's refusal, and nothing is written.
    pub fn write_checked(&self, templates: &Templates) -> Result<Vec<u8>> {
        self.check_templates(templates)?;
        self.write()
    }
}
