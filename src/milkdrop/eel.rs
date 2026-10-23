//! MilkDrop's expression language, ns-eel2: the per-frame, per-vertex, wave and shape code in
//! presets. Code is parsed once into a tree and walked each time it runs.
//!
//! Every value is an `f64`. Names are case-insensitive; a name that is not a function is a
//! variable, created at 0 the first time it is mentioned. Precedence and the meaning of each
//! operator and function follow projectM's documentation of MilkDrop's parser
//! (`projectm-eval`'s Expression-Syntax.md), with a few details from eel-wasm, butterchurn's
//! compiler, where it matched MilkDrop more closely (`%` of negative numbers, division by
//! exactly zero).
//!
//! Variables live in a [`Vars`] table that several programs can share (a preset's init and
//! per-frame code do), together with its own memory buffer (`megabuf`). [`Globals`] holds what
//! all code shares, even across presets: `reg00` to `reg99` and `gmegabuf`.

use std::collections::HashMap;
use std::fmt;

/// Comparisons treat numbers this close as equal, and truth means further than this from 0.
const CLOSE: f64 = 0.00001;
/// A single `loop` or `while` runs at most this many times, as in MilkDrop.
const MAX_LOOP: i64 = 1 << 20;
/// Loop iterations allowed in one run of a program, so that nested loops cannot hang the
/// player. MilkDrop had no such limit; presets that come near it are too slow to watch anyway.
const RUN_BUDGET: u64 = 1 << 24;
/// Entries in each memory block, and blocks in a buffer: 8,388,608 entries in all.
const BLOCK: usize = 1 << 16;
const BLOCKS: usize = 128;
/// Expressions nested deeper than this are refused, so walking them cannot overflow the stack.
const MAX_DEPTH: usize = 400;

/// Which version of the language a preset was written for. MilkDrop 1's `rand(n)` gave whole
/// numbers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Dialect {
    Eel1,
    #[default]
    Eel2,
}

/// Where a variable lives in its [`Vars`] table.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Slot(usize);

/// A table of variables, and the memory buffer (`megabuf`) of the code that uses it.
#[derive(Default)]
pub struct Vars {
    index: HashMap<Box<str>, Slot>,
    values: Vec<f64>,
    mem: Memory,
}

impl fmt::Debug for Vars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vars")
            .field("count", &self.values.len())
            .finish_non_exhaustive()
    }
}

impl Vars {
    #[must_use]
    pub fn new() -> Vars {
        Vars::default()
    }

    /// The slot of variable `name` (any case), created at 0 if it is new.
    pub fn slot(&mut self, name: &str) -> Slot {
        let name = name.to_ascii_lowercase();
        if let Some(&s) = self.index.get(name.as_str()) {
            return s;
        }
        let s = Slot(self.values.len());
        self.values.push(0.0);
        self.index.insert(name.into_boxed_str(), s);
        s
    }

    /// The slot of variable `name` (any case), if anything has mentioned it.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<Slot> {
        self.index.get(name.to_ascii_lowercase().as_str()).copied()
    }

    #[must_use]
    pub fn get(&self, s: Slot) -> f64 {
        self.values.get(s.0).copied().unwrap_or(0.0)
    }

    pub fn set(&mut self, s: Slot, value: f64) {
        if let Some(v) = self.values.get_mut(s.0) {
            *v = value;
        }
    }

    /// Every variable's name and slot, in no particular order.
    pub fn names(&self) -> impl Iterator<Item = (&str, Slot)> {
        self.index.iter().map(|(n, s)| (&**n, *s))
    }
}

/// What every program shares: `reg00` to `reg99`, the global memory buffer, and the random
/// numbers.
pub struct Globals {
    regs: [f64; 100],
    gmem: Memory,
    rng: Rng,
}

impl fmt::Debug for Globals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Globals").finish_non_exhaustive()
    }
}

impl Globals {
    /// Shared state whose random numbers start from `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Globals {
        Globals {
            regs: [0.0; 100],
            gmem: Memory::default(),
            rng: Rng::new(seed),
        }
    }

    /// A random number in `0..1`, from the same sequence as the code's `rand`.
    pub fn random(&mut self) -> f64 {
        f64::from(self.rng.next_u32()) / f64::from(u32::MAX)
    }
}

/// A memory buffer: 8,388,608 numbers, allocated 65,536 at a time as they are written.
#[derive(Default)]
struct Memory {
    blocks: Vec<Option<Box<[f64]>>>,
}

impl Memory {
    fn get(&self, i: usize) -> f64 {
        self.blocks
            .get(i / BLOCK)
            .and_then(Option::as_deref)
            .map_or(0.0, |b| b[i % BLOCK])
    }

    fn set(&mut self, i: usize, v: f64) {
        let b = i / BLOCK;
        if b >= BLOCKS {
            return;
        }
        if self.blocks.len() <= b {
            self.blocks.resize_with(b + 1, || None);
        }
        self.blocks[b].get_or_insert_with(|| vec![0.0; BLOCK].into_boxed_slice())[i % BLOCK] = v;
    }
}

/// A memory index from a number, as MilkDrop rounds them; `None` outside the buffer.
fn mem_index(v: f64) -> Option<usize> {
    let i = (v + CLOSE).trunc();
    // Checked against the range first, so the cast is exact.
    (0.0..(BLOCK * BLOCKS) as f64)
        .contains(&i)
        .then_some(i as usize)
}

/// xorshift64*: small and fast; presets only need numbers that look random.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }

    fn next_u32(&mut self) -> u32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        // The top 32 bits are the best mixed.
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }
}

/// Why code could not be compiled.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Error {
    /// Byte offset in the code.
    pub at: usize,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at character {}", self.message, self.at)
    }
}

impl std::error::Error for Error {}

/// Compiled code, to be run against the [`Vars`] it was compiled with.
#[derive(Debug)]
pub struct Program {
    code: Expr,
    dialect: Dialect,
}

