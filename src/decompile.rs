use std::collections::{HashMap, HashSet};

use crate::abc::{AbcFile, ClassInfo, Trait, TraitKind, ValueKind};
use crate::control_flow::{is_noise, seq_of, structure_method, SNode};
use crate::haxe_layer::{
    apply_matches, apply_register_naming, apply_synthetic_names, collapse_map_ops,
    detect_enums, fingerprint_abc, insert_var_decls, render_enum, EnumDef,
};
use crate::haxe_out::{
    emit_stmt, haxe_mode, normalize_class_name, render_tree, set_haxe_mode,
};
use crate::ir::{peel, peel_implicit, IrNode};
use crate::translate::{build_blocks, StackSimulator};

fn is_noise_node(n: &IrNode) -> bool {
    is_noise(n)
}

pub fn render_default_value(abc: &AbcFile, val: &crate::abc::Value) -> String {
    let k = val.kind;
    if matches!(
        k,
        ValueKind::Undefined | ValueKind::Null
    ) {
        return "null".to_string();
    }
    if k == ValueKind::VTrue {
        return "true".to_string();
    }
    if k == ValueKind::VFalse {
        return "false".to_string();
    }
    if k == ValueKind::String {
        let s = if val.data > 0 && ((val.data - 1) as usize) < abc.strings.len() {
            abc.strings[(val.data - 1) as usize].clone()
        } else {
            String::new()
        };
        return crate::haxe_out::haxe_str(&s);
    }
    if k == ValueKind::Int {
        let v = if val.data > 0 && ((val.data - 1) as usize) < abc.ints.len() {
            abc.ints[(val.data - 1) as usize]
        } else {
            0
        };
        return v.to_string();
    }
    if k == ValueKind::UInt {
        let v = if val.data > 0 && ((val.data - 1) as usize) < abc.uints.len() {
            abc.uints[(val.data - 1) as usize]
        } else {
            0
        };
        return v.to_string();
    }
    if k == ValueKind::Double {
        let v = if val.data > 0 && ((val.data - 1) as usize) < abc.doubles.len() {
            abc.doubles[(val.data - 1) as usize]
        } else {
            0.0
        };
        return crate::haxe_out::py_repr_f64(v);
    }
    if matches!(
        k,
        ValueKind::Namespace
            | ValueKind::PackageNs
            | ValueKind::PackageInternalNs
            | ValueKind::ProtectedNs
            | ValueKind::ExplicitNs
    ) {
        if val.data > 0 && ((val.data - 1) as usize) < abc.namespaces.len() {
            let ns = &abc.namespaces[(val.data - 1) as usize];
            let resolved = ns.resolve(abc);
            return format!("\"{}\"", resolved);
        }
        return "null".to_string();
    }
    "null".to_string()
}

pub fn emit_linear_body(abc: &AbcFile, body: &crate::abc::MethodBody, indent: usize) -> Vec<String> {
    let mut sim = StackSimulator::new(abc, body);
    let (_blocks, ordered) = build_blocks(&mut sim);
    let mut lines: Vec<String> = Vec::new();
    for bb in &ordered {
        for (_pc, n) in &bb.instrs {
            if is_noise_node(n) {
                continue;
            }
            for l in emit_stmt(n, indent) {
                if !l.trim().is_empty() {
                    lines.push(l);
                }
            }
        }
    }
    lines
}

fn pure_def_value(v: &IrNode) -> bool {
    let mut v = peel(v);
    loop {
        match v {
            IrNode::Cast { expr, .. } | IrNode::Convert { expr, .. } => v = *expr,
            other => {
                v = other;
                break;
            }
        }
    }
    match v {
        IrNode::RegAccess { .. }
        | IrNode::NameRef { .. }
        | IrNode::ImplicitThis
        | IrNode::This
        | IrNode::NullConst
        | IrNode::BoolConst(_)
        | IrNode::IntConst(_)
        | IrNode::UIntConst(_)
        | IrNode::DoubleConst(_)
        | IrNode::StringConst(_)
        | IrNode::UndefinedConst
        | IrNode::NaNConst => true,
        IrNode::PropGet { obj, .. } => pure_def_value(&obj),
        _ => false,
    }
}

