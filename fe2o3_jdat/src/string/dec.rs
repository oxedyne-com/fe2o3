//! 12024-12-23
//! Added implicit tuple string decoding for round brackets, e.g. explicit (tup2|[1,2]), implicit
//! (1,2).  This has no effect on tuple string encoding, which continues to use square brackets
//! either with a kindicle or without, e.g. explicit (tup2[1,2]) implicit [1,2].  The latter will
//! continue to be decoded as a list.
//!
use crate::{
    prelude::*,
    bdat::limits::DecodeLimits,
    note::NoteConfig,
    int::{
        DatInt,
        DatIntKind,
    },
    kind::{
        KindCase,
        KindClass,
    },
    usr::{
        UsrKind,
        UsrKinds,
        UsrKindId,
        UsrKindCode,
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    map::MapMut,
    mem::Extract,
};
use oxedyne_fe2o3_num::{
    float::{
        Float32,
        Float64,
    },
    string::NumberString,
};
use oxedyne_fe2o3_text::{
    base2x,
    string::Quote,
};

use std::{
    cell::RefCell,
    collections::BTreeMap,
    convert::TryInto,
    fmt,
    str::FromStr,
};


impl FromStr for Dat {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Dat::decode_string(s)
    }
}

#[derive(Clone, Debug)]
pub struct DecoderConfig<
    M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
    M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
>{
    pub quote_protection:       bool,
    pub comment_allowed:        bool,
    pub comment_capture:        bool,
    pub comment1_start_char:    char,
    pub comment1_end_char:      char,
    pub comment2_start_char:    char,
    pub comment2_end_char:      char,
    pub default_key:            DatIntKind,
    pub trailing_comma_allowed: bool,
    pub use_ordmaps:            bool,
    pub ukinds_opt:             Option<UsrKinds<M1, M2>>,
    pub omap_start:             u64,
    pub omap_delta:             u64,
    /// Bounds on the length and the nesting depth of the text the decoder will accept.
    pub limits:                 DecodeLimits,
}

impl<
    M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
    M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
>
    Default for DecoderConfig<M1, M2>
{
    fn default() -> Self {
        Self {
            quote_protection:       true,
            comment_allowed:        true,
            comment_capture:        true,
            comment1_start_char:    '!',
            comment1_end_char:      '!',
            comment2_start_char:    '#',
            comment2_end_char:      '#',
            default_key:            DatIntKind::U32,
            trailing_comma_allowed: true,
            use_ordmaps:            false,
            ukinds_opt:             None,
            omap_start:             Dat::OMAP_ORDER_START_DEFAULT,
            omap_delta:             Dat::OMAP_ORDER_DELTA_DEFAULT,
            limits:                 DecodeLimits::text(),
        }
    }
}

impl<
    M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
    M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
>
    DecoderConfig<M1, M2>
{
    pub fn jdat(ukinds_opt: Option<UsrKinds<M1, M2>>) -> Self {
        Self {
            ukinds_opt,
            ..Default::default()
        }
    }

    pub fn json(ukinds_opt: Option<UsrKinds<M1, M2>>) -> Self {
        Self {
            comment_allowed:        false,
            trailing_comma_allowed: false,
            ukinds_opt,
            ..Default::default()
        }
    }

    /// Returns the configuration with the decoding limits replaced.
    pub fn with_limits(mut self, limits: DecodeLimits) -> Self {
        self.limits = limits;
        self
    }
}

#[derive(Clone, Debug)]
pub struct Cursor {
    abs:    usize,  // Stream position.
    curr:   char,   // Current character.
    prev:   char,   // Previous character.
    line:   usize,  // Line position.
    x:      usize,  // Character position on line.
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            abs:    0,
            curr:   char::default(),
            prev:   char::default(),
            line:   1,
            x:      0,
        }
    }
}

impl fmt::Display for Cursor {
    fn fmt (&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "char '{}' line {} pos {}", self.curr, self.line, self.x)
    }
}

impl Cursor {
    pub fn advance(
        &mut self,
        c:                  char,
        quote_protection:   bool,
    ) {
        if self.abs > 0 {
            let p = self.curr;
            self.curr = c;
            self.prev = p;
        } else {
            self.curr = c
        }
        self.abs += 1;
        if c == '\n' || (quote_protection && c == '\\') {
            self.line += 1;
            self.x = 0;
        } else {
            if !c.is_control() {
                self.x += 1;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CommentCapture {
    Type1,
    Type2,
}

/// Per-character state machine for decoding RFC 8259 §7 string
/// escapes inside a quoted string literal.
///
/// * `None`          -- normal: characters are pushed verbatim
///                      into the slurp, except `\` which
///                      transitions into `Backslash`.
/// * `Backslash`     -- the previous character was a backslash
///                      and the current character is the escape
///                      type: `"`, `\`, `/`, `b`, `f`, `n`, `r`,
///                      `t`, or `u`. The first eight translate
///                      to a single character; `u` transitions
///                      into `Unicode`.
/// * `Unicode`       -- collecting four hex digits for a
///                      `\uXXXX` code point. On completion the
///                      code point is pushed if it is in the BMP
///                      and not a surrogate half, the high half
///                      of a surrogate pair transitions into
///                      `SurrogateBackslash`, and a bare low
///                      surrogate is a decode error.
/// * `SurrogateBackslash` -- after a high surrogate; expecting
///                           `\` to start the low half.
/// * `SurrogateU`    -- after `SurrogateBackslash`; expecting `u`.
/// * `LowSurrogate`  -- collecting four hex digits for the low
///                      half; on completion the pair is combined
///                      into a supplementary plane code point
///                      and pushed.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum StringEscape {
    #[default]
    None,
    Backslash,
    Unicode             { digits: u8, acc: u32 },
    SurrogateBackslash  { high: u16 },
    SurrogateU          { high: u16 },
    LowSurrogate        { digits: u8, acc: u32, high: u16 },
}

#[derive(Clone, Debug)]
pub struct DecoderState {
    // Data
    //pub cursor:             Cursor,
    pub kind_outer:         Kind,
    /// Nesting depth of the value being decoded, the root value sitting at depth 1.
    pub depth:              usize,
    // Switches
    pub explicit_kind:      bool, // The kind was defined explicitly.
    pub quote_protection:   Quote, // "a(' &{}" etc
    pub comment_capture:    Option<CommentCapture>,
    pub number_capture:     bool, // 23_786.345 contiguous
    pub kind_capture:       bool, // (k|
    pub atomic_capture:     bool,
    pub molecular_capture:  Option<MolecularCapture>,
    pub string_escape:      StringEscape, // Inside quoted strings only.
}

impl Default for DecoderState {
    fn default() -> Self {
        Self {
            kind_outer:         Kind::default(),
            depth:              1, // The state a decode starts from is the root value.
            explicit_kind:      false,
            quote_protection:   Quote::default(),
            comment_capture:    None,
            number_capture:     false,
            kind_capture:       false,
            atomic_capture:     false,
            molecular_capture:  None,
            string_escape:      StringEscape::default(),
        }
    }
}

impl fmt::Display for DecoderState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<quo:{:?}|com:{:?}|num:{}|knd:{}|atm:{}|mol:{:?}>",
            self.quote_protection,
            self.comment_capture,
            self.number_capture as u8,
            self.kind_capture as u8,
            self.atomic_capture as u8,
            self.molecular_capture,
        )
    }
}