impl Program {
    /// Compile `src`, creating the variables it names in `vars`.
    ///
    /// # Errors
    ///
    /// When `src` is not valid code: a syntax error, an unknown function, or a function given
    /// the wrong number of arguments.
    pub fn compile(src: &str, vars: &mut Vars, dialect: Dialect) -> Result<Program, Error> {
        let tokens = lex(src)?;
        let mut p = Parser {
            tokens: &tokens,
            pos: 0,
            vars,
            depth: 0,
            end: src.len(),
        };
        let code = p.list()?;
        if let Some(t) = p.tokens.get(p.pos) {
            return Err(Error {
                at: t.at,
                message: format!("unexpected {}", t.kind),
            });
        }
        Ok(Program { code, dialect })
    }

    /// Whether the code does nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        matches!(&self.code, Expr::Seq(list) if list.is_empty())
    }

    /// Run the code, returning the value of its last expression.
    pub fn run(&self, vars: &mut Vars, globals: &mut Globals) -> f64 {
        let mut m = Machine {
            vars: &mut vars.values,
            mem: &mut vars.mem,
            globals,
            dialect: self.dialect,
            budget: RUN_BUDGET,
        };
        m.eval(&self.code)
    }
}

// ---------------------------------------------------------------------------------------------
// Lexing

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    BitOr,
    BitAnd,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

#[derive(Clone, PartialEq, Debug)]
enum Kind {
    Num(f64),
    Name(String),
    /// A binary operator.
    Op(Op),
    /// `=`, or a compound assignment such as `+=`.
    Assign(Option<Op>),
    AndAnd,
    OrOr,
    Not,
    Question,
    Colon,
    Comma,
    Semi,
    LParen,
    RParen,
    LBracket,
    RBracket,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Kind::Num(n) => write!(f, "number {n}"),
            Kind::Name(n) => write!(f, "'{n}'"),
            Kind::Op(_) | Kind::Assign(_) => f.write_str("operator"),
            Kind::AndAnd => f.write_str("'&&'"),
            Kind::OrOr => f.write_str("'||'"),
            Kind::Not => f.write_str("'!'"),
            Kind::Question => f.write_str("'?'"),
            Kind::Colon => f.write_str("':'"),
            Kind::Comma => f.write_str("','"),
            Kind::Semi => f.write_str("';'"),
            Kind::LParen => f.write_str("'('"),
            Kind::RParen => f.write_str("')'"),
            Kind::LBracket => f.write_str("'['"),
            Kind::RBracket => f.write_str("']'"),
        }
    }
}

#[derive(Debug)]
struct Token {
    kind: Kind,
    at: usize,
}

fn lex(src: &str) -> Result<Vec<Token>, Error> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let err = |at: usize, message: &str| Error {
        at,
        message: message.to_owned(),
    };
    while i < b.len() {
        let c = b[i];
        let next = b.get(i + 1).copied();
        let at = i;
        // Comments: // and \\ to the end of the line, /* to */.
        if (c == b'/' && next == Some(b'/')) || (c == b'\\' && next == Some(b'\\')) {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && next == Some(b'*') {
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i = (i + 2).min(b.len());
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let kind = if c.is_ascii_digit() || c == b'.' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            // An exponent only when digits follow, so "2e" stays a number and a name.
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                let mut j = i + 1;
                if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                    j += 1;
                }
                if j < b.len() && b[j].is_ascii_digit() {
                    i = j;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let text = &src[start..i];
            // "1." is 1 and a lone "." is 0; "1.2.3" reads as far as it can, like atof.
            let value = number_prefix(text);
            Kind::Num(value)
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') {
                i += 1;
            }
            Kind::Name(src[start..i].to_ascii_lowercase())
        } else if c == b'$' {
            i += 1;
            if next == Some(b'\'') {
                // $'c': the character's code.
                let ch = b
                    .get(i + 1)
                    .copied()
                    .ok_or_else(|| err(at, "unfinished $'c'"))?;
                if b.get(i + 2) != Some(&b'\'') {
                    return Err(err(at, "unfinished $'c'"));
                }
                i += 3;
                Kind::Num(f64::from(ch))
            } else {
                let start = i;
                while i < b.len() && b[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                let name = src[start..i].to_ascii_lowercase();
                match name.as_str() {
                    "pi" => Kind::Num(std::f64::consts::PI),
                    "e" => Kind::Num(std::f64::consts::E),
                    "phi" => Kind::Num(1.618_033_99),
                    _ => match name.strip_prefix('x') {
                        Some(hex) if !hex.is_empty() => {
                            let v = u64::from_str_radix(hex, 16)
                                .map_err(|_| err(at, "bad hexadecimal number"))?;
                            // Hexadecimal constants are small in practice; precision beyond
                            // 2^53 does not matter.
                            Kind::Num(v as f64)
                        }
                        _ => return Err(err(at, "unknown $ constant")),
                    },
                }
            }
        } else {
            let two = |second: u8| next == Some(second);
            let (kind, len) = match c {
                b'+' if two(b'=') => (Kind::Assign(Some(Op::Add)), 2),
                b'-' if two(b'=') => (Kind::Assign(Some(Op::Sub)), 2),
                b'*' if two(b'=') => (Kind::Assign(Some(Op::Mul)), 2),
                b'/' if two(b'=') => (Kind::Assign(Some(Op::Div)), 2),
                b'%' if two(b'=') => (Kind::Assign(Some(Op::Mod)), 2),
                b'^' if two(b'=') => (Kind::Assign(Some(Op::Pow)), 2),
                b'|' if two(b'=') => (Kind::Assign(Some(Op::BitOr)), 2),
                b'&' if two(b'=') => (Kind::Assign(Some(Op::BitAnd)), 2),
                b'=' if two(b'=') => (Kind::Op(Op::Eq), 2),
                b'!' if two(b'=') => (Kind::Op(Op::Ne), 2),
                b'<' if two(b'=') => (Kind::Op(Op::Le), 2),
                b'>' if two(b'=') => (Kind::Op(Op::Ge), 2),
                b'&' if two(b'&') => (Kind::AndAnd, 2),
                b'|' if two(b'|') => (Kind::OrOr, 2),
                b'+' => (Kind::Op(Op::Add), 1),
                b'-' => (Kind::Op(Op::Sub), 1),
                b'*' => (Kind::Op(Op::Mul), 1),
                b'/' => (Kind::Op(Op::Div), 1),
                b'%' => (Kind::Op(Op::Mod), 1),
                b'^' => (Kind::Op(Op::Pow), 1),
                b'|' => (Kind::Op(Op::BitOr), 1),
                b'&' => (Kind::Op(Op::BitAnd), 1),
                b'<' => (Kind::Op(Op::Lt), 1),
                b'>' => (Kind::Op(Op::Gt), 1),
                b'=' => (Kind::Assign(None), 1),
                b'!' => (Kind::Not, 1),
                b'?' => (Kind::Question, 1),
                b':' => (Kind::Colon, 1),
                b',' => (Kind::Comma, 1),
                b';' => (Kind::Semi, 1),
                b'(' => (Kind::LParen, 1),
                b')' => (Kind::RParen, 1),
                b'[' => (Kind::LBracket, 1),
                b']' => (Kind::RBracket, 1),
                _ => return Err(err(at, "unexpected character")),
            };
            i += len;
            kind
        };
        out.push(Token { kind, at });
    }
    Ok(out)
}

