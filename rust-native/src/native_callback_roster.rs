//! Bounded callback declarations shared by the owned-host producer and reader.
//! A decoded declaration is transport, never compiled-code or callback authority.
//! Actual symbol/DWARF mapping, loaded-image identity and object provenance must
//! be qualified separately before any entry can be called.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

pub const SCHEMA: &str = "ruycast.f1.native-callbacks.v1";
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub path: String,
    pub sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        let binding = serde_json::json!({"path":"/owned/input", "sha256":"a".repeat(64)});
        let function =
            serde_json::json!({"symbol":"fixture_entry", "image_offset":16, "evidence_index":0});
        serde_json::json!({
            "schema":SCHEMA, "native":binding, "native_source_manifest":binding,
            "compiler_receipt":binding, "compiler_arguments_receipt":binding,
            "objc2_rlib":binding, "objc2_define_class_source":binding,
            "objc2_defined_ivars_source":binding, "debug_evidence":[binding],
            "types":[{"rust_type":"fixture::Target", "objc_name":"FixtureTarget",
                "superclass":"NSObject", "source":binding,
                "ivars_layout":{"size":16,"align":8,"needs_drop":true,"drop_flag":true},
                "object_needs_drop":false,"registration":function,
                "methods":[{"selector":"invoke:", "encoding":"v@:@", "class_method":false,
                    "abi":"CUnwind", "entry":function}], "drop_glue":[function]}]
        })
    }
    fn decode(value: serde_json::Value) -> Result<Declaration> {
        Declaration::decode(&serde_json::to_vec(&value).unwrap())
    }
    #[test]
    fn declaration_requires_explicit_abi_role_and_cleanup_facts() {
        assert!(decode(fixture()).is_ok());
        for key in ["class_method", "abi", "entry"] {
            let mut value = fixture();
            value["types"][0]["methods"][0]
                .as_object_mut()
                .unwrap()
                .remove(key);
            assert!(decode(value).is_err(), "missing method field {key}");
        }
        let mut value = fixture();
        value["types"][0]["drop_glue"] = serde_json::json!([]);
        assert!(decode(value).is_err());
        let mut value = fixture();
        value["types"][0]["ivars_layout"]["drop_flag"] = false.into();
        assert!(decode(value).is_err());
        let mut value = fixture();
        value["types"][0]["object_needs_drop"] = true.into();
        value["types"][0]["ivars_layout"]["needs_drop"] = false.into();
        value["types"][0]["drop_glue"] = serde_json::json!([]);
        assert!(decode(value).is_err());
    }
    #[test]
    fn rejects_ambiguous_selectors_evidence_and_unbounded_inputs() {
        let mut value = fixture();
        let duplicate = value["types"][0]["methods"][0].clone();
        value["types"][0]["methods"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(decode(value.clone()).is_err());
        value["types"][0]["methods"][1]["class_method"] = true.into();
        assert!(decode(value).is_ok());
        for field in ["image_offset", "evidence_index"] {
            let mut value = fixture();
            value["types"][0]["registration"][field] = if field == "image_offset" {
                0.into()
            } else {
                1.into()
            };
            assert!(decode(value).is_err());
        }
        let mut value = fixture();
        value["native"]["path"] = "/owned/../other".into();
        assert!(decode(value).is_err());
        let mut value = fixture();
        value["native"]["extra"] = true.into();
        assert!(decode(value).is_err());
        assert!(Declaration::decode(&vec![b' '; MAX_BYTES + 1]).is_err());
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionPin {
    pub symbol: String,
    pub image_offset: u64,
    pub evidence_index: u32,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum Abi {
    C,
    CUnwind,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MethodPin {
    pub selector: String,
    pub encoding: String,
    pub class_method: bool,
    pub abi: Abi,
    pub entry: FunctionPin,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IvarsLayout {
    pub size: u64,
    pub align: u64,
    pub needs_drop: bool,
    pub drop_flag: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeType {
    pub rust_type: String,
    /// Informational name; it cannot establish class/code authority.
    pub objc_name: String,
    pub superclass: String,
    pub source: Binding,
    pub ivars_layout: IvarsLayout,
    /// Distinct from whether the ivars require Drop in objc2::setup_dealloc.
    pub object_needs_drop: bool,
    pub registration: FunctionPin,
    pub methods: Vec<MethodPin>,
    pub drop_glue: Vec<FunctionPin>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub schema: String,
    pub native: Binding,
    pub native_source_manifest: Binding,
    pub compiler_receipt: Binding,
    pub compiler_arguments_receipt: Binding,
    pub objc2_rlib: Binding,
    pub objc2_define_class_source: Binding,
    pub objc2_defined_ivars_source: Binding,
    pub debug_evidence: Vec<Binding>,
    pub types: Vec<NativeType>,
}
type Result<T> = std::result::Result<T, String>;
fn text(value: &str, limit: usize) -> Result<()> {
    if value.is_empty() || value.len() > limit || value.contains('\0') {
        return Err("Callback declaration text bound differs.".into());
    }
    Ok(())
}
impl Binding {
    pub fn check(&self) -> Result<()> {
        text(&self.path, 32_768)?;
        let path = Path::new(&self.path);
        if !path.is_absolute()
            || path
                .components()
                .any(|p| !matches!(p, Component::RootDir | Component::Normal(_)))
            || self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("Callback declaration binding differs.".into());
        }
        Ok(())
    }
}
impl FunctionPin {
    fn check(&self, evidence: usize) -> Result<()> {
        text(&self.symbol, 8192)?;
        if self.image_offset == 0 || self.evidence_index as usize >= evidence {
            return Err("Callback declaration function/evidence bound differs.".into());
        }
        Ok(())
    }
}
impl Declaration {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err("Callback declaration byte bound differs.".into());
        }
        let value: Self =
            serde_json::from_slice(bytes).map_err(|_| "Callback declaration schema differs.")?;
        value.check()?;
        Ok(value)
    }
    /// Only validates the declared structure. In particular, matching a symbol
    /// label or an executable offset is NOT evidence that the compiler emitted
    /// that entry for this type or that its transitive cleanup is safe.
    pub fn check(&self) -> Result<()> {
        if self.schema != SCHEMA
            || self.types.is_empty()
            || self.types.len() > 128
            || self.debug_evidence.is_empty()
            || self.debug_evidence.len() > 32
        {
            return Err("Callback declaration roster bound differs.".into());
        }
        for binding in self.bindings() {
            binding.check()?;
        }
        let mut names = BTreeSet::new();
        let mut functions = 0usize;
        for ty in &self.types {
            text(&ty.rust_type, 1024)?;
            text(&ty.objc_name, 1024)?;
            text(&ty.superclass, 256)?;
            if !names.insert(&ty.rust_type)
                || ty.methods.is_empty()
                || ty.methods.len() > 128
                || !ty.ivars_layout.align.is_power_of_two()
                || ty.ivars_layout.align > 4096
                || ty.ivars_layout.size > 16 * 1024 * 1024
                || ((ty.object_needs_drop || ty.ivars_layout.needs_drop)
                    && (!ty.ivars_layout.drop_flag || ty.drop_glue.is_empty()))
            {
                return Err("Callback declaration type/layout differs.".into());
            }
            functions = functions
                .checked_add(1 + ty.methods.len() + ty.drop_glue.len())
                .ok_or("Callback declaration function count overflow.")?;
            if functions > 2048 {
                return Err("Callback declaration function count exceeds bound.".into());
            }
            ty.registration.check(self.debug_evidence.len())?;
            for pin in &ty.drop_glue {
                pin.check(self.debug_evidence.len())?;
            }
            let mut selectors = BTreeSet::new();
            for method in &ty.methods {
                text(&method.selector, 256)?;
                text(&method.encoding, 512)?;
                if !selectors.insert((method.class_method, &method.selector)) {
                    return Err("Callback declaration repeats a selector.".into());
                }
                method.entry.check(self.debug_evidence.len())?;
            }
        }
        Ok(())
    }
    pub fn bindings(&self) -> Vec<&Binding> {
        let mut result = vec![
            &self.native,
            &self.native_source_manifest,
            &self.compiler_receipt,
            &self.compiler_arguments_receipt,
            &self.objc2_rlib,
            &self.objc2_define_class_source,
            &self.objc2_defined_ivars_source,
        ];
        result.extend(self.debug_evidence.iter());
        result.extend(self.types.iter().map(|ty| &ty.source));
        result
    }
}
