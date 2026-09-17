use crate::bitio::ByteReader;

pub const ABC_MAGIC: u32 = 0x002E0010;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NsKind {
    Private = 0x05,
    Namespace = 0x08,
    PackageNamespace = 0x16,
    PackageInternal = 0x17,
    Protected = 0x18,
    Explicit = 0x19,
    StaticProtected = 0x1A,
    Unknown = 0xFF,
}

impl From<u8> for NsKind {
    fn from(b: u8) -> Self {
        match b {
            0x05 => NsKind::Private,
            0x08 => NsKind::Namespace,
            0x16 => NsKind::PackageNamespace,
            0x17 => NsKind::PackageInternal,
            0x18 => NsKind::Protected,
            0x19 => NsKind::Explicit,
            0x1A => NsKind::StaticProtected,
            _ => NsKind::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MnKind {
    QName = 0x07,
    QNameA = 0x0D,
    RTQName = 0x0F,
    RTQNameA = 0x10,
    RTQNameLate = 0x11,
    RTQNameLateA = 0x12,
    Multiname = 0x09,
    MultinameA = 0x0E,
    MultinameLate = 0x1B,
    MultinameLateA = 0x1C,
    TypeName = 0x1D,
    Unknown = 0xFF,
}

impl From<u8> for MnKind {
    fn from(b: u8) -> Self {
        match b {
            0x07 => MnKind::QName,
            0x0D => MnKind::QNameA,
            0x0F => MnKind::RTQName,
            0x10 => MnKind::RTQNameA,
            0x11 => MnKind::RTQNameLate,
            0x12 => MnKind::RTQNameLateA,
            0x09 => MnKind::Multiname,
            0x0E => MnKind::MultinameA,
            0x1B => MnKind::MultinameLate,
            0x1C => MnKind::MultinameLateA,
            0x1D => MnKind::TypeName,
            _ => MnKind::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ValueKind {
    Undefined = 0x00,
    String = 0x01,
    Int = 0x03,
    UInt = 0x04,
    Double = 0x06,
    Namespace = 0x08,
    VFalse = 0x0A,
    VTrue = 0x0B,
    Null = 0x0C,
    PackageNs = 0x16,
    PackageInternalNs = 0x17,
    ProtectedNs = 0x18,
    ExplicitNs = 0x19,
    StaticProtectedNs = 0x1A,
    PrivateNs = 0x05,
    Unknown = 0xFF,
}

impl From<u8> for ValueKind {
    fn from(b: u8) -> Self {
        match b {
            0x00 => ValueKind::Undefined,
            0x01 => ValueKind::String,
            0x03 => ValueKind::Int,
            0x04 => ValueKind::UInt,
            0x06 => ValueKind::Double,
            0x08 => ValueKind::Namespace,
            0x0A => ValueKind::VFalse,
            0x0B => ValueKind::VTrue,
            0x0C => ValueKind::Null,
            0x16 => ValueKind::PackageNs,
            0x17 => ValueKind::PackageInternalNs,
            0x18 => ValueKind::ProtectedNs,
            0x19 => ValueKind::ExplicitNs,
            0x1A => ValueKind::StaticProtectedNs,
            0x05 => ValueKind::PrivateNs,
            _ => ValueKind::Unknown,
        }
    }
}

pub const MF_ARGUMENTS_DEFINED: u8 = 0x01;
pub const MF_NEW_ACTIVATION: u8 = 0x02;
pub const MF_VAR_ARGS: u8 = 0x04;
pub const MF_OPTIONAL_PARAMS: u8 = 0x08;
pub const MF_UNUSED: u8 = 0x10;
pub const MF_NATIVE: u8 = 0x20;
pub const MF_USES_DXNS: u8 = 0x40;
pub const MF_PARAM_NAMES: u8 = 0x80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TraitKind {
    Slot = 0,
    Method = 1,
    Getter = 2,
    Setter = 3,
    Class = 4,
    Function = 5,
    Const = 6,
    Unknown = 0xFF,
}

impl From<u8> for TraitKind {
    fn from(b: u8) -> Self {
        match b & 0x0F {
            0 => TraitKind::Slot,
            1 => TraitKind::Method,
            2 => TraitKind::Getter,
            3 => TraitKind::Setter,
            4 => TraitKind::Class,
            5 => TraitKind::Function,
            6 => TraitKind::Const,
            _ => TraitKind::Unknown,
        }
    }
}

pub const TRAIT_FINAL: u8 = 0x10;
pub const TRAIT_OVERRIDE: u8 = 0x20;
pub const TRAIT_METADATA: u8 = 0x40;

pub const CLASS_SEALED: u8 = 1;
pub const CLASS_FINAL: u8 = 2;
pub const CLASS_INTERFACE: u8 = 4;
pub const CLASS_PROTECTED_NS: u8 = 8;

#[derive(Debug, Clone)]
pub struct Namespace {
    pub kind: NsKind,
    pub name_idx: u32,
}

impl Namespace {
    pub fn resolve(&self, ctx: &AbcFile) -> String {
        if self.name_idx == 0 {
            match self.kind {
                NsKind::PackageNamespace
                | NsKind::PackageInternal
                | NsKind::Private
                | NsKind::StaticProtected => String::new(),
                _ => format!("<ns:{:?}>", self.kind),
            }
        } else {
            let idx = (self.name_idx - 1) as usize;
            if idx < ctx.strings.len() {
                ctx.strings[idx].clone()
            } else {
                format!("<str#{}>", self.name_idx)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Multiname {
    pub kind: MnKind,
    pub name_idx: u32,
    pub ns_idx: u32,
    pub nsset_idx: u32,
    pub base_idx: u32,
    pub param_idxs: Vec<u32>,
}

impl Multiname {
    pub fn is_any(&self) -> bool {
        self.kind == MnKind::QName && self.ns_idx == 0 && self.name_idx == 0
    }

    pub fn resolve(&self, ctx: &AbcFile) -> String {
        if self.is_any() {
            return "*".to_string();
        }
        match self.kind {
            MnKind::MultinameLate | MnKind::MultinameLateA => {
                if self.nsset_idx > 0 && ((self.nsset_idx - 1) as usize) < ctx.nsets.len() {
                    let nss = &ctx.nsets[(self.nsset_idx - 1) as usize];
                    let nsnames: Vec<String> = nss
                        .iter()
                        .map(|&n| {
                            if n > 0 && ((n - 1) as usize) < ctx.namespaces.len() {
                                ctx.namespaces[(n - 1) as usize].resolve(ctx)
                            } else {
                                "?".to_string()
                            }
                        })
                        .collect();
                    format!("RTMultiname({:?})", nsnames)
                } else {
                    "RTMultiname([])".to_string()
                }
            }
            MnKind::RTQNameLate | MnKind::RTQNameLateA => "RTLate".to_string(),
            MnKind::Multiname | MnKind::MultinameA => {
                let name = if self.name_idx > 0 && ((self.name_idx - 1) as usize) < ctx.strings.len() {
                    &ctx.strings[(self.name_idx - 1) as usize]
                } else {
                    "*"
                };
                let mut nsnames = Vec::new();
                if self.nsset_idx > 0 && ((self.nsset_idx - 1) as usize) < ctx.nsets.len() {
                    for &n in &ctx.nsets[(self.nsset_idx - 1) as usize] {
                        if n > 0 && ((n - 1) as usize) < ctx.namespaces.len() {
                            nsnames.push(ctx.namespaces[(n - 1) as usize].resolve(ctx));
                        }
                    }
                }
                format!("[{}]::{}", nsnames.join(" "), name)
            }
            MnKind::RTQName | MnKind::RTQNameA => {
                let name = if self.name_idx > 0 && ((self.name_idx - 1) as usize) < ctx.strings.len() {
                    &ctx.strings[(self.name_idx - 1) as usize]
                } else {
                    "*"
                };
                format!("RT::{}", name)
            }
            MnKind::QName | MnKind::QNameA => {
                let mut ns_name = String::new();
                if self.ns_idx > 0 && ((self.ns_idx - 1) as usize) < ctx.namespaces.len() {
                    let res = ctx.namespaces[(self.ns_idx - 1) as usize].resolve(ctx);
                    if !res.is_empty() {
                        ns_name = format!("{}::", res);
                    }
                }
                let name = if self.name_idx > 0 && ((self.name_idx - 1) as usize) < ctx.strings.len() {
                    &ctx.strings[(self.name_idx - 1) as usize]
                } else {
                    "*"
                };
                format!("{}{}", ns_name, name)
            }
            MnKind::TypeName => {
                let base = if self.base_idx > 0 && ((self.base_idx - 1) as usize) < ctx.multinames.len() {
                    ctx.multinames[(self.base_idx - 1) as usize].resolve(ctx)
                } else {
                    "?".to_string()
                };
                let params: Vec<String> = self
                    .param_idxs
                    .iter()
                    .map(|&p| {
                        if p > 0 && ((p - 1) as usize) < ctx.multinames.len() {
                            ctx.multinames[(p - 1) as usize].resolve(ctx)
                        } else {
                            "?".to_string()
                        }
                    })
                    .collect();
                format!("{}<{}>", base, params.join(","))
            }
            _ => format!(
                "<{:?}:{},{},{}>",
                self.kind, self.name_idx, self.ns_idx, self.nsset_idx
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Value {
    pub kind: ValueKind,
    pub data: u32,
    pub bool_val: bool,
}

impl Value {
    pub fn resolve(&self, ctx: &AbcFile) -> String {
        match self.kind {
            ValueKind::Undefined => "null".to_string(),
            ValueKind::VTrue => "true".to_string(),
            ValueKind::VFalse => "false".to_string(),
            ValueKind::Null => "null".to_string(),
            ValueKind::String => {
                if self.data > 0 && ((self.data - 1) as usize) < ctx.strings.len() {
                    ctx.strings[(self.data - 1) as usize].clone()
                } else {
                    "".to_string()
                }
            }
            ValueKind::Int => {
                if self.data > 0 && ((self.data - 1) as usize) < ctx.ints.len() {
                    ctx.ints[(self.data - 1) as usize].to_string()
                } else {
                    "0".to_string()
                }
            }
            ValueKind::UInt => {
                if self.data > 0 && ((self.data - 1) as usize) < ctx.uints.len() {
                    ctx.uints[(self.data - 1) as usize].to_string()
                } else {
                    "0".to_string()
                }
            }
            ValueKind::Double => {
                if self.data > 0 && ((self.data - 1) as usize) < ctx.doubles.len() {
                    format!("{:?}", ctx.doubles[(self.data - 1) as usize])
                } else {
                    "0.0".to_string()
                }
            }
            _ => format!("<{:?}:{}>", self.kind, self.data),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MethodInfo {
    pub idx: usize,
    pub params: Vec<u32>,
    pub returns: u32,
    pub debug_name_idx: u32,
    pub flags: u8,
    pub optional_params: Vec<Value>,
    pub param_names: Vec<u32>,
    pub body_idx: Option<usize>,
}

impl MethodInfo {
    pub fn native(&self) -> bool {
        (self.flags & MF_NATIVE) != 0
    }
    pub fn has_optional(&self) -> bool {
        (self.flags & MF_OPTIONAL_PARAMS) != 0
    }
    pub fn has_param_names(&self) -> bool {
        (self.flags & MF_PARAM_NAMES) != 0
    }
    pub fn need_activation(&self) -> bool {
        (self.flags & MF_NEW_ACTIVATION) != 0
    }
    pub fn var_args(&self) -> bool {
        (self.flags & MF_VAR_ARGS) != 0
    }
}

#[derive(Debug, Clone)]
pub struct ExceptionInfo {
    pub from_off: usize,
    pub to_off: usize,
    pub target_off: usize,
    pub type_idx: u32,
    pub name_idx: u32,
}

#[derive(Debug, Clone)]
pub struct Instruction {
    pub offset: usize,
    pub opcode: u8,
    pub name: &'static str,
    pub operands: Vec<i64>,
    pub length: usize,
    pub cases: Vec<i32>,
}

#[derive(Debug, Clone)]
pub struct Trait {
    pub name_idx: u32,
    pub kind: TraitKind,
    pub slot_id: u32,
    pub attrs: u8,
    pub type_idx: u32,
    pub value: Option<Value>,
    pub method_idx: usize,
    pub class_idx: usize,
    pub metadata: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct MethodBody {
    pub method_idx: usize,
    pub max_stack: u32,
    pub local_count: u32,
    pub init_scope_depth: u32,
    pub max_scope_depth: u32,
    pub code: Vec<u8>,
    pub instructions: Vec<Instruction>,
    pub exceptions: Vec<ExceptionInfo>,
    pub traits: Vec<Trait>,
}

#[derive(Debug, Clone)]
pub struct MetadataInfo {
    pub name_idx: u32,
    pub items: Vec<(u32, u32)>,
}

#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub idx: usize,
    pub name_idx: u32,
    pub super_idx: u32,
    pub flags: u8,
    pub protected_ns_idx: u32,
    pub interfaces: Vec<u32>,
    pub constructor_idx: usize,
    pub traits: Vec<Trait>,
    pub static_ctor_idx: usize,
    pub static_traits: Vec<Trait>,
}

impl ClassInfo {
    pub fn sealed(&self) -> bool {
        (self.flags & CLASS_SEALED) != 0
    }
    pub fn final_class(&self) -> bool {
        (self.flags & CLASS_FINAL) != 0
    }
    pub fn interface(&self) -> bool {
        (self.flags & CLASS_INTERFACE) != 0
    }
}

#[derive(Debug, Clone)]
pub struct ScriptInfo {
    pub init_method_idx: usize,
    pub traits: Vec<Trait>,
}

#[derive(Debug, Clone, Default)]
pub struct AbcFile {
    pub ints: Vec<i32>,
    pub uints: Vec<u32>,
    pub doubles: Vec<f64>,
    pub strings: Vec<String>,
    pub namespaces: Vec<Namespace>,
    pub nsets: Vec<Vec<u32>>,
    pub multinames: Vec<Multiname>,
    pub methods: Vec<MethodInfo>,
    pub metadatas: Vec<MetadataInfo>,
    pub classes: Vec<ClassInfo>,
    pub scripts: Vec<ScriptInfo>,
    pub bodies: Vec<MethodBody>,
    pub parse_warnings: Vec<String>,
}

pub fn read_vint(br: &mut ByteReader) -> Result<i32, String> {
    let mut result: u32 = 0;
    for &shift in &[0, 7, 14, 21, 28] {
        let b = br.u8()?;
        if shift < 28 {
            result |= ((b & 0x7F) as u32) << shift;
            if (b & 0x80) == 0 {
                break;
            }
        } else {
            if b > 0x0F {
                return Err("Invalid vint (fifth byte too large)".to_string());
            }
            result |= (b as u32) << 28;
            break;
        }
    }
    Ok(result as i32)
}

pub fn read_vint_u30(br: &mut ByteReader) -> Result<u32, String> {
    let mut result: u32 = 0;
    for &shift in &[0, 7, 14, 21] {
        let b = br.u8()?;
        result |= ((b & 0x7F) as u32) << shift;
        if (b & 0x80) == 0 {
            return Ok(result);
        }
    }
    let b = br.u8()?;
    if b > 0x0F {
        return Err("Invalid u30".to_string());
    }
    result |= (b as u32) << 28;
    Ok(result)
}

#[derive(Debug, Clone, Copy)]
pub struct OpDef {
    pub name: &'static str,
    pub fmt: &'static str,
}

pub fn get_opdef(op: u8) -> Option<OpDef> {
    let (name, fmt) = match op {
        0x01 => ("bkpt", ""),
        0x02 => ("nop", ""),
        0x03 => ("throw", ""),
        0x04 => ("getsuper", "id"),
        0x05 => ("setsuper", "id"),
        0x06 => ("dxns", "i"),
        0x07 => ("dxnslate", ""),
        0x08 => ("kill", "r"),
        0x09 => ("label", ""),
        0x0C => ("ifnlt", "i24"),
        0x0D => ("ifnle", "i24"),
        0x0E => ("ifngt", "i24"),
        0x0F => ("ifnge", "i24"),
        0x10 => ("jmp", "i24"),
        0x11 => ("iftrue", "i24"),
        0x12 => ("iffalse", "i24"),
        0x13 => ("ifeq", "i24"),
        0x14 => ("ifne", "i24"),
        0x15 => ("iflt", "i24"),
        0x16 => ("ifle", "i24"),
        0x17 => ("ifgt", "i24"),
        0x18 => ("ifge", "i24"),
        0x19 => ("ifstricteq", "i24"),
        0x1A => ("ifstrictne", "i24"),
        0x1B => ("lookupswitch", "sw"),
        0x1C => ("pushwith", ""),
        0x1D => ("popscope", ""),
        0x1E => ("nextname", ""),
        0x1F => ("hasnext", ""),
        0x20 => ("pushnull", ""),
        0x21 => ("pushundefined", ""),
        0x23 => ("nextvalue", ""),
        0x24 => ("pushbyte", "b"),
        0x25 => ("pushshort", "s32"),
        0x26 => ("pushtrue", ""),
        0x27 => ("pushfalse", ""),
        0x28 => ("pushnan", ""),
        0x29 => ("pop", ""),
        0x2A => ("dup", ""),
        0x2B => ("swap", ""),
        0x2C => ("pushstring", "i"),
        0x2D => ("pushint", "i"),
        0x2E => ("pushuint", "i"),
        0x2F => ("pushdouble", "i"),
        0x30 => ("pushscope", ""),
        0x31 => ("pushnamespace", "i"),
        0x32 => ("hasnext2", "rn"),
        0x40 => ("newfunction", "i"),
        0x41 => ("call", "i"),
        0x42 => ("construct", "i"),
        0x43 => ("callmethod", "rs"),
        0x44 => ("callstatic", "mn"),
        0x45 => ("callsuper", "is"),
        0x46 => ("callproperty", "is"),
        0x47 => ("returnvoid", ""),
        0x48 => ("returnvalue", ""),
        0x49 => ("constructsuper", "i"),
        0x4A => ("constructprop", "is"),
        0x4C => ("callproplex", "is"),
        0x4E => ("callsupervoid", "is"),
        0x4F => ("callpropvoid", "is"),
        0x50 => ("sxi1", ""),
        0x51 => ("sxi8", ""),
        0x52 => ("sxi16", ""),
        0x53 => ("applytype", "i"),
        0x55 => ("newobject", "i"),
        0x56 => ("newarray", "i"),
        0x57 => ("newactivation", ""),
        0x58 => ("newclass", "i"),
        0x59 => ("getdescendants", "id"),
        0x5A => ("newcatch", "i"),
        0x5D => ("findpropstrict", "id"),
        0x5E => ("findproperty", "id"),
        0x5F => ("finddef", "id"),
        0x60 => ("getlex", "id"),
        0x61 => ("setproperty", "id"),
        0x62 => ("getlocal", "r"),
        0x63 => ("setlocal", "r"),
        0x64 => ("getglobalscope", ""),
        0x65 => ("getscope", "b8"),
        0x66 => ("getproperty", "id"),
        0x68 => ("initproperty", "id"),
        0x6A => ("deleteproperty", "id"),
        0x6C => ("getslot", "i"),
        0x6D => ("setslot", "i"),
        0x6E => ("getglobalslot", "i"),
        0x6F => ("setglobalslot", "i"),
        0x70 => ("convert_s", ""),
        0x71 => ("esc_xelem", ""),
        0x72 => ("esc_xattr", ""),
        0x73 => ("convert_i", ""),
        0x74 => ("convert_u", ""),
        0x75 => ("convert_d", ""),
        0x76 => ("convert_b", ""),
        0x77 => ("convert_o", ""),
        0x78 => ("checkfilter", ""),
        0x80 => ("coerce", "id"),
        0x82 => ("coerce_a", ""),
        0x85 => ("coerce_s", ""),
        0x86 => ("astype", "id"),
        0x87 => ("astypelate", ""),
        0x89 => ("coerce_o", ""),
        0x90 => ("negate", ""),
        0x91 => ("increment", ""),
        0x92 => ("inclocal", "r"),
        0x93 => ("decrement", ""),
        0x94 => ("declocal", "r"),
        0x95 => ("typeof", ""),
        0x96 => ("not", ""),
        0x97 => ("bitnot", ""),
        0xA0 => ("add", ""),
        0xA1 => ("subtract", ""),
        0xA2 => ("multiply", ""),
        0xA3 => ("divide", ""),
        0xA4 => ("modulo", ""),
        0xA5 => ("lshift", ""),
        0xA6 => ("rshift", ""),
        0xA7 => ("urshift", ""),
        0xA8 => ("bitand", ""),
        0xA9 => ("bitor", ""),
        0xAA => ("bitxor", ""),
        0xAB => ("equals", ""),
        0xAC => ("strictequals", ""),
        0xAD => ("lessthan", ""),
        0xAE => ("lessequals", ""),
        0xAF => ("greaterthan", ""),
        0xB0 => ("greaterequals", ""),
        0xB1 => ("instanceof", ""),
        0xB2 => ("istype", "id"),
        0xB3 => ("istypelate", ""),
        0xB4 => ("in", ""),
        0xC0 => ("increment_i", ""),
        0xC1 => ("decrement_i", ""),
        0xC2 => ("inclocal_i", "r"),
        0xC3 => ("declocal_i", "r"),
        0xC4 => ("negate_i", ""),
        0xC5 => ("add_i", ""),
        0xC6 => ("subtract_i", ""),
        0xC7 => ("multiply_i", ""),
        0xD0 => ("getlocal0", ""),
        0xD1 => ("getlocal1", ""),
        0xD2 => ("getlocal2", ""),
        0xD3 => ("getlocal3", ""),
        0xD4 => ("setlocal0", ""),
        0xD5 => ("setlocal1", ""),
        0xD6 => ("setlocal2", ""),
        0xD7 => ("setlocal3", ""),
        0xEF => ("debug", "dbg"),
        0xF0 => ("debugline", "i"),
        0xF1 => ("debugfile", "i"),
        0xF2 => ("bkptline", "i"),
        0xF3 => ("timestamp", ""),
        _ => return None,
    };
    Some(OpDef { name, fmt })
}

fn read_int_pool(br: &mut ByteReader) -> Result<Vec<i32>, String> {
    let count = read_vint_u30(br)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity((count - 1) as usize);
    for _ in 0..(count - 1) {
        out.push(read_vint(br)?);
    }
    Ok(out)
}

fn read_uint_pool(br: &mut ByteReader) -> Result<Vec<u32>, String> {
    let count = read_vint_u30(br)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity((count - 1) as usize);
    for _ in 0..(count - 1) {
        out.push(read_vint_u30(br)?);
    }
    Ok(out)
}

fn read_double_pool(br: &mut ByteReader) -> Result<Vec<f64>, String> {
    let count = read_vint_u30(br)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity((count - 1) as usize);
    for _ in 0..(count - 1) {
        out.push(br.f64()?);
    }
    Ok(out)
}

fn read_string_pool(br: &mut ByteReader) -> Result<Vec<String>, String> {
    let count = read_vint_u30(br)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity((count - 1) as usize);
    for _ in 0..(count - 1) {
        let n = read_vint_u30(br)? as usize;
        out.push(br.string(n)?);
    }
    Ok(out)
}

fn read_ns_pool(br: &mut ByteReader) -> Result<Vec<Namespace>, String> {
    let count = read_vint_u30(br)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity((count - 1) as usize);
    for _ in 0..(count - 1) {
        let kind = NsKind::from(br.u8()?);
        let name_idx = read_vint_u30(br)?;
        out.push(Namespace { kind, name_idx });
    }
    Ok(out)
}

fn read_nsset_pool(br: &mut ByteReader) -> Result<Vec<Vec<u32>>, String> {
    let count = read_vint_u30(br)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity((count - 1) as usize);
    for _ in 0..(count - 1) {
        let n = br.u8()? as usize;
        let mut set = Vec::with_capacity(n);
        for _ in 0..n {
            set.push(read_vint_u30(br)?);
        }
        out.push(set);
    }
    Ok(out)
}

fn read_multiname(br: &mut ByteReader) -> Result<Multiname, String> {
    let k = br.u8()?;
    let kind = MnKind::from(k);
    match kind {
        MnKind::QName | MnKind::QNameA => {
            let ns_idx = read_vint_u30(br)?;
            let name_idx = read_vint_u30(br)?;
            Ok(Multiname {
                kind,
                name_idx,
                ns_idx,
                nsset_idx: 0,
                base_idx: 0,
                param_idxs: Vec::new(),
            })
        }
        MnKind::RTQName | MnKind::RTQNameA => {
            let name_idx = read_vint_u30(br)?;
            Ok(Multiname {
                kind,
                name_idx,
                ns_idx: 0,
                nsset_idx: 0,
                base_idx: 0,
                param_idxs: Vec::new(),
            })
        }
        MnKind::RTQNameLate | MnKind::RTQNameLateA => Ok(Multiname {
            kind,
            name_idx: 0,
            ns_idx: 0,
            nsset_idx: 0,
            base_idx: 0,
            param_idxs: Vec::new(),
        }),
        MnKind::Multiname | MnKind::MultinameA => {
            let name_idx = read_vint_u30(br)?;
            let nsset_idx = read_vint_u30(br)?;
            Ok(Multiname {
                kind,
                name_idx,
                ns_idx: 0,
                nsset_idx,
                base_idx: 0,
                param_idxs: Vec::new(),
            })
        }
        MnKind::MultinameLate | MnKind::MultinameLateA => {
            let nsset_idx = read_vint_u30(br)?;
            Ok(Multiname {
                kind,
                name_idx: 0,
                ns_idx: 0,
                nsset_idx,
                base_idx: 0,
                param_idxs: Vec::new(),
            })
        }
        MnKind::TypeName => {
            let name_idx = read_vint_u30(br)?;
            let n_params = br.u8()? as usize;
            let mut param_indices = Vec::with_capacity(n_params);
            for _ in 0..n_params {
                param_indices.push(read_vint_u30(br)?);
            }
            Ok(Multiname {
                kind,
                name_idx: 0,
                ns_idx: 0,
                nsset_idx: 0,
                base_idx: name_idx,
                param_idxs: param_indices,
            })
        }
        _ => Err(format!("Unknown multiname kind 0x{:02X}", k)),
    }
}

fn read_multiname_pool(br: &mut ByteReader) -> Result<Vec<Multiname>, String> {
    let count = read_vint_u30(br)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity((count - 1) as usize);
    for _ in 0..(count - 1) {
        out.push(read_multiname(br)?);
    }
    Ok(out)
}

pub fn read_value(br: &mut ByteReader, extra: bool) -> Value {
    let idx = read_vint_u30(br).unwrap_or(0);
    let kind_val = if extra || idx != 0 {
        br.u8().unwrap_or(0)
    } else {
        0
    };
    if kind_val == 0 {
        Value {
            kind: ValueKind::Undefined,
            data: 0,
            bool_val: false,
        }
    } else if kind_val == ValueKind::VFalse as u8 {
        Value {
            kind: ValueKind::VFalse,
            data: 0,
            bool_val: false,
        }
    } else if kind_val == ValueKind::VTrue as u8 {
        Value {
            kind: ValueKind::VTrue,
            data: 0,
            bool_val: true,
        }
    } else if kind_val == ValueKind::Null as u8 {
        Value {
            kind: ValueKind::Null,
            data: 0,
            bool_val: false,
        }
    } else {
        Value {
            kind: ValueKind::from(kind_val),
            data: idx,
            bool_val: false,
        }
    }
}

fn read_method(br: &mut ByteReader, idx: usize) -> Result<MethodInfo, String> {
    let param_count = read_vint_u30(br)? as usize;
    let ret_idx = read_vint_u30(br)?;
    let mut params = Vec::with_capacity(param_count);
    for _ in 0..param_count {
        params.push(read_vint_u30(br)?);
    }
    let debug_name = read_vint_u30(br)?;
    let flags = br.u8()?;
    let mut optionals = Vec::new();
    if (flags & MF_OPTIONAL_PARAMS) != 0 {
        let nopt = read_vint_u30(br)? as usize;
        for _ in 0..nopt {
            optionals.push(read_value(br, true));
        }
    }
    let mut param_names = Vec::new();
    if (flags & MF_PARAM_NAMES) != 0 {
        for _ in 0..param_count {
            param_names.push(read_vint_u30(br)?);
        }
    }
    Ok(MethodInfo {
        idx,
        params,
        returns: ret_idx,
        debug_name_idx: debug_name,
        flags,
        optional_params: optionals,
        param_names,
        body_idx: None,
    })
}

fn read_trait(br: &mut ByteReader) -> Result<Trait, String> {
    let name = read_vint_u30(br)?;
    let kind_byte = br.u8()?;
    let kind = TraitKind::from(kind_byte);
    let attrs = kind_byte & 0xF0;
    let slot = read_vint_u30(br)?;
    let mut t = Trait {
        name_idx: name,
        kind,
        slot_id: slot,
        attrs,
        type_idx: 0,
        value: None,
        method_idx: 0,
        class_idx: 0,
        metadata: Vec::new(),
    };
    match kind {
        TraitKind::Slot | TraitKind::Const => {
            t.type_idx = read_vint_u30(br)?;
            t.value = Some(read_value(br, false));
        }
        TraitKind::Method | TraitKind::Getter | TraitKind::Setter | TraitKind::Function => {
            t.method_idx = read_vint_u30(br)? as usize;
        }
        TraitKind::Class => {
            t.class_idx = read_vint_u30(br)? as usize;
        }
        _ => {}
    }
    if (attrs & TRAIT_METADATA) != 0 {
        let n = read_vint_u30(br)? as usize;
        for _ in 0..n {
            t.metadata.push(read_vint_u30(br)?);
        }
    }
    Ok(t)
}

fn read_trait_list(br: &mut ByteReader) -> Result<Vec<Trait>, String> {
    let count = read_vint_u30(br)? as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        out.push(read_trait(br)?);
    }
    Ok(out)
}

fn read_class(br: &mut ByteReader, idx: usize) -> Result<ClassInfo, String> {
    let name = read_vint_u30(br)?;
    let super_mn = read_vint_u30(br)?;
    let flags = br.u8()?;
    let mut protected_ns = 0;
    if (flags & CLASS_PROTECTED_NS) != 0 {
        protected_ns = read_vint_u30(br)?;
    }
    let n_interfaces = read_vint_u30(br)? as usize;
    let mut interfaces = Vec::with_capacity(n_interfaces);
    for _ in 0..n_interfaces {
        interfaces.push(read_vint_u30(br)?);
    }
    let constructor = read_vint_u30(br)? as usize;
    let traits = read_trait_list(br)?;
    Ok(ClassInfo {
        idx,
        name_idx: name,
        super_idx: super_mn,
        flags,
        protected_ns_idx: protected_ns,
        interfaces,
        constructor_idx: constructor,
        traits,
        static_ctor_idx: 0,
        static_traits: Vec::new(),
    })
}

fn read_instr(br: &mut ByteReader, base: usize) -> Result<Instruction, String> {
    let off = br.tell() - base;
    let op = br.u8()?;
    let defn = get_opdef(op);
    let name: &'static str = match defn {
        Some(d) => d.name,
        None => Box::leak(format!("unknown_0x{:02X}", op).into_boxed_str()),
    };
    let fmt = defn.map(|d| d.fmt).unwrap_or("");
    let mut ops = Vec::new();
    let mut cases = Vec::new();

    match fmt {
        "i" | "r" | "id" => {
            let v = read_vint_u30(br)? as i64;
            ops.push(v);
        }
        "b" => {
            let v = br.s8()? as i64;
            ops.push(v);
        }
        "b8" => {
            let v = br.u8()? as i64;
            ops.push(v);
        }
        "s32" => {
            let v = read_vint(br)? as i64;
            ops.push(v);
        }
        "dbg" => {
            let t = br.u8()? as i64;
            let idx = read_vint_u30(br)? as i64;
            let reg = br.u8()? as i64;
            let extra = read_vint_u30(br)? as i64;
            ops.extend_from_slice(&[t, idx, reg, extra]);
        }
        "i24" => {
            let v = br.s24()? as i64;
            ops.push(v);
        }
        "is" | "rs" => {
            let idx = read_vint_u30(br)? as i64;
            let nargs = read_vint_u30(br)? as i64;
            ops.push(idx);
            ops.push(nargs);
        }
        "mn" => {
            let mid = read_vint_u30(br)? as i64;
            let nargs = read_vint_u30(br)? as i64;
            ops.push(mid);
            ops.push(nargs);
        }
        "rn" => {
            let r1 = read_vint_u30(br)? as i64;
            let r2 = read_vint_u30(br)? as i64;
            ops.push(r1);
            ops.push(r2);
        }
        "sw" => {
            let default_off = br.s24()?;
            let ncases_plus_1 = read_vint_u30(br)? as usize;
            let ncases = ncases_plus_1 + 1;
            ops.push(default_off as i64);
            for _ in 0..ncases {
                cases.push(br.s24()?);
            }
        }
        _ => {}
    }

    let length = br.tell() - base - off;
    Ok(Instruction {
        offset: off,
        opcode: op,
        name,
        operands: ops,
        length,
        cases,
    })
}

fn disassemble(code: &[u8]) -> Vec<Instruction> {
    let mut br = ByteReader::new(code, 0);
    let mut instrs = Vec::new();
    while br.avail() > 0 {
        match read_instr(&mut br, 0) {
            Ok(ins) => instrs.push(ins),
            Err(_) => {
                instrs.push(Instruction {
                    offset: br.tell(),
                    opcode: 0x02,
                    name: "nop",
                    operands: Vec::new(),
                    length: 1,
                    cases: Vec::new(),
                });
                break;
            }
        }
    }
    instrs
}

fn read_exception(br: &mut ByteReader) -> Result<ExceptionInfo, String> {
    let f = read_vint_u30(br)? as usize;
    let t = read_vint_u30(br)? as usize;
    let h = read_vint_u30(br)? as usize;
    let ex_t = read_vint_u30(br)?;
    let ex_n = read_vint_u30(br)?;
    Ok(ExceptionInfo {
        from_off: f,
        to_off: t,
        target_off: h,
        type_idx: ex_t,
        name_idx: ex_n,
    })
}

fn read_body(br: &mut ByteReader) -> Result<MethodBody, String> {
    let method_idx = read_vint_u30(br)? as usize;
    let max_stack = read_vint_u30(br)?;
    let local_count = read_vint_u30(br)?;
    let init_scope = read_vint_u30(br)?;
    let max_scope = read_vint_u30(br)?;
    let code_len = read_vint_u30(br)? as usize;
    let code_slice = br.bytes(code_len)?;
    let code = code_slice.to_vec();
    let instructions = disassemble(&code);
    let n_excs = read_vint_u30(br)? as usize;
    let mut excs = Vec::with_capacity(n_excs);
    for _ in 0..n_excs {
        excs.push(read_exception(br)?);
    }
    let traits = read_trait_list(br)?;
    Ok(MethodBody {
        method_idx,
        max_stack,
        local_count,
        init_scope_depth: init_scope,
        max_scope_depth: max_scope,
        code,
        instructions,
        exceptions: excs,
        traits,
    })
}

pub fn parse_abc(data: &[u8]) -> Result<AbcFile, String> {
    let mut warnings = Vec::new();
    let mut br = ByteReader::new(data, 0);

    let magic = br.u32()?;
    if magic != ABC_MAGIC {
        br.seek(0);
        let b0 = br.u8()? as u32;
        let b1 = br.u8()? as u32;
        let b2 = br.u8()? as u32;
        let b3 = br.u8()? as u32;
        let magic_val = b0 | (b1 << 8) | (b2 << 16) | (b3 << 24);
        if magic_val != ABC_MAGIC {
            return Err(format!(
                "Invalid ABC magic: expected 0x{:08X}, got bytes {:02X} {:02X} {:02X} {:02X}",
                ABC_MAGIC, b0, b1, b2, b3
            ));
        }
    }

    let ints = read_int_pool(&mut br)?;
    let uints = read_uint_pool(&mut br)?;
    let doubles = read_double_pool(&mut br)?;
    let strings = read_string_pool(&mut br)?;
    let namespaces = read_ns_pool(&mut br)?;
    let nsets = read_nsset_pool(&mut br)?;
    let multinames = read_multiname_pool(&mut br)?;

    let method_count = read_vint_u30(&mut br)? as usize;
    let mut methods = Vec::with_capacity(method_count);
    for i in 0..method_count {
        methods.push(read_method(&mut br, i)?);
    }

    let meta_count = read_vint_u30(&mut br)? as usize;
    let mut metadatas = Vec::with_capacity(meta_count);
    for _ in 0..meta_count {
        let name = read_vint_u30(&mut br)?;
        let n = read_vint_u30(&mut br)? as usize;
        let mut items = Vec::with_capacity(n);
        for _ in 0..n {
            let k = read_vint_u30(&mut br)?;
            let v = read_vint_u30(&mut br)?;
            items.push((k, v));
        }
        metadatas.push(MetadataInfo {
            name_idx: name,
            items,
        });
    }

    let class_count = read_vint_u30(&mut br)? as usize;
    let mut classes = Vec::with_capacity(class_count);
    for i in 0..class_count {
        classes.push(read_class(&mut br, i)?);
    }

    for cls in &mut classes {
        cls.static_ctor_idx = read_vint_u30(&mut br)? as usize;
        cls.static_traits = read_trait_list(&mut br)?;
    }

    let script_count = read_vint_u30(&mut br)? as usize;
    let mut scripts = Vec::with_capacity(script_count);
    for _ in 0..script_count {
        let init_idx = read_vint_u30(&mut br)? as usize;
        let traits = read_trait_list(&mut br)?;
        scripts.push(ScriptInfo {
            init_method_idx: init_idx,
            traits,
        });
    }

    let body_count = read_vint_u30(&mut br)? as usize;
    let mut bodies = Vec::with_capacity(body_count);
    for _ in 0..body_count {
        match read_body(&mut br) {
            Ok(body) => {
                let midx = body.method_idx;
                if midx < methods.len() {
                    methods[midx].body_idx = Some(bodies.len());
                }
                bodies.push(body);
            }
            Err(e) => {
                warnings.push(format!("method body skipped: {}", e));
                break;
            }
        }
    }

    Ok(AbcFile {
        ints,
        uints,
        doubles,
        strings,
        namespaces,
        nsets,
        multinames,
        methods,
        metadatas,
        classes,
        scripts,
        bodies,
        parse_warnings: warnings,
    })
}
