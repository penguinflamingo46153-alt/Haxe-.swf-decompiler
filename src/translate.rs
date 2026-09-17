use std::collections::{HashMap, HashSet, VecDeque};
use crate::abc::{AbcFile, ExceptionInfo, Instruction, MethodBody, MnKind};
use crate::ir::{peel, IrNode, PhiTag};

pub const TERMINATORS: &[&str] = &["jmp", "returnvoid", "returnvalue", "throw", "lookupswitch"];
pub const COND_JUMPS: &[&str] = &[
    "iftrue", "iffalse", "ifeq", "ifne", "iflt", "ifle", "ifgt", "ifge", "ifnlt", "ifnle",
    "ifngt", "ifnge", "ifstricteq", "ifstrictne",
];

pub fn jump_target(ins: &Instruction) -> usize {
    if ins.operands.is_empty() {
        return ins.offset + ins.length;
    }
    let delta = ins.operands[0];
    ((ins.offset + ins.length) as i64 + delta) as usize
}

#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub start_pc: usize,
    pub end_pc: usize,
    pub instrs: Vec<(usize, IrNode)>,
    pub successors: Vec<usize>,
    pub predecessors: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct CfgBlock {
    pub start_pc: usize,
    pub end_pc: usize,
    pub instrs: Vec<Instruction>,
    pub nodes: Vec<(usize, IrNode)>,
    pub successors: Vec<usize>,
    pub predecessors: Vec<usize>,
    pub succ_tags: Vec<(usize, String)>,
    pub entry_stack: Vec<IrNode>,
    pub term_kind: String,
    pub term_node: Option<IrNode>,
    pub term_fall: Option<usize>,
    pub exc_starts: Vec<ExceptionInfo>,
}

pub struct StackSimulator<'a> {
    pub abc: &'a AbcFile,
    pub body: &'a MethodBody,
    pub stack: Vec<IrNode>,
    pub reg_names: HashMap<usize, String>,
    pub reg_types: HashMap<usize, Option<IrNode>>,
    pub act_slot_names: HashMap<usize, HashMap<u32, String>>,
    pub method_name: String,
    pub temp_counter: usize,
}

impl<'a> StackSimulator<'a> {
    pub fn new(abc: &'a AbcFile, body: &'a MethodBody) -> Self {
        let mut reg_names = HashMap::new();
        let mut reg_types = HashMap::new();
        let mut act_slot_names = HashMap::new();
        act_slot_names.insert(1, HashMap::new());

        for t in &body.traits {
            if t.slot_id > 0 && t.name_idx > 0 {
                let nm = if t.name_idx > 0 && ((t.name_idx - 1) as usize) < abc.multinames.len() {
                    abc.multinames[(t.name_idx - 1) as usize].resolve(abc)
                } else {
                    format!("slot{}", t.slot_id)
                };
                if nm.chars().all(|c| c.is_alphanumeric() || c == '_') && nm != "slot" {
                    act_slot_names.entry(1).or_default().insert(t.slot_id, nm);
                }
            }
        }

        let mut method_name = "<method>".to_string();
        if let Some(m) = abc.methods.get(body.method_idx) {
            for (i, &pt) in m.params.iter().enumerate() {
                let r = i + 1;
                let name = if !m.param_names.is_empty()
                    && i < m.param_names.len()
                    && m.param_names[i] > 0
                    && ((m.param_names[i] - 1) as usize) < abc.strings.len()
                {
                    abc.strings[(m.param_names[i] - 1) as usize].clone()
                } else {
                    format!("_param{}_", i + 1)
                };
                reg_names.insert(r, name);
                let ty_node = if pt > 0 && ((pt - 1) as usize) < abc.multinames.len() {
                    Some(IrNode::NameRef {
                        name: abc.multinames[(pt - 1) as usize].resolve(abc),
                        raw_ns: String::new(),
                    })
                } else {
                    None
                };
                reg_types.insert(r, ty_node);
            }

            if m.debug_name_idx > 0 {
                reg_names.insert(0, "this".to_string());
                if ((m.debug_name_idx - 1) as usize) < abc.strings.len() {
                    method_name = abc.strings[(m.debug_name_idx - 1) as usize].clone();
                }
            } else {
                method_name = format!("method#{}", m.idx);
            }

            let mut next_reg = m.params.len() + 1;
            if m.need_activation() {
                next_reg += 1;
            }
            for t in &body.traits {
                let nm = if t.name_idx > 0 && ((t.name_idx - 1) as usize) < abc.multinames.len() {
                    abc.multinames[(t.name_idx - 1) as usize].resolve(abc)
                } else {
                    format!("slot{}", t.slot_id)
                };
                reg_names.insert(next_reg, nm);
                let ty_node = if t.type_idx > 0 && ((t.type_idx - 1) as usize) < abc.multinames.len() {
                    Some(IrNode::NameRef {
                        name: abc.multinames[(t.type_idx - 1) as usize].resolve(abc),
                        raw_ns: String::new(),
                    })
                } else {
                    None
                };
                reg_types.insert(next_reg, ty_node);
                next_reg += 1;
            }
        }

        for ins in &body.instructions {
            if ins.name == "debug" && ins.operands.len() >= 4 && ins.operands[0] == 1 {
                let name_idx = ins.operands[1] as usize;
                let reg = ins.operands[2] as usize;
                if name_idx > 0 && name_idx <= abc.strings.len() && !reg_names.contains_key(&reg) {
                    reg_names.insert(reg, abc.strings[name_idx - 1].clone());
                }
            }
        }

        Self {
            abc,
            body,
            stack: Vec::new(),
            reg_names,
            reg_types,
            act_slot_names,
            method_name,
            temp_counter: 0,
        }
    }

    pub fn push(&mut self, n: IrNode) {
        self.stack.push(n);
    }

    pub fn pop(&mut self) -> IrNode {
        self.stack.pop().unwrap_or(IrNode::NullConst)
    }

    pub fn popn(&mut self, n: usize) -> Vec<IrNode> {
        let mut res = Vec::with_capacity(n);
        for _ in 0..n {
            res.push(self.pop());
        }
        res.reverse();
        res
    }

    pub fn peek(&self) -> IrNode {
        self.stack.last().cloned().unwrap_or(IrNode::NullConst)
    }

    pub fn resolve_name(&self, idx: u32) -> IrNode {
        if idx == 0 || ((idx - 1) as usize) >= self.abc.multinames.len() {
            return IrNode::NameRef {
                name: "*".to_string(),
                raw_ns: String::new(),
            };
        }
        let mn = &self.abc.multinames[(idx - 1) as usize];
        let name = mn.resolve(self.abc);
        IrNode::NameRef {
            name,
            raw_ns: String::new(),
        }
    }

    pub fn alloc_temp(&mut self) -> usize {
        self.temp_counter += 1;
        10000 + self.temp_counter
    }

    pub fn read_reg(&self, r: usize) -> IrNode {
        let hint = self.reg_names.get(&r).cloned();
        IrNode::RegAccess { reg: r, hint }
    }

    pub fn write_reg(&self, r: usize, v: IrNode) -> IrNode {
        let hint = self.reg_names.get(&r).cloned();
        IrNode::RegSet {
            reg: r,
            value: Box::new(v),
            hint,
        }
    }

