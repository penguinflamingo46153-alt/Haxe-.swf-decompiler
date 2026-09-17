use crate::abc::{AbcFile, ClassInfo, Instruction, MethodBody, MethodInfo, Trait, TraitKind, ValueKind};

pub fn py_repr_str(s: &str) -> String {
    if s.contains('\'') && !s.contains('"') {
        format!("\"{}\"", s)
    } else {
        format!("'{}'", s)
    }
}

pub fn namespace_str(ctx: &AbcFile, idx: u32) -> String {
    if idx == 0 {
        return "<none>".to_string();
    }
    let ns = match ctx.namespaces.get((idx - 1) as usize) {
        Some(n) => n,
        None => return format!("<invalid ns #{}>", idx),
    };
    let pfx = match ns.kind as u8 {
        0x05 => "private",
        0x08 => "ns",
        0x16 => "public",
        0x17 => "internal",
        0x18 => "protected",
        0x19 => "explicit",
        0x1A => "static_protected",
        k => return format!("ns({:02x})", k),
    };
    let name = ns.resolve(ctx);
    if !name.is_empty() {
        format!("{}:{}", pfx, name)
    } else {
        pfx.to_string()
    }
}

pub fn mn_str(ctx: &AbcFile, idx: u32) -> String {
    if idx == 0 {
        return "*".to_string();
    }
    match ctx.multinames.get((idx - 1) as usize) {
        Some(mn) => mn.resolve(ctx),
        None => format!("<invalid mn #{}>", idx),
    }
}

pub fn trait_str(ctx: &AbcFile, t: &Trait, indent: &str) -> String {
    let name = mn_str(ctx, t.name_idx);
    let mut attrs = Vec::new();
    if (t.attrs & 0x10) != 0 {
        attrs.push("final");
    }
    if (t.attrs & 0x20) != 0 {
        attrs.push("override");
    }
    let attr_str = if !attrs.is_empty() {
        format!("{} ", attrs.join(" "))
    } else {
        "".to_string()
    };

    match t.kind {
        TraitKind::Slot | TraitKind::Const => {
            let kw = if t.kind == TraitKind::Const {
                "const"
            } else {
                "var"
            };
            let ty = mn_str(ctx, t.type_idx);
            let mut val = String::new();
            if let Some(ref v) = t.value {
                if v.kind != ValueKind::Undefined {
                    val = format!(" = {}", v.resolve(ctx));
                }
            }
            format!(
                "{}{}{} {}: {}{}   ; slot {}",
                indent, attr_str, kw, name, ty, val, t.slot_id
            )
        }
        TraitKind::Method | TraitKind::Getter | TraitKind::Setter => {
            let kw = match t.kind {
                TraitKind::Method => "function",
                TraitKind::Getter => "get",
                TraitKind::Setter => "set",
                _ => "function",
            };
            let m = ctx.methods.get(t.method_idx);
            if m.is_none() {
                return format!(
                    "{}{}{} {} #method{} ; slot {}",
                    indent, attr_str, kw, name, t.method_idx, t.slot_id
                );
            }
            let m = m.unwrap();
            let mut args = Vec::new();
            let mut pcount = 0;
            for (i, &p) in m.params.iter().enumerate() {
                let pname = if !m.param_names.is_empty()
                    && i < m.param_names.len()
                    && m.param_names[i] > 0
                    && ((m.param_names[i] - 1) as usize) < ctx.strings.len()
                {
                    ctx.strings[(m.param_names[i] - 1) as usize].clone()
                } else {
                    let s = format!("p{}", pcount);
                    pcount += 1;
                    s
                };
                args.push(format!("{}:{}", pname, mn_str(ctx, p)));
            }
            let ret = mn_str(ctx, m.returns);
            let mut extras = Vec::new();
            if m.native() {
                extras.push("native");
            }
            if m.var_args() {
                extras.push("...");
            }
            if m.need_activation() {
                extras.push("activation");
            }
            let extra = if !extras.is_empty() {
                format!(" {}", extras.join(" "))
            } else {
                "".to_string()
            };
            format!(
                "{}{}{} {}({}){}: {}   ; method #{} slot {}",
                indent,
                attr_str,
                kw,
                name,
                args.join(", "),
                extra,
                ret,
                t.method_idx,
                t.slot_id
            )
        }
        TraitKind::Class => {
            let cidx = t.class_idx;
            let cls = ctx.classes.get(cidx);
            let cls_name = if let Some(c) = cls {
                mn_str(ctx, c.name_idx)
            } else {
                format!("class#{}", cidx)
            };
            format!(
                "{}{}class {} = {} ; slot {}",
                indent, attr_str, name, cls_name, t.slot_id
            )
        }
        TraitKind::Function => {
            format!(
                "{}{}{}function {} #method{} ; slot {}",
                indent, attr_str, indent, name, t.method_idx, t.slot_id
            )
        }
        _ => format!(
            "{}{}<trait {:?}> {} slot {}",
            indent, attr_str, t.kind, name, t.slot_id
        ),
    }
}