/// The value of the longest prefix of `text` (digits, dots and an exponent) that is a number,
/// as C's `atof` reads it; 0 if none is.
fn number_prefix(text: &str) -> f64 {
    (1..=text.len())
        .rev()
        .find_map(|n| text.get(..n).and_then(|t| t.parse::<f64>().ok()))
        .unwrap_or(0.0)
}

// ---------------------------------------------------------------------------------------------
// Parsing

/// A function of one number.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum F1 {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sqr,
    Sqrt,
    Exp,
    Log,
    Log10,
    Abs,
    Sign,
    Floor,
    Ceil,
    InvSqrt,
    Not,
    Neg,
    Rand,
}

/// A function of two numbers (operators included), both always evaluated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum F2 {
    Op(Op),
    Atan2,
    Min,
    Max,
    Sigmoid,
    Band,
    Bor,
}

#[derive(Debug)]
enum Expr {
    Num(f64),
    Var(Slot),
    Reg(usize),
    /// An entry in the local memory buffer, by index.
    Mem(Box<Expr>),
    /// An entry in the global memory buffer, by index.
    GMem(Box<Expr>),
    Call1(F1, Box<Expr>),
    Call2(F2, Box<Expr>, Box<Expr>),
    /// `&&` and `||`, which evaluate their right side only when needed.
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `target = value`, or a compound assignment such as `+=`.
    Assign(Option<Op>, Box<Expr>, Box<Expr>),
    /// Expressions run in turn; the last one's value is the result.
    Seq(Vec<Expr>),
    Loop(Box<Expr>, Box<Expr>),
    While(Box<Expr>),
    MemCpy(Box<Expr>, Box<Expr>, Box<Expr>),
    MemSet(Box<Expr>, Box<Expr>, Box<Expr>),
}

/// What a name followed by `(` means, and how many arguments it takes.
#[derive(Clone, Copy, Debug)]
enum Func {
    One(F1),
    Two(F2),
    If,
    And,
    Or,
    Loop,
    While,
    Exec(usize),
    Assign(Option<Op>),
    Mem,
    GMem,
    FreeMem,
    MemCpy,
    MemSet,
}

impl Func {
    fn lookup(name: &str) -> Option<Func> {
        use F1::*;
        let f = match name {
            "sin" => Func::One(Sin),
            "cos" => Func::One(Cos),
            "tan" => Func::One(Tan),
            "asin" => Func::One(Asin),
            "acos" => Func::One(Acos),
            "atan" => Func::One(Atan),
            "sqr" => Func::One(Sqr),
            "sqrt" => Func::One(Sqrt),
            "exp" => Func::One(Exp),
            "log" => Func::One(Log),
            "log10" => Func::One(Log10),
            "abs" => Func::One(Abs),
            "sign" => Func::One(Sign),
            // MilkDrop's int() rounds down, like floor().
            "floor" | "int" => Func::One(Floor),
            "ceil" => Func::One(Ceil),
            "invsqrt" => Func::One(InvSqrt),
            "bnot" | "_not" => Func::One(Not),
            "_neg" => Func::One(Neg),
            "rand" => Func::One(Rand),
            "atan2" => Func::Two(F2::Atan2),
            "pow" => Func::Two(F2::Op(Op::Pow)),
            "min" => Func::Two(F2::Min),
            "max" => Func::Two(F2::Max),
            "sigmoid" => Func::Two(F2::Sigmoid),
            "band" => Func::Two(F2::Band),
            "bor" => Func::Two(F2::Bor),
            "above" | "_above" => Func::Two(F2::Op(Op::Gt)),
            "below" | "_below" => Func::Two(F2::Op(Op::Lt)),
            "equal" | "_equal" => Func::Two(F2::Op(Op::Eq)),
            "_noteq" => Func::Two(F2::Op(Op::Ne)),
            "_aboeq" => Func::Two(F2::Op(Op::Ge)),
            "_beleq" => Func::Two(F2::Op(Op::Le)),
            "_add" => Func::Two(F2::Op(Op::Add)),
            "_sub" => Func::Two(F2::Op(Op::Sub)),
            "_mul" => Func::Two(F2::Op(Op::Mul)),
            "_div" => Func::Two(F2::Op(Op::Div)),
            "_mod" => Func::Two(F2::Op(Op::Mod)),
            "_and" => Func::And,
            "_or" => Func::Or,
            "if" | "_if" => Func::If,
            "loop" => Func::Loop,
            "while" => Func::While,
            "exec2" => Func::Exec(2),
            "exec3" => Func::Exec(3),
            "assign" | "_set" => Func::Assign(None),
            "_addop" => Func::Assign(Some(Op::Add)),
            "_subop" => Func::Assign(Some(Op::Sub)),
            "_mulop" => Func::Assign(Some(Op::Mul)),
            "_divop" => Func::Assign(Some(Op::Div)),
            "_modop" => Func::Assign(Some(Op::Mod)),
            "_powop" => Func::Assign(Some(Op::Pow)),
            "_orop" => Func::Assign(Some(Op::BitOr)),
            "_andop" => Func::Assign(Some(Op::BitAnd)),
            "megabuf" | "_mem" => Func::Mem,
            "gmegabuf" | "_gmem" => Func::GMem,
            "freembuf" => Func::FreeMem,
            "memcpy" => Func::MemCpy,
            "memset" => Func::MemSet,
            _ => return None,
        };
        Some(f)
    }