fn peel_wrappers(v: &IrNode) -> IrNode {
    let mut cur = v.clone();
    loop {
        match cur {
            IrNode::Cast { expr, .. } | IrNode::Convert { expr, .. } => cur = *expr,
            other => return other,
        }
    }
}

fn mark_return_switch(tree: &mut SNode) {
    let is_seq = matches!(tree, SNode::Seq(_));
    if !is_seq {
        if !matches!(tree, SNode::Switch { .. }) {
            return;
        }
        mark_switch(tree);
        return;
    }
    let SNode::Seq(stmts) = tree else { return };
    if stmts.is_empty() {
        return;
    }
    let last_is_switch = matches!(stmts.last(), Some(SNode::Switch { .. }));
    if !last_is_switch {
        return;
    }
    let lead_count = stmts.len() - 1;
    for s in &stmts[..lead_count] {
        let ok = match s {
            SNode::Expr(IrNode::RegSet { value, .. }) => pure_def_value(value),
            _ => false,
        };
        if !ok {
            return;
        }
    }
    let SNode::Switch {
        cases,
        case_plan,
        match_enum: _,
        match_base: _,
        value: _,
        ..
    } = stmts.last_mut().unwrap()
    else {
        return;
    };
    let entries: Vec<(Option<&Vec<EnumCtorLike>>, &SNode)> = match case_plan {
        Some(plan) => plan.iter().map(|(c, b)| (c.as_ref(), b)).collect(),
        None => cases.iter().map(|(_vals, b)| (None, b)).collect(),
    };
    if entries.is_empty() {
        return;
    }
    let mut value_cases = 0usize;
    let mut case_reads: HashSet<usize> = HashSet::new();
    for (ctors, body) in &entries {
        let body_stmts = seq_of(body);
        if body_stmts.len() != 1 || !matches!(&body_stmts[0], SNode::Return(_)) {
            return;
        }
        if let Some(ctors) = ctors {
            if !ctors.is_empty() {
                value_cases += 1;
            }
        }
        if let SNode::Return(Some(v)) = &body_stmts[0] {
            crate::ir::collect_regs(Some(v), &mut case_reads);
        }
    }
    if value_cases == 0 {
        return;
    }
    let mut dropped: HashSet<usize> = HashSet::new();
    for (i, s) in stmts[..lead_count].iter().enumerate() {
        if let SNode::Expr(IrNode::RegSet { reg, value, .. }) = s {
            let mut reads = HashSet::new();
            crate::ir::collect_regs(Some(&peel_wrappers(value)), &mut reads);
            let mut others = case_reads.clone();
            for (j, s2) in stmts[..lead_count].iter().enumerate() {
                if j == i {
                    continue;
                }
                if let SNode::Expr(IrNode::RegSet { value: v2, .. }) = s2 {
                    let mut r2 = HashSet::new();
                    crate::ir::collect_regs(Some(&peel_wrappers(v2)), &mut r2);
                    others.extend(r2);
                }
            }
            if !others.contains(reg) {
                dropped.insert(i);
            }
        }
    }
    let disc = {
        let sw = stmts.last().unwrap();
        match sw {
            SNode::Switch {
                match_enum,
                match_base,
                value,
                ..
            } => {
                if match_enum.is_some() {
                    match_base.as_ref().cloned().unwrap_or_else(|| value.clone())
                } else {
                    (*value).clone()
                }
            }
            _ => return,
        }
    };
    let mut disc = peel_wrappers(&disc);
    if let IrNode::RegAccess { reg: _disc_reg, .. } = &disc {
        if !stmts[..lead_count].is_empty() {
            let defs: HashMap<usize, IrNode> = stmts[..lead_count]
                .iter()
                .enumerate()
                .filter(|(i, _s)| dropped.contains(i))
                .filter_map(|(_i, s)| match s {
                    SNode::Expr(IrNode::RegSet { reg, value, .. }) => Some((*reg, (**value).clone())),
                    _ => None,
                })
                .collect();
            loop {
                let IrNode::RegAccess { reg: r, .. } = &disc else { break };
                let Some(d) = defs.get(r) else { break };
                let v = peel_wrappers(d);
                if !(pure_def_value(&v) || matches!(v, IrNode::RegAccess { .. })) {
                    break;
                }
                disc = v;
            }
        }
    }
    if !matches!(disc, IrNode::RegAccess { .. }) {
        if let SNode::Switch {
            match_enum,
            match_base,
            value,
            ..
        } = stmts.last_mut().unwrap()
        {
            if match_enum.is_some() {
                *match_base = Some(disc);
            } else {
                *value = disc;
            }
        }
    }
    let mut kept: Vec<SNode> = Vec::new();
    for (i, s) in std::mem::take(stmts).into_iter().enumerate() {
        if dropped.contains(&i) {
            continue;
        }
        kept.push(s);
    }
    *stmts = kept;
    if let Some(SNode::Switch { expr_return, .. }) = stmts.last_mut() {
        *expr_return = true;
    }
}

