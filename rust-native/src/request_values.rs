//! Preserve Foundation/Swift dictionary ownership for execute.values.
//! Canonical duplicates collapse in received order before serde_json's sorted
//! object map discards that order. The existing CLI serializer is unchanged.
use serde::{
    de::{self, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use serde_json::{Map, Value};
use std::fmt;

enum Values {
    Object(Vec<(String, Value)>),
    Other(Value),
}
impl Values {
    fn materialize(self, canonical: bool) -> Value {
        match self {
            Self::Other(value) => value,
            Self::Object(pairs) => {
                let mut object = Map::new();
                for (key, value) in pairs {
                    object.insert(
                        if canonical {
                            objc2::rc::autoreleasepool(|_| crate::templates::canonical(&key))
                        } else {
                            key
                        },
                        value,
                    );
                }
                Value::Object(object)
            }
        }
    }
}
impl<'de> Deserialize<'de> for Values {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Values;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a JSON value")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Values, M::Error> {
                let mut pairs = Vec::new();
                while let Some(pair) = map.next_entry::<String, Value>()? {
                    pairs.push(pair);
                }
                Ok(Values::Object(pairs))
            }
            fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Values, S::Error> {
                let mut v = vec![];
                while let Some(x) = seq.next_element::<Value>()? {
                    v.push(x);
                }
                Ok(Values::Other(Value::Array(v)))
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Values, E> {
                Ok(Values::Other(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Values, E> {
                Ok(Values::Other(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Values, E> {
                Ok(Values::Other(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Values, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| Values::Other(Value::Number(n)))
                    .ok_or_else(|| E::custom("Invalid JSON number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Values, E> {
                Ok(Values::Other(v.into()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Values, E> {
                Ok(Values::Other(v.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Values, E> {
                Ok(Values::Other(Value::Null))
            }
        }
        d.deserialize_any(V)
    }
}
struct Incoming(Value);
impl<'de> Deserialize<'de> for Incoming {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Request;
        impl<'de> Visitor<'de> for Request {
            type Value = Incoming;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a request object")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Incoming, M::Error> {
                let mut object = Map::new();
                let mut values = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key == "values" {
                        values = Some(map.next_value::<Values>()?);
                    } else {
                        object.insert(key, map.next_value::<Value>()?);
                    }
                }
                if let Some(values) = values {
                    let canonical = object.get("op").and_then(Value::as_str) == Some("execute");
                    object.insert("values".into(), values.materialize(canonical));
                }
                Ok(Incoming(Value::Object(object)))
            }
        }
        d.deserialize_map(Request)
    }
}
pub(crate) fn decode(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    serde_json::from_slice::<Incoming>(bytes).map(|v| v.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn received_order_not_sorted_order_owns_canonical_duplicates() {
        for (pairs, expected) in [
            (r#""café":"first","cafe\u0301":"last""#, "last"),
            (r#""cafe\u0301":"first","café":"last""#, "last"),
        ] {
            let wire = format!(r#"{{"values":{{{pairs}}},"op":"execute"}}"#);
            let v = decode(wire.as_bytes()).unwrap();
            assert_eq!(v["values"]["café"], expected);
            assert_eq!(v["values"].as_object().unwrap().len(), 1);
        }
    }
    #[test]
    fn only_execute_values_are_canonicalized_and_last_outer_values_wins() {
        let v =
            decode(r#"{"op":"other","values":{"café":"a","cafe\u0301":"b"}}"#.as_bytes()).unwrap();
        assert_eq!(v["values"].as_object().unwrap().len(), 2);
        let v = decode(r#"{"op":"execute","values":{"old":"a"},"values":{"new":"b"}}"#.as_bytes())
            .unwrap();
        assert!(v["values"].get("old").is_none());
        assert_eq!(v["values"]["new"], "b");
    }
    #[test]
    fn packet_sized_duplicate_runs_and_depth_keep_existing_parser_bounds() {
        let pairs = (0..10000)
            .map(|i| format!("\"cafe\\u0301\":\"{i}\""))
            .collect::<Vec<_>>()
            .join(",");
        let wire = format!(r#"{{"op":"execute","values":{{{pairs}}}}}"#);
        assert!(wire.len() < 1048576);
        let v = decode(wire.as_bytes()).unwrap();
        assert_eq!(v["values"]["café"], "9999");
        assert_eq!(v["values"].as_object().unwrap().len(), 1);
        for depth in [120, 128, 160] {
            let wire = format!(
                r#"{{"op":"execute","values":{}0{}}}"#,
                "[".repeat(depth),
                "]".repeat(depth)
            );
            assert_eq!(
                decode(wire.as_bytes()).is_ok(),
                serde_json::from_str::<Value>(&wire).is_ok()
            );
        }
    }
    #[test]
    fn shapes_and_strict_json_framing_remain_owned_by_existing_layers() {
        for shape in [
            "null",
            "true",
            "2",
            "-2",
            "2.5",
            r#""literal""#,
            r#"[1,"a"]"#,
            r#"{"a":false}"#,
        ] {
            let wire = format!(r#"{{"op":"execute","values":{shape}}}"#);
            let expected: Value = serde_json::from_str(&wire).unwrap();
            assert_eq!(decode(wire.as_bytes()).unwrap(), expected);
        }
        assert!(decode(b"{}{}\n").is_err());
        assert!(decode(b"[]\n").is_err());
        assert!(decode(b"{broken}\n").is_err());
    }
}
