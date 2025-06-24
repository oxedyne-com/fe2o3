//! 12024-12-23
//! Added implicit tuple string decoding for round brackets, e.g. explicit (tup2|[1,2]), implicit
//! (1,2).  This has no effect on tuple string encoding, which continues to use square brackets
//! either with a kindicle or without, e.g. explicit (tup2[1,2]) implicit [1,2].  The latter will
//! continue to be decoded as a list.
//!
use crate::{
    prelude::*,
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

#[derive(Clone, Debug, Default)]
pub struct DecoderState {
    // Data
    //pub cursor:             Cursor,
    pub kind_outer:         Kind,
    // Switches
    pub explicit_kind:      bool, // The kind was defined explicitly.
    pub quote_protection:   Quote, // "a(' &{}" etc
    pub comment_capture:    Option<CommentCapture>,
    pub number_capture:     bool, // 23_786.345 contiguous
    pub kind_capture:       bool, // (k|
    pub atomic_capture:     bool,
    pub molecular_capture:  Option<MolecularCapture>,
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
    pub fn recurse(&self) -> Self {
        Self {
            kind_outer: self.kind_outer.clone(),
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

impl MolecularCapture {
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
            Kind::Tup10u64  => Some(MolecularCapture::ListSame),
            _ => None,
        }
    }

    pub fn same_kind(kind: &Kind) -> Kind {
        match kind {
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
            Self::Tup10u64  => true,
            _ => false,
        }
    }
}

impl Dat {

    pub fn decode_string<
        S: Into<String>,
    >(
        s: S,
    )
        -> Outcome<Self>
    {
        // We want to take ownership of the String
        let mut iter = s.into().chars().collect::<Vec<_>>().into_iter().enumerate();

        let dec_cfg = DecoderConfig::<
            BTreeMap<UsrKindCode, UsrKind>,
            BTreeMap<String, UsrKindId>,
        >::default();

        Self::recursive_decode(
            &mut iter,
            &dec_cfg,
            DecoderState::default(),
            &RefCell::new(Cursor::default()),
        )
    }

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
        let mut iter = s.into().chars().collect::<Vec<_>>().into_iter().enumerate();
        Self::recursive_decode(
            &mut iter,
            cfg,
            DecoderState::default(),
            &RefCell::new(Cursor::default()),
        )
    }

    /// A recursive text processor, that aims to interpret in a single pass, while
    /// performing a second pass over numbers.
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
    #[allow(unused_assignments)]
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
        // Switches.
        let mut comment_required = false; 
        // Store.
        let mut store = DecoderStore::new(cfg);

        state.molecular_capture = MolecularCapture::from_kind(&state.kind_outer);

        while let Some((i, c)) = iter.next() {
            {
                cursor.borrow_mut().advance(c, state.quote_protection != Quote::None);
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
                            continue;
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
                            continue;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(capturing) = &state.comment_capture {
                // Finish comment capturing?
                if (
                    *capturing == CommentCapture::Type1
                    && (c == cfg.comment1_end_char || c == '\n')
                ) || (
                    *capturing == CommentCapture::Type2
                    && (c == cfg.comment2_end_char || c == '\n')
                ) {
                    state.comment_capture = None;
                    store.comment = store.comment.trim_start().to_string();
                    if c == '\n' {
                        if let Some(molecular_capture) = &state.molecular_capture {
                            let mut dat = match store.val_opt.take() {
                                Some(dat) => dat, // store.val_opt is now None.
                                None => {
                                    let dat = if store.slurp.len() > 0 {
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
                                                &mut store,
                                                (key, dat),
                                            ));
                                        }
                                        None => {
                                            res!(Self::map_insert(
                                                state.kind_outer == Kind::OrdMap,
                                                &mut store,
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
                    continue;
                }
                if cfg.comment_capture {
                    store.comment.push(c);
                }
                continue;
            }
            if cfg.comment_allowed && state.quote_protection == Quote::None {
                // Start comment capturing?
                if c == cfg.comment1_start_char {
                    state.comment_capture = Some(CommentCapture::Type1);
                    store.note_config = store.note_config.set_type1(true);
                    continue;
                }
                if c == cfg.comment2_start_char {
                    state.comment_capture = Some(CommentCapture::Type2);
                    store.note_config = store.note_config.set_type1(false);
                    continue;
                }
            }
            match c {
                '\t' => {
                    return Err(err!(
                        "Escaped tab characters are prohibited ({}).  Replace all tabs with spaces.",
                        cursor.borrow();
                    String, Input, Decode, Invalid));
                }
                ' ' | '\n' | '\r' => {
                    res!(store.slurp.char_slurped((i, c), &mut state));
                }
                '(' => {
                    if res!(store.slurp.char_slurped((i, c), &mut state)) {
                        continue;
                    }
                    if state.kind_outer == Kind::Unknown ||
                        state.kind_outer.case() == KindCase::MoleculeUnitary ||
                        state.molecular_capture != None
                    {
                        // Begin capturing the daticle kind (or "kindicle").
                        state.kind_capture = true;
                        store.slurp = Slurp::new();
                        continue;
                    } else {
                        // "(k1|(k2|v))"
                        //      ^ already expect k1 kind, invalid unless preceded by [ or {
                        match state.kind_outer.case() {
                            KindCase::MoleculeSame => {
                                return Err(err!(
                                    "Elements of a vector of kind {:?} are not daticles, \
                                    so no kind should be specified ({})",
                                    state.kind_outer, cursor.borrow();
                                String, Input, Decode, Invalid));
                            }
                            _ => {
                                return Err(err!(
                                    "The kind for the daticle has already been specified \
                                    as {:?} ({})", state.kind_outer, cursor.borrow();
                                String, Input, Decode, Invalid));
                            }
                        }
                    }
                }
                '|' => {
                    if state.quote_protection != Quote::None {
                        store.slurp.push(c);
                    } else if state.kind_capture {
                        // We have captured a kind, triggering a new level of recursion.
                        let kind_inner = res!(Kind::from_label(
                            &store.slurp.clone_string().to_lowercase(),
                            cfg.ukinds_opt.as_ref(),
                        ));
                        if kind_inner.case() == KindCase::AtomLogic {
                            // An unused '|' separator is not valid, e.g. (true|), (EMPTY|)
                            return Err(err!(
                                "The separation character \"|\" for {:?} is superfluous {}.",
                                kind_inner, cursor.borrow();
                            String, Input, Decode, Invalid));
                        }
                        state.kind_capture = false;
                        let mut new_state = state.recurse();
                        new_state.kind_outer = kind_inner;
                        new_state.explicit_kind = true;
                        let dat = res!(Self::recursive_decode(
                            &mut iter,
                            &cfg,
                            new_state,
                            cursor,
                        ));
                        store.slurp = Slurp::new();
                        let mut atomic = false;
                        if state.kind_outer == Kind::Unknown {
                            if state.molecular_capture == None {
                                atomic = true;
                            }
                        } else if state.kind_outer.case().class() == KindClass::Atomic {
                            atomic = true;
                        }
                        if atomic {
                            return Ok(dat);
                        }
                        store.val_opt = Some(dat);
                        // Otherwise, keep accumulating daticles...
                    }
                }
                ')' => {
                    // Deal with atoms (e.g. (FALSE)), which should only
                    // ever be processed within a recursion level.
                    if res!(store.slurp.char_slurped((i, c), &mut state)) {
                        continue;
                    }
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
                        if store.slurp.len() > 0 {
                            store.val_opt = Some(res!(Self::process_atom(&mut store.slurp, &kind)));
                        }
                        // Copied from ']' branch: (TODO rationalise code)
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

                        match store.list.len() {
                            0 => return Ok(Dat::Empty),
                            1 => return Ok(store.list[0].clone()),
                            2 => return Ok(Dat::Tup2(Box::new(
                                res!(store.list.try_into().map_err(|_| err!(
                                    "While decoding a 2-item tuple ({})", cursor.borrow();
                                String, Input, Decode, Invalid)))
                            ))),
                            3 => return Ok(Dat::Tup3(Box::new(
                                res!(store.list.try_into().map_err(|_| err!(
                                    "While decoding a 3-item tuple ({})", cursor.borrow();
                                String, Input, Decode, Invalid)))
                            ))),
                            4 => return Ok(Dat::Tup4(Box::new(
                                res!(store.list.try_into().map_err(|_| err!(
                                    "While decoding a 4-item tuple ({})", cursor.borrow();
                                String, Input, Decode, Invalid)))
                            ))),
                            5 => return Ok(Dat::Tup5(Box::new(
                                res!(store.list.try_into().map_err(|_| err!(
                                    "While decoding a 5-item tuple ({})", cursor.borrow();
                                String, Input, Decode, Invalid)))
                            ))),
                            6 => return Ok(Dat::Tup6(Box::new(
                                res!(store.list.try_into().map_err(|_| err!(
                                    "While decoding a 6-item tuple ({})", cursor.borrow();
                                String, Input, Decode, Invalid)))
                            ))),
                            7 => return Ok(Dat::Tup7(Box::new(
                                res!(store.list.try_into().map_err(|_| err!(
                                    "While decoding a 7-item tuple ({})", cursor.borrow();
                                String, Input, Decode, Invalid)))
                            ))),
                            8 => return Ok(Dat::Tup8(Box::new(
                                res!(store.list.try_into().map_err(|_| err!(
                                    "While decoding a 8-item tuple ({})", cursor.borrow();
                                String, Input, Decode, Invalid)))
                            ))),
                            9 => return Ok(Dat::Tup9(Box::new(
                                res!(store.list.try_into().map_err(|_| err!(
                                    "While decoding a 9-item tuple ({})", cursor.borrow();
                                String, Input, Decode, Invalid)))
                            ))),
                            10 => return Ok(Dat::Tup10(Box::new(
                                res!(store.list.try_into().map_err(|_| err!(
                                    "While decoding a 10-item tuple ({})", cursor.borrow();
                                String, Input, Decode, Invalid)))
                            ))),
                            n => return Err(err!(
                                "Tuples are limited to 10 items, {} found ({}).",
                                n, cursor.borrow();
                            String, Input, Decode, Invalid)),
                        }
                    };

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
                                    return Ok(Dat::ABox(
                                        store.note_config.clone(),
                                        Box::new(dat.clone()),
                                        store.comment,
                                    ));
                                } else {
                                    comment_required = true;
                                }
                            }
                            None => {
                                return Err(err!(
                                    "Daticle missing in Dat::ABox ({})", cursor.borrow();
                                String, Input, Decode, Invalid, Missing));
                            }
                        }
                    }
                    
                    if !comment_required && (
                        !kind.is_dataless() || state.molecular_capture == None
                    ) {
                        // Atom point cases were just dealt with, while molecular capture is
                        // ongoing until a `]` or `}` is encountered, with the exception of a
                        // base2x string for bytes.
                        if state.molecular_capture == None || (
                            state.molecular_capture == Some(MolecularCapture::Bytes)
                            && store.slurp.is_string()
                        ) {
                            let d = match store.val_opt {
                                Some(d) => d,
                                None => res!(Self::process_atom(&mut store.slurp, &kind)),
                            };
                            return Ok(match state.kind_outer {
                                // Unitary molecules
                                Kind::Usr(ukid) => Dat::Usr(ukid, Some(Box::new(d))),
                                Kind::Box(_)    => Dat::Box(Box::new(d)),
                                Kind::Some(_)   => Dat::Opt(Box::new(Some(d))),
                                _ => d,
                            });
                        } else {
                            if let Some(d) = store.val_opt {
                                return Ok(d);
                            } else {
                                return Err(err!(
                                    "Failed to capture the daticle of kind {:?} ({})",
                                    kind, cursor.borrow();
                                String, Input, Decode, Invalid));
                            }
                        }
                    }
                }
                '[' => {
                    if res!(store.slurp.char_slurped((i, c), &mut state)) {
                        continue;
                    }
                    if state.molecular_capture == None {
                        if state.kind_outer == Kind::Unknown {
                            state.molecular_capture = Some(MolecularCapture::ListMixed);
                        } else {
                            match MolecularCapture::from_kind(&state.kind_outer) {
                                Some(MolecularCapture::Map) => return Err(err!(
                                    "Expecting a store.map bracket '{{' but found a '[' ({})", cursor.borrow();
                                String, Input, Decode, Invalid)),
                                None => return Err(err!(
                                    "Found a '[' which is incompatible with a {:?} ({})",
                                    state.kind_outer, cursor.borrow();
                                String, Input, Decode, Invalid)),
                                _ => (),
                            }
                        }
                    }
                    let mut new_state = state.recurse();
                    if !state.kind_outer.uses_list_brackets() {
                        new_state.kind_outer = Kind::List;
                    }
                    store.val_opt = Some(res!(Self::recursive_decode(
                        &mut iter,
                        &cfg,
                        new_state,
                        cursor,
                    )));
                    store.slurp = Slurp::new();
                }
                ',' => {
                    if res!(store.slurp.char_slurped((i, c), &mut state)) {
                        continue;
                    }
                    res!(Self::comma_handler(
                        cfg,
                        &mut state,
                        cursor,
                        &mut store,
                    ));
                }
                ']' => {
                    if res!(store.slurp.char_slurped((i, c), &mut state)) {
                        continue;
                    }
                    // The remainder here is a terminal branch so no need to reset switches and store.
                    match state.molecular_capture {
                        Some(MolecularCapture::Bytes) => {
                            if store.slurp.len() > 0 {
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
                            if store.slurp.len() > 0 {
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
                '{' => {
                    if res!(store.slurp.char_slurped((i, c), &mut state)) {
                        continue;
                    }
                    if state.molecular_capture == None {
                        state.molecular_capture = Some(MolecularCapture::Map);
                    } else {
                        let mut new_state = state.recurse();
                        if !state.explicit_kind {
                            // We are free to define the kind of this store.map.
                            match cfg.use_ordmaps {
                                true => new_state.kind_outer = Kind::OrdMap,
                                false => new_state.kind_outer = Kind::Map,
                            }
                        }
                        store.val_opt = Some(res!(Self::recursive_decode(
                            &mut iter,
                            &cfg,
                            new_state,
                            cursor,
                        )));
                        store.slurp = Slurp::new();
                    }
                }
                ':' => {
                    if res!(store.slurp.char_slurped((i, c), &mut state)) {
                        continue;
                    }
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
                                    if store.slurp.len() > 0 {
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
                }
                '}' => {
                    if res!(store.slurp.char_slurped((i, c), &mut state)) {
                        continue;
                    }
                    // The remainder here is a terminal branch so no need to reset switches and store.
                    if state.molecular_capture == Some(MolecularCapture::Map) {
                        if store.slurp.len() > 0 {
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
                    return Err(err!(
                        "Map capture was not actived with a '{{' character ({})", cursor.borrow();
                    String, Input, Decode, Invalid));
                }
                _ => {
                    if res!(store.slurp.char_slurped((i, c), &mut state)) {
                        continue;
                    }
                    store.slurp.push(c);
                }
            } // match
        } // while loop
        if let Some(dat) = store.val_opt {
            return Ok(dat);
        }
        match state.molecular_capture {
            None => return Self::process_atom(&mut store.slurp, &state.kind_outer),
            Some(MolecularCapture::ListMixed)  |
            Some(MolecularCapture::ListSame)   |
            Some(MolecularCapture::Bytes) =>
                return Err(err!("Expected closure of a store.list with ']'";
                    String, Input, Decode, Missing)),
            Some(MolecularCapture::Map) =>
                return Err(err!("Expected closure of a store.map with '}}'";
                    String, Input, Decode, Missing)),
        }
    }

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
                        if store.slurp.len() > 0 {
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
                        let dat = if store.slurp.len() > 0 {
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

    /// Process a [`Slurp`] extracted during single-pass processing.  Numbers currently undergo an
    /// additional pass via validation in [`NumberString::new`], to cope with the various forms of
    /// representation, even when typed (e.g. different radices).
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