type EnumCtorLike = crate::haxe_layer::EnumCtor;

fn mark_switch(_sw: &mut SNode) {
}

pub fn emit_body(
    abc: &AbcFile,
    body: &crate::abc::MethodBody,
    indent: usize,
    param_types: Option<&HashMap<usize, String>>,
    field_types: Option<&HashMap<String, String>>,
    enums: Option<&HashMap<usize, EnumDef>>,
) -> Vec<String> {
    let empty_ft: HashMap<String, String> = HashMap::new();
    let empty_en: HashMap<usize, EnumDef> = HashMap::new();
    let field_types = field_types.unwrap_or(&empty_ft);
    let enums = enums.unwrap_or(&empty_en);
    let m = abc.methods.get(body.method_idx);
    let mut lines: Vec<String>;

    let mut tree = structure_method(abc, body);
    if haxe_mode() {
        collapse_map_ops(&mut tree);
        let has_types = param_types.map(|p| !p.is_empty()).unwrap_or(false)
            || !field_types.is_empty()
            || !enums.is_empty();
        if has_types {
            let empty_pt: HashMap<usize, String> = HashMap::new();
            apply_matches(
                &mut tree,
                abc,
                param_types.unwrap_or(&empty_pt),
                field_types,
                enums,
            );
        }
        apply_register_naming(&mut tree, abc, m, body);
        apply_synthetic_names(&mut tree);
    }
    mark_return_switch(&mut tree);
    if haxe_mode() {
        let n_params = m.map(|mm| mm.params.len()).unwrap_or(0);
        tree = insert_var_decls(tree, n_params);
    }
    lines = render_tree(&tree, indent, Some(abc), field_types, enums);

    if lines.is_empty() && !body.instructions.is_empty() {
        let linear = emit_linear_body(abc, body, indent);
        if !linear.is_empty() {
            lines.push(format!(
                "{}// (structurer produced no output — linear fallback)",
                "    ".repeat(indent)
            ));
            lines.extend(linear);
        }
    }
    while lines.last().map(|l| l.trim() == "return;").unwrap_or(false) {
        lines.pop();
    }
    lines
}

pub fn type_ref(abc: &AbcFile, idx: u32) -> String {
    if idx == 0 {
        return String::new();
    }
    let Some(mn) = abc.multinames.get((idx - 1) as usize) else {
        return String::new();
    };
    let name = mn.resolve(abc);
    let builtin: &[(&str, &str)] = &[
        ("int", "Int"), ("uint", "UInt"), ("Number", "Float"),
        ("Boolean", "Bool"), ("void", "Void"), ("String", "String"),
        ("Object", "Dynamic"), ("Array", "Array<Dynamic>"),
        ("Vector", "Array<Dynamic>"), ("Function", "Dynamic"),
        ("Class", "Class"), ("*", ""), ("XML", "Xml"), ("XMLList", "Xml"),
    ];
    let short = name.rsplit("::").next().unwrap_or("").to_string();
    const MAP_IMPLS: &[&str] = &[
        "StringMap", "IntMap", "ObjectMap", "HashMap",
        "UnsafeStringMap", "EnumValueMap", "IMap", "Map",
    ];
    if MAP_IMPLS.contains(&short.as_str()) {
        return "Map<Dynamic, Dynamic>".to_string();
    }
    for (k, v) in builtin {
        if short == *k {
            return v.to_string();
        }
    }
    let full = name.replace("::", ".");
    let last = full.rsplit('.').next().unwrap_or("");
    if ["StringMap", "IntMap", "ObjectMap", "HashMap", "UnsafeStringMap", "EnumValueMap"]
        .contains(&last)
    {
        return "Map<Dynamic, Dynamic>".to_string();
    }
    if full.starts_with('[') {
        if let Some(end) = full.find(']') {
            let pkg = &full[1..end];
            let base = full.get(end + 2..).unwrap_or("");
            if !base.is_empty() {
                return if pkg.is_empty() { base.to_string() } else { format!("{}.{}", pkg, base) };
            }
        }
    }
    if full == "haxe.UInt32" {
        return "UInt".to_string();
    }
    full
}

