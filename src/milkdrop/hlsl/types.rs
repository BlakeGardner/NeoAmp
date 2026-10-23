//! HLSL's types as far as presets use them, and how each is written in WGSL.
//!
//! A matrix `floatRxC` (R rows of C) becomes WGSL's `matRxC<f32>`: R columns of C, so that
//! each WGSL column holds one HLSL row. Constructors then take their elements in the same
//! order, `m[i]` is row `i` in both, and `mul(a, b)` becomes `b * a`.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Scalar {
    Bool,
    Int,
    Uint,
    Float,
}

impl Scalar {
    pub(super) fn wgsl(self) -> &'static str {
        match self {
            Scalar::Bool => "bool",
            Scalar::Int => "i32",
            Scalar::Uint => "u32",
            Scalar::Float => "f32",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Type {
    Void,
    Scalar(Scalar),
    /// A vector of 2 to 4.
    Vector(Scalar, u8),
    /// Rows and columns, each 2 to 4 (always of `float`).
    Matrix(u8, u8),
    Sampler {
        volume: bool,
    },
    Array(Box<Type>, u32),
}

impl Type {
    pub(super) const FLOAT: Type = Type::Scalar(Scalar::Float);
    pub(super) const INT: Type = Type::Scalar(Scalar::Int);
    pub(super) const BOOL: Type = Type::Scalar(Scalar::Bool);

    pub(super) fn vec(s: Scalar, n: u8) -> Type {
        if n == 1 {
            Type::Scalar(s)
        } else {
            Type::Vector(s, n)
        }
    }

    /// The scalar type of a number, vector or matrix.
    pub(super) fn scalar(&self) -> Option<Scalar> {
        match self {
            Type::Scalar(s) | Type::Vector(s, _) => Some(*s),
            Type::Matrix(..) => Some(Scalar::Float),
            _ => None,
        }
    }

    /// How many numbers a number, vector or matrix holds.
    pub(super) fn components(&self) -> u32 {
        match self {
            Type::Scalar(_) => 1,
            Type::Vector(_, n) => u32::from(*n),
            Type::Matrix(r, c) => u32::from(*r) * u32::from(*c),
            Type::Array(t, n) => t.components() * n,
            _ => 0,
        }
    }

    /// The same shape with `s` numbers.
    pub(super) fn with_scalar(&self, s: Scalar) -> Type {
        match self {
            Type::Scalar(_) => Type::Scalar(s),
            Type::Vector(_, n) => Type::Vector(s, *n),
            other => other.clone(),
        }
    }

    pub(super) fn is_numeric(&self) -> bool {
        matches!(self, Type::Scalar(_) | Type::Vector(..) | Type::Matrix(..))
    }

    /// The type in WGSL.
    pub(super) fn wgsl(&self) -> String {
        match self {
            Type::Void => String::new(),
            Type::Scalar(s) => s.wgsl().to_owned(),
            Type::Vector(s, n) => format!("vec{n}<{}>", s.wgsl()),
            Type::Matrix(r, c) => format!("mat{r}x{c}<f32>"),
            Type::Sampler { .. } => "sampler".to_owned(),
            Type::Array(t, n) => format!("array<{}, {n}>", t.wgsl()),
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Void => f.write_str("void"),
            Type::Scalar(s) | Type::Vector(s, _) => {
                let base = match s {
                    Scalar::Bool => "bool",
                    Scalar::Int => "int",
                    Scalar::Uint => "uint",
                    Scalar::Float => "float",
                };
                match self {
                    Type::Vector(_, n) => write!(f, "{base}{n}"),
                    _ => f.write_str(base),
                }
            }
            Type::Matrix(r, c) => write!(f, "float{r}x{c}"),
            Type::Sampler { volume: false } => f.write_str("sampler2D"),
            Type::Sampler { volume: true } => f.write_str("sampler3D"),
            Type::Array(t, n) => write!(f, "{t}[{n}]"),
        }
    }
}

/// A float in WGSL's syntax, always with a point or an exponent, kept within `f32`.
pub(super) fn float_literal(v: f64) -> String {
    let v = if v.is_nan() {
        0.0
    } else {
        v.clamp(f64::from(f32::MIN), f64::from(f32::MAX))
    };
    let s = format!("{v:?}");
    if s.contains(['.', 'e', 'E']) {
        s
    } else {
        format!("{s}.0")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wgsl_names() {
        assert_eq!(Type::Vector(Scalar::Float, 3).wgsl(), "vec3<f32>");
        assert_eq!(Type::Matrix(4, 3).wgsl(), "mat4x3<f32>");
        let arr = Type::Array(Box::new(Type::Vector(Scalar::Float, 4)), 5);
        assert_eq!(arr.wgsl(), "array<vec4<f32>, 5>");
        assert_eq!(arr.to_string(), "float4[5]");
    }

    #[test]
    fn float_literals_are_valid_wgsl() {
        assert_eq!(float_literal(2.0), "2.0");
        assert_eq!(float_literal(0.5), "0.5");
        assert_eq!(float_literal(1e30), "1e30");
        assert_eq!(float_literal(-3.25), "-3.25");
        assert_eq!(float_literal(1e300), format!("{:?}", f64::from(f32::MAX)));
    }
}
