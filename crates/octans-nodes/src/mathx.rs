//! Math / logic / utility nodes, modeled on Blender's shader & compositor node set: a single
//! `Math` node with an operation dropdown (not thirty node types), boolean logic, comparisons,
//! value sources, clamp/map-range/mix, deterministic white noise, and type conversions.
//!
//! Data inputs are **parameter ports** (wireable, with defaults when unconnected); the operation
//! selectors are **config enums** rendered as dropdowns by the property panel
//! (`#[param(options = "…")]`).

use octans_core::{Catalog, Context, NodeRegistry};
use octans_macros::{node, NodeParams};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Math (f64)
// ---------------------------------------------------------------------------

/// Scalar math on `a`, `b` (and `c` for `compare`/`multiply_add`/`wrap`), like Blender's Math
/// node. Unknown ops yield `0.0`.
#[derive(Serialize, Deserialize, NodeParams)]
pub struct Math {
    /// The operation.
    #[param(
        options = "add,subtract,multiply,divide,power,log,sqrt,abs,exp,min,max,less_than,greater_than,sign,compare,round,floor,ceil,trunc,fract,modulo,wrap,snap,ping_pong,sin,cos,tan,asin,acos,atan,atan2,sinh,cosh,tanh,radians,degrees,multiply_add"
    )]
    pub op: String,
}