pub fn is_setter_getter(name: &str) -> bool {
    name.starts_with("get_") || name.starts_with("set_")
}

pub fn field_name(name: &str) -> String {
    if name.starts_with("get_") || name.starts_with("set_") {
        name[4..].to_string()
    } else {
        name.to_string()
    }
}

fn trait_vis(abc: &AbcFile, t: &Trait) -> String {
    if t.name_idx > 0 {
        if let Some(mn) = abc.multinames.get((t.name_idx - 1) as usize) {
            if mn.ns_idx > 0 && ((mn.ns_idx - 1) as usize) < abc.namespaces.len() {
                let ns = &abc.namespaces[(mn.ns_idx - 1) as usize];
                if matches!(
                    ns.kind,
                    crate::abc::NsKind::Private
                        | crate::abc::NsKind::Protected
                        | crate::abc::NsKind::StaticProtected
                ) {
                    return "private ".to_string();
                }
            }
        }
    }
    "public ".to_string()
}

#[allow(clippy::too_many_arguments)]
pub fn emit_trait(
    abc: &AbcFile,
    t: &Trait,
    indent: usize,
    is_static: bool,
    class_info: &ClassInfo,
    body_lines: &mut Vec<String>,
    field_types: &HashMap<String, String>,
    enums: &HashMap<usize, EnumDef>,
    static_inits: &HashMap<String, IrNode>,
) {
    let pad = "    ".repeat(indent);
    let name_mn = if t.name_idx > 0 {
        abc.multinames.get((t.name_idx - 1) as usize)
    } else {
        None
    };
    let name = name_mn
        .map(|mn| mn.resolve(abc))
        .unwrap_or_else(|| format!("slot{}", t.slot_id));
    let short = field_name(name.rsplit("::").next().unwrap_or(""));

    match t.kind {
        TraitKind::Slot | TraitKind::Const => {
            let kw = if t.kind == TraitKind::Const { "const" } else { "var" };
            let ty = type_ref(abc, t.type_idx);
            let mut val = String::new();
            let has_real_default = t
                .value
                .as_ref()
                .map(|v| {
                    !matches!(v.kind, ValueKind::Undefined | ValueKind::Null)
                })
                .unwrap_or(false);
            if has_real_default {
                if let Some(v) = &t.value {
                    val = format!(" = {}", render_default_value(abc, v));
                }
            }
            if val.is_empty() && is_static && !static_inits.is_empty() {
                if let Some(init) = static_inits.get(&short) {
                    val = format!(" = {}", crate::haxe_out::emit_expr(init, 0));
                }
            }
            let vis = trait_vis(abc, t);
            let mut line = format!(
                "{}{}{}{} {}",
                pad,
                vis,
                if is_static { "static " } else { "" },
                kw,
                short
            );
            if !ty.is_empty() {
                line += &format!(" : {}", ty);
            }
            line += &val;
            line.push(';');
            body_lines.push(line);
        }
        TraitKind::Class => {
        }
        TraitKind::Function => {
            let vis = trait_vis(abc, t);
            body_lines.push(format!(
                "{}{}{}var {} : Function;  // function slot",
                pad,
                vis,
                if is_static { "static " } else { "" },
                short
            ));
        }
        TraitKind::Method | TraitKind::Getter | TraitKind::Setter => {
            let midx = t.method_idx;
            let Some(m) = abc.methods.get(midx) else { return };
            let is_getter = t.kind == TraitKind::Getter;
            let is_setter = t.kind == TraitKind::Setter;
            let mut params: Vec<String> = Vec::new();
            for (i, &pt) in m.params.iter().enumerate() {
                let mut pname = format!("_param{}_", i + 1);
                if !m.param_names.is_empty()
                    && i < m.param_names.len()
                    && m.param_names[i] > 0
                    && ((m.param_names[i] - 1) as usize) < abc.strings.len()
                {
                    pname = abc.strings[(m.param_names[i] - 1) as usize].clone();
                }
                let ty = type_ref(abc, pt);
                let mut ps = pname;
                if !ty.is_empty() {
                    ps += &format!(" : {}", ty);
                }
                if !m.optional_params.is_empty()
                    && i + m.optional_params.len() >= m.params.len()
                {
                    let di = i + m.optional_params.len() - m.params.len();
                    if let Some(dv) = m.optional_params.get(di) {
                        ps += &format!(" = {}", render_default_value(abc, dv));
                    }
                }
                params.push(ps);
            }
            if m.var_args() {
                params.push("args:Rest<Dynamic>".to_string());
            }
            let ret = type_ref(abc, m.returns);
            let mut access = if class_info.interface() {
                String::new()
            } else {
                trait_vis(abc, t)
            };
            if (t.attrs & 0x10) != 0 && !is_static {
                access += "final ";
            }
            if (t.attrs & 0x20) != 0 {
                access += "override ";
            }
            let fn_kw = if is_getter {
                "function get"
            } else if is_setter {
                "function set"
            } else {
                "function"
            };
            let mut decl = format!(
                "{}{}{}{} {}({})",
                pad,
                access,
                if is_static { "static " } else { "" },
                fn_kw,
                short,
                params.join(", ")
            );
            if !ret.is_empty() {
                decl += &format!(" : {}", ret);
            }
            if class_info.interface() {
                body_lines.push(format!("{};", decl));
                return;
            }
            body_lines.push(format!("{} {{", decl));
            if let Some(bi) = m.body_idx {
                if let Some(body) = abc.bodies.get(bi) {
                    let mut ptypes: HashMap<usize, String> = HashMap::new();
                    for (i, &pt) in m.params.iter().enumerate() {
                        let ty = type_ref(abc, pt);
                        if !ty.is_empty() {
                            ptypes.insert(i + 1, ty);
                        }
                    }
                    body_lines.extend(emit_body(
                        abc,
                        body,
                        indent + 1,
                        Some(&ptypes),
                        Some(field_types),
                        Some(enums),
                    ));
                }
            }
            body_lines.push(format!("{}}}", pad));
            body_lines.push(String::new());
        }
        _ => {}
    }
}

