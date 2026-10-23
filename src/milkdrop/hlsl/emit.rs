//! Checking the types of a shader's HLSL and writing it out as WGSL: HLSL converts between
//! numbers, vectors and booleans freely, WGSL never, so every conversion HLSL would make is
//! written out. Multi-component swizzles cannot be assigned to in WGSL, so they are assigned
//! a component at a time; `for` loops become `loop`s with a `continuing` block.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use super::Error;
use super::syntax::{BinOp, Expr, ExprKind, Function, Item, ParamMode, Stmt, UnOp, VarDecl};
use super::types::{Scalar, Type, float_literal};

/// A texture the shader samples.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Texture {
    Main,
    Blur(u8),
    NoiseLq,
    NoiseLqLite,
    NoiseMq,
    NoiseHq,
    NoiseVolLq,
    NoiseVolHq,
    /// A texture from the presets' textures, by its index in [`Used::user`].
    User(usize),
}

impl Texture {
    /// The WGSL name it is bound under.
    pub(super) fn binding_name(&self) -> String {
        match self {
            Texture::Main => "t_main".into(),
            Texture::Blur(n) => format!("t_blur{n}"),
            Texture::NoiseLq => "t_noise_lq".into(),
            Texture::NoiseLqLite => "t_noise_lq_lite".into(),
            Texture::NoiseMq => "t_noise_mq".into(),
            Texture::NoiseHq => "t_noise_hq".into(),
            Texture::NoiseVolLq => "t_noisevol_lq".into(),
            Texture::NoiseVolHq => "t_noisevol_hq".into(),
            Texture::User(i) => format!("t_user{i}"),
        }
    }

    pub(super) fn is_volume(&self) -> bool {
        matches!(self, Texture::NoiseVolLq | Texture::NoiseVolHq)
    }
}

/// How a sampler filters and what it does past the edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Filter {
    LinearWrap,
    LinearClamp,
    PointWrap,
    PointClamp,
}

impl Filter {
    pub(super) fn binding_name(self) -> &'static str {
        match self {
            Filter::LinearWrap => "s_linear_wrap",
            Filter::LinearClamp => "s_linear_clamp",
            Filter::PointWrap => "s_point_wrap",
            Filter::PointClamp => "s_point_clamp",
        }
    }
}

/// What the translated shader uses, so that only that is declared and bound.
#[derive(Debug, Default)]
pub(super) struct Used {
    pub textures: HashSet<Texture>,
    pub filters: HashSet<Filter>,
    /// User textures by name (lower case), in the order first used.
    pub user: Vec<String>,
    /// The highest blur texture used, 0 for none.
    pub blur: u8,
    pub inputs: bool,
}

/// A sampler as the shader names it: `sampler_[fw_|fc_|pw_|pc_]texture`.
#[derive(Clone, Debug)]
struct SamplerRef {
    texture: Texture,
    filter: Filter,
}

/// Where an expression can be assigned to.
#[derive(Clone, Debug)]
enum Place {
    /// The expression's code is a WGSL reference.
    Whole,
    /// Some components of a vector, which WGSL assigns one at a time.
    Swizzle { base: String, comps: Vec<u8> },
}

/// A checked expression: its WGSL code and type.
#[derive(Clone, Debug)]
struct Val {
    code: String,
    ty: Type,
    /// Its value, when it is a plain number written in the shader.
    lit: Option<f64>,
    place: Option<Place>,
    /// A pointer to it, when it can be passed to an `out` parameter.
    address: Option<String>,
    sampler: Option<SamplerRef>,
}

impl Val {
    fn new(code: String, ty: Type) -> Val {
        Val {
            code,
            ty,
            lit: None,
            place: None,
            address: None,
            sampler: None,
        }
    }

    fn lit(v: f64, ty: Type) -> Val {
        let code = match ty {
            Type::Scalar(Scalar::Float) => float_literal(v),
            Type::Scalar(Scalar::Bool) => (v != 0.0).to_string(),
            // Integers in range by construction (they came from the shader as integers).
            _ => format!("{}", v as i64),
        };
        Val {
            lit: Some(v),
            ..Val::new(code, ty)
        }
    }
}

/// A variable in scope.
#[derive(Clone, Debug)]
struct Var {
    code: String,
    ty: Type,
    constant: bool,
    /// The pointer it is reached through, for `out` and `inout` parameters.
    pointer: Option<String>,
    /// Whether it lives in the function (and so can be passed to `out` parameters).
    local: bool,
}

#[derive(Clone, Debug)]
struct Signature {
    wgsl: String,
    params: Vec<(Type, ParamMode)>,
    ret: Type,
}

/// The uniforms MilkDrop sets, as named in its `include.fx`, and where they are in WGSL.
fn uniform(name: &str) -> Option<(String, Type)> {
    let v4 = Type::Vector(Scalar::Float, 4);
    if let Some(i) = name.strip_prefix("_c").and_then(|n| n.parse::<u8>().ok())
        && i < 14
    {
        return Some((format!("inp.c[{i}]"), v4));
    }
    if let Some(i) = name.strip_prefix("_d").and_then(|n| n.parse::<u8>().ok())
        && i < 3
    {
        return Some((format!("inp.d[{i}]"), v4));
    }
    if let Some(c) = name.strip_prefix("_q")
        && c.len() == 1
        && let Some(i) = "abcdefgh".find(c)
    {
        return Some((format!("inp.q[{i}]"), v4));
    }
    match name {
        "rand_frame" => return Some(("inp.rand_frame".into(), v4)),
        "rand_preset" => return Some(("inp.rand_preset".into(), v4)),
        _ => {}
    }
    let rot = name.strip_prefix("rot_")?;
    let groups = ["s", "d", "f", "vf", "uf", "rand"];
    let split = rot.find(|c: char| c.is_ascii_digit())?;
    let (group, n) = rot.split_at(split);
    let g = groups.iter().position(|g| *g == group)?;
    let n: usize = n.parse().ok()?;
    (1..=4)
        .contains(&n)
        .then(|| (format!("inp.rot[{}]", g * 4 + n - 1), Type::Matrix(4, 3)))
}

/// What `sampler_<name>` refers to.
fn sampler_named(name: &str, used: &mut Used) -> SamplerRef {
    let lower = name.to_ascii_lowercase();
    let (filter, rest) = match lower.split_at(lower.len().min(3)) {
        ("fw_", r) => (Some(Filter::LinearWrap), r),
        ("fc_", r) => (Some(Filter::LinearClamp), r),
        ("pw_", r) => (Some(Filter::PointWrap), r),
        ("pc_", r) => (Some(Filter::PointClamp), r),
        _ => (None, lower.as_str()),
    };
    let texture = match rest {
        "main" => Texture::Main,
        "blur1" => Texture::Blur(1),
        "blur2" => Texture::Blur(2),
        "blur3" => Texture::Blur(3),
        "noise_lq" => Texture::NoiseLq,
        "noise_lq_lite" => Texture::NoiseLqLite,
        "noise_mq" => Texture::NoiseMq,
        "noise_hq" => Texture::NoiseHq,
        "noisevol_lq" => Texture::NoiseVolLq,
        "noisevol_hq" => Texture::NoiseVolHq,
        user => {
            let i = match used.user.iter().position(|u| u == user) {
                Some(i) => i,
                None => {
                    used.user.push(user.to_owned());
                    used.user.len() - 1
                }
            };
            Texture::User(i)
        }
    };
    // Unprefixed: filtered, and wrapping except for the blur textures.
    let filter = filter.unwrap_or(if matches!(texture, Texture::Blur(_)) {
        Filter::LinearClamp
    } else {
        Filter::LinearWrap
    });
    SamplerRef { texture, filter }
}

/// The type both of two numbers' types convert to for arithmetic, as HLSL picks it.
fn arith_type(a: &Type, b: &Type) -> Option<Type> {
    let (sa, sb) = (a.scalar()?, b.scalar()?);
    let s = if sa == Scalar::Float || sb == Scalar::Float {
        Scalar::Float
    } else if sa == Scalar::Uint || sb == Scalar::Uint {
        Scalar::Uint
    } else {
        Scalar::Int
    };
    Some(match (a, b) {
        (Type::Scalar(_), Type::Scalar(_)) => Type::Scalar(s),
        (Type::Scalar(_), Type::Vector(_, n)) | (Type::Vector(_, n), Type::Scalar(_)) => {
            Type::Vector(s, *n)
        }
        (Type::Vector(_, n), Type::Vector(_, m)) => Type::Vector(s, *n.min(m)),
        (Type::Matrix(r, c), Type::Scalar(_)) | (Type::Scalar(_), Type::Matrix(r, c)) => {
            Type::Matrix(*r, *c)
        }
        (Type::Matrix(r, c), Type::Matrix(r2, c2)) if r == r2 && c == c2 => Type::Matrix(*r, *c),
        _ => return None,
    })
}

