//! Static compiler evidence only. No class lookup, IMP reflection, callback,
//! loader, Source14 admission or SDK operation is performed by this module.
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub type Result<T> = std::result::Result<T, String>;
const MAX_CAPTURE: usize = 32 * 1024 * 1024;
const MAX_DIES: usize = 300_000;
pub const TARGET: &str = "ruycast_native::native_menu_bar::Target";
pub const IVARS: &str = "ruycast_native::native_menu_bar::Binding";

#[derive(Clone, Debug)]
pub struct Die {
    pub offset: u64,
    pub tag: String,
    pub parent: Option<usize>,
    pub attributes: BTreeMap<String, String>,
}
fn hex(value: &str) -> Result<u64> {
    u64::from_str_radix(value.trim().strip_prefix("0x").unwrap_or(value.trim()), 16)
        .map_err(|_| "compiler capture hexadecimal value differs".into())
}
fn quoted(value: &str) -> Result<String> {
    let start = value.find('"').ok_or("DWARF quoted value absent")?;
    let end = value.rfind('"').ok_or("DWARF quoted terminator absent")?;
    if end <= start {
        return Err("DWARF quoted value empty".into());
    }
    let value = &value[start + 1..end];
    if value.contains('\\') || value.contains('\0') || value.len() > 16_384 {
        return Err("unsupported escaped/bounded DWARF identifier".into());
    }
    Ok(value.into())
}
impl Die {
    fn name(&self) -> Result<String> {
        quoted(
            self.attributes
                .get("DW_AT_name")
                .ok_or("DWARF name absent")?,
        )
    }
    fn number(&self, key: &str) -> Result<u64> {
        let value = self
            .attributes
            .get(key)
            .ok_or("DWARF numeric attribute absent")?;
        let value = value
            .trim()
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .ok_or("DWARF numeric syntax differs")?;
        if value.starts_with("0x") {
            hex(value)
        } else {
            value
                .parse()
                .map_err(|_| "DWARF decimal value differs".into())
        }
    }
}
/// Parses the full bounded output of Apple's dwarfdump --debug-info on an
/// actual compiler object. Namespace ancestry is retained; a bare Target name
/// cannot accidentally select actions::Target or keyword_target::Target.
pub fn dwarf_dies(text: &str) -> Result<Vec<Die>> {
    if text.len() > MAX_CAPTURE {
        return Err("DWARF capture exceeds bound".into());
    }
    let mut dies = Vec::<Die>::new();
    let mut stack = Vec::<(usize, usize)>::new();
    let mut current = None;
    let mut offsets = BTreeSet::new();
    for line in text.lines() {
        if line.starts_with("0x") {
            let (address, body) = line.split_once(':').ok_or("DWARF entry separator absent")?;
            let offset = hex(address)?;
            if body.trim_start().starts_with("Compile Unit:") {
                stack.clear();
                current = None;
                continue;
            }
            let indentation = body.len() - body.trim_start().len();
            while stack.last().is_some_and(|(depth, _)| *depth >= indentation) {
                stack.pop();
            }
            let tag = body
                .split_whitespace()
                .next()
                .ok_or("empty DWARF entry")?;
            if tag == "NULL" {
                current = None;
                continue;
            }
            if !tag.starts_with("DW_TAG_")
                || dies.len() >= MAX_DIES
                || dies.last().is_some_and(|die| die.offset >= offset)
                || !offsets.insert(offset)
            {
                return Err("DWARF entry/tag/offset bound differs".into());
            }
            let index = dies.len();
            dies.push(Die {
                offset,
                tag: tag.into(),
                parent: stack.last().map(|(_, i)| *i),
                attributes: BTreeMap::new(),
            });
            stack.push((indentation, index));
            current = Some(index);
        } else if line.trim_start().starts_with("DW_AT_") {
            let index = current.ok_or("DWARF attribute has no entry")?;
            let line = line.trim();
            let end = line
                .find(char::is_whitespace)
                .ok_or("DWARF attribute syntax differs")?;
            if dies[index]
                .attributes
                .insert(line[..end].into(), line[end..].trim().into())
                .is_some()
            {
                return Err("duplicate DWARF attribute".into());
            }
        }
    }
    if dies.is_empty() {
        return Err("no typed DWARF entries; symbol labels are insufficient".into());
    }
    Ok(dies)
}
fn scope(dies: &[Die], index: usize) -> Result<String> {
    let mut names = Vec::new();
    let mut parent = Some(index);
    let mut visited = BTreeSet::new();
    while let Some(i) = parent {
        if i >= dies.len() || !visited.insert(i) {
            return Err("DWARF parent cycle/index differs".into());
        }
        let die = &dies[i];
        if matches!(
            die.tag.as_str(),
            "DW_TAG_namespace" | "DW_TAG_structure_type" | "DW_TAG_class_type"
        ) {
            names.push(die.name()?);
        }
        parent = die.parent;
    }
    names.reverse();
    Ok(names.join("::"))
}
fn declaration(dies: &[Die], mut index: usize) -> Result<usize> {
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(index) || seen.len() > 32 {
            return Err("DWARF declaration cycle/bound differs".into());
        }
        let next = dies[index]
            .attributes
            .get("DW_AT_specification")
            .or_else(|| dies[index].attributes.get("DW_AT_abstract_origin"));
        let Some(value) = next else { return Ok(index) };
        let start = value
            .find("0x")
            .ok_or("DWARF declaration reference absent")?;
        let end = value[start..]
            .find(|c: char| !c.is_ascii_hexdigit() && c != 'x')
            .map_or(value.len(), |n| start + n);
        let offset = hex(&value[start..end])?;
        index = dies
            .binary_search_by_key(&offset, |die| die.offset)
            .map_err(|_| "DWARF declaration reference leaves this exact object")?;
    }
}
fn attribute(dies: &[Die], index: usize, key: &str) -> Result<Option<String>> {
    if let Some(value) = dies[index].attributes.get(key) {
        return Ok(Some(value.clone()));
    }
    let root = declaration(dies, index)?;
    Ok(dies[root].attributes.get(key).cloned())
}
// Resolve actual object-local DIE references. The optional rendered type name
// is diagnostic only; it is never a substitute for the referenced subtree.
fn type_reference(dies: &[Die], value: &str) -> Result<usize> {
    let body = value.trim().strip_prefix('(').ok_or("type reference syntax differs")?;
    let end = body.find(|c: char| !c.is_ascii_hexdigit() && c != 'x').unwrap_or(body.len());
    if !body.starts_with("0x") || end <= 2 || !value.trim().ends_with(')') {
        return Err("object-local type reference absent".into());
    }
    let offset = hex(&body[..end])?;
    dies.binary_search_by_key(&offset, |die| die.offset)
        .map_err(|_| "type reference leaves this exact compiler object".into())
}
fn referred_type(dies: &[Die], index: usize) -> Result<usize> {
    type_reference(dies, &attribute(dies, index, "DW_AT_type")?.ok_or("type DIE reference absent")?)
}
fn concrete_type(dies: &[Die], mut index: usize) -> Result<usize> {
    let mut seen = BTreeSet::new();
    loop {
        if index >= dies.len() || !seen.insert(index) || seen.len() > 32 {
            return Err("type qualifier/reference cycle or bound differs".into());
        }
        if !matches!(dies[index].tag.as_str(), "DW_TAG_typedef" | "DW_TAG_const_type" | "DW_TAG_volatile_type" | "DW_TAG_restrict_type") {
            return Ok(index);
        }
        index = referred_type(dies, index)?;
    }
}
fn typed_child(dies: &[Die], parent: usize, tag: &str, name: &str) -> Result<usize> {
    let found = dies.iter().enumerate().filter(|(_, d)| d.parent == Some(parent) && d.tag == tag && d.name().ok().as_deref() == Some(name))
        .map(|(i, _)| i).collect::<Vec<_>>();
    if found.len() != 1 { return Err(format!("exact typed child {name} absent or ambiguous")); }
    concrete_type(dies, referred_type(dies, found[0])?)
}
fn exact_struct(dies: &[Die], index: usize, expected: &str) -> Result<()> {
    if dies[index].tag != "DW_TAG_structure_type" || scope(dies, index)? != expected
        || dies[index].number("DW_AT_byte_size")? > 1024 * 1024
        || !dies[index].number("DW_AT_alignment")?.is_power_of_two()
        || dies[index].number("DW_AT_alignment")? > 4096 {
        return Err("typed structure identity/layout differs".into());
    }
    Ok(())
}
fn template_type(dies: &[Die], index: usize, name: &str) -> Result<usize> {
    typed_child(dies, index, "DW_TAG_template_type_parameter", name)
}
fn member_type(dies: &[Die], index: usize, name: &str) -> Result<usize> {
    typed_child(dies, index, "DW_TAG_member", name)
}
/// Rust enums are variant-part -> variant -> member -> payload structure.
/// Resolve that chain, not the printed Result/Option generic name.
fn enum_payloads(dies: &[Die], index: usize, expected: &[&str]) -> Result<BTreeMap<String, usize>> {
    let parts = dies.iter().enumerate().filter(|(_, d)| d.parent == Some(index) && d.tag == "DW_TAG_variant_part")
        .map(|(i, _)| i).collect::<Vec<_>>();
    if parts.len() != 1 { return Err("Rust enum variant part absent or ambiguous".into()); }
    let mut result = BTreeMap::new();
    for (variant, _) in dies.iter().enumerate().filter(|(_, d)| d.parent == Some(parts[0]) && d.tag == "DW_TAG_variant") {
        let members = dies.iter().enumerate().filter(|(_, d)| d.parent == Some(variant) && d.tag == "DW_TAG_member")
            .collect::<Vec<_>>();
        if members.len() != 1 { return Err("Rust enum payload membership differs".into()); }
        let (member, die) = members[0];
        let name = die.name()?;
        if !expected.contains(&name.as_str()) || result.insert(name, concrete_type(dies, referred_type(dies, member)?)?).is_some() {
            return Err("Rust enum payload roster differs".into());
        }
    }
    if result.len() != expected.len() { return Err("Rust enum required payload absent".into()); }
    Ok(result)
}
#[derive(Clone, Copy, Serialize, Debug)]
pub enum InputOwnerFactory { Carbon, Hid, Keyword }
impl InputOwnerFactory {
    fn definition(self) -> (&'static str, &'static str) {
        match self {
            Self::Carbon => ("for_carbon", "carbon_hotkeys"),
            Self::Hid => ("for_hid", "hid_service"),
            Self::Keyword => ("for_keyword", "keyword_capture"),
        }
    }
}
#[derive(Serialize, Debug)]
pub struct InputOwnerTypes {
    pub parameter_die: u64,
    pub owner_die: u64,
    pub callback_die: u64,
    pub tuple_die: u64,
    pub result_die: u64,
    pub option_die: u64,
    pub ticket_die: u64,
    pub error_die: u64,
}
#[derive(Serialize, Debug)]
pub struct InputHostIoSlot {
    pub name: &'static str,
    pub offset: u64,
    pub member_die: u64,
    pub pointer_die: u64,
    pub signature_die: u64,
    pub parameter_dies: Vec<u64>,
    pub result_die: u64,
}
#[derive(Serialize, Debug)]
pub struct InputHostIoTypes {
    pub ticket_die: u64,
    pub publisher_arc_die: u64,
    pub publisher_die: u64,
    pub api_reference_die: u64,
    pub api_die: u64,
    pub consumed_phase_die: u64,
    pub consumed_payload_die: u64,
    pub bound_box_die: u64,
    pub bound_host_die: u64,
    pub slots: Vec<InputHostIoSlot>,
}
fn field_die(dies: &[Die], parent: usize, name: &str) -> Result<usize> {
    let fields=dies.iter().enumerate().filter(|(_,d)|d.parent==Some(parent)&&d.tag=="DW_TAG_member"&&d.name().ok().as_deref()==Some(name))
        .map(|(i,_)|i).collect::<Vec<_>>();
    if fields.len()!=1 {return Err(format!("exact member {name} absent or ambiguous"));}
    Ok(fields[0])
}
fn field_at(dies: &[Die], parent: usize, name: &str, offset: u64) -> Result<usize> {
    let field=field_die(dies,parent,name)?;
    if dies[field].number("DW_AT_data_member_location")?!=offset {return Err("typed ABI field offset differs".into());}
    concrete_type(dies,referred_type(dies,field)?)
}
fn integer_type(dies:&[Die],index:usize,name:&str,bytes:u64,signed:bool)->Result<()> {
    let encoding=if signed {"(DW_ATE_signed)"} else {"(DW_ATE_unsigned)"};
    if dies[index].tag!="DW_TAG_base_type" || dies[index].name()?!=name
        || dies[index].number("DW_AT_byte_size")?!=bytes
        || dies[index].attributes.get("DW_AT_encoding").map(String::as_str)!=Some(encoding) {
        return Err("actual ABI integer DIE differs".into());
    }
    Ok(())
}
fn abi_struct(dies:&[Die],index:usize,name:&str,bytes:u64)->Result<()> {
    exact_struct(dies,index,&format!("ruycast_native::profile_io_abi::{name}"))?;
    if dies[index].number("DW_AT_byte_size")?!=bytes || dies[index].number("DW_AT_alignment")?!=8 {
        return Err("actual Host IO ABI layout differs".into());
    }
    Ok(())
}
/// Actual member/type joins only. Runtime table identity and indirect-callsite
/// dataflow are separate checks; this never creates a function/capture permit.
pub fn input_host_io_types(dies:&[Die],ticket:usize)->Result<InputHostIoTypes> {
    if ticket>=dies.len() {return Err("Host IO ticket DIE absent".into());}
    exact_struct(dies,ticket,"ruycast_native::owned_host_entry::NativeInputSourceTicket")?;
    let arc=member_type(dies,ticket,"publisher")?;
    let publisher=template_type(dies,arc,"T")?;
    exact_struct(dies,publisher,"ruycast_native::owned_host_entry::NativeInputPublisher")?;
    exact_struct(dies,arc,"alloc::sync::Arc<ruycast_native::owned_host_entry::NativeInputPublisher, alloc::alloc::Global>")?;
    let global=template_type(dies,arc,"A")?;
    exact_struct(dies,global,"alloc::alloc::Global")?;
    let nonnull=member_type(dies,arc,"ptr")?;
    let inner=template_type(dies,nonnull,"T")?;
    exact_struct(dies,nonnull,"core::ptr::non_null::NonNull<alloc::sync::ArcInner<ruycast_native::owned_host_entry::NativeInputPublisher>>")?;
    exact_struct(dies,inner,"alloc::sync::ArcInner<ruycast_native::owned_host_entry::NativeInputPublisher>")?;
    let pointer=member_type(dies,nonnull,"pointer")?;
    if template_type(dies,inner,"T")?!=publisher || member_type(dies,inner,"data")?!=publisher
        || dies[pointer].tag!="DW_TAG_pointer_type" || concrete_type(dies,referred_type(dies,pointer)?)?!=inner {
        return Err("publisher Arc allocation type differs".into());
    }
    let api_reference=member_type(dies,publisher,"api")?;
    if dies[api_reference].tag!="DW_TAG_pointer_type" {return Err("publisher Api reference differs".into());}
    let api=concrete_type(dies,referred_type(dies,api_reference)?)?;
    abi_struct(dies,api,"Api",56)?;
    integer_type(dies,field_at(dies,api,"version",0)?,"u32",4,false)?;
    integer_type(dies,field_at(dies,api,"bytes",4)?,"u32",4,false)?;
    let reserved=field_at(dies,api,"reserved",8)?;
    if dies[reserved].tag!="DW_TAG_array_type" {return Err("Api reserved field is not an array".into());}
    integer_type(dies,concrete_type(dies,referred_type(dies,reserved)?)?,"u64",8,false)?;
    let ranges=dies.iter().filter(|d|d.parent==Some(reserved)&&d.tag=="DW_TAG_subrange_type").collect::<Vec<_>>();
    if ranges.len()!=1 || ranges[0].number("DW_AT_count")?!=2
        || ranges[0].attributes.contains_key("DW_AT_lower_bound")&&ranges[0].number("DW_AT_lower_bound")?!=0
        || dies.iter().filter(|d|d.parent==Some(api)&&d.tag=="DW_TAG_member").count()!=7 {
        return Err("Api reserved/member layout differs".into());
    }
    let mut slots=Vec::new();
    for (name,offset,parameters,result) in [
        ("bind",24,vec![("Bytes",16)],("Reply",128)),
        ("request",32,vec![("Handle",16),("Request",64)],("Reply",128)),
        ("release_handle",40,vec![("Handle",16)],("i32",4)),
        ("return_buffer",48,vec![("Handle",16),("u64",8)],("i32",4)),
    ] {
        let member=field_die(dies,api,name)?;
        let pointer=field_at(dies,api,name,offset)?;
        if dies[pointer].tag!="DW_TAG_pointer_type" {return Err("Api slot is not a pointer DIE".into());}
        let signature=concrete_type(dies,referred_type(dies,pointer)?)?;
        if dies[signature].tag!="DW_TAG_subroutine_type" {return Err("Api slot has no actual subroutine type".into());}
        let args=dies.iter().enumerate().filter(|(_,d)|d.parent==Some(signature)&&d.tag=="DW_TAG_formal_parameter")
            .map(|(i,_)|concrete_type(dies,referred_type(dies,i)?)).collect::<Result<Vec<_>>>()?;
        if args.len()!=parameters.len() {return Err("Api subroutine parameter count differs".into());}
        for (arg,(name,bytes)) in args.iter().zip(parameters) {
            if name=="u64" {integer_type(dies,*arg,name,bytes,false)?;} else {abi_struct(dies,*arg,name,bytes)?;}
        }
        let returns=concrete_type(dies,referred_type(dies,signature)?)?;
        if result.0=="i32" {integer_type(dies,returns,result.0,result.1,true)?;} else {abi_struct(dies,returns,result.0,result.1)?;}
        slots.push(InputHostIoSlot{name,offset,member_die:dies[member].offset,pointer_die:dies[pointer].offset,signature_die:dies[signature].offset,
            parameter_dies:args.iter().map(|i|dies[*i].offset).collect(),result_die:dies[returns].offset});
    }
    let phases=dies.iter().enumerate().filter(|(_,d)|d.tag=="DW_TAG_structure_type")
        .filter(|(i,_)|scope(dies,*i).ok().as_deref()==Some("ruycast_native::owned_host_entry::Phase"))
        .map(|(i,_)|i).collect::<Vec<_>>();
    if phases.len()!=1 {return Err("unique actual Consumed phase type absent".into());}
    let phase=phases[0];
    let payloads=enum_payloads(dies,phase,&["Empty","Bound","Consumed"])?;
    let consumed=payloads["Consumed"];
    if member_type(dies,consumed,"__0")?!=arc {return Err("Consumed does not retain this exact publisher Arc type".into());}
    let bound_box=member_type(dies,payloads["Bound"],"__0")?;
    if dies[bound_box].tag!="DW_TAG_pointer_type" || dies[bound_box].name()?!= "alloc::boxed::Box<ruycast_native::owned_host_entry::QualifiedOwnedHost, alloc::alloc::Global>" {
        return Err("Bound does not own the qualified Host box".into());
    }
    let bound_host=concrete_type(dies,referred_type(dies,bound_box)?)?;
    exact_struct(dies,bound_host,"ruycast_native::owned_host_entry::QualifiedOwnedHost")?;
    if member_type(dies,bound_host,"input_publisher")?!=arc {return Err("Bound and Consumed publisher types differ".into());}
    Ok(InputHostIoTypes{ticket_die:dies[ticket].offset,publisher_arc_die:dies[arc].offset,publisher_die:dies[publisher].offset,
        api_reference_die:dies[api_reference].offset,api_die:dies[api].offset,consumed_phase_die:dies[phase].offset,
        consumed_payload_die:dies[consumed].offset,bound_box_die:dies[bound_box].offset,bound_host_die:dies[bound_host].offset,slots})
}
/// Typed, same-object owner/result graph for one of the three private factories.
/// This establishes compiler types only; source-reviewed same-allocation return
/// and complete calltarget/capture closure remain separate required conjuncts.
pub fn input_owner_types(dies: &[Die], function: usize, family: InputOwnerFactory) -> Result<InputOwnerTypes> {
    let (name, module) = family.definition();
    if function >= dies.len() || dies[function].tag != "DW_TAG_subprogram"
        || function_name(dies, function)?.as_deref() != Some(name)
        || scope(dies, declaration(dies, function)?)? != "ruycast_native::owned_host_entry::NativeInputSourceTicket" {
        return Err("private Native owner factory identity differs".into());
    }
    let declared = declaration(dies, function)?;
    let parameter_parent = if dies.iter().any(|d| d.parent == Some(function) && d.tag == "DW_TAG_formal_parameter") { function } else { declared };
    let parameters = dies.iter().enumerate().filter(|(_, d)| d.parent == Some(parameter_parent) && d.tag == "DW_TAG_formal_parameter")
        .map(|(i, _)| i).collect::<Vec<_>>();
    if parameters.len() != 1 { return Err("by-value owner factory arity differs".into()); }
    let owner = concrete_type(dies, referred_type(dies, parameters[0])?)?;
    let callback_name = format!("ruycast_native::{module}::Callback");
    let callback = match family {
        InputOwnerFactory::Carbon => {
            // rustc emits sized Box<T,Global> as a named pointer DIE, not a
            // reference and not the unsized Box's {data_ptr,length} structure.
            if dies[owner].tag != "DW_TAG_pointer_type" || dies[owner].name()? != format!("alloc::boxed::Box<{callback_name}, alloc::alloc::Global>") {
                return Err("factory parameter is not the compiler's sized Box owner".into());
            }
            concrete_type(dies, referred_type(dies, owner)?)?
        }
        InputOwnerFactory::Hid | InputOwnerFactory::Keyword => {
            exact_struct(dies, owner, &format!("alloc::rc::Rc<{callback_name}, alloc::alloc::Global>"))?;
            let callback = template_type(dies, owner, "T")?;
            let alloc = template_type(dies, owner, "A")?;
            exact_struct(dies, alloc, "alloc::alloc::Global")?;
            let nonnull = member_type(dies, owner, "ptr")?;
            exact_struct(dies, nonnull, &format!("core::ptr::non_null::NonNull<alloc::rc::RcInner<{callback_name}>>"))?;
            let inner = template_type(dies, nonnull, "T")?;
            exact_struct(dies, inner, &format!("alloc::rc::RcInner<{callback_name}>"))?;
            if template_type(dies, inner, "T")? != callback || member_type(dies, inner, "value")? != callback {
                return Err("Rc allocation value does not own this callback type".into());
            }
            let pointer = member_type(dies, nonnull, "pointer")?;
            if dies[pointer].tag != "DW_TAG_pointer_type" || concrete_type(dies, referred_type(dies, pointer)?)? != inner {
                return Err("Rc NonNull allocation pointer differs".into());
            }
            callback
        }
    };
    exact_struct(dies, callback, &callback_name)?;
    let tuple = concrete_type(dies, referred_type(dies, function)?)?;
    if dies[tuple].tag != "DW_TAG_structure_type" || member_type(dies, tuple, "__0")? != owner
        || dies.iter().filter(|d| d.parent == Some(tuple) && d.tag == "DW_TAG_member").count() != 2 {
        return Err("factory result is not the exact owner/result tuple".into());
    }
    let result = member_type(dies, tuple, "__1")?;
    if !scope(dies, result)?.starts_with("core::result::Result<") { return Err("factory result enum namespace differs".into()); }
    let variants = enum_payloads(dies, result, &["Ok", "Err"])?;
    let option = member_type(dies, variants["Ok"], "__0")?;
    let error = member_type(dies, variants["Err"], "__0")?;
    for payload in variants.values() {
        if template_type(dies, *payload, "T")? != option || template_type(dies, *payload, "E")? != error
            || dies.iter().filter(|d| d.parent == Some(*payload) && d.tag == "DW_TAG_member").count() != 1 {
            return Err("Result payload/template ownership graph differs".into());
        }
    }
    exact_struct(dies, error, "alloc::string::String")?;
    if !scope(dies, option)?.starts_with("core::option::Option<") { return Err("factory option namespace differs".into()); }
    let variants = enum_payloads(dies, option, &["None", "Some"])?;
    if dies.iter().any(|d| d.parent == Some(variants["None"]) && d.tag == "DW_TAG_member") { return Err("None payload unexpectedly owns data".into()); }
    let ticket = member_type(dies, variants["Some"], "__0")?;
    if template_type(dies, variants["Some"], "T")? != ticket
        || dies.iter().filter(|d| d.parent == Some(variants["Some"]) && d.tag == "DW_TAG_member").count() != 1 {
        return Err("Option payload/template ownership graph differs".into());
    }
    exact_struct(dies, ticket, "ruycast_native::owned_host_entry::NativeInputSourceTicket")?;
    Ok(InputOwnerTypes {parameter_die:dies[parameters[0]].offset,owner_die:dies[owner].offset,callback_die:dies[callback].offset,
        tuple_die:dies[tuple].offset,result_die:dies[result].offset,option_die:dies[option].offset,ticket_die:dies[ticket].offset,error_die:dies[error].offset})
}
fn function_name(dies: &[Die], index: usize) -> Result<Option<String>> {
    attribute(dies, index, "DW_AT_name")?
        .map(|value| quoted(&value))
        .transpose()
}
#[derive(Serialize, Debug, PartialEq)]
pub struct Layout {
    pub rust_type: String,
    pub dwarf_offset: u64,
    pub size: u64,
    pub align: u64,
}
pub fn layout(dies: &[Die], name: &str) -> Result<Layout> {
    let mut candidates = Vec::new();
    for (i, die) in dies.iter().enumerate() {
        if matches!(
            die.tag.as_str(),
            "DW_TAG_structure_type" | "DW_TAG_class_type"
        ) && die.name().ok().as_deref() == name.rsplit("::").next()
            && scope(dies, i)? == name
            && die.attributes.contains_key("DW_AT_byte_size")
            && die.attributes.contains_key("DW_AT_alignment")
        {
            candidates.push(Layout {
                rust_type: name.into(),
                dwarf_offset: die.offset,
                size: die.number("DW_AT_byte_size")?,
                align: die.number("DW_AT_alignment")?,
            });
        }
    }
    if candidates.len() != 1 {
        return Err(format!("exact typed layout {name} absent or ambiguous"));
    }
    let value = candidates.remove(0);
    if !value.align.is_power_of_two() || value.align > 4096 || value.size > 16 * 1024 * 1024 {
        return Err("typed compiler layout bound differs".into());
    }
    Ok(value)
}
#[derive(Debug)]
pub struct MapEntry {
    pub object: String,
    pub symbol: String,
    pub object_address: u64,
    pub binary_address: u64,
    pub size: u64,
}
fn yaml_scalar(value: &str) -> Result<String> {
    let s = value.trim();
    let s = if let Some(x) = s.strip_prefix('\'') {
        x.strip_suffix('\'')
            .ok_or("debug-map scalar quote differs")?
    } else {
        s
    };
    if s.is_empty() || s.contains('\n') || s.contains('\0') || s.len() > 32768 {
        return Err("debug-map scalar bound differs".into());
    }
    Ok(s.into())
}
/// Exact existing dsymutil --dump-debug-map symbol/object associations. Object
/// names remain the compiler's original paths even when immutable evidence is
/// held elsewhere; matching only a basename is never sufficient.
pub fn debug_map(text: &str) -> Result<Vec<MapEntry>> {
    if text.len() > MAX_CAPTURE {
        return Err("debug map exceeds bound".into());
    }
    let mut object = None;
    let mut result = Vec::new();
    let mut keys = BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("- filename:") {
            object = Some(yaml_scalar(value)?);
        } else if let Some(body) = line.strip_prefix("- { sym: ") {
            let (symbol, rest) = body
                .split_once(", objAddr: ")
                .ok_or("debug-map symbol syntax differs")?;
            let (address, rest) = rest
                .split_once(", binAddr: ")
                .ok_or("debug-map binary address absent")?;
            let (binary, size) = rest
                .split_once(", size: ")
                .ok_or("debug-map symbol size absent")?;
            let entry = MapEntry {
                object: object.clone().ok_or("debug-map object absent")?,
                symbol: yaml_scalar(symbol)?,
                object_address: hex(address)?,
                binary_address: hex(binary)?,
                size: hex(size
                    .trim()
                    .strip_suffix('}')
                    .ok_or("debug-map end absent")?
                    .trim())?,
            };
            if !keys.insert((entry.object.clone(), entry.symbol.clone())) {
                return Err("debug-map duplicate object/symbol".into());
            }
            result.push(entry);
        }
    }
    if result.is_empty() {
        return Err("empty debug map".into());
    }
    Ok(result)
}
fn word(image: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        image
            .get(at..at + 4)
            .ok_or("truncated native image")?
            .try_into()
            .map_err(|_| "native image word")?,
    ))
}
fn wide(image: &[u8], at: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        image
            .get(at..at + 8)
            .ok_or("truncated native image")?
            .try_into()
            .map_err(|_| "native image wide")?,
    ))
}
fn rx(image: &[u8], address: u64) -> Result<u64> {
    if word(image, 0)? != 0xfeedfacf || word(image, 12)? != 2 {
        return Err("native compiler image/type differs".into());
    }
    let mut at = 32_usize;
    let mut base = None;
    let mut executable = false;
    for _ in 0..word(image, 16)? {
        let command = word(image, at)?;
        let size = word(image, at + 4)? as usize;
        if size < 8 || at.checked_add(size).is_none_or(|end| end > image.len()) {
            return Err("native load command bound differs".into());
        }
        if command == 0x19 {
            if size < 72 {
                return Err("native segment truncated".into());
            }
            let start = wide(image, at + 24)?;
            let end = start
                .checked_add(wide(image, at + 32)?)
                .ok_or("native segment overflow")?;
            if image.get(at + 8..at + 14) == Some(b"__TEXT") {
                base = Some(start);
            }
            if (start..end).contains(&address) && word(image, at + 60)? & 7 == 5 {
                executable = true;
            }
        }
        at += size;
    }
    if !executable {
        return Err("compiler-linked entry is outside actual native RX segments".into());
    }
    address
        .checked_sub(base.ok_or("native text base absent")?)
        .filter(|v| *v > 0)
        .ok_or("native linked offset differs".into())
}
#[derive(Serialize, Debug)]
pub struct LinkedFunction {
    pub dwarf_name: String,
    pub linkage_name: String,
    pub dwarf_offset: u64,
    pub object_address: u64,
    pub image_address: u64,
    pub image_offset: u64,
    pub object_path: String,
    pub function_size: u64,
    pub parameter_types: Vec<String>,
    pub result_type: Option<String>,
    pub declaration_file: Option<String>,
    pub declaration_line: Option<String>,
}
/// Joins actual typed DIE -> exact original object/debug-map -> actual nm entry
/// -> RX bytes. A Rust symbol hash alone does not identify a private generic T.
pub fn linked_function(
    dies: &[Die],
    pretty_name: &str,
    required_scope: Option<&str>,
    object: &str,
    map: &[MapEntry],
    nm: &str,
    image: &[u8],
) -> Result<LinkedFunction> {
    if nm.len() > MAX_CAPTURE {
        return Err("nm capture exceeds bound".into());
    }
    let candidates = dies
        .iter()
        .enumerate()
        .filter(|(i, die)| {
            die.tag == "DW_TAG_subprogram"
                && function_name(dies, *i).ok().flatten().as_deref() == Some(pretty_name)
                && die.attributes.contains_key("DW_AT_low_pc")
        })
        .filter(|(i, _)| {
            required_scope.is_none_or(|expected| {
                declaration(dies, *i)
                    .and_then(|root| scope(dies, root))
                    .ok()
                    .as_deref()
                    == Some(expected)
            })
        })
        .collect::<Vec<_>>();
    if candidates.len() != 1 {
        return Err(format!(
            "typed compiler entry {pretty_name} absent or ambiguous"
        ));
    }
    linked_function_at(dies,candidates[0].0,object,map,Some(nm),image)
}
/// Extract an actual compiler body selected by its object-local DIE index.
/// A missing pretty name is metadata only: linkage/map/symbol/body checks stay
/// mandatory. This does not infer a C ABI from a Swift thunk's symbol spelling.
pub fn linked_function_at(
    dies:&[Die],index:usize,object:&str,map:&[MapEntry],nm:Option<&str>,image:&[u8],
)->Result<LinkedFunction> {
    let die=dies.get(index).ok_or("typed compiler DIE index leaves object")?;
    if die.tag!="DW_TAG_subprogram" || !die.attributes.contains_key("DW_AT_low_pc") {
        return Err("typed compiler DIE is not an actual function body".into());
    }
    let linkage = quoted(
        &attribute(dies, index, "DW_AT_linkage_name")?.ok_or("typed compiler linkage absent")?,
    )?;
    let symbol = format!("_{linkage}");
    let entries = map
        .iter()
        .filter(|entry| entry.object == object && entry.symbol == symbol)
        .collect::<Vec<_>>();
    if entries.len() != 1 {
        return Err("compiler object/linkage debug-map association absent or ambiguous".into());
    }
    let entry = entries[0];
    if entry.object_address != die.number("DW_AT_low_pc")? || entry.size == 0 {
        return Err("typed DIE/object relocation address differs".into());
    }
    let mut addresses = BTreeSet::new();
    if let Some(nm)=nm {
        if nm.len()>MAX_CAPTURE {return Err("nm capture exceeds bound".into());}
        for line in nm.lines() {
            let parts = line.split_whitespace().collect::<Vec<_>>();
            if parts.len() == 3 && parts[2] == symbol && matches!(parts[1], "t" | "T") {
                addresses.insert(hex(parts[0])?);
            }
        }
    } else {
        addresses=image_text_symbol_addresses(image,&symbol)?;
    }
    if addresses.len() != 1 || !addresses.contains(&entry.binary_address) {
        return Err("actual binary symbol/debug-map address differs".into());
    }
    let mut parameters = Vec::new();
    let declared = declaration(dies, index)?;
    let parameter_owner = if dies
        .iter()
        .any(|child| child.parent == Some(index) && child.tag == "DW_TAG_formal_parameter")
    {
        index
    } else {
        declared
    };
    for (child_index, _child) in dies.iter().enumerate().filter(|(_, child)| {
        child.parent == Some(parameter_owner) && child.tag == "DW_TAG_formal_parameter"
    }) {
        parameters.push(
            attribute(dies, child_index, "DW_AT_type")?.ok_or("typed parameter evidence absent")?,
        );
    }
    Ok(LinkedFunction {
        dwarf_name: function_name(dies,index)?.unwrap_or_else(||linkage.clone()),
        linkage_name: linkage,
        dwarf_offset: die.offset,
        object_address: entry.object_address,
        image_address: entry.binary_address,
        image_offset: rx(image, entry.binary_address)?,
        object_path: object.into(),
        function_size: entry.size,
        parameter_types: parameters,
        result_type: attribute(dies, index, "DW_AT_type")?,
        declaration_file: attribute(dies, index, "DW_AT_decl_file")?
            .map(|v| quoted(&v))
            .transpose()?,
        declaration_line: attribute(dies, index, "DW_AT_decl_line")?,
    })
}

