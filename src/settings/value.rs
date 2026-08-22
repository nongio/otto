//! Setting values, and the three representations they have to live in: the
//! schema's type names, TOML (the configuration file), and D-Bus variants.

use zbus::zvariant::{OwnedValue, Value};

/// The types a setting can have. The wire names are part of the contract in
/// `docs/developer/settings-dbus-api.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingType {
    Bool,
    Int,
    Double,
    Str,
    /// A string constrained to the schema's `choices`.
    Enum,
    StrList,
}

impl SettingType {
    pub fn wire_name(self) -> &'static str {
        match self {
            SettingType::Bool => "bool",
            SettingType::Int => "int",
            SettingType::Double => "double",
            SettingType::Str => "string",
            SettingType::Enum => "enum",
            SettingType::StrList => "string-list",
        }
    }

    /// The type a value of this setting actually carries: an enum is a string
    /// on the bus and in the file, it is only the schema that narrows it.
    pub fn wire_repr(self) -> SettingType {
        match self {
            SettingType::Enum => SettingType::Str,
            other => other,
        }
    }
}

/// One setting's value.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Int(i64),
    Double(f64),
    Str(String),
    StrList(Vec<String>),
}

impl SettingValue {
    pub fn ty(&self) -> SettingType {
        match self {
            SettingValue::Bool(_) => SettingType::Bool,
            SettingValue::Int(_) => SettingType::Int,
            SettingValue::Double(_) => SettingType::Double,
            SettingValue::Str(_) => SettingType::Str,
            SettingValue::StrList(_) => SettingType::StrList,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            SettingValue::Str(text) => Some(text),
            _ => None,
        }
    }

