// SPDX-License-Identifier: GPL-3.0-or-later

//! The Scheme-facing output configuration surface: `(wm-configure-output!
//! name alist)` and `(wm-output-heads)`.
//!
//! The alist parser is split from the FFI so it is a pure, unit-testable
//! function over [`SpecValue`] (a tiny Scheme-datum mirror); the gsubr in
//! `guile/mod.rs` only converts SCM to [`SpecValue`] and forwards the
//! parsed [`OutputChangeSpec`] as `WmCommand::ConfigureOutput`. The
//! compositor thread resolves the head by name and applies it through the
//! same `apply_output_configuration` the wlr-output-management protocol
//! uses (`src/handlers/output_management.rs`).

use smithay::utils::Transform;

/// A Scheme datum as far as the output configuration alist is concerned.
#[derive(Clone, Debug, PartialEq)]
pub enum SpecValue {
    Bool(bool),
    Int(i64),
    Real(f64),
    Str(String),
    Sym(String),
    List(Vec<SpecValue>),
}

impl std::fmt::Display for SpecValue {
    /// Scheme `write` syntax, for error messages.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpecValue::Bool(true) => f.write_str("#t"),
            SpecValue::Bool(false) => f.write_str("#f"),
            SpecValue::Int(i) => write!(f, "{i}"),
            SpecValue::Real(r) => write!(f, "{r:?}"),
            SpecValue::Str(s) => write!(f, "{s:?}"),
            SpecValue::Sym(s) => f.write_str(s),
            SpecValue::List(items) => {
                f.write_str("(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(" ")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str(")")
            }
        }
    }
}

/// The parsed, backend-independent request behind `configure-output!`.
/// Every field is optional; `None` leaves the property untouched.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutputChangeSpec {
    /// `(width, height, refresh_mHz)`; refresh 0 = any refresh of that size.
    pub mode: Option<(i32, i32, i32)>,
    pub position: Option<(i32, i32)>,
    pub scale: Option<f64>,
    pub transform: Option<Transform>,
    pub enabled: Option<bool>,
    pub adaptive_sync: Option<bool>,
}

/// One head as reported by `(wm-output-heads)`: a snapshot of every known
/// head (enabled or not) pushed by the compositor on every
/// output-management refresh, so Scheme reads never touch `MindeState`.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputHeadInfo {
    pub name: String,
    pub enabled: bool,
    pub make: String,
    pub model: String,
    pub serial: String,
    pub description: String,
    /// `(w, h, refresh_mHz)` of the current mode, if any.
    pub current_mode: Option<(i32, i32, i32)>,
    pub preferred_mode: Option<(i32, i32, i32)>,
    pub position: (i32, i32),
    pub scale: f64,
    pub transform: Transform,
    /// `Some(on)` where the backend supports VRR, `None` where it cannot.
    pub adaptive_sync: Option<bool>,
    /// Every advertised mode as `(w, h, refresh_mHz)`.
    pub modes: Vec<(i32, i32, i32)>,
}

/// The Scheme symbol name for a transform (`wlr-randr` spelling).
pub fn transform_name(t: Transform) -> &'static str {
    match t {
        Transform::Normal => "normal",
        Transform::_90 => "90",
        Transform::_180 => "180",
        Transform::_270 => "270",
        Transform::Flipped => "flipped",
        Transform::Flipped90 => "flipped-90",
        Transform::Flipped180 => "flipped-180",
        Transform::Flipped270 => "flipped-270",
    }
}

/// Parses a transform name (`normal`, `90`, `flipped-90`, ...; also
/// `flipped90` and `rotate-90` spellings).
pub fn parse_transform(name: &str) -> Option<Transform> {
    let n = name.trim().to_ascii_lowercase();
    let n = n.strip_prefix("rotate-").unwrap_or(&n).replace('_', "-");
    Some(match n.as_str() {
        "normal" | "0" => Transform::Normal,
        "90" => Transform::_90,
        "180" => Transform::_180,
        "270" => Transform::_270,
        "flipped" | "flipped-0" | "flipped0" => Transform::Flipped,
        "flipped-90" | "flipped90" => Transform::Flipped90,
        "flipped-180" | "flipped180" => Transform::Flipped180,
        "flipped-270" | "flipped270" => Transform::Flipped270,
        _ => return None,
    })
}

