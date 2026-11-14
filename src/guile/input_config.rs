// SPDX-License-Identifier: GPL-3.0-or-later
//! Validated optional libinput settings; parsing does not touch devices.

use super::output_config::SpecValue;
use smithay::reexports::input::ScrollMethod;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct InputOptions {
    pub accel_speed: Option<f64>,
    pub scroll_method: Option<ScrollMethod>,
    pub scroll_button: Option<u32>,
    pub left_handed: Option<bool>,
    pub disable_while_typing: Option<bool>,
    pub middle_emulation: Option<bool>,
}

impl InputOptions {
    pub fn parse(entries: &[(String, SpecValue)]) -> Result<Self, String> {
        let mut options = Self::default();
        for (index, (key, value)) in entries.iter().enumerate() {
            if entries[..index].iter().any(|(previous, _)| previous == key) {
                return Err(format!("duplicate input setting: {key}"));
            }
            match (key.as_str(), value) {
                ("accel-speed", SpecValue::Int(speed)) if (-1..=1).contains(speed) => {
                    options.accel_speed = Some(*speed as f64);
                }
                ("accel-speed", SpecValue::Real(speed)) if (-1.0..=1.0).contains(speed) => {
                    options.accel_speed = Some(*speed);
                }
                ("scroll-method", SpecValue::Sym(method) | SpecValue::Str(method)) => {
                    options.scroll_method = Some(match method.as_str() {
                        "none" => ScrollMethod::NoScroll,
                        "two-finger" => ScrollMethod::TwoFinger,
                        "edge" => ScrollMethod::Edge,
                        "on-button-down" => ScrollMethod::OnButtonDown,
                        _ => return Err(format!("unknown scroll-method: {method}")),
                    });
                }
                ("scroll-button", SpecValue::Int(button)) if (0..=0x2ff).contains(button) => {
                    options.scroll_button = Some(*button as u32);
                }
                ("left-handed", SpecValue::Bool(on)) => options.left_handed = Some(*on),
                ("disable-while-typing", SpecValue::Bool(on)) => {
                    options.disable_while_typing = Some(*on);
                }
                ("middle-emulation", SpecValue::Bool(on)) => options.middle_emulation = Some(*on),
                _ => {
                    return Err(format!(
                        "unknown input setting or invalid value: {key} = {value}"
                    ));
                }
            }
        }
        Ok(options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_preserve_false_zero_and_omission() {
        let options = InputOptions::parse(&[
            ("accel-speed".into(), SpecValue::Real(-0.4)),
            (
                "scroll-method".into(),
                SpecValue::Sym("on-button-down".into()),
            ),
            ("scroll-button".into(), SpecValue::Int(274)),
            ("left-handed".into(), SpecValue::Bool(false)),
            ("disable-while-typing".into(), SpecValue::Bool(true)),
            ("middle-emulation".into(), SpecValue::Bool(false)),
        ])
        .unwrap();
        assert_eq!(options.accel_speed, Some(-0.4));
        assert_eq!(options.scroll_method, Some(ScrollMethod::OnButtonDown));
        assert_eq!(options.scroll_button, Some(274));
        assert_eq!(options.left_handed, Some(false));
        assert_eq!(options.disable_while_typing, Some(true));
        assert_eq!(options.middle_emulation, Some(false));
        let options = InputOptions::parse(&[
            ("accel-speed".into(), SpecValue::Int(0)),
            ("scroll-method".into(), SpecValue::Str("none".into())),
            ("scroll-button".into(), SpecValue::Int(0)),
        ])
        .unwrap();
        assert_eq!(options.accel_speed, Some(0.0));
        assert_eq!(options.scroll_method, Some(ScrollMethod::NoScroll));
        assert_eq!(options.scroll_button, Some(0));
        assert_eq!(options.left_handed, None);
        assert_eq!(InputOptions::parse(&[]).unwrap(), InputOptions::default());
    }

    #[test]
    fn invalid_options_reject_the_whole_request() {
        for (key, value) in [
            ("accel-speed", SpecValue::Real(f64::NAN)),
            ("accel-speed", SpecValue::Real(f64::INFINITY)),
            ("accel-speed", SpecValue::Real(-1.01)),
            ("accel-speed", SpecValue::Int(2)),
            ("scroll-method", SpecValue::Sym("twofinger".into())),
            ("scroll-button", SpecValue::Int(-1)),
            ("scroll-button", SpecValue::Int(0x300)),
            ("scroll-button", SpecValue::Real(274.0)),
            ("left-handed", SpecValue::Int(1)),
            ("disable-while-typing", SpecValue::Sym("yes".into())),
            ("middle-emulation", SpecValue::List(vec![])),
            ("typo", SpecValue::Bool(true)),
        ] {
            assert!(
                InputOptions::parse(&[(key.into(), value)]).is_err(),
                "{key}"
            );
        }
        assert!(
            InputOptions::parse(&[
                ("left-handed".into(), SpecValue::Bool(true)),
                ("left-handed".into(), SpecValue::Bool(false)),
            ])
            .is_err()
        );
    }
}