#[node(id = "octans.math.math", out = "value", serde, params)]
impl Math {
    fn process(
        &self,
        #[param(default = 0.0f64)] a: &f64,
        #[param(default = 0.0f64)] b: &f64,
        #[param(default = 0.0f64)] c: &f64,
    ) -> f64 {
        let (a, b, c) = (*a, *b, *c);
        match self.op.as_str() {
            "add" => a + b,
            "subtract" => a - b,
            "multiply" => a * b,
            "divide" => {
                if b != 0.0 {
                    a / b
                } else {
                    0.0
                }
            }
            "power" => a.powf(b),
            "log" => {
                if b > 0.0 && b != 1.0 {
                    a.log(b)
                } else {
                    a.ln()
                }
            }
            "sqrt" => a.max(0.0).sqrt(),
            "abs" => a.abs(),
            "exp" => a.exp(),
            "min" => a.min(b),
            "max" => a.max(b),
            "less_than" => (a < b) as u8 as f64,
            "greater_than" => (a > b) as u8 as f64,
            "sign" => {
                if a > 0.0 {
                    1.0
                } else if a < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            }
            "compare" => ((a - b).abs() <= c) as u8 as f64,
            "round" => a.round(),
            "floor" => a.floor(),
            "ceil" => a.ceil(),
            "trunc" => a.trunc(),
            "fract" => a - a.floor(),
            "modulo" => {
                if b != 0.0 {
                    a % b
                } else {
                    0.0
                }
            }
            // Wrap `a` into [c, b) (Blender's wrap(value, max, min)).
            "wrap" => {
                let range = b - c;
                if range != 0.0 {
                    a - range * ((a - c) / range).floor()
                } else {
                    0.0
                }
            }
            "snap" => {
                if b != 0.0 {
                    (a / b).floor() * b
                } else {
                    0.0
                }
            }
            "ping_pong" => {
                if b != 0.0 {
                    b - ((a.rem_euclid(2.0 * b)) - b).abs()
                } else {
                    0.0
                }
            }
            "sin" => a.sin(),
            "cos" => a.cos(),
            "tan" => a.tan(),
            "asin" => a.asin(),
            "acos" => a.acos(),
            "atan" => a.atan(),
            "atan2" => a.atan2(b),
            "sinh" => a.sinh(),
            "cosh" => a.cosh(),
            "tanh" => a.tanh(),
            "radians" => a.to_radians(),
            "degrees" => a.to_degrees(),
            "multiply_add" => a * b + c,
            _ => 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Boolean logic & comparison
// ---------------------------------------------------------------------------

/// Boolean logic on `a`, `b` (`not` uses only `a`), like Blender's Boolean Math.
#[derive(Serialize, Deserialize, NodeParams)]
pub struct BoolMath {
    /// The operation.
    #[param(options = "and,or,xor,nand,nor,xnor,not")]
    pub op: String,
}

#[node(id = "octans.logic.boolean", out = "value", serde, params)]
impl BoolMath {
    fn process(
        &self,
        #[param(default = false)] a: &bool,
        #[param(default = false)] b: &bool,
    ) -> bool {
        let (a, b) = (*a, *b);
        match self.op.as_str() {
            "and" => a && b,
            "or" => a || b,
            "xor" => a != b,
            "nand" => !(a && b),
            "nor" => !(a || b),
            "xnor" => a == b,
            "not" => !a,
            _ => false,
        }
    }
}

/// Compare two scalars → bool (`equal`/`not_equal` use `epsilon`).
#[derive(Serialize, Deserialize, NodeParams)]
pub struct Compare {
    /// The comparison.
    #[param(options = "less,less_equal,greater,greater_equal,equal,not_equal")]
    pub op: String,
    /// Tolerance for (in)equality.
    #[param(min = 0.0, max = 1000000.0)]
    pub epsilon: f64,
}

#[node(id = "octans.logic.compare", out = "value", serde, params)]
impl Compare {
    fn process(
        &self,
        #[param(default = 0.0f64)] a: &f64,
        #[param(default = 0.0f64)] b: &f64,
    ) -> bool {
        let (a, b) = (*a, *b);
        match self.op.as_str() {
            "less" => a < b,
            "less_equal" => a <= b,
            "greater" => a > b,
            "greater_equal" => a >= b,
            "equal" => (a - b).abs() <= self.epsilon,
            "not_equal" => (a - b).abs() > self.epsilon,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Value sources
// ---------------------------------------------------------------------------

/// A constant scalar (Blender's Value node).
#[derive(Serialize, Deserialize, NodeParams)]
pub struct FloatValue {
    /// The emitted value.
    pub value: f64,
}

#[node(id = "octans.math.value", out = "value", serde, params)]
impl FloatValue {
    fn process(&self) -> f64 {
        self.value
    }
}

/// A constant integer.
#[derive(Serialize, Deserialize, NodeParams)]
pub struct IntValue {
    /// The emitted value.
    pub value: i64,
}

#[node(id = "octans.math.integer", out = "value", serde, params)]
impl IntValue {
    fn process(&self) -> i64 {
        self.value
    }
}

/// A constant boolean.
#[derive(Serialize, Deserialize, NodeParams)]
pub struct BoolValue {
    /// The emitted value.
    pub value: bool,
}

#[node(id = "octans.logic.bool_value", out = "value", serde, params)]
impl BoolValue {
    fn process(&self) -> bool {
        self.value
    }
}

/// The current tick as a scalar — the scene-time source for animating anything.
#[derive(Serialize, Deserialize)]
pub struct Time;

#[node(id = "octans.math.time", out = "value", serde)]
impl Time {
    fn process(&self, #[ctx] ctx: &Context) -> f64 {
        ctx.tick() as f64
    }
}

/// Deterministic white noise in `[0, 1)`, re-rolled each tick (hash of tick + seed).
#[derive(Serialize, Deserialize, NodeParams)]
pub struct WhiteNoise {
    /// Seed — different seeds give independent streams.
    pub seed: u32,
}

#[node(id = "octans.math.white_noise", out = "value", serde, params)]
impl WhiteNoise {
    fn process(&self, #[ctx] ctx: &Context) -> f64 {
        let mut x = ctx
            .tick()
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(self.seed as u64);
        x ^= x >> 30;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^= x >> 31;
        (x >> 11) as f64 / (1u64 << 53) as f64
    }
}

// ---------------------------------------------------------------------------
// Range utilities
// ---------------------------------------------------------------------------

/// Clamp `value` into `[min, max]`.
#[derive(Serialize, Deserialize)]
pub struct Clamp;

#[node(id = "octans.math.clamp", out = "value", serde)]
impl Clamp {
    fn process(
        &self,
        #[param(default = 0.0f64)] value: &f64,
        #[param(default = 0.0f64)] min: &f64,
        #[param(default = 1.0f64)] max: &f64,
    ) -> f64 {
        value.clamp(*min, (*max).max(*min))
    }
}

/// Remap `value` from `[from_min, from_max]` to `[to_min, to_max]` (Blender's Map Range).
#[derive(Serialize, Deserialize, NodeParams)]
pub struct MapRange {
    /// Clamp the result to the target range.
    pub clamp: bool,
}

#[node(id = "octans.math.map_range", out = "value", serde, params)]
impl MapRange {
    fn process(
        &self,
        #[param(default = 0.0f64)] value: &f64,
        #[param(default = 0.0f64)] from_min: &f64,
        #[param(default = 1.0f64)] from_max: &f64,
        #[param(default = 0.0f64)] to_min: &f64,
        #[param(default = 1.0f64)] to_max: &f64,
    ) -> f64 {
        let span = from_max - from_min;
        let mut t = if span != 0.0 {
            (value - from_min) / span
        } else {
            0.0
        };
        if self.clamp {
            t = t.clamp(0.0, 1.0);
        }
        to_min + t * (to_max - to_min)
    }
}

/// Linear blend: `a·(1−fac) + b·fac`.
#[derive(Serialize, Deserialize)]
pub struct Mix;

#[node(id = "octans.math.mix", out = "value", serde)]
impl Mix {
    fn process(
        &self,
        #[param(default = 0.5f64)] fac: &f64,
        #[param(default = 0.0f64)] a: &f64,
        #[param(default = 0.0f64)] b: &f64,
    ) -> f64 {
        a * (1.0 - fac) + b * fac
    }
}

// ---------------------------------------------------------------------------
// Conversions (for wiring across port types, e.g. Compare → Switch.select)
// ---------------------------------------------------------------------------

/// `bool` → `u32` (`0`/`1`) — e.g. to drive a `Switch`'s `select`.
#[derive(Serialize, Deserialize)]
pub struct BoolToInt;

#[node(id = "octans.convert.bool_to_int", out = "value", serde)]
impl BoolToInt {
    fn process(&self, #[param(default = false)] value: &bool) -> u32 {
        *value as u32
    }
}

/// `f64` → `u32` (rounded, clamped at 0).
#[derive(Serialize, Deserialize)]
pub struct FloatToInt;

#[node(id = "octans.convert.float_to_int", out = "value", serde)]
impl FloatToInt {
    fn process(&self, #[param(default = 0.0f64)] value: &f64) -> u32 {
        value.round().clamp(0.0, u32::MAX as f64) as u32
    }
}

/// `u32` → `f64`.
#[derive(Serialize, Deserialize)]
pub struct IntToFloat;

#[node(id = "octans.convert.int_to_float", out = "value", serde)]
impl IntToFloat {
    fn process(&self, #[param(default = 0u32)] value: &u32) -> f64 {
        *value as f64
    }
}

/// `u8` → `f64` (raw 0–255).
#[derive(Serialize, Deserialize)]
pub struct ByteToFloat;

#[node(id = "octans.convert.byte_to_float", out = "value", serde)]
impl ByteToFloat {
    fn process(&self, #[param(default = 0u8)] value: &u8) -> f64 {
        *value as f64
    }
}

/// `f64` → `u8` (rounded, clamped to 0–255) — e.g. to drive a `Threshold`'s `thr`.
#[derive(Serialize, Deserialize)]
pub struct FloatToByte;

#[node(id = "octans.convert.float_to_byte", out = "value", serde)]
impl FloatToByte {
    fn process(&self, #[param(default = 0.0f64)] value: &f64) -> u8 {
        value.round().clamp(0.0, 255.0) as u8
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register_math_factories(reg: &mut NodeRegistry) {
    reg.register_serde::<Math>("octans.math.math");
    reg.register_serde::<BoolMath>("octans.logic.boolean");
    reg.register_serde::<Compare>("octans.logic.compare");
    reg.register_serde::<FloatValue>("octans.math.value");
    reg.register_serde::<IntValue>("octans.math.integer");
    reg.register_serde::<BoolValue>("octans.logic.bool_value");
    reg.register_serde::<Time>("octans.math.time");
    reg.register_serde::<WhiteNoise>("octans.math.white_noise");
    reg.register_serde::<Clamp>("octans.math.clamp");
    reg.register_serde::<MapRange>("octans.math.map_range");
    reg.register_serde::<Mix>("octans.math.mix");
    reg.register_serde::<BoolToInt>("octans.convert.bool_to_int");
    reg.register_serde::<FloatToInt>("octans.convert.float_to_int");
    reg.register_serde::<IntToFloat>("octans.convert.int_to_float");
    reg.register_serde::<ByteToFloat>("octans.convert.byte_to_float");
    reg.register_serde::<FloatToByte>("octans.convert.float_to_byte");
}

pub fn register_math_catalog(cat: &mut Catalog) {
    cat.add(|| Math { op: "add".into() });
    cat.add(|| BoolMath { op: "and".into() });
    cat.add(|| Compare {
        op: "less".into(),
        epsilon: 1e-9,
    });
    cat.add(|| FloatValue { value: 0.0 });
    cat.add(|| IntValue { value: 0 });
    cat.add(|| BoolValue { value: false });
    cat.add(|| Time);
    cat.add(|| WhiteNoise { seed: 0 });
    cat.add(|| Clamp);
    cat.add(|| MapRange { clamp: true });
    cat.add(|| Mix);
    cat.add(|| BoolToInt);
    cat.add(|| FloatToInt);
    cat.add(|| IntToFloat);
    cat.add(|| ByteToFloat);
    cat.add(|| FloatToByte);
}