pub fn method_name(ctx: &AbcFile, m: &MethodInfo) -> String {
    if m.debug_name_idx > 0 && ((m.debug_name_idx - 1) as usize) < ctx.strings.len() {
        ctx.strings[(m.debug_name_idx - 1) as usize].clone()
    } else {
        format!("<method #{}>", m.idx)
    }
}

pub fn dump_instruction(ctx: &AbcFile, ins: &Instruction) -> String {
    let name = ins.name;
    if !ins.operands.is_empty() {
        if matches!(
            name,
            "getsuper"
                | "setsuper"
                | "findpropstrict"
                | "findproperty"
                | "finddef"
                | "getlex"
                | "setproperty"
                | "getproperty"
                | "initproperty"
                | "deleteproperty"
                | "getdescendants"
                | "cast"
                | "astype"
                | "istype"
                | "callsuper"
                | "callproperty"
                | "constructprop"
                | "callproplex"
                | "callsupervoid"
                | "callpropvoid"
        ) {
            let mn = mn_str(ctx, ins.operands[0] as u32);
            let args_part = if ins.operands.len() > 1 {
                format!(" ({} args)", ins.operands[1])
            } else {
                "".to_string()
            };
            return format!("  {:04}: {} {}{}", ins.offset, name, mn, args_part);
        }
        if name == "pushstring" {
            let idx = ins.operands[0] as usize;
            let s = if idx > 0 && idx <= ctx.strings.len() {
                &ctx.strings[idx - 1]
            } else {
                ""
            };
            return format!("  {:04}: pushstring \"{}\"", ins.offset, s);
        }
        if name == "pushint" {
            let idx = ins.operands[0] as usize;
            let val = if idx > 0 && idx <= ctx.ints.len() {
                ctx.ints[idx - 1]
            } else {
                0
            };
            return format!("  {:04}: pushint {}", ins.offset, val);
        }
        if name == "pushuint" {
            let idx = ins.operands[0] as usize;
            let val = if idx > 0 && idx <= ctx.uints.len() {
                ctx.uints[idx - 1]
            } else {
                0
            };
            return format!("  {:04}: pushuint {}", ins.offset, val);
        }
        if name == "pushdouble" {
            let idx = ins.operands[0] as usize;
            let val = if idx > 0 && idx <= ctx.doubles.len() {
                ctx.doubles[idx - 1]
            } else {
                0.0
            };
            return format!("  {:04}: pushdouble {:?}", ins.offset, val);
        }
        if name == "pushnamespace" {
            let idx = ins.operands[0] as u32;
            return format!(
                "  {:04}: pushnamespace {}",
                ins.offset,
                namespace_str(ctx, idx)
            );
        }
        if matches!(
            name,
            "getlocal"
                | "setlocal"
                | "kill"
                | "inclocal"
                | "declocal"
                | "inclocal_i"
                | "declocal_i"
        ) {
            return format!("  {:04}: {} r{}", ins.offset, name, ins.operands[0]);
        }
        if name == "pushbyte" {
            return format!("  {:04}: pushbyte {}", ins.offset, ins.operands[0]);
        }
        if name == "pushshort" {
            return format!("  {:04}: pushshort {}", ins.offset, ins.operands[0]);
        }
        if name == "dxns" {
            let idx = ins.operands[0] as usize;
            let s = if idx > 0 && idx <= ctx.strings.len() {
                &ctx.strings[idx - 1]
            } else {
                ""
            };
            return format!("  {:04}: dxns \"{}\"", ins.offset, s);
        }
        if matches!(
            name,
            "jmp"
                | "iftrue"
                | "iffalse"
                | "ifeq"
                | "ifne"
                | "iflt"
                | "ifnle"
                | "ifgt"
                | "ifnge"
                | "ifstricteq"
                | "ifstrictne"
                | "ifnlt"
                | "ifngt"
        ) {
            let shown = (ins.offset as i64) + ins.operands[0];
            return format!(
                "  {:04}: {} {:+}  (target {})",
                ins.offset, name, shown, shown
            );
        }
        if name == "lookupswitch" {
            let default_off = ins.operands[0];
            let default_target = (ins.offset as i64) + default_off;
            let case_targets: Vec<i64> = ins
                .cases
                .iter()
                .map(|&c| (ins.offset as i64) + (c as i64))
                .collect();
            return format!(
                "  {:04}: switch default->{} cases={:?}",
                ins.offset, default_target, case_targets
            );
        }
        if name == "newfunction" {
            return format!("  {:04}: newfunction #{}", ins.offset, ins.operands[0]);
        }
        if name == "newclass" {
            return format!("  {:04}: newclass #{}", ins.offset, ins.operands[0]);
        }
        if matches!(
            name,
            "call" | "construct" | "constructsuper" | "newobject" | "newarray" | "applytype"
        ) {
            return format!("  {:04}: {} ({})", ins.offset, name, ins.operands[0]);
        }
        if name == "callmethod" {
            return format!(
                "  {:04}: callmethod slot:{} ({} args)",
                ins.offset, ins.operands[0], ins.operands[1]
            );
        }
        if name == "callstatic" {
            return format!(
                "  {:04}: callstatic #{} ({} args)",
                ins.offset, ins.operands[0], ins.operands[1]
            );
        }
        if name == "getslot" || name == "setslot" {
            return format!("  {:04}: {} slot {}", ins.offset, name, ins.operands[0]);
        }
        if name == "hasnext2" {
            return format!(
                "  {:04}: hasnext2 r{},r{}",
                ins.offset, ins.operands[0], ins.operands[1]
            );
        }
        if name == "newcatch" {
            return format!("  {:04}: newcatch #{}", ins.offset, ins.operands[0]);
        }
        if name == "debug" && ins.operands.len() >= 4 {
            let idx = ins.operands[1] as usize;
            let reg = ins.operands[2];
            let line = ins.operands[3];
            let nm = if idx > 0 && idx <= ctx.strings.len() {
                &ctx.strings[idx - 1]
            } else {
                "?"
            };
            return format!(
                "  {:04}: .debug reg=r{} name={} line={}",
                ins.offset, reg, py_repr_str(nm), line
            );
        }
        if name == "getglobalslot" || name == "setglobalslot" {
            return format!("  {:04}: {} slot {}", ins.offset, name, ins.operands[0]);
        }
        if name == "debugline" {
            return format!("  {:04}: .line {}", ins.offset, ins.operands[0]);
        }
        if name == "debugfile" {
            let idx = ins.operands[0] as usize;
            let s = if idx > 0 && idx <= ctx.strings.len() {
                &ctx.strings[idx - 1]
            } else {
                "?"
            };
            return format!("  {:04}: .file {}", ins.offset, s);
        }
        if name == "getscope" {
            return format!("  {:04}: getscope depth {}", ins.offset, ins.operands[0]);
        }
        return format!("  {:04}: {} {:?}", ins.offset, name, ins.operands);
    }
    format!("  {:04}: {}", ins.offset, name)
}