    pub fn scope_slot_name(&self, obj: &IrNode, slot: u32) -> Option<String> {
        if let IrNode::Scope(depth) = obj {
            return self
                .act_slot_names
                .get(&(*depth as usize))
                .and_then(|m| m.get(&slot).cloned());
        }
        None
    }

    pub fn slot_fake_reg(&self, depth: u32, slot: u32) -> usize {
        (100000 + depth * 1000 + slot) as usize
    }

    pub fn sim_instr(
        &mut self,
        ins: &Instruction,
        targets: &mut HashSet<usize>,
        pc: usize,
    ) -> IrNode {
        let name = ins.name;
        let ops = &ins.operands;

        match name {
            "pushbyte" => {
                let v = ops.get(0).copied().unwrap_or(0) as i32;
                self.push(IrNode::IntConst(v));
                IrNode::Nop
            }
            "pushshort" => {
                let v = ops.get(0).copied().unwrap_or(0) as i32;
                self.push(IrNode::IntConst(v));
                IrNode::Nop
            }
            "pushint" => {
                let idx = ops.get(0).copied().unwrap_or(0) as usize;
                let v = if idx > 0 && idx <= self.abc.ints.len() {
                    self.abc.ints[idx - 1]
                } else {
                    0
                };
                self.push(IrNode::IntConst(v));
                IrNode::Nop
            }
            "pushuint" => {
                let idx = ops.get(0).copied().unwrap_or(0) as usize;
                let v = if idx > 0 && idx <= self.abc.uints.len() {
                    self.abc.uints[idx - 1]
                } else {
                    0
                };
                self.push(IrNode::UIntConst(v));
                IrNode::Nop
            }
            "pushdouble" => {
                let idx = ops.get(0).copied().unwrap_or(0) as usize;
                let v = if idx > 0 && idx <= self.abc.doubles.len() {
                    self.abc.doubles[idx - 1]
                } else {
                    0.0
                };
                self.push(IrNode::DoubleConst(v));
                IrNode::Nop
            }
            "pushstring" => {
                let idx = ops.get(0).copied().unwrap_or(0) as usize;
                let s = if idx > 0 && idx <= self.abc.strings.len() {
                    self.abc.strings[idx - 1].clone()
                } else {
                    "".to_string()
                };
                self.push(IrNode::StringConst(s));
                IrNode::Nop
            }
            "pushnull" => {
                self.push(IrNode::NullConst);
                IrNode::Nop
            }
            "pushundefined" => {
                self.push(IrNode::UndefinedConst);
                IrNode::Nop
            }
            "pushtrue" => {
                self.push(IrNode::BoolConst(true));
                IrNode::Nop
            }
            "pushfalse" => {
                self.push(IrNode::BoolConst(false));
                IrNode::Nop
            }
            "pushnan" => {
                self.push(IrNode::NaNConst);
                IrNode::Nop
            }
            "pushnamespace" => {
                let idx = ops.get(0).copied().unwrap_or(0) as u32;
                let (k, nm) = if idx > 0 && ((idx - 1) as usize) < self.abc.namespaces.len() {
                    let ns = &self.abc.namespaces[(idx - 1) as usize];
                    (format!("{:?}", ns.kind), ns.resolve(self.abc))
                } else {
                    ("".to_string(), "".to_string())
                };
                self.push(IrNode::NamespaceRef { kind: k, name: nm });
                IrNode::Nop
            }
            "pushscope" => {
                let v = self.pop();
                IrNode::PushScope(Box::new(v))
            }
            "popscope" => IrNode::PopScope,
            "pop" => {
                let _v = self.pop();
                IrNode::Pop
            }
            "newactivation" => {
                self.push(IrNode::NewActivation);
                IrNode::Nop
            }
            "pushwith" => {
                let v = self.pop();
                IrNode::ExprStmt(Box::new(IrNode::PushScope(Box::new(v))))
            }
            "dxnslate" => {
                let v = self.pop();
                IrNode::SetScopeValue {
                    ns: Box::new(v),
                    is_late: true,
                }
            }
            "dup" => {
                let v = self.peek();
                self.push(v);
                IrNode::Nop
            }
            "swap" => {
                let b = self.pop();
                let a = self.pop();
                self.push(b);
                self.push(a);
                IrNode::Nop
            }
            "getlocal0" => {
                let r = self.read_reg(0);
                self.push(r);
                IrNode::Nop
            }
            "getlocal1" => {
                let r = self.read_reg(1);
                self.push(r);
                IrNode::Nop
            }
            "getlocal2" => {
                let r = self.read_reg(2);
                self.push(r);
                IrNode::Nop
            }
            "getlocal3" => {
                let r = self.read_reg(3);
                self.push(r);
                IrNode::Nop
            }
            "getlocal" => {
                let r = ops.get(0).copied().unwrap_or(0) as usize;
                let reg = self.read_reg(r);
                self.push(reg);
                IrNode::Nop
            }
            "setlocal0" => {
                let v = self.pop();
                self.write_reg(0, v)
            }
            "setlocal1" => {
                let v = self.pop();
                self.write_reg(1, v)
            }
            "setlocal2" => {
                let v = self.pop();
                self.write_reg(2, v)
            }
            "setlocal3" => {
                let v = self.pop();
                self.write_reg(3, v)
            }
            "setlocal" => {
                let r = ops.get(0).copied().unwrap_or(0) as usize;
                let v = self.pop();
                self.write_reg(r, v)
            }
            "kill" => {
                let r = ops.get(0).copied().unwrap_or(0) as usize;
                IrNode::Kill(r)
            }
            "inclocal" | "inclocal_i" | "declocal" | "declocal_i" => {
                let r = ops.get(0).copied().unwrap_or(0) as usize;
                let op = match name {
                    "inclocal" => "increment",
                    "declocal" => "decrement",
                    "inclocal_i" => "increment_i",
                    "declocal_i" => "decrement_i",
                    _ => "increment",
                };
                let top = self.stack.last().cloned();
                if let Some(top) = top {
                    let inner = peel(&top);
                    if let IrNode::RegAccess { reg: ir, .. } = inner {
                        if ir == r {
                            let tmp = self.alloc_temp();
                            let tname = format!("__t{}", tmp - 10000);
                            if let Some(slot) = self.stack.last_mut() {
                                *slot = IrNode::RegAccess {
                                    reg: tmp,
                                    hint: Some(tname.clone()),
                                };
                            }
                            let snap = IrNode::RegSet {
                                reg: tmp,
                                value: Box::new(top),
                                hint: Some(tname),
                            };
                            let inc = self.write_reg(
                                r,
                                IrNode::UnaryOp {
                                    op: op.to_string(),
                                    expr: Box::new(IrNode::RegAccess { reg: r, hint: None }),
                                },
                            );
                            return IrNode::Block(vec![snap, inc]);
                        }
                    }
                }
                let reg = self.read_reg(r);
                self.write_reg(
                    r,
                    IrNode::UnaryOp {
                        op: op.to_string(),
                        expr: Box::new(reg),
                    },
                )
            }
            "getglobalscope" => {
                self.push(IrNode::GlobalScope);
                IrNode::Nop
            }
            "getscope" => {
                let d = ops.get(0).copied().unwrap_or(0) as u32;
                self.push(IrNode::Scope(d));
                IrNode::Nop
            }
            "findproperty" | "findpropstrict" | "finddef" => {
                let idx = ops.get(0).copied().unwrap_or(0) as u32;
                let nm = if idx > 0 && ((idx - 1) as usize) < self.abc.multinames.len() {
                    self.abc.multinames[(idx - 1) as usize].resolve(self.abc)
                } else {
                    "*".to_string()
                };
                let strict = name == "findpropstrict";
                self.push(IrNode::PropertyLookup {
                    name: nm,
                    is_strict: strict,
                    source: "scope".to_string(),
                });
                IrNode::Nop
            }
            "getlex" => {
                let idx = ops.get(0).copied().unwrap_or(0) as u32;
                let nm = if idx > 0 && ((idx - 1) as usize) < self.abc.multinames.len() {
                    self.abc.multinames[(idx - 1) as usize].resolve(self.abc)
                } else {
                    "*".to_string()
                };
                self.push(IrNode::PropertyLookup {
                    name: nm,
                    is_strict: true,
                    source: "global".to_string(),
                });
                IrNode::Nop
            }
            "getdescendants" => {
                let idx = ops.get(0).copied().unwrap_or(0) as u32;
                let n = self.resolve_name(idx);
                let obj = self.pop();
                self.push(IrNode::Descendants {
                    obj: Box::new(obj),
                    name: Box::new(n),
                });
                IrNode::Nop
            }
            "getproperty" => {
                let idx = ops.get(0).copied().unwrap_or(0) as u32;
                let is_late = if idx > 0 && ((idx - 1) as usize) < self.abc.multinames.len() {
                    matches!(
                        self.abc.multinames[(idx - 1) as usize].kind,
                        MnKind::MultinameLate | MnKind::RTQNameLate | MnKind::MultinameLateA
                    )
                } else {
                    false
                };
                if is_late {
                    let key = self.pop();
                    let base = self.pop();
                    self.push(IrNode::PropGet {
                        obj: Box::new(base),
                        prop: Box::new(key),
                        is_dynamic: true,
                    });
                } else {
                    let name_node = self.resolve_name(idx);
                    let base = self.pop();
                    if let IrNode::PropertyLookup {
                        ref name,
                        ref source,
                        ..
                    } = base
                    {
                        if source == "scope" {
                            self.push(IrNode::PropGet {
                                obj: Box::new(IrNode::ImplicitThis),
                                prop: Box::new(IrNode::NameRef {
                                    name: name.clone(),
                                    raw_ns: String::new(),
                                }),
                                is_dynamic: false,
                            });
                        } else {
                            self.push(IrNode::PropGet {
                                obj: Box::new(IrNode::NameRef {
                                    name: name.clone(),
                                    raw_ns: String::new(),
                                }),
                                prop: Box::new(name_node),
                                is_dynamic: false,
                            });
                        }
                    } else {
                        self.push(IrNode::PropGet {
                            obj: Box::new(base),
                            prop: Box::new(name_node),
                            is_dynamic: false,
                        });
                    }
                }
                IrNode::Nop
            }
            "setproperty" => {
                let idx = ops.get(0).copied().unwrap_or(0) as u32;
                let is_late = if idx > 0 && ((idx - 1) as usize) < self.abc.multinames.len() {
                    matches!(
                        self.abc.multinames[(idx - 1) as usize].kind,
                        MnKind::MultinameLate | MnKind::RTQNameLate | MnKind::MultinameLateA
                    )
                } else {
                    false
                };
                if is_late {
                    let val = self.pop();
                    let key = self.pop();
                    let base = self.pop();
                    IrNode::ExprStmt(Box::new(IrNode::PropSet {
                        obj: Box::new(base),
                        prop: Box::new(key),
                        value: Box::new(val),
                        is_init: false,
                        is_dynamic: true,
                    }))
                } else {
                    let name_node = self.resolve_name(idx);
                    let val = self.pop();
                    let base = self.pop();
                    if let IrNode::PropertyLookup {
                        ref name,
                        ref source,
                        ..
                    } = base
                    {
                        let recv = if source == "global" {
                            IrNode::NameRef {
                                name: name.clone(),
                                raw_ns: String::new(),
                            }
                        } else {
                            IrNode::ImplicitThis
                        };
                        IrNode::ExprStmt(Box::new(IrNode::PropSet {
                            obj: Box::new(recv),
                            prop: Box::new(name_node),
                            value: Box::new(val),
                            is_init: false,
                            is_dynamic: false,
                        }))
                    } else {
                        IrNode::ExprStmt(Box::new(IrNode::PropSet {
                            obj: Box::new(base),
                            prop: Box::new(name_node),
                            value: Box::new(val),
                            is_init: false,
                            is_dynamic: false,
                        }))
                    }
                }
            }
            "initproperty" => {
                let idx = ops.get(0).copied().unwrap_or(0) as u32;
                let name_node = self.resolve_name(idx);
                let val = self.pop();
                let base = self.pop();
                if let IrNode::PropertyLookup {
                    ref name,
                    ref source,
                    ..
                } = base
                {
                    let recv = if source == "global" {
                        IrNode::NameRef {
                            name: name.clone(),
                            raw_ns: String::new(),
                        }
                    } else {
                        IrNode::ImplicitThis
                    };
                    IrNode::ExprStmt(Box::new(IrNode::PropSet {
                        obj: Box::new(recv),
                        prop: Box::new(name_node),
                        value: Box::new(val),
                        is_init: true,
                        is_dynamic: false,
                    }))
                } else {
                    IrNode::ExprStmt(Box::new(IrNode::PropSet {
                        obj: Box::new(base),
                        prop: Box::new(name_node),
                        value: Box::new(val),
                        is_init: true,
                        is_dynamic: false,
                    }))
                }
            }
            "deleteproperty" => {
                let idx = ops.get(0).copied().unwrap_or(0) as u32;
                let name_node = self.resolve_name(idx);
                let obj = self.pop();
                self.push(IrNode::PropDelete {
                    obj: Box::new(obj),
                    prop: Box::new(name_node),
                });
                IrNode::Nop
            }
            "getsuper" => {
                let idx = ops.get(0).copied().unwrap_or(0) as u32;
                let name_node = self.resolve_name(idx);
                let obj = self.pop();
                self.push(IrNode::PropGet {
                    obj: Box::new(obj),
                    prop: Box::new(name_node),
                    is_dynamic: false,
                });
                IrNode::Nop
            }
            "setsuper" => {
                let idx = ops.get(0).copied().unwrap_or(0) as u32;
                let name_node = self.resolve_name(idx);
                let val = self.pop();
                let obj = self.pop();
                IrNode::ExprStmt(Box::new(IrNode::PropSet {
                    obj: Box::new(obj),
                    prop: Box::new(name_node),
                    value: Box::new(val),
                    is_init: false,
                    is_dynamic: false,
                }))
            }
            "getslot" => {
                let s = ops.get(0).copied().unwrap_or(0) as u32;
                let obj = self.pop();
                if let Some(nm) = self.scope_slot_name(&obj, s) {
                    let depth = if let IrNode::Scope(d) = obj { d } else { 0 };
                    self.push(IrNode::RegAccess {
                        reg: self.slot_fake_reg(depth, s),
                        hint: Some(nm),
                    });
                } else {
                    self.push(IrNode::SlotGet {
                        obj: Box::new(obj),
                        slot: s,
                    });
                }
                IrNode::Nop
            }
            "setslot" => {
                let s = ops.get(0).copied().unwrap_or(0) as u32;
                let val = self.pop();
                let obj = self.pop();
                if let Some(nm) = self.scope_slot_name(&obj, s) {
                    let depth = if let IrNode::Scope(d) = obj { d } else { 0 };
                    IrNode::ExprStmt(Box::new(IrNode::RegSet {
                        reg: self.slot_fake_reg(depth, s),
                        value: Box::new(val),
                        hint: Some(nm),
                    }))
                } else {
                    IrNode::ExprStmt(Box::new(IrNode::SlotSet {
                        obj: Box::new(obj),
                        slot: s,
                        value: Box::new(val),
                    }))
                }
            }
            "newcatch" => {
                let cid = ops.get(0).copied().unwrap_or(0) as usize;
                self.push(IrNode::NewCatch {
                    catch_id: cid,
                    type_node: None,
                    var_name: None,
                });
                IrNode::Nop
            }
            "getglobalslot" => {
                let s = ops.get(0).copied().unwrap_or(0) as u32;
                self.push(IrNode::SlotGet {
                    obj: Box::new(IrNode::GlobalScope),
                    slot: s,
                });
                IrNode::Nop
            }
            "setglobalslot" => {
                let s = ops.get(0).copied().unwrap_or(0) as u32;
                let val = self.pop();
                IrNode::ExprStmt(Box::new(IrNode::SlotSet {
                    obj: Box::new(IrNode::GlobalScope),
                    slot: s,
                    value: Box::new(val),
                }))
            }
            "call" => {
                let nargs = ops.get(0).copied().unwrap_or(0) as usize;
                let args = self.popn(nargs);
                let _obj = self.pop();
                let func = self.pop();
                self.push(IrNode::Call {
                    func: Box::new(func),
                    args,
                });
                IrNode::Nop
            }
            "callproperty" | "callproplex" => {
                let idx = ops.get(0).copied().unwrap_or(0) as u32;
                let nargs = ops.get(1).copied().unwrap_or(0) as usize;
                let name_node = self.resolve_name(idx);
                let args = self.popn(nargs);
                let base = self.pop();
                let is_lex = name == "callproplex";
                if let IrNode::PropertyLookup {
                    ref name,
                    ref source,
                    ..
                } = base
                {
                    let recv = if source == "global" {
                        IrNode::NameRef {
                            name: name.clone(),
                            raw_ns: String::new(),
                        }
                    } else {
                        IrNode::ImplicitThis
                    };
                    self.push(IrNode::CallProp {
                        obj: Box::new(recv),
                        prop: Box::new(name_node),
                        args,
                        is_super: false,
                        is_void: false,
                        is_lex,
                    });
                } else {
                    self.push(IrNode::CallProp {
                        obj: Box::new(base),
                        prop: Box::new(name_node),
                        args,
                        is_super: false,
                        is_void: false,
                        is_lex,
                    });
                }
                IrNode::Nop
            }
            "callpropvoid" => {
                let idx = ops.get(0).copied().unwrap_or(0) as u32;
                let nargs = ops.get(1).copied().unwrap_or(0) as usize;
                let name_node = self.resolve_name(idx);
                let args = self.popn(nargs);
                let base = self.pop();
                if let IrNode::PropertyLookup {
                    ref name,
                    ref source,
                    ..
                } = base
                {
                    let recv = if source == "global" {
                        IrNode::NameRef {
                            name: name.clone(),
                            raw_ns: String::new(),
                        }
                    } else {
                        IrNode::ImplicitThis
                    };
                    IrNode::ExprStmt(Box::new(IrNode::CallProp {
                        obj: Box::new(recv),
                        prop: Box::new(name_node),
                        args,
                        is_super: false,
                        is_void: true,
                        is_lex: false,
                    }))
                } else {
                    IrNode::ExprStmt(Box::new(IrNode::CallProp {
                        obj: Box::new(base),
                        prop: Box::new(name_node),
                        args,
                        is_super: false,
                        is_void: true,
                        is_lex: false,
                    }))
                }
            }
            "callsuper" => {
                let idx = ops.get(0).copied().unwrap_or(0) as u32;
                let nargs = ops.get(1).copied().unwrap_or(0) as usize;
                let name_node = self.resolve_name(idx);
                let args = self.popn(nargs);
                let _obj = self.pop();
                self.push(IrNode::CallSuper {
                    prop: Box::new(name_node),
                    args,
                    is_void: false,
                });
                IrNode::Nop
            }
            "callsupervoid" => {
                let idx = ops.get(0).copied().unwrap_or(0) as u32;
                let nargs = ops.get(1).copied().unwrap_or(0) as usize;
                let name_node = self.resolve_name(idx);
                let args = self.popn(nargs);
                let _obj = self.pop();
                IrNode::ExprStmt(Box::new(IrNode::CallSuper {
                    prop: Box::new(name_node),
                    args,
                    is_void: true,
                }))
            }
            "callstatic" => {
                let midx = ops.get(0).copied().unwrap_or(0) as usize;
                let nargs = ops.get(1).copied().unwrap_or(0) as usize;
                let args = self.popn(nargs);
                self.push(IrNode::CallStatic {
                    method: Box::new(IrNode::NameRef {
                        name: format!("static#{}", midx),
                        raw_ns: String::new(),
                    }),
                    args,
                });
                IrNode::Nop
            }
            "construct" => {
                let nargs = ops.get(0).copied().unwrap_or(0) as usize;
                let args = self.popn(nargs);
                let mut cls = self.pop();
                if let IrNode::PropertyLookup { name, .. } = cls {
                    cls = IrNode::NameRef {
                        name,
                        raw_ns: String::new(),
                    };
                }
                self.push(IrNode::Construct {
                    cls: Box::new(cls),
                    args,
                });
                IrNode::Nop
            }
            "constructprop" => {
                let idx = ops.get(0).copied().unwrap_or(0) as u32;
                let nargs = ops.get(1).copied().unwrap_or(0) as usize;
                let _name_node = self.resolve_name(idx);
                let args = self.popn(nargs);
                let base = self.pop();
                if let IrNode::PropertyLookup { name, .. } = base {
                    self.push(IrNode::Construct {
                        cls: Box::new(IrNode::NameRef {
                            name,
                            raw_ns: String::new(),
                        }),
                        args,
                    });
                } else {
                    self.push(IrNode::ConstructProp {
                        cls: Box::new(base),
                        args,
                    });
                }
                IrNode::Nop
            }
            "constructsuper" => {
                let nargs = ops.get(0).copied().unwrap_or(0) as usize;
                let args = self.popn(nargs);
                let _ = self.pop();
                IrNode::ExprStmt(Box::new(IrNode::ConstructSuper(args)))
            }
            "applytype" => {
                let n = ops.get(0).copied().unwrap_or(0) as usize;
                let params = self.popn(n);
                let base = self.pop();
                self.push(IrNode::ApplyType {
                    base: Box::new(base),
                    params,
                });
                IrNode::Nop
            }
            "newobject" => {
                let n = ops.get(0).copied().unwrap_or(0) as usize;
                let mut props = Vec::with_capacity(n);
                for _ in 0..n {
                    let v = self.pop();
                    let k = self.pop();
                    props.push((k, v));
                }
                props.reverse();
                self.push(IrNode::NewObject(props));
                IrNode::Nop
            }
            "newarray" => {
                let n = ops.get(0).copied().unwrap_or(0) as usize;
                let items = self.popn(n);
                self.push(IrNode::NewArray(items));
                IrNode::Nop
            }
            "newfunction" => {
                let midx = ops.get(0).copied().unwrap_or(0) as usize;
                self.push(IrNode::NewFunction {
                    method_index: midx,
                    name: None,
                    params: None,
                    body: None,
                });
                IrNode::Nop
            }
            "newclass" => {
                let cidx = ops.get(0).copied().unwrap_or(0) as usize;
                let _ = self.pop();
                self.push(IrNode::NewClass {
                    class_index: cidx,
                    name: None,
                });
                IrNode::Nop
            }
            "hasnext2" => {
                let obj_reg = ops.get(0).copied().unwrap_or(0) as usize;
                let idx_reg = ops.get(1).copied().unwrap_or(0) as usize;
                self.push(IrNode::HasNext {
                    obj_reg,
                    idx_reg,
                    prime: false,
                });
                IrNode::Nop
            }
            "nextname" => {
                let idx = self.pop();
                let obj = self.pop();
                self.push(IrNode::NextName {
                    obj: Box::new(obj),
                    prop: Box::new(idx),
                    is_dynamic: true,
                });
                IrNode::Nop
            }
            "nextvalue" => {
                let idx = self.pop();
                let obj = self.pop();
                self.push(IrNode::NextValue {
                    obj: Box::new(obj),
                    prop: Box::new(idx),
                    is_dynamic: true,
                });
                IrNode::Nop
            }
            "hasnext" => {
                let _idx = self.pop();
                let obj = self.pop();
                self.push(IrNode::UnaryOp {
                    op: "hasnext".to_string(),
                    expr: Box::new(obj),
                });
                IrNode::Nop
            }
            "astype" | "astypelate" => {
                let is_late = name == "astypelate";
                let (t, v) = if is_late {
                    let ty = self.pop();
                    let val = self.pop();
                    (ty, val)
                } else {
                    let idx = ops.get(0).copied().unwrap_or(0) as u32;
                    (self.resolve_name(idx), self.pop())
                };
                self.push(IrNode::Cast {
                    expr: Box::new(v),
                    target_type: Box::new(t),
                    is_strict: true,
                });
                IrNode::Nop
            }
            "coerce" => {
                let idx = ops.get(0).copied().unwrap_or(0) as u32;
                let t = self.resolve_name(idx);
                let v = self.pop();
                self.push(IrNode::Cast {
                    expr: Box::new(v),
                    target_type: Box::new(t),
                    is_strict: false,
                });
                IrNode::Nop
            }
            "istype" | "istypelate" => {
                let is_late = name == "istypelate";
                let (t, v) = if is_late {
                    let ty = self.pop();
                    let val = self.pop();
                    (ty, val)
                } else {
                    let idx = ops.get(0).copied().unwrap_or(0) as u32;
                    (self.resolve_name(idx), self.pop())
                };
                self.push(IrNode::BinaryOp {
                    op: "is".to_string(),
                    left: Box::new(v),
                    right: Box::new(t),
                });
                IrNode::Nop
            }
            "coerce_a" | "coerce_o" | "convert_o" | "coerce_s" | "convert_s" | "convert_i"
            | "convert_u" | "convert_d" | "convert_b" | "esc_xelem" | "esc_xattr" => {
                let target = match name {
                    "coerce_a" => "*",
                    "coerce_o" | "convert_o" => "Object",
                    "coerce_s" | "convert_s" => "String",
                    "convert_i" => "Int",
                    "convert_u" => "UInt",
                    "convert_d" => "Number",
                    "convert_b" => "Boolean",
                    "esc_xelem" => "XML",
                    "esc_xattr" => "XMLAttr",
                    _ => "*",
                };
                let v = self.pop();
                self.push(IrNode::Convert {
                    expr: Box::new(v),
                    target: target.to_string(),
                });
                IrNode::Nop
            }
            "checkfilter" => {
                let v = self.pop();
                self.push(IrNode::CheckXml(Box::new(v)));
                IrNode::Nop
            }
            "negate" | "not" | "bitnot" | "increment" | "decrement" | "typeof" | "negate_i"
            | "increment_i" | "decrement_i" => {
                let op = match name {
                    "negate" => "neg",
                    "not" => "not",
                    "bitnot" => "bitnot",
                    "increment" => "increment",
                    "decrement" => "decrement",
                    "typeof" => "typeof",
                    "negate_i" => "neg_i",
                    "increment_i" => "increment_i",
                    "decrement_i" => "decrement_i",
                    _ => "not",
                };
                let v = self.pop();
                self.push(IrNode::UnaryOp {
                    op: op.to_string(),
                    expr: Box::new(v),
                });
                IrNode::Nop
            }
            "add" | "subtract" | "multiply" | "divide" | "modulo" | "lshift" | "rshift"
            | "urshift" | "bitand" | "bitor" | "bitxor" | "equals" | "strictequals" | "lessthan"
            | "lessequals" | "greaterthan" | "greaterequals" | "add_i" | "subtract_i"
            | "multiply_i" | "instanceof" | "in" => {
                let op = match name {
                    "add" => "add",
                    "subtract" => "sub",
                    "multiply" => "mul",
                    "divide" => "div",
                    "modulo" => "mod",
                    "lshift" => "shl",
                    "rshift" => "shr",
                    "urshift" => "ushr",
                    "bitand" => "and",
                    "bitor" => "or",
                    "bitxor" => "xor",
                    "equals" => "eq",
                    "strictequals" => "stricteq",
                    "lessthan" => "lt",
                    "lessequals" => "lte",
                    "greaterthan" => "gt",
                    "greaterequals" => "gte",
                    "add_i" => "add_i",
                    "subtract_i" => "sub_i",
                    "multiply_i" => "multiply_i",
                    "instanceof" => "instanceof",
                    "in" => "in",
                    _ => "add",
                };
                let b = self.pop();
                let a = self.pop();
                self.push(IrNode::BinaryOp {
                    op: op.to_string(),
                    left: Box::new(a),
                    right: Box::new(b),
                });
                IrNode::Nop
            }
            "label" => IrNode::Label(format!("L{}", pc)),
            "jmp" => {
                let delta = ops.get(0).copied().unwrap_or(0);
                let tgt = ((pc + ins.length) as i64 + delta) as usize;
                targets.insert(tgt);
                IrNode::Goto {
                    target: tgt,
                    cond: None,
                }
            }
            "iftrue" | "iffalse" | "ifeq" | "ifne" | "iflt" | "ifle" | "ifgt" | "ifge"
            | "ifnlt" | "ifnle" | "ifngt" | "ifnge" | "ifstricteq" | "ifstrictne" => {
                let delta = ops.get(0).copied().unwrap_or(0);
                let tgt = ((pc + ins.length) as i64 + delta) as usize;
                targets.insert(tgt);
                let v = self.pop();
                let cond = if name == "iftrue" || name == "iffalse" {
                    if name == "iffalse" {
                        IrNode::UnaryOp {
                            op: "not".to_string(),
                            expr: Box::new(v),
                        }
                    } else {
                        v
                    }
                } else {
                    let right = v;
                    let left = self.pop();
                    let op = match name {
                        "ifeq" => "eq",
                        "ifne" => "neq",
                        "iflt" => "lt",
                        "ifle" => "lte",
                        "ifgt" => "gt",
                        "ifge" => "gte",
                        "ifnlt" => "gte",
                        "ifnle" => "gt",
                        "ifngt" => "lte",
                        "ifnge" => "lt",
                        "ifstricteq" => "stricteq",
                        "ifstrictne" => "strictne",
                        _ => "eq",
                    };
                    IrNode::BinaryOp {
                        op: op.to_string(),
                        left: Box::new(left),
                        right: Box::new(right),
                    }
                };
                IrNode::Goto {
                    target: tgt,
                    cond: Some(Box::new(cond)),
                }
            }
            "lookupswitch" => {
                let default_d = ops.get(0).copied().unwrap_or(0);
                let default_tgt = (pc as i64 + default_d) as usize;
                targets.insert(default_tgt);
                let mut cases_list = Vec::new();
                for (i, &cd) in ins.cases.iter().enumerate() {
                    let ctgt = (pc as i64 + cd as i64) as usize;
                    targets.insert(ctgt);
                    cases_list.push((
                        Some(i as i32),
                        Box::new(IrNode::Goto {
                            target: ctgt,
                            cond: None,
                        }),
                    ));
                }
                cases_list.push((
                    None,
                    Box::new(IrNode::Goto {
                        target: default_tgt,
                        cond: None,
                    }),
                ));
                let v = self.pop();
                IrNode::Switch {
                    value: Box::new(v),
                    cases: cases_list,
                }
            }
            "returnvoid" => IrNode::Return(None),
            "returnvalue" => {
                let v = self.pop();
                IrNode::Return(Some(Box::new(v)))
            }
            "throw" => {
                let v = self.pop();
                IrNode::Throw(Box::new(v))
            }
            "dxns" => {
                let idx = ops.get(0).copied().unwrap_or(0) as u32;
                let s = self.resolve_name(idx);
                IrNode::SetScopeValue {
                    ns: Box::new(s),
                    is_late: false,
                }
            }
            "debugline" => {
                let l = ops.get(0).copied().unwrap_or(0) as i32;
                IrNode::DebugLine(l)
            }
            "debugfile" => {
                let idx = ops.get(0).copied().unwrap_or(0) as usize;
                let s = if idx > 0 && idx <= self.abc.strings.len() {
                    self.abc.strings[idx - 1].clone()
                } else {
                    "".to_string()
                };
                IrNode::DebugFile(s)
            }
            "debug" => {
                if ops.len() >= 4 {
                    let idx = ops[1] as usize;
                    let reg = ops[2] as usize;
                    let line = ops[3] as i32;
                    let s = if idx > 0 && idx <= self.abc.strings.len() {
                        self.abc.strings[idx - 1].clone()
                    } else {
                        "".to_string()
                    };
                    IrNode::DebugReg {
                        reg,
                        var_name: s,
                        line,
                    }
                } else {
                    IrNode::Nop
                }
            }
            name if name.starts_with("mget") || name.starts_with("mset") => {
                if name.starts_with("mget") {
                    let addr = self.pop();
                    self.push(IrNode::MemOp {
                        op: name.to_string(),
                        args: vec![addr],
                    });
                } else {
                    let val = self.pop();
                    let addr = self.pop();
                    self.push(IrNode::MemOp {
                        op: name.to_string(),
                        args: vec![addr, val],
                    });
                }
                IrNode::Nop
            }
            _ => IrNode::Nop,
        }
    }