impl DecoderState {
    /// Returns the state for the next level down, one deeper than the present one.
    pub fn recurse(&self) -> Self {
        Self {
            kind_outer: self.kind_outer.clone(),
            depth:      self.depth + 1,
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DecoderStore {
    pub val_opt:    Option<Dat>,
    pub key_opt:    Option<Dat>,
    pub list:       Vec<Dat>,
    pub comment:    String,
    pub note_config:    NoteConfig,
    pub byts:       Vec<u8>,
    pub map:        DaticleMap,
    pub ordmap:     OrdDaticleMap,
    pub slurp:      Slurp,
    pub map_count:  MapCounter,
}

impl DecoderStore {
    pub fn new<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        cfg: &DecoderConfig<M1, M2>,
    )
        -> Self
    {
        Self {
            val_opt:    None,
            key_opt:    None,
            list:       Vec::new(),
            comment:    String::new(),
            note_config:    NoteConfig::default(),
            byts:       Vec::new(),
            map:        DaticleMap::new(),
            ordmap:     OrdDaticleMap::new(),
            slurp:      Slurp::new(),
            map_count:  MapCounter {
                count: 0,
                order: cfg.omap_start,
                delta: cfg.omap_delta,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MolecularCapture {
    Bytes,
    ListMixed,
    ListSame,
    Map,
}

/// What the decoder does with the daticle returned by the level below.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Descent {
    /// The level was opened by a kindicle, e.g. `(u8|42)`, and may complete this one.
    Kindicle,
    /// The level was opened by a `[` or a `{`, so it is one item of the molecule being gathered.
    Molecule,
}

/// What reading one character asks of the level that character belongs to.
///
/// The descent is asked for rather than taken, so that the locals of the character-reading code are
/// off the stack before the decoder enters the level below.
#[derive(Debug)]
enum Step {
    /// Read the next character at this level.
    Continue,
    /// The daticle at this level is complete.
    Done(Dat),
    /// Open a level below this one, decoding it under the given state.
    Descend(DecoderState, Descent),
}

impl MolecularCapture {
    #[inline(never)]
    pub fn from_kind(kind: &Kind) -> Option<Self> {
        match kind {
            Kind::List      |
            Kind::Tup2      |
            Kind::Tup3      |
            Kind::Tup4      |
            Kind::Tup5      |
            Kind::Tup6      |
            Kind::Tup7      |
            Kind::Tup8      |
            Kind::Tup9      |
            Kind::Tup10     => Some(MolecularCapture::ListMixed),
            Kind::OrdMap    |
            Kind::Map       => Some(MolecularCapture::Map),
            Kind::BU8       |
            Kind::BU16      |
            Kind::BU32      |
            Kind::BU64      |
            Kind::BC64      |
            Kind::B2        |
            Kind::B3        |
            Kind::B4        |
            Kind::B5        |
            Kind::B6        |
            Kind::B7        |
            Kind::B8        |
            Kind::B9        |
            Kind::B10       |
            Kind::B16       |
            Kind::B32       => Some(MolecularCapture::Bytes),
            Kind::Vek       |
            Kind::Tup2u8    |
            Kind::Tup3u8    |
            Kind::Tup4u8    |
            Kind::Tup5u8    |
            Kind::Tup6u8    |
            Kind::Tup7u8    |
            Kind::Tup8u8    |
            Kind::Tup9u8    |
            Kind::Tup10u8   |
            Kind::Tup2u16   |
            Kind::Tup3u16   |
            Kind::Tup4u16   |
            Kind::Tup5u16   |
            Kind::Tup6u16   |
            Kind::Tup7u16   |
            Kind::Tup8u16   |
            Kind::Tup9u16   |
            Kind::Tup10u16  |
            Kind::Tup2u32   |
            Kind::Tup3u32   |
            Kind::Tup4u32   |
            Kind::Tup5u32   |
            Kind::Tup6u32   |
            Kind::Tup7u32   |
            Kind::Tup8u32   |
            Kind::Tup9u32   |
            Kind::Tup10u32  |
            Kind::Tup2u64   |
            Kind::Tup3u64   |
            Kind::Tup4u64   |
            Kind::Tup5u64   |
            Kind::Tup6u64   |
            Kind::Tup7u64   |
            Kind::Tup8u64   |
            Kind::Tup9u64   |
            Kind::Tup10u64  |
            Kind::Tup2i8    |
            Kind::Tup3i8    |
            Kind::Tup4i8    |
            Kind::Tup5i8    |
            Kind::Tup6i8    |
            Kind::Tup7i8    |
            Kind::Tup8i8    |
            Kind::Tup9i8    |
            Kind::Tup10i8   |
            Kind::Tup2i16   |
            Kind::Tup3i16   |
            Kind::Tup4i16   |
            Kind::Tup5i16   |
            Kind::Tup6i16   |
            Kind::Tup7i16   |
            Kind::Tup8i16   |
            Kind::Tup9i16   |
            Kind::Tup10i16  |
            Kind::Tup2i32   |
            Kind::Tup3i32   |
            Kind::Tup4i32   |
            Kind::Tup5i32   |
            Kind::Tup6i32   |
            Kind::Tup7i32   |
            Kind::Tup8i32   |
            Kind::Tup9i32   |
            Kind::Tup10i32  |
            Kind::Tup2i64   |
            Kind::Tup3i64   |
            Kind::Tup4i64   |
            Kind::Tup5i64   |
            Kind::Tup6i64   |
            Kind::Tup7i64   |
            Kind::Tup8i64   |
            Kind::Tup9i64   |
            Kind::Tup10i64  => Some(MolecularCapture::ListSame),
            _ => None,
        }
    }

    #[inline(never)]
    pub fn same_kind(kind: &Kind) -> Kind {
        match kind {
            Kind::Tup2u8    |
            Kind::Tup3u8    |
            Kind::Tup4u8    |
            Kind::Tup5u8    |
            Kind::Tup6u8    |
            Kind::Tup7u8    |
            Kind::Tup8u8    |
            Kind::Tup9u8    |
            Kind::Tup10u8   => Kind::U8,
            Kind::Tup2u16   |
            Kind::Tup3u16   |
            Kind::Tup4u16   |
            Kind::Tup5u16   |
            Kind::Tup6u16   |
            Kind::Tup7u16   |
            Kind::Tup8u16   |
            Kind::Tup9u16   |
            Kind::Tup10u16  => Kind::U16,
            Kind::Tup2u32   |
            Kind::Tup3u32   |
            Kind::Tup4u32   |
            Kind::Tup5u32   |
            Kind::Tup6u32   |
            Kind::Tup7u32   |
            Kind::Tup8u32   |
            Kind::Tup9u32   |
            Kind::Tup10u32  => Kind::U32,
            Kind::Tup2u64   |
            Kind::Tup3u64   |
            Kind::Tup4u64   |
            Kind::Tup5u64   |
            Kind::Tup6u64   |
            Kind::Tup7u64   |
            Kind::Tup8u64   |
            Kind::Tup9u64   |
            Kind::Tup10u64  => Kind::U64,
            Kind::Tup2i8    |
            Kind::Tup3i8    |
            Kind::Tup4i8    |
            Kind::Tup5i8    |
            Kind::Tup6i8    |
            Kind::Tup7i8    |
            Kind::Tup8i8    |
            Kind::Tup9i8    |
            Kind::Tup10i8   => Kind::I8,
            Kind::Tup2i16   |
            Kind::Tup3i16   |
            Kind::Tup4i16   |
            Kind::Tup5i16   |
            Kind::Tup6i16   |
            Kind::Tup7i16   |
            Kind::Tup8i16   |
            Kind::Tup9i16   |
            Kind::Tup10i16  => Kind::I16,
            Kind::Tup2i32   |
            Kind::Tup3i32   |
            Kind::Tup4i32   |
            Kind::Tup5i32   |
            Kind::Tup6i32   |
            Kind::Tup7i32   |
            Kind::Tup8i32   |
            Kind::Tup9i32   |
            Kind::Tup10i32  => Kind::I32,
            Kind::Tup2i64   |
            Kind::Tup3i64   |
            Kind::Tup4i64   |
            Kind::Tup5i64   |
            Kind::Tup6i64   |
            Kind::Tup7i64   |
            Kind::Tup8i64   |
            Kind::Tup9i64   |
            Kind::Tup10i64  => Kind::I64,
            _ => Kind::Unknown,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Slurp {
    s:          String,
    is_string:  bool,
}

impl Slurp {
    fn new() -> Self {
        Slurp {
            s:          String::new(),
            is_string:  false,
        }
    }

    fn reset(&mut self) {
        self.s = String::new();
        self.is_string = false;
    }

    fn push(&mut self, c: char) {
        self.s.push(c);
    }

    fn flag_as_string(&mut self) {
        self.is_string = true;
    }

    fn is_string(&self) -> bool {
        self.is_string
    }

    fn get_str(&self) -> &str {
        &self.s
    }

    fn clone_string(&self) -> String {
        self.s.clone()
    }

    fn take_string(&mut self) -> String {
        std::mem::replace(&mut self.s, String::new())
    }

    fn len(&self) -> usize {
        self.s.len()
    }

    /// Whether this slurp has captured anything at all. An empty
    /// quoted string (e.g. `""`) has zero characters but is still
    /// content -- it decodes to `Dat::Str("")` -- so `len()` alone
    /// is not enough to decide whether a pending value is waiting
    /// to be processed at a `,`, `}` or `]` boundary.
    fn has_content(&self) -> bool {
        self.is_string || !self.s.is_empty()
    }

    #[inline(never)]
    fn char_slurped(
        &mut self,
        (i, c):  (usize, char),
        state:  &mut DecoderState,
    )
        -> Outcome<bool>
    {
        if state.quote_protection != Quote::None { 
            self.push(c);
            return Ok(true)
        }
        if state.kind_capture {
            match c {
                'A' ..= 'Z' |
                'a' ..= 'z' |
                '0' ..= '9' |
                '/'         |
                '.'         |
                '_'         =>
                {
                    self.push(c);
                    return Ok(true);
                }
                ' '     |
                '\n'    => {}
                ',' => {
                    // Allow for the possibility that we have a tuple without a kindicle.
                    // e.g. (1,2,..)
                    state.kind_capture = false;
                    state.molecular_capture = Some(MolecularCapture::ListMixed);
                    return Ok(false);
                }
                '(' => {
                    // Allow for the possibility that we have a tuple without a kindicle.
                    // e.g. ((u8|1), ..)
                    state.molecular_capture = Some(MolecularCapture::ListMixed);
                    return Ok(false);
                }
                ')' => return Ok(false),
                _ => {
                    return Err(err!(
                        "Found unrecognised '{}' at position {} while capturing daticle kind.", c, i + 1;
                    String, Input, Decode, Invalid));
                }
            }
        }
        Ok(false)
    }
}

impl fmt::Display for Slurp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.s[..])
    }
}

#[derive(Clone, Debug, Default)]
pub struct MapCounter {
    count:  u64,
    order:  u64,
    delta:  u64,
}

impl MapCounter {
    fn order(&self) -> u64 { self.order }
    fn inc(&mut self) -> Outcome<()> {
        self.count = try_add!(self.count, 1);    
        Ok(())
    }
    fn inc_all(&mut self) -> Outcome<()> {
        self.count = try_add!(self.count, 1);    
        self.order = try_add!(self.order, self.delta);    
        Ok(())
    }
}

impl Kind {

    pub const USR_LABEL_PREFIX: &'static str = "usr_";

    /// Reverse lookup from name to enum value that first tries the standard names then the given
    /// user names.
    #[inline(never)]
    pub fn from_label<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        s: &str,
        ukinds_opt: Option<&UsrKinds<M1, M2>>,
    )
        -> Outcome<Self>
    {
        match Self::from_str(s) {
            Ok(k) => Ok(k),
            Err(e1) => match ukinds_opt {
                Some(ukids) => match ukids.get_label(s) {
                    Some(ukid) => Ok(Self::Usr(ukid.clone())),
                    None => Err(err!(
                        "Daticle kind label not recognised as standard or custom: '{}'", s;
                    Input, String, Unknown)),
                }
                None => Err(err!(e1, "No custom user kinds supplied"; Input, String, Unknown)),
            }
        }
    }

    #[inline(never)]
    fn decode_number(self, ns: NumberString) -> Outcome<Dat> {
        match self {
            Kind::U8 => {
                if ns.is_negative() {
                    return Err(err!(
                	    "U8 '{}' cannot be negative",
                        &ns.int_string();
                    String, Input, Decode, Invalid));
                }
                let n = res!(<u8>::from_str_radix(ns.abs_integer_str(), ns.radix()));
                return Ok(Dat::U8(n));
            }
            Kind::U16 => {
                if ns.is_negative() {
                    return Err(err!(
                	    "U16 '{}' cannot be negative",
                        &ns.int_string();
                    String, Input, Decode, Invalid));
                }
                let n = res!(<u16>::from_str_radix(ns.abs_integer_str(), ns.radix()));
                return Ok(Dat::U16(n));
            }
            Kind::U32 => {
                if ns.is_negative() {
                    return Err(err!(
                	    "U32 '{}' cannot be negative",
                        &ns.int_string();
                    String, Input, Decode, Invalid));
                }
                let n = res!(<u32>::from_str_radix(ns.abs_integer_str(), ns.radix()));
                return Ok(Dat::U32(n));
            }
            Kind::U64 | Kind::C64 => {
                if ns.is_negative() {
                    return Err(err!(
                	    "U64 '{}' cannot be negative",
                        &ns.int_string();
                    String, Input, Decode, Invalid));
                }
                let n = res!(<u64>::from_str_radix(ns.abs_integer_str(), ns.radix()));
                if self == Kind::U64 {
                    return Ok(Dat::U64(n));
                } else {
                    return Ok(Dat::C64(n));
                }
            }
            Kind::U128 => {
                if ns.is_negative() {
                    return Err(err!(
                	    "U128 '{}' cannot be negative",
                        &ns.int_string();
                    String, Input, Decode, Invalid));
                }
                let n = res!(<u128>::from_str_radix(ns.abs_integer_str(), ns.radix()));
                return Ok(Dat::U128(n));
            }
            Kind::I8 => {
                let n = res!(<i8>::from_str_radix(&ns.int_string(), ns.radix()));
                return Ok(Dat::I8(n));
            }
            Kind::I16 => {
                let n = res!(<i16>::from_str_radix(&ns.int_string(), ns.radix()));
                return Ok(Dat::I16(n));
            }
            Kind::I32 => {
                let n = res!(<i32>::from_str_radix(&ns.int_string(), ns.radix()));
                return Ok(Dat::I32(n));
            }
            Kind::I64 => {
                let n = res!(<i64>::from_str_radix(&ns.int_string(), ns.radix()));
                return Ok(Dat::I64(n));
            }
            Kind::I128 => {
                let n = res!(<i128>::from_str_radix(&ns.int_string(), ns.radix()));
                return Ok(Dat::I128(n));
            }
            Kind::F32 => {
                return Ok(Dat::F32(res!(Float32::from_str(ns.source()))));
            }
            Kind::F64 => {
                return Ok(Dat::F64(res!(Float64::from_str(ns.source()))));
            }
            Kind::Aint => {
                if ns.has_point() || ns.has_exp() {
                    return Err(err!(
                        "Decimal or scientific notation not accepted for AINT, use ADEC";
                    String, Input, Decode, Invalid));
                }
                match ns.as_bigint() {
                    Ok(n) => return Ok(Dat::Aint(n)),
                    Err(e) => return Err(err!(e, "While decoding an integer."; Decode, Integer)),
                }
            }
            Kind::Adec => {
                return Ok(Dat::Adec(res!(ns.as_bigdecimal())));
            }
            _ => return Err(err!(
                "NumberString {:?} is of invalid kind {:?}",
                ns, self;
            String, Input, Decode, Invalid)),
        }
    }

    /// Classify which `Kind`s are numbers for the purposes of string decoding.
    pub fn is_number(&self) -> bool {
        match *self {
            Self::U8    |
            Self::U16   |
            Self::U32   |
            Self::U64   |
            Self::C64   |
            Self::U128  |
            Self::I8    |
            Self::I16   |
            Self::I32   |
            Self::I64   |
            Self::I128  |
            Self::F32   |
            Self::F64   |
            Self::Aint  |
            Self::Adec  => true,
            _ => false,
        }
    }

    pub fn uses_list_brackets(&self) -> bool {
        match *self {
            // Heterogenous
            Self::List      |
            Self::Tup2      |
            Self::Tup3      |
            Self::Tup4      |
            Self::Tup5      |
            Self::Tup6      |
            Self::Tup7      |
            Self::Tup8      |
            Self::Tup9      |
            Self::Tup10     |
            // Homogenous
            Self::Vek       |
            // Variable length bytes
            Self::BU8       |
            Self::BU16      |
            Self::BU32      |
            Self::BU64      |
            Self::BC64      |
            // Fixed length bytes
            Self::B2        |
            Self::B3        |
            Self::B4        |
            Self::B5        |
            Self::B6        |
            Self::B7        |
            Self::B8        |
            Self::B9        |
            Self::B10       |
            Self::B16       |
            Self::B32       |
            // Fixed length numbers
            Self::Tup2u8    |
            Self::Tup3u8    |
            Self::Tup4u8    |
            Self::Tup5u8    |
            Self::Tup6u8    |
            Self::Tup7u8    |
            Self::Tup8u8    |
            Self::Tup9u8    |
            Self::Tup10u8   |
            Self::Tup2u16   |
            Self::Tup3u16   |
            Self::Tup4u16   |
            Self::Tup5u16   |
            Self::Tup6u16   |
            Self::Tup7u16   |
            Self::Tup8u16   |
            Self::Tup9u16   |
            Self::Tup10u16  |
            Self::Tup2u32   |
            Self::Tup3u32   |
            Self::Tup4u32   |
            Self::Tup5u32   |
            Self::Tup6u32   |
            Self::Tup7u32   |
            Self::Tup8u32   |
            Self::Tup9u32   |
            Self::Tup10u32  |
            Self::Tup2u64   |
            Self::Tup3u64   |
            Self::Tup4u64   |
            Self::Tup5u64   |
            Self::Tup6u64   |
            Self::Tup7u64   |
            Self::Tup8u64   |
            Self::Tup9u64   |
            Self::Tup10u64  |
            Self::Tup2i8    |
            Self::Tup3i8    |
            Self::Tup4i8    |
            Self::Tup5i8    |
            Self::Tup6i8    |
            Self::Tup7i8    |
            Self::Tup8i8    |
            Self::Tup9i8    |
            Self::Tup10i8   |
            Self::Tup2i16   |
            Self::Tup3i16   |
            Self::Tup4i16   |
            Self::Tup5i16   |
            Self::Tup6i16   |
            Self::Tup7i16   |
            Self::Tup8i16   |
            Self::Tup9i16   |
            Self::Tup10i16  |
            Self::Tup2i32   |
            Self::Tup3i32   |
            Self::Tup4i32   |
            Self::Tup5i32   |
            Self::Tup6i32   |
            Self::Tup7i32   |
            Self::Tup8i32   |
            Self::Tup9i32   |
            Self::Tup10i32  |
            Self::Tup2i64   |
            Self::Tup3i64   |
            Self::Tup4i64   |
            Self::Tup5i64   |
            Self::Tup6i64   |
            Self::Tup7i64   |
            Self::Tup8i64   |
            Self::Tup9i64   |
            Self::Tup10i64  => true,
            _ => false,
        }
    }
}

impl Dat {

    /// Decodes JDAT text under the default limits, which refuse text that is too long or nested too
    /// deep.
    pub fn decode_string<
        S: Into<String>,
    >(
        s: S,
    )
        -> Outcome<Self>
    {
        let dec_cfg = DecoderConfig::<
            BTreeMap<UsrKindCode, UsrKind>,
            BTreeMap<String, UsrKindId>,
        >::default();

        Self::decode_string_with_config(s, &dec_cfg)
    }

    /// Decodes JDAT text under the given limits, which refuse text that is too long or nested too
    /// deep.
    pub fn decode_string_limited<
        S: Into<String>,
    >(
        s:      S,
        lims:   &DecodeLimits,
    )
        -> Outcome<Self>
    {
        let dec_cfg = DecoderConfig::<
            BTreeMap<UsrKindCode, UsrKind>,
            BTreeMap<String, UsrKindId>,
        >::default().with_limits(*lims);

        Self::decode_string_with_config(s, &dec_cfg)
    }

    /// Decodes JDAT text using the given configuration, whose limits refuse text that is too long
    /// or nested too deep.
    pub fn decode_string_with_config<
        S: Into<String>,
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        s: S,
        cfg: &DecoderConfig<M1, M2>,
    )
        -> Outcome<Self>
    {
        // We want to take ownership of the String.
        let s = s.into();
        res!(cfg.limits.check_len(s.len()));
        let mut iter = s.chars().collect::<Vec<_>>().into_iter().enumerate();
        Self::recursive_decode(
            &mut iter,
            cfg,
            DecoderState::default(),
            &RefCell::new(Cursor::default()),
        )
    }

    /// Decodes a value at one level of nesting, recursing at each `(k|`, `[` and `{`.
    ///
    /// The recursion is bounded by `cfg.limits.max_depth`, and the frame is kept small by moving
    /// each of the fatter arms of the match below into its own `#[inline(never)]` function, so that
    /// the cost of a level of nesting is the cost of the loop and its store, not the sum of every
    /// arm's locals.  A hostile text nesting a list a million deep must return an error, not exhaust
    /// the stack, since a stack overflow aborts the process and cannot be caught.
    ///
    /// * `kind_outer` - if not None, indicates that the upper recursion level is a
    /// `List` or `Map`/`OrdMap`
    ///
    /// BNF (does not include comments)
    /// <atom> ::= "" | <string> | <number> | "(" <kind> ")" | "(" <kind> "|" ")"
    /// <daticle> ::= <atom> | <seq> | "(" <kind> "|" <daticle> ")"
    /// <kind> ::= EMPTY | TRUE | FALSE | U8 ...
    /// <seq> ::= <lbrac> <seq> "," <daticle> <rbrac> | <lbrac> <seq> "," <pair> <rbrac>
    /// <pair> ::= <daticle> ":" <daticle>
    /// <lbrac> ::= "[" | "{"
    /// <rbrac> ::= "]" | "}"
    pub fn recursive_decode<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        mut iter:   &mut std::iter::Enumerate<std::vec::IntoIter<char>>,
        cfg:        &DecoderConfig<M1, M2>,
        mut state:  DecoderState,
        cursor:     &RefCell<Cursor>,
    )
        -> Outcome<Self>
    {
        // Refuse the level before descending into it, naming the character we are looking at.
        let pos = cursor.borrow().abs;
        res!(cfg.limits.check_char_depth(state.depth, pos));

        // Switches.
        let mut comment_required = false;
        // Store.
        let mut store = DecoderStore::new(cfg);

        state.molecular_capture = MolecularCapture::from_kind(&state.kind_outer);

        while let Some((i, c)) = iter.next() {
            match res!(Self::step(
                (i, c),
                cfg,
                &mut state,
                &mut store,
                cursor,
                &mut comment_required,
            )) {
                Step::Continue => (),
                Step::Done(dat) => return Ok(dat),
                Step::Descend(new_state, descent) => {
                    // The one place the decoder descends a level.  Every other part of reading a
                    // character has already returned by now, so the frame that carries the
                    // recursion holds only the store, the state and this daticle.
                    let dat = res!(Self::recursive_decode(
                        &mut iter,
                        cfg,
                        new_state,
                        cursor,
                    ));
                    store.slurp = Slurp::new();
                    match descent {
                        Descent::Kindicle if Self::kindicle_completes(&state) => return Ok(dat),
                        _ => store.val_opt = Some(dat),
                    }
                }
            }
        } // while loop
        Self::finish(&state, &mut store)
    }

    /// Whether a kindicled daticle, e.g. the `42` of `(u8|42)`, is the whole of this level.
    #[inline(never)]
    fn kindicle_completes(state: &DecoderState) -> bool {
        if state.kind_outer == Kind::Unknown {
            state.molecular_capture == None
        } else {
            state.kind_outer.case().class() == KindClass::Atomic
        }
    }

    /// Reads one character, returning what the level it belongs to should do next.
    ///
    /// Every arm of the match lives here rather than in [`Self::recursive_decode`], and the descent
    /// is handed back to the caller rather than taken here, so that none of this function's locals
    /// are on the stack while the decoder is inside a nested level.  That is what makes the cost of
    /// a level of nesting a fixed few hundred bytes.
    #[inline(never)]
    fn step<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        (i, c):             (usize, char),
        cfg:                &DecoderConfig<M1, M2>,
        state:              &mut DecoderState,
        store:              &mut DecoderStore,
        cursor:             &RefCell<Cursor>,
        comment_required:   &mut bool,
    )
        -> Outcome<Step>
    {
        {
            cursor.borrow_mut().advance(c, state.quote_protection != Quote::None);
        }
        // String escape handling (RFC 8259 §7). Only active
        // inside a quoted string. Runs ahead of the outer
        // `"` / `'` matching so that `\"` inside a string is
        // interpreted as an escape, not a string terminator.
        if state.quote_protection != Quote::None {
            if res!(Self::handle_string_escape(c, state, &mut store.slurp)) {
                return Ok(Step::Continue);
            }
        }
        if cfg.quote_protection {
            match c {
                '"'  => {
                    if state.quote_protection != Quote::Single {
                        if state.quote_protection == Quote::Double {
                            state.quote_protection = Quote::None;
                            // Whenever quote protection is applied to some or all of a string,
                            // the entire string is flagged as a STR kind.
                            store.slurp.flag_as_string();
                        } else {
                            state.quote_protection = Quote::Double;
                        }
                        return Ok(Step::Continue);
                    }
                }
                '\''  => {
                    if state.quote_protection != Quote::Double {
                        if state.quote_protection == Quote::Single {
                            state.quote_protection = Quote::None;
                            // Whenever quote protection is applied to some or all of a string,
                            // the entire string is flagged as a STR kind
                            store.slurp.flag_as_string();
                        } else {
                            state.quote_protection = Quote::Single;
                        }
                        return Ok(Step::Continue);
                    }
                }
                _ => {}
            }
        }
        if state.comment_capture.is_some() {
            res!(Self::capture_comment(c, cfg, state, store, cursor));
            return Ok(Step::Continue);
        }
        if cfg.comment_allowed && state.quote_protection == Quote::None {
            // Start comment capturing?
            if c == cfg.comment1_start_char {
                state.comment_capture = Some(CommentCapture::Type1);
                store.note_config = store.note_config.extract().set_type1(true);
                return Ok(Step::Continue);
            }
            if c == cfg.comment2_start_char {
                state.comment_capture = Some(CommentCapture::Type2);
                store.note_config = store.note_config.extract().set_type1(false);
                return Ok(Step::Continue);
            }
        }

        // Every character is first offered to the slurp, which takes it while a number, a quoted
        // string or a kind label is being gathered.  A single call site here, rather than the same
        // call at the head of a dozen match arms, keeps the frame small.  The separator '|' is the
        // exception, since a kind capture must see it and the slurp would refuse it.
        let whitespace = matches!(c, ' ' | '\t' | '\n' | '\r');
        let slurped = if c == '|' {
            if state.quote_protection != Quote::None {
                store.slurp.push(c);
                true
            } else {
                false
            }
        } else {
            res!(store.slurp.char_slurped((i, c), state))
        };
        // Whitespace is offered to the slurp but never ends the character's handling, since a
        // space may also close an atom.
        if slurped && !whitespace {
            return Ok(Step::Continue);
        }

        match c {
            // JSON RFC 8259 §2 allows U+0009 tab as whitespace, so
            // JDAT accepts it alongside space / LF / CR. This lets
            // JSON files using tab indentation round-trip through
            // `Dat::decode_string` unchanged.
            ' ' | '\t' | '\n' | '\r' => (), // The slurp has taken it, if it wanted it.
            '(' => {
                if state.kind_outer == Kind::Unknown ||
                    state.kind_outer.case() == KindCase::MoleculeUnitary ||
                    state.molecular_capture != None
                {
                    // Begin capturing the daticle kind (or "kindicle").
                    state.kind_capture = true;
                    store.slurp = Slurp::new();
                } else {
                    // "(k1|(k2|v))"
                    //      ^ already expect k1 kind, invalid unless preceded by [ or {
                    return Err(Self::open_paren_err(state, cursor));
                }
            }
            '|' => {
                if state.kind_capture {
                    // We have captured a kind, triggering a new level of recursion.
                    let kind_inner = res!(Kind::from_label(
                        &store.slurp.clone_string().to_lowercase(),
                        cfg.ukinds_opt.as_ref(),
                    ));
                    if kind_inner.case() == KindCase::AtomLogic {
                        // An unused '|' separator is not valid, e.g. (true|), (EMPTY|)
                        return Err(Self::superfluous_bar_err(&kind_inner, cursor));
                    }
                    state.kind_capture = false;
                    let mut new_state = state.recurse();
                    new_state.kind_outer = kind_inner;
                    new_state.explicit_kind = true;
                    return Ok(Step::Descend(new_state, Descent::Kindicle));
                }
            }
            ')' => {
                // Deal with atoms (e.g. (FALSE)), which should only
                // ever be processed within a recursion level.
                match res!(Self::close_paren(cfg, state, store, cursor, comment_required)) {
                    Some(dat) => return Ok(Step::Done(dat)),
                    None => (), // The daticle may yet be one item of a molecule.
                }
            }
            '[' => {
                if state.molecular_capture == None {
                    if state.kind_outer == Kind::Unknown {
                        state.molecular_capture = Some(MolecularCapture::ListMixed);
                    } else {
                        match MolecularCapture::from_kind(&state.kind_outer) {
                            Some(MolecularCapture::Map) |
                            None => return Err(Self::open_bracket_err(state, cursor)),
                            _ => (),
                        }
                    }
                }
                let mut new_state = state.recurse();
                if !state.kind_outer.uses_list_brackets() {
                    new_state.kind_outer = Kind::List;
                }
                return Ok(Step::Descend(new_state, Descent::Molecule));
            }
            ',' => {
                res!(Self::comma_handler(cfg, state, cursor, store));
            }
            ']' => {
                // A terminal branch, so the state and the store are given away.
                return Ok(Step::Done(res!(Self::close_bracket(
                    state.extract(),
                    store.extract(),
                    cursor,
                ))));
            }
            '{' => {
                if state.molecular_capture == None {
                    state.molecular_capture = Some(MolecularCapture::Map);
                    // Also honour cfg.use_ordmaps at the top
                    // level. Without this, a root `{...}`
                    // decoded with `use_ordmaps = true` still
                    // lands in a `Dat::Map` because kind_outer
                    // was never set from Kind::Unknown -- only
                    // the nested recurse branch below did it.
                    if !state.explicit_kind && cfg.use_ordmaps {
                        state.kind_outer = Kind::OrdMap;
                    }
                } else {
                    let mut new_state = state.recurse();
                    if !state.explicit_kind {
                        // We are free to define the kind of this store.map.
                        match cfg.use_ordmaps {
                            true => new_state.kind_outer = Kind::OrdMap,
                            false => new_state.kind_outer = Kind::Map,
                        }
                    }
                    return Ok(Step::Descend(new_state, Descent::Molecule));
                }
            }
            ':' => {
                res!(Self::colon(state, store, cursor));
            }
            '}' => {
                // A terminal branch, so the store is given away.
                return Ok(Step::Done(res!(Self::close_brace(
                    cfg,
                    state,
                    store.extract(),
                    cursor,
                ))));
            }
            _ => {
                store.slurp.push(c);
            }
        } // match
        Ok(Step::Continue)
    }

    /// Concludes a level whose text ran out before a closing bracket did, which is legal only for
    /// an atom.
    #[inline(never)]
    fn finish(
        state:  &DecoderState,
        store:  &mut DecoderStore,
    )
        -> Outcome<Self>
    {
        if let Some(dat) = store.val_opt.take() {
            return Ok(dat);
        }
        match state.molecular_capture {
            None => Self::process_atom(&mut store.slurp, &state.kind_outer),
            Some(MolecularCapture::ListMixed)  |
            Some(MolecularCapture::ListSame)   |
            Some(MolecularCapture::Bytes) =>
                Err(err!("Expected closure of a store.list with ']'";
                    String, Input, Decode, Missing)),
            Some(MolecularCapture::Map) =>
                Err(err!("Expected closure of a store.map with '}}'";
                    String, Input, Decode, Missing)),
        }
    }

    /// Reports a `(` which repeats a kind that has already been given.
    #[inline(never)]
    fn open_paren_err(
        state:  &DecoderState,
        cursor: &RefCell<Cursor>,
    )
        -> Error<ErrTag>
    {
        match state.kind_outer.case() {
            KindCase::MoleculeSame => err!(
                "Elements of a vector of kind {:?} are not daticles, \
                so no kind should be specified ({})",
                state.kind_outer, cursor.borrow();
            String, Input, Decode, Invalid),
            _ => err!(
                "The kind for the daticle has already been specified \
                as {:?} ({})", state.kind_outer, cursor.borrow();
            String, Input, Decode, Invalid),
        }
    }

    /// Reports a `|` separating a kind that carries no data, e.g. `(true|)`.
    #[inline(never)]
    fn superfluous_bar_err(
        kind_inner: &Kind,
        cursor:     &RefCell<Cursor>,
    )
        -> Error<ErrTag>
    {
        err!(
            "The separation character \"|\" for {:?} is superfluous {}.",
            kind_inner, cursor.borrow();
        String, Input, Decode, Invalid)
    }

    /// Reports a `[` opening a molecule that the outer kind cannot hold.
    #[inline(never)]
    fn open_bracket_err(
        state:  &DecoderState,
        cursor: &RefCell<Cursor>,
    )
        -> Error<ErrTag>
    {
        match MolecularCapture::from_kind(&state.kind_outer) {
            Some(MolecularCapture::Map) => err!(
                "Expecting a store.map bracket '{{' but found a '[' ({})", cursor.borrow();
            String, Input, Decode, Invalid),
            _ => err!(
                "Found a '[' which is incompatible with a {:?} ({})",
                state.kind_outer, cursor.borrow();
            String, Input, Decode, Invalid),
        }
    }

    /// Consumes a character while a comment is being captured, closing the comment at the end
    /// character or the end of the line.
    #[inline(never)]
    fn capture_comment<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        c:      char,
        cfg:    &DecoderConfig<M1, M2>,
        state:  &mut DecoderState,
        store:  &mut DecoderStore,
        cursor: &RefCell<Cursor>,
    )
        -> Outcome<()>
    {
        let capturing = match &state.comment_capture {
            Some(capturing) => capturing.clone(),
            None => return Ok(()),
        };
        // Finish comment capturing?
        if (
            capturing == CommentCapture::Type1
            && (c == cfg.comment1_end_char || c == '\n')
        ) || (
            capturing == CommentCapture::Type2
            && (c == cfg.comment2_end_char || c == '\n')
        ) {
            state.comment_capture = None;
            store.comment = store.comment.trim_start().to_string();
            if c == '\n' {
                if let Some(molecular_capture) = &state.molecular_capture {
                    let mut dat = match store.val_opt.take() {
                        Some(dat) => dat, // store.val_opt is now None.
                        None => {
                            let dat = if store.slurp.has_content() {
                                res!(Self::process_atom(&mut store.slurp, &Kind::Unknown))
                            } else {
                                Dat::Empty
                            };
                            dat
                        }
                    };
                    dat = Dat::ABox(
                        store.note_config.extract(),
                        Box::new(dat),
                        store.comment.extract(),
                    );
                    store.slurp = Slurp::new();
                    match molecular_capture {
                        MolecularCapture::Map => {
                            match store.key_opt.take() {
                                Some(key) => {
                                    res!(Self::map_insert(
                                        state.kind_outer == Kind::OrdMap,
                                        store,
                                        (key, dat),
                                    ));
                                }
                                None => {
                                    res!(Self::map_insert(
                                        state.kind_outer == Kind::OrdMap,
                                        store,
                                        (dat, Dat::Empty),
                                    ));
                                }
                            }
                        }
                        _ => {
                            store.list.push(dat);
                        }
                    }
                } else {
                    return Err(err!(
                        "Line comments currently only allowed in molecules \
                        (lists and maps). ({})", cursor.borrow();
                    String, Input, Decode, Invalid));
                }
            }
            return Ok(());
        }
        if cfg.comment_capture {
            store.comment.push(c);
        }
        Ok(())
    }

