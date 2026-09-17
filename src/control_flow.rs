use std::collections::{HashMap, HashSet};

use crate::abc::{AbcFile, ExceptionInfo, MethodBody};
use crate::haxe_layer::{EnumCtor, EnumDef};
use crate::ir::{peel, unwrap_reg, collect_regs, IrNode};
use crate::translate::{build_cfg, CfgBlock, StackSimulator};

#[derive(Debug, Clone)]
pub enum SNode {
    Removed,
    Seq(Vec<SNode>),
    Expr(IrNode),
    If {
        cond: IrNode,
        then_body: Box<SNode>,
        else_body: Option<Box<SNode>>,
    },
    While {
        cond: Option<IrNode>,
        body: Box<SNode>,
    },
    DoWhile {
        cond: IrNode,
        body: Box<SNode>,
    },
    ForIn {
        var_name: String,
        kind: String,
        obj: IrNode,
        body: Box<SNode>,
        val_name: String,
        var_reg: i64,
        val_reg: i64,
    },
    ForRange {
        var_name: String,
        start: IrNode,
        end: IrNode,
        body: Box<SNode>,
        index_reg: i64,
        index_name: Option<String>,
        var_reg: i64,
    },
    Switch {
        value: IrNode,
        cases: Vec<(Vec<i32>, SNode)>,
        has_default: bool,
        match_enum: Option<EnumDef>,
        match_base: Option<IrNode>,
        case_plan: Option<Vec<(Option<Vec<EnumCtor>>, SNode)>>,
        expr_return: bool,
    },
    Try {
        body: Box<SNode>,
        catches: Vec<(String, String, SNode)>,
        finally_body: Option<Box<SNode>>,
    },
    Throw(IrNode),
    Return(Option<IrNode>),
    Break,
    Continue,
    Goto {
        target: usize,
        comment: String,
    },
}

impl SNode {
    pub fn empty_seq() -> SNode {
        SNode::Seq(Vec::new())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LoopCtx {
    pub header: usize,
    pub exit_pc: Option<usize>,
    pub continue_pc: Option<usize>,
    #[allow(dead_code)]
    pub cond_block: Option<usize>,
}

pub fn is_noise(node: &IrNode) -> bool {
    match node {
        IrNode::Nop
        | IrNode::Label(_)
        | IrNode::PushScope(_)
        | IrNode::PopScope
        | IrNode::DebugLine(_)
        | IrNode::DebugFile(_)
        | IrNode::DebugReg { .. }
        | IrNode::Dup
        | IrNode::Swap
        | IrNode::Pop
        | IrNode::Kill(_)
        | IrNode::HasNext { .. }
        | IrNode::NextIter { .. } => true,
        IrNode::RegSet { value, .. } => {
            matches!(**value, IrNode::NewActivation | IrNode::This)
        }
        IrNode::ExprStmt(e) => matches!(**e, IrNode::PushScope(_)),
        _ => false,
    }
}

pub fn unnegate(cond: Option<&IrNode>) -> IrNode {
    match cond {
        None => IrNode::BoolConst(true),
        Some(c) => crate::haxe_out::simplify_not(c),
    }
}

pub fn is_empty_seq(n: &SNode) -> bool {
    match n {
        SNode::Seq(stmts) => stmts.iter().all(is_empty_seq),
        _ => false,
    }
}

pub fn seq_of(n: &SNode) -> Vec<SNode> {
    match n {
        SNode::Seq(stmts) => stmts.clone(),
        other => vec![other.clone()],
    }
}

pub struct Structurer<'a> {
    pub abc: &'a AbcFile,
    pub body: &'a MethodBody,
    pub blocks: HashMap<usize, CfgBlock>,
    pub ordered: Vec<CfgBlock>,
    pub exceptions: Vec<ExceptionInfo>,
    pub done: HashSet<usize>,
    pub loop_active: HashSet<usize>,
    pub reg_names: HashMap<usize, String>,
    pub back_edges: HashMap<usize, Vec<(usize, Option<IrNode>)>>,
    pub exc_at: HashMap<usize, Vec<ExceptionInfo>>,
    pub structured_exc: HashSet<usize>,
}

impl<'a> Structurer<'a> {
    pub fn new(
        abc: &'a AbcFile,
        body: &'a MethodBody,
        blocks: HashMap<usize, CfgBlock>,
        ordered: Vec<CfgBlock>,
    ) -> Self {
        let sim = StackSimulator::new(abc, body);
        let mut back_edges: HashMap<usize, Vec<(usize, Option<IrNode>)>> = HashMap::new();
        for blk in &ordered {
            if let Some(IrNode::Goto { target, cond }) = &blk.term_node {
                if blocks.contains_key(target) && *target < blk.start_pc {
                    let cond = if blk.term_kind == "cgoto" { cond.clone() } else { None };
                    back_edges
                        .entry(*target)
                        .or_default()
                        .push((blk.start_pc, cond.map(|c| *c)));
                }
            }
        }
        let mut exc_at: HashMap<usize, Vec<ExceptionInfo>> = HashMap::new();
        for ex in &body.exceptions {
            if blocks.contains_key(&ex.from_off) {
                exc_at.entry(ex.from_off).or_default().push(ex.clone());
            }
        }
        Structurer {
            abc,
            body,
            blocks,
            ordered,
            exceptions: body.exceptions.clone(),
            done: HashSet::new(),
            loop_active: HashSet::new(),
            reg_names: sim.reg_names,
            back_edges,
            exc_at,
            structured_exc: HashSet::new(),
        }
    }

    pub fn reach(&self, start: usize, stops: &HashSet<usize>) -> HashSet<usize> {
        let mut seen = HashSet::new();
        let mut stack = vec![start];
        while let Some(pc) = stack.pop() {
            if seen.len() >= 500 {
                break;
            }
            if seen.contains(&pc) || stops.contains(&pc) || self.done.contains(&pc)
                || !self.blocks.contains_key(&pc)
            {
                continue;
            }
            seen.insert(pc);
            if let Some(blk) = self.blocks.get(&pc) {
                stack.extend(blk.successors.iter().copied());
            }
        }
        seen
    }

    pub fn find_join(
        &self,
        a: usize,
        b: usize,
        stops: &HashSet<usize>,
        loop_ctx: Option<&LoopCtx>,
    ) -> Option<usize> {
        let mut stops = stops.clone();
        if let Some(l) = loop_ctx {
            stops.insert(l.header);
        }
        let ra = self.reach(a, &stops);
        let rb = self.reach(b, &stops);
        let common: HashSet<usize> = ra.intersection(&rb).copied().collect();
        if common.is_empty() {
            None
        } else {
            common.into_iter().min()
        }
    }

    pub fn terminates(&self, start: usize, stops: &HashSet<usize>) -> bool {
        let mut seen = HashSet::new();
        let mut stack = vec![start];
        while let Some(pc) = stack.pop() {
            if seen.contains(&pc) {
                continue;
            }
            if stops.contains(&pc) {
                return false;
            }
            if self.done.contains(&pc) {
                return false;
            }
            if !self.blocks.contains_key(&pc) {
                continue;
            }
            seen.insert(pc);
            let blk = &self.blocks[&pc];
            if blk.term_kind == "return" {
                continue;
            }
            stack.extend(blk.successors.iter().copied());
        }
        true
    }

