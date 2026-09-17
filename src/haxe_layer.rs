use std::collections::{HashMap, HashSet};

use crate::abc::{AbcFile, TraitKind};
use crate::control_flow::{seq_of, SNode};
use crate::ir::{peel_cast, substitute_reg, IrNode};
use crate::translate::{build_blocks, StackSimulator};

#[derive(Debug, Clone, PartialEq)]
pub struct EnumCtor {
    pub name: String,
    pub index: i32,
    pub nparams: usize,
    pub types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef {
    pub name: String,
    pub class_index: usize,
    pub ctors: Vec<EnumCtor>,
}

impl EnumDef {
    pub fn by_index(&self, i: i32) -> Option<&EnumCtor> {
        self.ctors.iter().find(|c| c.index == i)
    }
}

fn iter_nodes(abc: &AbcFile, body: &crate::abc::MethodBody) -> Vec<IrNode> {
    let mut sim = StackSimulator::new(abc, body);
    let (_blocks, ordered) = build_blocks(&mut sim);
    let mut out = Vec::new();
    for bb in &ordered {
        for (_pc, n) in &bb.instrs {
            out.push(n.clone());
        }
    }
    out
}

fn cls_short(cls_node: &IrNode) -> String {
    match cls_node {
        IrNode::NameRef { name, .. } => name
            .rsplit("::")
            .next()
            .unwrap_or("")
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

fn is_enum_class(abc: &AbcFile, ci: usize) -> bool {
    let Some(c) = abc.classes.get(ci) else { return false };
    let mut names: HashSet<String> = HashSet::new();
    for t in c.traits.iter().chain(c.static_traits.iter()) {
        let nm = if t.name_idx > 0
            && ((t.name_idx - 1) as usize) < abc.multinames.len()
        {
            abc.multinames[(t.name_idx - 1) as usize]
                .resolve(abc)
                .rsplit("::")
                .next()
                .unwrap_or("")
                .to_string()
        } else {
            String::new()
        };
        names.insert(nm);
    }
    if !names.contains("__isenum") || !names.contains("__constructs__") {
        return false;
    }
    ["tag", "index", "params"].iter().all(|k| names.contains(*k))
}

fn ctor_from_construct(edef: &EnumDef, node: &IrNode) -> Option<EnumCtor> {
    let (cls, args) = match node {
        IrNode::ConstructProp { cls, args, .. } | IrNode::Construct { cls, args } => (cls, args),
        _ => return None,
    };
    if cls_short(cls) != edef.name {
        return None;
    }
    if args.len() != 3 {
        return None;
    }
    let (tag, index, params) = (&args[0], &args[1], &args[2]);
    let (tag, index) = match (tag, index) {
        (IrNode::StringConst(t), IrNode::IntConst(i)) => (t.clone(), *i),
        _ => return None,
    };
    match params {
        IrNode::NewArray(items) => Some(EnumCtor {
            name: tag,
            index,
            nparams: items.len(),
            types: Vec::new(),
        }),
        IrNode::NullConst => Some(EnumCtor {
            name: tag,
            index,
            nparams: 0,
            types: Vec::new(),
        }),
        _ => None,
    }
}

fn walk_nodes<'a>(n: &'a IrNode, out: &mut Vec<&'a IrNode>) {
    out.push(n);
    n.for_each_child(&mut |c| walk_nodes(c, out));
}

fn find_ctors(abc: &AbcFile, edef: &EnumDef, body: &crate::abc::MethodBody) -> Vec<EnumCtor> {
    let mut found = Vec::new();
    for top in iter_nodes(abc, body) {
        let mut all = Vec::new();
        walk_nodes(&top, &mut all);
        for n in all {
            if let Some(ctor) = ctor_from_construct(edef, n) {
                found.push(ctor);
            }
        }
    }
    found
}

fn script_init_for_class(abc: &AbcFile, class_index: usize) -> Option<crate::abc::MethodBody> {
    for s in &abc.scripts {
        for t in &s.traits {
            if t.kind == TraitKind::Class && t.class_idx == class_index {
                if let Some(m) = abc.methods.get(s.init_method_idx) {
                    if let Some(bi) = m.body_idx {
                        return abc.bodies.get(bi).cloned();
                    }
                }
            }
        }
    }
    let c = &abc.classes[class_index];
    let m = abc.methods.get(c.static_ctor_idx)?;
    let bi = m.body_idx?;
    abc.bodies.get(bi).cloned()
}

pub fn detect_enums(abc: &AbcFile) -> HashMap<usize, EnumDef> {
    let mut out: HashMap<usize, EnumDef> = HashMap::new();
    for (ci, c) in abc.classes.iter().enumerate() {
        if !is_enum_class(abc, ci) {
            continue;
        }
        let name = abc
            .multinames
            .get((c.name_idx - 1) as usize)
            .map(|mn| mn.resolve(abc).rsplit("::").next().unwrap_or("").to_string())
            .unwrap_or_default();
        let mut edef = EnumDef { name: name.clone(), class_index: ci, ctors: Vec::new() };
        for t in &c.static_traits {
            if t.kind != TraitKind::Method {
                continue;
            }
            let nm = if t.name_idx > 0 && ((t.name_idx - 1) as usize) < abc.multinames.len() {
                abc.multinames[(t.name_idx - 1) as usize]
                    .resolve(abc)
                    .rsplit("::")
                    .next()
                    .unwrap_or("")
                    .to_string()
            } else {
                String::new()
            };
            if nm.starts_with("__") {
                continue;
            }
            let Some(m) = abc.methods.get(t.method_idx) else { continue };
            let types: Vec<String> = m
                .params
                .iter()
                .map(|&p| type_ref_safe(abc, p))
                .collect();
            let Some(bi) = m.body_idx else { continue };
            let Some(body) = abc.bodies.get(bi) else { continue };
            for ctor in find_ctors(abc, &edef, body) {
                let mut ctor = ctor;
                ctor.types = types.clone();
                edef.ctors.push(ctor);
                break;
            }
        }
        if let Some(init_body) = script_init_for_class(abc, ci) {
            for top in iter_nodes(abc, &init_body) {
                let mut all = Vec::new();
                walk_nodes(&top, &mut all);
                for n in all {
                    if let IrNode::PropSet { value, .. } = n {
                        if matches!(**value, IrNode::ConstructProp { .. } | IrNode::Construct { .. }) {
                            if let Some(ctor) = ctor_from_construct(&edef, value) {
                                edef.ctors.push(ctor);
                            }
                        }
                    }
                }
            }
        }
        let mut seen: HashMap<i32, EnumCtor> = HashMap::new();
        for ct in edef.ctors {
            seen.entry(ct.index).or_insert(ct);
        }
        let mut idxs: Vec<i32> = seen.keys().copied().collect();
        idxs.sort();
        edef.ctors = idxs.into_iter().filter_map(|i| seen.remove(&i)).collect();
        if !edef.ctors.is_empty() {
            out.insert(ci, edef);
        }
    }
    out
}

pub fn type_ref_safe(abc: &AbcFile, idx: u32) -> String {
    if idx == 0 {
        return "Dynamic".to_string();
    }
    let t = crate::decompile::type_ref(abc, idx);
    if t.is_empty() {
        "Dynamic".to_string()
    } else {
        t
    }
}

pub fn render_enum(e: &EnumDef) -> String {
    let mut lines = vec![format!("enum {} {{", e.name)];
    for c in &e.ctors {
        if c.nparams > 0 {
            let params: Vec<String> = c
                .types
                .iter()
                .take(c.nparams)
                .enumerate()
                .map(|(i, t)| format!("_param{}_ : {}", i + 1, t))
                .collect();
            lines.push(format!("\t{}({});", c.name, params.join(", ")));
        } else {
            lines.push(format!("\t{};", c.name));
        }
    }
    lines.push("}".to_string());
    lines.join("\n")
}

pub struct MatchCtx {
    pub enums: HashMap<String, EnumDef>,
    pub regs: HashMap<usize, String>,
    pub fields: HashMap<String, String>,
}

impl MatchCtx {
    pub fn new(
        enums: HashMap<String, EnumDef>,
        param_types: &HashMap<usize, String>,
        field_types: &HashMap<String, String>,
    ) -> Self {
        MatchCtx {
            enums,
            regs: param_types.clone(),
            fields: field_types.clone(),
        }
    }

    pub fn type_of(&self, node: &IrNode) -> Option<String> {
        let node = peel_cast(node);
        match &node {
            IrNode::RegAccess { reg, .. } => self.regs.get(reg).cloned(),
            IrNode::PropGet { obj, prop, .. } => {
                if let IrNode::NameRef { name, .. } = &**prop {
                    if matches!(**obj, IrNode::ImplicitThis) {
                        return self.fields.get(name).cloned();
                    }
                    if let IrNode::RegAccess { reg, .. } = &**obj {
                        if let Some(t) = self.regs.get(reg) {
                            if self.enums.contains_key(t) && name == "index" {
                                return Some("Int".to_string());
                            }
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub fn is_enum(&self, node: &IrNode) -> Option<EnumDef> {
        self.type_of(node).and_then(|t| self.enums.get(&t).cloned())
    }
}

pub fn transform_expr<C, F>(node: IrNode, ctx: &mut C, f: &mut F) -> IrNode
where
    F: FnMut(&IrNode, &mut C) -> Option<IrNode>,
{
    if let Some(repl) = f(&node, ctx) {
        return repl;
    }
    match node {
        IrNode::UnaryOp { op, expr } => IrNode::UnaryOp {
            op,
            expr: Box::new(transform_expr(*expr, ctx, f)),
        },
        IrNode::BinaryOp { op, left, right } => IrNode::BinaryOp {
            op,
            left: Box::new(transform_expr(*left, ctx, f)),
            right: Box::new(transform_expr(*right, ctx, f)),
        },
        IrNode::Ternary { cond, then_val, else_val } => IrNode::Ternary {
            cond: Box::new(transform_expr(*cond, ctx, f)),
            then_val: Box::new(transform_expr(*then_val, ctx, f)),
            else_val: Box::new(transform_expr(*else_val, ctx, f)),
        },
        IrNode::Cast { expr, target_type, is_strict } => IrNode::Cast {
            expr: Box::new(transform_expr(*expr, ctx, f)),
            target_type: Box::new(transform_expr(*target_type, ctx, f)),
            is_strict,
        },
        IrNode::Convert { expr, target } => IrNode::Convert {
            expr: Box::new(transform_expr(*expr, ctx, f)),
            target,
        },
        IrNode::CheckXml(e) => IrNode::CheckXml(Box::new(transform_expr(*e, ctx, f))),
        IrNode::PropGet { obj, prop, is_dynamic } => IrNode::PropGet {
            obj: Box::new(transform_expr(*obj, ctx, f)),
            prop: Box::new(transform_expr(*prop, ctx, f)),
            is_dynamic,
        },
        IrNode::NextName { obj, prop, is_dynamic } => IrNode::NextName {
            obj: Box::new(transform_expr(*obj, ctx, f)),
            prop: Box::new(transform_expr(*prop, ctx, f)),
            is_dynamic,
        },
        IrNode::NextValue { obj, prop, is_dynamic } => IrNode::NextValue {
            obj: Box::new(transform_expr(*obj, ctx, f)),
            prop: Box::new(transform_expr(*prop, ctx, f)),
            is_dynamic,
        },
        IrNode::PropSet { obj, prop, value, is_init, is_dynamic } => IrNode::PropSet {
            obj: Box::new(transform_expr(*obj, ctx, f)),
            prop: Box::new(transform_expr(*prop, ctx, f)),
            value: Box::new(transform_expr(*value, ctx, f)),
            is_init,
            is_dynamic,
        },
        IrNode::PropDelete { obj, prop } => IrNode::PropDelete {
            obj: Box::new(transform_expr(*obj, ctx, f)),
            prop: Box::new(transform_expr(*prop, ctx, f)),
        },
        IrNode::Descendants { obj, name } => IrNode::Descendants {
            obj: Box::new(transform_expr(*obj, ctx, f)),
            name: Box::new(transform_expr(*name, ctx, f)),
        },
        IrNode::SlotGet { obj, slot } => IrNode::SlotGet {
            obj: Box::new(transform_expr(*obj, ctx, f)),
            slot,
        },
        IrNode::SlotSet { obj, slot, value } => IrNode::SlotSet {
            obj: Box::new(transform_expr(*obj, ctx, f)),
            slot,
            value: Box::new(transform_expr(*value, ctx, f)),
        },
        IrNode::Call { func, args } => IrNode::Call {
            func: Box::new(transform_expr(*func, ctx, f)),
            args: args.into_iter().map(|a| transform_expr(a, ctx, f)).collect(),
        },
        IrNode::CallProp { obj, prop, args, is_super, is_void, is_lex } => IrNode::CallProp {
            obj: Box::new(transform_expr(*obj, ctx, f)),
            prop: Box::new(transform_expr(*prop, ctx, f)),
            args: args.into_iter().map(|a| transform_expr(a, ctx, f)).collect(),
            is_super,
            is_void,
            is_lex,
        },
        IrNode::CallStatic { method, args } => IrNode::CallStatic {
            method: Box::new(transform_expr(*method, ctx, f)),
            args: args.into_iter().map(|a| transform_expr(a, ctx, f)).collect(),
        },
        IrNode::Construct { cls, args } => IrNode::Construct {
            cls: Box::new(transform_expr(*cls, ctx, f)),
            args: args.into_iter().map(|a| transform_expr(a, ctx, f)).collect(),
        },
        IrNode::ConstructProp { cls, args } => IrNode::ConstructProp {
            cls: Box::new(transform_expr(*cls, ctx, f)),
            args: args.into_iter().map(|a| transform_expr(a, ctx, f)).collect(),
        },
        IrNode::ConstructSuper(args) => {
            IrNode::ConstructSuper(args.into_iter().map(|a| transform_expr(a, ctx, f)).collect())
        }
        IrNode::CallSuper { prop, args, is_void } => IrNode::CallSuper {
            prop: Box::new(transform_expr(*prop, ctx, f)),
            args: args.into_iter().map(|a| transform_expr(a, ctx, f)).collect(),
            is_void,
        },
        IrNode::ApplyType { base, params } => IrNode::ApplyType {
            base: Box::new(transform_expr(*base, ctx, f)),
            params: params.into_iter().map(|a| transform_expr(a, ctx, f)).collect(),
        },
        IrNode::NewObject(props) => IrNode::NewObject(
            props
                .into_iter()
                .map(|(k, v)| (transform_expr(k, ctx, f), transform_expr(v, ctx, f)))
                .collect(),
        ),
        IrNode::NewArray(items) => {
            IrNode::NewArray(items.into_iter().map(|a| transform_expr(a, ctx, f)).collect())
        }
        IrNode::VarDecl { name, reg, ty, init, is_const } => IrNode::VarDecl {
            name,
            reg,
            ty: ty.map(|t| Box::new(transform_expr(*t, ctx, f))),
            init: init.map(|t| Box::new(transform_expr(*t, ctx, f))),
            is_const,
        },
        IrNode::Goto { target, cond } => IrNode::Goto {
            target,
            cond: cond.map(|c| Box::new(transform_expr(*c, ctx, f))),
        },
        IrNode::PushScope(v) => IrNode::PushScope(Box::new(transform_expr(*v, ctx, f))),
        IrNode::SetScopeValue { ns, is_late } => IrNode::SetScopeValue {
            ns: Box::new(transform_expr(*ns, ctx, f)),
            is_late,
        },
        IrNode::WithBlock { value, body } => IrNode::WithBlock {
            value: Box::new(transform_expr(*value, ctx, f)),
            body: Box::new(transform_expr(*body, ctx, f)),
        },
        IrNode::MemOp { op, args } => IrNode::MemOp {
            op,
            args: args.into_iter().map(|a| transform_expr(a, ctx, f)).collect(),
        },
        IrNode::RegSet { reg, value, hint } => IrNode::RegSet {
            reg,
            value: Box::new(transform_expr(*value, ctx, f)),
            hint,
        },
        IrNode::Phi { inputs } => IrNode::Phi {
            inputs: inputs
                .into_iter()
                .map(|(tag, v)| {
                    let tag = match tag {
                        crate::ir::PhiTag::Branch { cond, taken } => crate::ir::PhiTag::Branch {
                            cond: transform_expr(cond, ctx, f),
                            taken,
                        },
                        other => other,
                    };
                    (tag, transform_expr(v, ctx, f))
                })
                .collect(),
        },
        IrNode::Return(Some(v)) => IrNode::Return(Some(Box::new(transform_expr(*v, ctx, f)))),
        IrNode::Throw(v) => IrNode::Throw(Box::new(transform_expr(*v, ctx, f))),
        IrNode::NewCatch { catch_id, type_node, var_name } => IrNode::NewCatch {
            catch_id,
            type_node: type_node.map(|t| Box::new(transform_expr(*t, ctx, f))),
            var_name,
        },
        other => other,
    }
}

fn rewrite_params_reads(node: &IrNode, ctx: &MatchCtx) -> Option<IrNode> {
    if let IrNode::PropGet { obj, prop, .. } = node {
        if let IrNode::IntConst(k) = &**prop {
            if let IrNode::PropGet { obj: obj2, prop: prop2, .. } = &**obj {
                if let IrNode::NameRef { name, .. } = &**prop2 {
                    if name == "params" && ctx.is_enum(&peel_cast(obj2)).is_some() {
                        return Some(IrNode::ParamRef(*k as usize));
                    }
                }
            }
        }
    }
    None
}

fn collect_types_stmt(node: &SNode, ctx: &mut MatchCtx) {
    if let SNode::Expr(n) = node {
        if let IrNode::RegSet { reg, value, .. } = n {
            let t = static_type_of(value, ctx);
            if let Some(t) = t {
                ctx.regs.insert(*reg, t);
            }
        }
    }
}

fn static_type_of(node: &IrNode, ctx: &MatchCtx) -> Option<String> {
    let node = peel_cast(node);
    match &node {
        IrNode::RegAccess { reg, .. } => ctx.regs.get(reg).cloned(),
        IrNode::PropGet { obj, prop, .. } => {
            if let IrNode::NameRef { name, .. } = &**prop {
                if matches!(**obj, IrNode::ImplicitThis) {
                    return ctx.fields.get(name).cloned();
                }
                if let IrNode::RegAccess { reg, .. } = &**obj {
                    if let Some(t) = ctx.regs.get(reg) {
                        if ctx.enums.contains_key(t) && (name == "tag" || name == "index") {
                            return Some(if name == "index" { "Int" } else { "String" }.to_string());
                        }
                    }
                }
            }
            None
        }
        IrNode::CallProp { obj, .. } => {
            if let IrNode::NameRef { name, .. } = &**obj {
                let base = name.rsplit("::").next().unwrap_or("").rsplit('.').next().unwrap_or("");
                if ctx.enums.contains_key(base) {
                    return Some(base.to_string());
                }
            }
            None
        }
        IrNode::Call { func, .. } => {
            if let IrNode::NameRef { name, .. } = &**func {
                let nm = name.rsplit("::").next().unwrap_or("").rsplit('.').next().unwrap_or("");
                for (en, ed) in &ctx.enums {
                    if ed.ctors.iter().any(|c| c.name == nm) {
                        return Some(en.clone());
                    }
                }
            }
            None
        }
        IrNode::ConstructProp { cls, .. } => {
            let s = cls_short(cls);
            if s.is_empty() { None } else { Some(s) }
        }
        IrNode::NewArray(_) => Some("Array".to_string()),
        IrNode::BoolConst(_) => Some("Bool".to_string()),
        IrNode::IntConst(_) => Some("Int".to_string()),
        IrNode::StringConst(_) => Some("String".to_string()),
        _ => None,
    }
}

fn try_match(node: &mut SNode, ctx: &MatchCtx) {
    let SNode::Switch { value, cases, match_enum, match_base, case_plan, .. } = node else {
        return;
    };
    if match_enum.is_some() {
        return;
    }
    let IrNode::PropGet { obj, prop, .. } = value else { return };
    let IrNode::NameRef { name, .. } = &**prop else { return };
    if name != "index" {
        return;
    }
    let Some(edef) = ctx.is_enum(obj) else { return };
    let mut plan: Vec<(Option<Vec<EnumCtor>>, SNode)> = Vec::new();
    for (vals, body) in cases {
        if vals.is_empty() {
            plan.push((None, body.clone()));
            continue;
        }
        let ctors: Vec<Option<EnumCtor>> = vals.iter().map(|&x| edef.by_index(x).cloned()).collect();
        if ctors.iter().any(|c| c.is_none()) {
            return;
        }
        let ctors: Vec<EnumCtor> = ctors.into_iter().flatten().collect();
        if ctors.len() > 1 && ctors.iter().any(|c| c.nparams > 0) {
            return;
        }
        plan.push((Some(ctors), body.clone()));
    }
    *match_enum = Some(edef);
    *match_base = Some((**obj).clone());
    *case_plan = Some(plan);
}

pub fn apply_matches(
    tree: &mut SNode,
    abc: &AbcFile,
    param_types: &HashMap<usize, String>,
    field_types: &HashMap<String, String>,
    enums: &HashMap<usize, EnumDef>,
) {
    if enums.is_empty() {
        return;
    }
    let by_name: HashMap<String, EnumDef> = enums
        .values()
        .map(|e| (e.name.clone(), e.clone()))
        .collect();
    let mut ctx = MatchCtx::new(by_name, param_types, field_types);
    walk_s(tree, &mut ctx);
    let _ = abc;
}

fn walk_s(node: &mut SNode, ctx: &mut MatchCtx) {
    match node {
        SNode::Seq(stmts) => {
            for s in stmts.iter_mut() {
                walk_s(s, ctx);
            }
        }
        SNode::Expr(n) => {
            *n = transform_expr(std::mem::replace(n, IrNode::Nop), ctx, &mut |x, c| {
                rewrite_params_reads(x, c)
            });
            collect_types_stmt(node, ctx);
        }
        SNode::If { then_body, else_body, .. } => {
            walk_s(then_body, ctx);
            if let Some(e) = else_body {
                walk_s(e, ctx);
            }
        }
        SNode::While { body, .. } | SNode::DoWhile { body, .. } => {
            walk_s(body, ctx);
        }
        SNode::ForIn { body, .. } | SNode::ForRange { body, .. } => {
            walk_s(body, ctx);
        }
        SNode::Switch { .. } => {
            try_match(node, ctx);
            if let SNode::Switch { cases, case_plan, .. } = node {
                for (_vals, body) in cases.iter_mut() {
                    walk_s(body, ctx);
                    collapse_match_captures(body);
                }
                if let Some(plan) = case_plan {
                    for (_ctors, body) in plan.iter_mut() {
                        walk_s(body, ctx);
                        collapse_match_captures(body);
                    }
                }
            }
        }
        SNode::Try { body, catches, finally_body } => {
            walk_s(body, ctx);
            for (_var, _ty, b) in catches.iter_mut() {
                walk_s(b, ctx);
            }
            if let Some(fb) = finally_body {
                walk_s(fb, ctx);
            }
        }
        _ => {}
    }
}

fn deep_peel(node: &IrNode) -> IrNode {
    let mut cur = node.clone();
    loop {
        match cur {
            IrNode::Cast { expr, .. } | IrNode::Convert { expr, .. } => cur = *expr,
            other => return other,
        }
    }
}

fn collapse_match_captures(body: &mut SNode) {
    let SNode::Seq(stmts) = body else { return };
    let mut subs: HashMap<usize, IrNode> = HashMap::new();
    let mut out: Vec<SNode> = Vec::with_capacity(stmts.len());
    for s in stmts.drain(..) {
        let mut s = s;
        if let SNode::Expr(IrNode::RegSet { reg, value, .. }) = &mut s {
            let v = deep_peel(value);
            if let IrNode::ParamRef(idx) = v {
                subs.insert(*reg, IrNode::ParamRef(idx));
                continue;
            }
            subs.remove(reg);
        }
        if !subs.is_empty() {
            subst_in_stmt(&mut s, &subs);
        }
        out.push(s);
    }
    *stmts = out;
}

fn subst_in_stmt(s: &mut SNode, subs: &HashMap<usize, IrNode>) {
    match s {
        SNode::Expr(node) => {
            for (reg, pref) in subs {
                substitute_reg(node, *reg, pref);
            }
        }
        SNode::Seq(stmts) => {
            for x in stmts.iter_mut() {
                subst_in_stmt(x, subs);
            }
        }
        SNode::If { cond, then_body, else_body } => {
            for (reg, pref) in subs {
                substitute_reg(cond, *reg, pref);
            }
            subst_in_stmt(then_body, subs);
            if let Some(e) = else_body {
                subst_in_stmt(e, subs);
            }
        }
        SNode::While { cond, body } => {
            if let Some(c) = cond {
                for (reg, pref) in subs {
                    substitute_reg(c, *reg, pref);
                }
            }
            subst_in_stmt(body, subs);
        }
        SNode::DoWhile { cond, body } => {
            for (reg, pref) in subs {
                substitute_reg(cond, *reg, pref);
            }
            subst_in_stmt(body, subs);
        }
        SNode::Return(Some(v)) | SNode::Throw(v) => {
            for (reg, pref) in subs {
                substitute_reg(v, *reg, pref);
            }
        }
        _ => {}
    }
}

const HAXE_MARKER_STRINGS: &[&str] = &[
    "__constructs__", "__isenum", "__unprotect__", "__string_rec",
    "__instanceof", "enum_to_string", "__meta__", "__tostring__",
    "flash.Boot", "haxe.Log", "NativeStackTrace",
];

pub fn fingerprint_abc(abc: &AbcFile) -> (bool, i32, Vec<String>) {
    let strings: HashSet<&String> = abc.strings.iter().collect();
    let mut markers: Vec<String> = Vec::new();
    let mut score = 0i32;
    for s in HAXE_MARKER_STRINGS {
        let key = s.to_string();
        if strings.contains(&key) {
            markers.push(format!("str:{}", s));
            score += 1;
        }
    }
    let mut pkg_hits: HashSet<String> = HashSet::new();
    for c in &abc.classes {
        let full = crate::haxe_out::normalize_class_name(abc, c);
        let root = full.split('.').next().unwrap_or("").to_string();
        if ["haxe", "flash", "hxd", "openfl"].contains(&root.as_str()) {
            pkg_hits.insert(root);
        }
        if full == "flash.Boot" || full == "haxe.Log" {
            markers.push(format!("class:{}", full));
            score += 2;
        }
    }
    if !pkg_hits.is_empty() {
        let mut sorted: Vec<String> = pkg_hits.into_iter().collect();
        sorted.sort();
        markers.push(format!("pkgs:{}", sorted.join(",")));
        score += 1;
    }
    (score >= 3, score, markers)
}

fn is_reserved_check(node: &IrNode) -> bool {
    let node = deep_peel(node);
    let IrNode::BinaryOp { op, right, .. } = node else { return false };
    if op != "in" {
        return false;
    }
    let r = deep_peel(&right);
    let IrNode::PropGet { obj, prop, .. } = r else { return false };
    let IrNode::NameRef { name, .. } = &*prop else { return false };
    if name != "reserved" {
        return false;
    }
    let ro = deep_peel(&obj);
    matches!(ro, IrNode::NameRef { name: n, .. }
        if n.rsplit("::").next().unwrap_or("").rsplit('.').next().unwrap_or("") == "StringMap")
}

fn obj_key(node: &IrNode) -> (u8, i64, String) {
    let node = deep_peel(node);
    match node {
        IrNode::RegAccess { reg, .. } => (0, reg as i64, String::new()),
        other => (1, 0, format!("{:?}", other)),
    }
}

fn map_get_match(node: &IrNode) -> Option<IrNode> {
    let node = deep_peel(node);
    let (cond, tv, ev) = match node {
        IrNode::Ternary { cond, then_val, else_val } => (*cond, *then_val, *else_val),
        IrNode::Phi { inputs } => {
            let tagged: Vec<&(crate::ir::PhiTag, IrNode)> = inputs
                .iter()
                .filter(|(t, _)| matches!(t, crate::ir::PhiTag::Branch { .. }))
                .collect();
            if tagged.len() != 2 {
                return None;
            }
            let (t1, v1) = tagged[0];
            let (t2, v2) = tagged[1];
            let (c1, p1) = match t1 {
                crate::ir::PhiTag::Branch { cond, taken } => (cond, *taken),
                _ => return None,
            };
            let _p2 = match t2 {
                crate::ir::PhiTag::Branch { .. } => true,
                _ => return None,
            };
            let cond = crate::haxe_out::simplify_not(&c1);
            let (tv, ev) = if !p1 { (v1.clone(), v2.clone()) } else { (v2.clone(), v1.clone()) };
            (cond, tv, ev)
        }
        _ => return None,
    };
    if !is_reserved_check(&cond) {
        return None;
    }
    let tvp = deep_peel(&tv);
    let IrNode::CallProp { obj: tvobj, prop: tvprop, args: tvargs, .. } = tvp else {
        return None;
    };
    let IrNode::NameRef { name: tvname, .. } = &*tvprop else { return None };
    if tvname != "getReserved" || tvargs.len() != 1 {
        return None;
    }
    let evp = deep_peel(&ev);
    let IrNode::PropGet { obj: eobj, .. } = evp else { return None };
    let eobj_inner = deep_peel(&eobj);
    let IrNode::PropGet { obj: eeobj, prop: eprop, .. } = eobj_inner else { return None };
    let IrNode::NameRef { name: ename, .. } = &*eprop else { return None };
    if ename != "h" {
        return None;
    }
    if obj_key(&tvobj) != obj_key(&eeobj) {
        return None;
    }
    Some(IrNode::CallProp {
        obj: tvobj,
        prop: Box::new(IrNode::NameRef { name: "get".to_string(), raw_ns: String::new() }),
        args: vec![tvargs[0].clone()],
        is_super: false,
        is_void: false,
        is_lex: false,
    })
}

fn map_set_match(s: &SNode) -> Option<SNode> {
    let SNode::If { cond, then_body, else_body } = s else { return None };
    if !is_reserved_check(cond) {
        return None;
    }
    let then = seq_of(then_body);
    let els = else_body.as_ref().map(|b| seq_of(b)).unwrap_or_default();
    if then.len() != 1 || els.len() != 1 {
        return None;
    }
    let (t, e) = (&then[0], &els[0]);
    let SNode::Expr(tnode) = t else { return None };
    let IrNode::ExprStmt(texpr) = tnode else { return None };
    let tv = deep_peel(texpr);
    let IrNode::CallProp { obj: tvobj, prop: tvprop, args: tvargs, .. } = tv else {
        return None;
    };
    let IrNode::NameRef { name: tvname, .. } = &*tvprop else { return None };
    if tvname != "setReserved" || tvargs.len() != 2 {
        return None;
    }
    let SNode::Expr(enode) = e else { return None };
    let IrNode::ExprStmt(eexpr) = enode else { return None };
    let ev = deep_peel(eexpr);
    let IrNode::PropSet { obj: eobj, prop: eprop, value: _evalue, .. } = ev else {
        return None;
    };
    let eobj_inner = deep_peel(&eobj);
    let IrNode::PropGet { obj: eeobj, prop: eeprop, .. } = eobj_inner else { return None };
    let IrNode::NameRef { name: ename, .. } = &*eeprop else { return None };
    if ename != "h" {
        return None;
    }
    if obj_key(&tvobj) != obj_key(&eeobj) {
        return None;
    }
    let k1 = &tvargs[0];
    let v = &tvargs[1];
    if obj_key(k1) != obj_key(&eprop) && *k1 != *eprop {
        return None;
    }
    Some(SNode::Expr(IrNode::ExprStmt(Box::new(IrNode::CallProp {
        obj: tvobj,
        prop: Box::new(IrNode::NameRef { name: "set".to_string(), raw_ns: String::new() }),
        args: vec![k1.clone(), v.clone()],
        is_super: false,
        is_void: false,
        is_lex: false,
    }))))
}

fn count_reg_reads(tree: &SNode) -> HashMap<usize, i32> {
    let mut counts: HashMap<usize, i32> = HashMap::new();

    fn node(nd: &IrNode, counts: &mut HashMap<usize, i32>) {
        match nd {
            IrNode::RegAccess { reg, .. } => {
                *counts.entry(*reg).or_insert(0) += 1;
            }
            IrNode::RegSet { value, .. } => node(value, counts),
            IrNode::HasNext { obj_reg, idx_reg, .. } => {
                *counts.entry(*obj_reg).or_insert(0) += 1;
                *counts.entry(*idx_reg).or_insert(0) += 1;
            }
            other => {
                other.for_each_child(&mut |c| node(c, counts));
            }
        }
    }

    fn stmt(s: &SNode, counts: &mut HashMap<usize, i32>) {
        match s {
            SNode::Seq(stmts) => {
                for x in stmts {
                    stmt(x, counts);
                }
            }
            SNode::Expr(nd) => node(nd, counts),
            SNode::If { cond, then_body, else_body } => {
                node(cond, counts);
                stmt(then_body, counts);
                if let Some(e) = else_body {
                    stmt(e, counts);
                }
            }
            SNode::While { cond, body } => {
                if let Some(c) = cond {
                    node(c, counts);
                }
                stmt(body, counts);
            }
            SNode::DoWhile { cond, body } => {
                node(cond, counts);
                stmt(body, counts);
            }
            SNode::ForIn { obj, body, .. } => {
                node(obj, counts);
                stmt(body, counts);
            }
            SNode::ForRange { start, end, body, .. } => {
                node(start, counts);
                node(end, counts);
                stmt(body, counts);
            }
            SNode::Switch { value, cases, case_plan, .. } => {
                node(value, counts);
                for (_vals, body) in cases {
                    stmt(body, counts);
                }
                if let Some(plan) = case_plan {
                    for (_ctors, body) in plan {
                        stmt(body, counts);
                    }
                }
            }
            SNode::Try { body, catches, finally_body } => {
                stmt(body, counts);
                for (_var, _ty, b) in catches {
                    stmt(b, counts);
                }
                if let Some(fb) = finally_body {
                    stmt(fb, counts);
                }
            }
            SNode::Return(Some(v)) | SNode::Throw(v) => node(v, counts),
            _ => {}
        }
    }

    stmt(tree, &mut counts);
    counts
}

fn is_pure_ir(nd: &IrNode) -> bool {
    let nd = deep_peel(nd);
    match nd {
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
        IrNode::PropGet { obj, .. } => is_pure_ir(&obj),
        _ => false,
    }
}

pub fn collapse_map_ops(tree: &mut SNode) {
    fn try_inline_receiver(repl: &mut SNode, out: &mut [SNode]) {
        let mut recvs: Vec<usize> = Vec::new();
        {
            let mut collect = |nd: &IrNode| {
                fn c(nd: &IrNode, recvs: &mut Vec<usize>) {
                    match nd {
                        IrNode::RegAccess { reg, .. } => recvs.push(*reg),
                        IrNode::RegSet { value, .. } => c(value, recvs),
                        other => other.for_each_child(&mut |ch| c(ch, recvs)),
                    }
                }
                c(nd, &mut recvs);
            };
            match repl {
                SNode::Expr(n) => collect(n),
                SNode::If { cond, .. } => collect(cond),
                _ => {}
            }
        }
        let recvs_set: HashSet<usize> = recvs.into_iter().collect();
        for reg in recvs_set {
            for back in 1..4.min(out.len() + 1) {
                let idx = out.len() - back;
                let prev = &mut out[idx];
                let defines = defines_reg_anywhere(prev, reg);
                if defines {
                    if let SNode::Expr(IrNode::RegSet { reg: pr, value, .. }) = prev {
                        if *pr == reg {
                            let v = deep_peel(value);
                            let is_literal = matches!(
                                v,
                                IrNode::IntConst(_)
                                    | IrNode::UIntConst(_)
                                    | IrNode::DoubleConst(_)
                                    | IrNode::StringConst(_)
                                    | IrNode::BoolConst(_)
                                    | IrNode::NullConst
                                    | IrNode::UndefinedConst
                                    | IrNode::NaNConst
                            );
                            let is_regaccess = matches!(v, IrNode::RegAccess { .. });
                            if !is_regaccess && !is_literal && is_pure_ir(value) {
                                substitute_reg(
                                    match repl {
                                        SNode::Expr(n) => n,
                                        SNode::If { cond, .. } => cond,
                                        _ => continue,
                                    },
                                    reg,
                                    value,
                                );
                            }
                        }
                        break;
                    }
                    break;
                }
            }
        }
    }

    fn defines_reg_anywhere(s: &SNode, reg: usize) -> bool {
        fn check(nd: &SNode, reg: usize) -> bool {
            match nd {
                SNode::Expr(node) => matches!(node, IrNode::RegSet { reg: r, .. } if *r == reg),
                SNode::Seq(stmts) => stmts.iter().any(|x| check(x, reg)),
                SNode::If { then_body, else_body, .. } => {
                    check(then_body, reg)
                        || else_body.as_ref().map(|e| check(e, reg)).unwrap_or(false)
                }
                SNode::While { body, .. }
                | SNode::DoWhile { body, .. }
                | SNode::ForIn { body, .. }
                | SNode::ForRange { body, .. } => check(body, reg),
                SNode::Switch { cases, case_plan, .. } => {
                    cases.iter().any(|(_, b)| check(b, reg))
                        || case_plan
                            .as_ref()
                            .map(|p| p.iter().any(|(_, b)| check(b, reg)))
                            .unwrap_or(false)
                }
                SNode::Try { body, catches, finally_body } => {
                    check(body, reg)
                        || catches.iter().any(|(_, _, b)| check(b, reg))
                        || finally_body
                            .as_ref()
                            .map(|f| check(f, reg))
                            .unwrap_or(false)
                }
                _ => false,
            }
        }
        check(s, reg)
    }

    fn walk_stmts(seq: &mut SNode) {
        match seq {
            SNode::Seq(stmts) => {
                let mut out: Vec<SNode> = Vec::with_capacity(stmts.len());
                for s in stmts.drain(..) {
                    let mut s = s;
                    if let Some(mut repl) = map_set_match(&s) {
                        try_inline_receiver(&mut repl, &mut out);
                        out.push(repl);
                        continue;
                    }
                    walk(&mut s);
                    if let SNode::Expr(IrNode::RegSet { .. }) = &s {
                        let mut out_ref = std::mem::take(&mut out);
                        if let SNode::Expr(node) = &mut s {
                            try_inline_receiver_expr(node, &mut out_ref);
                        }
                        out = out_ref;
                    }
                    out.push(s);
                }
                *stmts = out;
            }
            _ => walk(seq),
        }
    }

    fn try_inline_receiver_expr(node: &mut IrNode, out: &mut Vec<SNode>) {
        let mut recvs: Vec<usize> = Vec::new();
        fn collect(nd: &IrNode, recvs: &mut Vec<usize>) {
            match nd {
                IrNode::RegAccess { reg, .. } => recvs.push(*reg),
                IrNode::RegSet { value, .. } => collect(value, recvs),
                other => other.for_each_child(&mut |ch| collect(ch, recvs)),
            }
        }
        collect(node, &mut recvs);
        let recvs_set: HashSet<usize> = recvs.into_iter().collect();
        for reg in recvs_set {
            for back in 1..4.min(out.len() + 1) {
                let idx = out.len() - back;
                let prev = &mut out[idx];
                if defines_reg_anywhere(prev, reg) {
                    if let SNode::Expr(IrNode::RegSet { reg: pr, value, .. }) = prev {
                        if *pr == reg {
                            let v = deep_peel(value);
                            let is_literal = matches!(
                                v,
                                IrNode::IntConst(_)
                                    | IrNode::UIntConst(_)
                                    | IrNode::DoubleConst(_)
                                    | IrNode::StringConst(_)
                                    | IrNode::BoolConst(_)
                                    | IrNode::NullConst
                                    | IrNode::UndefinedConst
                                    | IrNode::NaNConst
                            );
                            let is_regaccess = matches!(v, IrNode::RegAccess { .. });
                            if !is_regaccess && !is_literal && is_pure_ir(value) {
                                substitute_reg(node, reg, value);
                            }
                        }
                        break;
                    }
                    break;
                }
            }
        }
    }

    fn walk(s: &mut SNode) {
        match s {
            SNode::Expr(node) => {
                *node = transform_expr(std::mem::replace(node, IrNode::Nop), &mut (), &mut |n, _: &mut ()| {
                    map_get_match(n)
                });
            }
            SNode::If { then_body, else_body, .. } => {
                walk_stmts(then_body);
                if let Some(e) = else_body {
                    walk_stmts(e);
                }
            }
            SNode::While { body, .. }
            | SNode::DoWhile { body, .. }
            | SNode::ForIn { body, .. }
            | SNode::ForRange { body, .. } => {
                walk_stmts(body);
            }
            SNode::Switch { cases, case_plan, .. } => {
                for (_vals, body) in cases.iter_mut() {
                    walk_stmts(body);
                }
                if let Some(plan) = case_plan {
                    for (_ctors, body) in plan.iter_mut() {
                        walk_stmts(body);
                    }
                }
            }
            SNode::Try { body, catches, finally_body } => {
                walk_stmts(body);
                for (_var, _ty, b) in catches.iter_mut() {
                    walk_stmts(b);
                }
                if let Some(fb) = finally_body {
                    walk_stmts(fb);
                }
            }
            _ => {}
        }
    }

    walk_stmts(tree);

    let counts = count_reg_reads(tree);
    fn sweep(seq: &mut SNode, counts: &HashMap<usize, i32>) {
        match seq {
            SNode::Seq(stmts) => {
                stmts.retain(|s| {
                    !matches!(
                        s,
                        SNode::Expr(IrNode::RegSet { reg, value, .. })
                            if counts.get(reg).copied().unwrap_or(0) == 0
                                && *reg != 0
                                && is_pure_ir(value)
                    )
                });
                for s in stmts.iter_mut() {
                    sweep2(s, counts);
                }
            }
            _ => sweep2(seq, counts),
        }
    }
    fn sweep2(s: &mut SNode, counts: &HashMap<usize, i32>) {
        match s {
            SNode::If { then_body, else_body, .. } => {
                sweep(then_body, counts);
                if let Some(e) = else_body {
                    sweep(e, counts);
                }
            }
            SNode::While { body, .. }
            | SNode::DoWhile { body, .. }
            | SNode::ForIn { body, .. }
            | SNode::ForRange { body, .. } => sweep(body, counts),
            SNode::Switch { cases, case_plan, .. } => {
                for (_vals, body) in cases.iter_mut() {
                    sweep(body, counts);
                }
                if let Some(plan) = case_plan {
                    for (_ctors, body) in plan.iter_mut() {
                        sweep(body, counts);
                    }
                }
            }
            SNode::Try { body, catches, finally_body } => {
                sweep(body, counts);
                for (_var, _ty, b) in catches.iter_mut() {
                    sweep(b, counts);
                }
                if let Some(fb) = finally_body {
                    sweep(fb, counts);
                }
            }
            _ => {}
        }
    }
    sweep(tree, &counts);
}

const NAME_STOP: &[&str] = &[
    "h", "rh", "index", "tag", "params", "length", "value", "key", "next",
    "hasNext", "call", "apply", "bind", "toString", "constructor",
    "prototype", "hasOwnProperty", "get", "set", "args", "this", "super",
    "iterator", "keys", "values", "copy", "push", "pop", "shift", "concat",
    "join", "slice", "splice", "sort", "reverse", "indexOf", "lastIndexOf",
    "filter", "forEach", "reduce", "has", "exists", "remove", "clear",
    "add", "isNaN", "isFinite", "parseInt", "parseFloat", "random", "floor",
    "ceil", "round", "abs", "min", "max", "pow", "sqrt", "name", "string",
    "charAt", "charCodeAt", "substr", "substring", "split", "toUpperCase",
    "toLowerCase", "trim", "compare", "resolve", "getReserved",
    "setReserved", "reserved", "isReserved", "nextIndex", "main",
];

pub fn name_ok(nm: &str) -> bool {
    if nm.len() < 2 {
        return false;
    }
    if NAME_STOP.contains(&nm) {
        return false;
    }
    if nm.starts_with("__") || nm.starts_with("get_") || nm.starts_with("set_") {
        return false;
    }
    let first = nm.chars().next().unwrap();
    if !first.is_alphabetic() && first != '_' {
        return false;
    }
    nm.chars().all(|c| c.is_alphanumeric() || c == '_')
}

pub struct NamingCtx {
    pub current: HashMap<usize, String>,
    pub used: HashSet<String>,
}

impl NamingCtx {
    pub fn new(abc: &AbcFile, m: Option<&crate::abc::MethodInfo>, body: &crate::abc::MethodBody) -> Self {
        let mut current: HashMap<usize, String> = HashMap::new();
        let mut used: HashSet<String> =
            ["this", "trace", "super", "k", "v", "i", "j"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        if let Some(m) = m {
            for (i, _) in m.params.iter().enumerate() {
                let r = i + 1;
                let mut nm: Option<String> = None;
                if !m.param_names.is_empty()
                    && i < m.param_names.len()
                    && m.param_names[i] > 0
                    && ((m.param_names[i] - 1) as usize) < abc.strings.len()
                {
                    nm = Some(abc.strings[(m.param_names[i] - 1) as usize].clone());
                }
                if let Some(nm) = nm {
                    current.insert(r, nm.clone());
                    used.insert(nm);
                }
            }
            let mut next_reg = m.params.len() + 1;
            if m.need_activation() {
                next_reg += 1;
            }
            for t in &body.traits {
                if t.name_idx > 0 && ((t.name_idx - 1) as usize) < abc.multinames.len() {
                    let nm = abc.multinames[(t.name_idx - 1) as usize]
                        .resolve(abc)
                        .rsplit("::")
                        .next()
                        .unwrap_or("")
                        .to_string();
                    current.insert(next_reg, nm.clone());
                    used.insert(nm);
                }
                next_reg += 1;
            }
        }
        for ins in &body.instructions {
            if ins.name == "debug" && ins.operands.len() >= 4 && ins.operands[0] == 1 {
                let nidx = ins.operands[1] as usize;
                let reg = ins.operands[2] as usize;
                if nidx > 0 && !current.contains_key(&reg) && nidx <= abc.strings.len() {
                    let nm = &abc.strings[nidx - 1];
                    if name_ok(nm) {
                        current.insert(reg, nm.clone());
                        used.insert(nm.clone());
                    }
                }
            }
        }
        NamingCtx { current, used }
    }

    pub fn claim(&mut self, base: &str) -> String {
        let mut nm = base.to_string();
        let mut n2 = 2;
        while self.used.contains(&nm) {
            nm = format!("{}{}", base, n2);
            n2 += 1;
        }
        self.used.insert(nm.clone());
        nm
    }

    pub fn hint_for(&self, node: &IrNode) -> Option<String> {
        let node = deep_peel(node);
        match &node {
            IrNode::RegAccess { reg, .. } => self.current.get(reg).cloned(),
            IrNode::PropGet { obj, prop, .. } => {
                if let IrNode::NameRef { name, .. } = &**prop {
                    let obj = deep_peel(obj);
                    if matches!(
                        obj,
                        IrNode::ImplicitThis | IrNode::This | IrNode::RegAccess { .. } | IrNode::NameRef { .. }
                    ) {
                        if name_ok(name) {
                            return Some(name.clone());
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }
}

pub fn apply_register_naming(
    tree: &mut SNode,
    abc: &AbcFile,
    m: Option<&crate::abc::MethodInfo>,
    body: &crate::abc::MethodBody,
) {
    let mut ctx = NamingCtx::new(abc, m, body);

    fn touch_in_node(nd: &mut IrNode, ctx: &mut NamingCtx) {
        match nd {
            IrNode::RegAccess { reg, hint } | IrNode::RegSet { reg, hint, .. } => {
                if let Some(nm) = ctx.current.get(reg) {
                    if hint.is_none() {
                        *hint = Some(nm.clone());
                    }
                }
                if let IrNode::RegSet { value, .. } = nd {
                    touch_in_node(value, ctx);
                }
            }
            other => {
                other.map_children_mut(&mut |c| touch_in_node(c, ctx));
            }
        }
    }

    fn walk(seq: &mut SNode, ctx: &mut NamingCtx) {
        match seq {
            SNode::Seq(stmts) => {
                for s in stmts.iter_mut() {
                    walk(s, ctx);
                }
            }
            SNode::Expr(node) => {
                touch_in_node(node, ctx);
                if let IrNode::RegSet { reg, value, hint, .. } = node {
                    let r = *reg;
                    if !ctx.current.contains_key(&r) {
                        if let Some(cand) = ctx.hint_for(&**value) {
                            let nm = ctx.claim(&cand);
                            ctx.current.insert(r, nm.clone());
                            *hint = Some(nm);
                        }
                    } else if hint.is_none() {
                        *hint = ctx.current.get(&r).cloned();
                    }
                }
            }
            SNode::If { cond, then_body, else_body } => {
                touch_in_node(cond, ctx);
                walk(then_body, ctx);
                if let Some(e) = else_body {
                    walk(e, ctx);
                }
            }
            SNode::While { cond, body } => {
                if let Some(c) = cond {
                    touch_in_node(c, ctx);
                }
                walk(body, ctx);
            }
            SNode::DoWhile { cond, body } => {
                touch_in_node(cond, ctx);
                walk(body, ctx);
            }
            SNode::ForIn { obj, body, var_reg, var_name, .. } => {
                touch_in_node(obj, ctx);
                if *var_reg >= 0 {
                    if let Some(nm) = ctx.current.get(&(*var_reg as usize)) {
                        *var_name = nm.clone();
                    }
                }
                walk(body, ctx);
            }
            SNode::ForRange { start, end, body, .. } => {
                touch_in_node(start, ctx);
                touch_in_node(end, ctx);
                walk(body, ctx);
            }
            SNode::Switch { value, cases, case_plan, match_base, .. } => {
                touch_in_node(value, ctx);
                if let Some(mb) = match_base {
                    touch_in_node(mb, ctx);
                }
                for (_vals, body2) in cases.iter_mut() {
                    walk(body2, ctx);
                }
                if let Some(plan) = case_plan {
                    for (_ctors, body2) in plan.iter_mut() {
                        walk(body2, ctx);
                    }
                }
            }
            SNode::Return(Some(v)) | SNode::Throw(v) => touch_in_node(v, ctx),
            SNode::Try { body, catches, finally_body } => {
                walk(body, ctx);
                for (_var, _ty, body2) in catches.iter_mut() {
                    walk(body2, ctx);
                }
                if let Some(fb) = finally_body {
                    walk(fb, ctx);
                }
            }
            _ => {}
        }
    }

    walk(tree, &mut ctx);
}

pub fn apply_synthetic_names(tree: &mut SNode) {
    struct Names {
        map: HashMap<usize, String>,
        used: HashSet<String>,
        counter: usize,
    }
    impl Names {
        fn fresh(&mut self) -> String {
            let mut n = self.counter + 1;
            loop {
                let nm = format!("_variable{}_", n);
                if !self.used.contains(&nm) {
                    self.counter = n;
                    self.used.insert(nm.clone());
                    return nm;
                }
                n += 1;
            }
        }
        fn name_for(&mut self, reg: usize) -> String {
            if let Some(nm) = self.map.get(&reg) {
                return nm.clone();
            }
            let nm = self.fresh();
            self.map.insert(reg, nm.clone());
            nm
        }
    }
    let mut names = Names { map: HashMap::new(), used: HashSet::new(), counter: 0 };

    fn fix_node(nd: &mut IrNode, names: &mut Names) {
        match nd {
            IrNode::RegAccess { reg, hint } | IrNode::RegSet { reg, hint, .. } => {
                if hint.is_none() {
                    *hint = Some(names.name_for(*reg));
                }
                if let IrNode::RegSet { value, .. } = nd {
                    fix_node(value, names);
                }
            }
            IrNode::NewFunction { .. } | IrNode::NewClass { .. } => {}
            other => {
                other.map_children_mut(&mut |c| fix_node(c, names));
            }
        }
    }

    fn fix_loop_name(
        var_name: &mut String,
        var_reg: i64,
        names: &mut Names,
    ) {
        let reg = var_reg;
        let nm = var_name.clone();
        if reg >= 0 && (nm.is_empty() || nm == format!("__r{}", reg) || nm == format!("_variable{}_", reg)) {
            *var_name = names.name_for(reg as usize);
        }
    }

    fn walk(seq: &mut SNode, names: &mut Names) {
        match seq {
            SNode::Seq(stmts) => {
                for s in stmts.iter_mut() {
                    walk(s, names);
                }
            }
            SNode::Expr(node) => {
                let mut target = std::mem::replace(node, IrNode::Nop);
                if let IrNode::ExprStmt(e) = &mut target {
                    fix_node(e, names);
                } else {
                    fix_node(&mut target, names);
                }
                *node = target;
            }
            SNode::If { cond, then_body, else_body } => {
                fix_node(cond, names);
                walk(then_body, names);
                if let Some(e) = else_body {
                    walk(e, names);
                }
            }
            SNode::While { cond, body } => {
                if let Some(c) = cond {
                    fix_node(c, names);
                }
                walk(body, names);
            }
            SNode::DoWhile { cond, body } => {
                fix_node(cond, names);
                walk(body, names);
            }
            SNode::ForIn { obj, body, var_name, var_reg, .. } => {
                fix_node(obj, names);
                fix_loop_name(var_name, *var_reg, names);
                walk(body, names);
            }
            SNode::ForRange { start, end, body, var_name, var_reg, .. } => {
                fix_node(start, names);
                fix_node(end, names);
                fix_loop_name(var_name, *var_reg, names);
                walk(body, names);
            }
            SNode::Switch { value, cases, case_plan, match_base, .. } => {
                fix_node(value, names);
                if let Some(mb) = match_base {
                    fix_node(mb, names);
                }
                for (_v, body2) in cases.iter_mut() {
                    walk(body2, names);
                }
                if let Some(plan) = case_plan {
                    for (_c, body2) in plan.iter_mut() {
                        walk(body2, names);
                    }
                }
            }
            SNode::Return(Some(v)) | SNode::Throw(v) => fix_node(v, names),
            SNode::Try { body, catches, finally_body } => {
                walk(body, names);
                for (_v, _t, body2) in catches.iter_mut() {
                    walk(body2, names);
                }
                if let Some(fb) = finally_body {
                    walk(fb, names);
                }
            }
            _ => {}
        }
    }

    walk(tree, &mut names);
}

pub fn insert_var_decls(tree: SNode, n_params: usize) -> SNode {
    let mut tree = as_seq_root(tree);
    normalize_bodies(&mut tree);
    drop_dead_defs(&mut tree, n_params);
    compact(&mut tree);
    collapse_single_use_copies(&mut tree);
    compact(&mut tree);
    assign_declarations(&mut tree, n_params);
    compact(&mut tree);
    tree
}

fn as_seq_root(tree: SNode) -> SNode {
    match tree {
        SNode::Seq(_) => tree,
        other => SNode::Seq(vec![other]),
    }
}

fn as_seq(b: SNode) -> SNode {
    match b {
        SNode::Seq(_) => b,
        other => SNode::Seq(vec![other]),
    }
}

fn normalize_bodies(n: &mut SNode) {
    match n {
        SNode::Seq(stmts) => {
            for s in stmts.iter_mut() {
                normalize_bodies(s);
            }
        }
        SNode::If { then_body, else_body, .. } => {
            *then_body = Box::new(as_seq(std::mem::replace(then_body.as_mut(), SNode::empty_seq())));
            normalize_bodies(then_body);
            if let Some(e) = else_body {
                *e = Box::new(as_seq(std::mem::replace(e.as_mut(), SNode::empty_seq())));
                normalize_bodies(e);
            }
        }
        SNode::While { body, .. } | SNode::DoWhile { body, .. } => {
            *body = Box::new(as_seq(std::mem::replace(body.as_mut(), SNode::empty_seq())));
            normalize_bodies(body);
        }
        SNode::ForIn { body, .. } | SNode::ForRange { body, .. } => {
            *body = Box::new(as_seq(std::mem::replace(body.as_mut(), SNode::empty_seq())));
            normalize_bodies(body);
        }
        SNode::Switch { cases, case_plan, .. } => {
            for (_v, b) in cases.iter_mut() {
                *b = as_seq(std::mem::replace(b, SNode::empty_seq()));
                normalize_bodies(b);
            }
            if let Some(plan) = case_plan {
                for (_c, b) in plan.iter_mut() {
                    *b = as_seq(std::mem::replace(b, SNode::empty_seq()));
                    normalize_bodies(b);
                }
            }
        }
        SNode::Try { body, catches, finally_body } => {
            *body = Box::new(as_seq(std::mem::replace(body.as_mut(), SNode::empty_seq())));
            normalize_bodies(body);
            for (_v, _t, b) in catches.iter_mut() {
                *b = as_seq(std::mem::replace(b, SNode::empty_seq()));
                normalize_bodies(b);
            }
            if let Some(fb) = finally_body {
                *fb = Box::new(as_seq(std::mem::replace(fb.as_mut(), SNode::empty_seq())));
                normalize_bodies(fb);
            }
        }
        _ => {}
    }
}

fn compact(n: &mut SNode) {
    match n {
        SNode::Seq(stmts) => {
            stmts.retain(|s| !matches!(s, SNode::Removed));
            for s in stmts.iter_mut() {
                compact(s);
            }
        }
        SNode::If { then_body, else_body, .. } => {
            compact(then_body);
            if let Some(e) = else_body {
                compact(e);
            }
        }
        SNode::While { body, .. } | SNode::DoWhile { body, .. } => compact(body),
        SNode::ForIn { body, .. } | SNode::ForRange { body, .. } => compact(body),
        SNode::Switch { cases, case_plan, .. } => {
            for (_v, b) in cases.iter_mut() {
                compact(b);
            }
            if let Some(plan) = case_plan {
                for (_c, b) in plan.iter_mut() {
                    compact(b);
                }
            }
        }
        SNode::Try { body, catches, finally_body } => {
            compact(body);
            for (_v, _t, b) in catches.iter_mut() {
                compact(b);
            }
            if let Some(fb) = finally_body {
                compact(fb);
            }
        }
        _ => {}
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Step {
    Then,
    Else,
    Body,
    Catch(usize),
    Finally,
    Case(usize),
    Plan(usize),
}
type Path = Vec<Step>;

fn resolve<'a>(root: &'a mut SNode, path: &[Step]) -> Option<&'a mut SNode> {
    let mut cur: &mut SNode = root;
    for step in path {
        cur = match (step, cur) {
            (Step::Then, SNode::If { then_body, .. }) => then_body,
            (Step::Else, SNode::If { else_body: Some(e), .. }) => e,
            (Step::Body, SNode::While { body, .. })
            | (Step::Body, SNode::DoWhile { body, .. })
            | (Step::Body, SNode::ForIn { body, .. })
            | (Step::Body, SNode::ForRange { body, .. }) => body,
            (Step::Body, SNode::Try { body, .. }) => body,
            (Step::Catch(i), SNode::Try { catches, .. }) => &mut catches[*i].2,
            (Step::Finally, SNode::Try { finally_body: Some(fb), .. }) => fb,
            (Step::Case(i), SNode::Switch { cases, .. }) => &mut cases[*i].1,
            (Step::Plan(i), SNode::Switch { case_plan: Some(p), .. }) => &mut p[*i].1,
            _ => return None,
        };
    }
    Some(cur)
}

#[derive(Debug, Clone)]
struct Event {
    seq_path: Path,
    idx: usize,
    pos: usize,
    chain: Path,
    is_def: bool,
    implicit: bool,
}

fn collect_reads_expr(v: Option<&IrNode>, acc: &mut HashSet<usize>) {
    crate::ir::collect_reads(v, acc);
}

fn exprs_of(stmt: &SNode) -> Vec<&IrNode> {
    match stmt {
        SNode::Expr(node) => {
            if let IrNode::ExprStmt(e) = node {
                vec![e]
            } else {
                vec![node]
            }
        }
        SNode::If { cond, .. } => vec![cond],
        SNode::While { cond: Some(c), .. } => vec![c],
        SNode::DoWhile { cond, .. } => vec![cond],
        SNode::ForIn { obj, .. } => vec![obj],
        SNode::ForRange { start, end, .. } => vec![start, end],
        SNode::Switch { value, match_base, .. } => {
            let mut v = vec![value];
            if let Some(mb) = match_base {
                v.push(mb);
            }
            v
        }
        SNode::Return(Some(v)) | SNode::Throw(v) => vec![v],
        _ => vec![],
    }
}

struct EventScan {
    events: HashMap<usize, Vec<Event>>,
    loop_regs: HashSet<usize>,
    loops_by_reg: HashMap<usize, Vec<Path>>,
    pos: usize,
}

fn scan_events(root: &SNode) -> EventScan {
    let mut scan = EventScan {
        events: HashMap::new(),
        loop_regs: HashSet::new(),
        loops_by_reg: HashMap::new(),
        pos: 0,
    };
    walk_stmts(root, Vec::new(), &mut scan);
    scan
}

fn walk_stmts(seq: &SNode, path: Path, scan: &mut EventScan) {
    if let SNode::Seq(stmts) = seq {
        for (i, s) in stmts.iter().enumerate() {
            visit(s, &path, i, scan);
            walk_container(s, path.clone(), scan);
        }
    } else {
        walk_container(seq, path, scan);
    }
}

fn visit(stmt: &SNode, seq_path: &Path, idx: usize, scan: &mut EventScan) {
    let target: &IrNode = match stmt {
        SNode::Expr(node) => node,
        _ => {
            let mut reads = HashSet::new();
            for e in exprs_of(stmt) {
                collect_reads_expr(Some(e), &mut reads);
            }
            for r in reads {
                scan.pos += 1;
                scan.events.entry(r).or_default().push(Event {
                    seq_path: seq_path.to_vec(),
                    idx,
                    pos: scan.pos,
                    chain: seq_path.to_vec(),
                    is_def: false,
                    implicit: false,
                });
            }
            return;
        }
    };
    let reg_set = match target {
        IrNode::RegSet { reg, value, .. } => Some((*reg, value)),
        _ => None,
    };
    if let Some((reg, value)) = reg_set {
        let mut reads = HashSet::new();
        collect_reads_expr(Some(value), &mut reads);
        for r in reads {
            scan.pos += 1;
            scan.events.entry(r).or_default().push(Event {
                seq_path: seq_path.to_vec(),
                idx,
                pos: scan.pos,
                chain: seq_path.to_vec(),
                is_def: false,
                implicit: false,
            });
        }
        scan.pos += 1;
        scan.events.entry(reg).or_default().push(Event {
            seq_path: seq_path.to_vec(),
            idx,
            pos: scan.pos,
            chain: seq_path.to_vec(),
            is_def: true,
            implicit: false,
        });
    } else {
        let mut reads = HashSet::new();
        for e in exprs_of(stmt) {
            collect_reads_expr(Some(e), &mut reads);
        }
        for r in reads {
            scan.pos += 1;
            scan.events.entry(r).or_default().push(Event {
                seq_path: seq_path.to_vec(),
                idx,
                pos: scan.pos,
                chain: seq_path.to_vec(),
                is_def: false,
                implicit: false,
            });
        }
    }
    if let SNode::ForIn { var_reg, val_reg, kind, obj, .. } = stmt {
        if *var_reg >= 0 {
            let reg = *var_reg as usize;
            scan.loop_regs.insert(reg);
            scan.loops_by_reg.entry(reg).or_default().push(seq_path.clone());
            scan.pos += 1;
            scan.events.entry(reg).or_default().push(Event {
                seq_path: seq_path.to_vec(),
                idx,
                pos: scan.pos,
                chain: seq_path.to_vec(),
                is_def: true,
                implicit: true,
            });
        }
        if *kind == "map" && *val_reg >= 0 {
            let reg = *val_reg as usize;
            scan.loop_regs.insert(reg);
            scan.loops_by_reg.entry(reg).or_default().push(seq_path.clone());
            scan.pos += 1;
            scan.events.entry(reg).or_default().push(Event {
                seq_path: seq_path.to_vec(),
                idx,
                pos: scan.pos,
                chain: seq_path.to_vec(),
                is_def: true,
                implicit: true,
            });
        }
        let mut reads = HashSet::new();
        collect_reads_expr(Some(obj), &mut reads);
        for r in reads {
            scan.pos += 1;
            scan.events.entry(r).or_default().push(Event {
                seq_path: seq_path.to_vec(),
                idx,
                pos: scan.pos,
                chain: seq_path.to_vec(),
                is_def: false,
                implicit: false,
            });
        }
    } else if let SNode::ForRange { var_reg, index_reg, start, end, .. } = stmt {
        if *var_reg >= 0 {
            let reg = *var_reg as usize;
            scan.loop_regs.insert(reg);
            scan.loops_by_reg.entry(reg).or_default().push(seq_path.clone());
            scan.pos += 1;
            scan.events.entry(reg).or_default().push(Event {
                seq_path: seq_path.to_vec(),
                idx,
                pos: scan.pos,
                chain: seq_path.to_vec(),
                is_def: true,
                implicit: true,
            });
        }
        if *index_reg >= 0 {
            scan.loop_regs.insert(*index_reg as usize);
        }
        for e in [start, end] {
            let mut reads = HashSet::new();
            collect_reads_expr(Some(e), &mut reads);
            for r in reads {
                scan.pos += 1;
                scan.events.entry(r).or_default().push(Event {
                    seq_path: seq_path.to_vec(),
                    idx,
                    pos: scan.pos,
                    chain: seq_path.to_vec(),
                    is_def: false,
                    implicit: false,
                });
            }
        }
    }
}

fn walk_container(s: &SNode, path: Path, scan: &mut EventScan) {
    match s {
        SNode::Seq(_) => walk_stmts(s, path, scan),
        SNode::If { then_body, else_body, .. } => {
            let mut p = path.clone();
            p.push(Step::Then);
            walk_stmts(then_body, p, scan);
            if let Some(e) = else_body {
                let mut p = path.clone();
                p.push(Step::Else);
                walk_stmts(e, p, scan);
            }
        }
        SNode::While { body, .. } | SNode::DoWhile { body, .. } => {
            let mut p = path;
            p.push(Step::Body);
            walk_stmts(body, p, scan);
        }
        SNode::ForIn { body, .. } | SNode::ForRange { body, .. } => {
            let mut p = path;
            p.push(Step::Body);
            walk_stmts(body, p, scan);
        }
        SNode::Switch { cases, case_plan, .. } => {
            for (i, (_v, b)) in cases.iter().enumerate() {
                let mut p = path.clone();
                p.push(Step::Case(i));
                walk_stmts(b, p, scan);
            }
            if let Some(plan) = case_plan {
                for (i, (_c, b)) in plan.iter().enumerate() {
                    let mut p = path.clone();
                    p.push(Step::Plan(i));
                    walk_stmts(b, p, scan);
                }
            }
        }
        SNode::Try { body, catches, finally_body } => {
            let mut p = path.clone();
            p.push(Step::Body);
            walk_stmts(body, p, scan);
            for (i, (_v, _t, b)) in catches.iter().enumerate() {
                let mut p = path.clone();
                p.push(Step::Catch(i));
                walk_stmts(b, p, scan);
            }
            if let Some(fb) = finally_body {
                let mut p = path.clone();
                p.push(Step::Finally);
                walk_stmts(fb, p, scan);
            }
        }
        _ => {}
    }
}

fn is_pure_literal(v: &IrNode) -> bool {
    let v = deep_peel(v);
    matches!(
        v,
        IrNode::NullConst
            | IrNode::UndefinedConst
            | IrNode::BoolConst(_)
            | IrNode::IntConst(_)
            | IrNode::UIntConst(_)
            | IrNode::DoubleConst(_)
            | IrNode::StringConst(_)
    )
}

fn inlineable_source(v: &IrNode) -> Option<IrNode> {
    let v = deep_peel(v);
    match &v {
        IrNode::RegAccess { .. } => Some(v.clone()),
        IrNode::PropGet { obj, prop, .. } => {
            let obj = deep_peel(obj);
            if matches!(obj, IrNode::ImplicitThis | IrNode::This)
                && matches!(&**prop, IrNode::NameRef { .. })
            {
                return Some(v.clone());
            }
            None
        }
        _ => None,
    }
}

fn drop_stmt(root: &mut SNode, seq_path: &Path, idx: usize) {
    if let Some(SNode::Seq(stmts)) = resolve(root, seq_path) {
        if idx < stmts.len() {
            stmts[idx] = SNode::Removed;
        }
    }
}

fn stmt_at<'a>(root: &'a mut SNode, seq_path: &Path, idx: usize) -> Option<&'a SNode> {
    if let Some(SNode::Seq(stmts)) = resolve(root, seq_path) {
        return stmts.get(idx);
    }
    None
}

fn overwritten_on_every_path(
    prev: &Event,
    d: &Event,
    reads: &[Event],
) -> bool {
    if prev.seq_path == d.seq_path {
        return true;
    }
    if !prev.chain_is_prefix_of(&d.chain) {
        return false;
    }
    let loop_depth: Vec<Step> = d
        .chain
        .iter()
        .filter(|s| matches!(s, Step::Body))
        .cloned()
        .collect();
    if loop_depth.is_empty() {
        return false;
    }
    let d_pos = d.pos;
    for r in reads {
        if r.pos <= prev.pos {
            continue;
        }
        if !r.chain_contains_any(&loop_depth) {
            return false;
        }
        if r.pos <= d_pos {
            return false;
        }
        if !d.chain_is_prefix_of(&r.chain) {
            return false;
        }
    }
    true
}

impl Event {
    fn chain_is_prefix_of(&self, other: &Path) -> bool {
        if self.chain.len() > other.len() {
            return false;
        }
        self.chain.iter().zip(other.iter()).all(|(a, b)| a == b)
    }
    fn chain_contains_any(&self, steps: &[Step]) -> bool {
        self.chain.iter().any(|c| steps.contains(c))
    }
}

fn drop_dead_defs(tree: &mut SNode, _n_params: usize) {
    let scan = scan_events(tree);
    let mut drops: Vec<(Path, usize)> = Vec::new();
    for (_reg, evs) in &scan.events {
        let defs: Vec<&Event> = evs.iter().filter(|e| e.is_def && !e.implicit).collect();
        let reads: Vec<&Event> = evs.iter().filter(|e| !e.is_def).collect();
        if defs.is_empty() {
            continue;
        }
        if reads.is_empty() {
            for d in defs {
                let pure = stmt_at(tree, &d.seq_path, d.idx)
                    .and_then(|s| match s {
                        SNode::Expr(IrNode::RegSet { value, .. }) => Some(value.clone()),
                        _ => None,
                    })
                    .map(|v| is_pure_literal(&v) || inlineable_source(&v).is_some())
                    .unwrap_or(false);
                if pure {
                    drops.push((d.seq_path.clone(), d.idx));
                }
            }
            continue;
        }
        let mut timeline: Vec<(usize, usize, &Event)> = evs
            .iter()
            .filter(|e| e.is_def)
            .map(|e| (e.pos, 0usize, e as &Event))
            .chain(reads.iter().map(|r| (r.pos, 1usize, *r as &Event)))
            .collect();
        timeline.sort_by_key(|(pos, kind, _)| (*pos, *kind));
        let mut prev_def: Option<&Event> = None;
        let mut dropped: Vec<&Event> = Vec::new();
        for (_p, kind, d) in timeline {
            if kind == 0 {
                if let Some(prev) = prev_def {
                    if prev.implicit {
                        prev_def = Some(d);
                        continue;
                    }
                    let pv = stmt_at(tree, &prev.seq_path, prev.idx).and_then(|s| match s {
                        SNode::Expr(IrNode::RegSet { value, .. }) => Some(value.clone()),
                        _ => None,
                    });
                    if let Some(pv) = pv {
                        if (is_pure_literal(&pv) || inlineable_source(&pv).is_some())
                            && overwritten_on_every_path(prev, d, &reads.iter().map(|r| (**r).clone()).collect::<Vec<_>>())
                        {
                            dropped.push(prev);
                        }
                    }
                }
                prev_def = Some(d);
            } else {
                prev_def = None;
            }
        }
        for d in dropped {
            drops.push((d.seq_path.clone(), d.idx));
        }
    }
    for (path, idx) in drops {
        drop_stmt(tree, &path, idx);
    }
}

fn collapse_single_use_copies(tree: &mut SNode) {
    let scan = scan_events(tree);
    for (reg, evs) in &scan.events {
        if scan.loop_regs.contains(reg) {
            continue;
        }
        let defs: Vec<&Event> = evs.iter().filter(|e| e.is_def && !e.implicit).collect();
        let reads: Vec<&Event> = evs.iter().filter(|e| !e.is_def).collect();
        if defs.len() != 1 || reads.len() != 1 {
            continue;
        }
        let d = defs[0];
        let r = reads[0];
        let src = stmt_at(tree, &d.seq_path, d.idx).and_then(|s| match s {
            SNode::Expr(IrNode::RegSet { value, .. }) => inlineable_source(value),
            _ => None,
        });
        let Some(src) = src else { continue };
        if d.seq_path == r.seq_path && d.idx == r.idx {
            continue;
        }
        if let IrNode::RegAccess { reg: src_reg, .. } = &src {
            if let Some(sev) = scan.events.get(src_reg) {
                let conflict = sev.iter().any(|e| {
                    e.is_def && !e.implicit && e.pos > d.pos && e.pos < r.pos
                });
                if conflict {
                    continue;
                }
            }
        }
        let rd_seq = r.seq_path.clone();
        let rd_idx = r.idx;
        let reg_usize = *reg;
        if let Some(SNode::Seq(stmts)) = resolve(tree, &rd_seq) {
            if rd_idx < stmts.len() {
                let stmt = &mut stmts[rd_idx];
                subst_read_in_stmt(stmt, reg_usize, &src);
                let mut acc = HashSet::new();
                if let SNode::Expr(node) = stmt {
                    crate::ir::collect_reads(Some(node), &mut acc);
                }
                let mut acc2 = HashSet::new();
                for e in exprs_of(stmt) {
                    collect_reads_expr(Some(e), &mut acc2);
                }
                if !acc2.contains(&reg_usize) {
                    if let Some(SNode::Seq(dstmts)) = resolve(tree, &d.seq_path) {
                        if d.idx < dstmts.len() {
                            dstmts[d.idx] = SNode::Removed;
                        }
                    }
                }
                let _ = acc;
            }
        }
    }
}

fn subst_read_in_stmt(stmt: &mut SNode, reg: usize, repl: &IrNode) -> bool {
    let mut changed = false;
    match stmt {
        SNode::Expr(node) => subst_in_node(node, reg, repl, &mut changed),
        SNode::If { cond, .. } => subst_in_node(cond, reg, repl, &mut changed),
        SNode::While { cond: Some(c), .. } => subst_in_node(c, reg, repl, &mut changed),
        SNode::DoWhile { cond, .. } => subst_in_node(cond, reg, repl, &mut changed),
        SNode::ForIn { obj, .. } => subst_in_node(obj, reg, repl, &mut changed),
        SNode::ForRange { start, end, .. } => {
            subst_in_node(start, reg, repl, &mut changed);
            subst_in_node(end, reg, repl, &mut changed);
        }
        SNode::Switch { value, match_base, .. } => {
            subst_in_node(value, reg, repl, &mut changed);
            if let Some(mb) = match_base {
                subst_in_node(mb, reg, repl, &mut changed);
            }
        }
        SNode::Return(Some(v)) | SNode::Throw(v) => subst_in_node(v, reg, repl, &mut changed),
        _ => {}
    }
    changed
}

fn subst_in_node(nd: &mut IrNode, reg: usize, repl: &IrNode, changed: &mut bool) {
    match nd {
        IrNode::NewFunction { .. } | IrNode::NewClass { .. } => {}
        other => {
            other.map_children_mut(&mut |child| {
                if matches!(child, IrNode::RegAccess { reg: r, .. } if *r == reg) {
                    *child = repl.clone();
                    *changed = true;
                } else {
                    subst_in_node(child, reg, repl, changed);
                }
            });
        }
    }
}

fn infer_type(v: &IrNode) -> Option<IrNode> {
    let v = deep_peel(v);
    Some(match v {
        IrNode::IntConst(_) => IrNode::NameRef { name: "Int".into(), raw_ns: String::new() },
        IrNode::UIntConst(_) => IrNode::NameRef { name: "UInt".into(), raw_ns: String::new() },
        IrNode::DoubleConst(_) => IrNode::NameRef { name: "Float".into(), raw_ns: String::new() },
        IrNode::BoolConst(_) => IrNode::NameRef { name: "Bool".into(), raw_ns: String::new() },
        IrNode::StringConst(_) => IrNode::NameRef { name: "String".into(), raw_ns: String::new() },
        IrNode::NewArray(_) => IrNode::NameRef { name: "Array".into(), raw_ns: String::new() },
        _ => return None,
    })
}

fn peel_deep_name(ty: &IrNode) -> String {
    match ty {
        IrNode::NameRef { name, .. } => name.rsplit("::").next().unwrap_or("").to_string(),
        _ => String::new(),
    }
}

fn hoist_init(ty: &IrNode) -> IrNode {
    let n = peel_deep_name(ty);
    match n.as_str() {
        "Int" | "UInt" => IrNode::IntConst(0),
        "Float" => IrNode::DoubleConst(0.0),
        "Bool" => IrNode::BoolConst(false),
        _ => IrNode::NullConst,
    }
}

fn assign_declarations(tree: &mut SNode, n_params: usize) {
    let scan = scan_events(tree);
    let param_regs: HashSet<usize> = (1..=n_params).collect();
    let mut hoisted: Vec<SNode> = Vec::new();
    let mut inline_decls: Vec<(Path, usize, String, Option<IrNode>, IrNode)> = Vec::new();
    for (reg, evs) in &scan.events {
        if param_regs.contains(reg) || scan.loop_regs.contains(reg) {
            continue;
        }
        let defs: Vec<&Event> = evs.iter().filter(|e| e.is_def && !e.implicit).collect();
        let reads: Vec<&Event> = evs.iter().filter(|e| !e.is_def).collect();
        if defs.is_empty() || reads.is_empty() {
            continue;
        }
        let first = defs[0];
        let Some(rs) = stmt_at(tree, &first.seq_path, first.idx).and_then(|s| match s {
            SNode::Expr(IrNode::RegSet { value, hint, .. }) => {
                Some((hint.clone(), (**value).clone()))
            }
            _ => None,
        }) else {
            continue;
        };
        let (hint, val) = rs;
        let name = hint.unwrap_or_else(|| format!("_variable{}_", reg));

        let mut inline_ok = true;
        for r in &reads {
            let read_before = r.pos < first.pos;
            let cross_seq = r.seq_path != first.seq_path
                && !first.seq_path_is_ancestor_of(&r.seq_path);
            if read_before || cross_seq {
                inline_ok = false;
                break;
            }
        }
        if inline_ok {
            let ty = if matches!(deep_peel(&val), IrNode::NullConst | IrNode::UndefinedConst) {
                Some(IrNode::NameRef { name: "Dynamic".into(), raw_ns: String::new() })
            } else {
                None
            };
            inline_decls.push((first.seq_path.clone(), first.idx, name, ty, val));
        } else {
            let ty = infer_type(&val)
                .unwrap_or(IrNode::NameRef { name: "Dynamic".into(), raw_ns: String::new() });
            hoisted.push(SNode::Expr(IrNode::VarDecl {
                name,
                reg: *reg,
                ty: Some(Box::new(ty.clone())),
                init: Some(Box::new(hoist_init(&ty))),
                is_const: false,
            }));
        }
    }
    for (path, idx, name, ty, val) in inline_decls {
        if let Some(SNode::Seq(stmts)) = resolve(tree, &path) {
            if idx < stmts.len() {
                stmts[idx] = SNode::Expr(IrNode::VarDecl {
                    name,
                    reg: 0,
                    ty: ty.map(Box::new),
                    init: Some(Box::new(val)),
                    is_const: false,
                });
            }
        }
    }
    for (i, decl) in hoisted.into_iter().enumerate() {
        if let SNode::Seq(stmts) = tree {
            stmts.insert(i, decl);
        }
    }
}

impl Event {
    fn seq_path_is_ancestor_of(&self, other: &Path) -> bool {
        other.len() > self.seq_path.len()
            && other[..self.seq_path.len()] == self.seq_path[..]
    }
}