// Read the actual linked image's nlist64 records when no complete nm capture
// exists. Only defined symbols in the exact __TEXT,__text section qualify.
fn image_text_symbol_addresses(image:&[u8],symbol:&str)->Result<BTreeSet<u64>> {
    if word(image,0)?!=0xfeedfacf || word(image,4)?!=0x0100000c || word(image,12)?!=2 {
        return Err("linked symbol image is not arm64 MH_EXECUTE".into());
    }
    let count=word(image,16)? as usize;
    let end=32usize.checked_add(word(image,20)? as usize).ok_or("image command overflow")?;
    if count>4096 || end>image.len() {return Err("image symbol commands exceed bounds".into());}
    let mut at=32usize;
    let mut table=None;
    let mut ordinal=0u32;
    let mut text=None;
    for _ in 0..count {
        let size=word(image,at+4)? as usize;
        if size<8 || at.checked_add(size).is_none_or(|next|next>end) {return Err("image symbol command truncated".into());}
        match word(image,at)? {
            2=>{
                if size<24 || table.is_some() {return Err("image symbol table command differs".into());}
                table=Some((word(image,at+8)? as usize,word(image,at+12)? as usize,
                    word(image,at+16)? as usize,word(image,at+20)? as usize));
            },
            0x19=>{
                if size<72 {return Err("image symbol segment truncated".into());}
                let sections=word(image,at+64)? as usize;
                if sections>4096 || 72+sections*80>size {return Err("image symbol section bounds differ".into());}
                for i in 0..sections {
                    ordinal=ordinal.checked_add(1).ok_or("image section ordinal overflow")?;
                    let section=at+72+i*80;
                    if image.get(section..section+16)==Some(&b"__text\0\0\0\0\0\0\0\0\0\0"[..])
                        && image.get(section+16..section+32)==Some(&b"__TEXT\0\0\0\0\0\0\0\0\0\0"[..]) {
                        if word(image,section+64)?&0xff!=0 {return Err("image symbol text is not regular file data".into());}
                        let start=wide(image,section+32)?;
                        let length=wide(image,section+40)?;
                        let end=start.checked_add(length).ok_or("image text address overflow")?;
                        let file=word(image,section+48)? as usize;
                        if usize::try_from(length).ok().and_then(|length|file.checked_add(length)).is_none_or(|end|end>image.len()) {
                            return Err("image symbol text leaves actual bytes".into());
                        }
                        if text.replace((ordinal,start,end)).is_some() {return Err("image text section is ambiguous".into());}
                    }
                }
            },_=>{},
        }
        at+=size;
    }
    if at!=end {return Err("image symbol command size differs".into());}
    let (text,text_start,text_end)=text.ok_or("image text section absent")?;
    let (table,count,strings,length)=table.ok_or("image symbol table absent")?;
    if count>4_000_000 || count.checked_mul(16).and_then(|bytes|table.checked_add(bytes)).is_none_or(|last|last>image.len())
        || strings.checked_add(length).is_none_or(|last|last>image.len()) {return Err("image symbol table bounds differ".into());}
    let mut addresses=BTreeSet::new();
    for i in 0..count {
        let entry=table+i*16;
        if image[entry+4]&0xe0!=0 || image[entry+4]&0x0e!=0x0e || u32::from(image[entry+5])!=text {continue;}
        let offset=word(image,entry)? as usize;
        if offset>=length {return Err("image symbol name leaves string table".into());}
        let name=&image[strings+offset..strings+length];
        let end=name.iter().take(8193).position(|byte|*byte==0).ok_or("image symbol name lacks bounded terminator")?;
        if name.get(..end)==Some(symbol.as_bytes()) {
            let address=wide(image,entry+8)?;
            if !(text_start..text_end).contains(&address) {return Err("image symbol value leaves its declared text section".into());}
            rx(image,address)?;
            addresses.insert(address);
        }
    }
    Ok(addresses)
}
struct TextSection {
    address: u64,
    size: u64,
    file: usize,
    relocations: usize,
    count: usize,
}
fn text_section(bytes: &[u8], kind: u32) -> Result<TextSection> {
    if word(bytes, 0)? != 0xfeedfacf || word(bytes, 12)? != kind {
        return Err("compiler text Mach-O type differs".into());
    }
    let mut at = 32_usize;
    let mut found = None;
    for _ in 0..word(bytes, 16)? {
        let size = word(bytes, at + 4)? as usize;
        if size < 8 || at.checked_add(size).is_none_or(|end| end > bytes.len()) {
            return Err("compiler text command bound differs".into());
        }
        if word(bytes, at)? == 0x19 {
            let count = word(bytes, at + 64)? as usize;
            if size < 72 || count > 4096 || 72 + count * 80 > size {
                return Err("compiler text section roster differs".into());
            }
            for i in 0..count {
                let s = at + 72 + i * 80;
                if bytes.get(s..s + 6) == Some(b"__text")
                    && bytes.get(s + 16..s + 22) == Some(b"__TEXT")
                {
                    if found.is_some() {
                        return Err("duplicate compiler text section".into());
                    }
                    found = Some(TextSection {
                        address: wide(bytes, s + 32)?,
                        size: wide(bytes, s + 40)?,
                        file: word(bytes, s + 48)? as usize,
                        relocations: word(bytes, s + 56)? as usize,
                        count: word(bytes, s + 60)? as usize,
                    });
                }
            }
        }
        at += size;
    }
    found.ok_or("actual compiler text section absent".into())
}
#[derive(Serialize, Debug)]
pub struct RelocatedInstruction {
    pub byte_offset: u64,
    pub relocation_type: u32,
    pub immediate_mask: u32,
    pub symbol_index: u32,
    pub external: bool,
    pub pc_relative: bool,
    pub target_symbol: Option<String>,
    pub linked_target: Option<u64>,
    pub target_matches: Option<bool>,
}
#[derive(Serialize, Debug)]
pub struct TextProof {
    pub bytes: u64,
    pub unchanged_bits_equal: bool,
    pub relocated_instructions: Vec<RelocatedInstruction>,
    /// A target-symbol/addend/indirect-stub check is separately required. Opcode
    /// equality alone must not become transitive relocation/cleanup authority.
    pub relocation_targets_verified: bool,
}
fn object_symbol(object: &[u8], index: u32) -> Result<String> {
    let mut at = 32_usize;
    let mut table = None;
    for _ in 0..word(object, 16)? {
        let size = word(object, at + 4)? as usize;
        if size < 8 || at.checked_add(size).is_none_or(|end| end > object.len()) {
            return Err("object symbol command bound differs".into());
        }
        if word(object, at)? == 2 {
            if table.is_some() {
                return Err("duplicate object symbol table".into());
            }
            table = Some((
                word(object, at + 8)? as usize,
                word(object, at + 12)?,
                word(object, at + 16)? as usize,
                word(object, at + 20)? as usize,
            ));
        }
        at += size;
    }
    let (table, count, strings, length) = table.ok_or("object symbol table absent")?;
    if index >= count {
        return Err("object relocation symbol index differs".into());
    }
    let offset = word(object, table + index as usize * 16)? as usize;
    if offset >= length {
        return Err("object relocation symbol string differs".into());
    }
    let bytes = object
        .get(strings + offset..strings + length)
        .ok_or("object symbol strings truncated")?;
    let end = bytes
        .iter()
        .position(|b| *b == 0)
        .ok_or("object symbol lacks terminator")?;
    if end == 0 || end > 8192 {
        return Err("object symbol name bound differs".into());
    }
    std::str::from_utf8(&bytes[..end])
        .map(str::to_owned)
        .map_err(|_| "object symbol UTF8 differs".into())
}
/// Exact byte equality except the architecturally defined immediate fields of
/// actual ARM64 relocations at these instructions. Unknown encodings, linker
/// opcode relaxation or differences elsewhere fail; no blanket byte skipping.
pub fn text_proof(object: &[u8], native: &[u8], pin: &LinkedFunction) -> Result<TextProof> {
    if word(object, 4)? != 0x0100000c || word(native, 4)? != 0x0100000c {
        return Err("relocation proof currently requires actual ARM64 compiler artifacts".into());
    }
    let source = text_section(object, 1)?;
    let linked = text_section(native, 2)?;
    let length =
        usize::try_from(pin.function_size).map_err(|_| "compiler function size overflow")?;
    if length == 0 || length > 1024 * 1024 || !length.is_multiple_of(4) {
        return Err("compiler function text bound differs".into());
    }
    let offset = pin
        .object_address
        .checked_sub(source.address)
        .ok_or("object function before text")?;
    let destination = pin
        .image_address
        .checked_sub(linked.address)
        .ok_or("native function before text")?;
    if offset
        .checked_add(pin.function_size)
        .is_none_or(|end| end > source.size)
        || destination
            .checked_add(pin.function_size)
            .is_none_or(|end| end > linked.size)
    {
        return Err("function leaves actual text section".into());
    }
    let source_at = source
        .file
        .checked_add(offset as usize)
        .ok_or("object text offset overflow")?;
    let native_at = linked
        .file
        .checked_add(destination as usize)
        .ok_or("native text offset overflow")?;
    let a = object
        .get(source_at..source_at + length)
        .ok_or("object text truncated")?;
    let b = native
        .get(native_at..native_at + length)
        .ok_or("native text truncated")?;
    let mut masks = BTreeMap::<usize, u32>::new();
    let mut relocations = Vec::new();
    if source.count > 1_000_000 {
        return Err("object relocation table exceeds bound".into());
    }
    for index in 0..source.count {
        let at = source
            .relocations
            .checked_add(index * 8)
            .ok_or("relocation offset overflow")?;
        let position = word(object, at)?;
        if position & 0x80000000 != 0 {
            return Err("unsupported scattered relocation".into());
        }
        if (position as u64) < offset || (position as u64) >= offset + pin.function_size {
            continue;
        }
        let bits = word(object, at + 4)?;
        let kind = bits >> 28;
        let width = (bits >> 25) & 3;
        let byte = (position as u64 - offset) as usize;
        if kind == 10 {
            continue;
        } // paired ADDEND has no independent instruction bits
        if width != 2 || !byte.is_multiple_of(4) || byte + 4 > length {
            return Err("unaligned/noninstruction relocation".into());
        }
        let mask = match kind {
            2 => 0x03ff_ffff,
            3 | 5 | 8 => 0x60ff_ffe0,
            4 | 6 | 9 => 0x003f_fc00,
            _ => return Err(format!("unqualified ARM64 text relocation kind {kind}")),
        };
        *masks.entry(byte).or_default() |= mask;
        relocations.push(RelocatedInstruction {
            byte_offset: byte as u64,
            relocation_type: kind,
            immediate_mask: mask,
            symbol_index: bits & 0x00ff_ffff,
            external: bits & (1 << 27) != 0,
            pc_relative: bits & (1 << 24) != 0,
            target_symbol: if bits & (1 << 27) != 0 {
                Some(object_symbol(object, bits & 0x00ff_ffff)?)
            } else {
                None
            },
            linked_target: None,
            target_matches: None,
        });
    }
    for byte in (0..length).step_by(4) {
        let source = u32::from_le_bytes(
            a[byte..byte + 4]
                .try_into()
                .map_err(|_| "source instruction")?,
        );
        let linked = u32::from_le_bytes(
            b[byte..byte + 4]
                .try_into()
                .map_err(|_| "linked instruction")?,
        );
        if (source ^ linked) & !masks.get(&byte).copied().unwrap_or(0) != 0 {
            return Err(format!(
                "actual function text/opcode differs outside relocation at byte {byte}"
            ));
        }
    }
    Ok(TextProof {
        bytes: pin.function_size,
        unchanged_bits_equal: true,
        relocated_instructions: relocations,
        relocation_targets_verified: false,
    })
}
/// Resolve zero-addend direct ARM64 branches against the actual native symbol
/// roster. Data relocations/import stubs remain explicitly unresolved here.
pub fn direct_branch_targets(
    object: &[u8],
    native: &[u8],
    pin: &LinkedFunction,
    nm: &str,
    proof: &mut TextProof,
) -> Result<()> {
    let a = text_section(object, 1)?;
    let b = text_section(native, 2)?;
    let source = a.file + (pin.object_address - a.address) as usize;
    let linked = b.file + (pin.image_address - b.address) as usize;
    let mut addresses = BTreeMap::<&str, BTreeSet<u64>>::new();
    for line in nm.lines() {
        let p = line.split_whitespace().collect::<Vec<_>>();
        if p.len() == 3 && p[1].len() == 1 && p[1] != "U" {
            if let Ok(address) = hex(p[0]) {
                addresses.entry(p[2]).or_default().insert(address);
            }
        }
    }
    for relocation in &mut proof.relocated_instructions {
        if relocation.relocation_type != 2 || !relocation.external || !relocation.pc_relative {
            continue;
        }
        let byte = relocation.byte_offset as usize;
        let before = word(object, source + byte)?;
        let after = word(native, linked + byte)?;
        if before & 0x03ff_ffff != 0 {
            return Err("direct branch addend needs explicit qualification".into());
        }
        if !matches!(after & 0xfc00_0000, 0x9400_0000 | 0x1400_0000) {
            return Err("relocated branch opcode differs".into());
        }
        let immediate = (((after & 0x03ff_ffff) as i32) << 6 >> 6) as i64 * 4;
        let destination =
            (pin.image_address as i128 + relocation.byte_offset as i128 + immediate as i128)
                .try_into()
                .map_err(|_| "branch target overflow")?;
        relocation.linked_target = Some(destination);
        if let Some(name) = &relocation.target_symbol {
            if let Some(values) = addresses.get(name.as_str()) {
                if values.len() != 1 {
                    return Err("native branch target symbol ambiguous".into());
                }
                let matches = values.contains(&destination);
                relocation.target_matches = Some(matches);
                if !matches {
                    return Err("relocated branch target differs from actual symbol".into());
                }
            }
        }
    }
    proof.relocation_targets_verified = proof
        .relocated_instructions
        .iter()
        .all(|r| r.target_matches == Some(true));
    Ok(())
}