    pub fn region(&mut self, start: usize, stops: &HashSet<usize>, loop_ctx: Option<LoopCtx>) -> SNode {
        let mut stmts: Vec<SNode> = Vec::new();
        let mut pc: Option<usize> = Some(start);
        let mut guard = 0;
        while let Some(cur) = pc {
            if stops.contains(&cur) {
                break;
            }
            guard += 1;
            if guard > 2000 {
                stmts.push(SNode::Goto { target: cur, comment: "region guard".into() });
                break;
            }
            if !self.blocks.contains_key(&cur) || self.done.contains(&cur) {
                break;
            }
            if self.exc_at.contains_key(&cur) && !self.structured_exc.contains(&cur) {
                let ex = self.exc_at[&cur][0].clone();
                let (node, nxt) = self.structure_try(&ex, stops, loop_ctx);
                stmts.push(node);
                pc = nxt;
                continue;
            }
            if self.back_edges.contains_key(&cur)
                && !self.done.contains(&cur)
                && !self.loop_active.contains(&cur)
            {
                let (node, nxt) = self.structure_posttested_loop(cur, stops, loop_ctx);
                if let Some(node) = node {
                    stmts.push(node);
                    pc = nxt;
                    continue;
                }
            }
            self.done.insert(cur);
            let blk = self.blocks[&cur].clone();
            let nnodes = blk.nodes.len();
            for (i, (_npc, node)) in blk.nodes.iter().enumerate() {
                if i == nnodes - 1 && blk.term_kind != "fall" {
                    break;
                }
                if is_noise(node) {
                    continue;
                }
                stmts.push(SNode::Expr(node.clone()));
            }
            let term = blk.term_kind.clone();
            let tnode = blk.term_node.clone();
            let pre = if term == "goto" {
                self.match_pretested_loop(&blk, stops)
            } else {
                None
            };
            if let Some(pre) = pre {
                let (node, nxt) = self.structure_pretested_loop(&blk, &pre, stops, loop_ctx);
                stmts.push(node);
                pc = nxt;
                continue;
            }
            match term.as_str() {
                "return" | "throw" => {
                    match tnode {
                        Some(IrNode::Return(v)) => {
                            stmts.push(SNode::Return(v.map(|b| *b)));
                        }
                        Some(IrNode::Throw(v)) => {
                            stmts.push(SNode::Throw(*v));
                        }
                        _ => {}
                    }
                    pc = None;
                }
                "goto" => {
                    if let Some(IrNode::Goto { target: t, .. }) = &tnode {
                        let t = *t;
                        if let Some(l) = &loop_ctx {
                            if Some(t) == Some(l.header) || Some(t) == l.continue_pc {
                                stmts.push(SNode::Continue);
                                pc = None;
                                continue;
                            }
                            if Some(t) == l.exit_pc {
                                stmts.push(SNode::Break);
                                pc = None;
                                continue;
                            }
                        }
                        if stops.contains(&t) || self.done.contains(&t) || !self.blocks.contains_key(&t) {
                            pc = None;
                        } else {
                            pc = Some(t);
                        }
                    } else {
                        pc = None;
                    }
                }
                "cgoto" => {
                    let (t, f, cond) = match &tnode {
                        Some(IrNode::Goto { target, cond }) => (
                            Some(*target),
                            blk.term_fall,
                            cond.clone().map(|c| *c),
                        ),
                        _ => (None, blk.term_fall, None),
                    };
                    if let Some(l) = &loop_ctx {
                        if t == Some(l.header) && f.is_none() {
                            pc = None;
                            continue;
                        }
                        if t == l.continue_pc && f.is_none() {
                            pc = None;
                            continue;
                        }
                        if f.is_some() && t.is_some() &&
                           (t == Some(l.header) || t == l.continue_pc) {
                            stmts.push(SNode::If {
                                cond: cond.clone().unwrap_or(IrNode::BoolConst(true)),
                                then_body: Box::new(SNode::Seq(vec![SNode::Continue])),
                                else_body: None,
                            });
                            pc = f;
                            continue;
                        }
                        if f.is_some() && t == l.exit_pc {
                            stmts.push(SNode::If {
                                cond: cond.clone().unwrap_or(IrNode::BoolConst(true)),
                                then_body: Box::new(SNode::Seq(vec![SNode::Break])),
                                else_body: None,
                            });
                            pc = f;
                            continue;
                        }
                    }
                    let join = if f.is_some() {
                        self.find_join(t.unwrap(), f.unwrap(), stops, loop_ctx.as_ref())
                    } else {
                        None
                    };
                    if join.is_none() && f.is_some() {
                        let f_dead = self.terminates(f.unwrap(), stops);
                        let t_dead = self.terminates(t.unwrap(), stops);
                        if f_dead && !t_dead {
                            let then_node = match f {
                                Some(fv) => self.region(fv, &stops.clone(), loop_ctx),
                                None => SNode::empty_seq(),
                            };
                            stmts.push(SNode::If {
                                cond: unnegate(cond.as_ref()),
                                then_body: Box::new(then_node),
                                else_body: None,
                            });
                            pc = t.filter(|&t| !stops.contains(&t) && !self.done.contains(&t));
                            continue;
                        }
                        if t_dead && !f_dead {
                            let else_node = match t {
                                Some(tv) => self.region(tv, &stops.clone(), loop_ctx),
                                None => SNode::empty_seq(),
                            };
                            stmts.push(SNode::If {
                                cond: cond.unwrap_or(IrNode::BoolConst(true)),
                                then_body: Box::new(else_node),
                                else_body: None,
                            });
                            pc = f;
                            continue;
                        }
                        let stop_set = stops.clone();
                        let then_node = match f {
                            Some(fv) => self.region(fv, &stop_set, loop_ctx),
                            None => SNode::empty_seq(),
                        };
                        let else_node = match t {
                            Some(tv) => self.region(tv, &stop_set, loop_ctx),
                            None => SNode::empty_seq(),
                        };
                        let else_empty = is_empty_seq(&else_node);
                        let then_empty = is_empty_seq(&then_node);
                        if then_empty && else_empty {
                        } else if then_empty {
                            stmts.push(SNode::If {
                                cond: cond.unwrap_or(IrNode::BoolConst(true)),
                                then_body: Box::new(else_node),
                                else_body: None,
                            });
                        } else {
                            stmts.push(SNode::If {
                                cond: unnegate(cond.as_ref()),
                                then_body: Box::new(then_node),
                                else_body: if else_empty { None } else { Some(Box::new(else_node)) },
                            });
                        }
                        pc = None;
                        continue;
                    }
                    let mut stop_set = stops.clone();
                    if let Some(j) = join {
                        stop_set.insert(j);
                    }
                    let then_node = match f {
                        Some(fv) => self.region(fv, &stop_set, loop_ctx),
                        None => SNode::empty_seq(),
                    };
                    let else_node = match t {
                        Some(tv) => self.region(tv, &stop_set, loop_ctx),
                        None => SNode::empty_seq(),
                    };
                    let else_empty = is_empty_seq(&else_node);
                    let then_empty = is_empty_seq(&then_node);
                    if then_empty && else_empty {
                    } else if then_empty && !else_empty {
                        stmts.push(SNode::If {
                            cond: cond.unwrap_or(IrNode::BoolConst(true)),
                            then_body: Box::new(else_node),
                            else_body: None,
                        });
                    } else {
                        stmts.push(SNode::If {
                            cond: unnegate(cond.as_ref()),
                            then_body: Box::new(then_node),
                            else_body: if else_empty { None } else { Some(Box::new(else_node)) },
                        });
                    }
                    match join {
                        None => pc = None,
                        Some(j) => {
                            if self.done.contains(&j) || stops.contains(&j) {
                                pc = None;
                            } else {
                                pc = Some(j);
                            }
                        }
                    }
                }
                "switch" => {
                    let (node, nxt) = self.structure_switch(&blk, stops, loop_ctx);
                    stmts.push(node);
                    pc = nxt;
                }
                _ => {
                    pc = blk.term_fall;
                }
            }
        }
        SNode::Seq(stmts)
    }

    fn match_pretested_loop(
        &self,
        blk: &CfgBlock,
        stops: &HashSet<usize>,
    ) -> Option<(usize, usize)> {
        if blk.term_kind != "goto" {
            return None;
        }
        let Some(IrNode::Goto { target: cpc, cond: None }) = &blk.term_node else {
            return None;
        };
        let cpc = *cpc;
        let Some(cblk) = self.blocks.get(&cpc) else { return None };
        if cblk.term_kind != "cgoto" {
            return None;
        }
        let Some(IrNode::Goto { target: h, .. }) = &cblk.term_node else {
            return None;
        };
        let h = *h;
        if h >= cpc || !self.blocks.contains_key(&h) || self.done.contains(&h)
            || self.done.contains(&cpc)
        {
            return None;
        }
        if stops.contains(&h) || stops.contains(&cpc) {
            return None;
        }
        Some((cpc, h))
    }

    fn structure_pretested_loop(
        &mut self,
        blk: &CfgBlock,
        pre: &(usize, usize),
        stops: &HashSet<usize>,
        _loop_ctx: Option<LoopCtx>,
    ) -> (SNode, Option<usize>) {
        let (cpc, h) = *pre;
        let cblk = self.blocks[&cpc].clone();
        let cond = match &cblk.term_node {
            Some(IrNode::Goto { cond: Some(c), .. }) => (**c).clone(),
            _ => IrNode::BoolConst(true),
        };
        let exit_pc = cblk.term_fall;
        self.loop_active.insert(h);
        self.done.insert(blk.start_pc);
        let mut body_stops = stops.clone();
        body_stops.insert(cpc);
        let inner = LoopCtx {
            header: h,
            exit_pc,
            continue_pc: Some(cpc),
            cond_block: Some(cpc),
        };
        let body = self.region(h, &body_stops, Some(inner));
        let cond_stmts: Vec<SNode> = cblk.nodes[..cblk.nodes.len().saturating_sub(1)]
            .iter()
            .filter(|(_, n)| !is_noise(n))
            .map(|(_, n)| SNode::Expr(n.clone()))
            .collect();
        let body = if !cond_stmts.is_empty() {
            let mut items = seq_of(&body);
            items.extend(cond_stmts);
            SNode::Seq(items)
        } else {
            body
        };
        self.done.insert(cpc);
        (SNode::While { cond: Some(cond), body: Box::new(body) }, exit_pc)
    }

