use std::sync::atomic::{AtomicBool, Ordering};

use crate::abc::AbcFile;
use crate::control_flow::{is_empty_seq, seq_of, SNode};
use crate::ir::IrNode;

pub static HAXE_MODE: AtomicBool = AtomicBool::new(true);

pub fn haxe_mode() -> bool {
    HAXE_MODE.load(Ordering::Relaxed)
}

pub fn set_haxe_mode(v: bool) {
    HAXE_MODE.store(v, Ordering::Relaxed);
}

const ROOT_RUNTIME_EXACT: &[&str] = &["Std", "Type", "Reflect", "EReg", "Dynamic", "EnumValue", "Void"];

fn is_root_runtime_name(base: &str) -> bool {
    if ROOT_RUNTIME_EXACT.contains(&base) {
        return true;
    }
    if let Some(tail) = base.strip_prefix("boot_") {
        if !tail.is_empty() && tail.len() == 4 {
            return tail.chars().all(|c| c.is_ascii_hexdigit());
        }
    }
    false
}

pub fn is_runtime_full_name(full: &str) -> bool {
    let (pkg, base) = match full.rfind('.') {
        Some(i) => (&full[..i], &full[i + 1..]),
        None => ("", full),
    };
    const RUNTIME_PKGS: &[&str] = &[
        "haxe", "flash", "hxd", "openfl", "format", "haxe.ds", "haxe.io",
        "haxe.iterators", "haxe.xml", "haxe.crypto", "haxe.ds._StringMap",
        "haxe.ds._IntMap", "haxe.ds._ObjectMap", "haxe.ds._HashMap",
        "haxe._EnumValueMap", "flash._Boot", "flash._Lib", "flash.display",
        "flash.events", "flash.net", "flash.media", "flash.text", "flash.utils",
    ];
    if RUNTIME_PKGS.contains(&pkg) {
        return true;
    }
    for p in ["haxe.", "flash.", "hxd.", "openfl.", "format."] {
        if pkg.starts_with(p) {
            return true;
        }
    }
    if pkg.is_empty() && haxe_mode() && is_root_runtime_name(base) {
        return true;
    }
    false
}

pub fn normalize_class_name(abc: &AbcFile, class_info: &crate::abc::ClassInfo) -> String {
    if class_info.name_idx == 0 {
        return "?".to_string();
    }
    let Some(mn) = abc.multinames.get((class_info.name_idx - 1) as usize) else {
        return "?".to_string();
    };
    let r = mn.resolve(abc);
    if r.starts_with('[') {
        if let Some(end) = r.find(']') {
            let pkg = &r[1..end];
            let base = r[end + 1..].rsplit("::").next().unwrap_or("");
            if !pkg.is_empty() {
                return format!("{}.{}", pkg, base);
            }
            return base.to_string();
        }
    }
    r.replace("::", ".")
}