    pub fn run(&mut self) -> (Vec<(usize, IrNode)>, HashSet<usize>) {
        let mut out = Vec::new();
        let mut targets = HashSet::new();
        let instrs = self.body.instructions.clone();
        for ins in &instrs {
            let node = self.sim_instr(ins, &mut targets, ins.offset);
            out.push((ins.offset, node));
        }
        (out, targets)
    }
}

pub fn build_blocks(sim: &mut StackSimulator) -> (HashMap<usize, BasicBlock>, Vec<BasicBlock>) {
    let (out_nodes, targets) = sim.run();
    let mut leaders: HashSet<usize> = HashSet::new();
    leaders.insert(0);
    for &t in &targets {
        leaders.insert(t);
    }

    let ins_by_pc: HashMap<usize, Instruction> = sim
        .body
        .instructions
        .iter()
        .map(|ins| (ins.offset, ins.clone()))
        .collect();

    for &(pc, ref node) in &out_nodes {
        if let Some(ins) = ins_by_pc.get(&pc) {
            let next_pc = pc + ins.length;
            if matches!(node, IrNode::Goto { .. } | IrNode::Return(_) | IrNode::Throw(_)) {
                if ins_by_pc.contains_key(&next_pc) {
                    leaders.insert(next_pc);
                }
            }
        }
    }

    for ex in &sim.body.exceptions {
        leaders.insert(ex.from_off);
        leaders.insert(ex.to_off);
        leaders.insert(ex.target_off);
    }

    let mut sorted_leaders: Vec<usize> = leaders
        .into_iter()
        .filter(|pc| ins_by_pc.contains_key(pc))
        .collect();
    sorted_leaders.sort();

    let mut blocks: HashMap<usize, BasicBlock> = HashMap::new();
    let mut ordered: Vec<BasicBlock> = Vec::new();

    for &start in &sorted_leaders {
        let mut block_instrs = Vec::new();
        let mut pc = start;
        while let Some(ins) = ins_by_pc.get(&pc) {
            let node = out_nodes
                .iter()
                .find(|(pp, _)| *pp == pc)
                .map(|(_, n)| n.clone())
                .unwrap_or(IrNode::Nop);
            block_instrs.push((pc, node.clone()));
            let next_pc = pc + ins.length;
            let is_end = matches!(
                node,
                IrNode::Goto { .. } | IrNode::Return(_) | IrNode::Throw(_)
            ) || sorted_leaders.contains(&next_pc)
                || !ins_by_pc.contains_key(&next_pc);
            pc = next_pc;
            if is_end {
                break;
            }
        }
        let bb = BasicBlock {
            start_pc: start,
            end_pc: pc,
            instrs: block_instrs,
            successors: Vec::new(),
            predecessors: Vec::new(),
        };
        blocks.insert(start, bb.clone());
        ordered.push(bb);
    }

    for bb in &mut ordered {
        if let Some((last_pc, ref last_node)) = bb.instrs.last() {
            if let IrNode::Goto { target, ref cond } = last_node {
                if cond.is_none() {
                    bb.successors.push(*target);
                } else {
                    if let Some(ins) = ins_by_pc.get(last_pc) {
                        let fall = last_pc + ins.length;
                        if ins_by_pc.contains_key(&fall) {
                            bb.successors.push(fall);
                        }
                    }
                    bb.successors.push(*target);
                }
            } else if matches!(last_node, IrNode::Return(_) | IrNode::Throw(_)) {
            } else if let Some(ins) = ins_by_pc.get(last_pc) {
                let fall = last_pc + ins.length;
                if blocks.contains_key(&fall) {
                    bb.successors.push(fall);
                }
            }
        }
    }

    for bb in &ordered {
        for &s in &bb.successors {
            if let Some(succ_block) = blocks.get_mut(&s) {
                succ_block.predecessors.push(bb.start_pc);
            }
        }
    }

    (blocks, ordered)
}