pub fn dump_method_body(ctx: &AbcFile, body: &MethodBody) -> String {
    let mut lines = Vec::new();
    let m = ctx.methods.get(body.method_idx);
    let mname = if let Some(m) = m {
        method_name(ctx, m)
    } else {
        format!("<method {}>", body.method_idx)
    };
    lines.push(format!("  --- method #{} {} ---", body.method_idx, mname));
    lines.push(format!(
        "  stack={} locals={} scope_depth={}..{}",
        body.max_stack, body.local_count, body.init_scope_depth, body.max_scope_depth
    ));

    for exc in &body.exceptions {
        lines.push(format!(
            "  try @{}-{} catch @{} type={} name={}",
            exc.from_off,
            exc.to_off,
            exc.target_off,
            mn_str(ctx, exc.type_idx),
            mn_str(ctx, exc.name_idx)
        ));
    }

    for t in &body.traits {
        lines.push(trait_str(ctx, t, "    local "));
    }

    for ins in &body.instructions {
        lines.push(dump_instruction(ctx, ins));
    }

    lines.join("\n")
}

pub fn dump_class(ctx: &AbcFile, c: &ClassInfo) -> String {
    let mut lines = Vec::new();
    let mut flags: Vec<String> = Vec::new();
    if c.final_class() {
        flags.push("final".to_string());
    }
    if c.sealed() {
        flags.push("sealed".to_string());
    } else {
        flags.push("dynamic".to_string());
    }
    if c.interface() {
        flags.push("interface".to_string());
    }
    if c.protected_ns_idx != 0 {
        let ns_str = namespace_str(ctx, c.protected_ns_idx);
        if !ns_str.is_empty() {
            flags.push(ns_str);
        }
    }
    let name = mn_str(ctx, c.name_idx);
    let super_ = if c.super_idx != 0 {
        mn_str(ctx, c.super_idx)
    } else {
        "".to_string()
    };
    let impls = c
        .interfaces
        .iter()
        .map(|&i| mn_str(ctx, i))
        .collect::<Vec<_>>()
        .join(", ");
    let mut decl = format!("{} class {}", flags.join(" "), name);
    if !super_.is_empty() {
        decl.push_str(&format!(" extends {}", super_));
    }
    if !impls.is_empty() {
        decl.push_str(&format!(" implements {}", impls));
    }
    lines.push(format!("{} {{", decl));
    for t in &c.traits {
        lines.push(trait_str(ctx, t, "    "));
    }
    lines.push("    ; --- static ---".to_string());
    for t in &c.static_traits {
        lines.push(trait_str(ctx, t, "    "));
    }
    lines.push(format!("    ; constructor #{}", c.constructor_idx));
    lines.push(format!("    ; static init #{}", c.static_ctor_idx));
    lines.push("}".to_string());
    lines.join("\n")
}