/// The type all of `types` convert to for a component-wise function.
fn common_type<'a>(
    types: impl IntoIterator<Item = &'a Type>,
    float_only: bool,
) -> Result<Type, String> {
    let mut scalar: Option<Scalar> = None;
    let mut size: Option<u8> = None;
    for t in types {
        let (s, n) = match t {
            Type::Scalar(s) => (*s, None),
            Type::Vector(s, n) => (*s, Some(*n)),
            other => return Err(format!("{other} cannot be used here")),
        };
        scalar = Some(match (scalar, s) {
            (Some(Scalar::Float), _) | (_, Scalar::Float) => Scalar::Float,
            (Some(Scalar::Uint), _) | (_, Scalar::Uint) => Scalar::Uint,
            _ => Scalar::Int,
        });
        if let Some(n) = n {
            size = Some(size.map_or(n, |m| m.min(n)));
        }
    }
    let s = if float_only {
        Scalar::Float
    } else {
        scalar.unwrap_or(Scalar::Float)
    };
    Ok(size.map_or(Type::Scalar(s), |n| Type::Vector(s, n)))
}

/// Swizzle letters as component indices.
fn swizzle(name: &str) -> Option<Vec<u8>> {
    let sets = ["xyzw", "rgba"];
    let set = sets.iter().find(|s| name.chars().all(|c| s.contains(c)))?;
    if name.is_empty() || name.len() > 4 {
        return None;
    }
    name.chars()
        .map(|c| set.find(c).and_then(|i| u8::try_from(i).ok()))
        .collect()
}

const XYZW: [char; 4] = ['x', 'y', 'z', 'w'];

/// Writes WGSL for a parsed shader.
pub(super) struct Emitter {
    pub used: Used,
    globals: HashMap<String, Var>,
    /// Private globals' declarations, and the statements giving them their starting values.
    pub private_decls: String,
    pub init: String,
    functions: HashMap<String, Signature>,
    pub function_code: String,
    scopes: Vec<HashMap<String, Var>>,
    names: HashSet<String>,
    /// Statements an expression needs to run first (assignments inside expressions).
    pending: Vec<String>,
    temp: u32,
    /// The type the function being written returns (`None` in the shader body).
    returns: Option<Type>,
    indent: usize,
    /// Globals the shader assigns to somewhere: uniforms among them get private copies.
    assigned: HashSet<String>,
}

type Result<T, E = String> = std::result::Result<T, E>;

impl Emitter {
    pub(super) fn new() -> Emitter {
        Emitter {
            used: Used::default(),
            globals: HashMap::new(),
            private_decls: String::new(),
            init: String::new(),
            functions: HashMap::new(),
            function_code: String::new(),
            scopes: Vec::new(),
            names: HashSet::new(),
            pending: Vec::new(),
            temp: 0,
            returns: None,
            indent: 1,
            assigned: HashSet::new(),
        }
    }