    /// Concludes a `)`, returning the completed daticle, or `None` if the level continues because
    /// the daticle may yet be one item of a molecule.
    #[inline(never)]
    fn close_paren<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        cfg:                &DecoderConfig<M1, M2>,
        state:              &mut DecoderState,
        store:              &mut DecoderStore,
        cursor:             &RefCell<Cursor>,
        comment_required:   &mut bool,
    )
        -> Outcome<Option<Self>>
    {
        let kind_inner = if state.kind_capture {
            // We were capturing a kind, but the daticle finished before
            // | and no data was found.  This is valid for AtomLogic
            // cases and usr kinds.  Set the inner kind.
            let slurped = store.slurp.clone_string().to_lowercase();
            let kind_inner = if slurped.len() == 0 {
                // First take care of case "()".
                Kind::Empty
            } else {
                // (EMPTY), (TRUE), (FALSE), (NONE), (my_kind), etc. are valid.
                let k = res!(Kind::from_label(&slurped, cfg.ukinds_opt.as_ref()));
                if !k.is_dataless() {
                    return Err(err!(
                        "Closing ')' without a kindicle should have triggered \
                        set MolecularCapture::ListMixed, but instead the state is \
                        {:?}. ({})", state.molecular_capture, cursor.borrow();
                    String, Input, Decode, Invalid));
                }
                k
            };
            state.kind_capture = false;
            store.slurp.reset();
            kind_inner
        } else {
            Kind::Unknown
        };
        let mut kind = kind_inner.clone();
        if kind == Kind::Unknown {
            kind = state.kind_outer.clone();
        }

        if kind == Kind::Unknown {
            // For unknown kinds, first attempt to interpret as a tuple.
            // When no kindicle is specified, items accumulate directly in store.list
            match state.molecular_capture {
                Some(MolecularCapture::ListMixed) => {}
                _ => {
                    return Err(err!(
                        "Closing ')' without a kindicle should have triggered \
                        set MolecularCapture::ListMixed, but instead the state is \
                        {:?}. ({})", state.molecular_capture, cursor.borrow();
                    String, Input, Decode, Invalid));
                }
            }
            // Complete the capture of the store.list by converting it to the correct
            // type.
            if store.slurp.has_content() {
                store.val_opt = Some(res!(Self::process_atom(&mut store.slurp, &kind)));
            }
            match store.val_opt.take() {
                Some(dat) => {
                    if store.comment.len() > 0 {
                        store.list.push(Dat::ABox(
                            store.note_config.extract(),
                            Box::new(dat),
                            store.comment.extract(),
                        ));
                    } else {
                        store.list.push(dat);
                    }
                }
                None => {
                    if store.comment.len() > 0 {
                        store.list.push(Dat::ABox(
                            store.note_config.extract(),
                            Box::new(Dat::Empty),
                            store.comment.extract(),
                        ));
                    }
                }
            }
            return Ok(Some(res!(Self::implicit_tuple(store.list.extract(), cursor))));
        }

        match &kind {
            // No data is needed for these kinds, so create them here.
            Kind::Empty => store.val_opt = Some(Dat::Empty),
            Kind::True  => store.val_opt = Some(Dat::Bool(true)),
            Kind::False => store.val_opt = Some(Dat::Bool(false)),
            Kind::None  => store.val_opt = Some(Dat::Opt(Box::new(None))),
            Kind::Usr(ukid) if ukid.kind().is_none() =>
                store.val_opt = Some(Dat::Usr(ukid.clone(), None)),
            _ => (),
            // We don't necessarily return out of the method here because the
            // daticle might be part of a molecule, with the exception of the outer
            // kind being a Dat::ABox.
        }
        if let Kind::ABox(_) = state.kind_outer {
            match &store.val_opt {
                Some(dat) => {
                    if store.comment.len() > 0 {
                        return Ok(Some(Dat::ABox(
                            store.note_config.extract(),
                            Box::new(dat.clone()),
                            store.comment.extract(),
                        )));
                    } else {
                        *comment_required = true;
                    }
                }
                None => {
                    return Err(err!(
                        "Daticle missing in Dat::ABox ({})", cursor.borrow();
                    String, Input, Decode, Invalid, Missing));
                }
            }
        }

        if !*comment_required && (
            !kind.is_dataless() || state.molecular_capture == None
        ) {
            // Atom point cases were just dealt with, while molecular capture is
            // ongoing until a `]` or `}` is encountered, with the exception of a
            // base2x string for bytes.
            if state.molecular_capture == None || (
                state.molecular_capture == Some(MolecularCapture::Bytes)
                && store.slurp.is_string()
            ) {
                let d = match store.val_opt.take() {
                    Some(d) => d,
                    None => res!(Self::process_atom(&mut store.slurp, &kind)),
                };
                return Ok(Some(match state.kind_outer.clone() {
                    // Unitary molecules
                    Kind::Usr(ukid) => Dat::Usr(ukid, Some(Box::new(d))),
                    Kind::Box(_)    => Dat::Box(Box::new(d)),
                    Kind::Some(_)   => Dat::Opt(Box::new(Some(d))),
                    _ => d,
                }));
            } else {
                match store.val_opt.take() {
                    Some(d) => return Ok(Some(d)),
                    None => {
                        return Err(err!(
                            "Failed to capture the daticle of kind {:?} ({})",
                            kind, cursor.borrow();
                        String, Input, Decode, Invalid));
                    }
                }
            }
        }
        Ok(None)
    }

    /// Converts the daticles gathered between round brackets into a tuple, e.g. `(1, 2)`.
    #[inline(never)]
    fn implicit_tuple(
        mut list:   Vec<Dat>,
        cursor:     &RefCell<Cursor>,
    )
        -> Outcome<Self>
    {
        match list.len() {
            0 => Ok(Dat::Empty),
            1 => Ok(list.remove(0)),
            2 => Ok(Dat::Tup2(Box::new(
                res!(list.try_into().map_err(|_| err!(
                    "While decoding a 2-item tuple ({})", cursor.borrow();
                String, Input, Decode, Invalid)))
            ))),
            3 => Ok(Dat::Tup3(Box::new(
                res!(list.try_into().map_err(|_| err!(
                    "While decoding a 3-item tuple ({})", cursor.borrow();
                String, Input, Decode, Invalid)))
            ))),
            4 => Ok(Dat::Tup4(Box::new(
                res!(list.try_into().map_err(|_| err!(
                    "While decoding a 4-item tuple ({})", cursor.borrow();
                String, Input, Decode, Invalid)))
            ))),
            5 => Ok(Dat::Tup5(Box::new(
                res!(list.try_into().map_err(|_| err!(
                    "While decoding a 5-item tuple ({})", cursor.borrow();
                String, Input, Decode, Invalid)))
            ))),
            6 => Ok(Dat::Tup6(Box::new(
                res!(list.try_into().map_err(|_| err!(
                    "While decoding a 6-item tuple ({})", cursor.borrow();
                String, Input, Decode, Invalid)))
            ))),
            7 => Ok(Dat::Tup7(Box::new(
                res!(list.try_into().map_err(|_| err!(
                    "While decoding a 7-item tuple ({})", cursor.borrow();
                String, Input, Decode, Invalid)))
            ))),
            8 => Ok(Dat::Tup8(Box::new(
                res!(list.try_into().map_err(|_| err!(
                    "While decoding a 8-item tuple ({})", cursor.borrow();
                String, Input, Decode, Invalid)))
            ))),
            9 => Ok(Dat::Tup9(Box::new(
                res!(list.try_into().map_err(|_| err!(
                    "While decoding a 9-item tuple ({})", cursor.borrow();
                String, Input, Decode, Invalid)))
            ))),
            10 => Ok(Dat::Tup10(Box::new(
                res!(list.try_into().map_err(|_| err!(
                    "While decoding a 10-item tuple ({})", cursor.borrow();
                String, Input, Decode, Invalid)))
            ))),
            n => Err(err!(
                "Tuples are limited to 10 items, {} found ({}).",
                n, cursor.borrow();
            String, Input, Decode, Invalid)),
        }
    }

    /// Concludes a `]`, completing the list, vector, byte string or tuple at this level.
    #[inline(never)]
    fn close_bracket(
        mut state:  DecoderState,
        mut store:  DecoderStore,
        cursor:     &RefCell<Cursor>,
    )
        -> Outcome<Self>
    {
        match state.molecular_capture {
            Some(MolecularCapture::Bytes) => {
                if store.slurp.has_content() {
                    let n = try_extract_dat!(
                        res!(Self::process_atom(&mut store.slurp, &Kind::U8)), U8,
                    );
                    store.byts.push(n);
                }
                return Self::decode_bytes(store.byts, &state.kind_outer);
            }
            Some(MolecularCapture::ListMixed) |
            Some(MolecularCapture::ListSame) => {
                // Complete the capture of the store.list by converting it to the correct
                // type.
                if store.slurp.has_content() {
                    let kind = match state.molecular_capture {
                        Some(MolecularCapture::ListSame) =>
                            MolecularCapture::same_kind(&state.kind_outer),
                        _ => state.kind_outer.clone(),
                    };
                    store.val_opt = Some(res!(Self::process_atom(&mut store.slurp, &kind)));
                }
                match store.val_opt {
                    Some(dat) => {
                        if store.comment.len() > 0 {
                            store.list.push(Dat::ABox(
                                store.note_config.clone(),
                                Box::new(dat),
                                store.comment,
                            ));
                        } else {
                            store.list.push(dat);
                        }
                    }
                    None => {
                        if store.comment.len() > 0 {
                            store.list.push(Dat::ABox(
                                store.note_config.clone(),
                                Box::new(Dat::Empty),
                                store.comment,
                            ));
                        }
                    }
                }
                state.molecular_capture = None;
                match state.kind_outer {
                    Kind::List | Kind::Unknown => return Ok(Dat::List(store.list)),

                    Kind::Tup2  => string_decode_heterogenous_tuple! { Tup2,    2, store.list, state },
                    Kind::Tup3  => string_decode_heterogenous_tuple! { Tup3,    3, store.list, state },
                    Kind::Tup4  => string_decode_heterogenous_tuple! { Tup4,    4, store.list, state },
                    Kind::Tup5  => string_decode_heterogenous_tuple! { Tup5,    5, store.list, state },
                    Kind::Tup6  => string_decode_heterogenous_tuple! { Tup6,    6, store.list, state },
                    Kind::Tup7  => string_decode_heterogenous_tuple! { Tup7,    7, store.list, state },
                    Kind::Tup8  => string_decode_heterogenous_tuple! { Tup8,    8, store.list, state },
                    Kind::Tup9  => string_decode_heterogenous_tuple! { Tup9,    9, store.list, state },
                    Kind::Tup10 => string_decode_heterogenous_tuple! { Tup10,   10, store.list, state },

                    Kind::Vek   => return Ok(Dat::Vek(res!(Vek::try_from(store.list)))),
                    Kind::Tup2u8    => string_decode_int_tuple! { Tup2u8,   U8,  u8,  2, store.list, state },
                    Kind::Tup3u8    => string_decode_int_tuple! { Tup3u8,   U8,  u8,  3, store.list, state },
                    Kind::Tup4u8    => string_decode_int_tuple! { Tup4u8,   U8,  u8,  4, store.list, state },
                    Kind::Tup5u8    => string_decode_int_tuple! { Tup5u8,   U8,  u8,  5, store.list, state },
                    Kind::Tup6u8    => string_decode_int_tuple! { Tup6u8,   U8,  u8,  6, store.list, state },
                    Kind::Tup7u8    => string_decode_int_tuple! { Tup7u8,   U8,  u8,  7, store.list, state },
                    Kind::Tup8u8    => string_decode_int_tuple! { Tup8u8,   U8,  u8,  8, store.list, state },
                    Kind::Tup9u8    => string_decode_int_tuple! { Tup9u8,   U8,  u8,  9, store.list, state },
                    Kind::Tup10u8   => string_decode_int_tuple! { Tup10u8,  U8,  u8,  10, store.list, state },

                    Kind::Tup2u16   => string_decode_int_tuple! { Tup2u16,  U16, u16, 2, store.list, state },
                    Kind::Tup3u16   => string_decode_int_tuple! { Tup3u16,  U16, u16, 3, store.list, state },
                    Kind::Tup4u16   => string_decode_int_tuple! { Tup4u16,  U16, u16, 4, store.list, state },
                    Kind::Tup5u16   => string_decode_int_tuple! { Tup5u16,  U16, u16, 5, store.list, state },
                    Kind::Tup6u16   => string_decode_int_tuple! { Tup6u16,  U16, u16, 6, store.list, state },
                    Kind::Tup7u16   => string_decode_int_tuple! { Tup7u16,  U16, u16, 7, store.list, state },
                    Kind::Tup8u16   => string_decode_int_tuple! { Tup8u16,  U16, u16, 8, store.list, state },
                    Kind::Tup9u16   => string_decode_int_tuple! { Tup9u16,  U16, u16, 9, store.list, state },
                    Kind::Tup10u16  => string_decode_int_tuple! { Tup10u16, U16, u16, 10, store.list, state },

                    Kind::Tup2u32   => string_decode_int_tuple! { Tup2u32,  U32, u32, 2, store.list, state },
                    Kind::Tup3u32   => string_decode_int_tuple! { Tup3u32,  U32, u32, 3, store.list, state },
                    Kind::Tup4u32   => string_decode_int_tuple! { Tup4u32,  U32, u32, 4, store.list, state },
                    Kind::Tup5u32   => string_decode_int_tuple! { Tup5u32,  U32, u32, 5, store.list, state },
                    Kind::Tup6u32   => string_decode_int_tuple! { Tup6u32,  U32, u32, 6, store.list, state },
                    Kind::Tup7u32   => string_decode_int_tuple! { Tup7u32,  U32, u32, 7, store.list, state },
                    Kind::Tup8u32   => string_decode_int_tuple! { Tup8u32,  U32, u32, 8, store.list, state },
                    Kind::Tup9u32   => string_decode_int_tuple! { Tup9u32,  U32, u32, 9, store.list, state },
                    Kind::Tup10u32  => string_decode_int_tuple! { Tup10u32, U32, u32, 10, store.list, state },

                    Kind::Tup2u64   => string_decode_int_tuple! { Tup2u64,  U64, u64, 2, store.list, state },
                    Kind::Tup3u64   => string_decode_int_tuple! { Tup3u64,  U64, u64, 3, store.list, state },
                    Kind::Tup4u64   => string_decode_int_tuple! { Tup4u64,  U64, u64, 4, store.list, state },
                    Kind::Tup5u64   => string_decode_int_tuple! { Tup5u64,  U64, u64, 5, store.list, state },
                    Kind::Tup6u64   => string_decode_int_tuple! { Tup6u64,  U64, u64, 6, store.list, state },
                    Kind::Tup7u64   => string_decode_int_tuple! { Tup7u64,  U64, u64, 7, store.list, state },
                    Kind::Tup8u64   => string_decode_int_tuple! { Tup8u64,  U64, u64, 8, store.list, state },
                    Kind::Tup9u64   => string_decode_int_tuple! { Tup9u64,  U64, u64, 9, store.list, state },
                    Kind::Tup10u64  => string_decode_int_tuple! { Tup10u64, U64, u64, 10, store.list, state },
                    Kind::Tup2i8    => string_decode_int_tuple! { Tup2i8,   I8,  i8,  2, store.list, state },
                    Kind::Tup3i8    => string_decode_int_tuple! { Tup3i8,   I8,  i8,  3, store.list, state },
                    Kind::Tup4i8    => string_decode_int_tuple! { Tup4i8,   I8,  i8,  4, store.list, state },
                    Kind::Tup5i8    => string_decode_int_tuple! { Tup5i8,   I8,  i8,  5, store.list, state },
                    Kind::Tup6i8    => string_decode_int_tuple! { Tup6i8,   I8,  i8,  6, store.list, state },
                    Kind::Tup7i8    => string_decode_int_tuple! { Tup7i8,   I8,  i8,  7, store.list, state },
                    Kind::Tup8i8    => string_decode_int_tuple! { Tup8i8,   I8,  i8,  8, store.list, state },
                    Kind::Tup9i8    => string_decode_int_tuple! { Tup9i8,   I8,  i8,  9, store.list, state },
                    Kind::Tup10i8   => string_decode_int_tuple! { Tup10i8,  I8,  i8,  10, store.list, state },
                    Kind::Tup2i16   => string_decode_int_tuple! { Tup2i16,  I16, i16, 2, store.list, state },
                    Kind::Tup3i16   => string_decode_int_tuple! { Tup3i16,  I16, i16, 3, store.list, state },
                    Kind::Tup4i16   => string_decode_int_tuple! { Tup4i16,  I16, i16, 4, store.list, state },
                    Kind::Tup5i16   => string_decode_int_tuple! { Tup5i16,  I16, i16, 5, store.list, state },
                    Kind::Tup6i16   => string_decode_int_tuple! { Tup6i16,  I16, i16, 6, store.list, state },
                    Kind::Tup7i16   => string_decode_int_tuple! { Tup7i16,  I16, i16, 7, store.list, state },
                    Kind::Tup8i16   => string_decode_int_tuple! { Tup8i16,  I16, i16, 8, store.list, state },
                    Kind::Tup9i16   => string_decode_int_tuple! { Tup9i16,  I16, i16, 9, store.list, state },
                    Kind::Tup10i16  => string_decode_int_tuple! { Tup10i16, I16, i16, 10, store.list, state },
                    Kind::Tup2i32   => string_decode_int_tuple! { Tup2i32,  I32, i32, 2, store.list, state },
                    Kind::Tup3i32   => string_decode_int_tuple! { Tup3i32,  I32, i32, 3, store.list, state },
                    Kind::Tup4i32   => string_decode_int_tuple! { Tup4i32,  I32, i32, 4, store.list, state },
                    Kind::Tup5i32   => string_decode_int_tuple! { Tup5i32,  I32, i32, 5, store.list, state },
                    Kind::Tup6i32   => string_decode_int_tuple! { Tup6i32,  I32, i32, 6, store.list, state },
                    Kind::Tup7i32   => string_decode_int_tuple! { Tup7i32,  I32, i32, 7, store.list, state },
                    Kind::Tup8i32   => string_decode_int_tuple! { Tup8i32,  I32, i32, 8, store.list, state },
                    Kind::Tup9i32   => string_decode_int_tuple! { Tup9i32,  I32, i32, 9, store.list, state },
                    Kind::Tup10i32  => string_decode_int_tuple! { Tup10i32, I32, i32, 10, store.list, state },
                    Kind::Tup2i64   => string_decode_int_tuple! { Tup2i64,  I64, i64, 2, store.list, state },
                    Kind::Tup3i64   => string_decode_int_tuple! { Tup3i64,  I64, i64, 3, store.list, state },
                    Kind::Tup4i64   => string_decode_int_tuple! { Tup4i64,  I64, i64, 4, store.list, state },
                    Kind::Tup5i64   => string_decode_int_tuple! { Tup5i64,  I64, i64, 5, store.list, state },
                    Kind::Tup6i64   => string_decode_int_tuple! { Tup6i64,  I64, i64, 6, store.list, state },
                    Kind::Tup7i64   => string_decode_int_tuple! { Tup7i64,  I64, i64, 7, store.list, state },
                    Kind::Tup8i64   => string_decode_int_tuple! { Tup8i64,  I64, i64, 8, store.list, state },
                    Kind::Tup9i64   => string_decode_int_tuple! { Tup9i64,  I64, i64, 9, store.list, state },
                    Kind::Tup10i64  => string_decode_int_tuple! { Tup10i64, I64, i64, 10, store.list, state },

                    _ => {
                        return Err(err!(
                            "Unexpected outer kind {:?} case while concluding {:?}.",
                            state.kind_outer, state.molecular_capture;
                        Input, Mismatch, Unexpected, Bug));
                    }
                }
            }
            _ => {
                return Err(err!(
                    "List capture was not actived with a '[' character ({})",
                    cursor.borrow();
                String, Input, Decode, Invalid));
            }
        }
    }

    /// Concludes a `:`, taking the daticle just gathered as the key of a map entry.
    #[inline(never)]
    fn colon(
        state:  &DecoderState,
        store:  &mut DecoderStore,
        cursor: &RefCell<Cursor>,
    )
        -> Outcome<()>
    {
        match state.molecular_capture {
            None => return Err(err!(
                    "Map capture not active ({})", cursor.borrow();
            String, Input, Decode, Invalid)),
            Some(MolecularCapture::ListMixed)   |
            Some(MolecularCapture::ListSame)    |
            Some(MolecularCapture::Bytes)       => {
                return Err(err!(
                    "List capture active, incompatible character ({})", cursor.borrow();
                String, Input, Decode, Invalid));
            }
            Some(MolecularCapture::Map) => {
                // We're expecting to have a daticle to add to the store.map.
                let mut dat = match store.val_opt.take() {
                    Some(dat) => dat,
                    None => {
                        if store.slurp.has_content() {
                            res!(Self::process_atom(&mut store.slurp, &Kind::Unknown))
                        } else {
                            Dat::Empty
                        }
                    }
                };
                if store.comment.len() > 0 {
                    dat = Dat::ABox(
                        store.note_config.extract(),
                        Box::new(dat),
                        store.comment.extract(),
                    );
                }
                if store.key_opt == None {
                    store.key_opt = Some(dat);
                    store.val_opt = None;
                }
                store.slurp = Slurp::new();
            }
        }
        Ok(())
    }

    /// Concludes a `}`, completing the map at this level.
    #[inline(never)]
    fn close_brace<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        cfg:        &DecoderConfig<M1, M2>,
        state:      &DecoderState,
        mut store:  DecoderStore,
        cursor:     &RefCell<Cursor>,
    )
        -> Outcome<Self>
    {
        if state.molecular_capture == Some(MolecularCapture::Map) {
            if store.slurp.has_content() {
                store.val_opt =
                    Some(res!(Self::process_atom(&mut store.slurp, &state.kind_outer)));
            }
            match (store.key_opt.take(), store.val_opt.take()) {
                (Some(key), Some(mut dat)) => {
                    // The key has already been abox-wrapped if necessary.
                    if store.comment.len() > 0 {
                        dat = Dat::ABox(
                            store.note_config.extract(),
                            Box::new(dat),
                            store.comment.extract(),
                        );
                    }
                    res!(Self::map_insert(
                        state.kind_outer == Kind::OrdMap,
                        &mut store,
                        (key, dat),
                    ));
                }
                (None, Some(dat)) => {
                    return Err(err!(
                        "Unpaired value {:?} at end of store.map {} ({})",
                        dat, match cfg.use_ordmaps {
                            true => fmt!("{:?}", store.ordmap),
                            false => fmt!("{:?}", store.map),
                        }, cursor.borrow();
                    String, Input, Decode, Invalid, Missing));
                }
                (Some(key), None) => {
                    if store.comment.len() > 0 {
                        let dat = Dat::ABox(
                            store.note_config.extract(),
                            Box::new(Dat::Empty),
                            store.comment.extract(),
                        );
                        res!(Self::map_insert(
                            state.kind_outer == Kind::OrdMap,
                            &mut store,
                            (key, dat),
                        ));
                    } else {
                        return Err(err!(
                            "Unpaired key {:?} at end of store.map {} ({})",
                            key, match cfg.use_ordmaps {
                                true => fmt!("{:?}", store.ordmap),
                                false => fmt!("{:?}", store.map),
                            }, cursor.borrow();
                        String, Input, Decode, Invalid, Missing));
                    }
                }
                (None, None) => {}
            }
            return Ok(if state.kind_outer == Kind::OrdMap {
                Dat::OrdMap(store.ordmap)
            } else {
                Dat::Map(store.map)
            });
        }
        Err(err!(
            "Map capture was not actived with a '{{' character ({})", cursor.borrow();
        String, Input, Decode, Invalid))
    }

    #[inline(never)]
    fn comma_handler<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        cfg:        &DecoderConfig<M1, M2>,
        state:      &mut DecoderState,
        cursor:     &RefCell<Cursor>,
        mut store:  &mut DecoderStore,
    )
        -> Outcome<()>
    {
        match state.molecular_capture {
            None => {
                return Err(err!(
                    "List or map capture not active ({})", cursor.borrow();
                String, Input, Decode, Invalid));
            }
            Some(MolecularCapture::Bytes) => {
                // We're expecting a byte to add to the list.
                let n = try_extract_dat!(res!(Self::process_atom(&mut store.slurp, &Kind::U8)), U8);
                store.byts.push(n);
                store.slurp = Slurp::new();
            }
            Some(MolecularCapture::ListSame) => {
                // We're expecting a daticle to add to the list.
                let kind_same = MolecularCapture::same_kind(&state.kind_outer);
                let dat = match store.val_opt.take() {
                    Some(dat) => dat, // store.dt_opt is now None.
                    None => res!(Self::process_atom(&mut store.slurp, &kind_same)),
                };
                store.list.push(dat);
                store.slurp = Slurp::new();
            }
            Some(MolecularCapture::ListMixed) => {
                // We're expecting a daticle to add to the list.
                let dat = match store.val_opt.take() { // store.val_opt is now None.
                    Some(dat) => dat, 
                    None => {
                        if store.slurp.has_content() {
                            res!(Self::process_atom(&mut store.slurp, &state.kind_outer))
                        } else {
                            Dat::Empty
                        }
                    }
                };
                if store.comment.len() > 0 {
                    store.list.push(Dat::ABox(
                        store.note_config.extract(),
                        Box::new(dat),
                        store.comment.extract(),
                    ));
                } else {
                    store.list.push(dat);
                }
                store.slurp = Slurp::new();
            }
            Some(MolecularCapture::Map) => {
                // We're expecting a daticle to add to the map.
                let mut dat = match store.val_opt.take() {
                    Some(dat) => dat, // store.val_opt is now None.
                    None => {
                        let dat = if store.slurp.has_content() {
                            res!(Self::process_atom(&mut store.slurp, &Kind::Unknown))
                        } else {
                            Dat::Empty
                        };
                        store.val_opt = Some(dat.clone());
                        dat
                    }
                };
                if store.comment.len() > 0 {
                    dat = Dat::ABox(
                        store.note_config.extract(),
                        Box::new(dat),
                        store.comment.extract(),
                    );
                    store.val_opt = Some(dat.clone());
                }
                match store.key_opt.take() {
                    Some(key) => { // store.key_opt is now None.
                        let key = match key {
                            Dat::Str(s) if s.len() == 0 =>
                                res!(cfg.default_key.rand().to_dat()),
                            _ => key,
                        };
                        res!(Self::map_insert(
                            state.kind_outer == Kind::OrdMap,
                            &mut store,
                            (key, dat),
                        ));
                    }
                    None => {
                        store.key_opt = store.val_opt.take();
                    }
                }
                store.val_opt = None;
                store.slurp = Slurp::new();
            }
        }
        Ok(())
    }
        
    /// `Dat::OrdMap` allows map key ordering to be preserved, but the trade off is an additional
    /// search prior to insertion to ensure uniqueness of the key daticle.  This will become
    /// costly as the map size grows.
    #[inline(never)]
    fn map_insert(
        use_ordmap: bool,
        store:      &mut DecoderStore,
        (key, dat): (Dat, Dat),
    )
        -> Outcome<()>
    {
        match use_ordmap {
            true => {
                if store.ordmap.iter().any(|(mk, _)| *mk.dat() == key) {
                    return Err(err!(
                        "The key {:?} already exists in the {:?} being \
                        interpreted from the given string.", key, store.ordmap;
                    Invalid, Input, Exists));
                }
                let mkey = MapKey::new(store.map_count.order(), key);
                store.ordmap.insert(mkey, dat);
                res!(store.map_count.inc_all());
            }
            false => {
                if store.map.contains_key(&key) {
                    return Err(err!(
                        "The key {:?} already exists in the {:?} being \
                        interpreted from the given string.", key, store.map;
                    Invalid, Input, Exists));
                } else {
                    store.map.insert(key, dat);
                    res!(store.map_count.inc());
                }
            }
        }
        Ok(())
    }

    /// Advance the string-escape state machine by one character.
    ///
    /// Called from the top of the decoder's outer loop whenever
    /// we are inside a quoted string. Returns `Ok(true)` if the
    /// character was consumed by the escape state machine and
    /// the outer loop should `continue`; returns `Ok(false)` if
    /// the character is an ordinary literal character and the
    /// outer loop should fall through to its normal handling
    /// (including the `"` / `'` string-terminator branches).
    ///
    /// Decodes RFC 8259 §7 escapes:
    ///
    /// * `\"` `\\` `\/` `\b` `\f` `\n` `\r` `\t` -- one-char
    ///   translations, pushed into the slurp.
    /// * `\uXXXX` -- four hex digits forming a 16-bit code unit.
    ///   A BMP code point is pushed directly; a high surrogate
    ///   must be followed immediately by `\uYYYY` with a low
    ///   surrogate, which is combined into a supplementary
    ///   plane code point; a bare low surrogate is a decode
    ///   error.
    #[inline(never)]
    fn handle_string_escape(
        c:      char,
        state:  &mut DecoderState,
        slurp:  &mut Slurp,
    )
        -> Outcome<bool>
    {
        use StringEscape::*;
        match state.string_escape.clone() {
            None => {
                if c == '\\' {
                    state.string_escape = Backslash;
                    // Ensure the slurp is marked as a string
                    // even when the whole payload is escapes
                    // (e.g. `"\n"` would otherwise look like a
                    // zero-length non-string slurp).
                    slurp.flag_as_string();
                    return Ok(true);
                }
                Ok(false)
            }
            Backslash => {
                let translated = match c {
                    '"'     => '"',
                    '\\'    => '\\',
                    '/'     => '/',
                    'b'     => '\u{0008}',
                    'f'     => '\u{000C}',
                    'n'     => '\n',
                    'r'     => '\r',
                    't'     => '\t',
                    'u'     => {
                        state.string_escape = Unicode { digits: 0, acc: 0 };
                        return Ok(true);
                    }
                    _ => return Err(err!(
                        "Invalid string escape sequence '\\{}' \
                        inside quoted string. Expected one of \
                        `\"`, `\\`, `/`, `b`, `f`, `n`, `r`, `t`, \
                        `u`.", c;
                        String, Input, Decode, Invalid)),
                };
                slurp.push(translated);
                state.string_escape = None;
                Ok(true)
            }
            Unicode { digits, acc } => {
                let hex = res!(Self::hex_digit_value(c));
                let new_acc = (acc << 4) | hex;
                let new_digits = digits + 1;
                if new_digits < 4 {
                    state.string_escape = Unicode {
                        digits: new_digits,
                        acc:    new_acc,
                    };
                    return Ok(true);
                }
                // Four digits collected. Decide what to do with
                // the 16-bit value.
                let code_unit = new_acc as u16;
                if (0xD800..=0xDBFF).contains(&code_unit) {
                    // High surrogate -- must be followed by a
                    // low surrogate `\uYYYY`.
                    state.string_escape = SurrogateBackslash { high: code_unit };
                    return Ok(true);
                }
                if (0xDC00..=0xDFFF).contains(&code_unit) {
                    return Err(err!(
                        "Bare low surrogate U+{:04X} in \\uXXXX \
                        escape without a preceding high \
                        surrogate.", code_unit;
                        String, Input, Decode, Invalid));
                }
                // BMP code point, push directly.
                match char::from_u32(new_acc) {
                    Some(ch) => {
                        slurp.push(ch);
                        state.string_escape = None;
                        Ok(true)
                    }
                    Option::None => Err(err!(
                        "Invalid Unicode code point U+{:04X} in \
                        \\uXXXX escape.", new_acc;
                        String, Input, Decode, Invalid)),
                }
            }
            SurrogateBackslash { high } => {
                if c != '\\' {
                    return Err(err!(
                        "High surrogate U+{:04X} not followed by \
                        a `\\` to start the low-surrogate escape \
                        (saw {:?}).", high, c;
                        String, Input, Decode, Invalid));
                }
                state.string_escape = SurrogateU { high };
                Ok(true)
            }
            SurrogateU { high } => {
                if c != 'u' {
                    return Err(err!(
                        "High surrogate U+{:04X} not followed by \
                        a `\\u` to start the low-surrogate escape \
                        (saw \\{:?}).", high, c;
                        String, Input, Decode, Invalid));
                }
                state.string_escape = LowSurrogate {
                    digits: 0,
                    acc:    0,
                    high,
                };
                Ok(true)
            }
            LowSurrogate { digits, acc, high } => {
                let hex = res!(Self::hex_digit_value(c));
                let new_acc = (acc << 4) | hex;
                let new_digits = digits + 1;
                if new_digits < 4 {
                    state.string_escape = LowSurrogate {
                        digits: new_digits,
                        acc:    new_acc,
                        high,
                    };
                    return Ok(true);
                }
                let low = new_acc as u16;
                if !(0xDC00..=0xDFFF).contains(&low) {
                    return Err(err!(
                        "Expected a low surrogate (U+DC00..U+DFFF) \
                        after high surrogate U+{:04X}, got U+{:04X}.",
                        high, low;
                        String, Input, Decode, Invalid));
                }
                // Combine the surrogate pair into a
                // supplementary plane code point.
                let h = high as u32;
                let l = low as u32;
                let cp = 0x10000 + ((h - 0xD800) << 10) + (l - 0xDC00);
                match char::from_u32(cp) {
                    Some(ch) => {
                        slurp.push(ch);
                        state.string_escape = None;
                        Ok(true)
                    }
                    Option::None => Err(err!(
                        "Surrogate pair U+{:04X} U+{:04X} does \
                        not form a valid code point (U+{:06X}).",
                        high, low, cp;
                        String, Input, Decode, Invalid)),
                }
            }
        }
    }

    /// Convert a single ASCII hex digit to its numeric value.
    /// Rejects anything else with a clear error message tagged
    /// for the decoder's error chain.
    fn hex_digit_value(c: char) -> Outcome<u32> {
        match c {
            '0'..='9'   => Ok((c as u32) - ('0' as u32)),
            'a'..='f'   => Ok((c as u32) - ('a' as u32) + 10),
            'A'..='F'   => Ok((c as u32) - ('A' as u32) + 10),
            _ => Err(err!(
                "Invalid hex digit '{}' in \\uXXXX escape \
                sequence.", c;
                String, Input, Decode, Invalid)),
        }
    }

    /// Process a [`Slurp`] extracted during single-pass processing.  Numbers currently undergo an
    /// additional pass via validation in [`NumberString::new`], to cope with the various forms of
    /// representation, even when typed (e.g. different radices).
    #[inline(never)]
    fn process_atom(
        slurp: &mut Slurp,
        kind: &Kind,
    )
        -> Outcome<Self>
    {
        let d = match kind {
            // If there is no explicit kind, and it's in quotes, it's a string.
            Kind::Unknown if slurp.is_string() => Dat::Str(slurp.clone_string()),
            // If it's not quoted and nothing was captured, it is empty.
            Kind::Unknown if (!slurp.is_string() && slurp.len() == 0) => Dat::Empty,
            // If the kind is declared a STR, even without quotes, it's a string.
            Kind::Str => Dat::Str(slurp.clone_string()),
            // These kinds don't need data.
            Kind::Empty => Dat::Empty,
            Kind::True => Dat::Bool(true),
            Kind::False => Dat::Bool(false),
            Kind::None => Dat::Opt(Box::new(None)),
            Kind::BU8   |
            Kind::BU16  |
            Kind::BU32  |
            Kind::BU64  |
            Kind::BC64 |
            Kind::B2    |
            Kind::B3    |
            Kind::B4    |
            Kind::B5    |
            Kind::B6    |
            Kind::B7    |
            Kind::B8    |
            Kind::B9    |
            Kind::B10   |
            Kind::B16   |
            Kind::B32 if slurp.is_string() => {
                // Interpret as base2x, which are quoted strings.
                let byts = res!(base2x::HEMATITE64.from_str(slurp.get_str()));
                res!(Self::decode_bytes(byts, kind))
            }
            _ => if slurp.is_string() {
                if kind.case().accepts_strings() {
                    Dat::Str(slurp.clone_string())
                } else {
                    return Err(err!(
                        "A quoted string '{}' is not compatible with {:?}.", slurp, kind;
                    String, Input, Decode, Mismatch));
                }
            } else {
                if slurp.len() == 0 {
                    return Err(err!(
                        "The kind {:?} requires data, but none was found.", kind;
                    String, Input, Decode, Missing));
                } else {
                    // We allow a limited number of keywords that can be used a shorthand for kinds, with or
                    // without quotes.
                    match Kind::from_str(&slurp.clone_string()) {
                        Ok(Kind::False) => Dat::Bool(false),
                        Ok(Kind::True) => Dat::Bool(true),
                        Ok(Kind::Empty) => Dat::Empty,
                        Ok(Kind::None) => Dat::Opt(Box::new(None)),
                        _ => match NumberString::new(slurp.clone_string()) {
                            // Anything else, let's see if it's a number.
                            Ok(ns) => {
                                if kind.is_number() { 
                                    // Decode as a number when the kind is specified.
                                    res!(kind.clone().decode_number(ns))
                                } else {
                                    if ns.is_zero() {
                                        Dat::U8(0)
                                    } else {
                                        if ns.has_point() || ns.has_exp() {
                                            Dat::Adec(res!(ns.as_bigdecimal()))
                                        } else {
                                            // Interpret the number as the smallest possible
                                            // native integer, otherwise use an Aint.
                                            match u128::from_str_radix(ns.abs_integer_str(), ns.radix()) {
                                                Ok(nu128) => {
                                                    // n is a member of the set -u128::MAX..u128::MAX
                                                    // i.e. n E -u128::MAX..u128::MAX
                                                    if ns.is_negative() {
                                                        if nu128 > (i128::MAX as u128 + 1) {
                                                            // n E -u128::MAX..i128::MIN, can only be
                                                            // represented by a BigInt.
                                                            match ns.as_bigint() {
                                                                Ok(n) => Dat::Aint(n),
                                                                Err(e) => return Err(err!(e,
                                                                    "While interpreting '{}' with outer kind {:?}.",
                                                                    slurp, kind;
                                                                String, Input, Decode)),
                                                            }
                                                        } else {
                                                            // n E i128::MIN..0
                                                            // But the cast (nu128 as i128) works
                                                            // differently when nu128 =
                                                            // |i128::MIN|.  In this case, Rust
                                                            // recognises that nu128 could not have
                                                            // been positive, and automatically
                                                            // negates the result.  For all other
                                                            // values, the cast is positive and we
                                                            // need to negate manually.
                                                            let n = if nu128 == i128::MAX as u128 + 1 {
                                                                nu128 as i128
                                                            } else {
                                                                -(nu128 as i128)
                                                            };
                                                            res!(DatInt::from(n).min_size().to_dat())
                                                        }
                                                    } else {
                                                        // n E 0..u128::MAX
                                                        res!(DatInt::from(nu128).min_size().to_dat())
                                                    }
                                                }
                                                Err(_e) => {
                                                    match ns.as_bigint() {
                                                        Ok(n) => Dat::Aint(n),
                                                        Err(e) => return Err(err!(e,
                                                            "While interpreting '{}' with outer kind {:?}.",
                                                            slurp, kind;
                                                        String, Input, Decode)),
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => if *kind == Kind::Unknown {
                                Dat::Str(slurp.take_string())
                            } else {
                                return Err(err!(e,
                                    "While interpreting '{}' with outer kind {:?}.",
                                    slurp, kind;
                                String, Input, Decode));
                            }
                        }
                    }
                }
            }
        };

        slurp.reset();
        Ok(d)
    }

    #[inline(never)]
    fn decode_bytes(v: Vec<u8>, k: &Kind) -> Outcome<Self> {
        Ok(match k {
            Kind::BU8   => Self::BU8(v),
            Kind::BU16  => Self::BU16(v),
            Kind::BU32  => Self::BU32(v),
            Kind::BU64  => Self::BU64(v),
            Kind::BC64  => Self::BC64(v),
            Kind::B2    => Self::B2(res!(<[u8; 2]>::try_from(&v[..]), Decode, Bytes)),
            Kind::B3    => Self::B3(res!(<[u8; 3]>::try_from(&v[..]), Decode, Bytes)),
            Kind::B4    => Self::B4(res!(<[u8; 4]>::try_from(&v[..]), Decode, Bytes)),
            Kind::B5    => Self::B5(res!(<[u8; 5]>::try_from(&v[..]), Decode, Bytes)),
            Kind::B6    => Self::B6(res!(<[u8; 6]>::try_from(&v[..]), Decode, Bytes)),
            Kind::B7    => Self::B7(res!(<[u8; 7]>::try_from(&v[..]), Decode, Bytes)),
            Kind::B8    => Self::B8(res!(<[u8; 8]>::try_from(&v[..]), Decode, Bytes)),
            Kind::B9    => Self::B9(res!(<[u8; 9]>::try_from(&v[..]), Decode, Bytes)),
            Kind::B10   => Self::B10(res!(<[u8; 10]>::try_from(&v[..]), Decode, Bytes)),
            Kind::B16   => Self::B16(res!(<[u8; 16]>::try_from(&v[..]), Decode, Bytes)),
            Kind::B32   => Self::B32(res!(B32::from_bytes(&v[..])).0),
            _ => return Err(err!(
                "{:?} is not suitable for bytes.", k;
            Input, Mismatch, Bug)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::thread;

    /// The stack Rust gives a spawned thread, and the stack the default depth limit is sized
    /// against.
    const SPAWNED_THREAD_STACK: usize = 2 * 1024 * 1024;

    /// Nests `levels` lists around nothing, e.g. `[[[]]]`, which the decoder reads at depth
    /// `levels + 1`, the root sitting at depth 1.
    ///
    /// This is the bomb: a few characters per level, describing a nesting deep enough to exhaust
    /// the stack of whoever reads it.  It is written rather than encoded, since the encoder recurses
    /// as the decoder does, and an attacker is under no obligation to use it.
    fn nested_lists(levels: usize) -> String {
        let mut s = String::with_capacity(2 * levels);
        for _ in 0..levels {
            s.push('[');
        }
        for _ in 0..levels {
            s.push(']');
        }
        s
    }

    /// Nests `levels` maps around a scalar, e.g. `{a:{a:1}}`, which the decoder reads at depth
    /// `levels`, since the outermost brace opens the root rather than a level below it.
    fn nested_maps(levels: usize) -> String {
        let mut s = String::with_capacity(4 * levels);
        for _ in 0..levels {
            s.push_str("{a:");
        }
        s.push('1');
        for _ in 0..levels {
            s.push('}');
        }
        s
    }

    /// Decodes on a thread with the stack a spawned thread gets, which is the stack the default
    /// limit is sized against.
    ///
    /// A decoder that overflows this stack aborts the process, taking the whole test run with it,
    /// which is precisely the failure these tests exist to catch: the abort cannot be caught, so it
    /// must not happen.
    fn decode_on_a_small_stack(s: String) -> Outcome<Outcome<Dat>> {
        let builder = thread::Builder::new().stack_size(SPAWNED_THREAD_STACK);
        let handle = match builder.spawn(move || Dat::decode_string(s)) {
            Ok(handle) => handle,
            Err(e) => return Err(err!(e, "While spawning the decoding thread."; Test, Init)),
        };
        match handle.join() {
            Ok(result) => Ok(result),
            Err(_) => Err(err!(
                "The decoding thread died rather than returning, which is what a stack overflow \
                looks like.";
            Test, Invalid)),
        }
    }

    #[test]
    fn test_text_depth_limit_accepts_at_the_limit() -> Outcome<()> {
        // A leaf inside 15 lists sits at depth 16.
        const LEVELS: usize = 15;
        let lims = DecodeLimits::text().with_max_depth(LEVELS + 1);
        let dat = res!(Dat::decode_string_limited(nested_lists(LEVELS), &lims));
        let mut expected = Dat::List(Vec::new());
        for _ in 1..LEVELS {
            expected = Dat::List(vec![expected]);
        }
        assert_eq!(dat, expected);
        Ok(())
    }

    #[test]
    fn test_text_depth_limit_refuses_past_the_limit() -> Outcome<()> {
        // The same nesting, one level shallower than it needs, is refused rather than decoded.
        const LEVELS: usize = 15;
        let lims = DecodeLimits::text().with_max_depth(LEVELS);
        match Dat::decode_string_limited(nested_lists(LEVELS), &lims) {
            Ok(dat) => Err(err!(
                "Expected a depth limit error, but decoded {:?}.", dat;
            Test, Invalid)),
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains("depth"), "Error should name the depth limit: {}", msg);
                assert!(msg.contains("offset"), "Error should name the offset: {}", msg);
                Ok(())
            }
        }
    }

    #[test]
    fn test_text_list_bomb_is_refused() -> Outcome<()> {
        // A hostile document: 100,000 nested lists, 200 kB of text, which would exhaust the stack
        // of a decoder that trusted it.  The decode must return an error, on a stack that a stack
        // overflow would abort.
        match res!(decode_on_a_small_stack(nested_lists(100_000))) {
            Ok(_) => Err(err!(
                "A nesting of 100,000 lists should be refused by the default limits.";
            Test, Invalid)),
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains("depth"), "Error should name the depth limit: {}", msg);
                Ok(())
            }
        }
    }

    #[test]
    fn test_text_map_bomb_is_refused() -> Outcome<()> {
        // The same attack through the other bracket.
        match res!(decode_on_a_small_stack(nested_maps(100_000))) {
            Ok(_) => Err(err!(
                "A nesting of 100,000 maps should be refused by the default limits.";
            Test, Invalid)),
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains("depth"), "Error should name the depth limit: {}", msg);
                Ok(())
            }
        }
    }

    #[test]
    fn test_text_at_the_default_limit_decodes_on_a_small_stack() -> Outcome<()> {
        // The limit is only worth having if everything under it is safe.  A document nested as deep
        // as the default allows must decode on the stack the default is sized against, rather than
        // aborting just short of the error the decoder promises.
        let levels = DecodeLimits::DEFAULT_MAX_TEXT_DEPTH - 1;
        let dat = res!(res!(decode_on_a_small_stack(nested_lists(levels))));
        let mut depth = 0;
        let mut node = &dat;
        while let Dat::List(items) = node {
            depth += 1;
            match items.first() {
                Some(item) => node = item,
                None => break,
            }
        }
        assert_eq!(depth, levels, "The decoded nesting is not the nesting that was given.");
        Ok(())
    }

    #[test]
    fn test_text_byte_limit_refuses_an_oversized_input() -> Outcome<()> {
        let lims = DecodeLimits::text().with_max_bytes(8);
        match Dat::decode_string_limited("\"a string longer than a tiny limit\"", &lims) {
            Ok(dat) => Err(err!(
                "Expected a byte limit error, but decoded {:?}.", dat;
            Test, Invalid)),
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains("8 bytes"), "Error should name the byte limit: {}", msg);
                Ok(())
            }
        }
    }

    #[test]
    fn test_ordinary_documents_still_decode() -> Outcome<()> {
        // The limits are no use if they cost the decoder its day job.
        let txt = r#"{
            "name": "Hematite",
            "version": (u16|3),
            "tags": ["a", "b"],
            "nested": {"n": [1, 2, 3]},
            "pair": (t2|[1, 2]),
            "opt": (some|(u8|7)),
        }"#;
        let expected = mapdat!{
            dat!("name")    => dat!("Hematite"),
            dat!("version") => dat!(3u16),
            dat!("tags")    => listdat![dat!("a"), dat!("b")],
            dat!("nested")  => mapdat!{
                dat!("n") => listdat![dat!(1u8), dat!(2u8), dat!(3u8)],
            },
            dat!("pair")    => Dat::Tup2(Box::new([dat!(1u8), dat!(2u8)])),
            dat!("opt")     => Dat::Opt(Box::new(Some(dat!(7u8)))),
        };
        assert_eq!(res!(Dat::decode_string(txt)), expected);
        Ok(())
    }

    #[test]
    fn test_limits_can_be_tightened_by_the_caller() -> Outcome<()> {
        // A caller with an opinion about depth states it, and the decoder holds them to it.
        let cfg = DecoderConfig::<
            BTreeMap<UsrKindCode, UsrKind>,
            BTreeMap<String, UsrKindId>,
        >::default().with_limits(DecodeLimits::text().with_max_depth(2));
        assert!(Dat::decode_string_with_config("[1]", &cfg).is_ok());
        assert!(Dat::decode_string_with_config("[[1]]", &cfg).is_err());
        Ok(())
    }
}