fn int_of(v: &SpecValue, what: &str) -> Result<i32, String> {
    match v {
        SpecValue::Int(i) => i32::try_from(*i).map_err(|_| format!("{what}: {i} out of range")),
        SpecValue::Real(r) if r.fract() == 0.0 => Ok(*r as i32),
        other => Err(format!("{what}: expected an integer, got {other}")),
    }
}

fn real_of(v: &SpecValue, what: &str) -> Result<f64, String> {
    match v {
        SpecValue::Int(i) => Ok(*i as f64),
        SpecValue::Real(r) => Ok(*r),
        other => Err(format!("{what}: expected a number, got {other}")),
    }
}

fn bool_of(v: &SpecValue, what: &str) -> Result<bool, String> {
    match v {
        SpecValue::Bool(b) => Ok(*b),
        other => Err(format!("{what}: expected #t or #f, got {other}")),
    }
}

/// `"1920x1080@59.94"` / `"1920x1080@60Hz"` / `"1920x1080"` -> `(w, h, mHz)`
/// (refresh 0 when omitted).
pub fn parse_mode_string(s: &str) -> Result<(i32, i32, i32), String> {
    let s = s.trim();
    let (size, refresh) = match s.split_once('@') {
        Some((size, r)) => (size, Some(r)),
        None => (s, None),
    };
    let (w, h) = size
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("mode {s:?}: expected WIDTHxHEIGHT[@REFRESH]"))?;
    let w: i32 = w
        .trim()
        .parse()
        .map_err(|_| format!("mode {s:?}: bad width"))?;
    let h: i32 = h
        .trim()
        .parse()
        .map_err(|_| format!("mode {s:?}: bad height"))?;
    let mhz = match refresh {
        None => 0,
        Some(r) => {
            let r = r.trim();
            let r = r
                .strip_suffix("Hz")
                .or_else(|| r.strip_suffix("hz"))
                .or_else(|| r.strip_suffix("HZ"))
                .unwrap_or(r)
                .trim();
            let hz: f64 = r
                .parse()
                .map_err(|_| format!("mode {s:?}: bad refresh rate"))?;
            (hz * 1000.0).round() as i32
        }
    };
    if w <= 0 || h <= 0 {
        return Err(format!("mode {s:?}: size must be positive"));
    }
    Ok((w, h, mhz))
}