    fn structure_posttested_loop(
        &mut self,
        h: usize,
        stops: &HashSet<usize>,
        _loop_ctx: Option<LoopCtx>,
    ) -> (Option<SNode>, Option<usize>) {
        let edges = self.back_edges.get(&h).cloned().unwrap_or_default();
        let cond_edges: Vec<(usize, Option<IrNode>)> =
            edges.iter().filter(|(_, c)| c.is_some()).cloned().collect();
        let plain_edges: Vec<(usize, Option<IrNode>)> =
            edges.iter().filter(|(_, c)| c.is_none()).cloned().collect();
        if !cond_edges.is_empty() && plain_edges.is_empty() {
            let (src, cond) = cond_edges[0].clone();
            let sblk = self.blocks[&src].clone();
            let exit_pc = sblk.term_fall;
            self.loop_active.insert(h);
            let inner = LoopCtx { header: h, exit_pc, continue_pc: Some(h), cond_block: None };
            let mut body_stops = stops.clone();
            body_stops.insert(src);
            let body = self.region(h, &body_stops, Some(inner));
            self.done.insert(src);
            let cond_node = match cond {
                Some(c) => c,
                None => IrNode::BoolConst(true),
            };
            return (
                Some(SNode::DoWhile { cond: cond_node, body: Box::new(body) }),
                exit_pc,
            );
        }
        if !plain_edges.is_empty() {
            let (src, _) = plain_edges[0];
            let mut nxt = None;
            for ob in &self.ordered {
                if ob.start_pc > src {
                    nxt = Some(ob.start_pc);
                    break;
                }
            }
            self.loop_active.insert(h);
            let inner = LoopCtx { header: h, exit_pc: nxt, continue_pc: Some(h), cond_block: None };
            let mut body_stops = stops.clone();
            if let Some(n) = nxt {
                body_stops.insert(n);
            }
            let body = self.region(h, &body_stops, Some(inner));
            self.done.insert(src);
            return (
                Some(SNode::While {
                    cond: Some(IrNode::BoolConst(true)),
                    body: Box::new(body),
                }),
                nxt,
            );
        }
        (None, None)
    }

    fn structure_switch(
        &mut self,
        blk: &CfgBlock,
        stops: &HashSet<usize>,
        loop_ctx: Option<LoopCtx>,
    ) -> (SNode, Option<usize>) {
        let tnode = blk.term_node.clone();
        let Some(IrNode::Switch { value, cases }) = tnode else {
            return (SNode::empty_seq(), None);
        };
        let mut base: i32 = 0;
        let sub_min = match &*value {
            IrNode::BinaryOp { op, left, right } if op == "sub" || op == "sub_i" => {
                if let IrNode::IntConst(v) = **right {
                    Some(((**left).clone(), v))
                } else {
                    None
                }
            }
            _ => None,
        };
        let value = match sub_min {
            Some((left, v)) => {
                base = v;
                left
            }
            None => *value,
        };
        let mut default_pc: Option<usize> = None;
        for (val, body) in &cases {
            if val.is_none() {
                if let IrNode::Goto { target, .. } = &**body {
                    default_pc = Some(*target);
                }
            }
        }
        let mut case_entries: Vec<(i32, usize)> = Vec::new();
        for (val, body) in &cases {
            let Some(v) = val else { continue };
            let IrNode::Goto { target: tgt, .. } = &**body else { continue };
            let tgt = *tgt;
            if Some(tgt) == default_pc {
                continue;
            }
            case_entries.push((base.wrapping_add(*v), tgt));
        }
        let mut merged: Vec<(Vec<i32>, usize)> = Vec::new();
        for (val, tgt) in case_entries {
            if let Some(last) = merged.last_mut() {
                if last.1 == tgt {
                    last.0.push(val);
                    continue;
                }
            }
            merged.push((vec![val], tgt));
        }
        let mut stops_all = stops.clone();
        let mut reach_count: HashMap<usize, usize> = HashMap::new();
        let mut join_sources: Vec<usize> = merged.iter().map(|(_, t)| *t).collect();
        if let Some(d) = default_pc {
            join_sources.push(d);
        }
        for tgt in join_sources {
            for b in self.reach(tgt, &stops_all) {
                *reach_count.entry(b).or_insert(0) += 1;
            }
        }
        let shared: Vec<usize> = reach_count
            .into_iter()
            .filter(|(_, c)| *c >= 2)
            .map(|(b, _)| b)
            .collect();
        let join = shared.iter().min().copied();
        if let Some(j) = join {
            stops_all.insert(j);
        }
        let mut out_cases: Vec<(Vec<i32>, SNode)> = Vec::new();
        for (vals, tgt) in &merged {
            let body = self.region(*tgt, &stops_all, loop_ctx);
            out_cases.push((vals.clone(), body));
        }
        let mut has_default = false;
        if let Some(d) = default_pc {
            let body = self.region(d, &stops_all, loop_ctx);
            out_cases.push((Vec::new(), body));
            has_default = true;
        }
        (
            SNode::Switch {
                value,
                cases: out_cases,
                has_default,
                match_enum: None,
                match_base: None,
                case_plan: None,
                expr_return: false,
            },
            join,
        )
    }

