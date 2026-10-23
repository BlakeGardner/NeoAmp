//! The C preprocessor, as much of it as presets' shaders use: comments, `#define` (with or
//! without parameters), `#undef`, and `#if`, `#ifdef`, `#ifndef`, `#elif`, `#else`, `#endif`.
//! Each line of input stays one line of output, so errors can name the line.

use std::collections::{HashMap, HashSet};

use super::Error;

/// Macro expansions nested deeper than this are refused.
const MAX_DEPTH: usize = 64;
/// Output larger than this (in bytes) is refused, so macros cannot multiply without end.
const MAX_OUTPUT: usize = 4 << 20;
/// A line may expand to at most this many tokens.
const MAX_TOKENS: usize = 1 << 18;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    /// Anything else: a number, an operator character, a run of spaces.
    Other(String),
}

impl Tok {
    fn text(&self) -> &str {
        match self {
            Tok::Ident(s) | Tok::Other(s) => s,
        }
    }

    fn is_space(&self) -> bool {
        matches!(self, Tok::Other(s) if s.chars().all(char::is_whitespace))
    }
}

#[derive(Clone, Debug)]
struct Macro {
    params: Option<Vec<String>>,
    body: Vec<Tok>,
}

/// Remove comments, keeping every newline so lines still count the same.
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('/', Some('/')) => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                out.push(' ');
                let mut prev = ' ';
                for c in chars.by_ref() {
                    if prev == '*' && c == '/' {
                        break;
                    }
                    if c == '\n' {
                        out.push('\n');
                    }
                    prev = c;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn tokenize(line: &str) -> Vec<Tok> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let at = |k: usize| chars.get(k).map(|&(_, c)| c);
    let offset = |k: usize| chars.get(k).map_or(line.len(), |&(i, _)| i);
    let mut out = Vec::new();
    let mut k = 0;
    while let Some(c) = at(k) {
        let start = k;
        k += 1;
        let tok = if c.is_ascii_alphabetic() || c == '_' {
            while at(k).is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                k += 1;
            }
            Tok::Ident(line[offset(start)..offset(k)].to_owned())
        } else if c.is_ascii_digit() || (c == '.' && at(k).is_some_and(|d| d.is_ascii_digit())) {
            // A preprocessing number: digits, letters, dots, and signs after an exponent.
            while let Some(d) = at(k) {
                let prev = at(k - 1).unwrap_or(' ');
                let sign_after_e = matches!(d, '+' | '-') && matches!(prev, 'e' | 'E');
                if !(d.is_ascii_alphanumeric() || d == '.' || d == '_' || sign_after_e) {
                    break;
                }
                k += 1;
            }
            Tok::Other(line[offset(start)..offset(k)].to_owned())
        } else if c.is_whitespace() {
            while at(k).is_some_and(char::is_whitespace) {
                k += 1;
            }
            Tok::Other(line[offset(start)..offset(k)].to_owned())
        } else {
            Tok::Other(c.to_string())
        };
        out.push(tok);
    }
    out
}

/// One `#if` (or `#ifdef`, `#ifndef`) group being read.
#[derive(Debug)]
struct Cond {
    /// Whether the lines around the group are kept.
    outer: bool,
    /// Whether the branch being read is kept.
    active: bool,
    /// Whether a branch has been kept already (so later ones are not).
    taken: bool,
    seen_else: bool,
}

/// The preprocessor's state.
#[derive(Debug, Default)]
pub(super) struct Preprocessor {
    macros: HashMap<String, Macro>,
}