fn merge_values(v1: IrNode, v2: IrNode, tag1: PhiTag, tag2: PhiTag) -> IrNode {
    if v1 == v2 {
        return v1;
    }
    if let IrNode::Phi { inputs } = &v1 {
        let existing: Vec<(PhiTag, IrNode)> =
            inputs.iter().filter(|(_, v)| *v != v2).cloned().collect();
        if existing.len() == inputs.len() {
            let mut all = inputs.clone();
            all.push((tag2, v2));
            return IrNode::Phi { inputs: all };
        }
        return IrNode::Phi { inputs: existing };
    }
    if let IrNode::Phi { inputs } = &v2 {
        let existing: Vec<(PhiTag, IrNode)> =
            inputs.iter().filter(|(_, v)| *v != v1).cloned().collect();
        if existing.len() == inputs.len() {
            let mut all = inputs.clone();
            all.push((tag1, v1));
            return IrNode::Phi { inputs: all };
        }
        return IrNode::Phi { inputs: existing };
    }
    IrNode::Phi {
        inputs: vec![(tag1, v1), (tag2, v2)],
    }
}

fn merge_stacks(
    mut old: Vec<IrNode>,
    mut new: Vec<IrNode>,
    old_tag: PhiTag,
    new_tag: PhiTag,
) -> Vec<IrNode> {
    if old.len() < new.len() {
        let diff = new.len() - old.len();
        let mut pad = vec![IrNode::UnknownValue("depth mismatch".to_string()); diff];
        pad.append(&mut old);
        old = pad;
    } else if new.len() < old.len() {
        let diff = old.len() - new.len();
        let mut pad = vec![IrNode::UnknownValue("depth mismatch".to_string()); diff];
        pad.append(&mut new);
        new = pad;
    }
    old.into_iter()
        .zip(new.into_iter())
        .map(|(a, b)| {
            merge_values(a, b, old_tag.clone(), new_tag.clone())
        })
        .collect()
}