fn static_initializers(
    all: &HashMap<String, HashMap<String, IrNode>>,
    abc: &AbcFile,
    c: &ClassInfo,
) -> HashMap<String, IrNode> {
    let full = normalize_class_name(abc, c);
    let short = full.rsplit('.').next().unwrap_or("");
    for key in [short, full.as_str()] {
        if let Some(m) = all.get(key) {
            return m.clone();
        }
    }
    HashMap::new()
}

fn collect_script_static_inits(abc: &AbcFile) -> HashMap<String, HashMap<String, IrNode>> {
    let mut result: HashMap<String, HashMap<String, IrNode>> = HashMap::new();
    if !haxe_mode() {
        return result;
    }

    fn pure(v: &IrNode) -> bool {
        let v = peel_implicit(v);
        match &v {
            IrNode::IntConst(_)
            | IrNode::UIntConst(_)
            | IrNode::DoubleConst(_)
            | IrNode::BoolConst(_)
            | IrNode::StringConst(_)
            | IrNode::NullConst => true,
            IrNode::UnaryOp { op, expr } if op == "neg" || op == "not" || op == "bitnot" => pure(expr),
            IrNode::NewArray(items) => items.iter().all(pure),
            IrNode::NewObject(props) => props.iter().all(|(_k, x)| pure(x)),
            _ => false,
        }
    }

    for sc in &abc.scripts {
        let Some(m) = abc.methods.get(sc.init_method_idx) else { continue };
        let Some(bi) = m.body_idx else { continue };
        let Some(body) = abc.bodies.get(bi) else { continue };
        if body.code.is_empty() {
            continue;
        }
        let tree = structure_method(abc, body);
        let stmts = seq_of(&tree);
        for s in stmts {
            let mut node = match s {
                SNode::Expr(n) => n,
                _ => continue,
            };
            if let IrNode::ExprStmt(e) = node {
                node = *e;
            }
            let IrNode::PropSet { obj, prop, value, .. } = node else { continue };
            let IrNode::NameRef { name: prop_name, .. } = &*prop else { continue };
            let recv = peel_implicit(&obj);
            let IrNode::NameRef { name: recv_name, .. } = recv else { continue };
            let cls = recv_name.rsplit("::").next().unwrap_or("").to_string();
            if cls.starts_with("__") {
                continue;
            }
            let field = prop_name.rsplit("::").next().unwrap_or("").to_string();
            if field == "__constructs__" || field.starts_with("__") {
                continue;
            }
            if !pure(&value) {
                continue;
            }
            result.entry(cls).or_default().insert(field, (*value).clone());
        }
    }
    result
}