/// Parses the `configure-output!` alist. Keys (symbols or strings):
/// `mode` (`(w h refresh-mhz)`, `(w h)` or `"WxH@R"`), `position`
/// (`(x y)`), `scale` (positive real), `transform` (symbol/string name or
/// integer degrees), `enabled` and `adaptive-sync` (booleans). Unknown
/// keys and ill-typed values are errors so typos never silently no-op.
pub fn parse_output_change_spec(
    entries: &[(String, SpecValue)],
) -> Result<OutputChangeSpec, String> {
    let mut spec = OutputChangeSpec::default();
    for (key, value) in entries {
        match key.as_str() {
            "mode" => {
                spec.mode = Some(match value {
                    SpecValue::Str(s) => parse_mode_string(s)?,
                    SpecValue::Sym(s) => parse_mode_string(s)?,
                    SpecValue::List(items) if items.len() == 2 || items.len() == 3 => {
                        let w = int_of(&items[0], "mode width")?;
                        let h = int_of(&items[1], "mode height")?;
                        let r = match items.get(2) {
                            Some(r) => int_of(r, "mode refresh (mHz)")?,
                            None => 0,
                        };
                        if w <= 0 || h <= 0 {
                            return Err("mode: size must be positive".into());
                        }
                        (w, h, r)
                    }
                    other => {
                        return Err(format!(
                            "mode: expected (w h refresh-mhz) or \"WxH@R\", got {other}"
                        ));
                    }
                });
            }
            "position" => {
                spec.position = Some(match value {
                    SpecValue::List(items) if items.len() == 2 => (
                        int_of(&items[0], "position x")?,
                        int_of(&items[1], "position y")?,
                    ),
                    other => return Err(format!("position: expected (x y), got {other}")),
                });
            }
            "scale" => {
                let s = real_of(value, "scale")?;
                if s <= 0.0 || !s.is_finite() {
                    return Err(format!("scale: must be a positive number, got {s}"));
                }
                spec.scale = Some(s);
            }
            "transform" => {
                spec.transform = Some(match value {
                    SpecValue::Sym(s) | SpecValue::Str(s) => parse_transform(s)
                        .ok_or_else(|| format!("transform: unknown transform {s:?}"))?,
                    SpecValue::Int(i) => parse_transform(&i.to_string())
                        .ok_or_else(|| format!("transform: unknown rotation {i}"))?,
                    other => {
                        return Err(format!("transform: expected a symbol, got {other}"));
                    }
                });
            }
            "enabled" => spec.enabled = Some(bool_of(value, "enabled")?),
            "adaptive-sync" => spec.adaptive_sync = Some(bool_of(value, "adaptive-sync")?),
            other => return Err(format!("unknown output setting {other:?}")),
        }
    }
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(k: &str, v: SpecValue) -> (String, SpecValue) {
        (k.to_string(), v)
    }

    #[test]
    fn empty_alist_is_a_noop_spec() {
        assert_eq!(
            parse_output_change_spec(&[]),
            Ok(OutputChangeSpec::default())
        );
    }

    #[test]
    fn parses_every_key() {
        let spec = parse_output_change_spec(&[
            e(
                "mode",
                SpecValue::List(vec![
                    SpecValue::Int(1920),
                    SpecValue::Int(1080),
                    SpecValue::Int(60000),
                ]),
            ),
            e(
                "position",
                SpecValue::List(vec![SpecValue::Int(1920), SpecValue::Int(0)]),
            ),
            e("scale", SpecValue::Real(1.5)),
            e("transform", SpecValue::Sym("flipped-90".into())),
            e("enabled", SpecValue::Bool(true)),
            e("adaptive-sync", SpecValue::Bool(false)),
        ])
        .unwrap();
        assert_eq!(
            spec,
            OutputChangeSpec {
                mode: Some((1920, 1080, 60000)),
                position: Some((1920, 0)),
                scale: Some(1.5),
                transform: Some(Transform::Flipped90),
                enabled: Some(true),
                adaptive_sync: Some(false),
            }
        );
    }

    #[test]
    fn mode_strings_and_short_lists() {
        assert_eq!(
            parse_mode_string("1920x1080@59.94"),
            Ok((1920, 1080, 59940))
        );
        assert_eq!(parse_mode_string("3840x2160@60Hz"), Ok((3840, 2160, 60000)));
        assert_eq!(parse_mode_string("1280X720"), Ok((1280, 720, 0)));
        assert!(parse_mode_string("1920").is_err());
        assert!(parse_mode_string("0x1080").is_err());
        let spec =
            parse_output_change_spec(&[e("mode", SpecValue::Str("1920x1080@60".into()))]).unwrap();
        assert_eq!(spec.mode, Some((1920, 1080, 60000)));
        let spec = parse_output_change_spec(&[e(
            "mode",
            SpecValue::List(vec![SpecValue::Int(1920), SpecValue::Int(1080)]),
        )])
        .unwrap();
        assert_eq!(spec.mode, Some((1920, 1080, 0)));
    }

    #[test]
    fn integer_scale_and_degree_transforms() {
        let spec = parse_output_change_spec(&[
            e("scale", SpecValue::Int(2)),
            e("transform", SpecValue::Int(270)),
        ])
        .unwrap();
        assert_eq!(spec.scale, Some(2.0));
        assert_eq!(spec.transform, Some(Transform::_270));
        assert_eq!(parse_transform("normal"), Some(Transform::Normal));
        assert_eq!(parse_transform("flipped180"), Some(Transform::Flipped180));
        assert_eq!(parse_transform("rotate-90"), Some(Transform::_90));
        assert_eq!(parse_transform("45"), None);
    }

    #[test]
    fn transform_names_round_trip() {
        for t in [
            Transform::Normal,
            Transform::_90,
            Transform::_180,
            Transform::_270,
            Transform::Flipped,
            Transform::Flipped90,
            Transform::Flipped180,
            Transform::Flipped270,
        ] {
            assert_eq!(parse_transform(transform_name(t)), Some(t));
        }
    }

    #[test]
    fn rejects_bad_values_and_unknown_keys() {
        assert!(parse_output_change_spec(&[e("scale", SpecValue::Int(0))]).is_err());
        assert!(parse_output_change_spec(&[e("scale", SpecValue::Str("2".into()))]).is_err());
        assert!(parse_output_change_spec(&[e("enabled", SpecValue::Int(1))]).is_err());
        assert!(
            parse_output_change_spec(&[e("position", SpecValue::List(vec![SpecValue::Int(1)]))])
                .is_err()
        );
        assert!(
            parse_output_change_spec(&[e("transform", SpecValue::Sym("upside".into()))]).is_err()
        );
        assert!(parse_output_change_spec(&[e("scael", SpecValue::Int(2))]).is_err());
    }
}
