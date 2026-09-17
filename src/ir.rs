use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub enum PhiTag {
    Plain,
    Branch { cond: IrNode, taken: bool },
}

impl PhiTag {
    pub fn polarity(&self) -> &str {
        match self {
            PhiTag::Plain => "plain",
            PhiTag::Branch { taken: true, .. } => "jump",
            PhiTag::Branch { taken: false, .. } => "fall",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum IrNode {
    Nop,
    This,
    ImplicitThis,
    NullConst,
    UndefinedConst,
    BoolConst(bool),
    IntConst(i32),
    UIntConst(u32),
    DoubleConst(f64),
    NaNConst,
    StringConst(String),
    NameRef {
        name: String,
        raw_ns: String,
    },
    PropertyLookup {
        name: String,
        is_strict: bool,
        source: String,
    },
    NamespaceRef {
        kind: String,
        name: String,
    },
    RegAccess {
        reg: usize,
        hint: Option<String>,
    },
    ParamRef(usize),
    CatchVarRef(String),
    GlobalScope,
    Scope(u32),
    UnknownValue(String),
    ExceptionValue,
    Phi {
        inputs: Vec<(PhiTag, IrNode)>,
    },

    UnaryOp {
        op: String,
        expr: Box<IrNode>,
    },
    BinaryOp {
        op: String,
        left: Box<IrNode>,
        right: Box<IrNode>,
    },
    Ternary {
        cond: Box<IrNode>,
        then_val: Box<IrNode>,
        else_val: Box<IrNode>,
    },

    Cast {
        expr: Box<IrNode>,
        target_type: Box<IrNode>,
        is_strict: bool,
    },
    Convert {
        expr: Box<IrNode>,
        target: String,
    },
    CheckXml(Box<IrNode>),

    PropGet {
        obj: Box<IrNode>,
        prop: Box<IrNode>,
        is_dynamic: bool,
    },
    NextName {
        obj: Box<IrNode>,
        prop: Box<IrNode>,
        is_dynamic: bool,
    },
    NextValue {
        obj: Box<IrNode>,
        prop: Box<IrNode>,
        is_dynamic: bool,
    },
    PropSet {
        obj: Box<IrNode>,
        prop: Box<IrNode>,
        value: Box<IrNode>,
        is_init: bool,
        is_dynamic: bool,
    },
    PropDelete {
        obj: Box<IrNode>,
        prop: Box<IrNode>,
    },
    Descendants {
        obj: Box<IrNode>,
        name: Box<IrNode>,
    },
    SlotGet {
        obj: Box<IrNode>,
        slot: u32,
    },
    SlotSet {
        obj: Box<IrNode>,
        slot: u32,
        value: Box<IrNode>,
    },

    Call {
        func: Box<IrNode>,
        args: Vec<IrNode>,
    },
    CallProp {
        obj: Box<IrNode>,
        prop: Box<IrNode>,
        args: Vec<IrNode>,
        is_super: bool,
        is_void: bool,
        is_lex: bool,
    },
    CallStatic {
        method: Box<IrNode>,
        args: Vec<IrNode>,
    },
    Construct {
        cls: Box<IrNode>,
        args: Vec<IrNode>,
    },
    ConstructProp {
        cls: Box<IrNode>,
        args: Vec<IrNode>,
    },
    ConstructSuper(Vec<IrNode>),
    CallSuper {
        prop: Box<IrNode>,
        args: Vec<IrNode>,
        is_void: bool,
    },
    ApplyType {
        base: Box<IrNode>,
        params: Vec<IrNode>,
    },

    NewObject(Vec<(IrNode, IrNode)>),
    NewArray(Vec<IrNode>),
    NewFunction {
        method_index: usize,
        name: Option<String>,
        params: Option<Vec<String>>,
        body: Option<Box<IrNode>>,
    },
    NewClass {
        class_index: usize,
        name: Option<String>,
    },
    NewActivation,
    NewCatch {
        catch_id: usize,
        type_node: Option<Box<IrNode>>,
        var_name: Option<String>,
    },

    Statement(Box<IrNode>),
    Block(Vec<IrNode>),
    ExprStmt(Box<IrNode>),
    VarDecl {
        name: String,
        reg: usize,
        ty: Option<Box<IrNode>>,
        init: Option<Box<IrNode>>,
        is_const: bool,
    },
    Return(Option<Box<IrNode>>),
    Throw(Box<IrNode>),
    IfElse {
        cond: Box<IrNode>,
        then_body: Box<IrNode>,
        else_body: Option<Box<IrNode>>,
    },
    While {
        kind: String,
        cond: Box<IrNode>,
        body: Box<IrNode>,
    },
    ForLoop {
        init: Option<Box<IrNode>>,
        cond: Option<Box<IrNode>>,
        update: Option<Box<IrNode>>,
        body: Box<IrNode>,
    },
    Switch {
        value: Box<IrNode>,
        cases: Vec<(Option<i32>, Box<IrNode>)>,
    },
    TryCatch {
        body: Box<IrNode>,
        catches: Vec<(String, String, Box<IrNode>)>,
        finally_body: Option<Box<IrNode>>,
    },
    BreakSentinel,
    ContinueSentinel,
    Label(String),
    Goto {
        target: usize,
        cond: Option<Box<IrNode>>,
    },
    PushScope(Box<IrNode>),
    PopScope,
    SetScopeValue {
        ns: Box<IrNode>,
        is_late: bool,
    },
    WithBlock {
        value: Box<IrNode>,
        body: Box<IrNode>,
    },
    NextIter {
        obj_reg: usize,
        idx_reg: usize,
        each: bool,
    },
    HasNext {
        obj_reg: usize,
        idx_reg: usize,
        prime: bool,
    },
    MemOp {
        op: String,
        args: Vec<IrNode>,
    },
    Kill(usize),
    RegSet {
        reg: usize,
        value: Box<IrNode>,
        hint: Option<String>,
    },

    DebugLine(i32),
    DebugFile(String),
    DebugReg {
        reg: usize,
        var_name: String,
        line: i32,
    },
    Dup,
    Swap,
    Pop,
}

impl IrNode {
    pub fn is_noise(&self) -> bool {
        match self {
            IrNode::PushScope(_)
            | IrNode::PopScope
            | IrNode::DebugLine(_)
            | IrNode::DebugFile(_)
            | IrNode::DebugReg { .. }
            | IrNode::Nop
            | IrNode::Label(_)
            | IrNode::Dup
            | IrNode::Swap
            | IrNode::Pop
            | IrNode::Kill(_) => true,
            _ => false,
        }
    }
}

impl IrNode {
    pub fn map_children_mut<F: FnMut(&mut IrNode)>(&mut self, f: &mut F) {
        match self {
            IrNode::UnaryOp { expr, .. } => f(expr),
            IrNode::BinaryOp { left, right, .. } => {
                f(left);
                f(right);
            }
            IrNode::Ternary {
                cond, then_val, else_val,
            } => {
                f(cond);
                f(then_val);
                f(else_val);
            }
            IrNode::Cast { expr, target_type, .. } => {
                f(expr);
                f(target_type);
            }
            IrNode::Convert { expr, .. } => f(expr),
            IrNode::CheckXml(e) => f(e),
            IrNode::PropGet { obj, prop, .. }
            | IrNode::NextName { obj, prop, .. }
            | IrNode::NextValue { obj, prop, .. }
            | IrNode::PropDelete { obj, prop }
            | IrNode::Descendants { obj, name: prop } => {
                f(obj);
                f(prop);
            }
            IrNode::PropSet { obj, prop, value, .. } => {
                f(obj);
                f(prop);
                f(value);
            }
            IrNode::SlotGet { obj, .. } => f(obj),
            IrNode::SlotSet { obj, value, .. } => {
                f(obj);
                f(value);
            }
            IrNode::Call { func, args } => {
                f(func);
                for a in args.iter_mut() {
                    f(a);
                }
            }
            IrNode::CallProp { obj, prop, args, .. } => {
                f(obj);
                f(prop);
                for a in args.iter_mut() {
                    f(a);
                }
            }
            IrNode::CallStatic { method, args } => {
                f(method);
                for a in args.iter_mut() {
                    f(a);
                }
            }
            IrNode::Construct { cls, args } => {
                f(cls);
                for a in args.iter_mut() {
                    f(a);
                }
            }
            IrNode::ConstructProp { cls, args } => {
                f(cls);
                for a in args.iter_mut() {
                    f(a);
                }
            }
            IrNode::ConstructSuper(args) => {
                for a in args.iter_mut() {
                    f(a);
                }
            }
            IrNode::CallSuper { prop, args, .. } => {
                f(prop);
                for a in args.iter_mut() {
                    f(a);
                }
            }
            IrNode::ApplyType { base, params } => {
                f(base);
                for a in params.iter_mut() {
                    f(a);
                }
            }
            IrNode::NewObject(props) => {
                for (k, v) in props.iter_mut() {
                    f(k);
                    f(v);
                }
            }
            IrNode::NewArray(items) => {
                for a in items.iter_mut() {
                    f(a);
                }
            }
            IrNode::NewFunction { body, .. } => {
                if let Some(b) = body {
                    f(b);
                }
            }
            IrNode::NewCatch { type_node, .. } => {
                if let Some(t) = type_node {
                    f(t);
                }
            }
            IrNode::Statement(e) | IrNode::ExprStmt(e) => f(e),
            IrNode::Block(stmts) => {
                for s in stmts.iter_mut() {
                    f(s);
                }
            }
            IrNode::VarDecl { ty, init, .. } => {
                if let Some(t) = ty {
                    f(t);
                }
                if let Some(i) = init {
                    f(i);
                }
            }
            IrNode::Return(v) => {
                if let Some(v) = v {
                    f(v);
                }
            }
            IrNode::Throw(v) => f(v),
            IrNode::IfElse {
                cond, then_body, else_body,
            } => {
                f(cond);
                f(then_body);
                if let Some(e) = else_body {
                    f(e);
                }
            }
            IrNode::While { cond, body, .. } => {
                f(cond);
                f(body);
            }
            IrNode::ForLoop { init, cond, update, body } => {
                if let Some(i) = init {
                    f(i);
                }
                if let Some(c) = cond {
                    f(c);
                }
                if let Some(u) = update {
                    f(u);
                }
                f(body);
            }
            IrNode::Switch { value, cases, .. } => {
                f(value);
                for (_v, body) in cases.iter_mut() {
                    f(body);
                }
            }
            IrNode::TryCatch { body, catches, finally_body } => {
                f(body);
                for (_v, _t, b) in catches.iter_mut() {
                    f(b);
                }
                if let Some(fb) = finally_body {
                    f(fb);
                }
            }
            IrNode::Goto { cond, .. } => {
                if let Some(c) = cond {
                    f(c);
                }
            }
            IrNode::PushScope(v) => f(v),
            IrNode::SetScopeValue { ns, .. } => f(ns),
            IrNode::WithBlock { value, body } => {
                f(value);
                f(body);
            }
            IrNode::MemOp { args, .. } => {
                for a in args.iter_mut() {
                    f(a);
                }
            }
            IrNode::RegSet { value, .. } => f(value),
            IrNode::Phi { inputs } => {
                for (_tag, v) in inputs.iter_mut() {
                    f(v);
                }
            }
            _ => {}
        }
    }

    pub fn for_each_child<'a, F: FnMut(&'a IrNode)>(&'a self, f: &mut F) {
        match self {
            IrNode::UnaryOp { expr, .. } => f(expr),
            IrNode::BinaryOp { left, right, .. } => {
                f(left);
                f(right);
            }
            IrNode::Ternary { cond, then_val, else_val } => {
                f(cond);
                f(then_val);
                f(else_val);
            }
            IrNode::Cast { expr, target_type, .. } => {
                f(expr);
                f(target_type);
            }
            IrNode::Convert { expr, .. } => f(expr),
            IrNode::CheckXml(e) => f(e),
            IrNode::PropGet { obj, prop, .. }
            | IrNode::NextName { obj, prop, .. }
            | IrNode::NextValue { obj, prop, .. }
            | IrNode::PropDelete { obj, prop }
            | IrNode::Descendants { obj, name: prop } => {
                f(obj);
                f(prop);
            }
            IrNode::PropSet { obj, prop, value, .. } => {
                f(obj);
                f(prop);
                f(value);
            }
            IrNode::SlotGet { obj, .. } => f(obj),
            IrNode::SlotSet { obj, value, .. } => {
                f(obj);
                f(value);
            }
            IrNode::Call { func, args } => {
                f(func);
                for a in args {
                    f(a);
                }
            }
            IrNode::CallProp { obj, prop, args, .. } => {
                f(obj);
                f(prop);
                for a in args {
                    f(a);
                }
            }
            IrNode::CallStatic { method, args } => {
                f(method);
                for a in args {
                    f(a);
                }
            }
            IrNode::Construct { cls, args } => {
                f(cls);
                for a in args {
                    f(a);
                }
            }
            IrNode::ConstructProp { cls, args } => {
                f(cls);
                for a in args {
                    f(a);
                }
            }
            IrNode::ConstructSuper(args) => {
                for a in args {
                    f(a);
                }
            }
            IrNode::CallSuper { prop, args, .. } => {
                f(prop);
                for a in args {
                    f(a);
                }
            }
            IrNode::ApplyType { base, params } => {
                f(base);
                for a in params {
                    f(a);
                }
            }
            IrNode::NewObject(props) => {
                for (k, v) in props {
                    f(k);
                    f(v);
                }
            }
            IrNode::NewArray(items) => {
                for a in items {
                    f(a);
                }
            }
            IrNode::NewFunction { body, .. } => {
                if let Some(b) = body {
                    f(b);
                }
            }
            IrNode::NewCatch { type_node, .. } => {
                if let Some(t) = type_node {
                    f(t);
                }
            }
            IrNode::Statement(e) | IrNode::ExprStmt(e) => f(e),
            IrNode::Block(stmts) => {
                for s in stmts {
                    f(s);
                }
            }
            IrNode::VarDecl { ty, init, .. } => {
                if let Some(t) = ty {
                    f(t);
                }
                if let Some(i) = init {
                    f(i);
                }
            }
            IrNode::Return(v) => {
                if let Some(v) = v {
                    f(v);
                }
            }
            IrNode::Throw(v) => f(v),
            IrNode::IfElse { cond, then_body, else_body } => {
                f(cond);
                f(then_body);
                if let Some(e) = else_body {
                    f(e);
                }
            }
            IrNode::While { cond, body, .. } => {
                f(cond);
                f(body);
            }
            IrNode::ForLoop { init, cond, update, body } => {
                if let Some(i) = init {
                    f(i);
                }
                if let Some(c) = cond {
                    f(c);
                }
                if let Some(u) = update {
                    f(u);
                }
                f(body);
            }
            IrNode::Switch { value, cases, .. } => {
                f(value);
                for (_v, body) in cases {
                    f(body);
                }
            }
            IrNode::TryCatch { body, catches, finally_body } => {
                f(body);
                for (_v, _t, b) in catches {
                    f(b);
                }
                if let Some(fb) = finally_body {
                    f(fb);
                }
            }
            IrNode::Goto { cond, .. } => {
                if let Some(c) = cond {
                    f(c);
                }
            }
            IrNode::PushScope(v) => f(v),
            IrNode::SetScopeValue { ns, .. } => f(ns),
            IrNode::WithBlock { value, body } => {
                f(value);
                f(body);
            }
            IrNode::MemOp { args, .. } => {
                for a in args {
                    f(a);
                }
            }
            IrNode::RegSet { value, .. } => f(value),
            IrNode::Phi { inputs } => {
                for (_tag, v) in inputs {
                    f(v);
                }
            }
            _ => {}
        }
    }

    pub fn walk_mut<F: FnMut(&mut IrNode)>(&mut self, f: &mut F) {
        self.map_children_mut(&mut |child| child.walk_mut(f));
        f(self);
    }
}

pub fn collect_regs(n: Option<&IrNode>, acc: &mut HashSet<usize>) {
    let Some(n) = n else { return };
    match n {
        IrNode::RegAccess { reg, .. } => {
            acc.insert(*reg);
        }
        IrNode::RegSet { reg, value, .. } => {
            acc.insert(*reg);
            collect_regs(Some(value), acc);
        }
        IrNode::HasNext { obj_reg, idx_reg, .. } => {
            acc.insert(*obj_reg);
            acc.insert(*idx_reg);
        }
        IrNode::Kill(reg) => {
            acc.insert(*reg);
        }
        _ => {
            n.for_each_child(&mut |c| collect_regs(Some(c), acc));
        }
    }
}

pub fn collect_reads(v: Option<&IrNode>, acc: &mut HashSet<usize>) {
    let Some(v) = v else { return };
    match v {
        IrNode::RegAccess { reg, .. } => {
            acc.insert(*reg);
        }
        IrNode::RegSet { value, .. } => collect_reads(Some(value), acc),
        IrNode::NewFunction { .. } | IrNode::NewClass { .. } | IrNode::NewCatch { .. } => {}
        _ => v.for_each_child(&mut |c| collect_reads(Some(c), acc)),
    }
}

pub fn substitute_reg(nd: &mut IrNode, reg: usize, value: &IrNode) {
    replace_or_recurse(nd, reg, value);
}

fn replace_or_recurse(nd: &mut IrNode, reg: usize, value: &IrNode) {
    nd.map_children_mut(&mut |child| {
        if matches!(child, IrNode::RegAccess { reg: r, .. } if *r == reg) {
            *child = value.clone();
        } else if matches!(child, IrNode::RegSet { reg: r, .. } if *r == reg) {
            replace_or_recurse(child, reg, value);
        } else {
            replace_or_recurse(child, reg, value);
        }
    });
}

pub fn peel_implicit(node: &IrNode) -> IrNode {
    let mut cur = node.clone();
    loop {
        match cur {
            IrNode::Cast { expr, .. } | IrNode::Convert { expr, .. } => cur = *expr,
            _ => return cur,
        }
    }
}

pub fn peel(v: &IrNode) -> IrNode {
    let mut cur = v.clone();
    loop {
        match cur {
            IrNode::Convert { expr, .. } => cur = *expr,
            IrNode::Cast { expr, is_strict: false, .. } => cur = *expr,
            _ => return cur,
        }
    }
}

pub fn peel_cast(node: &IrNode) -> IrNode {
    let mut cur = node.clone();
    loop {
        match cur {
            IrNode::Cast { expr, .. } => cur = *expr,
            _ => return cur,
        }
    }
}

pub fn unwrap_reg(v: &IrNode) -> Option<usize> {
    let mut cur = v.clone();
    loop {
        match cur {
            IrNode::Convert { expr, .. } => cur = *expr,
            IrNode::Cast { expr, is_strict: false, .. } => cur = *expr,
            IrNode::RegAccess { reg, .. } => return Some(reg),
            _ => return None,
        }
    }
}