    fn structure_try(
        &mut self,
        ex: &ExceptionInfo,
        stops: &HashSet<usize>,
        loop_ctx: Option<LoopCtx>,
    ) -> (SNode, Option<usize>) {
        self.structured_exc.insert(ex.from_off);
        let mut after: Option<usize> = None;
        let mut pcw = ex.to_off;
        for _ in 0..10 {
            let Some(b) = self.blocks.get(&pcw) else { break };
            if b.term_kind == "goto" {
                if let Some(IrNode::Goto { target: nxt, cond: None }) = &b.term_node {
                    let nxt = *nxt;
                    if nxt == pcw {
                        break;
                    }
                    pcw = nxt;
                    continue;
                }
            }
            after = Some(pcw);
            break;
        }
        let mut try_stops = stops.clone();
        try_stops.insert(ex.to_off);
        try_stops.insert(ex.target_off);
        if let Some(a) = after {
            try_stops.insert(a);
        }
        let try_body = self.region(ex.from_off, &try_stops, loop_ctx);
        let mut handler_stops = stops.clone();
        if let Some(a) = after {
            handler_stops.insert(a);
        }
        handler_stops.insert(ex.to_off);
        let mut catch_body = self.region(ex.target_off, &handler_stops, loop_ctx);
        let mut var_name: Option<String> = None;
        let mut var_type = "Dynamic".to_string();
        let mut catch_reg: Option<usize> = None;

        fn strip_catch_assign(
            n: &mut SNode,
            var_name: &mut Option<String>,
            catch_reg: &mut Option<usize>,
            reg_names: &HashMap<usize, String>,
        ) {
            match n {
                SNode::Seq(stmts) => {
                    let mut out = Vec::with_capacity(stmts.len());
                    for s in stmts.iter_mut() {
                        if let SNode::Expr(IrNode::RegSet { reg, value, hint }) = s {
                            let v = peel(value);
                            if matches!(v, IrNode::ExceptionValue) {
                                if var_name.is_none() {
                                    *var_name = hint
                                        .clone()
                                        .or_else(|| reg_names.get(reg).cloned());
                                    *catch_reg = Some(*reg);
                                }
                                continue;
                            }
                        }
                        strip_catch_assign(s, var_name, catch_reg, reg_names);
                        out.push(std::mem::replace(s, SNode::Removed));
                    }
                    *stmts = out;
                }
                SNode::If { then_body, else_body, .. } => {
                    strip_catch_assign(then_body, var_name, catch_reg, reg_names);
                    if let Some(e) = else_body {
                        strip_catch_assign(e, var_name, catch_reg, reg_names);
                    }
                }
                SNode::While { body, .. }
                | SNode::DoWhile { body, .. }
                | SNode::ForIn { body, .. }
                | SNode::ForRange { body, .. } => {
                    strip_catch_assign(body, var_name, catch_reg, reg_names);
                }
                _ => {}
            }
        }

        strip_catch_assign(&mut catch_body, &mut var_name, &mut catch_reg, &self.reg_names);
        let mut var_name = var_name.unwrap_or_else(|| "e".to_string());
        if ex.name_idx > 0 {
            if let Some(mn) = self.abc.multinames.get((ex.name_idx - 1) as usize) {
                let r = mn.resolve(self.abc);
                var_name = r.rsplit("::").next().unwrap_or("e").to_string();
            }
        }
        if ex.type_idx > 0 {
            if let Some(mn) = self.abc.multinames.get((ex.type_idx - 1) as usize) {
                let r = mn.resolve(self.abc);
                var_type = r.rsplit("::").next().unwrap_or("Dynamic").to_string();
            }
        }
        if let Some(reg) = catch_reg {
            bind_catch_var(&mut catch_body, reg, &var_name);
        }
        if self.blocks.contains_key(&ex.to_off) {
            self.done.insert(ex.to_off);
        }
        let st = SNode::Try {
            body: Box::new(try_body),
            catches: vec![(var_name, var_type, catch_body)],
            finally_body: None,
        };
        (st, after)
    }
}

fn bind_catch_var(body: &mut SNode, reg: usize, nm: &str) {
    fn fix_node(nd: &mut IrNode, reg: usize, nm: &str) {
        match nd {
            IrNode::RegAccess { .. } => {}
            IrNode::NewFunction { .. } | IrNode::NewClass { .. } => {}
            IrNode::RegSet { reg: r, value, hint, .. } => {
                if *r == reg {
                    *hint = Some(nm.to_string());
                }
                fix_node(value, reg, nm);
            }
            other => {
                other.map_children_mut(&mut |child| {
                    if matches!(child, IrNode::RegAccess { reg: r, .. } if *r == reg) {
                        *child = IrNode::CatchVarRef(nm.to_string());
                    } else {
                        fix_node(child, reg, nm);
                    }
                });
            }
        }
    }

    fn walk_body(b: &mut SNode, reg: usize, nm: &str) {
        match b {
            SNode::Seq(stmts) => {
                for s in stmts.iter_mut() {
                    walk_body(s, reg, nm);
                }
            }
            SNode::Expr(node) => fix_node(node, reg, nm),
            SNode::If { then_body, else_body, .. } => {
                walk_body(then_body, reg, nm);
                if let Some(e) = else_body {
                    walk_body(e, reg, nm);
                }
            }
            SNode::While { body, .. }
            | SNode::DoWhile { body, .. }
            | SNode::ForIn { body, .. }
            | SNode::ForRange { body, .. } => {
                walk_body(body, reg, nm);
            }
            SNode::Return(Some(v)) => fix_node(v, reg, nm),
            SNode::Throw(v) => fix_node(v, reg, nm),
            _ => {}
        }
    }

    fn replace_direct(n: &mut SNode, reg: usize, nm: &str) {
        match n {
            SNode::Seq(stmts) => {
                for s in stmts.iter_mut() {
                    replace_direct(s, reg, nm);
                }
            }
            SNode::Expr(node) => {
                node.map_children_mut(&mut |child| {
                    if matches!(child, IrNode::RegAccess { reg: r, .. } if *r == reg) {
                        *child = IrNode::CatchVarRef(nm.to_string());
                    } else {
                        fix_node(child, reg, nm);
                    }
                });
            }
            SNode::If { cond, then_body, else_body } => {
                if matches!(*cond, IrNode::RegAccess { reg: r, .. } if r == reg) {
                    *cond = IrNode::CatchVarRef(nm.to_string());
                } else {
                    fix_node(cond, reg, nm);
                }
                replace_direct(then_body, reg, nm);
                if let Some(e) = else_body {
                    replace_direct(e, reg, nm);
                }
            }
            SNode::While { cond, body } => {
                if cond.is_none() { *cond = Some(IrNode::BoolConst(true)); }
                if let Some(c) = cond.as_mut() {
                    fix_node(c, reg, nm);
                }
                replace_direct(body, reg, nm);
            }
            SNode::DoWhile { cond, body } => {
                fix_node(cond, reg, nm);
                replace_direct(body, reg, nm);
            }
            SNode::ForIn { obj, body, .. } | SNode::ForRange { start: obj, body, .. } => {
                fix_node(obj, reg, nm);
                replace_direct(body, reg, nm);
            }
            SNode::Switch { value, cases, match_base, case_plan, .. } => {
                fix_node(value, reg, nm);
                if let Some(mb) = match_base {
                    fix_node(mb, reg, nm);
                }
                for (_, b) in cases.iter_mut() {
                    replace_direct(b, reg, nm);
                }
                if let Some(plan) = case_plan {
                    for (_, b) in plan.iter_mut() {
                        replace_direct(b, reg, nm);
                    }
                }
            }
            SNode::Try { body, catches, finally_body } => {
                replace_direct(body, reg, nm);
                for (_, _, b) in catches.iter_mut() {
                    replace_direct(b, reg, nm);
                }
                if let Some(fb) = finally_body {
                    replace_direct(fb, reg, nm);
                }
            }
            SNode::Return(Some(v)) => fix_node(v, reg, nm),
            SNode::Throw(v) => fix_node(v, reg, nm),
            _ => {}
        }
    }

    walk_body(body, reg, nm);
    replace_direct(body, reg, nm);
}

fn stmt_writes_reg(s: &SNode) -> Option<usize> {
    match s {
        SNode::Expr(IrNode::RegSet { reg, .. }) => Some(*reg),
        _ => None,
    }
}

fn is_zero_init(s: &SNode, reg: usize) -> bool {
    if let SNode::Expr(IrNode::RegSet { reg: r, value, .. }) = s {
        if *r == reg {
            let v = peel(value);
            return matches!(v, IrNode::IntConst(0) | IrNode::UIntConst(0));
        }
    }
    false
}

#[allow(dead_code)]
fn body_has_break_continue(n: &SNode) -> bool {
    match n {
        SNode::Break | SNode::Continue => true,
        SNode::Seq(stmts) => stmts.iter().any(body_has_break_continue),
        SNode::If { then_body, else_body, .. } => {
            body_has_break_continue(then_body)
                || else_body.as_ref().map(|e| body_has_break_continue(e)).unwrap_or(false)
        }
        SNode::While { body, .. }
        | SNode::DoWhile { body, .. }
        | SNode::ForIn { body, .. }
        | SNode::ForRange { body, .. } => body_has_break_continue(body),
        SNode::Try { body, catches, finally_body } => {
            body_has_break_continue(body)
                || catches.iter().any(|(_, _, b)| body_has_break_continue(b))
                || finally_body.as_ref().map(|f| body_has_break_continue(f)).unwrap_or(false)
        }
        SNode::Switch { cases, case_plan, .. } => {
            cases.iter().any(|(_, b)| body_has_break_continue(b))
                || case_plan
                    .as_ref()
                    .map(|p| p.iter().any(|(_, b)| body_has_break_continue(b)))
                    .unwrap_or(false)
        }
        _ => false,
    }
}

fn is_counter_inc(v: &IrNode, reg: usize) -> bool {
    match v {
        IrNode::UnaryOp { op, expr } => {
            (op == "increment" || op == "increment_i")
                && matches!(**expr, IrNode::RegAccess { reg: r, .. } if r == reg)
        }
        _ => false,
    }
}

fn reg_name(reg: usize) -> String {
    crate::haxe_out::emit_expr(&IrNode::RegAccess { reg, hint: None }, 0)
}

fn try_match_loop(wh: &SNode, out: &[SNode]) -> Option<(Vec<SNode>, SNode)> {
    let SNode::While { cond: Some(cond), body } = wh else { return None };
    let cond = (*cond).clone();
    let IrNode::BinaryOp { op, ref left, ref right } = cond else { return None };
    if op != "lt" {
        return None;
    }
    let Some(i) = unwrap_reg(left) else { return None };
    let body = seq_of(body);
    if body.is_empty() {
        return None;
    }

    let mut var_reg: Option<usize> = None;
    let mut consumed = 0usize;
    let mut arr_obj: Option<IrNode> = None;

    if let Some(SNode::Expr(IrNode::Block(stmts))) = body.first() {
        if stmts.len() == 2 {
            if let (
                IrNode::RegSet { reg: treg, .. },
                IrNode::RegSet { reg: s1reg, value: s1val, .. },
            ) = (&stmts[0], &stmts[1])
            {
                let treg = *treg;
                if treg >= 10000 && *s1reg == i && is_counter_inc(s1val, i) {
                    consumed = 1;
                    if body.len() >= 2 {
                        if let Some(SNode::Expr(IrNode::RegSet { reg: vr, value, .. })) = body.get(1) {
                            if unwrap_reg(&peel(value)) == Some(treg) {
                                var_reg = Some(*vr);
                                consumed = 2;
                            }
                        }
                    }
                }
            }
        }
    }

    if consumed == 0 {
        if let Some(SNode::Expr(IrNode::RegSet { reg: fr, value, .. })) = body.first() {
            let v = peel(value);
            if let IrNode::PropGet { obj, prop, .. } = &v {
                if unwrap_reg(&peel(prop)) == Some(i) && body.len() >= 2 {
                    if let Some(SNode::Expr(IrNode::RegSet { reg: sr, value: sval, .. })) = body.get(1) {
                        if *sr == i && is_counter_inc(&peel(sval), i) {
                            var_reg = Some(*fr);
                            arr_obj = Some((**obj).clone());
                            consumed = 2;
                        }
                    }
                }
            }
        }
    }

    if consumed == 0 {
        if let Some(SNode::Expr(IrNode::RegSet { reg: fr, value, .. })) = body.first() {
            if *fr == i && is_counter_inc(&peel(value), i) {
                consumed = 1;
                if body.len() >= 2 {
                    if let Some(SNode::Expr(IrNode::RegSet { reg: sr, value: sval, .. })) = body.get(1) {
                        if unwrap_reg(&peel(sval)) == Some(i) {
                            var_reg = Some(*sr);
                            consumed = 2;
                        }
                    }
                }
            }
        }
    }

    let var_reg = var_reg?;
    if consumed == 0 {
        return None;
    }

    let mut init_pos: Option<usize> = None;
    for back in 1..4usize {
        let j = out.len().checked_sub(back)?;
        let s2 = &out[j];
        if stmt_writes_reg(s2) == Some(i) {
            if is_zero_init(s2, i) {
                init_pos = Some(j);
            }
            break;
        }
    }
    let init_pos = init_pos?;

    let rest = body[consumed..].to_vec();

    if let Some(arr) = arr_obj {
        let cr = peel(right);
        let len_obj = match &cr {
            IrNode::PropGet { obj, .. } => Some((**obj).clone()),
            _ => None,
        };
        if let Some(lo) = len_obj {
            if lo == arr {
                let new_out: Vec<SNode> = out
                    .iter()
                    .enumerate()
                    .filter(|(idx, _)| *idx != init_pos)
                    .map(|(_, s)| s.clone())
                    .collect();
                return Some((
                    new_out,
                    SNode::ForIn {
                        var_name: reg_name(var_reg),
                        kind: "value".to_string(),
                        obj: arr,
                        body: Box::new(SNode::Seq(rest)),
                        val_name: String::new(),
                        var_reg: var_reg as i64,
                        val_reg: -1,
                    },
                ));
            }
        }
        return None;
    }

    let new_out: Vec<SNode> = out
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx != init_pos)
        .map(|(_, s)| s.clone())
        .collect();
    Some((
        new_out,
        SNode::ForRange {
            var_name: reg_name(var_reg),
            start: IrNode::IntConst(0),
            end: (**right).clone(),
            body: Box::new(SNode::Seq(rest)),
            index_reg: i as i64,
            index_name: None,
            var_reg: var_reg as i64,
        },
    ))
}