pub fn decompile_class(
    abc: &AbcFile,
    c: &ClassInfo,
    enums: &HashMap<usize, EnumDef>,
    script_inits: &HashMap<String, HashMap<String, IrNode>>,
) -> String {
    let mut out: Vec<String> = Vec::new();
    let is_interface = c.interface();
    let mut ftypes: HashMap<String, String> = HashMap::new();
    for t in c.traits.iter().chain(c.static_traits.iter()) {
        if matches!(t.kind, TraitKind::Slot | TraitKind::Const) && t.name_idx > 0 {
            if let Some(mn) = abc.multinames.get((t.name_idx - 1) as usize) {
                let nm = mn.resolve(abc).rsplit("::").next().unwrap_or("").to_string();
                let ty = type_ref(abc, t.type_idx);
                if !nm.is_empty() && !ty.is_empty() {
                    ftypes.insert(nm, ty);
                }
            }
        }
    }
    let mut access = String::new();
    if c.final_class() {
        access += "final ";
    }
    access += if is_interface { "interface " } else { "class " };
    let name = if c.name_idx > 0 {
        abc.multinames
            .get((c.name_idx - 1) as usize)
            .map(|mn| mn.resolve(abc).rsplit("::").next().unwrap_or("?").to_string())
            .unwrap_or_else(|| "?".to_string())
    } else {
        "?".to_string()
    };
    let super_s = if c.super_idx > 0 { type_ref(abc, c.super_idx) } else { String::new() };
    let mut impls: Vec<String> = c
        .interfaces
        .iter()
        .map(|&i| type_ref(abc, i))
        .filter(|x| !x.is_empty() && x != "Dynamic")
        .collect();
    impls.dedup();
    let mut decl = format!("{}{}{}", "", access, name);
    if !super_s.is_empty() && !is_interface && super_s != "Dynamic" {
        decl += &format!(" extends {}", super_s);
    }
    if !impls.is_empty() {
        decl += &format!(" implements {}", impls.join(", "));
    }
    out.push(format!("{} {{", decl));
    for t in &c.traits {
        emit_trait(abc, t, 1, false, c, &mut out, &ftypes, enums, &HashMap::new());
    }
    let static_inits = static_initializers(script_inits, abc, c);
    for t in &c.static_traits {
        emit_trait(abc, t, 1, true, c, &mut out, &ftypes, enums, &static_inits);
    }
    if !is_interface {
        if let Some(m) = abc.methods.get(c.constructor_idx) {
            let mut params: Vec<String> = Vec::new();
            for (i, &pt) in m.params.iter().enumerate() {
                let mut pname = format!("_param{}_", i + 1);
                if !m.param_names.is_empty()
                    && i < m.param_names.len()
                    && m.param_names[i] > 0
                    && ((m.param_names[i] - 1) as usize) < abc.strings.len()
                {
                    pname = abc.strings[(m.param_names[i] - 1) as usize].clone();
                }
                let ty = type_ref(abc, pt);
                params.push(if ty.is_empty() { pname } else { format!("{} : {}", pname, ty) });
            }
            out.push(format!("    public function new({}) {{", params.join(", ")));
            if let Some(bi) = m.body_idx {
                if let Some(body) = abc.bodies.get(bi) {
                    let mut ptypes: HashMap<usize, String> = HashMap::new();
                    for (i, &pt) in m.params.iter().enumerate() {
                        let ty = type_ref(abc, pt);
                        if !ty.is_empty() {
                            ptypes.insert(i + 1, ty);
                        }
                    }
                    out.extend(emit_body(abc, body, 2, Some(&ptypes), Some(&ftypes), Some(enums)));
                }
            }
            out.push("    }".to_string());
            out.push(String::new());
        }
    }
    out.push("}".to_string());
    out.join("\n")
}