/// Recognizes only the exact three-instruction ARM64 const-bool body emitted
/// in these actual typed needs_drop monomorphs. No function is invoked and no
/// arbitrary instruction evaluator or guessed needs-drop default is allowed.
pub fn typed_const_bool(native: &[u8], pin: &LinkedFunction) -> Result<bool> {
    if pin.function_size != 12
        || !pin.parameter_types.is_empty()
        || pin
            .result_type
            .as_ref()
            .is_none_or(|value| quoted(value).ok().as_deref() != Some("bool"))
    {
        return Err("typed const-bool compiler signature/body differs".into());
    }
    let section = text_section(native, 2)?;
    let offset = pin
        .image_address
        .checked_sub(section.address)
        .ok_or("const-bool address precedes text")? as usize;
    let at = section
        .file
        .checked_add(offset)
        .ok_or("const-bool file overflow")?;
    if word(native, at + 4)? != 0x12000100 || word(native, at + 8)? != 0xd65f03c0 {
        return Err("const-bool AND/RET body is not the observed closed compiler pattern".into());
    }
    match word(native, at)? {
        0x52800008 => Ok(false),
        0x52800028 => Ok(true),
        _ => Err("const-bool MOV body is not proven".into()),
    }
}
pub struct ObjectCapture<'a> {
    /// Original absolute object path retained in this binary's debug map.
    pub original_path: &'a str,
    /// Exact full dwarfdump --debug-info output from the hash-bound object.
    pub dwarf: &'a str,
    pub bytes: &'a [u8],
}
// Object-only constant storage. This does not imply that its eventual MAIN
// mapping is RO or that a particular captured instance actually uses it.
fn object_const_slot(object:&[u8],address:u64)->Result<(u64,Option<String>)> {
    if word(object,0)?!=0xfeedfacf || word(object,4)?!=0x0100000c || word(object,12)?!=1 {
        return Err("closure table requires an arm64 compiler object".into());
    }
    let count=word(object,16)? as usize;
    let end=32usize.checked_add(word(object,20)? as usize).ok_or("object command overflow")?;
    if count>4096 || end>object.len() {return Err("object command bounds differ".into());}
    let mut at=32;
    let mut found=None;
    for _ in 0..count {
        let kind=word(object,at)?;
        let size=word(object,at+4)? as usize;
        if size<8 || at.checked_add(size).is_none_or(|next|next>end) {return Err("object command truncated".into());}
        if kind==0x19 {
            if size<72 {return Err("object segment truncated".into());}
            let sections=word(object,at+64)? as usize;
            if sections>4096 || 72+sections*80>size {return Err("object section bounds differ".into());}
            for i in 0..sections {
                let sec=at+72+i*80;
                if object.get(sec..sec+8)!=Some(b"__const\0") || object.get(sec+16..sec+23)!=Some(b"__DATA\0") {continue;}
                let start=wide(object,sec+32)?;
                let length=wide(object,sec+40)?;
                let Some(offset)=address.checked_sub(start) else {continue};
                if offset.checked_add(8).is_none_or(|last|last>length) {continue;}
                if found.is_some() {return Err("object constant slot is ambiguous".into());}
                let flags=word(object,sec+64)?;
                if flags&0xff!=0 || flags&0x8000_0400!=0 {
                    return Err("closure const section is not regular non-instruction file data".into());
                }
                let section_file=u64::from(word(object,sec+48)?);
                let section_end=section_file.checked_add(length).ok_or("object section file range overflow")?;
                let segment_file=wide(object,at+40)?;
                let segment_end=segment_file.checked_add(wide(object,at+48)?).ok_or("object segment file range overflow")?;
                if section_file<segment_file || section_end>segment_end || section_end>object.len() as u64 {
                    return Err("closure const section is not fully file backed".into());
                }
                let file=(word(object,sec+48)? as usize).checked_add(usize::try_from(offset).map_err(|_|"object slot offset")?).ok_or("object slot offset")?;
                if file.checked_add(8).is_none_or(|last|last>object.len()) {return Err("object constant slot leaves actual file".into());}
                let addend=wide(object,file)?;
                let relocations=word(object,sec+56)? as usize;
                let count=word(object,sec+60)? as usize;
                if count>1_000_000 || count.checked_mul(8).and_then(|bytes|relocations.checked_add(bytes)).is_none_or(|last|last>object.len()) {
                    return Err("object DATA relocation bounds differ".into());
                }
                let mut target=None;
                for r in 0..count {
                    let p=relocations+r*8;
                    let position=word(object,p)?;
                    let bits=word(object,p+4)?;
                    let scattered=position&(1<<31)!=0;
                    let relocation_start=if scattered {u64::from(position&0x00ff_ffff)} else {u64::from(position)};
                    let width=1u64<<if scattered {(position>>28)&3} else {(bits>>25)&3};
                    let relocation_end=relocation_start.checked_add(width).ok_or("object DATA relocation range overflow")?;
                    if relocation_start>=offset+8 || relocation_end<=offset {continue;}
                    if scattered || relocation_start!=offset || width!=8 {
                        return Err("overlapping closure DATA relocation is not the exact slot".into());
                    }
                    if target.is_some() {return Err("duplicate closure DATA relocation".into());}
                    if bits>>28!=0 || bits&(1<<24)!=0 || (bits>>25)&3!=3 || bits&(1<<27)==0 || addend!=0 {
                        return Err("closure slot is not a zero-addend external UNSIGNED64 relocation".into());
                    }
                    target=Some(object_symbol(object,bits&0x00ff_ffff)?);
                }
                found=Some((addend,target));
            }
        }
        at+=size;
    }
    if at!=end {return Err("object command size differs".into());}
    found.ok_or_else(||"closure table slot leaves object DATA const storage".into())
}
/// Link one compiler-DWARF closure vtable to its actual object relocation
/// callees. This deliberately does NOT identify the MAIN table address or the
/// runtime instance/callsite/upcast. Those missing joins remain admission gates.
pub fn input_closure_vtable_object(object:&ObjectCapture<'_>,map_text:&str,nm:&str,image:&[u8],variable:usize)->Result<serde_json::Value> {
    let dies=dwarf_dies(object.dwarf)?;
    if variable>=dies.len() || dies[variable].tag!="DW_TAG_variable" {return Err("closure vtable variable absent".into());}
    let table=concrete_type(&dies,referred_type(&dies,variable)?)?;
    if dies[table].tag!="DW_TAG_structure_type" || dies[table].number("DW_AT_byte_size")?!=48 || dies[table].number("DW_AT_alignment")?!=8 {
        return Err("closure vtable compiler layout differs".into());
    }
    let environment=concrete_type(&dies,type_reference(&dies,dies[table].attributes.get("DW_AT_containing_type").ok_or("closure containing environment absent")?)?)?;
    if dies[environment].tag!="DW_TAG_structure_type" {return Err("closure environment is not a structure".into());}
    let environment_size=dies[environment].number("DW_AT_byte_size")?;
    let environment_align=dies[environment].number("DW_AT_alignment")?;
    if environment_size>1024*1024 || !environment_align.is_power_of_two() || environment_align>4096
        || dies.iter().filter(|d|d.parent==Some(table)&&d.tag=="DW_TAG_member").count()!=6 {
        return Err("closure environment/vtable member bounds differ".into());
    }
    let location=dies[variable].attributes.get("DW_AT_location").ok_or("closure vtable object location absent")?;
    let address=hex(location.strip_prefix("(DW_OP_addr ").and_then(|s|s.strip_suffix(')')).ok_or("closure vtable location is not one object address")?)?;
    if !address.is_multiple_of(8) {return Err("closure table object alignment differs".into());}
    for (name,offset,expected) in [("size",8,environment_size),("align",16,environment_align)] {
        integer_type(&dies,field_at(&dies,table,name,offset)?,"usize",8,false)?;
        let (value,relocation)=object_const_slot(object.bytes,address.checked_add(offset).ok_or("closure metadata overflow")?)?;
        if relocation.is_some() || value!=expected {return Err("actual closure table metadata differs from its typed environment".into());}
    }
    let map=debug_map(map_text)?;
    let mut slots=Vec::new();
    let mut methods=BTreeMap::new();
    for (name,offset) in [("drop_in_place",0),("__method3",24),("__method4",32),("__method5",40)] {
        let pointer=field_at(&dies,table,name,offset)?;
        if dies[pointer].tag!="DW_TAG_pointer_type" {return Err("closure vtable slot is not a pointer DIE".into());}
        let (value,symbol)=object_const_slot(object.bytes,address.checked_add(offset).ok_or("closure slot overflow")?)?;
        let Some(symbol)=symbol else {
            if offset!=0 || value!=0 {return Err("closure method has no actual code relocation".into());}
            slots.push(serde_json::json!({"name":name,"offset":offset,"null_object_slot":true}));
            continue;
        };
        let candidates=dies.iter().enumerate().filter(|(_,d)|d.tag=="DW_TAG_subprogram"&&d.attributes.contains_key("DW_AT_low_pc"))
            .filter_map(|(i,_)|match attribute(&dies,i,"DW_AT_linkage_name") {
                Ok(Some(v)) if quoted(&v).ok().is_some_and(|s|format!("_{s}")==symbol)=>Some(Ok(i)),
                Err(e)=>Some(Err(e)),_=>None,
            }).collect::<Result<Vec<_>>>()?;
        if candidates.len()!=1 {return Err("closure relocation target has no unique typed body in this object".into());}
        let function=candidates[0];
        let scope=scope(&dies,declaration(&dies,function)?)?;
        let function_name=function_name(&dies,function)?.ok_or("closure callee name absent")?;
        let linked=linked_function(&dies,&function_name,Some(&scope),object.original_path,&map,nm,image)?;
        let mut text=text_proof(object.bytes,image,&linked)?;
        direct_branch_targets(object.bytes,image,&linked,nm,&mut text)?;
        let self_type=if offset==0 {Some(template_type(&dies,function,"T")?)} else if offset==24 {Some(template_type(&dies,function,"Self")?)} else {None};
        if self_type.is_some_and(|i|i!=environment) {return Err("closure shim/drop Self type differs from containing environment".into());}
        if self_type.is_some() {
            let declared=declaration(&dies,function)?;
            let parent=if dies.iter().any(|d|d.parent==Some(function)&&d.tag=="DW_TAG_formal_parameter") {function} else {declared};
            let parameters=dies.iter().enumerate().filter(|(_,d)|d.parent==Some(parent)&&d.tag=="DW_TAG_formal_parameter")
                .map(|(i,_)|concrete_type(&dies,referred_type(&dies,i)?)).collect::<Result<Vec<_>>>()?;
            if parameters.len()!=if offset==0 {1} else {2} {return Err("closure shim/drop actual parameter count differs".into());}
            if dies[parameters[0]].tag!="DW_TAG_pointer_type" || concrete_type(&dies,referred_type(&dies,parameters[0])?)?!=environment {
                return Err("closure shim/drop does not accept its actual environment pointer".into());
            }
            if offset==24 && template_type(&dies,function,"Args")?!=parameters[1] {return Err("closure shim argument tuple differs".into());}
        }
        if offset!=0 {methods.insert(offset,(function,linked.image_address,publisher_calls(image,&linked)?));}
        slots.push(serde_json::json!({"name":name,"offset":offset,"symbol":symbol,"self_die":self_type.map(|i|dies[i].offset),"linked":linked,"text":text}));
    }
    let once=methods.get(&24).ok_or("closure FnOnce shim absent")?;
    let mutable=methods.get(&32).ok_or("closure FnMut target absent")?;
    let shared=methods.get(&40).ok_or("closure Fn target absent")?;
    if mutable.1!=shared.1 || !once.2.contains(&shared.1) {
        return Err("closure shim does not directly join the same FnMut/Fn target".into());
    }
    let result=|function|attribute(&dies,function,"DW_AT_type")?.map(|value|concrete_type(&dies,type_reference(&dies,&value)?)).transpose();
    if result(once.0)?!=result(shared.0)? {return Err("closure shim and body result DIEs differ".into());}
    let captures=dies.iter().enumerate().filter(|(_,d)|d.parent==Some(environment)&&d.tag=="DW_TAG_member")
        .map(|(i,d)|Ok(serde_json::json!({"member_die":d.offset,"name":d.name()?,"offset":d.number("DW_AT_data_member_location")?,"type_die":dies[concrete_type(&dies,referred_type(&dies,i)?)?].offset})))
        .collect::<Result<Vec<serde_json::Value>>>()?;
    Ok(serde_json::json!({"object_path":object.original_path,"variable_die":dies[variable].offset,"table_die":dies[table].offset,
        "environment_die":dies[environment].offset,"object_address":address,"captures":captures,"slots":slots,"authority":false,
        "missing_for_admission":["linked MAIN RO table and full fixups","instance/upcast/callsite binding","complete transitive source and lifetime/retirement qualification"]}))
}
/// Extract every ARM64 direct outgoing branch, including non-relocated local
/// calls. An indirect BR/BLR (including authenticated forms) is not resolved by
/// symbol spelling; its typed data/capture target must be qualified separately.
fn publisher_calls(image: &[u8], pin: &LinkedFunction) -> Result<Vec<u64>> {
    let section = text_section(image, 2)?;
    let offset = pin.image_address.checked_sub(section.address).ok_or("publisher precedes text")?;
    let end = offset.checked_add(pin.function_size).ok_or("publisher range overflow")?;
    if end > section.size || pin.function_size == 0 || !pin.function_size.is_multiple_of(4) {
        return Err("publisher instruction range differs".into());
    }
    let at=section.file.checked_add(usize::try_from(offset).map_err(|_|"publisher file offset overflow")?)
        .ok_or("publisher file offset overflow")?;
    let end=at.checked_add(usize::try_from(pin.function_size).map_err(|_|"publisher file size overflow")?)
        .ok_or("publisher file range overflow")?;
    let code=image.get(at..end).ok_or("publisher body leaves actual file")?;
    let mut targets = BTreeSet::new();
    let mut pending=vec![0u64];
    let mut visited=BTreeSet::new();
    while let Some(byte)=pending.pop() {
        if !visited.insert(byte) {continue;}
        let instruction = word(code, byte as usize)?;
        // SVC/HVC/SMC/BRK/HLT/DCPS and UDF transfer control outside this
        // proved call graph. They are never ordinary qualified fallthrough.
        if instruction & 0xff00_0000 == 0xd400_0000 || instruction & 0xffff_0000 == 0 {
            return Err("publisher exception/kernel transition is not qualified".into());
        }
        if instruction & 0xfe00_0000 == 0xd600_0000 {
            // RET/RETAA/RETAB return through the caller's link register; all
            // other register branches need their actual capture/vtable proof.
            if !matches!(instruction, 0xd65f_03c0 | 0xd65f_0bff | 0xd65f_0fff) {
                return Err("publisher indirect call/jump target lacks typed capture proof".into());
            }
            continue;
        }
        let direct=instruction & 0x7c00_0000 == 0x1400_0000;
        let call=instruction & 0xfc00_0000 == 0x9400_0000;
        let delta = if instruction & 0x7c00_0000 == 0x1400_0000 {
            Some((((instruction & 0x03ff_ffff) as i32) << 6 >> 6) as i64 * 4)
        } else if instruction & 0xfe00_0000 == 0x5400_0000 || instruction & 0x7e00_0000 == 0x3400_0000 {
            Some(((((instruction >> 5) & 0x7ffff) as i32) << 13 >> 13) as i64 * 4)
        } else if instruction & 0x7e00_0000 == 0x3600_0000 {
            Some(((((instruction >> 5) & 0x3fff) as i32) << 18 >> 18) as i64 * 4)
        } else { None };
        if let Some(delta)=delta {
            let address = pin.image_address.checked_add(byte).ok_or("publisher PC overflow")?;
            let target = address.checked_add_signed(delta).ok_or("publisher branch overflow")?;
            let body_end = pin.image_address.checked_add(pin.function_size).ok_or("publisher end overflow")?;
            if target < pin.image_address || target >= body_end { targets.insert(target); }
            else {pending.push(target-pin.image_address);}
        }
        if !direct || call {
            let next=byte.checked_add(4).ok_or("publisher fallthrough overflow")?;
            if next>=pin.function_size {return Err("publisher reachable code falls outside typed function".into());}
            pending.push(next);
        }
    }
    Ok(targets.into_iter().collect())
}
/// Fixed three-factory compiler data, never an SDK witness. The closure walks
/// actual outgoing instructions and requires a unique same-capture typed body
/// for each target. Missing/inlined bodies, data/import relocations or indirect
/// captures are refusal, not an incomplete-but-qualified route.
pub fn input_publisher_linked_code(objects: &[ObjectCapture<'_>], map_text: &str, nm: &str, image: &[u8]) -> Result<serde_json::Value> {
    if objects.is_empty() || objects.len() > 32 { return Err("publisher object bound differs".into()); }
    let uuid = input_image_uuid(image)?;
    let map = debug_map(map_text)?;
    let dies = objects.iter().map(|object| dwarf_dies(object.dwarf)).collect::<Result<Vec<_>>>()?;
    let mut closure_vtables=Vec::new();
    let mut pending = Vec::new();
    for (object,(capture,entries)) in objects.iter().zip(&dies).enumerate() {
        for (index,die) in entries.iter().enumerate() {
            if die.tag!="DW_TAG_variable" || !die.attributes.contains_key("DW_AT_type") {continue;}
            let table=concrete_type(entries,referred_type(entries,index)?)?;
            let Some(containing)=entries[table].attributes.get("DW_AT_containing_type") else {continue};
            let environment=concrete_type(entries,type_reference(entries,containing)?)?;
            if scope(entries,environment)?.starts_with("ruycast_native::native_keyboard::") {
                let record=input_closure_vtable_object(capture,map_text,nm,image,index)?;
                for slot in record["slots"].as_array().ok_or("closure object slots absent")? {
                    let Some(linked)=slot.get("linked") else {continue};
                    let offset=linked["dwarf_offset"].as_u64().ok_or("closure slot body DIE absent")?;
                    let function=entries.binary_search_by_key(&offset,|die|die.offset).map_err(|_|"closure slot body leaves its object")?;
                    let name=function_name(entries,function)?.ok_or("closure slot body name absent")?;
                    let namespace=scope(entries,declaration(entries,function)?)?;
                    let actual=linked_function(entries,&name,Some(&namespace),capture.original_path,&map,nm,image)?;
                    if serde_json::to_value(&actual).map_err(|_|"closure slot body serialization failed")?!=*linked {
                        return Err("closure slot body differs from its actual object/DIE/link join".into());
                    }
                    pending.push((object,function));
                }
                closure_vtables.push(record);
            }
        }
    }
    let mut factories = Vec::new();
    for role in [InputOwnerFactory::Carbon, InputOwnerFactory::Hid, InputOwnerFactory::Keyword] {
        let (name, _) = role.definition();
        let mut found = None;
        for (object, (capture, entries)) in objects.iter().zip(&dies).enumerate() {
            for (index, die) in entries.iter().enumerate() {
                if die.tag != "DW_TAG_subprogram" || !die.attributes.contains_key("DW_AT_low_pc")
                    || function_name(entries, index)?.as_deref() != Some(name)
                    || scope(entries, declaration(entries, index)?)? != "ruycast_native::owned_host_entry::NativeInputSourceTicket" { continue; }
                if found.is_some() { return Err("publisher factory occurs in multiple typed bodies".into()); }
                let types = input_owner_types(entries, index, role)?;
                let ticket=entries.binary_search_by_key(&types.ticket_die,|die|die.offset).map_err(|_|"factory ticket type leaves object")?;
                let host_io=input_host_io_types(entries,ticket)?;
                let linked = linked_function(entries, name, Some("ruycast_native::owned_host_entry::NativeInputSourceTicket"), capture.original_path, &map, nm, image)?;
                if !linked.declaration_file.as_deref().is_some_and(|p| p.ends_with("/rust-native/src/owned_host_entry.rs")) {
                    return Err("publisher factory declaration source differs".into());
                }
                pending.push((object, index));
                found = Some(serde_json::json!({"role":role,"types":types,"host_io_types":host_io,"linked":linked}));
            }
        }
        factories.push(found.ok_or("fixed publisher factory typed body absent")?);
    }
    // Registration and destruction are incoming callers of the factories, not
    // necessarily reachable from their outgoing call graph. Require their
    // actual typed bodies as additional fixed roots before walking the union.
    let mut route_roots = Vec::new();
    for (module,owner,name) in [
        ("owned_host_entry",Some("NativeInputSourceTicket"),"publish"),
        ("owned_host_entry",Some("NativeInputSourceTicket"),"publish_from"),
        ("owned_host_entry",Some("NativeInputSourceTicket"),"revoke"),
        ("owned_host_entry",Some("NativeInputPublisher"),"request"),
        ("owned_host_entry",None,"receive"),("owned_host_entry",None,"bind_frame"),("owned_host_entry",None,"take_bound_host"),
        ("carbon_hotkeys",Some("Carbon"),"new"),("carbon_hotkeys",None,"finish_callback_removal"),
        ("carbon_hotkeys",Some("Carbon"),"drop"),
        ("hid_service",Some("Callback"),"publish_source_ticket"),("hid_service",Some("Callback"),"revoke_source_ticket"),
        ("hid_service",Some("Service"),"new"),("hid_service",Some("Service"),"refresh"),
        ("hid_service",Some("Service"),"retire"),("hid_service",Some("Service"),"stop"),
        ("hid_service",Some("Service"),"drop"),
        ("keyword_capture",Some("Callback"),"publish_source_ticket"),("keyword_capture",Some("Callback"),"revoke_source_ticket"),
        ("keyword_capture",Some("Service"),"with_ownership"),("keyword_capture",Some("Service"),"refresh"),
        ("keyword_capture",Some("Service"),"retire"),("keyword_capture",Some("Service"),"stop"),
        ("keyword_capture",Some("Service"),"drop"),
        ("native_keyboard",Some("Service"),"new"),
    ] {
        let namespace=owner.map_or_else(||format!("ruycast_native::{module}"),|owner|format!("ruycast_native::{module}::{owner}"));
        let mut found=None;
        for (object,(capture,entries)) in objects.iter().zip(&dies).enumerate() {
            for (index,die) in entries.iter().enumerate() {
                if die.tag!="DW_TAG_subprogram" || !die.attributes.contains_key("DW_AT_low_pc")
                    || function_name(entries,index)?.as_deref()!=Some(name)
                    || scope(entries,declaration(entries,index)?)?!=namespace {continue;}
                if found.is_some() {return Err("publisher registration/cleanup root is ambiguous".into());}
                let pin=linked_function(entries,name,Some(&namespace),capture.original_path,&map,nm,image)?;
                if !pin.declaration_file.as_deref().is_some_and(|p|p.ends_with(&format!("/rust-native/src/{module}.rs"))) {
                    return Err("publisher registration/cleanup root source differs".into());
                }
                found=Some((object,index,pin));
            }
        }
        let (object,index,linked)=found.ok_or_else(||format!("required typed publisher route {namespace}::{name} absent"))?;
        pending.push((object,index));
        route_roots.push(linked);
    }
    let mut visited = BTreeSet::new();
    let mut bodies = Vec::new();
    while let Some((object, index)) = pending.pop() {
        if !visited.insert((object,index)) { continue; }
        if visited.len() > 4096 { return Err("publisher transitive body bound exceeded".into()); }
        let entries = &dies[object];
        let name = function_name(entries,index)?.ok_or("publisher callee name absent")?;
        let namespace = scope(entries,declaration(entries,index)?)?;
        let linked = linked_function(entries,&name,Some(&namespace),objects[object].original_path,&map,nm,image)?;
        if linked.declaration_file.as_ref().is_none_or(|p| p.is_empty() || p.contains('\0')) || linked.declaration_line.is_none() {
            return Err("publisher callee source declaration absent".into());
        }
        let mut text = text_proof(objects[object].bytes,image,&linked)?;
        direct_branch_targets(objects[object].bytes,image,&linked,nm,&mut text)?;
        if !text.relocation_targets_verified { return Err("publisher relocation/import/capture target is not qualified".into()); }
        let targets = publisher_calls(image,&linked)?;
        for target in &targets {
            let mut candidates = BTreeSet::new();
            for entry in map.iter().filter(|e| e.binary_address == *target) {
                for (o,_) in objects.iter().enumerate().filter(|(_,c)| c.original_path == entry.object) {
                    for (i,die) in dies[o].iter().enumerate() {
                        if die.tag == "DW_TAG_subprogram" && die.attributes.contains_key("DW_AT_low_pc")
                            && die.number("DW_AT_low_pc")? == entry.object_address
                            && attribute(&dies[o],i,"DW_AT_linkage_name")?.is_some_and(|v| quoted(&v).ok().is_some_and(|s| format!("_{s}") == entry.symbol)) {
                            candidates.insert((o,i));
                        }
                    }
                }
            }
            if candidates.len()!=1 { return Err("publisher outgoing target lacks unique object/DIE/source closure".into()); }
            pending.push(*candidates.first().ok_or("publisher target absent")?);
        }
        bodies.push(serde_json::json!({"linked":linked,"text":text,"outgoing_targets":targets}));
    }
    Ok(serde_json::json!({"schema":"ruycast.f1.native-input-publisher-code.v1","uuid":uuid,
        "factories":factories,"route_roots":route_roots,"bodies":bodies,"closure_object_vtables":closure_vtables,"authority":false,
        "missing_for_admission":["same-period complete compiler/source/extern custody","source-reviewed caller-before-register and retire-before-free roots","typed dynamic sink/gate/notifier and verified HostSink/Consumed publisher capture joins","Core private publisher-route verifier"]}))
}
/// The five original C bodies only. This compiler reader cannot construct an
/// SDK admission; same-build compiler/source custody and the actual Native-image
/// typed getter observations must also reach Core's private Input constructor.
pub fn input_five_linked_code(
    objects: &[ObjectCapture<'_>],
    map_text: &str,
    nm: &str,
    image: &[u8],
) -> Result<serde_json::Value> {
    if objects.is_empty() || objects.len() > 32 {
        return Err("Input five object bound differs".into());
    }
    let map = debug_map(map_text)?;
    let uuid = input_image_uuid(image)?;
    let mut capsule_types=0;
    for object in objects {
        let dies=dwarf_dies(object.dwarf)?;
        capsule_types+=input_capsule_types(&dies)?;
    }
    if capsule_types==0 { return Err("Input typed C capsule declaration absent".into()); }
    let mut rows = Vec::with_capacity(5);
    for (module, name, signed_result, signed_argument) in [
        ("carbon_hotkeys", "pressed", true, None),
        ("hid_service", "input", false, Some("i32")),
        ("hid_service", "removed", false, Some("i32")),
        ("hid_service", "filter", false, Some("u32")),
        ("keyword_capture", "filter", false, Some("u32")),
    ] {
        let expected_scope = format!("ruycast_native::{module}");
        let mut found = None;
        for object in objects {
            let dies = dwarf_dies(object.dwarf)?;
            let has_body = input_has_body(&dies, name, &expected_scope)?;
            if !has_body {
                continue;
            }
            if found.is_some() {
                return Err("Input C body occurs in multiple compiler objects".into());
            }
            let linked = linked_function(
                &dies,
                name,
                Some(&expected_scope),
                object.original_path,
                &map,
                nm,
                image,
            )?;
            // Type references retain the compiler's actual rendered type. An
            // absent/unknown rendering is refusal, never an arity-only proof.
            let expected_count = if signed_result { 3 } else { 4 };
            if linked.parameter_types.len() != expected_count {
                return Err("Input C body parameter count differs".into());
            }
            for (index, parameter) in linked.parameter_types.iter().enumerate() {
                let actual = quoted(parameter)?;
                let expected = if index == 1 {
                    signed_argument.unwrap_or("*mut core::ffi::c_void")
                } else {
                    "*mut core::ffi::c_void"
                };
                if actual != expected {
                    return Err("Input C body parameter type differs".into());
                }
            }
            let result = linked.result_type.as_deref().map(quoted).transpose()?;
            let expected_result = if signed_result {
                Some("i32")
            } else if name == "filter" {
                Some("*mut core::ffi::c_void")
            } else {
                None
            };
            if result.as_deref() != expected_result {
                return Err("Input C body result type differs".into());
            }
            let source = linked
                .declaration_file
                .as_deref()
                .ok_or("Input C body source declaration absent")?;
            if !source.ends_with(&format!("/rust-native/src/{module}.rs")) {
                return Err("Input C body declaration leaves its fixed source member".into());
            }
            let text = text_proof(object.bytes, image, &linked)?;
            found = Some(serde_json::json!({
                "source_member": format!("rust-native/src/{module}.rs"),
                "function": name,
                "linked": linked,
                "text": text,
            }));
        }
        rows.push(found.ok_or("one fixed Input C body lacks typed compiler evidence")?);
    }
    Ok(serde_json::json!({
        "family": "native-input-five-c-v1",
        "uuid": uuid,
        "typed_capsule_declarations": capsule_types,
        "bodies": rows,
        "authority": false,
        "missing_for_admission": [
            "same-build four-role compiler, exact source and transitive extern custody",
            "all body relocation targets, addends and transitive callback/import closure",
            "actual Native-image typed getter identity and callback userdata ownership",
            "Core private NativeInputAdmission construction and live guards"
        ]
    }))
}
fn input_type_label(value:&str)->Result<String> {
    let first=value.find('"').ok_or("Input type name absent")?;
    let last=value.rfind('"').filter(|last|*last>first).ok_or("Input type terminator absent")?;
    let value=&value[first+1..last];
    if value.len()>4096 || value.contains('\0') { return Err("Input type name exceeds bound".into()); }
    // Only the compiler's escaped C-ABI quotes are permitted; no generic
    // unescaping or normalization can turn another ABI into extern C.
    let value=value.replace("\\\"","\"");
    if value.contains('\\') { return Err("Input type escape differs".into()); }
    Ok(value)
}
fn input_capsule_types(dies:&[Die])->Result<usize> {
    let pointer="*mut core::ffi::c_void";
    let carbon=format!("unsafe extern \"C\" fn({pointer}, {pointer}, {pointer}) -> i32");
    let hid=format!("unsafe extern \"C\" fn({pointer}, i32, {pointer}, {pointer})");
    let tap=format!("unsafe extern \"C\" fn({pointer}, u32, {pointer}, {pointer}) -> {pointer}");
    let fields=[("magic",0,"u64"),("abi",8,"u32"),("count",12,"u32"),
        ("carbon_pressed",16,carbon.as_str()),("hid_input",24,hid.as_str()),
        ("hid_removed",32,hid.as_str()),("hid_filter",40,tap.as_str()),("keyword_filter",48,tap.as_str())];
    let mut found=0;
    for (index,die) in dies.iter().enumerate() {
        if !matches!(die.tag.as_str(),"DW_TAG_structure_type"|"DW_TAG_class_type")
            || !die.attributes.contains_key("DW_AT_name") || die.name()? != "NativeInputCallbacksV1" {
            continue;
        }
        if scope(dies,index)?!="ruycast_native::owned_host_entry::NativeInputCallbacksV1" {
            continue;
        }
        if die.number("DW_AT_byte_size")?!=56 || die.number("DW_AT_alignment")?!=8 {
            return Err("Input typed capsule size/alignment differs".into());
        }
        let members=dies.iter().filter(|child|child.parent==Some(index)&&child.tag=="DW_TAG_member").collect::<Vec<_>>();
        if members.len()!=fields.len() { return Err("Input typed capsule fields differ".into()); }
        for (member,(name,offset,expected)) in members.into_iter().zip(fields) {
            if member.name()!=Ok(name.into()) || member.number("DW_AT_data_member_location")?!=offset
                || input_type_label(member.attributes.get("DW_AT_type").ok_or("Input capsule field type absent")?)?!=expected {
                return Err("Input typed capsule field offset/C ABI differs".into());
            }
        }
        found+=1;
    }
    Ok(found)
}
fn input_has_body(dies: &[Die], name: &str, expected_scope: &str) -> Result<bool> {
    let mut found = false;
    for (index, die) in dies.iter().enumerate() {
        if die.tag != "DW_TAG_subprogram" || !die.attributes.contains_key("DW_AT_low_pc") {
            continue;
        }
        // An unresolved name cannot establish that this body is unrelated.
        // Once a different actual name is known, its scope is irrelevant here.
        if function_name(dies, index)?.as_deref() != Some(name) {
            continue;
        }
        let actual_scope = scope(dies, declaration(dies, index)?)?;
        if actual_scope == expected_scope {
            found = true;
        }
    }
    Ok(found)
}
fn input_image_uuid(image: &[u8]) -> Result<String> {
    if image.len() < 32 || word(image, 0)? != 0xfeedfacf || word(image, 4)? != 0x0100000c {
        return Err("Input image is not an arm64 Mach-O".into());
    }
    let count = word(image, 16)? as usize;
    let commands_end = 32_usize
        .checked_add(word(image, 20)? as usize)
        .filter(|end| *end <= image.len())
        .ok_or("Input load commands leave image")?;
    if count > 4096 {
        return Err("Input load command bound differs".into());
    }
    let mut at = 32_usize;
    let mut uuid = None;
    for _ in 0..count {
        if at.checked_add(8).is_none_or(|end| end > commands_end) {
            return Err("Input load command header leaves image".into());
        }
        let size = word(image, at + 4)? as usize;
        let end = at
            .checked_add(size)
            .filter(|end| *end <= commands_end)
            .ok_or("Input load command leaves image")?;
        if size < 8 {
            return Err("Input load command is short".into());
        }
        if word(image, at)? == 0x1b {
            if size != 24 || uuid.is_some() {
                return Err("Input image UUID absent or ambiguous".into());
            }
            let bytes = &image[at + 8..end];
            if bytes.iter().all(|byte| *byte == 0) {
                return Err("Input image UUID is zero".into());
            }
            uuid = Some(
                bytes
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>(),
            );
        }
        at = end;
    }
    if at != commands_end {
        return Err("Input load command count/size differs".into());
    }
    uuid.ok_or_else(|| "Input image UUID absent".into())
}

/// Emits useful linked-code evidence, never a completed Native callback grant.
/// Missing typed compiler/source/cleanup closure cannot be filled with defaults.
pub fn first_target_proof(
    objects: &[ObjectCapture<'_>],
    map_text: &str,
    nm: &str,
    image: &[u8],
) -> Result<serde_json::Value> {
    if objects.is_empty() || objects.len() > 32 {
        return Err("first Target object bound differs".into());
    }
    let map = debug_map(map_text)?;
    let mut layouts = Vec::new();
    let mut found: BTreeMap<&str, LinkedFunction> = BTreeMap::new();
    for object in objects {
        let dies = dwarf_dies(object.dwarf)?;
        if let (Ok(target), Ok(ivars)) = (layout(&dies, TARGET), layout(&dies, IVARS)) {
            layouts.push((target, ivars));
        }
        for (key, name, scope) in [
            ("invoke", "invoke".into(), Some(TARGET)),
            (
                "registration",
                "class".into(),
                Some("ruycast_native::native_menu_bar::_::{impl#0}"),
            ),
            ("dealloc", format!("dealloc<{TARGET}>"), None),
            ("setup_dealloc", format!("setup_dealloc<{TARGET}>"), None),
            ("ivars_drop", format!("drop_in_place<{IVARS}>"), None),
            ("object_needs_drop", format!("needs_drop<{TARGET}>"), None),
            ("ivars_needs_drop", format!("needs_drop<{IVARS}>"), None),
        ] {
            let has = dies.iter().enumerate().any(|(index, die)| {
                die.tag == "DW_TAG_subprogram"
                    && function_name(&dies, index).ok().flatten().as_deref() == Some(&name)
                    && die.attributes.contains_key("DW_AT_low_pc")
                    && scope.is_none_or(|expected| {
                        declaration(&dies, index)
                            .and_then(|root| self::scope(&dies, root))
                            .ok()
                            .as_deref()
                            == Some(expected)
                    })
            });
            if !has {
                continue;
            }
            let pin = linked_function(&dies, &name, scope, object.original_path, &map, nm, image)?;
            if found.insert(key, pin).is_some() {
                return Err("duplicate typed first Target function witness".into());
            }
        }
    }
    if layouts.is_empty() {
        return Err("first Target/Binding typed layouts absent".into());
    }
    let (target, ivars) = layouts.remove(0);
    if layouts.iter().any(|(t, i)| {
        t.size != target.size
            || t.align != target.align
            || i.size != ivars.size
            || i.align != ivars.align
    }) {
        return Err("first Target layouts differ across compiler objects".into());
    }
    for key in [
        "invoke",
        "registration",
        "dealloc",
        "setup_dealloc",
        "ivars_drop",
        "object_needs_drop",
        "ivars_needs_drop",
    ] {
        if !found.contains_key(key) {
            return Err(format!("first Target typed linked {key} proof absent"));
        }
    }
    for (key, expected) in [
        (
            "invoke",
            vec![
                format!("{TARGET} *"),
                "objc2::runtime::Sel".into(),
                "objc2_app_kit::generated::__NSMenuItem::NSMenuItem *".into(),
            ],
        ),
        (
            "dealloc",
            vec![
                format!("core::ptr::non_null::NonNull<{TARGET}>"),
                "objc2::runtime::Sel".into(),
            ],
        ),
        (
            "setup_dealloc",
            vec!["objc2::runtime::define::ClassBuilder *".into()],
        ),
        ("ivars_drop", vec![format!("{IVARS} *")]),
        ("registration", vec![]),
    ] {
        let pin = &found[key];
        let actual = pin
            .parameter_types
            .iter()
            .map(|v| quoted(v))
            .collect::<Result<Vec<_>>>()?;
        if actual != expected || pin.declaration_file.is_none() || pin.declaration_line.is_none() {
            return Err(format!(
                "first Target typed {key} signature/source origin differs"
            ));
        }
        let result = pin.result_type.as_deref().map(quoted).transpose()?;
        let expected_result =
            (key == "registration").then_some("objc2::runtime::AnyClass *".to_owned());
        if result != expected_result {
            return Err(format!("first Target typed {key} return differs"));
        }
    }
    if !found["registration"]
        .linkage_name
        .contains("ClassType$u20$for$u20$ruycast_native..native_menu_bar..Target$GT$5class")
    {
        return Err(
            "registration lexical namespace does not name the exact typed Target impl".into(),
        );
    }
    let mut text = BTreeMap::new();
    for (key, pin) in &found {
        let object = objects
            .iter()
            .find(|object| object.original_path == pin.object_path)
            .ok_or("linked object bytes absent")?;
        let mut proof = text_proof(object.bytes, image, pin)?;
        direct_branch_targets(object.bytes, image, pin, nm, &mut proof)?;
        text.insert(*key, proof);
    }
    let object_needs_drop = typed_const_bool(image, &found["object_needs_drop"])?;
    let ivars_needs_drop = typed_const_bool(image, &found["ivars_needs_drop"])?;
    Ok(
        serde_json::json!({"schema":"ruycast.f1.first-target.compiler-proof.v1","authority":false,
        "rust_type":TARGET,"object_rust_layout":target,"ivars_layout":ivars,"object_needs_drop":object_needs_drop,"ivars_needs_drop":ivars_needs_drop,"linked_functions":found,"text_relocations":text,
        "missing_for_callback_declaration":["source/compiler/arguments/locked-objc2 closure binding","typed drop-flag/registration and C-unwind macro/parameter proof","relocation target/addend/indirect-stub binding","complete transitive cleanup and parent authority"],
        "source_routes_qualified":false,"whole_family_admission":false}),
    )
}

// Compiler JSON carries mangled type encodings, not a runtime type permit.
const SWIFT_CG_C: &str = "$sys9UnmanagedVySo10CGEventRefaGSgs13OpaquePointerV_So0B4TypeVADSvSgtXCD";
const SWIFT_EVENT: &str = "$sSo10CGEventRefaD";
fn ast_kind(value: &serde_json::Value, kind: &str) -> bool { value["_kind"] == kind }
fn ast_nodes(value: &serde_json::Value) -> Result<Vec<&serde_json::Value>> {
    fn visit<'a>(value:&'a serde_json::Value,depth:usize,out:&mut Vec<&'a serde_json::Value>)->Result<()> {
        if depth>128 || out.len()>300_000 {return Err("Swift AST traversal bound differs".into());}
        match value {
            serde_json::Value::Object(fields)=>{out.push(value);for child in fields.values(){visit(child,depth+1,out)?;}},
            serde_json::Value::Array(values)=>for child in values {visit(child,depth+1,out)?;},
            _=>{}
        }
        Ok(())
    }
    let mut out=Vec::new();visit(value,0,&mut out)?;Ok(out)
}
fn ast_range(value:&serde_json::Value,source:&[u8])->Result<(usize,usize)> {
    let start=value["range"]["start"].as_u64().and_then(|n|usize::try_from(n).ok()).ok_or("Swift source range start absent")?;
    let end=value["range"]["end"].as_u64().and_then(|n|usize::try_from(n).ok()).ok_or("Swift source range end absent")?;
    if start>end || end>=source.len(){return Err("Swift source range leaves exact primary".into());} Ok((start,end))
}
fn ast_decl<'a>(value:&'a serde_json::Value,usr:&str)->Result<Vec<&'a serde_json::Value>> {
    Ok(ast_nodes(value)?.into_iter().filter(|v|ast_kind(v,"decl_ref")&&v["decl_usr"]==usr).collect())
}
fn ast_unmanaged(value:&serde_json::Value,name:&str,encoding:&str)->Result<()> {
    let usr=match name {"passUnretained"=>"s:s9UnmanagedV14passUnretainedyAByxGxFZ","toOpaque"=>"s:s9UnmanagedV8toOpaqueSvyF",
        "fromOpaque"=>"s:s9UnmanagedV10fromOpaqueyAByxGSVFZ","takeUnretainedValue"=>"s:s9UnmanagedV19takeUnretainedValuexyF",_=>return Err("unknown fixed Unmanaged operation".into())};
    let declarations=ast_decl(value,usr)?;
    if declarations.is_empty(){return Err(format!("typed Swift {name} association absent"));}
    for declaration in declarations {
        let substitutions=declaration["substitutions"]["substitutions"].as_array().ok_or("Swift generic substitutions absent")?;
        if substitutions.len()!=1 || substitutions[0]["generic_param"]!="$sxD" || substitutions[0]["replacement_type"]!=encoding {
            return Err("Swift userdata generic owner differs".into());
        }
    }
    Ok(())
}
fn ast_call<'a>(nodes:&[&'a serde_json::Value],usr:&str)->Result<&'a serde_json::Value> {
    let mut matches=Vec::new();
    for node in nodes {
        if ast_kind(node,"call_expr") && ast_decl(&node["fn"],usr)?.len()==1 {matches.push(*node);}
    }
    if matches.len()!=1{return Err("fixed imported Swift registration absent or ambiguous".into());} Ok(matches[0])
}
fn ast_function<'a>(nodes:&[&'a serde_json::Value],name:&str)->Result<&'a serde_json::Value> {
    let matches=nodes.iter().copied().filter(|v|ast_kind(v,"func_decl")&&v["name"]["base_name"]["name"]==name).collect::<Vec<_>>();
    if matches.len()!=1{return Err("Swift callback delegate function absent or ambiguous".into());} Ok(matches[0])
}
fn ast_same_brace_contains(root:&serde_json::Value,target:&serde_json::Value)->bool {
    fn contains(value:&serde_json::Value,target:&serde_json::Value)->bool {
        if std::ptr::eq(value,target){return true;}
        if ast_kind(value,"brace_stmt")||ast_kind(value,"closure_expr")||ast_kind(value,"func_decl"){return false;}
        match value {serde_json::Value::Object(fields)=>fields.values().any(|v|contains(v,target)),
            serde_json::Value::Array(values)=>values.iter().any(|v|contains(v,target)),_=>false}
    }
    root["elements"].as_array().is_some_and(|a|a.iter().any(|v|contains(v,target)))
}
fn ast_callee(value:&serde_json::Value)->Option<&serde_json::Value> {
    match value["_kind"].as_str()? {
        "declref_expr"=>value.get("decl"),
        "function_conversion_expr"=>ast_callee(&value["sub_expr"]),
        "dot_syntax_call_expr"|"constructor_ref_call_expr"=>ast_callee(&value["fn"]),
        _=>None
    }
}
fn ast_direct_calls<'a>(value:&'a serde_json::Value,usr:&str)->Result<Vec<&'a serde_json::Value>> {
    Ok(ast_nodes(value)?.into_iter().filter(|v|ast_kind(v,"call_expr")&&ast_callee(&v["fn"]).is_some_and(|d|d["decl_usr"]==usr)).collect())
}
fn ast_reference(value:&serde_json::Value,name:&str,encoding:&str)->bool {
    ast_kind(value,"declref_expr") && value["type"]==encoding && value["decl"]["base_name"]==name
        && value["decl"]["decl_usr"]=="" && value["decl"]["type_usr"]==encoding
}
fn ast_parameter_name(value:&serde_json::Value)->Result<&str> {
    value["name"]["base_name"]["name"].as_str().filter(|n|!n.is_empty()).ok_or_else(||"Swift formal name absent".into())
}
fn ast_arguments(value:&serde_json::Value)->Result<&[serde_json::Value]> {
    if !ast_kind(&value["args"],"argument_list"){return Err("Swift call argument list absent".into());}
    match value["args"].get("args") {Some(serde_json::Value::Array(args))=>Ok(args),None=>Ok(&[]),_=>Err("Swift call arguments differ".into())}
}
fn ast_receiver(value:&serde_json::Value)->Result<&serde_json::Value> {
    if !ast_kind(&value["fn"],"dot_syntax_call_expr"){return Err("Swift actual method receiver absent".into());}
    let args=ast_arguments(&value["fn"])?;
    if args.len()!=1{return Err("Swift actual method receiver arity differs".into());} Ok(&args[0]["expr"])
}
fn ast_no_shadow(body:&serde_json::Value,outer_formals:&[&serde_json::Value])->Result<()> {
    fn visit(value:&serde_json::Value,names:&[&str],outer_formals:&[&serde_json::Value])->Result<()> {
        let name=if ast_kind(value,"var_decl")||ast_kind(value,"parameter")||ast_kind(value,"pattern_named") {value["name"]["base_name"]["name"].as_str()} else {None};
        if name.is_some_and(|name|names.contains(&name)){return Err("Swift actual input formal is shadowed".into());}
        // A serialized capture repeats the actual declaration as metadata.
        // Only this exact outer-formal copy is not a new lexical declaration.
        let outer_capture=ast_kind(value,"captured_value")&&outer_formals.iter().any(|formal|value.get("decl")==Some(*formal));
        match value {
            serde_json::Value::Object(fields)=>for (key,child) in fields {
                if outer_capture&&key=="decl"{continue;}
                visit(child,names,outer_formals)?;
            },
            serde_json::Value::Array(values)=>for child in values {visit(child,names,outer_formals)?;},
            _=>{}
        }
        Ok(())
    }
    drop(ast_nodes(body)?);
    let names=outer_formals.iter().map(|formal|ast_parameter_name(formal)).collect::<Result<Vec<_>>>()?;
    visit(body,&names,outer_formals)
}
fn ast_registration_self(userdata:&serde_json::Value,owner:&str)->Result<()> {
    let opaque=if ast_kind(userdata,"inject_into_optional"){&userdata["sub_expr"]}else{userdata};
    if !ast_kind(opaque,"call_expr") || ast_callee(&opaque["fn"]).is_none_or(|v|v["decl_usr"]!="s:s9UnmanagedV8toOpaqueSvyF") || !ast_arguments(opaque)?.is_empty() {
        return Err("Swift userdata is not the actual toOpaque call".into());
    }
    let retained=ast_receiver(opaque)?;
    if !ast_kind(retained,"call_expr") || ast_callee(&retained["fn"]).is_none_or(|v|v["decl_usr"]!="s:s9UnmanagedV14passUnretainedyAByxGxFZ") {
        return Err("Swift userdata does not come from passUnretained".into());
    }
    let args=ast_arguments(retained)?;
    if args.len()!=1 || !ast_reference(&args[0]["expr"],"self",owner){return Err("Swift registration passes a different owner expression".into());}
    Ok(())
}
fn ast_userdata_parameter(closure:&serde_json::Value,index:usize,source:&[u8])->Result<serde_json::Value> {
    let formal=closure["params"]["params"].get(index).ok_or("Swift userdata formal absent")?;
    let name=ast_parameter_name(formal)?;
    if formal["interface_type"]!="$sSvSgD" {return Err("Swift userdata formal optionality differs".into());}
    let guard=&closure["body"]["elements"][0];
    if !ast_kind(guard,"guard_stmt"){return Err("Swift userdata optional binding is not the first guard".into());}
    let matches=guard["conditions"].as_array().ok_or("Swift userdata guard conditions absent")?.iter()
        .filter(|v|ast_kind(&v["pattern"],"pattern_optional_some")&&ast_reference(&v["initializer"],name,"$sSvSgD")).collect::<Vec<_>>();
    if matches.len()!=1{return Err("Swift userdata formal/optional binding is ambiguous".into());}
    let binding=&matches[0]["pattern"]["sub_pattern"];
    if matches[0]["pattern"]["type"]!="$sSvSgD" || !ast_kind(binding,"pattern_binding") || binding["kind"]!="let" || binding["type"]!="$sSvD"
        || !ast_kind(&binding["sub_pattern"],"pattern_named") || binding["sub_pattern"]["type"]!="$sSvD" || binding["sub_pattern"]["name"]["base_name"]["name"]!=name {
        return Err("Swift userdata guard does not unwrap this formal".into());
    }
    let names=ast_nodes(&closure["body"])?;
    if names.iter().filter(|v|ast_kind(v,"pattern_named")&&v["name"]["base_name"]["name"]==name).count()!=1
        || names.iter().any(|v|(ast_kind(v,"var_decl")||ast_kind(v,"parameter"))&&v["name"]["base_name"]["name"]==name) {
        return Err("Swift guarded userdata is shadowed".into());
    }
    let (closure_start,closure_end)=ast_range(closure,source)?;
    let (guard_start,guard_end)=ast_range(guard,source)?;
    if guard_start<closure_start || guard_end>closure_end{return Err("Swift userdata guard leaves its closure".into());}
    let calls=ast_direct_calls(closure,"s:s9UnmanagedV10fromOpaqueyAByxGSVFZ")?;
    if calls.is_empty(){return Err("Swift actual fromOpaque call absent".into());}
    let mut ranges=BTreeSet::new();
    for call in calls {
        let (start,end)=ast_range(call,source)?;
        let args=ast_arguments(call)?;
        if start<=guard_end || end>closure_end || args.len()!=1 || !ast_kind(&args[0]["expr"],"pointer_to_pointer")
            || args[0]["expr"]["type"]!="$sSVD" || !ast_reference(&args[0]["expr"]["sub_expr"],name,"$sSvD") {
            return Err("Swift fromOpaque is not fed by this guarded C userdata".into());
        }
        ranges.insert((start,end));
    }
    Ok(serde_json::json!({"formal_index":index,"formal_name":name,"guard_range":guard["range"],"from_opaque_ranges":ranges}))
}
fn ast_return_values(body:&serde_json::Value)->Vec<&serde_json::Value> {
    fn collect<'a>(value:&'a serde_json::Value,out:&mut Vec<&'a serde_json::Value>) {
        if ["closure_expr","func_decl","accessor_decl","constructor_decl","destructor_decl"].iter().any(|kind|ast_kind(value,kind)){return;}
        if ast_kind(value,"return_stmt"){out.push(value);return;}
        match value {serde_json::Value::Object(fields)=>for child in fields.values(){collect(child,out);},
            serde_json::Value::Array(values)=>for child in values{collect(child,out);},_=>{}}
    }
    let mut out=Vec::new();collect(body,&mut out);out
}
fn ast_cg_result(value:&serde_json::Value,event_name:&str)->Result<()> {
    if value["type"]!="$ss9UnmanagedVySo10CGEventRefaGSgD" {return Err("CG return nominal differs".into());}
    match value["_kind"].as_str() {
        Some("nil_literal_expr")=>Ok(()),
        Some("ternary_expr")=>{ast_cg_result(&value["then_expr"],event_name)?;ast_cg_result(&value["else_expr"],event_name)},
        Some("inject_into_optional")=>{
            let call=&value["sub_expr"];
            if !ast_kind(call,"call_expr") || call["type"]!="$ss9UnmanagedVySo10CGEventRefaGD"
                || ast_callee(&call["fn"]).is_none_or(|v|v["decl_usr"]!="s:s9UnmanagedV14passUnretainedyAByxGxFZ") {return Err("CG actual borrowed-return callee differs".into());}
            ast_unmanaged(&call["fn"],"passUnretained",SWIFT_EVENT)?;
            let args=call["args"]["args"].as_array().ok_or("CG return arguments absent")?;
            if args.len()!=1 || !ast_reference(&args[0]["expr"],event_name,SWIFT_EVENT) {return Err("CG return is not the actual event formal".into());} Ok(())
        }
        _=>Err("unsupported CG callback return expression".into())
    }
}
fn ast_cg_returns(function:&serde_json::Value,source:&[u8])->Result<Vec<serde_json::Value>> {
    let (function_start,function_end)=ast_range(function,source)?;
    let body=&function["body"];
    let params=function["params"]["params"].as_array().ok_or("CG delegate formals absent")?;
    if params.len()!=2 || params.iter().any(|v|!ast_kind(v,"parameter")) || params[0]["interface_type"]!="$sSo11CGEventTypeVD" || params[1]["interface_type"]!=SWIFT_EVENT
        || function["result"]!="$ss9UnmanagedVySo10CGEventRefaGSgD" {return Err("CG delegate actual formal/result nominal differs".into());}
    let event_name=ast_parameter_name(&params[1])?;
    ast_no_shadow(body,&[&params[1]])?;
    let returns=ast_return_values(body);
    if returns.is_empty(){return Err("CG callback delegate returns absent".into());}
    for value in &returns {
        let (start,end)=ast_range(value,source)?;
        if start<function_start || end>function_end{return Err("CG return range leaves its actual delegate".into());}
        ast_cg_result(&value["result"],event_name)?;
    }
    Ok(returns.iter().map(|v|v["range"].clone()).collect())
}
fn ast_cg_delegate_join(closure:&serde_json::Value,function:&serde_json::Value,owner:&str,source:&[u8])->Result<serde_json::Value> {
    let formals=closure["params"]["params"].as_array().ok_or("CG closure formals absent")?;
    let type_name=ast_parameter_name(&formals[1])?;
    let event_name=ast_parameter_name(&formals[2])?;
    ast_no_shadow(&closure["body"],&[&formals[1],&formals[2]])?;
    if function["implicit_self_decl"]["interface_type"]!=owner{return Err("CG delegate owner differs".into());}
    let usr=function["usr"].as_str().ok_or("CG delegate declaration USR absent")?;
    let calls=ast_direct_calls(closure,usr)?;
    if calls.len()!=1{return Err("CG actual delegate call absent or ambiguous".into());}
    let call=calls[0];
    let (start,end)=ast_range(call,source)?;
    let (closure_start,closure_end)=ast_range(closure,source)?;
    ast_range(function,source)?;
    if start<closure_start || end>closure_end{return Err("CG delegate call leaves its closure".into());}
    let args=ast_arguments(call)?;
    if args.len()!=2 || !ast_reference(&args[0]["expr"],type_name,"$sSo11CGEventTypeVD") || !ast_reference(&args[1]["expr"],event_name,SWIFT_EVENT) {
        return Err("CG delegate does not receive the actual closure inputs".into());
    }
    let receiver=ast_receiver(call)?;
    if !ast_kind(receiver,"call_expr") || receiver["type"]!=owner || !ast_arguments(receiver)?.is_empty()
        || ast_callee(&receiver["fn"]).is_none_or(|v|v["decl_usr"]!="s:s9UnmanagedV19takeUnretainedValuexyF") {
        return Err("CG delegate receiver is not the actual unretained owner".into());
    }
    let from_opaque=ast_receiver(receiver)?;
    if !ast_kind(from_opaque,"call_expr") || ast_callee(&from_opaque["fn"]).is_none_or(|v|v["decl_usr"]!="s:s9UnmanagedV10fromOpaqueyAByxGSVFZ") {
        return Err("CG delegate receiver does not come from guarded userdata".into());
    }
    let returns=ast_return_values(&closure["body"]);
    let mut delegate_returns=0;
    for value in returns {
        let (start,end)=ast_range(value,source)?;
        if start<closure_start || end>closure_end{return Err("CG return range leaves its actual closure".into());}
        if std::ptr::eq(&value["result"],call){delegate_returns+=1;}
        else {ast_cg_result(&value["result"],event_name)?;}
    }
    if delegate_returns!=1{return Err("CG delegate result is not the actual closure return".into());}
    let return_ranges=ast_cg_returns(function,source)?;
    Ok(serde_json::json!({"delegate_usr":usr,"delegate_call_range":call["range"],"closure_formal_indices":[1,2],
        "delegate_parameter_encodings":[function["params"]["params"][0]["interface_type"],function["params"]["params"][1]["interface_type"]],"return_ranges":return_ranges}))
}
/// Fixed compiler-AST DATA join, consumed by the existing bounded collector.
/// Exact encodings are observations of the pinned Swift compiler. No ABI,
/// old-Main/body, ARC, live userdata or callback admission is issued here.
pub fn reference_input_ast_types(primary:&str,ast:&serde_json::Value,source:&[u8])->Result<serde_json::Value> {
    let (class,roles)=match primary {
        "Hotkeys"=>("Hotkeys",vec![("Carbon","c:@F@InstallEventHandler",1usize,4usize,
            "$sys5Int32Vs13OpaquePointerVSg_AESvSgtXCD",vec!["$ss13OpaquePointerVSgD","$ss13OpaquePointerVSgD","$sSvSgD"])]),
        "HyperKey"=>("HyperKey",vec![
            ("HyperCg","c:@F@CGEventTapCreate",4,5,SWIFT_CG_C,vec!["$ss13OpaquePointerVD","$sSo11CGEventTypeVD",SWIFT_EVENT,"$sSvSgD"]),
            ("HidInput","c:@F@IOHIDManagerRegisterInputValueCallback",1,2,"$syySvSg_s5Int32VAASo13IOHIDValueRefatXCD",vec!["$sSvSgD","$ss5Int32VD","$sSvSgD","$sSo13IOHIDValueRefaD"]),
            ("HidRemoved","c:@F@IOHIDManagerRegisterDeviceRemovalCallback",1,2,"$syySvSg_s5Int32VAASo14IOHIDDeviceRefatXCD",vec!["$sSvSgD","$ss5Int32VD","$sSvSgD","$sSo14IOHIDDeviceRefaD"])]),
        "SnippetExpander"=>("SnippetExpander",vec![("SnippetCg","c:@F@CGEventTapCreate",4,5,SWIFT_CG_C,vec!["$ss13OpaquePointerVD","$sSo11CGEventTypeVD",SWIFT_EVENT,"$sSvSgD"])]),
        _=>return Err("unknown fixed Swift input primary".into())
    };
    if source.len()>MAX_CAPTURE || std::str::from_utf8(source).is_err() || !ast_kind(ast,"source_file")
        || !ast["filename"].as_str().is_some_and(|p|p.ends_with(&format!("/RuycastApp/{primary}.swift")))
        || ast["compiler_version"]["major"]!="6" || ast["compiler_version"]["minor"]!="2" {
        return Err("Swift input AST primary/compiler scope differs".into());
    }
    let nodes=ast_nodes(ast)?;
    let owner=format!("$s18RuycastF1Reference{}{class}CD",class.len());
    let mut callbacks=Vec::new();
    for (role,usr,callback_index,user_index,encoding,formal_types) in roles {
        let call=ast_call(&nodes,usr)?;
        let (start,end)=ast_range(call,source)?;
        let args=call["args"]["args"].as_array().ok_or("Swift registration arguments absent")?;
        let argument=&args.get(callback_index).ok_or("Swift callback argument absent")?["expr"];
        let userdata=&args.get(user_index).ok_or("Swift userdata argument absent")?["expr"];
        ast_unmanaged(userdata,"passUnretained",&owner)?;ast_unmanaged(userdata,"toOpaque",&owner)?;
        ast_registration_self(userdata,&owner)?;
        let self_refs=ast_nodes(userdata)?.into_iter().filter(|v|ast_kind(v,"decl_ref")&&v["base_name"]=="self").collect::<Vec<_>>();
        if self_refs.len()!=1 || self_refs[0]["type_usr"]!=owner{return Err("Swift registration userdata is not typed self".into());}
        let mut definition=serde_json::Value::Null;
        let conversion=if primary=="SnippetExpander" {
            if !ast_kind(argument,"declref_expr") || argument["decl"]["base_name"]!="callback" || argument["decl"]["decl_usr"]!="" || argument["type"]!=encoding {
                return Err("Snippet callback use is not the actual local binding".into());
            }
            let refresh=ast_function(&nodes,"refresh")?;
            let elements=refresh["body"]["elements"].as_array().ok_or("Snippet refresh lexical body absent")?;
            let variables=elements.iter().filter(|v|ast_kind(v,"var_decl")&&v["name"]["base_name"]["name"]=="callback").collect::<Vec<_>>();
            if variables.len()!=1 || variables[0]["let"]!=true || variables[0]["immutable"]!=true || variables[0]["interface_type"]!=encoding {
                return Err("Snippet callback is not one immutable typed local".into());
            }
            let bindings=elements.iter().filter(|v|ast_kind(v,"pattern_binding_decl")&&v["decl_context"]==variables[0]["decl_context"])
                .filter(|v|v["pattern_entries"][0]["pattern"]["sub_pattern"]["name"]["base_name"]["name"]=="callback").collect::<Vec<_>>();
            if bindings.len()!=1 || bindings[0]["pattern_entries"].as_array().is_none_or(|a|a.len()!=1){return Err("Snippet callback initializer absent or ambiguous".into());}
            let (binding_start,binding_end)=ast_range(bindings[0],source)?;
            let (var_start,var_end)=ast_range(variables[0],source)?;
            let (function_start,function_end)=ast_range(refresh,source)?;
            if !(ast_same_brace_contains(&refresh["body"],call)
                && function_start<=binding_start && binding_start<=var_start && var_end<=binding_end && binding_end<start && end<=function_end
                && source[binding_start..=binding_end].starts_with(b"let callback:")) {
                return Err("Snippet callback lexical definition/use order differs".into());
            }
            definition=serde_json::json!({"binding_range":bindings[0]["range"],"variable_range":variables[0]["range"],"function_range":refresh["range"],"local_decl_usr":""});
            &bindings[0]["pattern_entries"][0]["processed_init"]
        } else if ast_kind(argument,"inject_into_optional") {&argument["sub_expr"]} else {argument};
        if !ast_kind(conversion,"function_conversion_expr") || conversion["type"]!=encoding {return Err("actual Swift C callback conversion differs".into());}
        let closure=&conversion["sub_expr"];
        let (closure_start,closure_end)=ast_range(closure,source)?;
        if !ast_kind(closure,"closure_expr") || closure["type"]!=encoding.replace("XCD","cD")
            || closure["global_actor_isolated"]!="$sScMD" || closure.get("captures").is_some()
            || source[closure_start]!=b'{' || source[closure_end]!=b'}' {return Err("Swift registration closure structure differs".into());}
        let formals=closure["params"]["params"].as_array().ok_or("Swift callback formals absent")?;
        if formals.len()!=formal_types.len() || formals.iter().zip(&formal_types).any(|(v,t)|!ast_kind(v,"parameter")||v["interface_type"]!=*t){return Err("Swift callback formal nominal/nullability differs".into());}
        ast_unmanaged(closure,"fromOpaque",&owner)?;ast_unmanaged(closure,"takeUnretainedValue",&owner)?;
        let user_parameter=if role=="Carbon" {2}else if encoding==SWIFT_CG_C {3}else{0};
        let userdata_join=ast_userdata_parameter(closure,user_parameter,source)?;
        let cg_join=if encoding==SWIFT_CG_C {
            let method=if primary=="HyperKey" {"filter"} else {"handle"};
            let function=ast_function(&nodes,method)?;
            ast_cg_delegate_join(closure,function,&owner,source)?
        }else{serde_json::Value::Null};
        callbacks.push(serde_json::json!({"role":role,"callee_usr":usr,"call_range":call["range"],"closure_range":closure["range"],
            "conversion_encoding":conversion["type"],"formal_encodings":formal_types,"unmanaged_generic_encoding_observations":owner,"definition":definition,
            "userdata_parameter_join":userdata_join,"cg_delegate_input_return_join":cg_join}));
    }
    Ok(serde_json::json!({"schema":"ruycast.f1.reference-input-ast-types.v1","primary":primary,"filename":ast["filename"],"compiler_version":ast["compiler_version"],
        "callbacks":callbacks,"authority":false,"missing_for_admission":["closure/delegate actual input-argument use-def identity","fromOpaque argument use-def chain to actual C userdata parameter","actual object/DIE/registration-argument/thunk/callee equivalence to Main","physical initialized root/child userdata custody","queued/inflight monotone retirement and private issuer"]}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn swift_carbon_fixture()->(serde_json::Value,Vec<u8>) {
        let generic=|usr:&str|serde_json::json!({"_kind":"decl_ref","decl_usr":usr,"substitutions":{"substitutions":[{"generic_param":"$sxD","replacement_type":"$s18RuycastF1Reference7HotkeysCD"}]}});
        let reference=|name:&str,encoding:&str|serde_json::json!({"_kind":"declref_expr","type":encoding,"decl":{"_kind":"decl_ref","base_name":name,"decl_usr":"","type_usr":encoding}});
        let function=|usr:&str|serde_json::json!({"_kind":"declref_expr","decl":generic(usr)});
        let guard=serde_json::json!({"_kind":"guard_stmt","range":{"start":21,"end":25},"conditions":[{"pattern":{"_kind":"pattern_optional_some","type":"$sSvSgD","sub_pattern":{"_kind":"pattern_binding","kind":"let","type":"$sSvD","sub_pattern":{"_kind":"pattern_named","type":"$sSvD","name":{"base_name":{"name":"user"}}}}},"initializer":reference("user","$sSvSgD")}]});
        let from=serde_json::json!({"_kind":"call_expr","range":{"start":30,"end":40},"fn":function("s:s9UnmanagedV10fromOpaqueyAByxGSVFZ"),"args":{"_kind":"argument_list","args":[{"expr":{"_kind":"pointer_to_pointer","type":"$sSVD","sub_expr":reference("user","$sSvD")}}]}});
        let closure=serde_json::json!({"_kind":"closure_expr","type":"$sys5Int32Vs13OpaquePointerVSg_AESvSgtcD","range":{"start":20,"end":60},"global_actor_isolated":"$sScMD",
            "params":{"params":[{"_kind":"parameter","interface_type":"$ss13OpaquePointerVSgD"},{"_kind":"parameter","interface_type":"$ss13OpaquePointerVSgD"},{"_kind":"parameter","name":{"base_name":{"name":"user"}},"interface_type":"$sSvSgD"}]},
            "body":{"_kind":"brace_stmt","elements":[guard,from,generic("s:s9UnmanagedV19takeUnretainedValuexyF")]}});
        let conversion=serde_json::json!({"_kind":"inject_into_optional","sub_expr":{"_kind":"function_conversion_expr","type":"$sys5Int32Vs13OpaquePointerVSg_AESvSgtXCD","sub_expr":closure}});
        let pass=serde_json::json!({"_kind":"call_expr","fn":function("s:s9UnmanagedV14passUnretainedyAByxGxFZ"),"args":{"_kind":"argument_list","args":[{"expr":reference("self","$s18RuycastF1Reference7HotkeysCD")}]}});
        let userdata=serde_json::json!({"_kind":"call_expr","fn":{"_kind":"dot_syntax_call_expr","fn":function("s:s9UnmanagedV8toOpaqueSvyF"),"args":{"_kind":"argument_list","args":[{"expr":pass}]}},"args":{"_kind":"argument_list"}});
        let ast=serde_json::json!({"_kind":"source_file","filename":"/owned/RuycastApp/Hotkeys.swift","compiler_version":{"major":"6","minor":"2"},"items":[{"_kind":"call_expr","range":{"start":0,"end":63},"fn":{"_kind":"declref_expr","decl":{"_kind":"decl_ref","decl_usr":"c:@F@InstallEventHandler"}},"args":{"args":[{}, {"expr":conversion},{},{},{"expr":userdata}]}}]});
        let mut source=vec![b' ';64];source[20]=b'{';source[60]=b'}';
        (ast,source)
    }
    #[test]
    fn swift_ast_imported_conversion_preserves_nominals_and_rejects_ambiguous_or_captured_data() {
        let (ast,source)=swift_carbon_fixture();
        let parsed=reference_input_ast_types("Hotkeys",&ast,&source).unwrap();
        assert_eq!(parsed["authority"],false);assert_eq!(parsed["callbacks"].as_array().unwrap().len(),1);
        let mut wrong=ast.clone();wrong["items"][0]["args"]["args"][1]["expr"]["sub_expr"]["sub_expr"]["params"]["params"][0]["interface_type"]=serde_json::json!("$sSvSgD");
        assert!(reference_input_ast_types("Hotkeys",&wrong,&source).is_err());
        let mut wrong=ast.clone();wrong["items"][0]["args"]["args"][1]["expr"]["sub_expr"]["sub_expr"]["captures"]=serde_json::json!([]);
        assert!(reference_input_ast_types("Hotkeys",&wrong,&source).is_err());
        let mut wrong=ast.clone();wrong["items"].as_array_mut().unwrap().push(ast["items"][0].clone());
        assert!(reference_input_ast_types("Hotkeys",&wrong,&source).is_err());
        let mut wrong=ast.clone();
        wrong["items"][0]["args"]["args"][1]["expr"]["sub_expr"]["sub_expr"]["body"]["elements"][1]["args"]["args"][0]["expr"]["sub_expr"]["decl"]["base_name"]=serde_json::json!("other_pointer");
        assert!(reference_input_ast_types("Hotkeys",&wrong,&source).is_err());
        let mut wrong=ast.clone();
        wrong["items"][0]["args"]["args"][4]["expr"]["fn"]["args"]["args"][0]["expr"]["args"]["args"][0]["expr"]["decl"]["base_name"]=serde_json::json!("other_owner");
        assert!(reference_input_ast_types("Hotkeys",&wrong,&source).is_err());
        let mut wrong=ast.clone();
        wrong["items"][0]["args"]["args"][1]["expr"]["sub_expr"]["sub_expr"]["body"]["elements"].as_array_mut().unwrap().push(serde_json::json!({"_kind":"closure_expr","params":{"params":[{"_kind":"parameter","name":{"base_name":{"name":"user"}},"interface_type":"$sSvD"}]}}));
        assert!(reference_input_ast_types("Hotkeys",&wrong,&source).is_err());
        assert!(reference_input_ast_types("HyperKey",&ast,&source).is_err());
        assert!(reference_input_ast_types("Hotkeys",&ast,&source[..10]).is_err());
    }
    #[test]
    fn swift_ast_lexical_binding_does_not_cross_nested_scope_and_nominal_returns_refuse_unknown_expression() {
        let target=serde_json::json!({"_kind":"call_expr"});
        let direct=serde_json::json!({"_kind":"brace_stmt","elements":[{"_kind":"guard_stmt","condition":target.clone()}]});
        assert!(ast_same_brace_contains(&direct,&direct["elements"][0]["condition"]));
        let nested=serde_json::json!({"_kind":"brace_stmt","elements":[{"_kind":"brace_stmt","elements":[target]}]});
        assert!(!ast_same_brace_contains(&nested,&nested["elements"][0]["elements"][0]));
        let formal=serde_json::json!({"_kind":"parameter","name":{"base_name":{"name":"event"}},"decl_context":"outer","interface_type":"$sSo10CGEventRefaD"});
        let mut capture=serde_json::json!({"_kind":"captured_value","decl":formal.clone()});
        ast_no_shadow(&capture,&[&formal]).unwrap();
        capture["decl"]["decl_context"]=serde_json::json!("nested");
        assert!(ast_no_shadow(&capture,&[&formal]).is_err());
        assert!(ast_no_shadow(&serde_json::json!({"_kind":"closure_expr","params":{"params":[formal.clone()]}}),&[&formal]).is_err());
        let returns=serde_json::json!({"_kind":"brace_stmt","elements":[{"_kind":"func_decl","body":{"_kind":"brace_stmt","elements":[{"_kind":"return_stmt","result":{"_kind":"nil_literal_expr"}}]}},{"_kind":"return_stmt","range":{"start":7,"end":8}}]});
        assert_eq!(ast_return_values(&returns).len(),1);
        assert_eq!(ast_return_values(&returns)[0]["range"]["start"],7);
        let mut result=serde_json::json!({"_kind":"nil_literal_expr","type":"$ss9UnmanagedVySo10CGEventRefaGSgD"});
        ast_cg_result(&result,"event").unwrap();result["type"]=serde_json::json!("$sSvSgD");assert!(ast_cg_result(&result,"event").is_err());
        result=serde_json::json!({"_kind":"declref_expr","type":"$ss9UnmanagedVySo10CGEventRefaGSgD","decl":{"base_name":"replacement"}});
        assert!(ast_cg_result(&result,"event").is_err());
        let mut function=serde_json::json!({"range":{"start":0,"end":20},"result":"$ss9UnmanagedVySo10CGEventRefaGSgD","params":{"params":[{"_kind":"parameter","name":{"base_name":{"name":"type"}},"interface_type":"$sSo11CGEventTypeVD"},{"_kind":"parameter","name":{"base_name":{"name":"event"}},"interface_type":"$sSo10CGEventRefaD"}]},"body":{"_kind":"brace_stmt","elements":[{"_kind":"return_stmt","range":{"start":5,"end":9},"result":{"_kind":"nil_literal_expr","type":"$ss9UnmanagedVySo10CGEventRefaGSgD"}}]}});
        ast_cg_returns(&function,&[0;32]).unwrap();
        function["body"]["elements"][0]["range"]["end"]=serde_json::json!(99);
        assert!(ast_cg_returns(&function,&[0;32]).is_err());
        function["body"]["elements"][0].as_object_mut().unwrap().remove("range");
        assert!(ast_cg_returns(&function,&[0;32]).is_err());
        let result=serde_json::json!({"_kind":"inject_into_optional","type":"$ss9UnmanagedVySo10CGEventRefaGSgD","sub_expr":{"_kind":"call_expr","type":"$ss9UnmanagedVySo10CGEventRefaGD",
            "fn":{"_kind":"dot_syntax_call_expr","fn":{"_kind":"declref_expr","decl":{"_kind":"decl_ref","decl_usr":"different_actual_callee"}},"args":{"_kind":"argument_list","args":[{"expr":{"_kind":"decl_ref","decl_usr":"s:s9UnmanagedV14passUnretainedyAByxGxFZ","substitutions":{"substitutions":[{"generic_param":"$sxD","replacement_type":"$sSo10CGEventRefaD"}]}}}]}},
            "args":{"args":[{"expr":{"_kind":"declref_expr","type":"$sSo10CGEventRefaD","decl":{"base_name":"event","decl_usr":"","type_usr":"$sSo10CGEventRefaD"}}}]}}});
        assert!(ast_cg_result(&result,"event").is_err());
    }
    // A finite graph with the same pointer/template/member/variant grammar as
    // the retained rustc cgu02/cgu06 captures. It is parser input, never a grant.
    fn owner_type_fixture(family: InputOwnerFactory) -> (Vec<Die>, usize) {
        fn add(dies:&mut Vec<Die>,tag:&str,name:&str,parent:Option<usize>)->usize {
            let i=dies.len();
            let mut attributes=BTreeMap::new();
            attributes.insert("DW_AT_name".into(),format!("(\"{name}\")"));
            if tag=="DW_TAG_structure_type" {
                attributes.insert("DW_AT_byte_size".into(),"(8)".into());
                attributes.insert("DW_AT_alignment".into(),"(8)".into());
            }
            dies.push(Die{offset:i as u64+1,tag:tag.into(),parent,attributes});i
        }
        fn refer(dies:&mut [Die],from:usize,to:usize) {
            dies[from].attributes.insert("DW_AT_type".into(),format!("(0x{:x} \"untrusted display label\")",dies[to].offset));
        }
        fn child(dies:&mut Vec<Die>,parent:usize,tag:&str,name:&str,to:usize) {
            let from=add(dies,tag,name,Some(parent));refer(dies,from,to);
        }
        let mut d=Vec::new();
        let (name,module)=family.definition();
        let callback_name=format!("ruycast_native::{module}::Callback");
        let callback=add(&mut d,"DW_TAG_structure_type",&callback_name,None);
        let ticket=add(&mut d,"DW_TAG_structure_type","ruycast_native::owned_host_entry::NativeInputSourceTicket",None);
        let owner=match family {
            InputOwnerFactory::Carbon=>{
                let i=add(&mut d,"DW_TAG_pointer_type",&format!("alloc::boxed::Box<{callback_name}, alloc::alloc::Global>"),None);
                refer(&mut d,i,callback);i
            }
            InputOwnerFactory::Hid|InputOwnerFactory::Keyword=>{
                let global=add(&mut d,"DW_TAG_structure_type","alloc::alloc::Global",None);
                let inner=add(&mut d,"DW_TAG_structure_type",&format!("alloc::rc::RcInner<{callback_name}>"),None);
                child(&mut d,inner,"DW_TAG_template_type_parameter","T",callback);
                child(&mut d,inner,"DW_TAG_member","value",callback);
                let pointer=add(&mut d,"DW_TAG_pointer_type","raw allocation pointer",None);refer(&mut d,pointer,inner);
                let nonnull=add(&mut d,"DW_TAG_structure_type",&format!("core::ptr::non_null::NonNull<alloc::rc::RcInner<{callback_name}>>"),None);
                child(&mut d,nonnull,"DW_TAG_template_type_parameter","T",inner);
                child(&mut d,nonnull,"DW_TAG_member","pointer",pointer);
                let rc=add(&mut d,"DW_TAG_structure_type",&format!("alloc::rc::Rc<{callback_name}, alloc::alloc::Global>"),None);
                child(&mut d,rc,"DW_TAG_template_type_parameter","T",callback);
                child(&mut d,rc,"DW_TAG_template_type_parameter","A",global);
                child(&mut d,rc,"DW_TAG_member","ptr",nonnull);rc
            }
        };
        let error=add(&mut d,"DW_TAG_structure_type","alloc::string::String",None);
        let option=add(&mut d,"DW_TAG_structure_type","core::option::Option<ticket>",None);
        let result=add(&mut d,"DW_TAG_structure_type","core::result::Result<option, string>",None);
        for (root,variants) in [(option,vec![("None",None),("Some",Some(ticket))]),(result,vec![("Ok",Some(option)),("Err",Some(error))])] {
            let part=add(&mut d,"DW_TAG_variant_part","",Some(root));
            for (name,payload_type) in variants {
                let variant=add(&mut d,"DW_TAG_variant",name,Some(part));
                let payload=add(&mut d,"DW_TAG_structure_type",name,Some(root));
                child(&mut d,variant,"DW_TAG_member",name,payload);
                if let Some(t)=payload_type {child(&mut d,payload,"DW_TAG_member","__0",t);}
                if root==result {
                    child(&mut d,payload,"DW_TAG_template_type_parameter","T",option);
                    child(&mut d,payload,"DW_TAG_template_type_parameter","E",error);
                } else if name=="Some" {child(&mut d,payload,"DW_TAG_template_type_parameter","T",ticket);}
            }
        }
        let tuple=add(&mut d,"DW_TAG_structure_type","(owner,result)",None);
        child(&mut d,tuple,"DW_TAG_member","__0",owner);
        child(&mut d,tuple,"DW_TAG_member","__1",result);
        let function=add(&mut d,"DW_TAG_subprogram",name,Some(ticket));refer(&mut d,function,tuple);
        child(&mut d,function,"DW_TAG_formal_parameter","owner",owner);
        (d,function)
    }
    #[test]
    fn native_owner_factory_resolves_real_type_edges_not_rendered_labels() {
        for family in [InputOwnerFactory::Carbon,InputOwnerFactory::Hid,InputOwnerFactory::Keyword] {
            let (dies,function)=owner_type_fixture(family);
            let proof=input_owner_types(&dies,function,family).unwrap();
            assert_ne!(proof.owner_die,proof.callback_die);
            let parameter=proof.parameter_die as usize-1;
            let mut wrong=dies.clone();
            wrong[parameter].attributes.insert("DW_AT_type".into(),"(0xdead \"alloc::boxed::Box<ruycast_native::carbon_hotkeys::Callback, alloc::alloc::Global>\")".into());
            assert!(input_owner_types(&wrong,function,family).is_err());
            let mut wrong=dies.clone();
            wrong[proof.owner_die as usize-1].tag="DW_TAG_reference_type".into();
            assert!(input_owner_types(&wrong,function,family).is_err());
            let mut wrong=dies.clone();
            wrong[proof.callback_die as usize-1].attributes.insert("DW_AT_name".into(),"(\"wrong::Callback\")".into());
            assert!(input_owner_types(&wrong,function,family).is_err());
            let mut wrong=dies.clone();
            let returned=wrong.iter_mut().find(|d|d.parent==Some(proof.tuple_die as usize-1)&&d.name().ok().as_deref()==Some("__0")).unwrap();
            returned.attributes.insert("DW_AT_type".into(),format!("(0x{:x})",proof.callback_die));
            assert!(input_owner_types(&wrong,function,family).is_err());
            let mut wrong=dies.clone();
            wrong[proof.ticket_die as usize-1].attributes.insert("DW_AT_name".into(),"(\"untrusted::Ticket\")".into());
            assert!(input_owner_types(&wrong,function,family).is_err());
        }
    }
    fn host_io_fixture(d:&mut Vec<Die>,ticket:usize)->usize {
        fn add(d:&mut Vec<Die>,tag:&str,name:&str,parent:Option<usize>,bytes:u64)->usize {
            let i=d.len();
            let mut attributes=BTreeMap::from([("DW_AT_name".into(),format!("(\"{name}\")"))]);
            if tag=="DW_TAG_structure_type" || tag=="DW_TAG_base_type" {
                attributes.insert("DW_AT_byte_size".into(),format!("({bytes})"));
                attributes.insert("DW_AT_alignment".into(),"(8)".into());
            }
            d.push(Die{offset:i as u64+1,tag:tag.into(),parent,attributes});i
        }
        fn refer(d:&mut [Die],from:usize,to:usize) {d[from].attributes.insert("DW_AT_type".into(),format!("(0x{:x} \"not a proof\")",d[to].offset));}
        fn child(d:&mut Vec<Die>,parent:usize,tag:&str,name:&str,to:usize,offset:u64) {
            let i=add(d,tag,name,Some(parent),0);refer(d,i,to);
            d[i].attributes.insert("DW_AT_data_member_location".into(),format!("({offset})"));
        }
        let global=add(d,"DW_TAG_structure_type","alloc::alloc::Global",None,0);
        let publisher=add(d,"DW_TAG_structure_type","ruycast_native::owned_host_entry::NativeInputPublisher",None,40);
        let inner=add(d,"DW_TAG_structure_type","alloc::sync::ArcInner<ruycast_native::owned_host_entry::NativeInputPublisher>",None,56);
        child(d,inner,"DW_TAG_template_type_parameter","T",publisher,0);
        child(d,inner,"DW_TAG_member","data",publisher,16);
        let pointer=add(d,"DW_TAG_pointer_type","opaque allocation pointer",None,0);refer(d,pointer,inner);
        let nonnull=add(d,"DW_TAG_structure_type","core::ptr::non_null::NonNull<alloc::sync::ArcInner<ruycast_native::owned_host_entry::NativeInputPublisher>>",None,8);
        child(d,nonnull,"DW_TAG_template_type_parameter","T",inner,0);
        child(d,nonnull,"DW_TAG_member","pointer",pointer,0);
        let arc=add(d,"DW_TAG_structure_type","alloc::sync::Arc<ruycast_native::owned_host_entry::NativeInputPublisher, alloc::alloc::Global>",None,8);
        child(d,arc,"DW_TAG_template_type_parameter","T",publisher,0);
        child(d,arc,"DW_TAG_template_type_parameter","A",global,0);
        child(d,arc,"DW_TAG_member","ptr",nonnull,0);
        child(d,ticket,"DW_TAG_member","publisher",arc,0);
        let api=add(d,"DW_TAG_structure_type","ruycast_native::profile_io_abi::Api",None,56);
        let api_pointer=add(d,"DW_TAG_pointer_type","readonly borrowed Api",None,0);refer(d,api_pointer,api);
        child(d,publisher,"DW_TAG_member","api",api_pointer,0);
        let mut integers=BTreeMap::new();
        for (name,bytes,encoding) in [("u32",4,"(DW_ATE_unsigned)"),("u64",8,"(DW_ATE_unsigned)"),("i32",4,"(DW_ATE_signed)")] {
            let i=add(d,"DW_TAG_base_type",name,None,bytes);d[i].attributes.insert("DW_AT_encoding".into(),encoding.into());integers.insert(name,i);
        }
        child(d,api,"DW_TAG_member","version",integers["u32"],0);
        child(d,api,"DW_TAG_member","bytes",integers["u32"],4);
        let reserved=add(d,"DW_TAG_array_type","untrusted reserved label",None,0);refer(d,reserved,integers["u64"]);
        let range=add(d,"DW_TAG_subrange_type","",Some(reserved),0);d[range].attributes.insert("DW_AT_count".into(),"(2)".into());
        child(d,api,"DW_TAG_member","reserved",reserved,8);
        let mut structs=BTreeMap::new();
        for (name,bytes) in [("Bytes",16),("Handle",16),("Request",64),("Reply",128)] {
            structs.insert(name,add(d,"DW_TAG_structure_type",&format!("ruycast_native::profile_io_abi::{name}"),None,bytes));
        }
        for (name,offset,args,result) in [("bind",24,vec![structs["Bytes"]],structs["Reply"]),
            ("request",32,vec![structs["Handle"],structs["Request"]],structs["Reply"]),
            ("release_handle",40,vec![structs["Handle"]],integers["i32"]),
            ("return_buffer",48,vec![structs["Handle"],integers["u64"]],integers["i32"])] {
            let signature=add(d,"DW_TAG_subroutine_type","",None,0);refer(d,signature,result);
            for arg in args {child(d,signature,"DW_TAG_formal_parameter","",arg,0);}
            let pointer=add(d,"DW_TAG_pointer_type","misleading displayed ABI",None,0);refer(d,pointer,signature);
            child(d,api,"DW_TAG_member",name,pointer,offset);
        }
        let host=add(d,"DW_TAG_structure_type","ruycast_native::owned_host_entry::QualifiedOwnedHost",None,160);
        child(d,host,"DW_TAG_member","input_publisher",arc,152);
        let boxed=add(d,"DW_TAG_pointer_type","alloc::boxed::Box<ruycast_native::owned_host_entry::QualifiedOwnedHost, alloc::alloc::Global>",None,0);refer(d,boxed,host);
        let phase=add(d,"DW_TAG_structure_type","ruycast_native::owned_host_entry::Phase",None,16);
        let part=add(d,"DW_TAG_variant_part","",Some(phase),0);
        for (name,payload_type) in [("Empty",None),("Bound",Some(boxed)),("Consumed",Some(arc))] {
            let variant=add(d,"DW_TAG_variant",name,Some(part),0);
            let payload=add(d,"DW_TAG_structure_type",name,Some(phase),8);
            child(d,variant,"DW_TAG_member",name,payload,0);
            if let Some(value)=payload_type {child(d,payload,"DW_TAG_member","__0",value,0);}
        }
        api
    }
    #[test]
    fn publisher_host_io_joins_actual_arc_consumed_and_subroutine_type_edges() {
        let (mut dies,function)=owner_type_fixture(InputOwnerFactory::Carbon);
        let owner=input_owner_types(&dies,function,InputOwnerFactory::Carbon).unwrap();
        let ticket=owner.ticket_die as usize-1;
        let api=host_io_fixture(&mut dies,ticket);
        let proof=input_host_io_types(&dies,ticket).unwrap();
        assert_eq!(proof.api_die,dies[api].offset);
        assert_eq!(proof.slots.len(),4);
        let mut wrong=dies.clone();
        let request=proof.slots.iter().find(|s|s.name=="request").unwrap();
        wrong[request.member_die as usize-1].attributes.insert("DW_AT_data_member_location".into(),"(24)".into());
        assert!(input_host_io_types(&wrong,ticket).is_err());
        let mut wrong=dies.clone();
        let argument=wrong.iter_mut().find(|d|d.parent==Some(request.signature_die as usize-1)&&d.tag=="DW_TAG_formal_parameter").unwrap();
        argument.attributes.insert("DW_AT_type".into(),format!("(0x{:x} \"correct-looking Handle\")",owner.callback_die));
        assert!(input_host_io_types(&wrong,ticket).is_err());
        let mut wrong=dies.clone();
        let retained=field_die(&wrong,proof.consumed_payload_die as usize-1,"__0").unwrap();
        wrong[retained].attributes.insert("DW_AT_type".into(),format!("(0x{:x})",owner.owner_die));
        assert!(input_host_io_types(&wrong,ticket).is_err());
        let mut wrong=dies.clone();
        wrong[proof.api_die as usize-1].attributes.insert("DW_AT_byte_size".into(),"(104)".into());
        assert!(input_host_io_types(&wrong,ticket).is_err());
        let mut wrong=dies.clone();
        let field=field_die(&wrong,proof.bound_host_die as usize-1,"input_publisher").unwrap();
        wrong[field].attributes.insert("DW_AT_type".into(),format!("(0x{:x})",owner.owner_die));
        assert!(input_host_io_types(&wrong,ticket).is_err());
    }
    fn closure_const_fixture()->Vec<u8> {
        let mut object=vec![0u8;480];
        for (at,value) in [(0,0xfeedfacfu32),(4,0x0100000c),(12,1),(16,2),(20,176),
            (32,0x19),(36,152),(96,1),(152,256),(160,400),(164,1),
            (184,2),(188,24),(192,432),(196,1),(200,448),(204,7),
            (400,24),(404,(3<<25)|(1<<27)),(432,1)] {
            object[at..at+4].copy_from_slice(&value.to_le_bytes());
        }
        object[104..112].copy_from_slice(b"__const\0");
        object[120..127].copy_from_slice(b"__DATA\0");
        object[136..144].copy_from_slice(&0x1000u64.to_le_bytes());
        object[144..152].copy_from_slice(&48u64.to_le_bytes());
        object[264..272].copy_from_slice(&8u64.to_le_bytes());
        object[448..455].copy_from_slice(b"\0_slot\0");
        object[72..80].copy_from_slice(&256u64.to_le_bytes());
        object[80..88].copy_from_slice(&48u64.to_le_bytes());
        object
    }
    #[test]
    fn closure_object_slots_require_exact_data_storage_and_unsigned64_targets() {
        let object=closure_const_fixture();
        assert_eq!(object_const_slot(&object,0x1018).unwrap(),(0,Some("_slot".into())));
        assert_eq!(object_const_slot(&object,0x1008).unwrap(),(8,None));
        let mut wrong=object.clone();wrong[280]=1;
        assert!(object_const_slot(&wrong,0x1018).is_err());
        let mut wrong=object.clone();wrong[407]|=0x20;
        assert!(object_const_slot(&wrong,0x1018).is_err());
        let mut wrong=object.clone();wrong[164..168].copy_from_slice(&2u32.to_le_bytes());
        wrong[408..412].copy_from_slice(&24u32.to_le_bytes());wrong[412..416].copy_from_slice(&((3u32<<25)|(1<<27)).to_le_bytes());
        assert!(object_const_slot(&wrong,0x1018).is_err());
        let mut wrong=object.clone();wrong[152..156].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(object_const_slot(&wrong,0x1018).is_err());
        assert!(object_const_slot(&object,0x2000).is_err());
    }
    #[test]
    fn closure_object_slots_reject_every_overlapping_relocation_range() {
        for offset in [0u32,8,24] {
            let mut object=closure_const_fixture();
            object[400..404].copy_from_slice(&(offset+4).to_le_bytes());
            object[404..408].copy_from_slice(&((2u32<<25)|(1<<27)).to_le_bytes());
            assert!(object_const_slot(&object,0x1000+u64::from(offset)).is_err());
        }
        let mut object=closure_const_fixture();
        object[400..404].copy_from_slice(&4u32.to_le_bytes());
        assert!(object_const_slot(&object,0x1008).is_err());
        object[400..404].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(object_const_slot(&object,0x1008).unwrap(),(8,None));
    }
    #[test]
    fn closure_object_slots_reject_zerofill_and_non_file_backed_sections() {
        for flags in [1u32,0x0c,0x12,0x8000_0000,0x400] {
            let mut object=closure_const_fixture();
            object[168..172].copy_from_slice(&flags.to_le_bytes());
            assert!(object_const_slot(&object,0x1008).is_err());
        }
        let mut object=closure_const_fixture();
        object[152..156].copy_from_slice(&0u32.to_le_bytes());
        assert!(object_const_slot(&object,0x1008).is_err());
        let mut object=closure_const_fixture();
        object[80..88].copy_from_slice(&16u64.to_le_bytes());
        assert!(object_const_slot(&object,0x1008).is_err());
    }
    #[test]
    fn native_owner_factory_rejects_template_payload_and_qualifier_cycles() {
        let (dies,function)=owner_type_fixture(InputOwnerFactory::Hid);
        let proof=input_owner_types(&dies,function,InputOwnerFactory::Hid).unwrap();
        let mut wrong=dies.clone();
        let argument=wrong.iter_mut().find(|d|d.parent==Some(proof.owner_die as usize-1)&&d.name().ok().as_deref()==Some("T")).unwrap();
        argument.attributes.insert("DW_AT_type".into(),format!("(0x{:x})",proof.error_die));
        assert!(input_owner_types(&wrong,function,InputOwnerFactory::Hid).is_err());
        let mut wrong=dies.clone();
        let owner=proof.owner_die as usize-1;
        wrong[owner].tag="DW_TAG_typedef".into();
        wrong[owner].attributes.insert("DW_AT_type".into(),format!("(0x{:x})",proof.owner_die));
        assert!(input_owner_types(&wrong,function,InputOwnerFactory::Hid).is_err());
        let mut qualified=dies.clone();
        let alias=qualified.len();
        qualified.push(Die{offset:alias as u64+1,tag:"DW_TAG_const_type".into(),parent:None,
            attributes:BTreeMap::from([("DW_AT_type".into(),format!("(0x{:x})",proof.owner_die))])});
        qualified[proof.parameter_die as usize-1].attributes.insert("DW_AT_type".into(),format!("(0x{:x})",alias+1));
        input_owner_types(&qualified,function,InputOwnerFactory::Hid).unwrap();
        let mut wrong=dies.clone();
        let some=wrong.iter().position(|d|d.parent==Some(proof.option_die as usize-1)&&d.name().ok().as_deref()==Some("Some")&&d.tag=="DW_TAG_structure_type").unwrap();
        let member=wrong.iter_mut().find(|d|d.parent==Some(some)&&d.tag=="DW_TAG_member").unwrap();
        member.attributes.insert("DW_AT_type".into(),format!("(0x{:x})",proof.error_die));
        assert!(input_owner_types(&wrong,function,InputOwnerFactory::Hid).is_err());
    }
    #[test]
    fn publisher_outgoing_calls_include_nonrelocated_branches_and_refuse_indirects() {
        let mut image=vec![0u8;272];
        for (at,value) in [(0,0xfeedfacfu32),(12,2),(16,1),(20,152),(32,0x19),(36,152),(96,1),(152,16),(160,256)] {
            image[at..at+4].copy_from_slice(&value.to_le_bytes());
        }
        image[104..110].copy_from_slice(b"__text");
        image[120..126].copy_from_slice(b"__TEXT");
        image[136..144].copy_from_slice(&0x100000100u64.to_le_bytes());
        image[144..152].copy_from_slice(&16u64.to_le_bytes());
        image[152..156].copy_from_slice(&256u32.to_le_bytes());
        let mut pin=LinkedFunction{dwarf_name:"fixture".into(),linkage_name:"fixture".into(),dwarf_offset:1,
            object_address:0,image_address:0x100000100,image_offset:256,object_path:"owned.o".into(),function_size:8,
            parameter_types:vec![],result_type:None,declaration_file:None,declaration_line:None};
        // BL +8 and tail B +8 have an actual out-of-body target even when no
        // relocation record exists. This must not disappear from the closure.
        for branch in [0x94000002u32,0x14000002,0x54000040,0x34000040,0x36000040] {
            image[256..260].copy_from_slice(&branch.to_le_bytes());
            image[260..264].copy_from_slice(&0xd65f03c0u32.to_le_bytes());
            assert_eq!(publisher_calls(&image,&pin).unwrap(),vec![0x100000108]);
        }
        for indirect in [0xd63f0000u32,0xd61f0000,0xd73f0801] {
            image[256..260].copy_from_slice(&indirect.to_le_bytes());
            assert!(publisher_calls(&image,&pin).is_err());
        }
        image[256..260].copy_from_slice(&0xd65f03c0u32.to_le_bytes());
        assert!(publisher_calls(&image,&pin).unwrap().is_empty());
        image[260..264].copy_from_slice(&0xd65f03c0u32.to_le_bytes());
        for exception in [0xd4000001u32,0xd4000002,0xd4000003,0xd4200000,0xd4400000,0xd4a00001,0x00000000,0x00000001,0x0000001f,0x0000ffff] {
            image[256..260].copy_from_slice(&exception.to_le_bytes());
            assert!(publisher_calls(&image,&pin).is_err());
        }
        image[256..264].copy_from_slice(&[0x1f,0x20,0x03,0xd5,0x1f,0x20,0x03,0xd5]);
        assert!(publisher_calls(&image,&pin).is_err());
        pin.function_size=20;
        assert!(publisher_calls(&image,&pin).is_err());
        assert!(input_publisher_linked_code(&[],"","",&image).is_err());
    }
    #[test]
    fn text_proof_masks_only_actual_relocation_immediates_not_opcodes_or_missing_targets() {
        let make = |kind: u32, base: u64, instruction: u32, relocations: u32| {
            let mut b = vec![0_u8; 336];
            let w = |b: &mut [u8], at: usize, value: u32| {
                b[at..at + 4].copy_from_slice(&value.to_le_bytes())
            };
            let q = |b: &mut [u8], at: usize, value: u64| {
                b[at..at + 8].copy_from_slice(&value.to_le_bytes())
            };
            w(&mut b, 0, 0xfeedfacf);
            w(&mut b, 4, 0x0100000c);
            w(&mut b, 12, kind);
            w(&mut b, 16, 2);
            w(&mut b, 20, 176);
            w(&mut b, 32, 0x19);
            w(&mut b, 36, 152);
            b[40..46].copy_from_slice(b"__TEXT");
            q(&mut b, 56, base);
            q(&mut b, 64, 0x1000);
            w(&mut b, 92, 5);
            w(&mut b, 96, 1);
            b[104..110].copy_from_slice(b"__text");
            b[120..126].copy_from_slice(b"__TEXT");
            q(&mut b, 136, base + 0x100);
            q(&mut b, 144, 4);
            w(&mut b, 152, 256);
            w(&mut b, 160, 272);
            w(&mut b, 164, relocations);
            w(&mut b, 184, 2);
            w(&mut b, 188, 24);
            w(&mut b, 192, 288);
            w(&mut b, 196, 1);
            w(&mut b, 200, 304);
            w(&mut b, 204, 16);
            w(&mut b, 288, 1);
            b[304..313].copy_from_slice(b"\0_target\0");
            w(&mut b, 256, instruction);
            w(&mut b, 272, 0);
            w(&mut b, 276, (2 << 28) | (1 << 27) | (2 << 25) | (1 << 24));
            b
        };
        let object = make(1, 0, 0x94000000, 1);
        let native = make(2, 0x100000000, 0x94000004, 0);
        let pin = LinkedFunction {
            dwarf_name: "invoke".into(),
            linkage_name: "_invoke".into(),
            dwarf_offset: 0,
            object_address: 0x100,
            image_address: 0x100000100,
            image_offset: 0x100,
            object_path: "/owned.o".into(),
            function_size: 4,
            parameter_types: vec![],
            result_type: None,
            declaration_file: None,
            declaration_line: None,
        };
        let mut proof = text_proof(&object, &native, &pin).unwrap();
        assert!(proof.unchanged_bits_equal);
        assert!(!proof.relocation_targets_verified);
        direct_branch_targets(
            &object,
            &native,
            &pin,
            "0000000100000110 T _target",
            &mut proof,
        )
        .unwrap();
        assert!(proof.relocation_targets_verified);
        assert!(
            direct_branch_targets(
                &object,
                &native,
                &pin,
                "0000000100000120 T _target",
                &mut proof
            )
            .is_err()
        );
        let mut opcode = native.clone();
        opcode[259] ^= 0x80;
        assert!(text_proof(&object, &opcode, &pin).is_err());
        let no_relocation = make(1, 0, 0x94000000, 0);
        assert!(text_proof(&no_relocation, &native, &pin).is_err());
    }
    #[test]
    fn namespace_qualified_zst_and_ivars_are_distinct_and_no_name_fallback() {
        let text = "0x0000: DW_TAG_compile_unit\n0x0001:   DW_TAG_namespace\n DW_AT_name (\"ruycast_native\")\n0x0002:     DW_TAG_namespace\n DW_AT_name (\"native_menu_bar\")\n0x0003:       DW_TAG_structure_type\n DW_AT_name (\"Target\")\n DW_AT_byte_size (0x00)\n DW_AT_alignment (1)\n0x0004:       DW_TAG_structure_type\n DW_AT_name (\"Binding\")\n DW_AT_byte_size (0x10)\n DW_AT_alignment (8)\n0x0005:     DW_TAG_namespace\n DW_AT_name (\"actions\")\n0x0006:       DW_TAG_structure_type\n DW_AT_name (\"Target\")\n DW_AT_byte_size (0x28)\n DW_AT_alignment (8)\n";
        let dies = dwarf_dies(text).unwrap();
        assert_eq!(layout(&dies, TARGET).unwrap().size, 0);
        assert_eq!(layout(&dies, IVARS).unwrap().size, 16);
        assert!(layout(&dies, "Target").is_err());
        let duplicate = format!(
            "{text}0x0007:       DW_TAG_structure_type\n DW_AT_name (\"Target\")\n DW_AT_byte_size (0x08)\n DW_AT_alignment (8)\n"
        );
        assert!(
            layout(
                &dwarf_dies(&duplicate).unwrap(),
                "ruycast_native::actions::Target"
            )
            .is_err()
        );
    }
    #[test]
    fn declaration_references_reject_cycles_and_unsorted_objects() {
        let cycle = "0x0000: DW_TAG_compile_unit\n0x0001:   DW_TAG_subprogram\n DW_AT_specification (0x0002)\n0x0002:   DW_TAG_subprogram\n DW_AT_specification (0x0001)\n";
        let dies = dwarf_dies(cycle).unwrap();
        assert!(declaration(&dies, 1).is_err());
        assert!(dwarf_dies("0x0002: DW_TAG_compile_unit\n0x0001: DW_TAG_subprogram\n").is_err());
    }
    #[test]
    fn debug_map_never_matches_objects_by_basename_and_rejects_duplicates() {
        let text = "  - filename: '/owned/a.o'\n      - { sym: __Z_drop, objAddr: 0x20, binAddr: 0x100000020, size: 0x10 }\n  - filename: '/foreign/a.o'\n      - { sym: __Z_drop, objAddr: 0x20, binAddr: 0x100000040, size: 0x10 }\n";
        let entries = debug_map(text).unwrap();
        assert_eq!(entries.len(), 2);
        assert_ne!(entries[0].object, entries[1].object);
        assert!(debug_map(&format!("{text}      - {{ sym: __Z_drop, objAddr: 0x20, binAddr: 0x100000040, size: 0x10 }}\n")).is_err());
    }
    fn unnamed_image_fixture()->Vec<u8> {
        let mut image=vec![0u8;352];
        for (at,value) in [(0,0xfeedfacfu32),(4,0x0100000c),(12,2),(16,2),(20,176),
            (32,0x19),(36,152),(88,5),(92,5),(96,1),(152,256),(168,0x8000_0400),
            (184,2),(188,24),(192,320),(196,1),(200,336),(204,16),(320,1)] {
            image[at..at+4].copy_from_slice(&value.to_le_bytes());
        }
        image[40..46].copy_from_slice(b"__TEXT");
        image[56..64].copy_from_slice(&0x1_0000_0000u64.to_le_bytes());
        image[64..72].copy_from_slice(&0x1000u64.to_le_bytes());
        image[80..88].copy_from_slice(&352u64.to_le_bytes());
        image[104..110].copy_from_slice(b"__text");
        image[120..126].copy_from_slice(b"__TEXT");
        image[136..144].copy_from_slice(&0x1_0000_0100u64.to_le_bytes());
        image[144..152].copy_from_slice(&16u64.to_le_bytes());
        image[324]=0x0f;image[325]=1;
        image[328..336].copy_from_slice(&0x1_0000_0100u64.to_le_bytes());
        image[336..347].copy_from_slice(b"\0_callback\0");
        image
    }
    #[test]
    fn exact_die_extraction_supports_unnamed_bodies_without_changing_named_records() {
        let image=unnamed_image_fixture();
        let mut dies=dwarf_dies("0x0001: DW_TAG_compile_unit\n0x0002:   DW_TAG_subprogram\n DW_AT_linkage_name (\"callback\")\n DW_AT_low_pc (0x10)\n").unwrap();
        let map=vec![MapEntry{object:"/owned/input.o".into(),symbol:"_callback".into(),object_address:16,binary_address:0x1_0000_0100,size:4}];
        let unnamed=linked_function_at(&dies,1,"/owned/input.o",&map,None,&image).unwrap();
        assert_eq!(unnamed.dwarf_offset,2);
        assert!(unnamed.parameter_types.is_empty());
        assert!(unnamed.result_type.is_none());
        assert!(linked_function(&dies,"callback",None,"/owned/input.o",&map,"100000100 T _callback",&image).is_err());
        dies[1].attributes.insert("DW_AT_name".into(),"(\"callback\")".into());
        let named=linked_function(&dies,"callback",None,"/owned/input.o",&map,"100000100 T _callback",&image).unwrap();
        assert_eq!(serde_json::to_value(named).unwrap(),serde_json::to_value(unnamed).unwrap());
        assert!(linked_function_at(&dies,0,"/owned/input.o",&map,None,&image).is_err());
        assert!(linked_function_at(&dies,2,"/owned/input.o",&map,None,&image).is_err());
        assert!(linked_function_at(&dies,1,"/foreign/input.o",&map,None,&image).is_err());
    }
    #[test]
    fn linked_image_symbols_require_defined_text_and_bounded_real_strings() {
        let image=unnamed_image_fixture();
        assert_eq!(image_text_symbol_addresses(&image,"_callback").unwrap(),BTreeSet::from([0x1_0000_0100]));
        for (at,value) in [(324,0x01u8),(324,0xef),(325,2)] {
            let mut wrong=image.clone();wrong[at]=value;
            assert!(image_text_symbol_addresses(&wrong,"_callback").unwrap().is_empty());
        }
        let mut wrong=image.clone();wrong[320..324].copy_from_slice(&16u32.to_le_bytes());
        assert!(image_text_symbol_addresses(&wrong,"_callback").is_err());
        let mut wrong=image.clone();wrong[336..352].fill(b'x');
        assert!(image_text_symbol_addresses(&wrong,"_callback").is_err());
        let mut wrong=image.clone();wrong[196..200].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(image_text_symbol_addresses(&wrong,"_callback").is_err());
        let mut wrong=image.clone();wrong[328..336].copy_from_slice(&0x2_0000_0000u64.to_le_bytes());
        assert!(image_text_symbol_addresses(&wrong,"_callback").is_err());
    }
}

#[cfg(test)]
mod input_five_tests {
    use super::*;

    #[test]
    fn capsule_types_require_exact_c_abi_field_offsets_and_all_five_members() {
        let die=|offset,tag:&str,parent,name:&str| Die {offset,tag:tag.into(),parent,
            attributes:BTreeMap::from([("DW_AT_name".into(),format!("(\"{name}\")"))])};
        let mut dies=vec![die(1,"DW_TAG_namespace",None,"ruycast_native"),
            die(2,"DW_TAG_namespace",Some(0),"owned_host_entry"),
            die(3,"DW_TAG_structure_type",Some(1),"NativeInputCallbacksV1")];
        dies[2].attributes.insert("DW_AT_byte_size".into(),"(56)".into());
        dies[2].attributes.insert("DW_AT_alignment".into(),"(8)".into());
        let p="*mut core::ffi::c_void";
        let carbon=format!("unsafe extern \"C\" fn({p}, {p}, {p}) -> i32");
        let hid=format!("unsafe extern \"C\" fn({p}, i32, {p}, {p})");
        let tap=format!("unsafe extern \"C\" fn({p}, u32, {p}, {p}) -> {p}");
        for (index,(name,offset,kind)) in [("magic",0,"u64"),("abi",8,"u32"),("count",12,"u32"),
            ("carbon_pressed",16,carbon.as_str()),("hid_input",24,hid.as_str()),("hid_removed",32,hid.as_str()),
            ("hid_filter",40,tap.as_str()),("keyword_filter",48,tap.as_str())].into_iter().enumerate() {
            let mut member=die(index as u64+4,"DW_TAG_member",Some(2),name);
            member.attributes.insert("DW_AT_data_member_location".into(),format!("({offset})"));
            member.attributes.insert("DW_AT_type".into(),format!("(0x2000 \"{}\")",kind.replace('"',"\\\"")));
            dies.push(member);
        }
        assert_eq!(input_capsule_types(&dies).unwrap(),1);
        let copy=|dies:&[Die]|dies.iter().map(|die|Die{offset:die.offset,tag:die.tag.clone(),parent:die.parent,attributes:die.attributes.clone()}).collect::<Vec<_>>();
        let mut wrong=copy(&dies);
        wrong[6].attributes.get_mut("DW_AT_type").unwrap().replace_range(..,"(0x2000 \"unsafe extern \\\"C-unwind\\\" fn()\")");
        assert!(input_capsule_types(&wrong).is_err());
        let mut wrong=copy(&dies);
        wrong[6].attributes.insert("DW_AT_data_member_location".into(),"(24)".into());
        assert!(input_capsule_types(&wrong).is_err());
        dies.pop();
        assert!(input_capsule_types(&dies).is_err());
    }

    #[test]
    fn malformed_matching_body_is_not_skipped_for_another_good_object() {
        let good = dwarf_dies("0x0001: DW_TAG_compile_unit\n0x0002:   DW_TAG_namespace\n DW_AT_name (\"ruycast_native\")\n0x0003:     DW_TAG_namespace\n DW_AT_name (\"carbon_hotkeys\")\n0x0004:       DW_TAG_subprogram\n DW_AT_name (\"pressed\")\n DW_AT_low_pc (0x10)\n").unwrap();
        assert!(input_has_body(&good, "pressed", "ruycast_native::carbon_hotkeys").unwrap());
        let mut malformed = good;
        malformed[3].attributes.insert("DW_AT_specification".into(), "(0x9999)".into());
        assert!(input_has_body(&malformed, "pressed", "ruycast_native::carbon_hotkeys").is_err());
    }

    #[test]
    fn input_uuid_requires_one_bounded_nonzero_command() {
        let mut image = vec![0_u8; 56];
        for (at, value) in [
            (0, 0xfeedfacf_u32),
            (4, 0x0100000c),
            (16, 1),
            (20, 24),
            (32, 0x1b),
            (36, 24),
        ] {
            image[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        assert!(input_image_uuid(&image).is_err());
        image[40] = 7;
        assert_eq!(
            input_image_uuid(&image).unwrap(),
            "07000000000000000000000000000000"
        );
        image[36..40].copy_from_slice(&28_u32.to_le_bytes());
        assert!(input_image_uuid(&image).is_err());
        assert!(input_five_linked_code(&[], "", "", &image).is_err());
    }
}
