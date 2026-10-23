//! Reading the HLSL that presets' shaders are written in, after the preprocessor: tokens,
//! then a tree of declarations, statements and expressions. Only what MilkDrop's shaders
//! use is understood; anything else is an error, and the preset falls back to MilkDrop's
//! built-in shader.

use super::Error;
use super::types::{Scalar, Type};

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Tok {
    Ident(String),
    Int(i64),
    Float(f64),
    Punct(&'static str),
}

#[derive(Clone, Debug)]
pub(super) struct Token {
    pub tok: Tok,
    pub line: u32,
}

const PUNCTS: &[&str] = &[
    "<<=", ">>=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "==", "!=", "<=", ">=", "&&",
    "||", "++", "--", "<<", ">>", "::", "+", "-", "*", "/", "%", "=", "<", ">", "!", "~", "&", "|",
    "^", "?", ":", ";", ",", ".", "(", ")", "[", "]", "{", "}",
];

pub(super) fn lex(src: &str, first_line: u32) -> Result<Vec<Token>, Error> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = first_line;
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let err = |message: String| Error { line, message };
        let tok = if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            Tok::Ident(src[start..i].to_owned())
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            let start = i;
            let hex = c == b'0' && matches!(b.get(i + 1), Some(b'x' | b'X'));
            if hex {
                i += 2;
                while i < b.len() && b[i].is_ascii_hexdigit() {
                    i += 1;
                }
                let v = i64::from_str_radix(&src[start + 2..i], 16)
                    .map_err(|_| err("bad hexadecimal number".into()))?;
                while i < b.len() && matches!(b[i], b'u' | b'U' | b'l' | b'L') {
                    i += 1;
                }
                Tok::Int(v)
            } else {
                let mut float = false;
                while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                    float |= b[i] == b'.';
                    i += 1;
                }
                if i < b.len() && matches!(b[i], b'e' | b'E') {
                    let mut j = i + 1;
                    if j < b.len() && matches!(b[j], b'+' | b'-') {
                        j += 1;
                    }
                    if j < b.len() && b[j].is_ascii_digit() {
                        float = true;
                        i = j;
                        while i < b.len() && b[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                let text = &src[start..i];
                let suffix_float = i < b.len() && matches!(b[i], b'f' | b'F' | b'h' | b'H');
                while i < b.len()
                    && matches!(b[i], b'f' | b'F' | b'h' | b'H' | b'u' | b'U' | b'l' | b'L')
                {
                    i += 1;
                }
                if float || suffix_float {
                    let v: f64 = text
                        .parse()
                        .or_else(|_| format!("{text}0").parse())
                        .map_err(|_| err(format!("bad number '{text}'")))?;
                    Tok::Float(v)
                } else {
                    Tok::Int(
                        text.parse()
                            .map_err(|_| err(format!("bad number '{text}'")))?,
                    )
                }
            }
        } else {
            let rest = &src[i..];
            let Some(p) = PUNCTS.iter().find(|p| rest.starts_with(**p)) else {
                return Err(err(format!(
                    "unexpected character '{}'",
                    rest.chars().next().unwrap_or('?')
                )));
            };
            i += p.len();
            Tok::Punct(p)
        };
        out.push(Token { tok, line });
    }
    Ok(out)
}