    fn arity(self) -> usize {
        match self {
            Func::One(_) | Func::While | Func::Mem | Func::GMem | Func::FreeMem => 1,
            Func::Two(_) | Func::And | Func::Or | Func::Loop | Func::Assign(_) => 2,
            Func::If | Func::MemCpy | Func::MemSet => 3,
            Func::Exec(n) => n,
        }
    }

    fn build(self, mut args: Vec<Expr>) -> Expr {
        let mut next = || Box::new(args.remove(0));
        match self {
            Func::One(f) => Expr::Call1(f, next()),
            Func::Two(f) => Expr::Call2(f, next(), next()),
            Func::If => Expr::If(next(), next(), next()),
            Func::And => Expr::And(next(), next()),
            Func::Or => Expr::Or(next(), next()),
            Func::Loop => Expr::Loop(next(), next()),
            Func::While => Expr::While(next()),
            Func::Exec(_) => Expr::Seq(args),
            Func::Assign(op) => Expr::Assign(op, next(), next()),
            Func::Mem => Expr::Mem(next()),
            Func::GMem => Expr::GMem(next()),
            // freembuf() frees nothing in MilkDrop either.
            Func::FreeMem => *next(),
            Func::MemCpy => Expr::MemCpy(next(), next(), next()),
            Func::MemSet => Expr::MemSet(next(), next(), next()),
        }
    }
}

/// Binding powers, from projectM's grammar (lowest first): `=`; `+=` `-=`; `*=` `/=` `%=`;
/// `^=` `|=` `&=`; `?:`; `||`; `&&`; `|`; `&`; `==` `!=`; `>` `>=`; `<` `<=`; `+` `-`;
/// `*` `/`; `%`; `!`; `^`; unary `-` `+`; `[]`. Each level `n` binds at `2n`, one more on
/// the side that associates.
const fn level(n: u8) -> u8 {
    2 * n
}
const NOT_LEVEL: u8 = level(16);
const NEG_LEVEL: u8 = level(18);