fn is_pure_expr(n: &IrNode) -> bool {
    let n = peel(n);
    match n {
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
        IrNode::PropGet { obj, .. } => is_pure_expr(&obj),
        _ => false,
    }
}

fn has_break(n: &SNode) -> bool {
    match n {
        SNode::Break => true,
        SNode::Seq(stmts) => stmts.iter().any(has_break),
        _ => false,
    }
}

fn iter_regsets<'a>(n: &'a SNode, acc: &mut Vec<&'a IrNode>) {
    match n {
        SNode::Expr(node @ IrNode::RegSet { .. }) => acc.push(node),
        SNode::Expr(_) => {}
        SNode::If { then_body, else_body, .. } => {
            iter_regsets(then_body, acc);
            if let Some(e) = else_body {
                iter_regsets(e, acc);
            }
        }
        SNode::While { body, .. }
        | SNode::DoWhile { body, .. }
        | SNode::ForIn { body, .. }
        | SNode::ForRange { body, .. } => iter_regsets(body, acc),
        SNode::Seq(stmts) => {
            for x in stmts {
                iter_regsets(x, acc);
            }
        }
        _ => {}
    }
}

fn try_match_map_loop(wh: &SNode, out: &[SNode]) -> Option<(Vec<SNode>, SNode)> {
    let SNode::While { cond: _, body } = wh else { return None };
    if let SNode::While { cond: Some(c), .. } = wh {
        if !matches!( *c, IrNode::BoolConst(true)) {
            return None;
        }
    }
    let body = seq_of(body);
    if body.is_empty() {
        return None;
    }

    let mut pos = 0usize;
    let mut copies: HashMap<usize, usize> = HashMap::new();
    let mut lf_def: Option<(usize, usize, usize, usize)> = None;
    let mut ni_reg: Option<usize> = None;
    let mut break_pos: Option<usize> = None;

    while pos < body.len() {
        let s = &body[pos];
        if let SNode::Expr(IrNode::RegSet { reg, value, .. }) = s {
            let v = peel(value);
            if let IrNode::RegAccess { reg: src, .. } = &v {
                copies.insert(*reg, *src);
                pos += 1;
                continue;
            }
            if let IrNode::HasNext { obj_reg, idx_reg, .. } = &v {
                lf_def = Some((pos, *reg, *obj_reg, *idx_reg));
                pos += 1;
                break;
            }
        }
        break;
    }
    let (_lf_pos, lf_reg, has_obj, has_idx) = lf_def?;
    let keys_reg = *copies.get(&has_obj).unwrap_or(&has_obj);
    let idx_reg = *copies.get(&has_idx).unwrap_or(&has_idx);

    if pos < body.len() {
        if let SNode::If { cond, .. } = &body[pos] {
            let mut cond_regs = HashSet::new();
            collect_regs(Some(cond), &mut cond_regs);
            let copies_keys: HashSet<usize> = copies.keys().copied().collect();
            if cond_regs.is_subset(&copies_keys.union(&HashSet::from([lf_reg])).copied().collect())
                || cond_regs.contains(&lf_reg)
            {
                pos += 1;
            }
        }
    }
    if pos < body.len() {
        if let SNode::Expr(IrNode::RegSet { reg, value, .. }) = &body[pos] {
            let v = peel(value);
            if let IrNode::RegAccess { reg: src, .. } = &v {
                if *src == idx_reg || *src == has_idx {
                    ni_reg = Some(*reg);
                    pos += 1;
                }
            }
        }
    }

    if pos < body.len() {
        if let SNode::If { cond, then_body, else_body } = &body[pos] {
            let c = peel(cond);
            let ok = matches!(&c, IrNode::UnaryOp { op, expr }
                if op == "not" && matches!(peel(expr), IrNode::RegAccess { reg: r, .. } if r == lf_reg));
            let else_empty = else_body.as_ref().map(|e| is_empty_seq(e)).unwrap_or(true);
            if ok && has_break(then_body) && else_empty {
                break_pos = Some(pos);
                pos += 1;
            }
        }
    }
    let _break_pos = break_pos?;

    let mut fetches: Vec<(&str, usize)> = Vec::new();
    let mut i = pos;
    while i < body.len() {
        let s = &body[i];
        if let SNode::Expr(IrNode::RegSet { reg, value, .. }) = s {
            let v = peel(value);
            match &v {
                IrNode::NextName { obj, .. } | IrNode::NextValue { obj, .. } => {
                    let ob = peel(obj);
                    if let IrNode::RegAccess { reg: or, .. } = &ob {
                        if *or == keys_reg {
                            let kind = if matches!(v, IrNode::NextName { .. }) { "key" } else { "value" };
                            fetches.push((kind, *reg));
                            i += 1;
                            continue;
                        }
                    }
                }
                IrNode::RegAccess { reg: src, .. } => {
                    if Some(*src) == ni_reg && *reg == idx_reg {
                        i += 1;
                        continue;
                    }
                }
                _ => {}
            }
        }
        break;
    }
    let consumed = i;
    if fetches.is_empty() {
        return None;
    }

    let mut map_expr: Option<IrNode> = None;
    let mut m_reg: Option<usize> = None;
    let mut res_reg: Option<usize> = None;
    for j in (0..out.len()).rev() {
        let s = &out[j];
        let SNode::Expr(IrNode::RegSet { reg, value, .. }) = s else { continue };
        let v = peel(value);
        if let IrNode::PropGet { obj, prop, .. } = &v {
            if let IrNode::NameRef { name: pname, .. } = &**prop {
                if pname == "h" {
                    let ob = peel(obj);
                    if let IrNode::RegAccess { reg: cand, .. } = &ob {
                        let cand = *cand;
                        if *reg == keys_reg || Some(cand) == m_reg {
                            m_reg = Some(cand);
                            for k2 in (0..j).rev() {
                                if let SNode::Expr(IrNode::RegSet { reg: r2, value: v2, .. }) = &out[k2] {
                                    if *r2 == cand {
                                        map_expr = Some((**v2).clone());
                                        break;
                                    }
                                }
                            }
                            if map_expr.is_none() {
                                map_expr = Some(IrNode::RegAccess { reg: cand, hint: None });
                            }
                            for s2 in out {
                                if let SNode::Expr(IrNode::RegSet { reg: r2, value: v2, .. }) = s2 {
                                    let v2p = peel(v2);
                                    if let IrNode::PropGet { obj: o2, prop: p2, .. } = &v2p {
                                        if let (IrNode::NameRef { name: n2, .. },
                                                IrNode::RegAccess { reg: o2r, .. }) =
                                            (&**p2, &peel(o2))
                                        {
                                            if n2 == "rh" && *o2r == cand {
                                                res_reg = Some(*r2);
                                            }
                                        }
                                    }
                                }
                            }
                            break;
                        }
                    }
                }
            }
        }
    }
    let m_reg = m_reg?;
    let mut map_expr = map_expr?;
    let mut keep_m_def = false;
    if !is_pure_expr(&map_expr) {
        keep_m_def = true;
        map_expr = IrNode::RegAccess { reg: m_reg, hint: None };
    }

    let mut plumbing: HashSet<usize> = HashSet::new();
    plumbing.insert(keys_reg);
    plumbing.insert(idx_reg);
    plumbing.insert(lf_reg);
    plumbing.insert(has_obj);
    plumbing.insert(has_idx);
    plumbing.extend(copies.keys().copied());
    if !keep_m_def {
        plumbing.insert(m_reg);
    }
    if let Some(r) = res_reg {
        plumbing.insert(r);
    }
    if let Some(r) = ni_reg {
        plumbing.insert(r);
    }
    for (_k, r) in &fetches {
        plumbing.insert(*r);
    }
    fn plumb_discover(s: &SNode, plumbing: &mut HashSet<usize>) {
        match s {
            SNode::Expr(IrNode::RegSet { reg, value, .. }) => {
                if plumbing.contains(reg) {
                    return;
                }
                let v = peel(value);
                match &v {
                    IrNode::HasNext { .. } | IrNode::BoolConst(_) => {
                        plumbing.insert(*reg);
                    }
                    IrNode::RegAccess { reg: src, .. } => {
                        if plumbing.contains(src) {
                            plumbing.insert(*reg);
                        }
                    }
                    _ => {}
                }
            }
            SNode::If { then_body, else_body, .. } => {
                for part in [then_body.as_ref()]
                    .into_iter()
                    .chain(else_body.as_deref())
                {
                    for ss in seq_of(part) {
                        plumb_discover(&ss, plumbing);
                    }
                }
            }
            _ => {}
        }
    }
    let mut changed = true;
    while changed {
        changed = false;
        for s in out {
            let before = plumbing.len();
            plumb_discover(s, &mut plumbing);
            if plumbing.len() != before {
                changed = true;
            }
        }
    }

    let mut renames: HashMap<usize, String> = HashMap::new();
    let mut used_names: HashSet<String> = HashSet::new();
    for (kind, r) in &fetches {
        let base = if *kind == "key" { "k" } else { "v" };
        let mut nm = base.to_string();
        let mut n2 = 2;
        while used_names.contains(&nm) {
            nm = format!("{}{}", base, n2);
            n2 += 1;
        }
        used_names.insert(nm.clone());
        renames.insert(*r, nm);
    }
    let mut changed = true;
    while changed {
        changed = false;
        for s in &body[consumed..] {
            let mut regsets = Vec::new();
            iter_regsets(s, &mut regsets);
            for rs in regsets {
                if let IrNode::RegSet { reg, value, .. } = rs {
                    let v = peel(value);
                    if let IrNode::RegAccess { reg: src, .. } = &v {
                        if let Some(nm) = renames.get(src) {
                            if !renames.contains_key(reg) {
                                renames.insert(*reg, nm.clone());
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
    }

    fn rename_walk(nd: &mut IrNode, renames: &HashMap<usize, String>) {
        match nd {
            IrNode::RegAccess { reg, hint } => {
                if let Some(nm) = renames.get(reg) {
                    *hint = Some(nm.clone());
                }
            }
            IrNode::RegSet { reg, value, hint } => {
                if let Some(nm) = renames.get(reg) {
                    *hint = Some(nm.clone());
                }
                rename_walk(value, renames);
            }
            other => {
                other.map_children_mut(&mut |c| rename_walk(c, renames));
            }
        }
    }

    fn is_plumbing_clause(
        s: &SNode,
        plumbing: &HashSet<usize>,
        renames: &HashMap<usize, String>,
    ) -> bool {
        let SNode::If { cond, then_body, else_body } = s else { return false };
        let mut cr = HashSet::new();
        collect_regs(Some(cond), &mut cr);
        if cr.is_empty() || !cr.is_subset(plumbing) {
            return false;
        }
        let mut defs = HashSet::new();
        let mut reads = HashSet::new();
        for part in [then_body.as_ref()].into_iter().chain(else_body.as_deref()) {
            collect_reg_info(part, &mut defs, &mut reads);
        }
        let allowed: HashSet<usize> = plumbing
            .union(&renames.keys().copied().collect())
            .copied()
            .collect();
        defs.is_subset(&allowed) && reads.is_subset(&allowed)
    }

    fn collect_reg_info(nd: &SNode, defs_out: &mut HashSet<usize>, reads_out: &mut HashSet<usize>) {
        match nd {
            SNode::Expr(node) => {
                if let IrNode::RegSet { reg, value, .. } = node {
                    defs_out.insert(*reg);
                    let mut cr = HashSet::new();
                    collect_regs(Some(value.as_ref()), &mut cr);
                    reads_out.extend(cr);
                } else {
                    let mut cr = HashSet::new();
                    collect_regs(Some(node), &mut cr);
                    reads_out.extend(cr);
                }
            }
            SNode::If { cond, then_body, else_body } => {
                let mut cr = HashSet::new();
                collect_regs(Some(cond), &mut cr);
                reads_out.extend(cr);
                collect_reg_info(then_body, defs_out, reads_out);
                if let Some(e) = else_body {
                    collect_reg_info(e, defs_out, reads_out);
                }
            }
            SNode::While { body, .. }
            | SNode::DoWhile { body, .. }
            | SNode::ForIn { body, .. }
            | SNode::ForRange { body, .. } => {
                collect_reg_info(body, defs_out, reads_out);
            }
            SNode::Return(Some(v)) => {
                let mut cr = HashSet::new();
                collect_regs(Some(v), &mut cr);
                reads_out.extend(cr);
            }
            SNode::Throw(v) => {
                let mut cr = HashSet::new();
                collect_regs(Some(v), &mut cr);
                reads_out.extend(cr);
            }
            SNode::Seq(stmts) => {
                for x in stmts {
                    collect_reg_info(x, defs_out, reads_out);
                }
            }
            _ => {}
        }
    }

    let mut rest: Vec<SNode> = Vec::new();
    for s in &body[consumed..] {
        if let SNode::Expr(IrNode::RegSet { reg, value, .. }) = s {
            let v = peel(value);
            if let IrNode::RegAccess { reg: src, .. } = &v {
                if renames.contains_key(src) {
                    continue;
                }
            }
            let _ = reg;
        }
        if is_plumbing_clause(s, &plumbing, &renames) {
            continue;
        }
        let mut s2 = s.clone();
        rename_snode(&mut s2, &renames);
        rest.push(s2);
    }
    fn rename_snode(s: &mut SNode, renames: &HashMap<usize, String>) {
        match s {
            SNode::Expr(node) => rename_walk(node, renames),
            SNode::If { cond, then_body, else_body } => {
                rename_walk(cond, renames);
                rename_snode(then_body, renames);
                if let Some(e) = else_body {
                    rename_snode(e, renames);
                }
            }
            SNode::While { cond, body } => {
                if let Some(c) = cond {
                    rename_walk(c, renames);
                }
                rename_snode(body, renames);
            }
            SNode::DoWhile { cond, body } => {
                rename_walk(cond, renames);
                rename_snode(body, renames);
            }
            SNode::ForIn { obj, body, .. } | SNode::ForRange { start: obj, body, .. } => {
                rename_walk(obj, renames);
                rename_snode(body, renames);
            }
            SNode::Switch { value, cases, match_base, case_plan, .. } => {
                rename_walk(value, renames);
                if let Some(mb) = match_base {
                    rename_walk(mb, renames);
                }
                for (_, b) in cases.iter_mut() {
                    rename_snode(b, renames);
                }
                if let Some(plan) = case_plan {
                    for (_, b) in plan.iter_mut() {
                        rename_snode(b, renames);
                    }
                }
            }
            SNode::Try { body, catches, finally_body } => {
                rename_snode(body, renames);
                for (_, _, b) in catches.iter_mut() {
                    rename_snode(b, renames);
                }
                if let Some(fb) = finally_body {
                    rename_snode(fb, renames);
                }
            }
            SNode::Return(Some(v)) => rename_walk(v, renames),
            SNode::Throw(v) => rename_walk(v, renames),
            SNode::Seq(stmts) => {
                for x in stmts.iter_mut() {
                    rename_snode(x, renames);
                }
            }
            _ => {}
        }
    }

    let mut key_name = "k".to_string();
    let mut val_name = String::new();
    for (kind, r) in &fetches {
        let nm = renames.get(r).cloned().unwrap_or_else(|| {
            if *kind == "key" { "k".to_string() } else { "v".to_string() }
        });
        if *kind == "key" {
            key_name = nm;
        } else {
            val_name = nm;
        }
    }
    let kind = if !val_name.is_empty() { "map" } else { "mapkey" };

    let mut new_out: Vec<SNode> = Vec::new();
    for s in out {
        let mut dropped = false;
        if let SNode::Expr(IrNode::RegSet { reg, .. }) = s {
            if plumbing.contains(reg) {
                dropped = true;
            }
        } else if let SNode::If { cond, then_body, else_body } = s {
            let mut cr = HashSet::new();
            collect_regs(Some(cond), &mut cr);
            let mut defs = HashSet::new();
            for ss in seq_of(then_body) {
                if let SNode::Expr(IrNode::RegSet { reg, .. }) = ss {
                    defs.insert(reg);
                }
            }
            let renamed_keys: HashSet<usize> = renames.keys().copied().collect();
            let allowed: HashSet<usize> =
                plumbing.union(&renamed_keys).copied().collect();
            if !cr.is_empty() && cr.is_subset(&plumbing) && !defs.is_empty() && defs.is_subset(&allowed) {
                dropped = true;
            } else if is_plumbing_clause(s, &plumbing, &renames) {
                dropped = true;
            }
            let _ = else_body;
        }
        if !dropped {
            if let SNode::Expr(IrNode::RegSet { reg, value, .. }) = s {
                if renames.contains_key(reg) {
                    let pv = peel(value);
                    if matches!(
                        pv,
                        IrNode::IntConst(_)
                            | IrNode::UIntConst(_)
                            | IrNode::NullConst
                            | IrNode::BoolConst(_)
                    ) {
                        continue;
                    }
                }
            }
            let mut s2 = s.clone();
            rename_snode(&mut s2, &renames);
            new_out.push(s2);
        }
    }

    let var_reg = fetches.iter().find(|(k, _)| *k == "key").map(|(_, r)| *r as i64).unwrap_or(-1);
    let val_reg = fetches.iter().find(|(k, _)| *k == "value").map(|(_, r)| *r as i64).unwrap_or(-1);
    let node = SNode::ForIn {
        var_name: key_name.clone(),
        kind: kind.to_string(),
        obj: map_expr,
        body: Box::new(SNode::Seq(rest)),
        val_name: if val_name.is_empty() { key_name } else { val_name },
        var_reg,
        val_reg,
    };
    Some((new_out, node))
}

fn try_match_key_iterator(wh: &SNode, out: &[SNode]) -> Option<(Vec<SNode>, SNode)> {
    let SNode::While { cond, body } = wh else { return None };
    let cond = cond.as_ref()?;
    let c = peel(cond);
    let IrNode::CallProp { obj, prop, args, .. } = &c else { return None };
    if args.len() != 0 {
        return None;
    }
    let IrNode::NameRef { name: pname, .. } = &**prop else { return None };
    if pname != "hasNext" {
        return None;
    }
    let xo = peel(obj);
    let IrNode::RegAccess { reg: x_reg, .. } = &xo else { return None };
    let x_reg = *x_reg;
    let body = seq_of(body);
    if body.is_empty() {
        return None;
    }
    let SNode::Expr(first_node) = &body[0] else { return None };
    let IrNode::RegSet { reg: key_reg, value: fv, .. } = first_node else { return None };
    let kv = peel(fv);
    let IrNode::CallProp { obj: kvobj, prop: kvprop, args: kvargs, .. } = &kv else {
        return None;
    };
    if !kvargs.is_empty() {
        return None;
    }
    let IrNode::NameRef { name: kvname, .. } = &**kvprop else { return None };
    if kvname != "next" {
        return None;
    }
    if unwrap_reg(&peel(kvobj)) != Some(x_reg) {
        return None;
    }
    let key_reg = *key_reg;

    let mut map_expr: Option<IrNode> = None;
    let mut m_reg: Option<usize> = None;
    let mut x_def_pos: Option<usize> = None;
    for j in (0..out.len()).rev() {
        let s = &out[j];
        let SNode::Expr(IrNode::RegSet { reg, value, .. }) = s else { continue };
        let v = peel(value);
        let empty_args: Vec<IrNode> = Vec::new();
        let (ctor_args, is_ctor) = match &v {
            IrNode::ConstructProp { args, .. } => (args, true),
            IrNode::Construct { args, .. } => (args, true),
            _ => (&empty_args, false),
        };
        if !is_ctor {
            continue;
        }
        if *reg == x_reg && !ctor_args.is_empty() {
            let a0 = peel(&ctor_args[0]);
            if let IrNode::PropGet { obj, prop, .. } = &a0 {
                if let IrNode::NameRef { name: pname, .. } = &**prop {
                    if pname == "h" {
                        let m_obj = peel(obj);
                        match &m_obj {
                            IrNode::RegAccess { reg: mr, .. } => {
                                let mr = *mr;
                                m_reg = Some(mr);
                                x_def_pos = Some(j);
                                for k2 in (0..j).rev() {
                                    if let SNode::Expr(IrNode::RegSet { reg: r2, value: v2, .. }) = &out[k2] {
                                        if *r2 == mr {
                                            map_expr = Some((**v2).clone());
                                            break;
                                        }
                                    }
                                }
                                if map_expr.is_none() {
                                    map_expr = Some(IrNode::RegAccess { reg: mr, hint: None });
                                }
                            }
                            other => {
                                m_reg = None;
                                x_def_pos = Some(j);
                                map_expr = Some(other.clone());
                            }
                        }
                        break;
                    }
                }
            }
        }
    }
    x_def_pos?;
    let mut map_expr = map_expr?;
    if !is_pure_expr(&map_expr) {
        map_expr = IrNode::RegAccess { reg: m_reg?, hint: None };
    }

    let mut renames: HashMap<usize, String> = HashMap::new();
    renames.insert(key_reg, "k".to_string());
    let mut changed = true;
    while changed {
        changed = false;
        for s in &body[1..] {
            let mut regsets = Vec::new();
            iter_regsets(s, &mut regsets);
            for rs in regsets {
                if let IrNode::RegSet { reg, value, .. } = rs {
                    let v = peel(value);
                    if let IrNode::RegAccess { reg: src, .. } = &v {
                        if let Some(nm) = renames.get(src) {
                            if !renames.contains_key(reg) {
                                renames.insert(*reg, nm.clone());
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
    }

    let mut rest: Vec<SNode> = Vec::new();
    for s in &body[1..] {
        if let SNode::Expr(IrNode::RegSet { reg: _, value, .. }) = s {
            let v = peel(value);
            if matches!(&v, IrNode::RegAccess { reg: src, .. } if renames.contains_key(src)) {
                continue;
            }
        }
        let mut s2 = s.clone();
        fn rn(nd: &mut IrNode, renames: &HashMap<usize, String>) {
            match nd {
                IrNode::RegAccess { reg, hint } => {
                    if let Some(nm) = renames.get(reg) {
                        *hint = Some(nm.clone());
                    }
                }
                IrNode::RegSet { reg, value, hint } => {
                    if let Some(nm) = renames.get(reg) {
                        *hint = Some(nm.clone());
                    }
                    rn(value, renames);
                }
                other => other.map_children_mut(&mut |c| rn(c, renames)),
            }
        }
        match &mut s2 {
            SNode::Expr(n) => rn(n, &renames),
            _ => {}
        }
        rest.push(s2);
    }

    let mut new_out: Vec<SNode> = Vec::new();
    for (i, s) in out.iter().enumerate() {
        if Some(i) == x_def_pos {
            continue;
        }
        if let SNode::Expr(IrNode::RegSet { reg, value, .. }) = s {
            if renames.contains_key(reg) {
                let pv = peel(value);
                if matches!(
                    pv,
                    IrNode::IntConst(_)
                        | IrNode::UIntConst(_)
                        | IrNode::NullConst
                        | IrNode::BoolConst(_)
                ) {
                    continue;
                }
            }
        }
        new_out.push(s.clone());
    }

    let node = SNode::ForIn {
        var_name: "k".to_string(),
        kind: "mapkey".to_string(),
        obj: map_expr,
        body: Box::new(SNode::Seq(rest)),
        val_name: "k".to_string(),
        var_reg: key_reg as i64,
        val_reg: -1,
    };
    Some((new_out, node))
}

pub fn apply_map_iterations(tree: SNode) -> SNode {
    match tree {
        SNode::Seq(mut stmts) => {
            let mut out: Vec<SNode> = Vec::new();
            for s in stmts.drain(..) {
                match s {
                    SNode::Seq(_) => {
                        let inner = s;
                        out.push(apply_map_iterations(inner));
                    }
                    SNode::While { cond, body } => {
                        let cond_true = match &cond {
                            None => true,
                            Some(c) => matches!( *c, IrNode::BoolConst(true)),
                        };
                        let wh = SNode::While { cond: cond.clone(), body: body.clone() };
                        if cond_true {
                            if let Some((new_out, node)) = try_match_map_loop(&wh, &out) {
                                let mut node = node;
                                if let SNode::ForIn { body: b, .. } = &mut node {
                                    let nb = std::mem::replace(b.as_mut(), SNode::empty_seq());
                                    *b.as_mut() = apply_map_iterations(nb);
                                }
                                out = new_out;
                                out.push(node);
                                continue;
                            }
                        }
                        if let Some((new_out, node)) = try_match_key_iterator(&wh, &out) {
                            let mut node = node;
                            if let SNode::ForIn { body: b, .. } = &mut node {
                                let nb = std::mem::replace(b.as_mut(), SNode::empty_seq());
                                *b.as_mut() = apply_map_iterations(nb);
                            }
                            out = new_out;
                            out.push(node);
                            continue;
                        }
                        let mut s2 = SNode::While { cond, body };
                        if let SNode::While { body: b, .. } = &mut s2 {
                            let nb = std::mem::replace(b.as_mut(), SNode::empty_seq());
                            *b.as_mut() = apply_map_iterations(nb);
                        }
                        out.push(s2);
                    }
                    SNode::If { cond, then_body, else_body } => {
                        let s2 = SNode::If {
                            cond,
                            then_body: Box::new(apply_map_iterations(*then_body)),
                            else_body: else_body
                                .map(|e| Box::new(apply_map_iterations(*e))),
                        };
                        out.push(s2);
                    }
                    SNode::Try { body, catches, finally_body } => {
                        let mut new_catches = Vec::new();
                        for (v, t, b) in catches {
                            new_catches.push((v, t, apply_map_iterations(b)));
                        }
                        out.push(SNode::Try {
                            body: Box::new(apply_map_iterations(*body)),
                            catches: new_catches,
                            finally_body: finally_body
                                .map(|f| Box::new(apply_map_iterations(*f))),
                        });
                    }
                    other => out.push(other),
                }
            }
            SNode::Seq(out)
        }
        SNode::If { cond, then_body, else_body } => SNode::If {
            cond,
            then_body: Box::new(apply_map_iterations(*then_body)),
            else_body: else_body.map(|e| Box::new(apply_map_iterations(*e))),
        },
        SNode::While { cond, body } => SNode::While {
            cond,
            body: Box::new(apply_map_iterations(*body)),
        },
        SNode::DoWhile { cond, body } => SNode::DoWhile {
            cond,
            body: Box::new(apply_map_iterations(*body)),
        },
        SNode::ForIn { var_name, kind, obj, body, val_name, var_reg, val_reg } => SNode::ForIn {
            var_name,
            kind,
            obj,
            body: Box::new(apply_map_iterations(*body)),
            val_name,
            var_reg,
            val_reg,
        },
        SNode::ForRange { var_name, start, end, body, index_reg, index_name, var_reg } => {
            SNode::ForRange {
                var_name,
                start,
                end,
                body: Box::new(apply_map_iterations(*body)),
                index_reg,
                index_name,
                var_reg,
            }
        }
        other => other,
    }
}

fn try_match_hasnext(wh: &SNode) -> Option<SNode> {
    let SNode::While { cond: Some(cond), body } = wh else { return None };
    let IrNode::HasNext { obj_reg, idx_reg, .. } = cond else { return None };
    let _ = (obj_reg, idx_reg);
    let body = seq_of(body);
    if body.is_empty() {
        return None;
    }
    let SNode::Expr(IrNode::RegSet { reg: var_reg, value, .. }) = &body[0] else {
        return None;
    };
    let v = peel(value);
    let (kind, obj) = match &v {
        IrNode::NextName { obj, .. } => ("key", (**obj).clone()),
        IrNode::NextValue { obj, .. } => ("value", (**obj).clone()),
        IrNode::UnaryOp { op, expr } if op == "nextname" || op == "nextvalue" => {
            (if op == "nextname" { "key" } else { "value" }, (**expr).clone())
        }
        _ => return None,
    };
    let var_reg = *var_reg;
    let mut var = reg_name(var_reg);
    if var.starts_with("__r") || var.starts_with("_variable") {
        var = if kind == "key" { "k".to_string() } else { "v".to_string() };
    }
    Some(SNode::ForIn {
        var_name: var,
        kind: kind.to_string(),
        obj,
        body: Box::new(SNode::Seq(body[1..].to_vec())),
        val_name: String::new(),
        var_reg: var_reg as i64,
        val_reg: -1,
    })
}

pub fn apply_haxe_loop_patterns(tree: SNode) -> SNode {
    match tree {
        SNode::Seq(mut stmts) => {
            let mut out: Vec<SNode> = Vec::new();
            for s in stmts.drain(..) {
                match s {
                    SNode::Seq(_) => {
                        out.push(apply_haxe_loop_patterns(s));
                    }
                    SNode::While { cond, body } => {
                        let is_while_true = match &cond {
                            None => true,
                            Some(c) => matches!( *c, IrNode::BoolConst(true)),
                        };
                        let wh = SNode::While { cond: cond.clone(), body: body.clone() };
                        if is_while_true {
                            if let Some(mut hn) = try_match_hasnext(&wh) {
                                if let SNode::ForIn { body: b, .. } = &mut hn {
                                    let nb = std::mem::replace(b.as_mut(), SNode::empty_seq());
                                    *b.as_mut() = apply_haxe_loop_patterns(nb);
                                }
                                out.push(hn);
                                continue;
                            }
                        }
                        if is_while_true {
                            if let Some((new_out, mut node)) = try_match_loop(&wh, &out) {
                                match &mut node {
                                    SNode::ForIn { body: b, .. } | SNode::ForRange { body: b, .. } => {
                                        let nb = std::mem::replace(b.as_mut(), SNode::empty_seq());
                                        *b.as_mut() = apply_haxe_loop_patterns(nb);
                                    }
                                    _ => {}
                                }
                                out = new_out;
                                out.push(node);
                                continue;
                            }
                        }
                        let mut s2 = SNode::While { cond, body };
                        if let SNode::While { body: b, .. } = &mut s2 {
                            let nb = std::mem::replace(b.as_mut(), SNode::empty_seq());
                            *b.as_mut() = apply_haxe_loop_patterns(nb);
                        }
                        out.push(s2);
                    }
                    SNode::DoWhile { cond, body } => {
                        let mut s2 = SNode::DoWhile { cond, body };
                        if let SNode::DoWhile { body: b, .. } = &mut s2 {
                            let nb = std::mem::replace(b.as_mut(), SNode::empty_seq());
                            *b.as_mut() = apply_haxe_loop_patterns(nb);
                        }
                        out.push(s2);
                    }
                    SNode::If { cond, then_body, else_body } => {
                        out.push(SNode::If {
                            cond,
                            then_body: Box::new(apply_haxe_loop_patterns(*then_body)),
                            else_body: else_body.map(|e| Box::new(apply_haxe_loop_patterns(*e))),
                        });
                    }
                    SNode::Try { body, catches, finally_body } => {
                        let new_catches = catches
                            .into_iter()
                            .map(|(v, t, b)| (v, t, apply_haxe_loop_patterns(b)))
                            .collect();
                        out.push(SNode::Try {
                            body: Box::new(apply_haxe_loop_patterns(*body)),
                            catches: new_catches,
                            finally_body: finally_body
                                .map(|f| Box::new(apply_haxe_loop_patterns(*f))),
                        });
                    }
                    other => out.push(other),
                }
            }
            SNode::Seq(out)
        }
        SNode::If { cond, then_body, else_body } => SNode::If {
            cond,
            then_body: Box::new(apply_haxe_loop_patterns(*then_body)),
            else_body: else_body.map(|e| Box::new(apply_haxe_loop_patterns(*e))),
        },
        SNode::While { cond, body } => SNode::While {
            cond,
            body: Box::new(apply_haxe_loop_patterns(*body)),
        },
        SNode::DoWhile { cond, body } => SNode::DoWhile {
            cond,
            body: Box::new(apply_haxe_loop_patterns(*body)),
        },
        other => other,
    }
}

fn collect_read_temps_walk_node(nd: &IrNode, acc: &mut HashSet<usize>) {
    match nd {
        IrNode::RegAccess { reg, .. } if *reg >= 10000 => {
            acc.insert(*reg);
        }
        other => {
            other.for_each_child(&mut |c| collect_read_temps_walk_node(c, acc));
        }
    }
}

fn collect_read_temps(n: &SNode, acc: &mut HashSet<usize>) {
    match n {
        SNode::Seq(stmts) => {
            for s in stmts {
                collect_read_temps(s, acc);
            }
        }
        SNode::Expr(node) => collect_read_temps_walk_node(node, acc),
        SNode::If { then_body, else_body, .. } => {
            collect_read_temps(then_body, acc);
            if let Some(e) = else_body {
                collect_read_temps(e, acc);
            }
        }
        SNode::While { body, .. }
        | SNode::DoWhile { body, .. }
        | SNode::ForIn { body, .. }
        | SNode::ForRange { body, .. } => collect_read_temps(body, acc),
        SNode::Try { body, catches, finally_body } => {
            collect_read_temps(body, acc);
            for (_, _, b) in catches {
                collect_read_temps(b, acc);
            }
            if let Some(fb) = finally_body {
                collect_read_temps(fb, acc);
            }
        }
        SNode::Return(Some(v)) | SNode::Throw(v) => collect_read_temps_walk_node(v, acc),
        _ => {}
    }
}

fn drop_unused_temps(n: SNode, used: &HashSet<usize>) -> SNode {
    match n {
        SNode::Seq(stmts) => {
            let mut out = Vec::with_capacity(stmts.len());
            for s in stmts {
                match s {
                    SNode::Expr(IrNode::Block(block_stmts)) => {
                        let kept: Vec<IrNode> = block_stmts
                            .into_iter()
                            .filter(|st| match st {
                                IrNode::RegSet { reg, .. } => {
                                    !(*reg >= 10000 && !used.contains(reg))
                                }
                                _ => true,
                            })
                            .collect();
                        if !kept.is_empty() {
                            out.push(SNode::Expr(IrNode::Block(kept)));
                        }
                    }
                    SNode::Expr(IrNode::RegSet { reg, .. }) if reg >= 10000 && !used.contains(&reg) => {
                    }
                    other => out.push(drop_unused_temps(other, used)),
                }
            }
            SNode::Seq(out)
        }
        SNode::If { cond, then_body, else_body } => SNode::If {
            cond,
            then_body: Box::new(drop_unused_temps(*then_body, used)),
            else_body: else_body.map(|e| Box::new(drop_unused_temps(*e, used))),
        },
        SNode::While { cond, body } => SNode::While {
            cond,
            body: Box::new(drop_unused_temps(*body, used)),
        },
        SNode::DoWhile { cond, body } => SNode::DoWhile {
            cond,
            body: Box::new(drop_unused_temps(*body, used)),
        },
        SNode::ForIn { var_name, kind, obj, body, val_name, var_reg, val_reg } => SNode::ForIn {
            var_name,
            kind,
            obj,
            body: Box::new(drop_unused_temps(*body, used)),
            val_name,
            var_reg,
            val_reg,
        },
        SNode::ForRange { var_name, start, end, body, index_reg, index_name, var_reg } => {
            SNode::ForRange {
                var_name,
                start,
                end,
                body: Box::new(drop_unused_temps(*body, used)),
                index_reg,
                index_name,
                var_reg,
            }
        }
        SNode::Try { body, catches, finally_body } => {
            let new_catches = catches
                .into_iter()
                .map(|(v, t, b)| (v, t, drop_unused_temps(b, used)))
                .collect();
            SNode::Try {
                body: Box::new(drop_unused_temps(*body, used)),
                catches: new_catches,
                finally_body: finally_body.map(|f| Box::new(drop_unused_temps(*f, used))),
            }
        }
        other => other,
    }
}

pub fn structure_method(abc: &AbcFile, body: &MethodBody) -> SNode {
    let (blocks, ordered) = build_cfg(abc, body, 20000);
    if ordered.is_empty() {
        return SNode::empty_seq();
    }
    let mut st = Structurer::new(abc, body, blocks, ordered);
    let entry = st.ordered[0].start_pc;
    let empty: HashSet<usize> = HashSet::new();
    let tree = st.region(entry, &empty, None);
    let tree = apply_haxe_loop_patterns(tree);
    let tree = apply_map_iterations(tree);
    let mut used = HashSet::new();
    collect_read_temps(&tree, &mut used);
    drop_unused_temps(tree, &used)
}