pub fn is_trace_call_node(node: &IrNode) -> Option<IrNode> {
    let mut args: Option<&Vec<IrNode>> = None;
    match node {
        IrNode::Call { func, args: a } => match &**func {
            IrNode::NameRef { name, .. } => {
                if name == "trace" || name == "haxe::Log.trace" || name == "Log.trace" {
                    args = Some(a);
                }
            }
            IrNode::PropGet { prop, .. } => {
                if let IrNode::NameRef { name, .. } = &**prop {
                    if name == "trace" {
                        args = Some(a);
                    }
                }
            }
            _ => {}
        },
        IrNode::CallProp { obj, prop, args: a, .. } => {
            let mut recv_ok = false;
            match &**obj {
                IrNode::NameRef { name, .. } => {
                    let rn = name.rsplit("::").next().unwrap_or("").rsplit('.').next().unwrap_or("");
                    recv_ok = rn == "Log";
                }
                IrNode::PropGet { prop: p2, .. } => {
                    if let IrNode::NameRef { name, .. } = &**p2 {
                        let rn = name.rsplit("::").next().unwrap_or("");
                        recv_ok = rn == "Log";
                    }
                }
                _ => {}
            }
            if recv_ok {
                if let IrNode::NameRef { name, .. } = &**prop {
                    if name == "trace" && a.len() >= 2 {
                        args = Some(a);
                    }
                }
            }
        }
        _ => {}
    }
    if let Some(args) = args {
        if args.len() >= 2 {
            if let IrNode::NewObject(props) = &args[1] {
                let keys: std::collections::HashSet<String> = props
                    .iter()
                    .filter_map(|(k, _)| match k {
                        IrNode::StringConst(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
                for req in ["fileName", "lineNumber", "className", "methodName"] {
                    if !keys.contains(req) {
                        return None;
                    }
                }
                return Some(args[0].clone());
            }
        }
    }
    None
}

pub fn is_trace_call(cp: &IrNode) -> bool {
    let IrNode::CallProp { obj: _, prop, args, is_void, .. } = cp else {
        return false;
    };
    if !*is_void {
        return false;
    }
    let IrNode::NameRef { name, .. } = &**prop else { return false };
    if name != "trace" || args.len() != 2 {
        return false;
    }
    let IrNode::NewObject(props) = &args[1] else { return false };
    let keys: Vec<String> = props
        .iter()
        .filter_map(|(k, _)| match k {
            IrNode::StringConst(s) => Some(s.clone()),
            _ => None,
        })
        .collect();
    for req in ["fileName", "lineNumber", "className", "methodName"] {
        if !keys.contains(&req.to_string()) {
            return false;
        }
    }
    true
}

fn ref_base_name(fn_node: &IrNode) -> String {
    match fn_node {
        IrNode::NameRef { name, .. } => name.clone(),
        IrNode::PropGet { obj, prop, .. } => {
            if let IrNode::NameRef { name, .. } = &**prop {
                let base = ref_base_name(obj);
                if !base.is_empty() {
                    format!("{}.{}", base, name)
                } else {
                    name.clone()
                }
            } else {
                String::new()
            }
        }
        IrNode::CallProp { obj, prop, .. } => {
            if let IrNode::NameRef { name, .. } = &**prop {
                let base = ref_base_name(obj);
                if !base.is_empty() {
                    format!("{}.{}", base, name)
                } else {
                    name.clone()
                }
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

fn short(fn_node: &IrNode) -> String {
    ref_base_name(fn_node)
        .rsplit("::")
        .next()
        .unwrap_or("")
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_string()
}

pub fn map_runtime_call(func: &IrNode, args: &[IrNode]) -> Option<String> {
    if !haxe_mode() {
        return None;
    }
    let s = short(func);
    let nargs = args.len();
    match s.as_str() {
        "__string_rec" if nargs >= 1 => {
            Some(format!("Std.string({})", emit_expr(&args[0], 0)))
        }
        "__instanceof" if nargs == 2 => Some(format!(
            "Std.isOfType({}, {})",
            emit_expr(&args[0], 9),
            emit_expr(&args[1], 0)
        )),
        "enum_to_string" if nargs >= 1 => {
            Some(format!("Std.string({})", emit_expr(&args[0], 0)))
        }
        "__unprotect__" if nargs == 1 => Some(emit_expr(&args[0], 0)),
        "__typeof" if nargs == 1 => Some(format!("Type.typeof({})", emit_expr(&args[0], 0))),
        "__interfLoop" if nargs == 2 => Some(String::new()),
        "cca" if nargs == 2 => Some(format!(
            "StringTools.fastCodeAt({}, {})",
            emit_expr(&args[0], 16),
            emit_expr(&args[1], 0)
        )),
        "thrown" if nargs == 1 && ref_base_name(func).contains("Exception") => {
            Some(emit_expr(&args[0], 0))
        }
        "unwrap" if nargs == 0 => {
            let mut inner = func.clone();
            loop {
                match inner {
                    IrNode::Cast { expr, .. } => inner = *expr,
                    IrNode::Convert { expr, .. } => inner = *expr,
                    IrNode::PropGet { obj, .. } => inner = *obj,
                    IrNode::CallProp { obj, .. } if short(&inner) == "unwrap" => inner = *obj,
                    _ => break,
                }
            }
            if let IrNode::Cast { expr, .. } = inner {
                inner = *expr;
            } else if let IrNode::Convert { expr, .. } = inner {
                inner = *expr;
            }
            match inner {
                IrNode::Call { func: f, args: a } if short(&f) == "caught" && !a.is_empty() => {
                    Some(emit_expr(&a[0], 0))
                }
                IrNode::CallProp { args: a, .. } if short(&inner) == "caught" && !a.is_empty() => {
                    Some(emit_expr(&a[0], 0))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn notable_flip(op: &str) -> Option<&'static str> {
    Some(match op {
        "eq" => "neq",
        "neq" => "eq",
        "stricteq" => "strictne",
        "strictne" => "stricteq",
        "lt" => "gte",
        "gte" => "lt",
        "gt" => "lte",
        "lte" => "gt",
        _ => return None,
    })
}

pub fn simplify_not(cond: &IrNode) -> IrNode {
    match cond {
        IrNode::UnaryOp { op, expr } if op == "not" => (**expr).clone(),
        IrNode::BinaryOp { op, left, right } => match notable_flip(op) {
            Some(flip) => IrNode::BinaryOp {
                op: flip.to_string(),
                left: left.clone(),
                right: right.clone(),
            },
            None => IrNode::UnaryOp { op: "not".into(), expr: Box::new(cond.clone()) },
        },
        other => IrNode::UnaryOp { op: "not".into(), expr: Box::new(other.clone()) },
    }
}

fn shortcircuit(op: &str, left: &IrNode, right: &IrNode, prec: i32) -> String {
    let p = if op == "&&" { 14 } else { 15 };
    let ls = emit_expr(left, p);
    let rs = emit_expr(right, p + 1);
    let s = format!("{} {} {}", ls, op, rs);
    if prec > 0 {
        format!("({})", s)
    } else {
        s
    }
}

fn phi_expr(node: &IrNode, prec: i32) -> String {
    let IrNode::Phi { inputs } = node else { return "__unknown__".to_string() };
    if inputs.len() == 2 {
        let branch = inputs.iter().find(|(t, _)| matches!(t, crate::ir::PhiTag::Branch { .. }));
        if let Some((bt, bv)) = branch {
            if let crate::ir::PhiTag::Branch { cond, taken } = bt {
                let edge_cond = if *taken { cond.clone() } else { simplify_not(cond) };
                let plain_v: &IrNode = inputs.iter().find(|(t, _)| matches!(t, crate::ir::PhiTag::Plain)).map(|(_, v)| v).unwrap_or(bv);
                match bv {
                    IrNode::BoolConst(false) =>
                        return shortcircuit("&&", &simplify_not(&edge_cond), plain_v, prec),
                    IrNode::BoolConst(true) =>
                        return shortcircuit("||", &edge_cond, plain_v, prec),
                    _ => {}
                }
                if let IrNode::BoolConst(false) = plain_v {
                    return shortcircuit("&&", &edge_cond, bv, prec);
                }
                if let IrNode::BoolConst(true) = plain_v {
                    return shortcircuit("||", &simplify_not(&edge_cond), bv, prec);
                }
            }
        }
    }
    let tagged: Vec<(&crate::ir::PhiTag, &IrNode)> = inputs
        .iter()
        .filter(|(t, _)| matches!(t, crate::ir::PhiTag::Branch { .. }))
        .map(|(t, v)| (t, v))
        .collect();
    if tagged.len() == 2 {
        let (t1, v1) = tagged[0];
        let (t2, v2) = tagged[1];
        if let (
            crate::ir::PhiTag::Branch { cond: c1, taken: p1 },
            crate::ir::PhiTag::Branch { cond: c2, taken: p2 },
        ) = (t1, t2)
        {
            if *p1 && matches!(v1, IrNode::BoolConst(_)) {
                let other = v2;
                if let IrNode::BoolConst(false) = v1 {
                    return shortcircuit("&&", &simplify_not(c1), other, prec);
                }
                if let IrNode::BoolConst(true) = v1 {
                    return shortcircuit("||", c1, other, prec);
                }
            }
            if *p2 && matches!(v2, IrNode::BoolConst(_)) {
                let other = v1;
                if let IrNode::BoolConst(false) = v2 {
                    return shortcircuit("&&", &simplify_not(c2), other, prec);
                }
                if let IrNode::BoolConst(true) = v2 {
                    return shortcircuit("||", c2, other, prec);
                }
            }
            if !*p1 && matches!(v1, IrNode::BoolConst(false)) {
                return shortcircuit("&&", c1, v2, prec);
            }
            if !*p2 && matches!(v2, IrNode::BoolConst(false)) {
                return shortcircuit("&&", c2, v1, prec);
            }
            if c1 == c2 {
                let (tv, ev) = if !*p1 { (v1, v2) } else { (v2, v1) };
                let cond = simplify_not(c1);
                let s = format!(
                    "{} ? {} : {}",
                    emit_expr(&cond, 2),
                    emit_expr(tv, 0),
                    emit_expr(ev, 0)
                );
                return if prec > 1 { format!("({})", s) } else { s };
            }
        }
    }
    let vals: Vec<&IrNode> = inputs.iter().map(|(_, v)| v).collect();
    if !vals.is_empty() {
        return format!("{} /* phi({}) */", emit_expr(vals[0], prec), vals.len());
    }
    "__unknown__".to_string()
}

fn is_computed_key(node: &IrNode) -> bool {
    let node = peel_implicit(node);
    match node {
        IrNode::StringConst(_) | IrNode::NameRef { .. } => false,
        IrNode::IntConst(_) | IrNode::UIntConst(_) | IrNode::DoubleConst(_) => false,
        _ => true,
    }
}

pub fn haxe_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn map_abstract_of(impl_short: &str) -> String {
    let k = match impl_short {
        "StringMap" | "UnsafeStringMap" => "String",
        "IntMap" => "Int",
        "EnumValueMap" => "EnumValue",
        "ObjectMap" => "{}",
        _ => "Dynamic",
    };
    format!("Map<{}, Dynamic>", k)
}

fn peel_implicit(node: &IrNode) -> IrNode {
    let mut cur = node.clone();
    loop {
        match cur {
            IrNode::Cast { expr, .. } | IrNode::Convert { expr, .. } => cur = *expr,
            other => return other,
        }
    }
}

fn is_valid_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !s.chars().next().unwrap().is_ascii_digit()
}

fn name_ref_short(name: &str) -> String {
    let n = name.rsplit("::").next().unwrap_or(name).to_string();
    const SHORT_MAP: &[(&str, &str)] = &[
        ("String", "String"), ("int", "Int"), ("uint", "UInt"),
        ("Boolean", "Bool"), ("Number", "Float"), ("void", "Void"),
        ("*", "*"), ("Object", "Dynamic"), ("Array", "Array"),
        ("Function", "Dynamic"), ("Class", "Class"), ("Error", "haxe.Exception"),
        ("XML", "Xml"), ("XMLList", "Xml"),
    ];
    for (k, v) in SHORT_MAP {
        if n == *k {
            return v.to_string();
        }
    }
    n.replace("::", ".")
}

pub fn emit_expr(node: &IrNode, prec: i32) -> String {
    match node {
        IrNode::Nop => String::new(),
        IrNode::CatchVarRef(name) => name.clone(),
        IrNode::This | IrNode::ImplicitThis => "this".to_string(),
        IrNode::NullConst => "null".to_string(),
        IrNode::UndefinedConst => "undefined".to_string(),
        IrNode::BoolConst(v) => if *v { "true" } else { "false" }.to_string(),
        IrNode::IntConst(v) => v.to_string(),
        IrNode::UIntConst(v) => v.to_string(),
        IrNode::DoubleConst(v) => py_repr_f64(*v),
        IrNode::NaNConst => "Math.NaN".to_string(),
        IrNode::StringConst(s) => haxe_str(s),
        IrNode::NameRef { name, .. } => name_ref_short(name),
        IrNode::RegAccess { reg, hint } => hint
            .clone()
            .unwrap_or_else(|| format!("_variable{}_", reg)),
        IrNode::ParamRef(idx) => format!("_capture{}_", idx + 1),
        IrNode::VarDecl { name, ty, init, .. } => {
            let mut s = format!("var {}", name);
            if let Some(t) = ty {
                s += &format!(" : {}", emit_expr(t, 0));
            }
            if let Some(i) = init {
                s += &format!(" = {}", emit_expr(i, 0));
            }
            s
        }
        IrNode::Ternary { cond, then_val, else_val } => {
            let s = format!(
                "{} ? {} : {}",
                emit_expr(cond, 2),
                emit_expr(then_val, 0),
                emit_expr(else_val, 0)
            );
            if prec > 1 { format!("({})", s) } else { s }
        }
        IrNode::Phi { .. } => phi_expr(node, prec),
        IrNode::UnknownValue(why) => format!("__unknown__/*{}*/", why),
        IrNode::ExceptionValue => "__exception__".to_string(),
        IrNode::GlobalScope => "__global__".to_string(),
        IrNode::Scope(depth) => format!("__scope{}__", depth),
        IrNode::NewActivation => "<activation>".to_string(),
        IrNode::UnaryOp { op, expr } => {
            let sym = match op.as_str() {
                "neg" | "neg_i" => "-",
                "not" => "!",
                "bitnot" => "~",
                "increment" | "increment_i" => "++",
                "decrement" | "decrement_i" => "--",
                other => other,
            };
            let e = emit_expr(expr, 14);
            if op.starts_with("increment") || op.starts_with("decrement") {
                let lit = peel_implicit(expr);
                let d = if op.starts_with("increment") { 1i64 } else { -1 };
                match lit {
                    IrNode::IntConst(v) => return (v as i64 + d).to_string(),
                    IrNode::UIntConst(v) => return (v as i64 + d).to_string(),
                    _ => {}
                }
                return format!("{}{}", e, sym);
            }
            format!("{}{}", sym, e)
        }
        IrNode::BinaryOp { op, left, right } => {
            let sym = match op.as_str() {
                "add" | "add_i" => "+",
                "sub" | "sub_i" => "-",
                "mul" | "mul_i" => "*",
                "div" => "/",
                "mod" => "%",
                "shl" => "<<",
                "shr" => ">>",
                "ushr" => ">>>",
                "and" => "&",
                "or" => "|",
                "xor" => "^",
                "eq" => "==",
                "neq" => "!=",
                "stricteq" => "===",
                "strictne" => "!==",
                "lt" => "<",
                "lte" => "<=",
                "gt" => ">",
                "gte" => ">=",
                "instanceof" => "Std.isOfType",
                "in" => "in",
                other => return format!("{} {} {}", emit_expr(left, 0), other, emit_expr(right, 0)),
            };
            let p: i32 = match op.as_str() {
                "mul" | "mul_i" | "div" | "mod" => 5,
                "add" | "sub" | "add_i" | "sub_i" => 6,
                "shl" | "shr" | "ushr" => 7,
                "lt" | "lte" | "gt" | "gte" | "in" | "instanceof" => 9,
                "eq" | "neq" | "stricteq" | "strictne" => 10,
                "and" => 11,
                "xor" => 12,
                "or" => 13,
                _ => 6,
            };
            let le = emit_expr(left, p);
            let re = emit_expr(right, p + 1);
            let s = format!("{} {} {}", le, sym, re);
            if prec > p { format!("({})", s) } else { s }
        }
        IrNode::Convert { expr, target } => {
            let _ = target;
            emit_expr(expr, prec)
        }
        IrNode::Cast { expr, target_type, is_strict } => {
            let e = emit_expr(expr, prec);
            if !*is_strict {
                return e;
            }
            let t = emit_expr(target_type, 0);
            if ["String", "Dynamic", "Bool", "Int", "UInt", "Float", "*"].contains(&t.as_str()) {
                return e;
            }
            format!("(cast {}: {})", e, t)
        }
        IrNode::CheckXml(expr) => emit_expr(expr, prec),
        IrNode::PropGet { obj, prop, is_dynamic } => {
            let mut obj_node = (**obj).clone();
            if haxe_mode() {
                if let IrNode::PropGet { obj: inner_obj, prop: inner_prop, .. } = &obj_node {
                    if let IrNode::NameRef { name, .. } = &**inner_prop {
                        if name == "h"
                            && (*is_dynamic
                                || matches!(&**prop, IrNode::StringConst(_))
                                || is_computed_key(prop))
                        {
                            obj_node = peel_implicit(inner_obj);
                        }
                    }
                }
            }
            let mut obj_s = emit_expr(&obj_node, 16);
            if matches!(**obj, IrNode::ImplicitThis) {
                obj_s = String::new();
            }
            if let IrNode::StringConst(key) = &**prop {
                let key = haxe_str(key);
                return if obj_s.is_empty() { format!("this[{}]", key) } else { format!("{}[{}]", obj_s, key) };
            }
            if *is_dynamic || is_computed_key(prop) {
                let key = emit_expr(prop, 0);
                return if obj_s.is_empty() { format!("this[{}]", key) } else { format!("{}[{}]", obj_s, key) };
            }
            let prop_s = emit_expr(prop, 0);
            if is_valid_ident(&prop_s) {
                if obj_s.is_empty() {
                    prop_s
                } else {
                    format!("{}.{}", obj_s, prop_s)
                }
            } else if obj_s.is_empty() {
                format!("this[{}]", prop_s)
            } else {
                format!("{}[{}]", obj_s, prop_s)
            }
        }
        IrNode::PropSet { obj, prop, value, is_dynamic, .. } => {
            let mut obj_node = (**obj).clone();
            if haxe_mode() {
                if let IrNode::PropGet { obj: inner_obj, prop: inner_prop, .. } = &obj_node {
                    if let IrNode::NameRef { name, .. } = &**inner_prop {
                        if name == "h"
                            && (*is_dynamic
                                || matches!(&**prop, IrNode::StringConst(_))
                                || is_computed_key(prop))
                        {
                            obj_node = peel_implicit(inner_obj);
                        }
                    }
                }
            }
            let obj_s = emit_expr(&obj_node, 16);
            let prop_s = emit_expr(prop, 0);
            if matches!(**obj, IrNode::ImplicitThis) {
                return format!("{} = {}", prop_s, emit_expr(value, 0));
            }
            if *is_dynamic || matches!(&**prop, IrNode::StringConst(_)) || is_computed_key(prop) {
                format!("{}[{}] = {}", obj_s, prop_s, emit_expr(value, 0))
            } else {
                format!("{}.{} = {}", obj_s, prop_s, emit_expr(value, 0))
            }
        }
        IrNode::PropertyLookup { name, .. } => name.clone(),
        IrNode::PropDelete { obj, prop } => {
            format!("delete {}.{}", emit_expr(obj, 0), emit_expr(prop, 0))
        }
        IrNode::SlotGet { obj, slot } => {
            format!("{}.__slot{}__", emit_expr(obj, 0), slot)
        }
        IrNode::SlotSet { obj, slot, value } => {
            format!(
                "{}.__slot{}__ = {}",
                emit_expr(obj, 0),
                slot,
                emit_expr(value, 0)
            )
        }
        IrNode::Call { func, args } => {
            if let Some(trace_arg) = is_trace_call_node(node) {
                return format!("trace({})", emit_expr(&trace_arg, 0));
            }
            if let Some(mapped) = map_runtime_call(func, args) {
                return mapped;
            }
            let fn_s = emit_expr(func, 17);
            let args_str = args
                .iter()
                .map(|a| emit_expr(a, 0))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({})", fn_s, args_str)
        }
        IrNode::CallProp { obj, prop, args, .. } => {
            if let Some(trace_arg) = is_trace_call_node(node) {
                return format!("trace({})", emit_expr(&trace_arg, 0));
            }
            if let Some(mapped) = map_runtime_call(node, args) {
                return mapped;
            }
            let args_str = args
                .iter()
                .map(|a| emit_expr(a, 0))
                .collect::<Vec<_>>()
                .join(", ");
            let recv_s = emit_expr(obj, 16);
            let prop_s = emit_expr(prop, 0);
            if matches!(**obj, IrNode::ImplicitThis) {
                return format!("{}({})", prop_s, args_str);
            }
            if matches!(**obj, IrNode::GlobalScope) {
                return format!("{}({})", prop_s, args_str);
            }
            if is_valid_ident(&prop_s) {
                format!("{}.{}({})", recv_s, prop_s, args_str)
            } else {
                format!("{}[{}]({})", recv_s, prop_s, args_str)
            }
        }
        IrNode::CallStatic { method, args } => {
            let args_str = args
                .iter()
                .map(|a| emit_expr(a, 0))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({})", emit_expr(method, 0), args_str)
        }
        IrNode::CallSuper { prop, args, .. } => {
            let args_str = args
                .iter()
                .map(|a| emit_expr(a, 0))
                .collect::<Vec<_>>()
                .join(", ");
            format!("super.{}({})", emit_expr(prop, 0), args_str)
        }
        IrNode::ConstructSuper(args) => {
            let args_str = args
                .iter()
                .map(|a| emit_expr(a, 0))
                .collect::<Vec<_>>()
                .join(", ");
            format!("super({})", args_str)
        }
        IrNode::Construct { cls, args } => {
            let cls_s = emit_expr(cls, 0);
            let args_str = args
                .iter()
                .map(|a| emit_expr(a, 0))
                .collect::<Vec<_>>()
                .join(", ");
            if haxe_mode() {
                if let IrNode::NameRef { name, .. } = &**cls {
                    let short = name.rsplit("::").next().unwrap_or("");
                    if ["StringMap", "IntMap", "ObjectMap", "HashMap", "UnsafeStringMap", "EnumValueMap"]
                        .contains(&short)
                    {
                        return format!(
                            "new {}({})",
                            map_abstract_of(short),
                            args_str
                        );
                    }
                }
            }
            format!("new {}({})", cls_s, args_str)
        }
        IrNode::ConstructProp { cls, args } => {
            let args_str = args
                .iter()
                .map(|a| emit_expr(a, 0))
                .collect::<Vec<_>>()
                .join(", ");
            if let IrNode::NameRef { name, .. } = &**cls {
                let cn = name.rsplit("::").next().unwrap_or("");
                if haxe_mode()
                    && ["StringMap", "IntMap", "ObjectMap", "HashMap", "UnsafeStringMap", "EnumValueMap"]
                        .contains(&cn)
                {
                    return format!("new {}({})", map_abstract_of(cn), args_str);
                }
                return format!("new {}({})", emit_expr(cls, 0), args_str);
            }
            format!("new ({})({})", emit_expr(cls, 0), args_str)
        }
        IrNode::ApplyType { base, params } => {
            let ps = params
                .iter()
                .map(|p| emit_expr(p, 0))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}<{}>", emit_expr(base, 0), ps)
        }
        IrNode::NewObject(props) => {
            let parts: Vec<String> = props
                .iter()
                .map(|(k, v)| match k {
                    IrNode::StringConst(s) => format!("{}: {}", s, emit_expr(v, 0)),
                    other => format!("{}: {}", emit_expr(other, 0), emit_expr(v, 0)),
                })
                .collect();
            format!("{{ {} }}", parts.join(", "))
        }
        IrNode::NewArray(items) => {
            let parts: Vec<String> = items.iter().map(|a| emit_expr(a, 0)).collect();
            format!("[{}]", parts.join(", "))
        }
        IrNode::NewFunction { method_index, .. } => {
            format!("function() {{ /* m{} */ }}", method_index)
        }
        IrNode::NewClass { class_index, .. } => format!("class#c{}", class_index),
        IrNode::RegSet { reg, value, hint } => {
            let name = hint
                .clone()
                .unwrap_or_else(|| format!("_variable{}_", reg));
            if let IrNode::UnaryOp { op, expr } = &**value {
                if (op == "increment" || op == "decrement" || op == "increment_i" || op == "decrement_i")
                    && matches!(**expr, IrNode::RegAccess { reg: r, .. } if r == *reg)
                {
                    let sym = if op.starts_with("increment") { "++" } else { "--" };
                    return format!("{}{}", name, sym);
                }
            }
            format!("{} = {}", name, emit_expr(value, 0))
        }
        IrNode::PushScope(v) => format!("// pushscope {}", emit_expr(v, 0)),
        IrNode::PopScope => "// popscope".to_string(),
        IrNode::Return(None) => "return".to_string(),
        IrNode::Return(Some(v)) => format!("return {}", emit_expr(v, 0)),
        IrNode::Throw(v) => format!("throw {}", emit_expr(v, 0)),
        IrNode::Dup => "/*dup*/".to_string(),
        IrNode::Swap => "/*swap*/".to_string(),
        IrNode::Pop => "/*pop*/".to_string(),
        IrNode::Kill(reg) => format!("// kill r{}", reg),
        IrNode::NamespaceRef { kind, name } => format!("/* ns {} {} */", kind, name),
        IrNode::SetScopeValue { ns, .. } => format!("// dxns {}", emit_expr(ns, 0)),
        IrNode::DebugLine(_) | IrNode::DebugFile(_) | IrNode::DebugReg { .. } => String::new(),
        IrNode::Label(name) => format!("// {}:", name),
        IrNode::Goto { target, cond } => {
            let c = cond.as_ref().map(|c| emit_expr(c, 0)).unwrap_or_default();
            format!("// goto {} {}", target, c)
        }
        IrNode::MemOp { op, args } => {
            let parts: Vec<String> = args.iter().map(|a| emit_expr(a, 0)).collect();
            format!("__{}({})", op, parts.join(", "))
        }
        IrNode::NewCatch { catch_id, .. } => format!("catch_block#{}", catch_id),
        IrNode::NextIter { .. } => "/*for-in iter*/".to_string(),
        IrNode::HasNext { obj_reg, idx_reg, .. } => {
            format!("__hasNext2(__r{}, __r{})", obj_reg, idx_reg)
        }
        IrNode::ExprStmt(e) => emit_expr(e, 0),
        other => format!("{:?}", other),
    }
}

pub fn py_repr_f64(v: f64) -> String {
    if v.is_nan() {
        return "nan".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf".to_string() } else { "-inf".to_string() };
    }
    if v == v.trunc() && v.abs() < 1e16 {
        format!("{:.1}", v)
    } else {
        let s = format!("{}", v);
        s
    }
}

pub fn emit_stmt(node: &IrNode, indent: usize) -> Vec<String> {
    let pad = "    ".repeat(indent);
    let s = emit_expr(node, 0);
    if s.is_empty() || s.chars().all(|c| c.is_whitespace()) {
        return Vec::new();
    }
    vec![format!("{}{};", pad, s)]
}

pub fn render_tree(
    n: &SNode,
    indent: usize,
    abc: Option<&AbcFile>,
    field_types: &std::collections::HashMap<String, String>,
    enums: &std::collections::HashMap<usize, crate::haxe_layer::EnumDef>,
) -> Vec<String> {
    let pad = "    ".repeat(indent);
    let mut out: Vec<String> = Vec::new();
    match n {
        SNode::Removed => {}
        SNode::Seq(stmts) => {
            for s in stmts {
                out.extend(render_tree(s, indent, abc, field_types, enums));
            }
        }
        SNode::Expr(node) => {
            if let IrNode::Block(stmts) = node {
                for st in stmts {
                    out.extend(render_tree(&SNode::Expr(st.clone()), indent, abc, field_types, enums));
                }
                return out;
            }
            let mut target = node.clone();
            if let IrNode::ExprStmt(e) = &target {
                target = (**e).clone();
            }
            if let IrNode::VarDecl { name, init: Some(init), .. } = &target {
                let mut _v = (**init).clone();
                loop {
                    match _v {
                        IrNode::Cast { expr, .. } | IrNode::Convert { expr, .. } => _v = *expr,
                        _ => break,
                    }
                }
                if let IrNode::NewFunction { .. } = _v {
                    if let Some(abc) = abc {
                        out.extend(crate::haxe_out::render_closure_decl(
                            name, &_v, indent, abc, field_types, enums,
                        ));
                        return out;
                    }
                }
                let s = emit_expr(&target, 0);
                if !s.is_empty() && !s.chars().all(|c| c.is_whitespace()) {
                    out.push(format!("{}{};", pad, s));
                }
                return out;
            }
            if let IrNode::RegSet { reg, value, hint } = &target {
                let mut _v = (**value).clone();
                loop {
                    match _v {
                        IrNode::Cast { expr, .. } | IrNode::Convert { expr, .. } => _v = *expr,
                        _ => break,
                    }
                }
                if let IrNode::NewFunction { .. } = _v {
                    if let Some(abc) = abc {
                        let name = hint
                            .clone()
                            .unwrap_or_else(|| format!("_variable{}_", reg));
                        out.extend(render_closure_assign(&name, &_v, indent, abc, field_types, enums));
                        return out;
                    }
                }
            }
            if let IrNode::PropSet { prop, value, .. } = &target {
                let mut _v = (**value).clone();
                loop {
                    match _v {
                        IrNode::Cast { expr, .. } | IrNode::Convert { expr, .. } => _v = *expr,
                        _ => break,
                    }
                }
                if let IrNode::NewFunction { .. } = _v {
                    if let Some(abc) = abc {
                        out.extend(render_closure_propset(&target, &_v, indent, abc, field_types, enums));
                        return out;
                    }
                }
                let _ = prop;
            }
            let s = emit_expr(node, 0);
            if !s.is_empty() && !s.chars().all(|c| c.is_whitespace()) {
                if s.starts_with("//") {
                    out.push(format!("{}{}", pad, s));
                } else {
                    out.push(format!("{}{};", pad, s));
                }
            }
        }
        SNode::If { cond, then_body, else_body } => {
            let c = emit_expr(cond, 0);
            out.push(format!("{}if ({}) {{", pad, c));
            out.extend(render_tree(then_body, indent + 1, abc, field_types, enums));
            let mut els = else_body.clone();
            while let Some(e) = els {
                if let SNode::Seq(stmts) = &*e {
                    if stmts.len() == 1 {
                        if let SNode::If { cond: ic, then_body: itb, else_body: ie } = &stmts[0] {
                            out.push(format!("{}}} else if ({}) {{", pad, emit_expr(ic, 0)));
                            out.extend(render_tree(itb, indent + 1, abc, field_types, enums));
                            els = ie.clone();
                            continue;
                        }
                    }
                }
                if !is_empty_seq(&e) {
                    out.push(format!("{}}} else {{", pad));
                    out.extend(render_tree(&e, indent + 1, abc, field_types, enums));
                }
                break;
            }
            out.push(format!("{}}}", pad));
        }
        SNode::While { cond, body } => {
            let c = cond.as_ref().map(|c| emit_expr(c, 0)).unwrap_or_else(|| "true".to_string());
            out.push(format!("{}while ({}) {{", pad, c));
            out.extend(render_tree(body, indent + 1, abc, field_types, enums));
            out.push(format!("{}}}", pad));
        }
        SNode::DoWhile { cond, body } => {
            let c = emit_expr(cond, 0);
            out.push(format!("{}do {{", pad));
            out.extend(render_tree(body, indent + 1, abc, field_types, enums));
            out.push(format!("{}}} while ({});", pad, c));
        }
        SNode::ForIn { var_name, kind, obj, body, val_name, .. } => {
            let os = emit_expr(obj, 0);
            match kind.as_str() {
                "map" => out.push(format!("{}for ({} => {} in {}) {{", pad, var_name, val_name, os)),
                "mapkey" => out.push(format!("{}for ({} in {}.keys()) {{", pad, var_name, os)),
                _ => out.push(format!("{}for ({} in {}) {{", pad, var_name, os)),
            }
            out.extend(render_tree(body, indent + 1, abc, field_types, enums));
            out.push(format!("{}}}", pad));
        }
        SNode::ForRange { var_name, start, end, body, .. } => {
            out.push(format!(
                "{}for ({} in {}...{}) {{",
                pad,
                var_name,
                emit_expr(start, 0),
                emit_expr(end, 0)
            ));
            out.extend(render_tree(body, indent + 1, abc, field_types, enums));
            out.push(format!("{}}}", pad));
        }
        SNode::Switch { value, cases, has_default: _, match_enum, match_base, case_plan, expr_return } => {
            render_switch(n, &pad, indent, abc, field_types, enums, &mut out,
                          value, cases, match_enum, match_base, case_plan, *expr_return);
        }
        SNode::Try { body, catches, finally_body } => {
            out.push(format!("{}try {{", pad));
            out.extend(render_tree(body, indent + 1, abc, field_types, enums));
            for (var, ty, cbody) in catches {
                out.push(format!("{}}} catch ({}:{}) {{", pad, var, ty));
                out.extend(render_tree(cbody, indent + 1, abc, field_types, enums));
            }
            if let Some(fb) = finally_body {
                out.push(format!("{}}} finally {{", pad));
                out.extend(render_tree(fb, indent + 1, abc, field_types, enums));
            }
            out.push(format!("{}}}", pad));
        }
        SNode::Throw(v) => out.push(format!("{}throw {};", pad, emit_expr(v, 0))),
        SNode::Return(None) => out.push(format!("{}return;", pad)),
        SNode::Return(Some(v)) => out.push(format!("{}return {};", pad, emit_expr(v, 0))),
        SNode::Break => out.push(format!("{}break;", pad)),
        SNode::Continue => out.push(format!("{}continue;", pad)),
        SNode::Goto { target, comment } => {
            let c = if comment.is_empty() { String::new() } else { format!(" // {}", comment) };
            out.push(format!("{}// goto {}{}", pad, target, c));
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn render_switch(
    _n: &SNode,
    pad: &str,
    indent: usize,
    abc: Option<&AbcFile>,
    field_types: &std::collections::HashMap<String, String>,
    enums: &std::collections::HashMap<usize, crate::haxe_layer::EnumDef>,
    out: &mut Vec<String>,
    value: &IrNode,
    cases: &[(Vec<i32>, SNode)],
    match_enum: &Option<crate::haxe_layer::EnumDef>,
    match_base: &Option<IrNode>,
    case_plan: &Option<Vec<(Option<Vec<crate::haxe_layer::EnumCtor>>, SNode)>>,
    expr_return: bool,
) {
    if expr_return {
        let disc = if match_enum.is_some() {
            match_base.as_ref().map(|b| emit_expr(b, 0)).unwrap_or_default()
        } else {
            emit_expr(value, 0)
        };
        out.push(format!("{}return switch ({}) {{", pad, disc));
        let entries: Vec<(Option<Vec<crate::haxe_layer::EnumCtor>>, &SNode)> =
            if let Some(plan) = case_plan {
                plan.iter().map(|(c, b)| (c.clone(), b)).collect()
            } else {
                cases.iter().map(|(_vals, b)| (None, b)).collect()
            };
        for (ctors, body) in entries {
            let stmts = seq_of(body);
            let ret_node = if stmts.len() == 1 {
                match &stmts[0] {
                    SNode::Return(v) => Some(v.clone()),
                    _ => None,
                }
            } else {
                None
            };
            let Some(ret_value) = ret_node.flatten() else {
                out.extend(render_tree(body, indent + 1, abc, field_types, enums));
                continue;
            };
            match &ctors {
                None | Some(_) if ctors.as_ref().map(|c| c.is_empty()).unwrap_or(true) && case_plan.is_none() => {
                    let vals = &cases
                        .iter()
                        .find(|(_, b)| std::ptr::eq(b, body))
                        .map(|(v, _)| v.clone())
                        .unwrap_or_default();
                    if vals.is_empty() {
                        out.push(format!("{}    case _: {};", pad, emit_expr(&ret_value, 0)));
                    } else {
                        let vals_str = vals.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ");
                        out.push(format!("{}    case {}: {};", pad, vals_str, emit_expr(&ret_value, 0)));
                    }
                }
                None => {
                    if ret_node_matches_none(&ret_value) {
                        continue;
                    }
                    out.push(format!("{}    case _: {};", pad, emit_expr(&ret_value, 0)));
                }
                Some(ctors) => {
                    let hdr = ctors
                        .iter()
                        .map(|c| {
                            if c.nparams == 0 {
                                c.name.clone()
                            } else {
                                let caps: Vec<String> =
                                    (0..c.nparams).map(|i| format!("_capture{}_", i + 1)).collect();
                                format!("{}({})", c.name, caps.join(", "))
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" | ");
                    out.push(format!("{}    case {}: {};", pad, hdr, emit_expr(&ret_value, 0)));
                }
            }
        }
        out.push(format!("{}}}", pad));
        return;
    }
    if let Some(edef) = match_enum {
        let _ = edef;
        let disc = match_base.as_ref().map(|b| emit_expr(b, 0)).unwrap_or_default();
        out.push(format!("{}switch ({}) {{", pad, disc));
        if let Some(plan) = case_plan {
            for (ctors, body) in plan {
                match ctors {
                    None => out.push(format!("{}case _:", pad)),
                    Some(ctors) => {
                        let hdr = ctors
                            .iter()
                            .map(|c| {
                                if c.nparams == 0 {
                                    c.name.clone()
                                } else {
                                    let caps: Vec<String> = (0..c.nparams)
                                        .map(|i| format!("_capture{}_", i + 1))
                                        .collect();
                                    format!("{}({})", c.name, caps.join(", "))
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(" | ");
                        out.push(format!("{}case {}:", pad, hdr));
                    }
                }
                out.extend(render_tree(body, indent + 1, abc, field_types, enums));
            }
        }
        out.push(format!("{}}}", pad));
        return;
    }
    out.push(format!("{}switch ({}) {{", pad, emit_expr(value, 0)));
    for (vals, body) in cases {
        if vals.is_empty() {
            out.push(format!("{}default:", pad));
        } else {
            let vals_str = vals.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ");
            out.push(format!("{}case {}:", pad, vals_str));
        }
        out.extend(render_tree(body, indent + 1, abc, field_types, enums));
    }
    out.push(format!("{}}}", pad));
}

fn ret_node_matches_none(_v: &IrNode) -> bool {
    false
}

fn render_closure_decl(
    name: &str,
    nf: &IrNode,
    indent: usize,
    abc: &AbcFile,
    field_types: &std::collections::HashMap<String, String>,
    enums: &std::collections::HashMap<usize, crate::haxe_layer::EnumDef>,
) -> Vec<String> {
    render_closure_impl(name, nf, indent, abc, field_types, enums, true)
}

fn render_closure_assign(
    name: &str,
    nf: &IrNode,
    indent: usize,
    abc: &AbcFile,
    field_types: &std::collections::HashMap<String, String>,
    enums: &std::collections::HashMap<usize, crate::haxe_layer::EnumDef>,
) -> Vec<String> {
    render_closure_impl(name, nf, indent, abc, field_types, enums, false)
}

fn closure_sig(nf: &IrNode, abc: &AbcFile) -> (String, Option<crate::abc::MethodBody>) {
    let IrNode::NewFunction { method_index, .. } = nf else {
        return ("function()".to_string(), None);
    };
    let m = &abc.methods[*method_index];
    let mut params = Vec::new();
    for (i, &pt) in m.params.iter().enumerate() {
        let pname = if !m.param_names.is_empty()
            && i < m.param_names.len()
            && m.param_names[i] > 0
            && ((m.param_names[i] - 1) as usize) < abc.strings.len()
        {
            abc.strings[(m.param_names[i] - 1) as usize].clone()
        } else {
            format!("_param{}_", i + 1)
        };
        let ty = crate::decompile::type_ref(abc, pt);
        params.push(if ty.is_empty() { pname } else { format!("{} : {}", pname, ty) });
    }
    let ret = crate::decompile::type_ref(abc, m.returns);
    let sig = format!("function({})", params.join(", "))
        + &if ret.is_empty() { String::new() } else { format!(" : {}", ret) };
    let body = m
        .body_idx
        .and_then(|bi| abc.bodies.get(bi))
        .cloned();
    (sig, body)
}

fn render_closure_impl(
    name: &str,
    nf: &IrNode,
    indent: usize,
    abc: &AbcFile,
    field_types: &std::collections::HashMap<String, String>,
    enums: &std::collections::HashMap<usize, crate::haxe_layer::EnumDef>,
    decl: bool,
) -> Vec<String> {
    let pad = "    ".repeat(indent);
    let (sig, body) = closure_sig(nf, abc);
    let kw = if decl { "var " } else { "" };
    match body {
        None => vec![format!("{}{}{} = {} {{ /* no body */ }};", pad, kw, name, sig)],
        Some(body) => {
            let mut out = vec![format!("{}{}{} = {} {{", pad, kw, name, sig)];
            out.extend(crate::decompile::emit_body(abc, &body, indent + 1, None, Some(field_types), Some(enums)));
            out.push(format!("{}}};", pad));
            out
        }
    }
}

fn closure_lhs(prop_set: &IrNode) -> String {
    let IrNode::PropSet { obj, prop, is_dynamic, .. } = prop_set else {
        return String::new();
    };
    let obj_s = emit_expr(obj, 16);
    let prop_s = emit_expr(prop, 0);
    if matches!(**obj, IrNode::ImplicitThis) {
        return prop_s;
    }
    if *is_dynamic || matches!(&**prop, IrNode::StringConst(_)) || is_computed_key(prop) {
        format!("{}[{}]", obj_s, prop_s)
    } else {
        format!("{}.{}", obj_s, prop_s)
    }
}

fn render_closure_propset(
    ps: &IrNode,
    nf: &IrNode,
    indent: usize,
    abc: &AbcFile,
    field_types: &std::collections::HashMap<String, String>,
    enums: &std::collections::HashMap<usize, crate::haxe_layer::EnumDef>,
) -> Vec<String> {
    let pad = "    ".repeat(indent);
    let lhs = closure_lhs(ps);
    let (sig, body) = closure_sig(nf, abc);
    match body {
        None => vec![format!("{}{} = {} {{ /* no body */ }};", pad, lhs, sig)],
        Some(body) => {
            let mut out = vec![format!("{}{} = {} {{", pad, lhs, sig)];
            out.extend(crate::decompile::emit_body(abc, &body, indent + 1, None, Some(field_types), Some(enums)));
            out.push(format!("{}}};", pad));
            out
        }
    }
}

pub fn seq_empty(n: &SNode) -> bool {
    is_empty_seq(n)
}

pub fn emit_linear_lines(nodes: &[(usize, IrNode)], indent: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for (_pc, n) in nodes {
        if is_noise_ir(n) {
            continue;
        }
        for l in emit_stmt(n, indent) {
            if !l.trim().is_empty() {
                lines.push(l);
            }
        }
    }
    lines
}

fn is_noise_ir(n: &IrNode) -> bool {
    crate::control_flow::is_noise(n)
}

pub fn is_add_str_concat(_left: &IrNode, _right: &IrNode) -> bool {
    true
}