/// The left and right binding powers of an infix token.
fn infix_power(kind: &Kind) -> Option<(u8, u8)> {
    let left = |n: u8| (level(n), level(n) + 1);
    let right = |n: u8| (level(n) + 1, level(n));
    Some(match kind {
        Kind::Assign(None) => right(1),
        Kind::Assign(Some(Op::Add | Op::Sub)) => right(2),
        Kind::Assign(Some(Op::Mul | Op::Div | Op::Mod)) => right(3),
        Kind::Assign(Some(_)) => right(4),
        Kind::Question => right(5),
        Kind::OrOr => left(6),
        Kind::AndAnd => left(7),
        Kind::Op(Op::BitOr) => left(8),
        Kind::Op(Op::BitAnd) => left(9),
        Kind::Op(Op::Eq | Op::Ne) => left(10),
        Kind::Op(Op::Gt | Op::Ge) => left(11),
        Kind::Op(Op::Lt | Op::Le) => left(12),
        Kind::Op(Op::Add | Op::Sub) => left(13),
        Kind::Op(Op::Mul | Op::Div) => left(14),
        Kind::Op(Op::Mod) => left(15),
        Kind::Op(Op::Pow) => left(17),
        Kind::LBracket => left(19),
        _ => return None,
    })
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    vars: &'a mut Vars,
    depth: usize,
    /// Where the code ends, for errors there.
    end: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Kind> {
        self.tokens.get(self.pos).map(|t| &t.kind)
    }

    fn at(&self) -> usize {
        self.tokens.get(self.pos).map_or(self.end, |t| t.at)
    }

    fn error(&self, message: impl Into<String>) -> Error {
        Error {
            at: self.at(),
            message: message.into(),
        }
    }

    fn expect(&mut self, kind: &Kind) -> Result<(), Error> {
        if self.peek() == Some(kind) {
            self.pos += 1;
            Ok(())
        } else {
            Err(match self.peek() {
                Some(k) => self.error(format!("expected {kind}, found {k}")),
                None => self.error(format!("expected {kind} at the end")),
            })
        }
    }

    /// Expressions separated by semicolons, any of them empty; stops before `)`, `]`, `,`,
    /// `:` or the end.
    fn list(&mut self) -> Result<Expr, Error> {
        let mut items = Vec::new();
        loop {
            match self.peek() {
                None | Some(Kind::RParen | Kind::RBracket | Kind::Comma | Kind::Colon) => break,
                Some(Kind::Semi) => {
                    self.pos += 1;
                }
                Some(_) => {
                    items.push(self.expr(0)?);
                    match self.peek() {
                        Some(Kind::Semi) => self.pos += 1,
                        None | Some(Kind::RParen | Kind::RBracket | Kind::Comma | Kind::Colon) => {
                            break;
                        }
                        Some(k) => return Err(self.error(format!("expected ';', found {k}"))),
                    }
                }
            }
        }
        Ok(if items.len() == 1 {
            items.remove(0)
        } else {
            Expr::Seq(items)
        })
    }

    /// An expression whose operators all bind at least as tightly as `min`.
    fn expr(&mut self, min: u8) -> Result<Expr, Error> {
        // Each level of the tree counts, both nesting and each operator chained on the left,
        // since walking the tree recurses as deep as it goes.
        let saved = self.depth;
        let result = self.deeper().and_then(|()| self.expr_inner(min));
        self.depth = saved;
        result
    }

    fn deeper(&mut self) -> Result<(), Error> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error("expression nested too deeply"));
        }
        Ok(())
    }

    fn expr_inner(&mut self, min: u8) -> Result<Expr, Error> {
        let mut lhs = self.prefix()?;
        while let Some(kind) = self.peek() {
            let Some((l, r)) = infix_power(kind) else {
                break;
            };
            if l < min {
                break;
            }
            let kind = kind.clone();
            self.pos += 1;
            lhs = match kind {
                Kind::LBracket => {
                    // index[offset], or index[] for offset 0.
                    let offset = if self.peek() == Some(&Kind::RBracket) {
                        None
                    } else {
                        Some(self.list()?)
                    };
                    self.expect(&Kind::RBracket)?;
                    let index = match offset {
                        Some(o) => Expr::Call2(F2::Op(Op::Add), Box::new(lhs), Box::new(o)),
                        None => lhs,
                    };
                    Expr::Mem(Box::new(index))
                }
                Kind::Question => {
                    let yes = self.list()?;
                    self.expect(&Kind::Colon)?;
                    let no = self.expr(r)?;
                    Expr::If(Box::new(lhs), Box::new(yes), Box::new(no))
                }
                Kind::Assign(op) => Expr::Assign(op, Box::new(lhs), Box::new(self.expr(r)?)),
                Kind::AndAnd => Expr::And(Box::new(lhs), Box::new(self.expr(r)?)),
                Kind::OrOr => Expr::Or(Box::new(lhs), Box::new(self.expr(r)?)),
                Kind::Op(op) => Expr::Call2(F2::Op(op), Box::new(lhs), Box::new(self.expr(r)?)),
                _ => unreachable!("infix_power only knows infix tokens"),
            };
            self.deeper()?;
        }
        Ok(lhs)
    }

    fn prefix(&mut self) -> Result<Expr, Error> {
        let Some(token) = self.tokens.get(self.pos) else {
            return Err(self.error("unexpected end of code"));
        };
        self.pos += 1;
        match &token.kind {
            Kind::Num(n) => Ok(Expr::Num(*n)),
            Kind::Op(Op::Sub) => Ok(Expr::Call1(F1::Neg, Box::new(self.expr(NEG_LEVEL)?))),
            Kind::Op(Op::Add) => self.expr(NEG_LEVEL),
            Kind::Not => Ok(Expr::Call1(F1::Not, Box::new(self.expr(NOT_LEVEL)?))),
            Kind::LParen => {
                let inner = self.list()?;
                self.expect(&Kind::RParen)?;
                Ok(inner)
            }
            Kind::Name(name) => {
                let name = name.clone();
                if self.peek() == Some(&Kind::LParen) {
                    self.pos += 1;
                    return self.call(&name, token.at);
                }
                if name == "gmem" && self.peek() == Some(&Kind::LBracket) {
                    self.pos += 1;
                    let index = if self.peek() == Some(&Kind::RBracket) {
                        Expr::Num(0.0)
                    } else {
                        self.list()?
                    };
                    self.expect(&Kind::RBracket)?;
                    return Ok(Expr::GMem(Box::new(index)));
                }
                Ok(register(&name).map_or_else(|| Expr::Var(self.vars.slot(&name)), Expr::Reg))
            }
            k => Err(Error {
                at: token.at,
                message: format!("unexpected {k}"),
            }),
        }
    }

    fn call(&mut self, name: &str, at: usize) -> Result<Expr, Error> {
        let Some(f) = Func::lookup(name) else {
            return Err(Error {
                at,
                message: format!("unknown function '{name}'"),
            });
        };
        let mut args = Vec::new();
        if self.peek() != Some(&Kind::RParen) {
            loop {
                args.push(self.list()?);
                if self.peek() == Some(&Kind::Comma) {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }
        self.expect(&Kind::RParen)?;
        if args.len() != f.arity() {
            return Err(Error {
                at,
                message: format!("'{name}' takes {} arguments, not {}", f.arity(), args.len()),
            });
        }
        Ok(f.build(args))
    }
}

/// `reg00` to `reg99` (exactly two digits) are the shared registers.
fn register(name: &str) -> Option<usize> {
    let digits = name.strip_prefix("reg")?;
    if digits.len() == 2 && digits.bytes().all(|b| b.is_ascii_digit()) {
        digits.parse().ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------------------------
// Running

/// Where an assignment goes: a variable, a register, a memory entry, or nowhere (the target
/// was only a value, which the assignment then returns).
#[derive(Clone, Copy, Debug)]
enum Place {
    Var(Slot),
    Reg(usize),
    Mem(usize),
    GMem(usize),
    Value(f64),
}

struct Machine<'a> {
    vars: &'a mut [f64],
    mem: &'a mut Memory,
    globals: &'a mut Globals,
    dialect: Dialect,
    /// Loop iterations left in this run.
    budget: u64,
}

fn truthy(v: f64) -> bool {
    v.abs() > CLOSE
}

fn flag(b: bool) -> f64 {
    if b { 1.0 } else { 0.0 }
}

/// A number as x87's `fistp` made an integer of it in MilkDrop's `%`: its size, truncated, and
/// 2^31 for anything too large or not a number.
fn x87_int(v: f64) -> i64 {
    const LIMIT: f64 = 2_147_483_648.0;
    // Below 2^31 in size, so the cast is exact apart from the dropped fraction.
    if v < LIMIT { v.abs() as i64 } else { 1 << 31 }
}

fn apply(op: Op, a: f64, b: f64) -> f64 {
    match op {
        Op::Add => a + b,
        Op::Sub => a - b,
        Op::Mul => a * b,
        Op::Div => {
            if b == 0.0 {
                0.0
            } else {
                a / b
            }
        }
        Op::Mod => {
            let d = x87_int(b);
            if d == 0 {
                0.0
            } else {
                // Both non-negative and below 2^32, so neither the remainder nor its
                // conversion can go wrong.
                (x87_int(a) % d) as f64
            }
        }
        Op::Pow => {
            if a.abs() < CLOSE && b < 0.0 {
                return 0.0;
            }
            let r = a.powf(b);
            if r.is_nan() { 0.0 } else { r }
        }
        // Saturating casts: the bits of numbers past 2^63 do not matter to anyone.
        Op::BitOr => ((a as i64) | (b as i64)) as f64,
        Op::BitAnd => ((a as i64) & (b as i64)) as f64,
        Op::Eq => flag((a - b).abs() < CLOSE),
        Op::Ne => flag((a - b).abs() > CLOSE),
        Op::Lt => flag(a < b),
        Op::Gt => flag(a > b),
        Op::Le => flag(a <= b),
        Op::Ge => flag(a >= b),
    }
}

impl Machine<'_> {
    fn eval(&mut self, e: &Expr) -> f64 {
        match e {
            Expr::Num(n) => *n,
            Expr::Var(s) => self.vars.get(s.0).copied().unwrap_or(0.0),
            Expr::Reg(r) => self.globals.regs[*r],
            Expr::Mem(i) => {
                let i = self.eval(i);
                mem_index(i).map_or(0.0, |i| self.mem.get(i))
            }
            Expr::GMem(i) => {
                let i = self.eval(i);
                mem_index(i).map_or(0.0, |i| self.globals.gmem.get(i))
            }
            Expr::Call1(f, a) => {
                let a = self.eval(a);
                self.call1(*f, a)
            }
            Expr::Call2(f, a, b) => {
                let a = self.eval(a);
                let b = self.eval(b);
                match f {
                    F2::Op(op) => apply(*op, a, b),
                    F2::Atan2 => a.atan2(b),
                    F2::Min => a.min(b),
                    F2::Max => a.max(b),
                    F2::Sigmoid => {
                        let t = 1.0 + (-a * b).exp();
                        if t.abs() > CLOSE { 1.0 / t } else { 0.0 }
                    }
                    F2::Band => flag(truthy(a) && truthy(b)),
                    F2::Bor => flag(truthy(a) || truthy(b)),
                }
            }
            Expr::And(a, b) => flag(truthy(self.eval(a)) && truthy(self.eval(b))),
            Expr::Or(a, b) => flag(truthy(self.eval(a)) || truthy(self.eval(b))),
            Expr::If(c, yes, no) => {
                if truthy(self.eval(c)) {
                    self.eval(yes)
                } else {
                    self.eval(no)
                }
            }
            Expr::Assign(op, target, value) => {
                let p = self.assign(*op, target, value);
                self.read(p)
            }
            Expr::Seq(list) => {
                let mut last = 0.0;
                for e in list {
                    last = self.eval(e);
                }
                last
            }
            Expr::Loop(count, body) => {
                // Truncated towards zero; the saturating cast handles huge counts.
                let n = (self.eval(count) as i64).min(MAX_LOOP);
                let mut last = 0.0;
                for _ in 0..n {
                    if self.budget == 0 {
                        break;
                    }
                    self.budget -= 1;
                    last = self.eval(body);
                }
                last
            }
            Expr::While(body) => {
                let mut left = MAX_LOOP;
                loop {
                    let v = self.eval(body);
                    left -= 1;
                    if !truthy(v) || left == 0 || self.budget == 0 {
                        return v;
                    }
                    self.budget -= 1;
                }
            }
            Expr::MemCpy(dest, src, count) => {
                let (d, s, n) = (self.eval(dest), self.eval(src), self.eval(count));
                self.mem_copy(d, s, n);
                d
            }
            Expr::MemSet(dest, value, count) => {
                let (d, v, n) = (self.eval(dest), self.eval(value), self.eval(count));
                if let (Some(start), Some(n)) = (mem_index(d), mem_index(n)) {
                    let end = (start + n).min(BLOCK * BLOCKS);
                    for i in start..end {
                        self.mem.set(i, v);
                    }
                }
                d
            }
        }
    }

    fn call1(&mut self, f: F1, a: f64) -> f64 {
        match f {
            F1::Sin => a.sin(),
            F1::Cos => a.cos(),
            F1::Tan => a.tan(),
            F1::Asin => {
                if (-1.0..=1.0).contains(&a) {
                    a.asin()
                } else {
                    0.0
                }
            }
            F1::Acos => {
                if (-1.0..=1.0).contains(&a) {
                    a.acos()
                } else {
                    0.0
                }
            }
            F1::Atan => a.atan(),
            F1::Sqr => a * a,
            F1::Sqrt => a.abs().sqrt(),
            F1::Exp => a.exp(),
            F1::Log => {
                if a > 0.0 {
                    a.ln()
                } else {
                    0.0
                }
            }
            F1::Log10 => {
                if a > 0.0 {
                    a.log10()
                } else {
                    0.0
                }
            }
            F1::Abs => a.abs(),
            F1::Sign => {
                if a > 0.0 {
                    1.0
                } else if a < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            }
            F1::Floor => a.floor(),
            F1::Ceil => a.ceil(),
            F1::InvSqrt => {
                // The fast inverse square root MilkDrop used, in double precision.
                let y = f64::from_bits(0x5FE6_EB50_C7B5_37A9 - (a.to_bits() >> 1));
                let r = y * (1.5 - 0.5 * a * y * y);
                if r.is_nan() { 0.0 } else { r }
            }
            F1::Not => flag(!truthy(a)),
            F1::Neg => -a,
            F1::Rand => {
                let top = a.floor().max(1.0);
                let r = self.globals.random() * top;
                match self.dialect {
                    Dialect::Eel1 => r.floor(),
                    Dialect::Eel2 => r,
                }
            }
        }
    }

    /// Carry out an assignment and say where its value went.
    fn assign(&mut self, op: Option<Op>, target: &Expr, value: &Expr) -> Place {
        let p = self.place(target);
        let v = self.eval(value);
        let new = match op {
            None => v,
            Some(op) => apply(op, self.read(p), v),
        };
        self.write(p, new)
    }

    /// Where `e` refers to, evaluating whatever it needs to find out.
    fn place(&mut self, e: &Expr) -> Place {
        match e {
            Expr::Var(s) => Place::Var(*s),
            Expr::Reg(r) => Place::Reg(*r),
            Expr::Mem(i) => {
                let i = self.eval(i);
                mem_index(i).map_or(Place::Value(0.0), Place::Mem)
            }
            Expr::GMem(i) => {
                let i = self.eval(i);
                mem_index(i).map_or(Place::Value(0.0), Place::GMem)
            }
            Expr::Assign(op, target, value) => self.assign(*op, target, value),
            Expr::If(c, yes, no) => {
                if truthy(self.eval(c)) {
                    self.place(yes)
                } else {
                    self.place(no)
                }
            }
            Expr::Seq(list) => match list.split_last() {
                Some((last, rest)) => {
                    for e in rest {
                        self.eval(e);
                    }
                    self.place(last)
                }
                None => Place::Value(0.0),
            },
            _ => Place::Value(self.eval(e)),
        }
    }

    fn read(&self, p: Place) -> f64 {
        match p {
            Place::Var(s) => self.vars.get(s.0).copied().unwrap_or(0.0),
            Place::Reg(r) => self.globals.regs[r],
            Place::Mem(i) => self.mem.get(i),
            Place::GMem(i) => self.globals.gmem.get(i),
            Place::Value(v) => v,
        }
    }

    /// Store `v` at `p`; a place that is only a value becomes `v`.
    fn write(&mut self, p: Place, v: f64) -> Place {
        match p {
            Place::Var(s) => {
                if let Some(x) = self.vars.get_mut(s.0) {
                    *x = v;
                }
            }
            Place::Reg(r) => self.globals.regs[r] = v,
            Place::Mem(i) => self.mem.set(i, v),
            Place::GMem(i) => self.globals.gmem.set(i, v),
            Place::Value(_) => return Place::Value(v),
        }
        p
    }

    /// memcpy: `n` entries of the local buffer from `s` to `d`, overlapping or not, trimmed to
    /// the buffer.
    fn mem_copy(&mut self, d: f64, s: f64, n: f64) {
        let (Some(d), Some(s), Some(n)) = (mem_index(d), mem_index(s), mem_index(n)) else {
            return;
        };
        let size = BLOCK * BLOCKS;
        let n = n.min(size - d).min(size - s);
        if d < s {
            for k in 0..n {
                let v = self.mem.get(s + k);
                self.mem.set(d + k, v);
            }
        } else {
            for k in (0..n).rev() {
                let v = self.mem.get(s + k);
                self.mem.set(d + k, v);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Run `src` with fresh variables and return its result and the variables.
    fn run(src: &str) -> (f64, Vars) {
        let mut vars = Vars::new();
        let p = Program::compile(src, &mut vars, Dialect::Eel2).unwrap();
        let mut g = Globals::new(7);
        let r = p.run(&mut vars, &mut g);
        (r, vars)
    }

    fn value(src: &str) -> f64 {
        run(src).0
    }

    fn var(vars: &Vars, name: &str) -> f64 {
        vars.get(vars.find(name).unwrap())
    }

    #[test]
    fn arithmetic_follows_milkdrops_precedence() {
        assert_eq!(value("1 + 2 * 3"), 7.0);
        assert_eq!(value("(1 + 2) * 3"), 9.0);
        assert_eq!(value("2 ^ 3 ^ 2"), 64.0); // ^ associates to the left
        assert_eq!(value("-2 ^ 2"), 4.0); // unary minus binds tighter than ^
        assert_eq!(value("!0 + 1"), 2.0);
        assert_eq!(value("!(0 ^ 1)"), 1.0);
        assert_eq!(value("2 * 7 % 4"), 6.0); // % binds tighter than *
        assert_eq!(value("1 < 2 == 1"), 1.0);
        assert_eq!(value("1 + 1 > 1"), 1.0);
        assert_eq!(value("3 & 1 | 4"), 5.0);
        assert_eq!(value("0 || 2 && 3"), 1.0);
        assert_eq!(value("1 ? 2 : 3"), 2.0);
        assert_eq!(value("0 ? 2 : 0 ? 3 : 4"), 4.0);
        assert!((value(".5 + 1. + 2e1 + 1.5E-1") - 21.65).abs() < 1e-12);
        assert_eq!(value("$PI"), std::f64::consts::PI);
        assert_eq!(value("$xFF + $'a'"), 255.0 + 97.0);
    }

    #[test]
    fn statements_assign_and_the_last_one_is_the_result() {
        let (r, vars) = run("x = 2; y = x * 3;; z = (a = 1; b = 2; a + b);");
        assert_eq!(r, 3.0);
        assert_eq!(var(&vars, "y"), 6.0);
        assert_eq!(var(&vars, "z"), 3.0);
        let (_, vars) = run("x = 10; x += 5; x -= 1; x *= 2; x /= 4; X %= 4");
        assert_eq!(var(&vars, "x"), 3.0);
        let (_, vars) = run("a = b = 4");
        assert_eq!((var(&vars, "a"), var(&vars, "b")), (4.0, 4.0));
    }

    #[test]
    fn names_are_case_insensitive_and_start_at_zero() {
        let (r, vars) = run("Zoom = ZOOM + 1; zoom");
        assert_eq!(r, 1.0);
        assert_eq!(var(&vars, "zOoM"), 1.0);
        assert_eq!(value("never_set"), 0.0);
    }

    #[test]
    fn comments_of_every_kind_are_skipped() {
        let (_, vars) = run("x = 1; // x = 2\ny = 3; \\\\ y = 4\nz = /* 5 */ 6;");
        assert_eq!(
            (var(&vars, "x"), var(&vars, "y"), var(&vars, "z")),
            (1.0, 3.0, 6.0)
        );
        let (_, vars) = run("a = 1;\r\nb = 2;\r\n");
        assert_eq!(var(&vars, "b"), 2.0);
    }

    #[test]
    fn functions_behave_as_in_milkdrop() {
        assert_eq!(value("if(1, 2, 3) + if(0, 2, 3)"), 5.0);
        assert_eq!(value("above(2, 1) + below(2, 1) + equal(1, 1.000001)"), 2.0);
        assert_eq!(value("int(-1.5) + int(1.5)"), -1.0);
        assert_eq!(value("sqr(3) + sqrt(-16) + abs(-1)"), 14.0);
        assert_eq!(value("sign(-3) + sign(0) + sign(2)"), 0.0);
        assert_eq!(value("min(1, 2) + max(1, 2)"), 3.0);
        assert_eq!(value("bnot(0) + bnot(5)"), 1.0);
        assert_eq!(value("band(1, 0) + bor(1, 0)"), 1.0);
        assert_eq!(value("1 / 0 + log(-1) + asin(2)"), 0.0);
        assert_eq!(value("pow(0, -1) + pow(-8, 0.5)"), 0.0);
        assert_eq!(value("-7 % 3"), 1.0); // MilkDrop takes sizes
        assert_eq!(value("7 % 0"), 0.0);
        assert_eq!(value("exec3(a = 1, b = 2, a + b)"), 3.0);
        assert_eq!(value("_aboeq(2, 2)"), 1.0);
        assert!((value("invsqrt(4)") - 0.5).abs() < 0.01);
        assert!((value("sigmoid(0, 1)") - 0.5).abs() < 1e-9);
        assert!((value("atan2(1, 1)") - std::f64::consts::FRAC_PI_4).abs() < 1e-12);
    }

    #[test]
    fn rand_stays_in_range_and_eel1_rounds_down() {
        let mut vars = Vars::new();
        let mut g = Globals::new(42);
        let p2 = Program::compile("rand(10)", &mut vars, Dialect::Eel2).unwrap();
        let p1 = Program::compile("rand(10)", &mut vars, Dialect::Eel1).unwrap();
        let mut fractions = false;
        for _ in 0..200 {
            let r = p2.run(&mut vars, &mut g);
            assert!((0.0..=10.0).contains(&r));
            fractions |= r.fract() != 0.0;
            let r = p1.run(&mut vars, &mut g);
            assert!((0.0..=10.0).contains(&r) && r.fract() == 0.0);
        }
        assert!(fractions);
        let p0 = Program::compile("rand(0.5)", &mut vars, Dialect::Eel2).unwrap();
        assert!(p0.run(&mut vars, &mut g) <= 1.0);
    }

    #[test]
    fn short_circuits_skip_the_other_side() {
        let (_, vars) = run("0 && (a = 1); 1 || (b = 1); 1 && (c = 1); band(0, d = 1)");
        assert_eq!(var(&vars, "a"), 0.0);
        assert_eq!(var(&vars, "b"), 0.0);
        assert_eq!(var(&vars, "c"), 1.0);
        assert_eq!(var(&vars, "d"), 1.0);
        let (_, vars) = run("if(0, a = 1, b = 2)");
        assert_eq!((var(&vars, "a"), var(&vars, "b")), (0.0, 2.0));
    }

    #[test]
    fn assignments_go_through_if_and_assign() {
        let (_, vars) = run("x = 1; if(x > 0, a, b) = 5; (x < 0 ? a : b) = 6");
        assert_eq!((var(&vars, "a"), var(&vars, "b")), (5.0, 6.0));
        let (r, vars) = run("assign(c, 7); assign(3, 4)");
        assert_eq!(var(&vars, "c"), 7.0);
        assert_eq!(r, 4.0);
        // An assignment's result can be assigned to again.
        let (_, vars) = run("(a = 1) += 2");
        assert_eq!(var(&vars, "a"), 3.0);
    }

    #[test]
    fn memory_buffers_index_round_and_copy() {
        let (r, vars) = run("megabuf(10) = 3; x = 10[]; 5[5] += 1; y = megabuf(9.99999999)");
        assert_eq!(r, 4.0);
        assert_eq!(var(&vars, "x"), 3.0);
        assert_eq!(var(&vars, "y"), 4.0);
        assert_eq!(value("megabuf(-2) = 5; megabuf(-2)"), 0.0);
        // MilkDrop's rounding puts -1 at entry 0.
        assert_eq!(value("megabuf(-1) = 5; megabuf(0)"), 5.0);
        assert_eq!(value("megabuf(8388608) = 5"), 5.0);
        assert_eq!(
            value("memset(100, 2, 10); megabuf(109) + megabuf(110)"),
            2.0
        );
        assert_eq!(
            value("loop(5, megabuf(i) = i; i += 1); memcpy(1, 0, 4); megabuf(4) + megabuf(1)"),
            3.0
        );
        assert_eq!(value("70000[] = 9; 70000[]"), 9.0);
    }

    #[test]
    fn registers_and_global_memory_are_shared_between_programs() {
        let mut g = Globals::new(1);
        let (mut a, mut b) = (Vars::new(), Vars::new());
        let set = Program::compile(
            "reg07 = 3; gmegabuf(5) = 4; gmem[6] = 5; megabuf(1) = 9",
            &mut a,
            Dialect::Eel2,
        )
        .unwrap();
        let get = Program::compile(
            "reg07 + gmem[5] + gmegabuf(6) + megabuf(1); reg7",
            &mut b,
            Dialect::Eel2,
        )
        .unwrap();
        set.run(&mut a, &mut g);
        let mut probe = Vars::new();
        let sum = Program::compile(
            "reg07 + gmem[5] + gmegabuf(6) + megabuf(1)",
            &mut probe,
            Dialect::Eel2,
        )
        .unwrap();
        assert_eq!(sum.run(&mut probe, &mut g), 12.0);
        // reg7 is an ordinary variable, not a register.
        assert_eq!(get.run(&mut b, &mut g), 0.0);
    }

    #[test]
    fn loops_run_and_stop() {
        assert_eq!(value("i = 0; loop(10, i += 1); i"), 10.0);
        assert_eq!(value("i = 0; while(i += 1; i < 7); i"), 7.0);
        assert_eq!(value("loop(-3, i += 1); i"), 0.0);
        // Nested loops that would take hours stop at the run's budget.
        let (_, vars) = run("loop(1000000, loop(1000000, n += 1))");
        assert!(var(&vars, "n") <= RUN_BUDGET as f64);
    }

    #[test]
    fn bad_code_is_refused_with_its_place() {
        let mut vars = Vars::new();
        let e = Program::compile("x = 1;\ny = foo(2);", &mut vars, Dialect::Eel2).unwrap_err();
        assert_eq!(e.at, 11);
        assert!(e.message.contains("foo"));
        assert!(Program::compile("sin(1, 2)", &mut vars, Dialect::Eel2).is_err());
        assert!(Program::compile("x = (1 + 2", &mut vars, Dialect::Eel2).is_err());
        assert!(Program::compile("x = 1 2", &mut vars, Dialect::Eel2).is_err());
        assert!(Program::compile("x = #", &mut vars, Dialect::Eel2).is_err());
        let deep = format!("{}1{}", "(".repeat(1000), ")".repeat(1000));
        assert!(Program::compile(&deep, &mut vars, Dialect::Eel2).is_err());
        let long = format!("x = 1{}", " + 1".repeat(1000));
        assert!(Program::compile(&long, &mut vars, Dialect::Eel2).is_err());
    }

    #[test]
    fn empty_code_does_nothing() {
        let mut vars = Vars::new();
        let p = Program::compile(" ; // nothing\n", &mut vars, Dialect::Eel2).unwrap();
        assert!(p.is_empty());
        assert_eq!(p.run(&mut vars, &mut Globals::new(0)), 0.0);
    }
}