/// The type a name spells, if it is one of HLSL's built-in types.
pub(super) fn type_named(name: &str) -> Option<Type> {
    let scalar = |s: &str| -> Option<Scalar> {
        Some(match s {
            "float" | "half" | "double" | "min16float" | "min10float" => Scalar::Float,
            "int" | "min16int" | "min12int" => Scalar::Int,
            "uint" | "dword" | "min16uint" => Scalar::Uint,
            "bool" => Scalar::Bool,
            _ => return None,
        })
    };
    match name {
        "void" => return Some(Type::Void),
        "sampler" | "sampler2D" | "texture" | "texture2D" | "Texture2D" => {
            return Some(Type::Sampler { volume: false });
        }
        "sampler3D" | "texture3D" | "Texture3D" => return Some(Type::Sampler { volume: true }),
        "vector" => return Some(Type::Vector(Scalar::Float, 4)),
        "matrix" => return Some(Type::Matrix(4, 4)),
        _ => {}
    }
    if let Some(s) = scalar(name) {
        return Some(Type::Scalar(s));
    }
    // Vectors (float3) and matrices (float4x3), whose base is any scalar type name.
    let digits = name.trim_start_matches(|c: char| !c.is_ascii_digit());
    let base = &name[..name.len() - digits.len()];
    let s = scalar(base)?;
    let d = digits.as_bytes();
    let n = |c: u8| (b'1'..=b'4').contains(&c).then(|| c - b'0');
    match d {
        [c] => Some(match n(*c)? {
            1 => Type::Scalar(s),
            k => Type::Vector(s, k),
        }),
        [r, b'x', c] => Some(Type::Matrix(n(*r)?, n(*c)?)),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Shl,
    Shr,
    BitAnd,
    BitOr,
    BitXor,
    And,
    Or,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UnOp {
    Neg,
    Plus,
    Not,
    BitNot,
    PreInc,
    PreDec,
    PostInc,
    PostDec,
}

#[derive(Clone, Debug)]
pub(super) enum ExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    Name(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `target = value`, or with an operator such as `+=`.
    Assign(Option<BinOp>, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    /// A constructor such as `float3(a, b)`.
    Construct(Type, Vec<Expr>),
    /// A C-style cast such as `(float3)x`.
    Cast(Type, Box<Expr>),
    /// `.xyz` and the like.
    Member(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    /// `{ a, b, c }`, in initialisers.
    List(Vec<Expr>),
    /// `a, b` (in `for` clauses).
    Comma(Vec<Expr>),
}

#[derive(Clone, Debug)]
pub(super) struct Expr {
    pub kind: ExprKind,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub(super) struct VarDecl {
    pub ty: Type,
    pub name: String,
    pub init: Option<Expr>,
    pub constant: bool,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub(super) enum Stmt {
    Block(Vec<Stmt>),
    Decl(Vec<VarDecl>),
    Expr(Expr),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    For(Option<Box<Stmt>>, Option<Expr>, Option<Expr>, Box<Stmt>),
    While(Expr, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    Return(Option<Expr>, u32),
    Break,
    Continue,
    Discard,
    Empty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ParamMode {
    In,
    Out,
    InOut,
}

#[derive(Clone, Debug)]
pub(super) struct Param {
    pub ty: Type,
    pub name: String,
    pub mode: ParamMode,
}

#[derive(Clone, Debug)]
pub(super) struct Function {
    pub ret: Type,
    pub name: String,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    pub line: u32,
}

/// A declaration outside any function.
#[derive(Clone, Debug)]
pub(super) enum Item {
    /// A global variable; `uniform` ones (and non-static ones without a value) are set by
    /// MilkDrop rather than the shader.
    Var {
        decl: VarDecl,
        uniform: bool,
    },
    Function(Function),
}

pub(super) struct Parser {
    toks: Vec<Token>,
    pos: usize,
    depth: usize,
}

/// Statements and expressions nested deeper than this are refused.
const MAX_DEPTH: usize = 200;

impl Parser {
    pub(super) fn new(toks: Vec<Token>) -> Parser {
        Parser {
            toks,
            pos: 0,
            depth: 0,
        }
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    fn peek_at(&self, k: usize) -> Option<&Tok> {
        self.toks.get(self.pos + k).map(|t| &t.tok)
    }

    fn line(&self) -> u32 {
        self.toks
            .get(self.pos)
            .or_else(|| self.toks.last())
            .map_or(0, |t| t.line)
    }

    fn error<T>(&self, message: impl Into<String>) -> Result<T, Error> {
        Err(Error {
            line: self.line(),
            message: message.into(),
        })
    }

    fn is(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::Punct(q)) if *q == p)
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(q)) if q == w)
    }

    fn eat(&mut self, p: &str) -> bool {
        if self.is(p) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_word(&mut self, w: &str) -> bool {
        if self.is_word(w) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, p: &str) -> Result<(), Error> {
        if self.eat(p) {
            Ok(())
        } else {
            let found = match self.peek() {
                Some(Tok::Ident(s)) => format!("'{s}'"),
                Some(Tok::Punct(s)) => format!("'{s}'"),
                Some(Tok::Int(v)) => v.to_string(),
                Some(Tok::Float(v)) => v.to_string(),
                None => "the end".into(),
            };
            self.error(format!("expected '{p}', found {found}"))
        }
    }

    fn ident(&mut self) -> Result<String, Error> {
        match self.peek() {
            Some(Tok::Ident(s)) => {
                let s = s.clone();
                self.pos += 1;
                Ok(s)
            }
            _ => self.error("expected a name"),
        }
    }

    pub(super) fn at_end(&self) -> bool {
        self.pos >= self.toks.len()
    }

    fn deeper(&mut self) -> Result<(), Error> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return self.error("nested too deeply");
        }
        Ok(())
    }

    /// The type at the cursor, if one starts there.
    fn peek_type(&self) -> Option<Type> {
        match self.peek() {
            Some(Tok::Ident(s)) => type_named(s),
            _ => None,
        }
    }

    /// `[`...`]` attributes such as `[unroll]` or `[loop]`, which change nothing here.
    fn skip_attributes(&mut self) {
        while self.is("[")
            && matches!(self.peek_at(1), Some(Tok::Ident(_)))
            && matches!(self.peek_at(2), Some(Tok::Punct("]" | "(")))
        {
            let mut depth = 0;
            while let Some(t) = self.peek().cloned() {
                self.pos += 1;
                match t {
                    Tok::Punct("[") => depth += 1,
                    Tok::Punct("]") => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Qualifiers before a declaration. Returns (const, static, uniform).
    fn qualifiers(&mut self) -> (bool, bool, bool) {
        let (mut constant, mut stat, mut uniform) = (false, false, false);
        while let Some(Tok::Ident(w)) = self.peek() {
            match w.as_str() {
                "const" => constant = true,
                "static" => stat = true,
                "uniform" => uniform = true,
                "inline" | "extern" | "shared" | "volatile" | "row_major" | "column_major"
                | "precise" | "linear" | "centroid" | "nointerpolation" | "noperspective" => {}
                _ => break,
            }
            self.pos += 1;
        }
        (constant, stat, uniform)
    }

    /// `: SEMANTIC` or `: register(...)` after a declaration, which change nothing here.
    fn skip_semantic(&mut self) -> Result<(), Error> {
        if self.eat(":") {
            self.ident()?;
            if self.eat("(") {
                while !self.eat(")") {
                    if self.at_end() {
                        return self.error("unclosed register(");
                    }
                    self.pos += 1;
                }
            }
        }
        Ok(())
    }

    /// `[N]` after a declared name: the array length, which must be a number.
    fn array_suffix(&mut self, ty: Type) -> Result<Type, Error> {
        if !self.eat("[") {
            return Ok(ty);
        }
        let n = match self.peek() {
            Some(Tok::Int(n)) if *n > 0 && *n <= 4096 => *n,
            _ => return self.error("array lengths must be numbers"),
        };
        self.pos += 1;
        self.expect("]")?;
        if self.is("[") {
            return self.error("arrays of arrays are not supported");
        }
        // Checked to be 1..=4096 above.
        Ok(Type::Array(Box::new(ty), n as u32))
    }

    /// The declarations of the shader's header: globals and functions.
    pub(super) fn items(&mut self) -> Result<Vec<Item>, Error> {
        let mut items = Vec::new();
        while !self.at_end() {
            if self.eat(";") {
                continue;
            }
            if self.is_word("struct") || self.is_word("typedef") || self.is_word("cbuffer") {
                return self.error("structs, typedefs and constant buffers are not supported");
            }
            let line = self.line();
            let (constant, stat, uniform) = self.qualifiers();
            let Some(ty) = self.peek_type() else {
                return self.error("expected a declaration");
            };
            self.pos += 1;
            let name = self.ident()?;
            if self.is("(") {
                if let Some(f) = self.function(ty, name, line)? {
                    items.push(Item::Function(f));
                }
                continue;
            }
            // A sampler, possibly with a `sampler_state { ... }` block, which is skipped.
            let mut first = Some(name);
            loop {
                let name = match first.take() {
                    Some(n) => n,
                    None => self.ident()?,
                };
                let line = self.line();
                let ty = self.array_suffix(ty.clone())?;
                self.skip_semantic()?;
                let init = if self.eat("=") {
                    if self.eat_word("sampler_state") {
                        self.skip_braces()?;
                        None
                    } else {
                        Some(self.initializer()?)
                    }
                } else {
                    None
                };
                let implied_uniform = uniform || (!stat && init.is_none());
                items.push(Item::Var {
                    decl: VarDecl {
                        ty,
                        name,
                        init,
                        constant,
                        line,
                    },
                    uniform: implied_uniform,
                });
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(";")?;
        }
        Ok(items)
    }

    fn skip_braces(&mut self) -> Result<(), Error> {
        self.expect("{")?;
        let mut depth = 1;
        while depth > 0 {
            match self.peek() {
                None => return self.error("unclosed '{'"),
                Some(Tok::Punct("{")) => depth += 1,
                Some(Tok::Punct("}")) => depth -= 1,
                _ => {}
            }
            self.pos += 1;
        }
        Ok(())
    }

    /// A function after its return type and name; `None` for a prototype.
    fn function(&mut self, ret: Type, name: String, line: u32) -> Result<Option<Function>, Error> {
        self.expect("(")?;
        let mut params = Vec::new();
        if !self.eat(")") {
            if self.is_word("void") && matches!(self.peek_at(1), Some(Tok::Punct(")"))) {
                self.pos += 2;
            } else {
                loop {
                    let mut mode = ParamMode::In;
                    loop {
                        if self.eat_word("in") {
                        } else if self.eat_word("out") {
                            mode = ParamMode::Out;
                        } else if self.eat_word("inout") {
                            mode = ParamMode::InOut;
                        } else if !(self.eat_word("uniform") || self.eat_word("const")) {
                            break;
                        }
                    }
                    let Some(ty) = self.peek_type() else {
                        return self.error("expected a parameter type");
                    };
                    self.pos += 1;
                    let pname = self.ident()?;
                    let ty = self.array_suffix(ty)?;
                    self.skip_semantic()?;
                    if self.eat("=") {
                        return self.error("default parameter values are not supported");
                    }
                    params.push(Param {
                        ty,
                        name: pname,
                        mode,
                    });
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(")")?;
            }
        }
        self.skip_semantic()?;
        if self.eat(";") {
            return Ok(None);
        }
        let body = self.block()?;
        Ok(Some(Function {
            ret,
            name,
            params,
            body,
            line,
        }))
    }

    /// A `{ ... }` block's statements.
    pub(super) fn block(&mut self) -> Result<Vec<Stmt>, Error> {
        self.expect("{")?;
        let mut out = Vec::new();
        while !self.eat("}") {
            if self.at_end() {
                return self.error("unclosed '{'");
            }
            out.push(self.stmt()?);
        }
        Ok(out)
    }

    fn stmt(&mut self) -> Result<Stmt, Error> {
        self.deeper()?;
        let s = self.stmt_inner();
        self.depth -= 1;
        s
    }

    fn stmt_inner(&mut self) -> Result<Stmt, Error> {
        self.skip_attributes();
        if self.eat(";") {
            return Ok(Stmt::Empty);
        }
        if self.is("{") {
            return Ok(Stmt::Block(self.block()?));
        }
        let line = self.line();
        if let Some(Tok::Ident(w)) = self.peek() {
            match w.as_str() {
                "if" => {
                    self.pos += 1;
                    self.expect("(")?;
                    let c = self.expr()?;
                    self.expect(")")?;
                    let then = Box::new(self.stmt()?);
                    let otherwise = if self.eat_word("else") {
                        Some(Box::new(self.stmt()?))
                    } else {
                        None
                    };
                    return Ok(Stmt::If(c, then, otherwise));
                }
                "for" => {
                    self.pos += 1;
                    self.expect("(")?;
                    let init = if self.eat(";") {
                        None
                    } else {
                        let s = self.simple_stmt()?;
                        self.expect(";")?;
                        Some(Box::new(s))
                    };
                    let cond = if self.is(";") {
                        None
                    } else {
                        Some(self.expr()?)
                    };
                    self.expect(";")?;
                    let step = if self.is(")") {
                        None
                    } else {
                        Some(self.comma_expr()?)
                    };
                    self.expect(")")?;
                    let body = Box::new(self.stmt()?);
                    return Ok(Stmt::For(init, cond, step, body));
                }
                "while" => {
                    self.pos += 1;
                    self.expect("(")?;
                    let c = self.expr()?;
                    self.expect(")")?;
                    return Ok(Stmt::While(c, Box::new(self.stmt()?)));
                }
                "do" => {
                    self.pos += 1;
                    let body = Box::new(self.stmt()?);
                    if !self.eat_word("while") {
                        return self.error("expected 'while'");
                    }
                    self.expect("(")?;
                    let c = self.expr()?;
                    self.expect(")")?;
                    self.expect(";")?;
                    return Ok(Stmt::DoWhile(body, c));
                }
                "return" => {
                    self.pos += 1;
                    let e = if self.is(";") {
                        None
                    } else {
                        Some(self.expr()?)
                    };
                    self.expect(";")?;
                    return Ok(Stmt::Return(e, line));
                }
                "break" => {
                    self.pos += 1;
                    self.expect(";")?;
                    return Ok(Stmt::Break);
                }
                "continue" => {
                    self.pos += 1;
                    self.expect(";")?;
                    return Ok(Stmt::Continue);
                }
                "discard" => {
                    self.pos += 1;
                    self.expect(";")?;
                    return Ok(Stmt::Discard);
                }
                "switch" => return self.error("switch is not supported"),
                _ => {}
            }
        }
        let s = self.simple_stmt()?;
        self.expect(";")?;
        Ok(s)
    }

    /// A declaration or an expression, without its ";".
    fn simple_stmt(&mut self) -> Result<Stmt, Error> {
        let start = self.pos;
        let (constant, _stat, _uniform) = self.qualifiers();
        // A declaration is a type then a name; a type then "(" is a constructor.
        if let Some(ty) = self.peek_type()
            && matches!(self.peek_at(1), Some(Tok::Ident(_)))
        {
            self.pos += 1;
            let mut decls = Vec::new();
            loop {
                let line = self.line();
                let name = self.ident()?;
                let ty = self.array_suffix(ty.clone())?;
                self.skip_semantic()?;
                let init = if self.eat("=") {
                    Some(self.initializer()?)
                } else {
                    None
                };
                decls.push(VarDecl {
                    ty,
                    name,
                    init,
                    constant,
                    line,
                });
                if !self.eat(",") {
                    break;
                }
            }
            return Ok(Stmt::Decl(decls));
        }
        self.pos = start;
        Ok(Stmt::Expr(self.comma_expr()?))
    }

    /// An initialiser: an expression, or a `{ ... }` list.
    fn initializer(&mut self) -> Result<Expr, Error> {
        let line = self.line();
        if self.eat("{") {
            let mut items = Vec::new();
            while !self.eat("}") {
                items.push(self.initializer()?);
                if !self.eat(",") {
                    self.expect("}")?;
                    break;
                }
            }
            return Ok(Expr {
                kind: ExprKind::List(items),
                line,
            });
        }
        self.assign()
    }

    fn comma_expr(&mut self) -> Result<Expr, Error> {
        let line = self.line();
        let first = self.expr()?;
        if !self.is(",") {
            return Ok(first);
        }
        let mut items = vec![first];
        while self.eat(",") {
            items.push(self.expr()?);
        }
        Ok(Expr {
            kind: ExprKind::Comma(items),
            line,
        })
    }

    pub(super) fn expr(&mut self) -> Result<Expr, Error> {
        self.assign()
    }

    fn assign(&mut self) -> Result<Expr, Error> {
        self.deeper()?;
        let r = self.assign_inner();
        self.depth -= 1;
        r
    }

    fn assign_inner(&mut self) -> Result<Expr, Error> {
        let line = self.line();
        let lhs = self.ternary()?;
        let op = match self.peek() {
            Some(Tok::Punct(p)) => match *p {
                "=" => Some(None),
                "+=" => Some(Some(BinOp::Add)),
                "-=" => Some(Some(BinOp::Sub)),
                "*=" => Some(Some(BinOp::Mul)),
                "/=" => Some(Some(BinOp::Div)),
                "%=" => Some(Some(BinOp::Rem)),
                "&=" => Some(Some(BinOp::BitAnd)),
                "|=" => Some(Some(BinOp::BitOr)),
                "^=" => Some(Some(BinOp::BitXor)),
                "<<=" => Some(Some(BinOp::Shl)),
                ">>=" => Some(Some(BinOp::Shr)),
                _ => None,
            },
            _ => None,
        };
        let Some(op) = op else {
            return Ok(lhs);
        };
        self.pos += 1;
        let rhs = self.assign()?;
        Ok(Expr {
            kind: ExprKind::Assign(op, Box::new(lhs), Box::new(rhs)),
            line,
        })
    }

    fn ternary(&mut self) -> Result<Expr, Error> {
        let line = self.line();
        let c = self.binary(0)?;
        if !self.eat("?") {
            return Ok(c);
        }
        let a = self.assign()?;
        self.expect(":")?;
        let b = self.assign()?;
        Ok(Expr {
            kind: ExprKind::Ternary(Box::new(c), Box::new(a), Box::new(b)),
            line,
        })
    }

    fn binary(&mut self, min: u8) -> Result<Expr, Error> {
        // Each operator chained on the left nests the tree one deeper, which counts towards
        // the limit until this expression is done.
        let saved = self.depth;
        let r = self.binary_inner(min);
        self.depth = saved;
        r
    }

    fn binary_inner(&mut self, min: u8) -> Result<Expr, Error> {
        let mut lhs = self.unary()?;
        while let Some(Tok::Punct(p)) = self.peek() {
            let (prec, op) = match *p {
                "||" => (1, BinOp::Or),
                "&&" => (2, BinOp::And),
                "|" => (3, BinOp::BitOr),
                "^" => (4, BinOp::BitXor),
                "&" => (5, BinOp::BitAnd),
                "==" => (6, BinOp::Eq),
                "!=" => (6, BinOp::Ne),
                "<" => (7, BinOp::Lt),
                ">" => (7, BinOp::Gt),
                "<=" => (7, BinOp::Le),
                ">=" => (7, BinOp::Ge),
                "<<" => (8, BinOp::Shl),
                ">>" => (8, BinOp::Shr),
                "+" => (9, BinOp::Add),
                "-" => (9, BinOp::Sub),
                "*" => (10, BinOp::Mul),
                "/" => (10, BinOp::Div),
                "%" => (10, BinOp::Rem),
                _ => break,
            };
            if prec < min {
                break;
            }
            let line = self.line();
            self.pos += 1;
            self.deeper()?;
            let rhs = self.binary(prec + 1);
            self.depth -= 1;
            lhs = Expr {
                kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs?)),
                line,
            };
            self.deeper()?;
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, Error> {
        self.deeper()?;
        let r = self.unary_inner();
        self.depth -= 1;
        r
    }

    fn unary_inner(&mut self) -> Result<Expr, Error> {
        let line = self.line();
        let op = match self.peek() {
            Some(Tok::Punct(p)) => match *p {
                "-" => Some(UnOp::Neg),
                "+" => Some(UnOp::Plus),
                "!" => Some(UnOp::Not),
                "~" => Some(UnOp::BitNot),
                "++" => Some(UnOp::PreInc),
                "--" => Some(UnOp::PreDec),
                _ => None,
            },
            _ => None,
        };
        if let Some(op) = op {
            self.pos += 1;
            let e = self.unary()?;
            return Ok(Expr {
                kind: ExprKind::Unary(op, Box::new(e)),
                line,
            });
        }
        // A cast: "(" type ")".
        if self.is("(")
            && let Some(Tok::Ident(w)) = self.peek_at(1)
            && let Some(ty) = type_named(w)
            && matches!(self.peek_at(2), Some(Tok::Punct(")")))
        {
            self.pos += 3;
            let e = self.unary()?;
            return Ok(Expr {
                kind: ExprKind::Cast(ty, Box::new(e)),
                line,
            });
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr, Error> {
        let saved = self.depth;
        let r = self.postfix_inner();
        self.depth = saved;
        r
    }

    fn postfix_inner(&mut self) -> Result<Expr, Error> {
        let mut e = self.primary()?;
        loop {
            let line = self.line();
            if self.eat(".") {
                let name = self.ident()?;
                e = Expr {
                    kind: ExprKind::Member(Box::new(e), name),
                    line,
                };
            } else if self.eat("[") {
                let i = self.expr()?;
                self.expect("]")?;
                e = Expr {
                    kind: ExprKind::Index(Box::new(e), Box::new(i)),
                    line,
                };
            } else if self.eat("++") {
                e = Expr {
                    kind: ExprKind::Unary(UnOp::PostInc, Box::new(e)),
                    line,
                };
            } else if self.eat("--") {
                e = Expr {
                    kind: ExprKind::Unary(UnOp::PostDec, Box::new(e)),
                    line,
                };
            } else {
                return Ok(e);
            }
            self.deeper()?;
        }
    }

    fn args(&mut self) -> Result<Vec<Expr>, Error> {
        self.expect("(")?;
        let mut args = Vec::new();
        if self.eat(")") {
            return Ok(args);
        }
        loop {
            args.push(self.expr()?);
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        Ok(args)
    }

    fn primary(&mut self) -> Result<Expr, Error> {
        let line = self.line();
        let kind = match self.peek().cloned() {
            Some(Tok::Int(v)) => {
                self.pos += 1;
                ExprKind::Int(v)
            }
            Some(Tok::Float(v)) => {
                self.pos += 1;
                ExprKind::Float(v)
            }
            Some(Tok::Punct("(")) => {
                self.pos += 1;
                let e = self.comma_expr()?;
                self.expect(")")?;
                return Ok(e);
            }
            Some(Tok::Ident(w)) => {
                self.pos += 1;
                match w.as_str() {
                    "true" => ExprKind::Bool(true),
                    "false" => ExprKind::Bool(false),
                    _ => {
                        if let Some(ty) = type_named(&w)
                            && self.is("(")
                        {
                            ExprKind::Construct(ty, self.args()?)
                        } else if self.is("(") {
                            ExprKind::Call(w, self.args()?)
                        } else {
                            ExprKind::Name(w)
                        }
                    }
                }
            }
            _ => return self.error("expected an expression"),
        };
        Ok(Expr { kind, line })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn parse_body(src: &str) -> Vec<Stmt> {
        let toks = lex(&format!("{{{src}}}"), 1).unwrap();
        let mut p = Parser::new(toks);
        let b = p.block().unwrap();
        assert!(p.at_end());
        b
    }

    #[test]
    fn numbers_lex_with_their_suffixes() {
        let toks: Vec<Tok> = lex("1 1.5 .5 2. 3e2 1.0f 4h 0x1F 7u", 1)
            .unwrap()
            .into_iter()
            .map(|t| t.tok)
            .collect();
        assert_eq!(
            toks,
            [
                Tok::Int(1),
                Tok::Float(1.5),
                Tok::Float(0.5),
                Tok::Float(2.0),
                Tok::Float(300.0),
                Tok::Float(1.0),
                Tok::Float(4.0),
                Tok::Int(31),
                Tok::Int(7),
            ]
        );
    }

    #[test]
    fn type_names_cover_vectors_and_matrices() {
        assert_eq!(type_named("float3"), Some(Type::Vector(Scalar::Float, 3)));
        assert_eq!(type_named("half2"), Some(Type::Vector(Scalar::Float, 2)));
        assert_eq!(type_named("int4"), Some(Type::Vector(Scalar::Int, 4)));
        assert_eq!(type_named("float1"), Some(Type::Scalar(Scalar::Float)));
        assert_eq!(type_named("float4x3"), Some(Type::Matrix(4, 3)));
        assert_eq!(type_named("float5"), None);
        assert_eq!(type_named("ret"), None);
    }

    #[test]
    fn declarations_casts_and_constructors_parse() {
        let b = parse_body(
            "float3 a = 0, b; const float4 s[2] = { float4(1,2,3,4), {5,6,7,8} };\n\
             a = (float3)b.x + float3(1, 2, 3); ret.xy *= 2;",
        );
        assert_eq!(b.len(), 4);
        assert!(matches!(&b[0], Stmt::Decl(d) if d.len() == 2 && d[1].init.is_none()));
        assert!(
            matches!(&b[1], Stmt::Decl(d) if d[0].constant && matches!(d[0].ty, Type::Array(_, 2)))
        );
        let Stmt::Expr(e) = &b[2] else { panic!() };
        let ExprKind::Assign(None, _, rhs) = &e.kind else {
            panic!()
        };
        assert!(
            matches!(&rhs.kind, ExprKind::Binary(BinOp::Add, l, _) if matches!(l.kind, ExprKind::Cast(..)))
        );
    }

    #[test]
    fn control_flow_parses() {
        let b = parse_body(
            "[unroll] for (int i = 0, j = 1; i < 4; i++, j++) { if (i == 2) break; else continue; }\n\
             while (n <= anz) { n++; } do { x -= 1; } while (x > 0); return;",
        );
        assert!(matches!(&b[0], Stmt::For(Some(_), Some(_), Some(_), _)));
        assert!(matches!(&b[1], Stmt::While(..)));
        assert!(matches!(&b[2], Stmt::DoWhile(..)));
        assert!(matches!(&b[3], Stmt::Return(None, _)));
    }

    #[test]
    fn precedence_is_cs() {
        let b = parse_body("x = a + b * c > d && e ? f : g;");
        let Stmt::Expr(e) = &b[0] else { panic!() };
        let ExprKind::Assign(_, _, rhs) = &e.kind else {
            panic!()
        };
        let ExprKind::Ternary(c, _, _) = &rhs.kind else {
            panic!()
        };
        let ExprKind::Binary(BinOp::And, l, _) = &c.kind else {
            panic!()
        };
        let ExprKind::Binary(BinOp::Gt, l, _) = &l.kind else {
            panic!()
        };
        assert!(
            matches!(&l.kind, ExprKind::Binary(BinOp::Add, _, r) if matches!(r.kind, ExprKind::Binary(BinOp::Mul, ..)))
        );
    }

    #[test]
    fn header_items_parse() {
        let toks = lex(
            "sampler sampler_worms; sampler2D sampler_pic : register(s1);\n\
             sampler sampler_grad = sampler_state { Texture = <grad>; };\n\
             float4 texsize_worms; static const float3 c = float3(1,2,3);\n\
             float3 f(float2 uv, out float k) : COLOR { k = 1; return uv.xyx; }\n\
             float g(float x);",
            1,
        )
        .unwrap();
        let items = Parser::new(toks).items().unwrap();
        assert_eq!(items.len(), 6);
        assert!(
            matches!(&items[3], Item::Var { uniform: true, decl } if decl.name == "texsize_worms")
        );
        assert!(matches!(&items[4], Item::Var { uniform: false, decl } if decl.init.is_some()));
        let Item::Function(f) = &items[5] else {
            panic!()
        };
        assert_eq!(f.params[1].mode, ParamMode::Out);
    }

    #[test]
    fn deep_nesting_is_refused() {
        let src = format!("{{x = {}1{};}}", "(".repeat(500), ")".repeat(500));
        let toks = lex(&src, 1).unwrap();
        assert!(Parser::new(toks).block().is_err());
    }
}