    /// Note every name the shader assigns to (or passes to an `out` parameter), before
    /// anything is written.
    pub(super) fn find_assigned(&mut self, items: &[Item], body: &[Stmt]) {
        fn root(e: &Expr) -> Option<&str> {
            match &e.kind {
                ExprKind::Name(n) => Some(n),
                ExprKind::Member(b, _) | ExprKind::Index(b, _) => root(b),
                _ => None,
            }
        }
        fn expr(e: &Expr, out: &mut HashSet<String>) {
            match &e.kind {
                ExprKind::Assign(_, t, v) => {
                    if let Some(n) = root(t) {
                        out.insert(n.to_owned());
                    }
                    expr(t, out);
                    expr(v, out);
                }
                ExprKind::Unary(op, x) => {
                    if matches!(
                        op,
                        UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec
                    ) && let Some(n) = root(x)
                    {
                        out.insert(n.to_owned());
                    }
                    expr(x, out);
                }
                ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
                    expr(a, out);
                    expr(b, out);
                }
                ExprKind::Ternary(a, b, c) => {
                    expr(a, out);
                    expr(b, out);
                    expr(c, out);
                }
                ExprKind::Call(_, args) => {
                    // Any argument might be an out parameter.
                    for a in args {
                        if let Some(n) = root(a) {
                            out.insert(n.to_owned());
                        }
                        expr(a, out);
                    }
                }
                ExprKind::Construct(_, args) | ExprKind::List(args) | ExprKind::Comma(args) => {
                    for a in args {
                        expr(a, out);
                    }
                }
                ExprKind::Cast(_, x) | ExprKind::Member(x, _) => expr(x, out),
                ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Name(_) => {}
            }
        }
        fn stmt(s: &Stmt, out: &mut HashSet<String>) {
            match s {
                Stmt::Block(b) => b.iter().for_each(|s| stmt(s, out)),
                Stmt::Decl(ds) => ds
                    .iter()
                    .filter_map(|d| d.init.as_ref())
                    .for_each(|e| expr(e, out)),
                Stmt::Expr(e) | Stmt::Return(Some(e), _) => expr(e, out),
                Stmt::If(c, a, b) => {
                    expr(c, out);
                    stmt(a, out);
                    if let Some(b) = b {
                        stmt(b, out);
                    }
                }
                Stmt::For(i, c, st, b) => {
                    if let Some(i) = i {
                        stmt(i, out);
                    }
                    c.iter().chain(st.iter()).for_each(|e| expr(e, out));
                    stmt(b, out);
                }
                Stmt::While(c, b) | Stmt::DoWhile(b, c) => {
                    expr(c, out);
                    stmt(b, out);
                }
                _ => {}
            }
        }
        for item in items {
            match item {
                Item::Function(f) => f.body.iter().for_each(|s| stmt(s, &mut self.assigned)),
                Item::Var { decl, .. } => {
                    if let Some(e) = &decl.init {
                        expr(e, &mut self.assigned);
                    }
                }
            }
        }
        body.iter().for_each(|s| stmt(s, &mut self.assigned));
    }

    fn err<T>(line: u32, message: impl Into<String>) -> Result<T, Error> {
        Err(Error {
            line,
            message: message.into(),
        })
    }

    /// Declare the header's globals and functions.
    pub(super) fn items(&mut self, items: &[Item]) -> Result<(), Error> {
        // Signatures first, so functions can call ones declared later.
        for item in items {
            if let Item::Function(f) = item {
                if self.functions.contains_key(&f.name) {
                    return Self::err(f.line, format!("'{}' is declared twice", f.name));
                }
                self.functions.insert(
                    f.name.clone(),
                    Signature {
                        wgsl: format!("f_{}", f.name),
                        params: f.params.iter().map(|p| (p.ty.clone(), p.mode)).collect(),
                        ret: f.ret.clone(),
                    },
                );
            }
        }
        for item in items {
            match item {
                Item::Var { decl, uniform } => self.global(decl, *uniform)?,
                Item::Function(f) => self.function(f)?,
            }
        }
        Ok(())
    }

    fn global(&mut self, d: &VarDecl, is_uniform: bool) -> Result<(), Error> {
        let name = &d.name;
        if matches!(d.ty, Type::Sampler { .. }) || name.starts_with("texsize_") {
            // Samplers and texture sizes are found by name wherever they are used.
            return Ok(());
        }
        if is_uniform && let Some((code, ty)) = uniform(name) {
            let var = if self.assigned.contains(name) {
                // HLSL lets a shader change a uniform for itself; WGSL's are read-only.
                let wgsl = format!("g_{name}");
                let _ = writeln!(self.private_decls, "var<private> {wgsl}: {};", ty.wgsl());
                let _ = writeln!(self.init, "    {wgsl} = {code};");
                self.used.inputs = true;
                Var {
                    code: wgsl,
                    ty,
                    constant: false,
                    pointer: None,
                    local: false,
                }
            } else {
                Var {
                    code,
                    ty,
                    constant: true,
                    pointer: None,
                    local: false,
                }
            };
            self.globals.insert(name.clone(), var);
            return Ok(());
        }
        if self.globals.contains_key(name) {
            // Declared again (MilkDrop's own and the preset's): the first one stands.
            return Ok(());
        }
        let wgsl = format!("g_{name}");
        let _ = writeln!(self.private_decls, "var<private> {wgsl}: {};", d.ty.wgsl());
        if let Some(init) = &d.init {
            self.scopes.push(HashMap::new());
            let saved = std::mem::replace(&mut self.indent, 1);
            let value = self.initial_value(init, &d.ty, d.line);
            self.indent = saved;
            self.scopes.pop();
            let value = value?;
            let pending: String = self
                .pending
                .drain(..)
                .map(|p| format!("    {p}\n"))
                .collect();
            self.init.push_str(&pending);
            let _ = writeln!(self.init, "    {wgsl} = {value};");
        }
        self.globals.insert(
            name.clone(),
            Var {
                code: wgsl,
                ty: d.ty.clone(),
                constant: false,
                pointer: None,
                local: false,
            },
        );
        Ok(())
    }

    fn function(&mut self, f: &Function) -> Result<(), Error> {
        let Some(sig) = self.functions.get(&f.name).cloned() else {
            return Self::err(f.line, "function not declared");
        };
        self.names.clear();
        self.scopes.push(HashMap::new());
        self.returns = Some(f.ret.clone());
        let mut params = Vec::new();
        let mut prologue = String::new();
        for p in &f.params {
            if matches!(p.ty, Type::Sampler { .. } | Type::Void) {
                return Self::err(f.line, "sampler parameters are not supported");
            }
            let pname = format!("p_{}", p.name);
            match p.mode {
                ParamMode::In => {
                    params.push(format!("{pname}: {}", p.ty.wgsl()));
                    // HLSL parameters can be assigned to; WGSL's cannot.
                    let local = self.fresh(&p.name);
                    let _ = writeln!(prologue, "    var {local} = {pname};");
                    self.declare(&p.name, local, p.ty.clone(), false);
                }
                ParamMode::Out | ParamMode::InOut => {
                    params.push(format!("{pname}: ptr<function, {}>", p.ty.wgsl()));
                    if let Some(scope) = self.scopes.last_mut() {
                        scope.insert(
                            p.name.clone(),
                            Var {
                                code: format!("(*{pname})"),
                                ty: p.ty.clone(),
                                constant: false,
                                pointer: Some(pname.clone()),
                                local: true,
                            },
                        );
                    }
                }
            }
        }
        let ret = if f.ret == Type::Void {
            String::new()
        } else {
            format!(" -> {}", f.ret.wgsl())
        };
        let mut body = String::new();
        self.indent = 1;
        let result = self.block_stmts(&f.body, &mut body);
        self.scopes.clear();
        self.returns = None;
        result?;
        let _ = write!(
            self.function_code,
            "fn {}({}){ret} {{\n{prologue}{body}",
            sig.wgsl,
            params.join(", ")
        );
        if f.ret != Type::Void {
            // WGSL wants a return at the end even when HLSL's paths all return earlier.
            let _ = writeln!(self.function_code, "    return {}();", f.ret.wgsl());
        }
        self.function_code.push_str("}\n\n");
        Ok(())
    }

    /// Write the shader body as `fn shader_body`, which returns `ret`.
    pub(super) fn body(&mut self, stmts: &[Stmt], comp: bool) -> Result<String, Error> {
        self.names.clear();
        self.scopes.push(HashMap::new());
        self.returns = None;
        let mut out = String::new();
        let v2 = Type::Vector(Scalar::Float, 2);
        let v3 = Type::Vector(Scalar::Float, 3);
        let mut inputs = vec![
            ("uv", "uv_in", v2.clone()),
            ("uv_orig", "uv_orig_in", v2),
            ("rad", "rad_in", Type::FLOAT),
            ("ang", "ang_in", Type::FLOAT),
        ];
        if comp {
            inputs.push(("hue_shader", "hue_in", v3.clone()));
        }
        for (name, input, ty) in inputs {
            let local = self.fresh(name);
            let _ = writeln!(out, "    var {local} = {input};");
            self.declare(name, local, ty, false);
        }
        let ret = self.fresh("ret");
        let _ = writeln!(out, "    var {ret} = vec3<f32>(0.0);");
        self.declare("ret", ret.clone(), v3, false);
        self.indent = 1;
        let r = self.block_stmts(stmts, &mut out);
        self.scopes.clear();
        r?;
        let _ = writeln!(out, "    return {ret};");
        Ok(format!(
            "fn shader_body(uv_in: vec2<f32>, uv_orig_in: vec2<f32>, rad_in: f32, ang_in: f32, \
             hue_in: vec3<f32>) -> vec3<f32> {{\n{out}}}\n"
        ))
    }

    /// A new WGSL name for a local called `name`.
    fn fresh(&mut self, name: &str) -> String {
        let base = format!("u_{name}");
        let mut candidate = base.clone();
        let mut n = 1;
        while self.names.contains(&candidate) {
            candidate = format!("{base}_{n}");
            n += 1;
        }
        self.names.insert(candidate.clone());
        candidate
    }

    fn declare(&mut self, name: &str, code: String, ty: Type, constant: bool) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(
                name.to_owned(),
                Var {
                    code,
                    ty,
                    constant,
                    pointer: None,
                    local: true,
                },
            );
        }
    }

    fn temp(&mut self) -> String {
        self.temp += 1;
        format!("tmp{}", self.temp)
    }

    fn pad(&self) -> String {
        "    ".repeat(self.indent)
    }

    fn block_stmts(&mut self, stmts: &[Stmt], out: &mut String) -> Result<(), Error> {
        for s in stmts {
            self.stmt(s, out)?;
        }
        Ok(())
    }

    /// Write `s` as a `{ }` block on its own, in a new scope.
    fn nested(&mut self, s: &Stmt, out: &mut String) -> Result<(), Error> {
        self.scopes.push(HashMap::new());
        self.indent += 1;
        let r = match s {
            Stmt::Block(b) => self.block_stmts(b, out),
            other => self.stmt(other, out),
        };
        self.indent -= 1;
        self.scopes.pop();
        r
    }

    fn flush_pending(&mut self, out: &mut String) {
        let pad = self.pad();
        for p in self.pending.drain(..) {
            let _ = writeln!(out, "{pad}{p}");
        }
    }

    /// A condition for `if` or a loop: a scalar bool, with nothing to run first.
    fn condition(&mut self, e: &Expr) -> Result<String, Error> {
        let v = self.expr(e)?;
        if !self.pending.is_empty() {
            return Self::err(e.line, "assignments inside conditions are not supported");
        }
        let v = if let Type::Vector(..) = v.ty {
            // HLSL takes the first component of a vector condition.
            Val::new(
                format!("({}).x", v.code),
                v.ty.scalar().map_or(Type::FLOAT, Type::Scalar),
            )
        } else {
            v
        };
        self.conv(&v, &Type::BOOL).map_err(|m| Error {
            line: e.line,
            message: m,
        })
    }

    fn stmt(&mut self, s: &Stmt, out: &mut String) -> Result<(), Error> {
        let pad = self.pad();
        match s {
            Stmt::Empty => {}
            Stmt::Block(b) => {
                let _ = writeln!(out, "{pad}{{");
                self.scopes.push(HashMap::new());
                self.indent += 1;
                self.block_stmts(b, out)?;
                self.indent -= 1;
                self.scopes.pop();
                let _ = writeln!(out, "{pad}}}");
            }
            Stmt::Decl(decls) => {
                for d in decls {
                    self.local_decl(d, out)?;
                }
            }
            Stmt::Expr(e) => self.expr_stmt(e, out)?,
            Stmt::If(c, then, otherwise) => {
                let c = self.condition(c)?;
                let _ = writeln!(out, "{pad}if ({c}) {{");
                self.nested(then, out)?;
                match otherwise {
                    Some(o) => {
                        let _ = writeln!(out, "{pad}}} else {{");
                        self.nested(o, out)?;
                        let _ = writeln!(out, "{pad}}}");
                    }
                    None => {
                        let _ = writeln!(out, "{pad}}}");
                    }
                }
            }
            Stmt::For(init, cond, step, body) => {
                // HLSL's loop variable belongs to the enclosing scope (fxc's rule), so it is
                // declared there; the loop is a `loop` with the step in `continuing`.
                if let Some(init) = init {
                    self.stmt(init, out)?;
                }
                let _ = writeln!(out, "{pad}loop {{");
                let inner = format!("{pad}    ");
                if let Some(c) = cond {
                    let c = self.condition(c)?;
                    let _ = writeln!(out, "{inner}if !({c}) {{ break; }}");
                }
                self.nested(body, out)?;
                if let Some(step) = step {
                    self.indent += 2;
                    let mut cont = String::new();
                    let r = self.expr_stmt(step, &mut cont);
                    self.indent -= 2;
                    r?;
                    let _ = writeln!(out, "{inner}continuing {{");
                    out.push_str(&cont);
                    let _ = writeln!(out, "{inner}}}");
                }
                let _ = writeln!(out, "{pad}}}");
            }
            Stmt::While(c, body) => {
                let c = self.condition(c)?;
                let _ = writeln!(out, "{pad}while ({c}) {{");
                self.nested(body, out)?;
                let _ = writeln!(out, "{pad}}}");
            }
            Stmt::DoWhile(body, c) => {
                let _ = writeln!(out, "{pad}loop {{");
                self.nested(body, out)?;
                let c = self.condition(c)?;
                let _ = writeln!(out, "{pad}    continuing {{ break if !({c}); }}");
                let _ = writeln!(out, "{pad}}}");
            }
            Stmt::Return(e, line) => match (&self.returns, e) {
                (None, _) => {
                    // In the shader body: `ret` is the result.
                    let ret = self.lookup("ret").map(|v| v.code).unwrap_or_default();
                    let _ = writeln!(out, "{pad}return {ret};");
                }
                (Some(Type::Void), _) => {
                    let _ = writeln!(out, "{pad}return;");
                }
                (Some(ty), Some(e)) => {
                    let ty = ty.clone();
                    let v = self.expr(e)?;
                    let code = self.conv(&v, &ty).map_err(|m| Error {
                        line: *line,
                        message: m,
                    })?;
                    self.flush_pending(out);
                    let _ = writeln!(out, "{pad}return {code};");
                }
                (Some(_), None) => return Self::err(*line, "return without a value"),
            },
            Stmt::Break => {
                let _ = writeln!(out, "{pad}break;");
            }
            Stmt::Continue => {
                let _ = writeln!(out, "{pad}continue;");
            }
            Stmt::Discard => {
                let _ = writeln!(out, "{pad}discard;");
            }
        }
        Ok(())
    }

    fn local_decl(&mut self, d: &VarDecl, out: &mut String) -> Result<(), Error> {
        if matches!(d.ty, Type::Sampler { .. } | Type::Void) {
            return Self::err(d.line, "local samplers are not supported");
        }
        let value = match &d.init {
            Some(init) => Some(self.initial_value(init, &d.ty, d.line)?),
            None => None,
        };
        self.flush_pending(out);
        let name = self.fresh(&d.name);
        let pad = self.pad();
        let ty = d.ty.wgsl();
        let constant = d.constant && value.is_some() && !matches!(d.ty, Type::Array(..));
        match (value, constant) {
            (Some(v), true) => {
                let _ = writeln!(out, "{pad}let {name}: {ty} = {v};");
            }
            (Some(v), false) => {
                let _ = writeln!(out, "{pad}var {name}: {ty} = {v};");
            }
            (None, _) => {
                let _ = writeln!(out, "{pad}var {name}: {ty};");
            }
        }
        self.declare(&d.name, name, d.ty.clone(), constant);
        Ok(())
    }

    /// An initialiser converted to `ty`: an expression, or a `{ }` list.
    fn initial_value(&mut self, init: &Expr, ty: &Type, line: u32) -> Result<String, Error> {
        let wrap = |m: String| Error { line, message: m };
        if let ExprKind::List(items) = &init.kind {
            return match ty {
                Type::Array(elem, n) => {
                    let mut parts = Vec::new();
                    if items.len() == *n as usize {
                        for item in items {
                            parts.push(self.initial_value(item, elem, line)?);
                        }
                    } else {
                        // A flat list of numbers, filling the elements in turn.
                        let flat = self.flatten(items)?;
                        let per = elem.components() as usize;
                        if per == 0 || flat.len() != per * *n as usize {
                            return Err(wrap("wrong number of values for the array".into()));
                        }
                        for chunk in flat.chunks(per) {
                            parts.push(self.construct(elem, chunk.to_vec()).map_err(wrap)?.code);
                        }
                    }
                    Ok(format!("{}({})", ty.wgsl(), parts.join(", ")))
                }
                _ => {
                    let flat = self.flatten(items)?;
                    Ok(self.construct(ty, flat).map_err(wrap)?.code)
                }
            };
        }
        let v = self.expr(init)?;
        self.conv(&v, ty).map_err(wrap)
    }

    /// The values of a `{ }` list, nested lists flattened.
    fn flatten(&mut self, items: &[Expr]) -> Result<Vec<Val>, Error> {
        let mut out = Vec::new();
        for item in items {
            if let ExprKind::List(inner) = &item.kind {
                out.extend(self.flatten(inner)?);
            } else {
                out.push(self.expr(item)?);
            }
        }
        Ok(out)
    }

    /// An expression used as a statement.
    fn expr_stmt(&mut self, e: &Expr, out: &mut String) -> Result<(), Error> {
        let wrap = |m: String| Error {
            line: e.line,
            message: m,
        };
        match &e.kind {
            ExprKind::Comma(items) => {
                for i in items {
                    self.expr_stmt(i, out)?;
                }
                return Ok(());
            }
            ExprKind::Assign(op, target, value) => {
                let code = self.assign(*op, target, value)?;
                self.flush_pending(out);
                for line in code {
                    let _ = writeln!(out, "{}{line}", self.pad());
                }
                return Ok(());
            }
            ExprKind::Unary(
                op @ (UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec),
                target,
            ) => {
                let bin = if matches!(op, UnOp::PreInc | UnOp::PostInc) {
                    BinOp::Add
                } else {
                    BinOp::Sub
                };
                let one = Expr {
                    kind: ExprKind::Int(1),
                    line: e.line,
                };
                let code = self.assign(Some(bin), target, &one)?;
                self.flush_pending(out);
                for line in code {
                    let _ = writeln!(out, "{}{line}", self.pad());
                }
                return Ok(());
            }
            ExprKind::Call(name, args) if name == "sincos" && args.len() == 3 => {
                let x = self.expr(&args[0])?;
                let ty = common_type([&x.ty], true).map_err(wrap)?;
                let x = self.conv(&x, &ty).map_err(wrap)?;
                let t = self.temp();
                self.flush_pending(out);
                let _ = writeln!(out, "{}let {t} = {x};", self.pad());
                for (arg, f) in [(&args[1], "sin"), (&args[2], "cos")] {
                    let target = self.expr(arg)?;
                    let code =
                        self.store(&target, &Val::new(format!("{f}({t})"), ty.clone()), e.line)?;
                    for line in code {
                        let _ = writeln!(out, "{}{line}", self.pad());
                    }
                }
                return Ok(());
            }
            ExprKind::Call(name, args) if name == "clip" && args.len() == 1 => {
                let v = self.expr(&args[0])?;
                let ty = common_type([&v.ty], true).map_err(wrap)?;
                let code = self.conv(&v, &ty).map_err(wrap)?;
                let zero = Val::lit(0.0, Type::FLOAT);
                let zero = self.conv(&zero, &ty).map_err(wrap)?;
                self.flush_pending(out);
                let test = if matches!(ty, Type::Vector(..)) {
                    format!("any({code} < {zero})")
                } else {
                    format!("{code} < {zero}")
                };
                let _ = writeln!(out, "{}if ({test}) {{ discard; }}", self.pad());
                return Ok(());
            }
            _ => {}
        }
        let v = self.expr(e)?;
        self.flush_pending(out);
        let is_user_call =
            matches!(&e.kind, ExprKind::Call(n, _) if self.functions.contains_key(n));
        if is_user_call {
            let _ = writeln!(out, "{}{};", self.pad(), v.code);
        } else if v.ty != Type::Void && v.lit.is_none() {
            let _ = writeln!(out, "{}_ = {};", self.pad(), v.code);
        }
        Ok(())
    }

    /// Statements that assign `value` to `target`, with an operator for `+=` and the like.
    fn assign(
        &mut self,
        op: Option<BinOp>,
        target: &Expr,
        value: &Expr,
    ) -> Result<Vec<String>, Error> {
        let t = self.expr(target)?;
        let v = self.expr(value)?;
        let v = match op {
            None => v,
            Some(op) => self.binary(op, t.clone(), v, target.line)?,
        };
        self.store(&t, &v, target.line)
    }

    fn store(&mut self, t: &Val, v: &Val, line: u32) -> Result<Vec<String>, Error> {
        let wrap = |m: String| Error { line, message: m };
        let code = self.conv(v, &t.ty).map_err(wrap)?;
        match &t.place {
            Some(Place::Whole) => Ok(vec![format!("{} = {code};", t.code)]),
            Some(Place::Swizzle { base, comps }) => {
                let tmp = self.temp();
                let mut lines = vec![format!("let {tmp} = {code};")];
                for (k, c) in comps.iter().enumerate() {
                    let field = if comps.len() == 1 {
                        tmp.clone()
                    } else {
                        format!("{tmp}.{}", XYZW[k])
                    };
                    lines.push(format!("{base}.{} = {field};", XYZW[usize::from(*c)]));
                }
                Ok(lines)
            }
            None => Self::err(line, "this cannot be assigned to"),
        }
    }

    fn lookup(&self, name: &str) -> Option<Var> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v.clone());
            }
        }
        self.globals.get(name).cloned()
    }

    fn expr(&mut self, e: &Expr) -> Result<Val, Error> {
        let wrap = |m: String| Error {
            line: e.line,
            message: m,
        };
        match &e.kind {
            ExprKind::Int(v) => Ok(Val::lit(*v as f64, Type::INT)),
            ExprKind::Float(v) => Ok(Val::lit(*v, Type::FLOAT)),
            ExprKind::Bool(b) => Ok(Val::lit(f64::from(u8::from(*b)), Type::BOOL)),
            ExprKind::Name(name) => self.name(name, e.line),
            ExprKind::Unary(op, x) => {
                if matches!(
                    op,
                    UnOp::PreInc | UnOp::PreDec | UnOp::PostInc | UnOp::PostDec
                ) {
                    // Inside an expression: done first, then its value used.
                    let mut stmts = String::new();
                    let target = self.expr(x)?;
                    let before = self.temp();
                    if matches!(op, UnOp::PostInc | UnOp::PostDec) {
                        self.pending
                            .push(format!("let {before} = {};", target.code));
                    }
                    self.expr_stmt(
                        &Expr {
                            kind: ExprKind::Unary(*op, x.clone()),
                            line: e.line,
                        },
                        &mut stmts,
                    )?;
                    self.pending
                        .extend(stmts.lines().map(|l| l.trim().to_owned()));
                    let code = if matches!(op, UnOp::PostInc | UnOp::PostDec) {
                        before
                    } else {
                        target.code.clone()
                    };
                    return Ok(Val::new(code, target.ty));
                }
                let v = self.expr(x)?;
                self.unary(*op, v).map_err(wrap)
            }
            ExprKind::Binary(op, a, b) => {
                let a = self.expr(a)?;
                let b = self.expr(b)?;
                self.binary(*op, a, b, e.line)
            }
            ExprKind::Assign(op, target, value) => {
                // Inside an expression: done first, then the target's value used.
                let lines = self.assign(*op, target, value)?;
                self.pending.extend(lines);
                let t = self.expr(target)?;
                Ok(Val::new(t.code, t.ty))
            }
            ExprKind::Ternary(c, a, b) => {
                let c = self.expr(c)?;
                let a = self.expr(a)?;
                let b = self.expr(b)?;
                let ty = arith_type(&a.ty, &b.ty)
                    .or_else(|| (a.ty == b.ty).then(|| a.ty.clone()))
                    .ok_or_else(|| wrap(format!("{} and {} have no common type", a.ty, b.ty)))?;
                let ty =
                    if a.ty.scalar() == Some(Scalar::Bool) && b.ty.scalar() == Some(Scalar::Bool) {
                        ty.with_scalar(Scalar::Bool)
                    } else {
                        ty
                    };
                let cond_ty = match (&c.ty, &ty) {
                    (Type::Vector(_, n), Type::Vector(_, m)) if n == m => {
                        Type::Vector(Scalar::Bool, *n)
                    }
                    _ => Type::BOOL,
                };
                let c = if matches!(c.ty, Type::Vector(..)) && cond_ty == Type::BOOL {
                    Val::new(
                        format!("({}).x", c.code),
                        c.ty.scalar().map_or(Type::FLOAT, Type::Scalar),
                    )
                } else {
                    c
                };
                let cc = self.conv(&c, &cond_ty).map_err(wrap)?;
                let ac = self.conv(&a, &ty).map_err(wrap)?;
                let bc = self.conv(&b, &ty).map_err(wrap)?;
                Ok(Val::new(format!("select({bc}, {ac}, {cc})"), ty))
            }
            ExprKind::Call(name, args) => self.call(name, args, e.line),
            ExprKind::Construct(ty, args) => {
                let mut vals = Vec::new();
                for a in args {
                    vals.push(self.expr(a)?);
                }
                self.construct(ty, vals).map_err(wrap)
            }
            ExprKind::Cast(ty, x) => {
                let v = self.expr(x)?;
                let code = self.conv(&v, ty).map_err(wrap)?;
                Ok(Val::new(code, ty.clone()))
            }
            ExprKind::Member(base, name) => {
                let b = self.expr(base)?;
                self.member(b, name).map_err(wrap)
            }
            ExprKind::Index(base, index) => {
                let b = self.expr(base)?;
                let i = self.expr(index)?;
                let elem = match &b.ty {
                    Type::Vector(s, _) => Type::Scalar(*s),
                    Type::Matrix(_, c) => Type::vec(Scalar::Float, *c),
                    Type::Array(t, _) => (**t).clone(),
                    other => return Err(wrap(format!("{other} cannot be indexed"))),
                };
                let ic = match i.lit {
                    Some(v) => format!("{}", v as i64),
                    None => self.conv(&i, &Type::INT).map_err(wrap)?,
                };
                let code = format!("{}[{ic}]", b.code);
                let place = b.place.as_ref().map(|_| Place::Whole);
                Ok(Val {
                    place: if matches!(b.place, Some(Place::Whole)) {
                        place
                    } else {
                        None
                    },
                    ..Val::new(code, elem)
                })
            }
            ExprKind::List(_) => Self::err(e.line, "a { } list is only allowed in a declaration"),
            ExprKind::Comma(items) => {
                // Everything but the last runs first.
                let (last, rest) = items
                    .split_last()
                    .ok_or_else(|| wrap("empty expression".into()))?;
                for item in rest {
                    let mut stmts = String::new();
                    self.expr_stmt(item, &mut stmts)?;
                    self.pending
                        .extend(stmts.lines().map(|l| l.trim().to_owned()));
                }
                self.expr(last)
            }
        }
    }

    fn name(&mut self, name: &str, line: u32) -> Result<Val, Error> {
        if let Some(v) = self.lookup(name) {
            if v.code.starts_with("inp.") {
                self.used.inputs = true;
            }
            let assignable = !v.constant;
            let address = if v.local {
                Some(v.pointer.clone().unwrap_or_else(|| format!("&{}", v.code)))
            } else {
                None
            };
            return Ok(Val {
                place: assignable.then_some(Place::Whole),
                address,
                ..Val::new(v.code, v.ty)
            });
        }
        if let Some(rest) = name.strip_prefix("sampler_") {
            let s = sampler_named(rest, &mut self.used);
            let volume = s.texture.is_volume();
            return Ok(Val {
                sampler: Some(s),
                ..Val::new(String::new(), Type::Sampler { volume })
            });
        }
        if let Some(rest) = name.strip_prefix("texsize_") {
            let s = sampler_named(rest, &mut self.used);
            self.used.textures.insert(s.texture.clone());
            if let Texture::Blur(n) = s.texture {
                self.used.blur = self.used.blur.max(n);
            }
            let t = s.texture.binding_name();
            let dims = if s.texture.is_volume() {
                format!("vec2<f32>(textureDimensions({t}).xy)")
            } else {
                format!("vec2<f32>(textureDimensions({t}))")
            };
            return Ok(Val::new(
                format!("vec4<f32>({dims}, 1.0 / {dims})"),
                Type::Vector(Scalar::Float, 4),
            ));
        }
        Self::err(line, format!("unknown name '{name}'"))
    }

    /// `v` written as type `to`, converting as HLSL would.
    fn conv(&self, v: &Val, to: &Type) -> Result<String> {
        if &v.ty == to {
            return Ok(v.code.clone());
        }
        // Plain numbers are written straight in the type wanted.
        if let Some(x) = v.lit
            && let Some(s) = to.scalar()
            && matches!(to, Type::Scalar(_) | Type::Vector(..))
        {
            let scalar = Val::lit(
                match s {
                    Scalar::Int | Scalar::Uint => x.trunc(),
                    _ => x,
                },
                Type::Scalar(s),
            );
            let scalar = match s {
                Scalar::Uint => format!("{}u", (x.trunc().max(0.0)) as u64),
                _ => scalar.code,
            };
            return Ok(match to {
                Type::Scalar(_) => scalar,
                _ => format!("{}({scalar})", to.wgsl()),
            });
        }
        let from = &v.ty;
        let code = &v.code;
        let elem = |from: Scalar, to_s: Scalar, code: &str, n: Option<u8>| -> String {
            if from == to_s {
                return code.to_owned();
            }
            let ty = match n {
                Some(n) => Type::Vector(to_s, n),
                None => Type::Scalar(to_s),
            };
            if to_s == Scalar::Bool {
                let zero = match (from, n) {
                    (Scalar::Float, None) => "0.0".to_owned(),
                    (Scalar::Float, Some(n)) => format!("vec{n}<f32>(0.0)"),
                    (Scalar::Uint, None) => "0u".to_owned(),
                    (s, Some(n)) => format!("vec{n}<{}>(0)", s.wgsl()),
                    (_, None) => "0".to_owned(),
                };
                format!("({code} != {zero})")
            } else {
                format!("{}({code})", ty.wgsl())
            }
        };
        match (from, to) {
            (Type::Scalar(a), Type::Scalar(b)) => Ok(elem(*a, *b, code, None)),
            (Type::Scalar(a), Type::Vector(b, n)) => {
                let s = elem(*a, *b, code, None);
                Ok(format!("vec{n}<{}>({s})", b.wgsl()))
            }
            (Type::Scalar(a), Type::Matrix(r, c)) => {
                let s = elem(*a, Scalar::Float, code, None);
                let all = vec![s; usize::from(*r) * usize::from(*c)];
                Ok(format!("{}({})", to.wgsl(), all.join(", ")))
            }
            (Type::Vector(a, n), Type::Vector(b, m)) if m <= n => {
                let cut = if m < n {
                    format!(
                        "({code}).{}",
                        XYZW[..usize::from(*m)].iter().collect::<String>()
                    )
                } else {
                    code.clone()
                };
                Ok(elem(*a, *b, &cut, Some(*m)))
            }
            (Type::Vector(a, _), Type::Scalar(b)) => Ok(elem(*a, *b, &format!("({code}).x"), None)),
            (Type::Vector(a, n), Type::Matrix(r, c)) if u32::from(*n) == to.components() => {
                // Filled row by row, which is WGSL's column by column here.
                let _ = (r, c);
                let parts: Vec<String> = (0..*n)
                    .map(|k| elem(*a, Scalar::Float, &format!("({code})[{k}]"), None))
                    .collect();
                Ok(format!("{}({})", to.wgsl(), parts.join(", ")))
            }
            (Type::Matrix(r, c), Type::Vector(b, n)) if u32::from(*n) <= from.components() => {
                let parts: Vec<String> = (0..*n)
                    .map(|k| {
                        let (i, j) = (k / c, k % c);
                        elem(Scalar::Float, *b, &format!("({code})[{i}][{j}]"), None)
                    })
                    .collect();
                let _ = r;
                Ok(format!("{}({})", to.wgsl(), parts.join(", ")))
            }
            (Type::Matrix(r, c), Type::Matrix(r2, c2)) if r2 <= r && c2 <= c => {
                let rows: Vec<String> = (0..*r2)
                    .map(|i| {
                        let row = format!("({code})[{i}]");
                        if c2 < c {
                            format!(
                                "{row}.{}",
                                XYZW[..usize::from(*c2)].iter().collect::<String>()
                            )
                        } else {
                            row
                        }
                    })
                    .collect();
                Ok(format!("{}({})", to.wgsl(), rows.join(", ")))
            }
            _ => Err(format!("cannot convert {from} to {to}")),
        }
    }

    fn unary(&mut self, op: UnOp, v: Val) -> Result<Val> {
        match op {
            UnOp::Plus => Ok(v),
            UnOp::Neg => {
                if let Some(x) = v.lit {
                    return Ok(Val::lit(-x, v.ty));
                }
                let ty = if v.ty.scalar() == Some(Scalar::Bool) {
                    v.ty.with_scalar(Scalar::Int)
                } else {
                    v.ty.clone()
                };
                let code = self.conv(&v, &ty)?;
                if let Type::Matrix(..) = ty {
                    // WGSL has no negation of matrices.
                    return Ok(Val::new(format!("({code} * -1.0)"), ty));
                }
                Ok(Val::new(format!("(-{code})"), ty))
            }
            UnOp::Not => {
                let ty = v.ty.with_scalar(Scalar::Bool);
                if !ty.is_numeric() || matches!(ty, Type::Matrix(..)) {
                    return Err(format!("! cannot apply to {}", v.ty));
                }
                let code = self.conv(&v, &ty)?;
                Ok(Val::new(format!("(!{code})"), ty))
            }
            UnOp::BitNot => {
                let ty = v.ty.with_scalar(Scalar::Int);
                let code = self.conv(&v, &ty)?;
                Ok(Val::new(format!("(~{code})"), ty))
            }
            _ => Err("increments are handled elsewhere".into()),
        }
    }

    fn binary(&mut self, op: BinOp, a: Val, b: Val, line: u32) -> Result<Val, Error> {
        let wrap = |m: String| Error { line, message: m };
        let ty = arith_type(&a.ty, &b.ty)
            .ok_or_else(|| wrap(format!("{} and {} cannot be combined", a.ty, b.ty)))?;
        let sym = match op {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::Le => "<=",
            BinOp::Ge => ">=",
        };
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                if let Type::Matrix(r, _) = ty {
                    // HLSL's arithmetic on matrices is element by element (WGSL's `*` of two
                    // matrices multiplies them), so it is written out row by row, scalars
                    // repeated to fill a matrix.
                    let ac = self.conv(&a, &ty).map_err(wrap)?;
                    let bc = self.conv(&b, &ty).map_err(wrap)?;
                    if matches!(op, BinOp::Add | BinOp::Sub) {
                        return Ok(Val::new(format!("({ac} {sym} {bc})"), ty));
                    }
                    let rows: Vec<String> = (0..r)
                        .map(|i| format!("({ac})[{i}] {sym} ({bc})[{i}]"))
                        .collect();
                    return Ok(Val::new(format!("{}({})", ty.wgsl(), rows.join(", ")), ty));
                }
                // Scalars may stay scalars beside vectors: WGSL allows vec * f32 and the like.
                let side = |v: &Val| match (&v.ty, &ty) {
                    (Type::Scalar(_), Type::Vector(s, _)) => Type::Scalar(*s),
                    (Type::Scalar(_), Type::Matrix(..)) => Type::FLOAT,
                    _ => ty.clone(),
                };
                let (ta, tb) = (side(&a), side(&b));
                let ac = self.conv(&a, &ta).map_err(wrap)?;
                let bc = self.conv(&b, &tb).map_err(wrap)?;
                let lit = match (a.lit, b.lit, op) {
                    (Some(x), Some(y), BinOp::Add) => Some(x + y),
                    (Some(x), Some(y), BinOp::Sub) => Some(x - y),
                    (Some(x), Some(y), BinOp::Mul) => Some(x * y),
                    _ => None,
                };
                if let Some(l) = lit
                    && ty.scalar() == Some(Scalar::Float)
                {
                    return Ok(Val::lit(l, Type::FLOAT));
                }
                Ok(Val::new(format!("({ac} {sym} {bc})"), ty))
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                if matches!(ty, Type::Matrix(..)) {
                    return Self::err(line, "matrices cannot be compared");
                }
                let ty =
                    if a.ty.scalar() == Some(Scalar::Bool) && b.ty.scalar() == Some(Scalar::Bool) {
                        ty.with_scalar(Scalar::Bool)
                    } else {
                        ty
                    };
                let ac = self.conv(&a, &ty).map_err(wrap)?;
                let bc = self.conv(&b, &ty).map_err(wrap)?;
                Ok(Val::new(
                    format!("({ac} {sym} {bc})"),
                    ty.with_scalar(Scalar::Bool),
                ))
            }
            BinOp::And | BinOp::Or => {
                let ty = ty.with_scalar(Scalar::Bool);
                let ac = self.conv(&a, &ty).map_err(wrap)?;
                let bc = self.conv(&b, &ty).map_err(wrap)?;
                // Vectors are combined component by component, without short-circuiting.
                let sym = match (&ty, op) {
                    (Type::Vector(..), BinOp::And) => "&",
                    (Type::Vector(..), _) => "|",
                    _ => sym,
                };
                Ok(Val::new(format!("({ac} {sym} {bc})"), ty))
            }
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                let ty = if a.ty.scalar() == Some(Scalar::Bool)
                    && b.ty.scalar() == Some(Scalar::Bool)
                    && !matches!(op, BinOp::Shl | BinOp::Shr)
                {
                    ty.with_scalar(Scalar::Bool)
                } else if ty.scalar() == Some(Scalar::Uint) {
                    ty
                } else {
                    ty.with_scalar(Scalar::Int)
                };
                let ac = self.conv(&a, &ty).map_err(wrap)?;
                let bc = if matches!(op, BinOp::Shl | BinOp::Shr) {
                    self.conv(&b, &ty.with_scalar(Scalar::Uint)).map_err(wrap)?
                } else {
                    self.conv(&b, &ty).map_err(wrap)?
                };
                Ok(Val::new(format!("({ac} {sym} {bc})"), ty))
            }
        }
    }

    fn member(&mut self, b: Val, name: &str) -> Result<Val> {
        let Some(comps) = swizzle(name) else {
            return Err(format!("unknown field '.{name}'"));
        };
        let (scalar, size) = match &b.ty {
            Type::Scalar(s) => (*s, 1),
            Type::Vector(s, n) => (*s, *n),
            other => return Err(format!("{other} has no field '.{name}'")),
        };
        if comps.iter().any(|c| *c >= size) {
            return Err(format!("'.{name}' is past the end of {}", b.ty));
        }
        let n = u8::try_from(comps.len()).unwrap_or(4);
        let ty = Type::vec(scalar, n);
        if size == 1 {
            // A scalar's swizzle repeats it.
            let code = if n == 1 {
                b.code.clone()
            } else {
                format!("{}({})", ty.wgsl(), b.code)
            };
            return Ok(Val {
                place: if n == 1 { b.place } else { None },
                ..Val::new(code, ty)
            });
        }
        let letters: String = comps.iter().map(|c| XYZW[usize::from(*c)]).collect();
        let code = format!("{}.{letters}", b.code);
        let distinct = comps.iter().collect::<HashSet<_>>().len() == comps.len();
        let place = match (&b.place, n) {
            (Some(Place::Whole), 1) => Some(Place::Whole),
            (Some(Place::Whole), _) if distinct => Some(Place::Swizzle {
                base: b.code.clone(),
                comps,
            }),
            _ => None,
        };
        Ok(Val {
            place,
            ..Val::new(code, ty)
        })
    }

    /// A constructor such as `float3(a, b)`: its arguments' numbers, in order.
    fn construct(&mut self, ty: &Type, args: Vec<Val>) -> Result<Val> {
        let Some(s) = ty.scalar() else {
            return Err(format!("{ty} cannot be constructed"));
        };
        let want = ty.components();
        if args.len() == 1 && args[0].ty.components() >= want {
            // One argument: converted (a scalar is repeated; a longer vector is cut).
            let code = self.conv(&args[0], ty)?;
            return Ok(Val::new(code, ty.clone()));
        }
        let total: u32 = args.iter().map(|a| a.ty.components()).sum();
        if total != want {
            return Err(format!("{ty} needs {want} values, not {total}"));
        }
        let mut parts = Vec::new();
        if let Type::Matrix(r, c) = ty {
            // Matrices: whole rows, or else every number on its own.
            let rows = args.iter().all(|a| {
                a.ty == Type::Vector(Scalar::Float, *c)
                    || matches!(a.ty, Type::Vector(_, n) if n == *c)
            });
            if rows && args.len() == usize::from(*r) {
                for a in &args {
                    parts.push(self.conv(a, &Type::Vector(Scalar::Float, *c))?);
                }
            } else {
                for a in &args {
                    for k in 0..a.ty.components() {
                        let comp = match &a.ty {
                            Type::Scalar(_) => a.clone(),
                            Type::Vector(sa, _) => {
                                Val::new(format!("{}[{k}]", a.code), Type::Scalar(*sa))
                            }
                            _ => return Err("matrices cannot be built from matrices".into()),
                        };
                        parts.push(self.conv(&comp, &Type::FLOAT)?);
                    }
                }
            }
        } else {
            for a in &args {
                let target = a.ty.with_scalar(s);
                if !matches!(target, Type::Scalar(_) | Type::Vector(..)) {
                    return Err(format!("{} cannot be used in a {ty}", a.ty));
                }
                parts.push(self.conv(a, &target)?);
            }
        }
        Ok(Val::new(
            format!("{}({})", ty.wgsl(), parts.join(", ")),
            ty.clone(),
        ))
    }

    fn call(&mut self, name: &str, args: &[Expr], line: u32) -> Result<Val, Error> {
        let wrap = |m: String| Error { line, message: m };
        if let Some(sig) = self.functions.get(name).cloned() {
            if sig.params.len() != args.len() {
                return Self::err(
                    line,
                    format!("'{name}' takes {} arguments", sig.params.len()),
                );
            }
            let mut codes = Vec::new();
            for ((ty, mode), a) in sig.params.iter().zip(args) {
                let v = self.expr(a)?;
                match mode {
                    ParamMode::In => codes.push(self.conv(&v, ty).map_err(wrap)?),
                    ParamMode::Out | ParamMode::InOut => {
                        if &v.ty != ty {
                            return Self::err(
                                line,
                                "out arguments must match the parameter's type",
                            );
                        }
                        let Some(addr) = v.address else {
                            return Self::err(line, "out arguments must be local variables");
                        };
                        codes.push(addr);
                    }
                }
            }
            return Ok(Val::new(
                format!("{}({})", sig.wgsl, codes.join(", ")),
                sig.ret,
            ));
        }
        if name == "modf" && args.len() == 2 {
            // modf(x, out whole) returns the fraction: WGSL's gives both in a struct.
            let x = self.expr(&args[0])?;
            let ty = common_type([&x.ty], true).map_err(wrap)?;
            let x = self.conv(&x, &ty).map_err(wrap)?;
            let tmp = self.temp();
            self.pending.push(format!("let {tmp} = modf({x});"));
            let whole = self.expr(&args[1])?;
            let lines = self.store(&whole, &Val::new(format!("{tmp}.whole"), ty.clone()), line)?;
            self.pending.extend(lines);
            return Ok(Val::new(format!("{tmp}.fract"), ty));
        }
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.expr(a)?);
        }
        self.intrinsic(name, vals).map_err(wrap)
    }

    fn intrinsic(&mut self, name: &str, args: Vec<Val>) -> Result<Val> {
        let n = args.len();
        let arity = |k: usize| {
            if n == k {
                Ok(())
            } else {
                Err(format!("'{name}' takes {k} arguments, not {n}"))
            }
        };
        // Component-wise functions: every argument converted to one type.
        let wise =
            |me: &mut Self, wgsl: &str, k: usize, float_only: bool, args: &[Val]| -> Result<Val> {
                arity(k)?;
                let ty = common_type(args.iter().map(|a| &a.ty), float_only)?;
                let codes: Result<Vec<String>> = args.iter().map(|a| me.conv(a, &ty)).collect();
                Ok(Val::new(format!("{wgsl}({})", codes?.join(", ")), ty))
            };
        match name {
            "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "exp"
            | "exp2" | "log" | "log2" | "sqrt" | "ceil" | "floor" | "round" | "trunc"
            | "radians" | "degrees" | "saturate" | "fwidth" => wise(self, name, 1, true, &args),
            "frac" => wise(self, "fract", 1, true, &args),
            "rsqrt" => wise(self, "inverseSqrt", 1, true, &args),
            "ddx" => wise(self, "dpdx", 1, true, &args),
            "ddy" => wise(self, "dpdy", 1, true, &args),
            "abs" | "sign" => wise(self, name, 1, false, &args),
            "log10" => {
                let v = wise(self, "log", 1, true, &args)?;
                Ok(Val::new(format!("({} * 0.4342944819032518)", v.code), v.ty))
            }
            "rcp" => {
                let v = wise(self, "", 1, true, &args)?;
                Ok(Val::new(format!("(1.0 / {})", v.code), v.ty))
            }
            "pow" | "atan2" | "step" => wise(self, name, 2, true, &args),
            "min" | "max" => wise(self, name, 2, false, &args),
            "fmod" => {
                arity(2)?;
                let ty = common_type(args.iter().map(|a| &a.ty), true)?;
                let (a, b) = (self.conv(&args[0], &ty)?, self.conv(&args[1], &ty)?);
                Ok(Val::new(format!("({a} % {b})"), ty))
            }
            "lerp" => wise(self, "mix", 3, true, &args),
            "smoothstep" => wise(self, name, 3, true, &args),
            "clamp" => wise(self, name, 3, false, &args),
            "mad" => {
                arity(3)?;
                let ty = common_type(args.iter().map(|a| &a.ty), false)?;
                let c: Result<Vec<String>> = args.iter().map(|a| self.conv(a, &ty)).collect();
                let c = c?;
                Ok(Val::new(format!("({} * {} + {})", c[0], c[1], c[2]), ty))
            }
            "dot" => {
                arity(2)?;
                let ty = common_type(args.iter().map(|a| &a.ty), true)?;
                let (a, b) = (self.conv(&args[0], &ty)?, self.conv(&args[1], &ty)?);
                Ok(match ty {
                    Type::Scalar(_) => Val::new(format!("({a} * {b})"), Type::FLOAT),
                    _ => Val::new(format!("dot({a}, {b})"), Type::FLOAT),
                })
            }
            "length" => {
                arity(1)?;
                let ty = common_type([&args[0].ty], true)?;
                let a = self.conv(&args[0], &ty)?;
                Ok(Val::new(
                    if matches!(ty, Type::Scalar(_)) {
                        format!("abs({a})")
                    } else {
                        format!("length({a})")
                    },
                    Type::FLOAT,
                ))
            }
            "distance" => {
                arity(2)?;
                let ty = common_type(args.iter().map(|a| &a.ty), true)?;
                let (a, b) = (self.conv(&args[0], &ty)?, self.conv(&args[1], &ty)?);
                Ok(Val::new(
                    if matches!(ty, Type::Scalar(_)) {
                        format!("abs({a} - {b})")
                    } else {
                        format!("distance({a}, {b})")
                    },
                    Type::FLOAT,
                ))
            }
            "normalize" => {
                arity(1)?;
                let ty = common_type([&args[0].ty], true)?;
                let a = self.conv(&args[0], &ty)?;
                Ok(Val::new(
                    if matches!(ty, Type::Scalar(_)) {
                        format!("sign({a})")
                    } else {
                        format!("normalize({a})")
                    },
                    ty,
                ))
            }
            "cross" => {
                arity(2)?;
                let v3 = Type::Vector(Scalar::Float, 3);
                let (a, b) = (self.conv(&args[0], &v3)?, self.conv(&args[1], &v3)?);
                Ok(Val::new(format!("cross({a}, {b})"), v3))
            }
            "reflect" => wise(self, name, 2, true, &args),
            "refract" => {
                arity(3)?;
                let ty = common_type([&args[0].ty, &args[1].ty], true)?;
                let (a, b) = (self.conv(&args[0], &ty)?, self.conv(&args[1], &ty)?);
                let eta = self.conv(&args[2], &Type::FLOAT)?;
                Ok(Val::new(format!("refract({a}, {b}, {eta})"), ty))
            }
            "any" | "all" => {
                arity(1)?;
                let ty = args[0].ty.with_scalar(Scalar::Bool);
                let a = self.conv(&args[0], &ty)?;
                Ok(Val::new(
                    if matches!(ty, Type::Vector(..)) {
                        format!("{name}({a})")
                    } else {
                        a
                    },
                    Type::BOOL,
                ))
            }
            "mul" => {
                arity(2)?;
                self.mul(&args[0], &args[1])
            }
            "transpose" => {
                arity(1)?;
                let Type::Matrix(r, c) = args[0].ty else {
                    return Err("transpose needs a matrix".into());
                };
                Ok(Val::new(
                    format!("transpose({})", args[0].code),
                    Type::Matrix(c, r),
                ))
            }
            "determinant" => {
                arity(1)?;
                if !matches!(args[0].ty, Type::Matrix(r, c) if r == c) {
                    return Err("determinant needs a square matrix".into());
                }
                Ok(Val::new(
                    format!("determinant({})", args[0].code),
                    Type::FLOAT,
                ))
            }
            "tex2D" | "tex2Dlod" | "tex2Dbias" | "tex3D" | "tex2Dgrad" => self.texture(name, &args),
            "noise" | "tex2Dproj" | "texCUBE" | "tex1D" | "lit" | "frexp" | "modf" | "ldexp"
            | "isnan" | "isinf" | "isfinite" | "sincos" | "clip" => {
                Err(format!("'{name}' is not supported here"))
            }
            _ => Err(format!("unknown function '{name}'")),
        }
    }

    fn mul(&mut self, a: &Val, b: &Val) -> Result<Val> {
        match (&a.ty, &b.ty) {
            (Type::Scalar(_), _) | (_, Type::Scalar(_)) => {
                let ty = arith_type(&a.ty, &b.ty).ok_or("mul needs numbers")?;
                let side = |v: &Val| {
                    if matches!(v.ty, Type::Scalar(_)) {
                        Type::FLOAT
                    } else {
                        ty.clone()
                    }
                };
                let (x, y) = (self.conv(a, &side(a))?, self.conv(b, &side(b))?);
                Ok(Val::new(format!("({x} * {y})"), ty))
            }
            (Type::Matrix(r, c), Type::Matrix(r2, c2)) => {
                if c != r2 {
                    return Err(format!("mul cannot multiply {} by {}", a.ty, b.ty));
                }
                Ok(Val::new(
                    format!("({} * {})", b.code, a.code),
                    Type::Matrix(*r, *c2),
                ))
            }
            (Type::Vector(_, n), Type::Matrix(r, c)) => {
                // HLSL cuts the longer of the two down to the other's size.
                let k = (*n).min(*r);
                if k < 2 {
                    return Err("mul needs at least two rows".into());
                }
                let v = self.conv(a, &Type::vec(Scalar::Float, k))?;
                let m = self.conv(b, &Type::Matrix(k, *c))?;
                Ok(Val::new(
                    format!("({m} * {v})"),
                    Type::vec(Scalar::Float, *c),
                ))
            }
            (Type::Matrix(r, c), Type::Vector(_, n)) => {
                let k = (*n).min(*c);
                if k < 2 {
                    return Err("mul needs at least two columns".into());
                }
                let v = self.conv(b, &Type::vec(Scalar::Float, k))?;
                let m = self.conv(a, &Type::Matrix(*r, k))?;
                Ok(Val::new(
                    format!("({v} * {m})"),
                    Type::vec(Scalar::Float, *r),
                ))
            }
            (Type::Vector(..), Type::Vector(..)) => {
                self.intrinsic("dot", vec![a.clone(), b.clone()])
            }
            _ => {
                let ty = arith_type(&a.ty, &b.ty).ok_or("mul needs numbers")?;
                let (x, y) = (self.conv(a, &ty)?, self.conv(b, &ty)?);
                Ok(Val::new(format!("({x} * {y})"), ty))
            }
        }
    }

    fn texture(&mut self, name: &str, args: &[Val]) -> Result<Val> {
        let Some(first) = args.first() else {
            return Err(format!("'{name}' needs a sampler"));
        };
        let Some(s) = first.sampler.clone() else {
            return Err(format!("the first argument of '{name}' must be a sampler"));
        };
        let volume = s.texture.is_volume();
        if volume != (name == "tex3D") {
            return Err(format!("'{name}' cannot sample this texture"));
        }
        let t = s.texture.binding_name();
        let f = s.filter.binding_name();
        if let Texture::Blur(n) = s.texture {
            self.used.blur = self.used.blur.max(n);
        }
        self.used.textures.insert(s.texture);
        self.used.filters.insert(s.filter);
        let v4 = Type::Vector(Scalar::Float, 4);
        let coord = |me: &Self, v: &Val, n: u8| me.conv(v, &Type::Vector(Scalar::Float, n));
        let code = match (name, args.len()) {
            ("tex2D", 2) => format!("textureSample({t}, {f}, {})", coord(self, &args[1], 2)?),
            ("tex3D", 2) => format!("textureSample({t}, {f}, {})", coord(self, &args[1], 3)?),
            ("tex2Dlod", 2) | ("tex2Dbias", 2) => {
                let c = coord(self, &args[1], 4)?;
                let tmp = self.temp();
                self.pending.push(format!("let {tmp} = {c};"));
                let call = if name == "tex2Dlod" {
                    "textureSampleLevel"
                } else {
                    "textureSampleBias"
                };
                format!("{call}({t}, {f}, {tmp}.xy, {tmp}.w)")
            }
            ("tex2Dgrad", 4) => format!(
                "textureSampleGrad({t}, {f}, {}, {}, {})",
                coord(self, &args[1], 2)?,
                coord(self, &args[2], 2)?,
                coord(self, &args[3], 2)?
            ),
            _ => return Err(format!("'{name}' takes other arguments")),
        };
        Ok(Val::new(code, v4))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniforms_map_to_the_input_block() {
        assert_eq!(uniform("_c2").map(|u| u.0), Some("inp.c[2]".into()));
        assert_eq!(uniform("_qh").map(|u| u.0), Some("inp.q[7]".into()));
        assert_eq!(uniform("rot_s1").map(|u| u.0), Some("inp.rot[0]".into()));
        assert_eq!(
            uniform("rot_rand4").map(|u| u.0),
            Some("inp.rot[23]".into())
        );
        assert_eq!(uniform("rot_vf2").map(|u| u.0), Some("inp.rot[13]".into()));
        assert_eq!(uniform("_c14"), None);
        assert_eq!(uniform("rot_s5"), None);
    }

    #[test]
    fn samplers_name_their_texture_and_filter() {
        let mut used = Used::default();
        let s = sampler_named("fc_main", &mut used);
        assert_eq!((s.texture, s.filter), (Texture::Main, Filter::LinearClamp));
        let s = sampler_named("blur2", &mut used);
        assert_eq!(
            (s.texture, s.filter),
            (Texture::Blur(2), Filter::LinearClamp)
        );
        let s = sampler_named("pw_noise_lq", &mut used);
        assert_eq!((s.texture, s.filter), (Texture::NoiseLq, Filter::PointWrap));
        let s = sampler_named("worms", &mut used);
        assert_eq!(s.texture, Texture::User(0));
        let s = sampler_named("FW_Clouds", &mut used);
        assert_eq!(
            (s.texture, s.filter),
            (Texture::User(1), Filter::LinearWrap)
        );
        assert_eq!(used.user, ["worms", "clouds"]);
    }

    #[test]
    fn arithmetic_types_follow_hlsl() {
        let f3 = Type::Vector(Scalar::Float, 3);
        let f4 = Type::Vector(Scalar::Float, 4);
        assert_eq!(arith_type(&Type::FLOAT, &f3), Some(f3.clone()));
        assert_eq!(arith_type(&f4, &f3), Some(f3.clone()));
        assert_eq!(arith_type(&Type::INT, &Type::FLOAT), Some(Type::FLOAT));
        assert_eq!(arith_type(&Type::BOOL, &Type::INT), Some(Type::INT));
        assert_eq!(swizzle("xyz"), Some(vec![0, 1, 2]));
        assert_eq!(swizzle("bgr"), Some(vec![2, 1, 0]));
        assert_eq!(swizzle("xg"), None);
    }
}