fn branch_ctx(
    blocks: &HashMap<usize, CfgBlock>,
    blk: &CfgBlock,
    kind: &str,
    depth: usize,
) -> Option<PhiTag> {
    if depth > 24 {
        return None;
    }
    if blk.term_kind == "cgoto" && (kind == "jump" || kind == "fall") {
        if let Some(IrNode::Goto { cond: Some(cond), .. }) = &blk.term_node {
            return Some(PhiTag::Branch {
                cond: *cond.clone(),
                taken: kind == "jump",
            });
        }
    }
    if kind != "plain" || blk.predecessors.len() != 1 {
        return None;
    }
    let pred = blocks.get(&blk.predecessors[0])?;
    if pred.term_kind == "cgoto" {
        if let Some(IrNode::Goto { cond: Some(cond), .. }) = &pred.term_node {
            let pk = if pred.term_fall == Some(blk.start_pc) {
                "fall"
            } else {
                "jump"
            };
            return Some(PhiTag::Branch {
                cond: *cond.clone(),
                taken: pk == "jump",
            });
        }
    }
    if pred.successors.len() == 1 {
        return branch_ctx(blocks, pred, "plain", depth + 1);
    }
    None
}

pub fn build_cfg(
    abc: &AbcFile,
    body: &MethodBody,
    max_iters: usize,
) -> (HashMap<usize, CfgBlock>, Vec<CfgBlock>) {
    let mut sim = StackSimulator::new(abc, body);
    let instrs = &body.instructions;
    let ins_by_pc: HashMap<usize, Instruction> =
        instrs.iter().map(|ins| (ins.offset, ins.clone())).collect();
    let last_pc = instrs.iter().map(|ins| ins.offset).max().unwrap_or(0);

    let mut leaders: HashSet<usize> = HashSet::new();
    leaders.insert(0);

    for ins in instrs {
        let n = ins.name;
        if n == "jmp" || COND_JUMPS.contains(&n) {
            leaders.insert(jump_target(ins));
            let nxt = ins.offset + ins.length;
            if ins_by_pc.contains_key(&nxt) {
                leaders.insert(nxt);
            }
        } else if n == "lookupswitch" {
            let d = ins.operands.get(0).copied().unwrap_or(0);
            leaders.insert(((ins.offset as i64) + d) as usize);
            for &c in &ins.cases {
                leaders.insert(((ins.offset as i64) + (c as i64)) as usize);
            }
            let nxt = ins.offset + ins.length;
            if ins_by_pc.contains_key(&nxt) {
                leaders.insert(nxt);
            }
        } else if matches!(n, "returnvoid" | "returnvalue" | "throw") {
            let nxt = ins.offset + ins.length;
            if ins_by_pc.contains_key(&nxt) {
                leaders.insert(nxt);
            }
        }
    }

    for ex in &body.exceptions {
        for &pc in &[ex.from_off, ex.to_off, ex.target_off] {
            if ins_by_pc.contains_key(&pc) {
                leaders.insert(pc);
            }
        }
    }

    let mut starts: Vec<usize> = leaders
        .into_iter()
        .filter(|pc| ins_by_pc.contains_key(pc))
        .collect();
    starts.sort();

    let mut blocks: HashMap<usize, CfgBlock> = HashMap::new();
    let mut ordered: Vec<CfgBlock> = Vec::new();

    for (i, &start) in starts.iter().enumerate() {
        let end = if i + 1 < starts.len() {
            starts[i + 1]
        } else if let Some(last_ins) = instrs.last() {
            last_pc + last_ins.length
        } else {
            start
        };

        let mut blk = CfgBlock {
            start_pc: start,
            end_pc: end,
            instrs: Vec::new(),
            nodes: Vec::new(),
            successors: Vec::new(),
            predecessors: Vec::new(),
            succ_tags: Vec::new(),
            entry_stack: Vec::new(),
            term_kind: "fall".to_string(),
            term_node: None,
            term_fall: None,
            exc_starts: Vec::new(),
        };

        let mut pc = start;
        while pc < end && ins_by_pc.contains_key(&pc) {
            let ins = ins_by_pc[&pc].clone();
            blk.instrs.push(ins.clone());
            let n = ins.name;
            if n == "jmp" || COND_JUMPS.contains(&n) {
                blk.term_kind = if n == "jmp" { "goto" } else { "cgoto" }.to_string();
                pc += ins.length;
                break;
            }
            if n == "lookupswitch" {
                blk.term_kind = "switch".to_string();
                pc += ins.length;
                break;
            }
            if matches!(n, "returnvoid" | "returnvalue" | "throw") {
                blk.term_kind = if n != "throw" { "return" } else { "throw" }.to_string();
                pc += ins.length;
                break;
            }
            pc += ins.length;
        }
        blk.end_pc = pc;
        blocks.insert(start, blk.clone());
        ordered.push(blk);
    }

    for blk in &mut ordered {
        if let Some(last) = blk.instrs.last() {
            let n = last.name;
            if n == "jmp" {
                let t = jump_target(last);
                if starts.contains(&t) {
                    blk.successors = vec![t];
                    blk.succ_tags = vec![(t, "plain".to_string())];
                    if let Some(b) = blocks.get_mut(&blk.start_pc) {
                        b.successors = blk.successors.clone();
                        b.succ_tags = blk.succ_tags.clone();
                    }
                }
            } else if COND_JUMPS.contains(&n) {
                let t = jump_target(last);
                let fall = last.offset + last.length;
                let mut succs = Vec::new();
                let mut tags = Vec::new();
                if starts.contains(&fall) {
                    succs.push(fall);
                    tags.push((fall, "fall".to_string()));
                }
                if starts.contains(&t) {
                    succs.push(t);
                    tags.push((t, "jump".to_string()));
                }
                blk.successors = succs;
                blk.succ_tags = tags;
                blk.term_fall = if starts.contains(&fall) {
                    Some(fall)
                } else {
                    None
                };
                if let Some(b) = blocks.get_mut(&blk.start_pc) {
                    b.successors = blk.successors.clone();
                    b.succ_tags = blk.succ_tags.clone();
                    b.term_fall = blk.term_fall;
                }
            } else if n == "lookupswitch" {
                let mut tgts = Vec::new();
                let d = last.operands.get(0).copied().unwrap_or(0);
                for &c in &last.cases {
                    let tgt = ((last.offset as i64) + (c as i64)) as usize;
                    if starts.contains(&tgt) {
                        tgts.push((tgt, "plain".to_string()));
                    }
                }
                let dtgt = ((last.offset as i64) + d) as usize;
                if starts.contains(&dtgt) {
                    tgts.push((dtgt, "plain".to_string()));
                }
                blk.successors = tgts.iter().map(|(t, _)| *t).collect();
                blk.succ_tags = tgts;
                if let Some(b) = blocks.get_mut(&blk.start_pc) {
                    b.successors = blk.successors.clone();
                    b.succ_tags = blk.succ_tags.clone();
                }
            } else if !matches!(n, "returnvoid" | "returnvalue" | "throw") {
                if starts.contains(&blk.end_pc) {
                    blk.successors = vec![blk.end_pc];
                    blk.succ_tags = vec![(blk.end_pc, "plain".to_string())];
                    blk.term_fall = Some(blk.end_pc);
                    if let Some(b) = blocks.get_mut(&blk.start_pc) {
                        b.successors = blk.successors.clone();
                        b.succ_tags = blk.succ_tags.clone();
                        b.term_fall = blk.term_fall;
                    }
                }
            }
        }
    }

    for blk in &ordered {
        for &s in &blk.successors {
            if let Some(target_blk) = blocks.get_mut(&s) {
                target_blk.predecessors.push(blk.start_pc);
            }
        }
    }

    for ex in &body.exceptions {
        for blk in &mut ordered {
            if blk.start_pc <= ex.from_off && ex.from_off < blk.end_pc {
                blk.exc_starts.push(ex.clone());
            }
        }
    }

    if ordered.is_empty() {
        return (blocks, ordered);
    }

    let start_pc0 = ordered[0].start_pc;
    let mut entry: HashMap<usize, Vec<IrNode>> = HashMap::new();
    let mut entry_tag: HashMap<usize, PhiTag> = HashMap::new();
    let mut queue = VecDeque::new();
    let mut queued = HashSet::new();

    entry.insert(start_pc0, Vec::new());
    entry_tag.insert(start_pc0, PhiTag::Plain);
    queue.push_back(start_pc0);
    queued.insert(start_pc0);

    for ex in &body.exceptions {
        if starts.contains(&ex.target_off) && !entry.contains_key(&ex.target_off) {
            entry.insert(ex.target_off, vec![IrNode::ExceptionValue]);
            entry_tag.insert(ex.target_off, PhiTag::Plain);
            queue.push_back(ex.target_off);
            queued.insert(ex.target_off);
        }
    }

    let mut iters = 0;
    while let Some(pc) = queue.pop_front() {
        if iters >= max_iters {
            break;
        }
        iters += 1;
        queued.remove(&pc);

        let cur_entry = entry.get(&pc).cloned().unwrap_or_default();
        sim.stack = cur_entry.clone();
        let mut nodes = Vec::new();
        let mut dummy_targets = HashSet::new();
        let blocks_ref = blocks.clone();

        if let Some(blk) = blocks.get_mut(&pc) {
            blk.entry_stack = cur_entry;
            let n_instrs = blk.instrs.len();
            for (idx, ins) in blk.instrs.iter().enumerate() {
                if idx + 1 < n_instrs {
                    let node = sim.sim_instr(ins, &mut dummy_targets, ins.offset);
                    nodes.push((ins.offset, node));
                } else if blk.term_kind == "goto" {
                    let tgt = jump_target(ins);
                    nodes.push((
                        ins.offset,
                        IrNode::Goto {
                            target: tgt,
                            cond: None,
                        },
                    ));
                } else {
                    let node = sim.sim_instr(ins, &mut dummy_targets, ins.offset);
                    nodes.push((ins.offset, node));
                }
            }
            blk.nodes = nodes.clone();
            blk.term_node = nodes.last().map(|(_, n)| n.clone());

            let exit_stack = sim.stack.clone();
            for (succ, kind) in &blk.succ_tags {
                let tag = branch_ctx(&blocks_ref, &blocks_ref[&pc], kind, 0)
                    .unwrap_or(PhiTag::Plain);
                if !entry.contains_key(succ) {
                    entry.insert(*succ, exit_stack.clone());
                    entry_tag.insert(*succ, tag);
                    if !queued.contains(succ) {
                        queue.push_back(*succ);
                        queued.insert(*succ);
                    }
                } else {
                    let old_tag =
                        entry_tag.get(succ).cloned().unwrap_or(PhiTag::Plain);
                    let old_stack = entry.get(succ).cloned().unwrap();
                    let merged =
                        merge_stacks(old_stack.clone(), exit_stack.clone(), old_tag, tag);
                    if merged != old_stack {
                        entry.insert(*succ, merged);
                        if *succ > pc && !queued.contains(succ) {
                            queue.push_back(*succ);
                            queued.insert(*succ);
                        }
                    }
                }
            }
        }
    }

    for blk in ordered.clone() {
        if !entry.contains_key(&blk.start_pc) {
            entry.insert(blk.start_pc, Vec::new());
            entry_tag.insert(blk.start_pc, PhiTag::Plain);
            sim.stack = Vec::new();
            if let Some(target) = blocks.get_mut(&blk.start_pc) {
                let mut dummy_targets: HashSet<usize> = HashSet::new();
                let mut nodes = Vec::new();
                let n_instrs = target.instrs.len();
                for (idx, ins) in target.instrs.iter().enumerate() {
                    if idx + 1 < n_instrs {
                        let node = sim.sim_instr(ins, &mut dummy_targets, ins.offset);
                        nodes.push((ins.offset, node));
                    } else if target.term_kind == "goto" {
                        nodes.push((
                            ins.offset,
                            IrNode::Goto {
                                target: jump_target(ins),
                                cond: None,
                            },
                        ));
                    } else {
                        let node = sim.sim_instr(ins, &mut dummy_targets, ins.offset);
                        nodes.push((ins.offset, node));
                    }
                }
                target.nodes = nodes.clone();
                target.term_node = nodes.last().map(|(_, n)| n.clone());
                target.entry_stack = Vec::new();
            }
        }
    }

    for blk in &mut ordered {
        if let Some(b) = blocks.get(&blk.start_pc) {
            *blk = b.clone();
        }
    }

    (blocks, ordered)
}

pub fn stack_eq(s1: &[IrNode], s2: &[IrNode]) -> bool {
    s1.len() == s2.len() && s1.iter().zip(s2.iter()).all(|(a, b)| a == b)
}
