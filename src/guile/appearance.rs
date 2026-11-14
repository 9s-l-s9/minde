// SPDX-License-Identifier: GPL-3.0-or-later
//! Atomic appearance requests parsed before reaching compositor state.
use super::output_config::SpecValue;
use crate::render::Appearance;

#[derive(Debug, Clone)]
pub enum AppearanceChange {
    BorderWidth(i32),
    FontSize(i32),
    Padding(i32),
    MessageBorderWidth(i32),
    Foreground([u8; 4]),
    MessageBackground([u8; 4]),
    MessageBorderColor([u8; 4]),
    Background([u8; 4]),
}

impl AppearanceChange {
    pub fn apply(self, appearance: &mut Appearance) {
        match self {
            Self::BorderWidth(width) => appearance.border_width = width,
            Self::FontSize(size) => appearance.message.font_size = size,
            Self::Padding(padding) => appearance.message.padding = padding,
            Self::MessageBorderWidth(width) => appearance.message.border_width = width,
            Self::Foreground(color) => appearance.message.foreground = color,
            Self::MessageBackground(color) => appearance.message.background = color,
            Self::MessageBorderColor(color) => appearance.message.border_color = color,
            Self::Background(color) => appearance.background = color.map(|c| c as f32 / 255.0),
        }
    }
}

fn color(value: &SpecValue) -> Result<[u8; 4], String> {
    if let SpecValue::Str(hex) = value
        && hex.len() == 7
        && hex.starts_with('#')
        && hex.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        && let Ok(rgb) = u32::from_str_radix(&hex[1..], 16)
    {
        return Ok([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255]);
    }
    Err(format!("expected a #rrggbb color, got {value}"))
}

pub fn parse(entries: &[(String, SpecValue)]) -> Result<Vec<AppearanceChange>, String> {
    use AppearanceChange::*;
    entries
        .iter()
        .enumerate()
        .map(|(index, (key, value))| {
            if entries[..index].iter().any(|(previous, _)| previous == key) {
                return Err(format!("duplicate appearance setting: {key}"));
            }
            Ok(match (key.as_str(), value) {
                ("border-width", SpecValue::Int(v)) if (0..=32).contains(v) => {
                    BorderWidth(*v as i32)
                }
                ("message-font-size", SpecValue::Int(v)) if (6..=96).contains(v) => {
                    FontSize(*v as i32)
                }
                ("message-padding", SpecValue::Int(v)) if (0..=64).contains(v) => {
                    Padding(*v as i32)
                }
                ("message-border-width", SpecValue::Int(v)) if (0..=16).contains(v) => {
                    MessageBorderWidth(*v as i32)
                }
                ("message-foreground", v) => Foreground(color(v)?),
                ("message-background", v) => MessageBackground(color(v)?),
                ("message-border-color", v) => MessageBorderColor(color(v)?),
                ("background", v) => Background(color(v)?),
                _ => {
                    return Err(format!(
                        "unknown appearance setting or invalid value: {key} = {value}"
                    ));
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_requests_reject_typos_and_bad_values_atomically() {
        for (key, value) in [
            ("font-size", SpecValue::Int(20)),
            ("message-font-size", SpecValue::Real(20.0)),
            ("message-font-size", SpecValue::Int(0)),
            ("border-width", SpecValue::Int(-1)),
            ("message-padding", SpecValue::Int(65)),
            ("message-border-width", SpecValue::Int(17)),
            ("background", SpecValue::Str("#abcdef00".into())),
            ("background", SpecValue::Str("#+12345".into())),
            ("message-foreground", SpecValue::Str("#xyz123".into())),
        ] {
            let valid_entry = if key == "background" {
                ("border-width".into(), SpecValue::Int(0))
            } else {
                ("background".into(), SpecValue::Str("#000000".into()))
            };
            assert!(
                parse(&[valid_entry, (key.into(), value)]).is_err(),
                "accepted invalid {key}"
            );
        }
        assert!(
            parse(&[
                ("background".into(), SpecValue::Str("#abcdef".into())),
                ("background".into(), SpecValue::Str("#000000".into()))
            ])
            .is_err()
        );
        let mut appearance = Appearance::default();
        for change in parse(&[
            ("border-width".into(), SpecValue::Int(0)),
            ("message-font-size".into(), SpecValue::Int(24)),
            (
                "message-background".into(),
                SpecValue::Str("#12abEF".into()),
            ),
        ])
        .unwrap()
        {
            change.apply(&mut appearance);
        }
        assert_eq!(appearance.border_width, 0);
        assert_eq!(appearance.message.font_size, 24);
        assert_eq!(appearance.message.background, [0x12, 0xab, 0xef, 255]);
    }
}