pub fn decompile_abc(abc: &AbcFile, symbols: Option<&std::collections::HashMap<String, u16>>) -> String {
    let mut out: Vec<String> = vec!["// Decompiled with haxe-decompiler (experimental)".to_string()];
    let (is_haxe, score, markers) = fingerprint_abc(abc);
    set_haxe_mode(is_haxe);
    let markers_repr = format!(
        "[{}]",
        markers
            .iter()
            .map(|m| format!("'{}'", m))
            .collect::<Vec<_>>()
            .join(", ")
    );
    out.push(format!(
        "// Haxe fingerprint: score={} markers={} -> {}",
        score,
        markers_repr,
        if is_haxe { "Haxe-generated" } else { "not Haxe (raw AVM2)" }
    ));
    if let Some(symbols) = symbols {
        if !symbols.is_empty() {
            let sym_items: Vec<String> = symbols
                .iter()
                .take(8)
                .map(|(name, tid)| format!("{} -> tag#{}", name, tid))
                .collect();
            out.push(format!("// Symbols: {}", sym_items.join(", ")));
        }
    }
    for (kind, _full_name, body) in iter_units(abc) {
        match kind {
            "skip" => out.push(format!(
                "// (Haxe/Flash runtime class: {} — skipped)",
                _full_name
            )),
            _ => {
                if let Some(body) = body {
                    out.push(String::new());
                    out.push(body);
                    out.push(String::new());
                }
            }
        }
    }
    set_haxe_mode(true);
    out.join("\n")
}

pub fn prepare(abc: &AbcFile) -> (bool, i32, Vec<String>, HashMap<usize, EnumDef>) {
    let (is_haxe, score, markers) = fingerprint_abc(abc);
    set_haxe_mode(is_haxe);
    let mut enums = HashMap::new();
    if is_haxe {
        enums = detect_enums(abc);
    }
    (is_haxe, score, markers, enums)
}

pub fn iter_units(abc: &AbcFile) -> Vec<(&'static str, String, Option<String>)> {
    let (_is_haxe, _score, _markers, enums) = prepare(abc);
    let script_inits = collect_script_static_inits(abc);
    let mut out = Vec::new();
    for (ci, c) in abc.classes.iter().enumerate() {
        let full_name = normalize_class_name(abc, c);
        if crate::haxe_out::is_runtime_full_name(&full_name) {
            out.push(("skip", full_name, None));
            continue;
        }
        if let Some(edef) = enums.get(&ci) {
            out.push(("enum", full_name, Some(render_enum(edef))));
        } else {
            out.push(("class", full_name, Some(decompile_class(abc, c, &enums, &script_inits))));
        }
    }
    out
}

const IMPORT_EXEMPT: &[&str] = &[
    "Std", "Math", "String", "Array", "Int", "UInt", "Float", "Bool",
    "Dynamic", "Void", "Class", "Function", "EnumValue", "Xml",
];

fn type_index(abc: &AbcFile) -> HashMap<String, String> {
    let mut idx: HashMap<String, String> = HashMap::new();
    for c in &abc.classes {
        let full = normalize_class_name(abc, c);
        let short = full.rsplit('.').next().unwrap_or("").to_string();
        if !short.is_empty() {
            idx.entry(short).or_insert(full);
        }
    }
    for mn in &abc.multinames {
        let nm = mn.resolve(abc);
        if nm.is_empty() || !nm.contains("::") {
            continue;
        }
        let (pkg, short) = match nm.rfind("::") {
            Some(i) => (&nm[..i], &nm[i + 2..]),
            None => continue,
        };
        if short.is_empty() || pkg.starts_with('[') {
            continue;
        }
        let full = if pkg.is_empty() { short.to_string() } else { format!("{}.{}", pkg, short) };
        idx.entry(short.to_string()).or_insert(full);
    }
    idx
}