impl Preprocessor {
    /// Run `src` through the preprocessor. `first_line` numbers its first line in errors.
    pub(super) fn run(&mut self, src: &str, first_line: u32) -> Result<String, Error> {
        let src = strip_comments(src)
            .replace("\\\r\n", "")
            .replace("\\\n", "");
        let mut out = String::with_capacity(src.len());
        let mut conds: Vec<Cond> = Vec::new();
        // Ordinary lines are expanded together, so a macro's arguments can run over lines.
        let mut chunk = String::new();
        let mut chunk_line = first_line;
        for (n, line) in src.lines().enumerate() {
            let line_no = first_line + u32::try_from(n).unwrap_or(u32::MAX - first_line);
            let err = |message: String| Error {
                line: line_no,
                message,
            };
            let active = conds.last().is_none_or(|c| c.active);
            let trimmed = line.trim_start();
            let directive = trimmed.strip_prefix('#');
            if active && directive.is_none() {
                if chunk.is_empty() {
                    chunk_line = line_no;
                }
                chunk.push_str(line);
                chunk.push('\n');
                continue;
            }
            self.flush(&mut chunk, chunk_line, &mut out)?;
            if let Some(directive) = directive {
                let directive = directive.trim_start();
                let (name, rest) = directive
                    .split_once(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .map_or((directive, ""), |(a, _)| (a, &directive[a.len()..]));
                match name {
                    "if" | "ifdef" | "ifndef" => {
                        let value = active
                            && match name {
                                "if" => self.eval(rest).map_err(err)? != 0,
                                "ifdef" => self.macros.contains_key(rest.trim()),
                                _ => !self.macros.contains_key(rest.trim()),
                            };
                        conds.push(Cond {
                            outer: active,
                            active: value,
                            taken: value,
                            seen_else: false,
                        });
                    }
                    "elif" => {
                        let Some(c) = conds.last_mut() else {
                            return Err(err("#elif without #if".into()));
                        };
                        if c.seen_else {
                            return Err(err("#elif after #else".into()));
                        }
                        // Only evaluated when it could be taken, as C does.
                        let take = c.outer && !c.taken && self.eval(rest).map_err(err)? != 0;
                        c.active = take;
                        c.taken |= take;
                    }
                    "else" => {
                        let Some(c) = conds.last_mut() else {
                            return Err(err("#else without #if".into()));
                        };
                        if c.seen_else {
                            return Err(err("#else after #else".into()));
                        }
                        c.seen_else = true;
                        c.active = c.outer && !c.taken;
                        c.taken = true;
                    }
                    "endif" if conds.pop().is_none() => {
                        return Err(err("#endif without #if".into()));
                    }
                    "endif" => {}
                    _ if !active => {}
                    "define" => self.define(rest).map_err(err)?,
                    "undef" => {
                        self.macros.remove(rest.trim());
                    }
                    "error" => return Err(err(format!("#error{rest}"))),
                    "include" => return Err(err("#include is not supported".into())),
                    // #pragma, #line, #warning and the like change nothing here.
                    _ => {}
                }
            }
            out.push('\n');
        }
        self.flush(&mut chunk, chunk_line, &mut out)?;
        if !conds.is_empty() {
            return Err(Error {
                line: first_line,
                message: "#if without #endif".into(),
            });
        }
        Ok(out)
    }

    /// Expand a run of ordinary lines into `out`, keeping their number of lines.
    fn flush(&mut self, chunk: &mut String, line: u32, out: &mut String) -> Result<(), Error> {
        if chunk.is_empty() {
            return Ok(());
        }
        let tokens = tokenize(chunk);
        let expanded = self
            .expand(&tokens, &mut HashSet::new(), 0)
            .map_err(|message| Error { line, message })?;
        let start = out.len();
        for t in &expanded {
            out.push_str(t.text());
        }
        // Arguments a macro dropped may have held newlines: put them back at the end.
        let want = chunk.matches('\n').count();
        let have = out[start..].matches('\n').count();
        for _ in have..want {
            out.push('\n');
        }
        chunk.clear();
        if out.len() > MAX_OUTPUT {
            return Err(Error {
                line,
                message: "the preprocessed shader is too long".into(),
            });
        }
        Ok(())
    }

    fn define(&mut self, rest: &str) -> Result<(), String> {
        let rest = rest.trim_start();
        let name_end = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        let name = &rest[..name_end];
        if name.is_empty() {
            return Err("#define without a name".into());
        }
        let after = &rest[name_end..];
        // A parameter list only when "(" follows the name straight away.
        let (params, body) = if let Some(list) = after.strip_prefix('(') {
            let close = list
                .find(')')
                .ok_or("#define with an unclosed parameter list")?;
            let params = list[..close]
                .split(',')
                .map(|p| p.trim().to_owned())
                .filter(|p| !p.is_empty())
                .collect();
            (Some(params), &list[close + 1..])
        } else {
            (None, after)
        };
        let mut body = tokenize(body.trim());
        while body.last().is_some_and(Tok::is_space) {
            body.pop();
        }
        self.macros.insert(name.to_owned(), Macro { params, body });
        Ok(())
    }

    fn expand(
        &self,
        tokens: &[Tok],
        disabled: &mut HashSet<String>,
        depth: usize,
    ) -> Result<Vec<Tok>, String> {
        if depth > MAX_DEPTH {
            return Err("macros nested too deeply".into());
        }
        let mut out = Vec::with_capacity(tokens.len());
        let mut i = 0;
        while i < tokens.len() {
            let t = &tokens[i];
            i += 1;
            let Tok::Ident(name) = t else {
                out.push(t.clone());
                continue;
            };
            let Some(m) = self.macros.get(name).filter(|_| !disabled.contains(name)) else {
                out.push(t.clone());
                continue;
            };
            let call_start = i;
            let body = match &m.params {
                None => m.body.clone(),
                Some(params) => {
                    // Without an argument list straight after, the name is left alone.
                    let mut j = i;
                    while tokens.get(j).is_some_and(Tok::is_space) {
                        j += 1;
                    }
                    if tokens.get(j).map(Tok::text) != Some("(") {
                        out.push(t.clone());
                        continue;
                    }
                    let (args, next) = split_args(tokens, j + 1)?;
                    i = next;
                    if args.len() != params.len() && !(params.is_empty() && args.len() == 1) {
                        return Err(format!(
                            "macro '{name}' takes {} arguments, not {}",
                            params.len(),
                            args.len()
                        ));
                    }
                    let mut expanded_args = Vec::with_capacity(args.len());
                    for a in &args {
                        expanded_args.push(self.expand(a, disabled, depth + 1)?);
                    }
                    let mut body = Vec::new();
                    for tok in &m.body {
                        match tok {
                            Tok::Ident(p) => match params.iter().position(|q| q == p) {
                                Some(k) => body.extend(expanded_args[k].iter().cloned()),
                                None => body.push(tok.clone()),
                            },
                            other => body.push(other.clone()),
                        }
                    }
                    body
                }
            };
            disabled.insert(name.clone());
            let result = self.expand(&body, disabled, depth + 1);
            disabled.remove(name);
            let result = result?;
            // Newlines inside the call that the expansion dropped go after it, so the lines
            // that follow keep their numbers.
            let lines = |toks: &[Tok]| -> usize {
                toks.iter().map(|t| t.text().matches('\n').count()).sum()
            };
            let missing = lines(&tokens[call_start..i]).saturating_sub(lines(&result));
            out.extend(result);
            out.extend(std::iter::repeat_n(Tok::Other("\n".into()), missing));
            if out.len() > MAX_TOKENS {
                return Err("macros expand too far".into());
            }
        }
        Ok(out)
    }

    /// Evaluate an `#if` condition.
    fn eval(&self, expr: &str) -> Result<i64, String> {
        // `defined X` and `defined(X)` first, before any expansion.
        let tokens = tokenize(expr);
        let mut resolved = Vec::with_capacity(tokens.len());
        let mut i = 0;
        while i < tokens.len() {
            if tokens[i].text() == "defined" {
                let mut j = i + 1;
                let skip = |j: &mut usize| {
                    while tokens.get(*j).is_some_and(Tok::is_space) {
                        *j += 1;
                    }
                };
                skip(&mut j);
                let paren = tokens.get(j).map(Tok::text) == Some("(");
                if paren {
                    j += 1;
                    skip(&mut j);
                }
                let name = tokens
                    .get(j)
                    .map(Tok::text)
                    .ok_or("defined without a name")?;
                let value = if self.macros.contains_key(name) {
                    "1"
                } else {
                    "0"
                };
                j += 1;
                if paren {
                    skip(&mut j);
                    if tokens.get(j).map(Tok::text) != Some(")") {
                        return Err("unclosed defined(".into());
                    }
                    j += 1;
                }
                resolved.push(Tok::Other(value.into()));
                i = j;
            } else {
                resolved.push(tokens[i].clone());
                i += 1;
            }
        }
        let expanded = self.expand(&resolved, &mut HashSet::new(), 0)?;
        let mut words: Vec<&str> = expanded
            .iter()
            .filter(|t| !t.is_space())
            .map(Tok::text)
            .collect();
        // Two-character operators arrive as single characters; join them.
        let mut joined: Vec<String> = Vec::with_capacity(words.len());
        words.reverse();
        while let Some(w) = words.pop() {
            if let Some(&next) = words.last() {
                let pair = format!("{w}{next}");
                if ["&&", "||", "==", "!=", "<=", ">=", "<<", ">>"].contains(&pair.as_str()) {
                    words.pop();
                    joined.push(pair);
                    continue;
                }
            }
            joined.push(w.to_owned());
        }
        let mut p = CondParser {
            toks: &joined,
            pos: 0,
        };
        let v = p.ternary()?;
        if p.pos != joined.len() {
            return Err(format!("unexpected '{}' in #if", joined[p.pos]));
        }
        Ok(v)
    }
}

/// The arguments of a macro call starting after its "(": each a list of tokens, split at
/// commas outside brackets. Returns them and the position after the ")".
fn split_args(tokens: &[Tok], mut i: usize) -> Result<(Vec<Vec<Tok>>, usize), String> {
    let mut args = vec![Vec::new()];
    let mut depth = 0;
    while let Some(t) = tokens.get(i) {
        i += 1;
        match t.text() {
            "(" => depth += 1,
            ")" if depth == 0 => return Ok((args, i)),
            ")" => depth -= 1,
            "," if depth == 0 => {
                args.push(Vec::new());
                continue;
            }
            _ => {}
        }
        if let Some(a) = args.last_mut() {
            a.push(t.clone());
        }
    }
    Err("unclosed macro arguments".into())
}

/// Evaluates `#if` expressions: integers and C's operators. Unknown names are 0.
struct CondParser<'a> {
    toks: &'a [String],
    pos: usize,
}