pub fn dump_abc(ctx: &AbcFile, label: Option<&str>, show_code: bool) -> String {
    let mut out = Vec::new();
    if let Some(lbl) = label {
        out.push(format!("===== ABC: {} =====", lbl));
    }
    out.push(format!(
        "Pools: ints={} uints={} doubles={} strings={} ns={} nssets={} names={} methods={} metadatas={} classes={} scripts={} bodies={}",
        ctx.ints.len(),
        ctx.uints.len(),
        ctx.doubles.len(),
        ctx.strings.len(),
        ctx.namespaces.len(),
        ctx.nsets.len(),
        ctx.multinames.len(),
        ctx.methods.len(),
        ctx.metadatas.len(),
        ctx.classes.len(),
        ctx.scripts.len(),
        ctx.bodies.len()
    ));

    if show_code {
        out.push("\n--- Strings ---".to_string());
        for (i, s) in ctx.strings.iter().enumerate() {
            out.push(format!("  {:4}: {}", i + 1, py_repr_str(s)));
        }
    }

    out.push("\n--- Classes ---".to_string());
    for c in &ctx.classes {
        out.push(dump_class(ctx, c));
        out.push("".to_string());
    }

    out.push("--- Scripts ---".to_string());
    for (i, s) in ctx.scripts.iter().enumerate() {
        out.push(format!("Script #{} init=#{}", i, s.init_method_idx));
        for t in &s.traits {
            out.push(trait_str(ctx, t, "  "));
        }
        out.push("".to_string());
    }

    if show_code {
        out.push("--- Method Bodies ---".to_string());
        for b in &ctx.bodies {
            out.push(dump_method_body(ctx, b));
            out.push("".to_string());
        }
    }

    out.join("\n")
}

pub fn value_str(ctx: &AbcFile, v: Option<u32>) -> String {
    match v {
        None => "?".to_string(),
        Some(idx) => mn_str(ctx, idx),
    }
}