fn imports_for_unit(body: &str, full_name: &str, tindex: &HashMap<String, String>) -> Vec<String> {
    let (pkg, short) = match full_name.rfind('.') {
        Some(i) => (&full_name[..i], &full_name[i + 1..]),
        None => ("", full_name),
    };
    let mut imports: HashSet<String> = HashSet::new();
    let mut tokens: HashSet<String> = HashSet::new();
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_uppercase() || (c == '_' && i + 1 < chars.len() && chars[i + 1].is_ascii_uppercase()) {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let prev_ok = start == 0 || !(chars[start - 1].is_alphanumeric() || chars[start - 1] == '_');
            if prev_ok {
                tokens.insert(chars[start..i].iter().collect());
            }
        } else {
            i += 1;
        }
    }
    for token in tokens {
        if token == short || IMPORT_EXEMPT.contains(&token.as_str()) {
            continue;
        }
        let Some(full2) = tindex.get(&token) else { continue };
        if full2 == full_name {
            continue;
        }
        let pkg2 = full2.rsplit('.').next().map(|_| {
            match full2.rfind('.') {
                Some(i) => &full2[..i],
                None => "",
            }
        });
        if pkg2 != Some(pkg) {
            imports.insert(full2.clone());
        }
    }
    let mut v: Vec<String> = imports.into_iter().collect();
    v.sort();
    v
}

pub fn write_project(
    abc: &AbcFile,
    out_dir: &str,
    entry_class: Option<&str>,
    swf_version: Option<u8>,
) -> std::io::Result<Vec<String>> {
    let (_is_haxe, _score, _markers, _enums) = prepare(abc);
    let _script_inits = collect_script_static_inits(abc);
    let tindex = type_index(abc);
    let src_root = std::path::Path::new(out_dir).join("src");
    let mut written: Vec<String> = Vec::new();
    for (kind, full_name, body) in iter_units(abc) {
        if kind == "skip" {
            continue;
        }
        let Some(body) = body else { continue };
        let (pkg, short) = match full_name.rfind('.') {
            Some(i) => (&full_name[..i], &full_name[i + 1..]),
            None => ("", full_name.as_str()),
        };
        let d = if pkg.is_empty() {
            src_root.clone()
        } else {
            src_root.join(pkg.split('.').collect::<std::path::PathBuf>())
        };
        std::fs::create_dir_all(&d)?;
        let imports = imports_for_unit(&body, &full_name, &tindex);
        let mut head: Vec<String> = vec![format!(
            "// {} {} — decompiled with haxe-decompiler (experimental)",
            kind, full_name
        )];
        if !pkg.is_empty() {
            head.push(format!("package {};", pkg));
        }
        if !imports.is_empty() {
            head.push(String::new());
            for imp in &imports {
                head.push(format!("import {};", imp));
            }
        }
        let content = format!("{}\n\n{}\n", head.join("\n"), body);
        let path = d.join(format!("{}.hx", short));
        std::fs::write(&path, content)?;
        written.push(path.to_string_lossy().into_owned());
    }
    if let Some(entry) = entry_class {
        let mut hxml = vec!["-cp src".to_string(), format!("-main {}", entry)];
        if let Some(v) = swf_version {
            hxml.push(format!("-swf-version {}", v.min(41)));
        }
        hxml.push("-swf rebuilt.swf".to_string());
        let path = std::path::Path::new(out_dir).join("build.hxml");
        std::fs::write(&path, hxml.join("\n") + "\n")?;
        written.push(path.to_string_lossy().into_owned());
    }
    Ok(written)
}

fn is_regset(s: &SNode) -> bool {
    matches!(s, SNode::Expr(IrNode::RegSet { .. }))
}

fn _stack_eq_alias() {}