impl CondParser<'_> {
    fn peek(&self) -> Option<&str> {
        self.toks.get(self.pos).map(String::as_str)
    }

    fn eat(&mut self, s: &str) -> bool {
        if self.peek() == Some(s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn ternary(&mut self) -> Result<i64, String> {
        let c = self.binary(0)?;
        if self.eat("?") {
            let a = self.ternary()?;
            if !self.eat(":") {
                return Err("expected ':' in #if".into());
            }
            let b = self.ternary()?;
            return Ok(if c != 0 { a } else { b });
        }
        Ok(c)
    }

    fn binary(&mut self, min: u8) -> Result<i64, String> {
        let mut lhs = self.unary()?;
        while let Some(op) = self.peek() {
            let prec = match op {
                "||" => 1,
                "&&" => 2,
                "|" => 3,
                "^" => 4,
                "&" => 5,
                "==" | "!=" => 6,
                "<" | ">" | "<=" | ">=" => 7,
                "<<" | ">>" => 8,
                "+" | "-" => 9,
                "*" | "/" | "%" => 10,
                _ => break,
            };
            if prec < min {
                break;
            }
            let op = op.to_owned();
            self.pos += 1;
            let rhs = self.binary(prec + 1)?;
            lhs = match op.as_str() {
                "||" => i64::from(lhs != 0 || rhs != 0),
                "&&" => i64::from(lhs != 0 && rhs != 0),
                "|" => lhs | rhs,
                "^" => lhs ^ rhs,
                "&" => lhs & rhs,
                "==" => i64::from(lhs == rhs),
                "!=" => i64::from(lhs != rhs),
                "<" => i64::from(lhs < rhs),
                ">" => i64::from(lhs > rhs),
                "<=" => i64::from(lhs <= rhs),
                ">=" => i64::from(lhs >= rhs),
                "<<" => lhs.wrapping_shl(u32::try_from(rhs).unwrap_or(0)),
                ">>" => lhs.wrapping_shr(u32::try_from(rhs).unwrap_or(0)),
                "+" => lhs.wrapping_add(rhs),
                "-" => lhs.wrapping_sub(rhs),
                "*" => lhs.wrapping_mul(rhs),
                "/" => lhs.checked_div(rhs).ok_or("division by zero in #if")?,
                _ => lhs.checked_rem(rhs).ok_or("division by zero in #if")?,
            };
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<i64, String> {
        if self.eat("!") {
            return Ok(i64::from(self.unary()? == 0));
        }
        if self.eat("-") {
            return Ok(self.unary()?.wrapping_neg());
        }
        if self.eat("+") {
            return self.unary();
        }
        if self.eat("~") {
            return Ok(!self.unary()?);
        }
        if self.eat("(") {
            let v = self.ternary()?;
            if !self.eat(")") {
                return Err("unclosed '(' in #if".into());
            }
            return Ok(v);
        }
        let Some(t) = self.peek().map(str::to_owned) else {
            return Err("unexpected end of #if".into());
        };
        self.pos += 1;
        let t = t.as_str();
        if t.starts_with(|c: char| c.is_ascii_digit()) {
            let digits = t.trim_end_matches(['u', 'U', 'l', 'L']);
            let v = if let Some(hex) = digits
                .strip_prefix("0x")
                .or_else(|| digits.strip_prefix("0X"))
            {
                i64::from_str_radix(hex, 16)
            } else {
                digits.parse()
            };
            // A float in #if (as in "#if 1.0") counts by its integer part.
            v.or_else(|_| digits.parse::<f64>().map(|f| f as i64))
                .map_err(|_| format!("bad number '{t}' in #if"))
        } else if t.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
            Ok(0)
        } else {
            Err(format!("unexpected '{t}' in #if"))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn pp(src: &str) -> String {
        Preprocessor::default().run(src, 1).unwrap()
    }

    #[test]
    fn comments_go_and_lines_stay() {
        // A block comment leaves a space, and its newlines.
        assert_eq!(pp("a // b\nc /* d\ne */ f\n"), "a \nc  \n f\n");
    }

    #[test]
    fn object_and_function_macros_expand() {
        let out = pp("#define N 3\n#define SQ(x) ((x)*(x))\nfloat a = SQ(N + 1);\nSQ;\n");
        assert_eq!(out, "\n\nfloat a = ((3 + 1)*(3 + 1));\nSQ;\n");
        // Nested calls and commas inside brackets.
        let out = pp("#define F(a, b) a+b\nF(g(1, 2), (3, 4))\n");
        assert_eq!(out.trim(), "g(1, 2)+ (3, 4)");
        // A macro that names itself stops.
        assert_eq!(pp("#define X X+1\nX\n").trim(), "X+1");
    }

    #[test]
    fn macro_arguments_can_run_over_lines() {
        let out = pp("#define L(x) (x)\na = L(b\n - c);\nd\n");
        assert_eq!(out, "\na = (b\n - c);\nd\n");
        // A dropped argument's newline is kept, so later lines keep their numbers.
        let out = pp("#define K(x) 1\na = K(b\n);\nd\n");
        assert_eq!(out.lines().count(), 4);
        assert_eq!(out.lines().nth(3), Some("d"));
    }

    #[test]
    fn conditionals_choose_lines() {
        let src = "#if 0\na\n#elif defined(B)\nb\n#else\nc\n#endif\n#if 1\nd\n#endif\n";
        assert_eq!(pp(src), "\n\n\n\n\nc\n\n\nd\n\n");
        let src = "#define B 2\n#if B > 1 && !defined(C)\nyes\n#else\nno\n#endif\n";
        assert_eq!(pp(src).trim(), "yes");
        let src = "#ifdef B\nb\n#endif\n#ifndef B\nnot b\n#endif\n";
        assert_eq!(pp(src).trim(), "not b");
        // Directives inside a skipped branch do nothing.
        let src = "#if 0\n#define Z 1\n#endif\nZ\n";
        assert_eq!(pp(src).trim(), "Z");
    }

    #[test]
    fn numbers_are_not_names() {
        assert_eq!(pp("#define e 9\nx = 1e5 + e;\n").trim(), "x = 1e5 + 9;");
        assert_eq!(pp("#define f 9\nx = 1.0f;\n").trim(), "x = 1.0f;");
    }

    #[test]
    fn bad_input_is_reported_with_its_line() {
        let e = Preprocessor::default()
            .run("a\n#if\nb\n#endif\n", 10)
            .unwrap_err();
        assert_eq!(e.line, 11);
        assert!(Preprocessor::default().run("#if 1\n", 1).is_err());
        assert!(Preprocessor::default().run("#endif\n", 1).is_err());
        assert!(Preprocessor::default().run("#include \"x\"\n", 1).is_err());
        let bomb = "#define A x x x x x x x x\n#define B A A A A A A A A\n#define C B B B B B B B B\n\
                    #define D C C C C C C C C\n#define E D D D D D D D D\n#define F E E E E E E E E\n\
                    #define G F F F F F F F F\n#define H G G G G G G G G\nH\n";
        assert!(Preprocessor::default().run(bomb, 1).is_err());
    }
}