    /// The numeric value, for range checking.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            SettingValue::Int(number) => Some(*number as f64),
            SettingValue::Double(number) => Some(*number),
            _ => None,
        }
    }

    pub fn to_toml(&self) -> toml::Value {
        match self {
            SettingValue::Bool(flag) => toml::Value::Boolean(*flag),
            SettingValue::Int(number) => toml::Value::Integer(*number),
            SettingValue::Double(number) => toml::Value::Float(*number),
            SettingValue::Str(text) => toml::Value::String(text.clone()),
            SettingValue::StrList(items) => toml::Value::Array(
                items
                    .iter()
                    .map(|item| toml::Value::String(item.clone()))
                    .collect(),
            ),
        }
    }

    /// Read a value of type `ty` out of TOML.
    ///
    /// This direction *does* widen an integer to a double: `size = 1` in a
    /// hand-written file is the same setting as `size = 1.0`, and the schema
    /// decides which type the client sees.
    pub fn from_toml(value: &toml::Value, ty: SettingType) -> Option<Self> {
        Some(match (ty.wire_repr(), value) {
            (SettingType::Bool, toml::Value::Boolean(flag)) => SettingValue::Bool(*flag),
            (SettingType::Int, toml::Value::Integer(number)) => SettingValue::Int(*number),
            (SettingType::Double, toml::Value::Float(number)) => SettingValue::Double(*number),
            (SettingType::Double, toml::Value::Integer(number)) => {
                SettingValue::Double(*number as f64)
            }
            (SettingType::Str, toml::Value::String(text)) => SettingValue::Str(text.clone()),
            (SettingType::StrList, toml::Value::Array(items)) => SettingValue::StrList(
                items
                    .iter()
                    .map(|item| item.as_str().map(str::to_string))
                    .collect::<Option<_>>()?,
            ),
            _ => return None,
        })
    }

    /// The value a client sees when the configuration has nothing to say — an
    /// unset `Option<String>`, most often.
    pub fn empty(ty: SettingType) -> Self {
        match ty.wire_repr() {
            SettingType::Bool => SettingValue::Bool(false),
            SettingType::Int => SettingValue::Int(0),
            SettingType::Double => SettingValue::Double(0.0),
            SettingType::StrList => SettingValue::StrList(Vec::new()),
            _ => SettingValue::Str(String::new()),
        }
    }

    /// Decode a D-Bus variant. Integer widths are accepted interchangeably —
    /// clients differ on which one they send — but the type *category* is not
    /// coerced: a string never becomes a number.
    pub fn from_variant(value: &Value<'_>) -> Option<Self> {
        Some(match value {
            Value::Value(inner) => Self::from_variant(inner)?,
            Value::Bool(flag) => SettingValue::Bool(*flag),
            Value::U8(number) => SettingValue::Int(*number as i64),
            Value::I16(number) => SettingValue::Int(*number as i64),
            Value::U16(number) => SettingValue::Int(*number as i64),
            Value::I32(number) => SettingValue::Int(*number as i64),
            Value::U32(number) => SettingValue::Int(*number as i64),
            Value::I64(number) => SettingValue::Int(*number),
            Value::U64(number) => SettingValue::Int(i64::try_from(*number).ok()?),
            Value::F64(number) => SettingValue::Double(*number),
            Value::Str(text) => SettingValue::Str(text.to_string()),
            Value::Array(items) => SettingValue::StrList(
                items
                    .iter()
                    .map(|item| match item {
                        Value::Str(text) => Some(text.to_string()),
                        _ => None,
                    })
                    .collect::<Option<_>>()?,
            ),
            _ => return None,
        })
    }

    /// Encode for the bus. Integers go out as `i`, per the contract.
    pub fn to_variant(&self) -> OwnedValue {
        let value = match self {
            SettingValue::Bool(flag) => Value::from(*flag),
            SettingValue::Int(number) => Value::from(*number as i32),
            SettingValue::Double(number) => Value::from(*number),
            SettingValue::Str(text) => Value::from(text.clone()),
            SettingValue::StrList(items) => Value::from(items.clone()),
        };
        OwnedValue::try_from(value).unwrap_or_else(|_| {
            // Every arm above is a plain scalar or an array of strings, none of
            // which can fail to become owned; fall back rather than panic in a
            // D-Bus handler.
            OwnedValue::try_from(Value::from(String::new())).expect("a string is always ownable")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_round_trips() {
        let values = [
            (SettingValue::Bool(true), SettingType::Bool),
            (SettingValue::Int(7), SettingType::Int),
            (SettingValue::Double(1.25), SettingType::Double),
            (SettingValue::Str("left".into()), SettingType::Enum),
            (
                SettingValue::StrList(vec!["en".into(), "it".into()]),
                SettingType::StrList,
            ),
        ];
        for (value, ty) in values {
            let toml = value.to_toml();
            assert_eq!(SettingValue::from_toml(&toml, ty), Some(value));
        }
    }

    #[test]
    fn a_whole_number_in_the_file_reads_as_a_double() {
        assert_eq!(
            SettingValue::from_toml(&toml::Value::Integer(1), SettingType::Double),
            Some(SettingValue::Double(1.0))
        );
    }

    #[test]
    fn mismatched_toml_is_not_coerced() {
        assert_eq!(
            SettingValue::from_toml(&toml::Value::String("1".into()), SettingType::Double),
            None
        );
        assert_eq!(
            SettingValue::from_toml(&toml::Value::Float(1.0), SettingType::Int),
            None
        );
    }

    #[test]
    fn variants_decode_by_category() {
        assert_eq!(
            SettingValue::from_variant(&Value::from(1.25f64)),
            Some(SettingValue::Double(1.25))
        );
        assert_eq!(
            SettingValue::from_variant(&Value::from(3i32)),
            Some(SettingValue::Int(3))
        );
        assert_eq!(
            SettingValue::from_variant(&Value::from("left")),
            Some(SettingValue::Str("left".into()))
        );
        assert_eq!(
            SettingValue::from_variant(&Value::from(vec!["en".to_string()])),
            Some(SettingValue::StrList(vec!["en".into()]))
        );
    }
}
