use crate::{
    prelude::*,
    usr::{
        UsrKind,
        UsrKinds,
        UsrKindId,
        UsrKindCode,
    },
    int::DatInt,
    kind::KindCase,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    map::MapMut,
};
use oxedyne_fe2o3_num::{
    float::{
        Float32,
        Float64,
    },
};
use oxedyne_fe2o3_text::{
    base2x,
    string::{
        Indenter,
        Stringer,
    },
};

use std::{
    collections::BTreeMap,
    fmt,
};


/// Except for the addition of trailing commas and some explicit kindicles like (omap|, this is
/// essentially JSON format.
impl fmt::Display for Dat {
    fn fmt (&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.encode_string_with_config(&EncoderConfig::<(), ()>::display(None)) {
            Ok(s) => write!(f, "{}", s),
            Err(e) => write!(f, "{}", e),
        }
    }
}

/// This is essentially JDAT format, but with capitalised types which signals that
/// `fmt::Debug` formatting was used.
impl fmt::Debug for Dat {
    fn fmt (&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.encode_string_with_config(&EncoderConfig::<(), ()>::debug(None)) {
            Ok(s) => write!(f, "{}", s),
            Err(e) => write!(f, "{}", e),
        }
    }
}

/// Controls the visibility of type information (kindicles) during encoding.
///
/// # Variants
///
/// ## `Nothing`
/// - Used primarily in JSON-compatible mode
/// - Shows kindicles only for `AtomLogic` types (Empty, True, False, None)
/// - Suppresses all other type information
/// - Examples:
///   - `42` instead of `(u8|42)`
///   - `"text"` instead of `(str|"text")`
///   - Special handling of ABox comments: displays as strings with comment markers
///
/// ## `Some`
/// Shows kindicles for:
/// - Atomic Logic types (Empty, True, False, None)
/// - Molecule Unitary types (Usr, Box, Some, ABox)
/// - Tuples (Tup2-Tup10)
///
/// Hides kindicles for:
/// - All Atomic Fixed types (integers, floats)
/// - All Atomic Variable types (including strings)
/// - Lists, Maps, and their ordered variants
/// - All Molecule Same types (homogeneous collections like Vek, byte arrays)
///
/// Used by 
///
/// ## `Most`
/// The default scope.
///
/// Hides kindicles only for:
/// - Strings (shows as `"text"` instead of `(str|"text")`)
/// - Maps (shows as `{...}` instead of `(map|{...})`)
/// - Lists (shows as `[...]` instead of `(list|[...])`)
///
/// Shows kindicles for all other types.
///
/// ## `Everything`
/// - Shows kindicles for all types without exception
/// - Used in debug mode
/// - Most verbose output format
/// - Examples:
///   - `(str|"text")`
///   - `(map|{...})`
///   - `(list|[...])`
/// - ABox comments shown in full `(abox|data !comment!)` format
///
/// # Notes
///
/// - The `hide_usr_types` config flag can override the display of user-defined type kindicles
/// regardless of scope
/// - ABox (annotated box) types have special comment display handling based on scope:
///   - Everything: Full `(abox|data !comment!)` format
///   - Nothing: Comments as strings with comment markers
///   - Other modes: Data with trailing comment
///
/// # Kindicle Visibility Reference Table
///
/// |                           | Nothing | Some | Most | Everything |
/// | ------------------------- | ------- | ---- | ---- | ---------- |
/// | AtomLogic                 |         |      |      |            |
/// |   Empty,True,False,None   |    ✓    |  ✓   |    ✓ |       ✓    |
/// | AtomFixed                 |         |      |      |            |
/// |   U8-U128,I8-I128,F32/64  |    ✗    |  ✗   |    ✓ |       ✓    |
/// |                           |         |      |      |            |
/// | AtomVariable (except Str) |         |      |      |            |
/// |   Aint,Adec,C64           |    ✗    |  ✗   |    ✓ |       ✓    |
/// |                           |         |      |      |            |
/// | Str (special case)        |    ✗    |  ✗   |    ✗ |       ✓    |
/// |                           |         |      |      |            |
/// | MoleculeUnitary           |         |      |      |            |
/// |   Usr,Box,Some,ABox       |    ✗    |  ✓   |    ✓ |       ✓    |
/// |                           |         |      |      |            |
/// | MoleculeMixed             |         |      |      |            |
/// |   Tup2-10                 |    ✗    |  ✓   |    ✓ |       ✓    |
/// |   Map                     |    ✗    |  ✗   |    ✗ |       ✓    |
/// |   List                    |    ✗    |  ✗   |    ✗ |       ✓    |
/// |                           |         |      |      |            |
/// | MoleculeSame              |         |      |      |            |
/// |   Vek,BU8-64,B2-32,etc    |    ✗    |  ✗   |    ✓ |       ✓    |
/// ```
///
/// This enum provides a hierarchical system for controlling type information visibility
/// while maintaining data readability, ranging from JSON-compatible output to fully typed
/// JDAT format.
#[derive(Clone, Debug, PartialEq, PartialOrd)]
pub enum KindScope {
    Nothing,
    Some,
    Most,
    Everything,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ByteEncoding {
    Base2x,
    Binary,
    Decimal,
    Hex,
    Octal,
}

#[derive(Clone, Debug, PartialEq)]
pub enum IntEncoding {
    Binary,
    Decimal,
    Hex,
    Octal,
}

#[derive(Clone, Debug)]
pub struct EncoderConfig<
    M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
    M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
>{
    pub kind_scope:             KindScope,
    pub type_lower_case:        bool,
    pub byte_encoding:          ByteEncoding,
    pub int_encoding:           IntEncoding,
    pub to_lines:               bool,
    pub tab:                    String,
    pub trailing_commas:        bool,
    pub hide_usr_types:         bool,
    pub ukinds_opt:             Option<UsrKinds<M1, M2>>,
    pub comment_allowed:        bool, // Include Annotated Box comments?
    pub comment1_start_char:    char,
    pub comment1_end_char:      char,
    pub comment2_start_char:    char,
    pub comment2_end_char:      char,
}

impl<
    M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
    M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
>
    Default for EncoderConfig<M1, M2>
{
    fn default() -> Self {
        Self {
            kind_scope:             KindScope::Most,
            type_lower_case:        true,
            byte_encoding:          ByteEncoding::Hex,
            int_encoding:           IntEncoding::Decimal,
            to_lines:               false,
            tab:                    fmt!("  "),
            trailing_commas:        false,
            hide_usr_types:         false,
            ukinds_opt:             None,
            comment_allowed:        true,
            comment1_start_char:    '!',
            comment1_end_char:      '!',
            comment2_start_char:    '#',
            comment2_end_char:      '#',
        }
    }   
}

impl<
    M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
    M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
>
    EncoderConfig<M1, M2>
{
    pub fn debug(ukinds_opt: Option<UsrKinds<M1, M2>>) -> Self {
        Self {
            kind_scope:         KindScope::Everything,
            type_lower_case:    true,
            byte_encoding:      ByteEncoding::Decimal,
            trailing_commas:    true,
            ukinds_opt,
            ..Default::default()
        }
    }

    pub fn debug_to_lines(ukinds_opt: Option<UsrKinds<M1, M2>>, tab: &str) -> Self {
        let mut result = Self::debug(ukinds_opt); 
        result.to_lines = true;
        result.tab = tab.to_string();
        result
    }

    pub fn display(ukinds_opt: Option<UsrKinds<M1, M2>>) -> Self {
        Self {
            kind_scope:         KindScope::Most,
            type_lower_case:    true,
            byte_encoding:      ByteEncoding::Decimal,
            ukinds_opt,
            ..Default::default()
        }
    }

    pub fn display_to_lines(ukinds_opt: Option<UsrKinds<M1, M2>>, tab: &str) -> Self {
        let mut result = Self::display(ukinds_opt); 
        result.to_lines = true;
        result.tab = tab.to_string();
        result
    }

    pub fn display_some(ukinds_opt: Option<UsrKinds<M1, M2>>) -> Self {
        Self {
            kind_scope:         KindScope::Some,
            type_lower_case:    true,
            byte_encoding:      ByteEncoding::Decimal,
            ukinds_opt,
            ..Default::default()
        }
    }

    pub fn display_some_to_lines(ukinds_opt: Option<UsrKinds<M1, M2>>, tab: &str) -> Self {
        let mut result = Self::display_some(ukinds_opt); 
        result.to_lines = true;
        result.tab = tab.to_string();
        result
    }

    pub fn jdat(ukinds_opt: Option<UsrKinds<M1, M2>>) -> Self {
        Self {
            kind_scope:         KindScope::Most,
            byte_encoding:      ByteEncoding::Base2x,
            trailing_commas:    true,
            ukinds_opt,
            ..Default::default()
        }
    }

    pub fn jdat_to_lines(ukinds_opt: Option<UsrKinds<M1, M2>>, tab: &str) -> Self {
        let mut result = Self::jdat(ukinds_opt); 
        result.to_lines = true;
        result.tab = tab.to_string();
        result
    }

    pub fn jdat_full(ukinds_opt: Option<UsrKinds<M1, M2>>) -> Self {
        Self {
            kind_scope:         KindScope::Everything,
            byte_encoding:      ByteEncoding::Base2x,
            trailing_commas:    true,
            ukinds_opt,
            ..Default::default()
        }
    }

    pub fn jdat_full_to_lines(ukinds_opt: Option<UsrKinds<M1, M2>>, tab: &str) -> Self {
        let mut result = Self::jdat_full(ukinds_opt); 
        result.to_lines = true;
        result.tab = tab.to_string();
        result
    }

    pub fn json(ukinds_opt: Option<UsrKinds<M1, M2>>) -> Self {
        Self {
            kind_scope:         KindScope::Nothing,
            byte_encoding:      ByteEncoding::Decimal,
            ukinds_opt,
            comment_allowed:    false,
            ..Default::default()
        }
    }

    pub fn json_to_lines(ukinds_opt: Option<UsrKinds<M1, M2>>, tab: &str) -> Self {
        let mut result = Self::json(ukinds_opt); 
        result.to_lines = true;
        result.tab = tab.to_string();
        result
    }
}

#[derive(Clone, Debug, Default)]
pub struct EncoderState {
    pub indenter: Indenter,
}

impl EncoderState {

    pub fn recurse(&self) -> Self {
        Self {
            indenter: self.indenter.clone(),
            ..Default::default()
        }
    }

    pub fn inc_indent<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        mut self,
        cfg: &EncoderConfig<M1, M2>,
    )
        -> Self
    {
        if cfg.to_lines {
            self.indenter.inc();
        }
        self
    }

    pub fn dec_indent<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        mut self,
        cfg: &EncoderConfig<M1, M2>,
    )
        -> Self
    {
        if cfg.to_lines {
            self.indenter.dec();
        }
        self
    }
}

impl Dat {

    /// Uses `EncoderConfig::debug`, `EncoderConfig::display` and `oxedyne_fe2o3_core::string::Stringer`.
    pub fn to_lines(&self, tab: &str, print_kinds: bool) -> Vec<String> {
        let s = if print_kinds {
            fmt!("{:?}", self)
        } else {
            self.to_string()
        };
        Stringer::new(s).to_lines(tab)
    }

    pub fn debug< >(&self) -> Outcome<String> {
        let cfg = EncoderConfig::<(), ()>::debug(None);
        self.encode_string_with_config(&cfg)
    }

    pub fn debug_with_usr_kinds<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        &self,
        ukinds_opt: Option<UsrKinds<M1, M2>>,
    )
        -> Outcome<String>
    {
        let cfg = EncoderConfig::<M1, M2>::debug(ukinds_opt);
        self.encode_string_with_config(&cfg)
    }

    pub fn debug_to_lines(&self, tab: &str) -> Outcome<String> {
        let cfg = EncoderConfig::<(), ()>::debug_to_lines(None, tab);
        self.encode_string_with_config(&cfg)
    }

    pub fn debug_with_usr_kinds_to_lines<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        &self,
        ukinds_opt: Option<UsrKinds<M1, M2>>,
        tab:        &str,
    )
        -> Outcome<String>
    {
        let cfg = EncoderConfig::<M1, M2>::debug_to_lines(ukinds_opt, tab);
        self.encode_string_with_config(&cfg)
    }

    pub fn display(&self) -> Outcome<String> {
        let cfg = EncoderConfig::<(), ()>::display(None);
        self.encode_string_with_config(&cfg)
    }

    pub fn display_with_usr_kinds<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        &self,
        ukinds_opt: Option<UsrKinds<M1, M2>>,
    )
        -> Outcome<String>
    {
        let cfg = EncoderConfig::<M1, M2>::display(ukinds_opt);
        self.encode_string_with_config(&cfg)
    }

    pub fn display_to_lines(&self, tab: &str,) -> Outcome<String> {
        let cfg = EncoderConfig::<(), ()>::display_to_lines(None, tab);
        self.encode_string_with_config(&cfg)
    }

    pub fn display_some_to_lines(&self, tab: &str,) -> Outcome<String> {
        let cfg = EncoderConfig::<(), ()>::display_some_to_lines(None, tab);
        self.encode_string_with_config(&cfg)
    }

    pub fn display_with_usr_kinds_to_lines<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        &self,
        ukinds_opt: Option<UsrKinds<M1, M2>>,
        tab:        &str,
    )
        -> Outcome<String>
    {
        let cfg = EncoderConfig::<M1, M2>::display_to_lines(ukinds_opt, tab);
        self.encode_string_with_config(&cfg)
    }

    pub fn jdat(&self) -> Outcome<String> {
        let cfg = EncoderConfig::<(), ()>::jdat(None);
        self.encode_string_with_config(&cfg)
    }

    pub fn jdat_with_usr_kinds<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        &self,
        ukinds_opt: Option<UsrKinds<M1, M2>>,
    )
        -> Outcome<String>
    {
        let cfg = EncoderConfig::<M1, M2>::jdat(ukinds_opt);
        self.encode_string_with_config(&cfg)
    }

    pub fn jdat_to_lines(&self, tab: &str,) -> Outcome<String> {
        let cfg = EncoderConfig::<(), ()>::jdat_to_lines(None, tab);
        self.encode_string_with_config(&cfg)
    }

    pub fn jdat_with_usr_kinds_to_lines<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        &self,
        ukinds_opt: Option<UsrKinds<M1, M2>>,
        tab:        &str,
    )
        -> Outcome<String>
    {
        let cfg = EncoderConfig::<M1, M2>::jdat_to_lines(ukinds_opt, tab);
        self.encode_string_with_config(&cfg)
    }

    pub fn jdat_full(&self) -> Outcome<String> {
        let cfg = EncoderConfig::<(), ()>::jdat_full(None);
        self.encode_string_with_config(&cfg)
    }

    pub fn jdat_full_with_usr_kinds<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        &self,
        ukinds_opt: Option<UsrKinds<M1, M2>>,
    )
        -> Outcome<String>
    {
        let cfg = EncoderConfig::<M1, M2>::jdat_full(ukinds_opt);
        self.encode_string_with_config(&cfg)
    }

    pub fn jdat_full_to_lines(&self, tab: &str,) -> Outcome<String> {
        let cfg = EncoderConfig::<(), ()>::jdat_full_to_lines(None, tab);
        self.encode_string_with_config(&cfg)
    }

    pub fn jdat_full_with_usr_kinds_to_lines<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        &self,
        ukinds_opt: Option<UsrKinds<M1, M2>>,
        tab:        &str,
    )
        -> Outcome<String>
    {
        let cfg = EncoderConfig::<M1, M2>::jdat_full_to_lines(ukinds_opt, tab);
        self.encode_string_with_config(&cfg)
    }

    pub fn json(&self) -> Outcome<String> {
        let cfg = EncoderConfig::<(), ()>::json(None);
        self.encode_string_with_config(&cfg)
    }

    pub fn json_with_usr_kinds<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        &self,
        ukinds_opt: Option<UsrKinds<M1, M2>>,
    )
        -> Outcome<String>
    {
        let cfg = EncoderConfig::<M1, M2>::json(ukinds_opt);
        self.encode_string_with_config(&cfg)
    }

    pub fn json_to_lines(&self, tab: &str,) -> Outcome<String> {
        let cfg = EncoderConfig::<(), ()>::json_to_lines(None, tab);
        self.encode_string_with_config(&cfg)
    }

    pub fn json_with_usr_kinds_to_lines<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        &self,
        ukinds_opt: Option<UsrKinds<M1, M2>>,
        tab:        &str,
    )
        -> Outcome<String>
    {
        let cfg = EncoderConfig::<M1, M2>::json_to_lines(ukinds_opt, tab);
        self.encode_string_with_config(&cfg)
    }


    pub fn encode_string<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        &self,
    )
        -> Outcome<String>
    {
        let cfg = EncoderConfig::<
            BTreeMap<UsrKindCode, UsrKind>,
            BTreeMap<String, UsrKindId>,
        >::default();

        self.encode_string_with_config(&cfg)
    }

    pub fn encode_string_with_config<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        &self,
        cfg: &EncoderConfig<M1, M2>,
    )
        -> Outcome<String>
    {
        self.recursive_encode(
            cfg,
            EncoderState::default(),
        )
    }

    pub fn recursive_encode<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        &self,
        cfg:        &EncoderConfig<M1, M2>,
        mut state:  EncoderState,
    )
        -> Outcome<String>
    {
        let mut typ_str = if
            (cfg.kind_scope == KindScope::Nothing)
            && (self.kind().case() != KindCase::AtomLogic)
        {
            String::new()
        } else {
            if cfg.type_lower_case {
                self.kind().to_string().to_lowercase()
            } else {
                self.kind().to_string().to_uppercase()
            }
        };
        // The following match avoids returning from the method anywhere, except in the case of
        // Self::ABox.
        let (s, is_dataless) = match self {
            // Atomic Kinds ===========================
            // Logic
            Self::Empty     => (fmt!("\"\""), true),
            Self::Bool(_b)  => (fmt!("\"{}\"", typ_str), true),
            // Fixed
            Self::U8(n)     => (Self::encode_int(&DatInt::U8(*n), &cfg), false),
            Self::U16(n)    => (Self::encode_int(&DatInt::U16(*n), &cfg), false),
            Self::U32(n)    => (Self::encode_int(&DatInt::U32(*n), &cfg), false),
            Self::U64(n)    |
            Self::C64(n)    => (Self::encode_int(&DatInt::U64(*n), &cfg), false),
            Self::U128(n)   => (Self::encode_int(&DatInt::U128(*n), &cfg), false),
            Self::I8(n)     => (Self::encode_int(&DatInt::I8(*n), &cfg), false),
            Self::I16(n)    => (Self::encode_int(&DatInt::I16(*n), &cfg), false),
            Self::I32(n)    => (Self::encode_int(&DatInt::I32(*n), &cfg), false),
            Self::I64(n)    => (Self::encode_int(&DatInt::I64(*n), &cfg), false),
            Self::I128(n)   => (Self::encode_int(&DatInt::I128(*n), &cfg), false),
            Self::F32(Float32(v)) => {
                let native = fmt!("{:e}", v);
                let native_with_decimal = if !native.contains('.') {
                    native.replace("e", ".0e")
                } else {
                    native
                };
                (native_with_decimal, false)
            }
            Self::F64(Float64(v)) => {
                let native = fmt!("{:e}", v);
                let native_with_decimal = if !native.contains('.') {
                    native.replace("e", ".0e")
                } else {
                    native
                };
                (native_with_decimal, false)
            }
            // Variable
            Self::Aint(n) => (fmt!("{}", n), false),
            Self::Adec(n) => {
                let (bint, expi64) = n.as_bigint_and_exponent();
                (fmt!("{}e{}", bint, -expi64), false)
            }
            Self::Str(s) => (fmt!("\"{}\"", s), false),
            // Molecule Kinds =========================
            // Unitary
            Self::Usr(ukid, optboxd) => {
                typ_str = ukid.label().to_string();
                if let Some(ukinds) = &cfg.ukinds_opt {
                    // If a UsrKinds has been given it can override the string label.
                    if let Some(ukid2) = ukinds.get_code(&ukid.code()) {
                        typ_str = ukid2.label().to_string();
                    }
                }
                match optboxd {
                    None => (fmt!("\"\""), true),
                    Some(boxd) => (fmt!("{}", res!(boxd.recursive_encode(&cfg, state.recurse()))), false)
                }
            }
            Self::Box(d) => (fmt!("{}", res!(d.recursive_encode(&cfg, state.recurse()))), false),
            Self::Opt(boxoptd) => {
                match &**boxoptd {
                    None => (fmt!("\"{}\"", typ_str), true),
                    Some(d) => (fmt!("{}", res!(d.recursive_encode(&cfg, state.recurse()))), false),
                }
            }
            Self::ABox(ncfg, d, s) => {
                // Kindicle wrapping dealt with with specially here before exit, unlike all other
                // kinds.
                let (start_char, end_char) = if ncfg.is_type1() {
                    (cfg.comment1_start_char, cfg.comment1_end_char)
                } else {
                    (cfg.comment2_start_char, cfg.comment2_end_char)
                };
                return Ok(match cfg.kind_scope {
                    KindScope::Everything => {
                        // Bias toward a reluctance to show the (abox| kindicle.
                        fmt!(
                            "({}|{} {}{}{})",
                            typ_str,
                            res!(d.recursive_encode(&cfg, state.recurse())),
                            start_char,
                            s,
                            end_char,
                        )
                    }
                    KindScope::Nothing => {
                        // Display the comment as a plain string, but wrap it in the comment
                        // characters so that a reader can identify it as a comment.
                        if **d == Dat::Empty {
                            fmt!(
                                "\"{}{}{}\"",
                                start_char,
                                s,
                                end_char,
                            )

                        } else {
                            // Don't throw out comment, package it with the data in a list,
                            // including the comment characters.
                            fmt!(
                                "[{}, \"{}{}{}\"{}]",
                                res!(d.recursive_encode(&cfg, state.recurse())),
                                start_char,
                                s,
                                end_char,
                                if cfg.trailing_commas { ", " } else { "" },
                            )
                        }
                    }
                    _ => {
                        if **d == Dat::Empty {
                            fmt!(
                                "{}{}{}",
                                start_char,
                                s,
                                end_char,
                            )

                        } else {
                            fmt!(
                                "{} {}{}{}",
                                res!(d.recursive_encode(&cfg, state.recurse())),
                                start_char,
                                s,
                                end_char,
                            )
                        }
                    }
                });
            }
            // Heterogenous
            Self::List(list) => res!(Self::encode_list(list.as_slice(), &cfg, &mut state)),
            Self::Tup2(arr) => res!(Self::encode_list(&arr[..], &cfg, &mut state)),
            Self::Tup3(arr) => res!(Self::encode_list(&arr[..], &cfg, &mut state)),
            Self::Tup4(arr) => res!(Self::encode_list(&arr[..], &cfg, &mut state)),
            Self::Tup5(arr) => res!(Self::encode_list(&arr[..], &cfg, &mut state)),
            Self::Tup6(arr) => res!(Self::encode_list(&arr[..], &cfg, &mut state)),
            Self::Tup7(arr) => res!(Self::encode_list(&arr[..], &cfg, &mut state)),
            Self::Tup8(arr) => res!(Self::encode_list(&arr[..], &cfg, &mut state)),
            Self::Tup9(arr) => res!(Self::encode_list(&arr[..], &cfg, &mut state)),
            Self::Tup10(arr) => res!(Self::encode_list(&arr[..], &cfg, &mut state)),
            Self::Map(m) => {
                if m.len() == 0 {
                    (fmt!("{{}}"), false)
                } else {
                    let mut s = String::new();
                    let last = m.len() - 1;
                    let (indent0, indent, sep, sep_last) = Self::collection_encoding(
                        &cfg,
                        &mut state,
                    );
                    for (i, (k, v)) in m.iter().enumerate() {
                        res!(Self::encode_map_entry(
                            (k, v),
                            &mut s,
                            &cfg,
                            &mut state,
                            &indent,
                            &sep,
                            &sep_last,
                            i == last,
                        ));
                    }
                    (
                        fmt!("{{{}{}{}}}",
                            if cfg.to_lines { "\n" } else { " " },
                            s,
                            indent0,
                        ),
                        false,
                    )
                }
            }
            Self::OrdMap(m) => {
                if m.len() == 0 {
                    (fmt!("{{}}"), false)
                } else {
                    let mut s = String::new();
                    let last = m.len() - 1;
                    let (indent0, indent, sep, sep_last) = Self::collection_encoding(
                        &cfg,
                        &mut state,
                    );
                    for (i, (mk, v)) in m.iter().enumerate() {
                        let k = mk.dat();
                        res!(Self::encode_map_entry(
                            (k, v),
                            &mut s,
                            &cfg,
                            &mut state,
                            &indent,
                            &sep,
                            &sep_last,
                            i == last,
                        ));
                    }
                    (
                        fmt!("{{{}{}{}}}",
                            if cfg.to_lines { "\n" } else { " " },
                            s,
                            indent0,
                        ),
                        false,
                    )
                }
            }
            // Homogenous
            Self::Vek(vek) => res!(Self::encode_list(vek.as_slice(), &cfg, &mut state)),
            // Byte arrays
            // Variable length bytes
            Self::BU8(v)    |
            Self::BU16(v)   |
            Self::BU32(v)   |
            Self::BU64(v)   |
            Self::BC64(v)  => (Self::encode_bytes(&v, &cfg), false),
            // Fixed length bytes
            Self::B2(a)     => (Self::encode_bytes(&a[..], &cfg), false),
            Self::B3(a)     => (Self::encode_bytes(&a[..], &cfg), false),
            Self::B4(a)     => (Self::encode_bytes(&a[..], &cfg), false),
            Self::B5(a)     => (Self::encode_bytes(&a[..], &cfg), false),
            Self::B6(a)     => (Self::encode_bytes(&a[..], &cfg), false),
            Self::B7(a)     => (Self::encode_bytes(&a[..], &cfg), false),
            Self::B8(a)     => (Self::encode_bytes(&a[..], &cfg), false),
            Self::B9(a)     => (Self::encode_bytes(&a[..], &cfg), false),
            Self::B10(a)    => (Self::encode_bytes(&a[..], &cfg), false),
            Self::B16(a)    => (Self::encode_bytes(&a[..], &cfg), false),
            Self::B32(a)    => (Self::encode_bytes(&a[..], &cfg), false),
            // Fixed length numbers
            Self::Tup2u16(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup3u16(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup4u16(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup5u16(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup6u16(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup7u16(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup8u16(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup9u16(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup10u16(a)   => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup2u32(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup3u32(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup4u32(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup5u32(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup6u32(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup7u32(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup8u32(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup9u32(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup10u32(a)   => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup2u64(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup3u64(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup4u64(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup5u64(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup6u64(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup7u64(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup8u64(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup9u64(a)    => (Self::encode_ints(&a[..], &cfg), false),
            Self::Tup10u64(a)   => (Self::encode_ints(&a[..], &cfg), false),
        };
        let kind = self.kind();
        if kind.is_usr() && cfg.hide_usr_types {
            return Ok(if is_dataless {
                fmt!("\"{}\"", typ_str)
            } else {
                fmt!("{}", s)
            });
        }
        let hide_kindicle =
            cfg.kind_scope == KindScope::Nothing
            || (cfg.kind_scope == KindScope::Some &&
                match kind {
                    Kind::Str | Kind::Map | Kind::List => true,
                    _ => {
                        match kind.case() {
                            KindCase::AtomFixed     |
                            KindCase::AtomVariable  |
                            KindCase::MoleculeSame  => true,
                            _ => false,
                        }
                    }
                }
            )
            || (cfg.kind_scope == KindScope::Most &&
                match kind { // Don't show kindicles for strings, maps, and lists.
                    Kind::Str | Kind::Map | Kind::List => true,
                    _ => false,
                }
            );
        Ok(if hide_kindicle {
            if is_dataless && kind.is_usr() {
                // Special case of usr kinds carrying data in json format, e.g.
                // jdat: (my_kind)
                // json: "my_kind"
                fmt!("\"{}\"", typ_str)
            } else {
                fmt!("{}", s)
            }
        } else {
            if is_dataless {
                fmt!("({})", typ_str)
            } else {
                fmt!("({}|{})", typ_str, s)
            }
        })
    }

    pub fn encode_bytes<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        byts:   &[u8],
        cfg:    &EncoderConfig<M1, M2>,
    )
        -> String
    {
        if cfg.trailing_commas {
            match cfg.byte_encoding {
                ByteEncoding::Base2x => fmt!("\"{}\"", base2x::HEMATITE64.to_string(&byts)),
                ByteEncoding::Binary => fmt!("[{}]", byts.iter().map(|b| fmt!("{}, ", DatInt::U8(*b).fmt_bin()))
                    .collect::<Vec<String>>().join("")),
                ByteEncoding::Decimal => fmt!("[{}]", byts.iter().map(|b| fmt!("{}, ",  DatInt::U8(*b).fmt_dec()))
                    .collect::<Vec<String>>().join("")),
                ByteEncoding::Hex => fmt!("[{}]", byts.iter().map(|b| fmt!("{}, ", DatInt::U8(*b).fmt_hex()))
                    .collect::<Vec<String>>().join("")),
                ByteEncoding::Octal => fmt!("[{}]", byts.iter().map(|b| fmt!("{}, ", DatInt::U8(*b).fmt_oct()))
                    .collect::<Vec<String>>().join("")),
            }
        } else {
            match cfg.byte_encoding {
                ByteEncoding::Base2x => fmt!("\"{}\"", base2x::HEMATITE64.to_string(&byts)),
                ByteEncoding::Binary => fmt!("[{}]", byts.iter().map(|b| DatInt::U8(*b).fmt_bin())
                    .collect::<Vec<String>>().join(", ")),
                ByteEncoding::Decimal => fmt!("[{}]", byts.iter().map(|b| DatInt::U8(*b).fmt_dec())
                    .collect::<Vec<String>>().join(", ")),
                ByteEncoding::Hex => fmt!("[{}]", byts.iter().map(|b| DatInt::U8(*b).fmt_hex())
                    .collect::<Vec<String>>().join(", ")),
                ByteEncoding::Octal => fmt!("[{}]", byts.iter().map(|b| DatInt::U8(*b).fmt_oct())
                    .collect::<Vec<String>>().join(", ")),
            }
        }
    }

    pub fn encode_int<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        n:      &DatInt,
        cfg:    &EncoderConfig<M1, M2>,
    )
        -> String
    {
        match cfg.int_encoding {
            IntEncoding::Binary     => n.fmt_bin(),
            IntEncoding::Decimal    => n.fmt_dec(),
            IntEncoding::Octal      => n.fmt_oct(),
            IntEncoding::Hex        => n.fmt_hex(),
        }
    }

    pub fn encode_ints<
        T: Into<DatInt> + Copy,
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        a:      &[T],
        cfg:    &EncoderConfig<M1, M2>,
    )
        -> String
    {
        if cfg.trailing_commas {
            match cfg.int_encoding {
                IntEncoding::Binary => fmt!("[{}]", a.iter().map(|n| fmt!("{}, ", (*n).into().fmt_bin()))
                    .collect::<Vec<String>>().join("")),
                IntEncoding::Decimal => fmt!("[{}]", a.iter().map(|n| fmt!("{}, ",  (*n).into().fmt_dec()))
                    .collect::<Vec<String>>().join("")),
                IntEncoding::Hex => fmt!("[{}]", a.iter().map(|n| fmt!("{}, ", (*n).into().fmt_hex()))
                    .collect::<Vec<String>>().join("")),
                IntEncoding::Octal => fmt!("[{}]", a.iter().map(|n| fmt!("{}, ", (*n).into().fmt_oct()))
                    .collect::<Vec<String>>().join("")),
            }
        } else {
            match cfg.int_encoding {
                IntEncoding::Binary => fmt!("[{}]", a.iter().map(|n| (*n).into().fmt_bin())
                    .collect::<Vec<String>>().join(", ")),
                IntEncoding::Decimal => fmt!("[{}]", a.iter().map(|n| (*n).into().fmt_dec())
                    .collect::<Vec<String>>().join(", ")),
                IntEncoding::Hex => fmt!("[{}]", a.iter().map(|n| (*n).into().fmt_hex())
                    .collect::<Vec<String>>().join(", ")),
                IntEncoding::Octal => fmt!("[{}]", a.iter().map(|n| (*n).into().fmt_oct())
                    .collect::<Vec<String>>().join(", ")),
            }
        }
    }

    pub fn encode_map_entry<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        (k, v):     (&Dat, &Dat),
        s:          &mut String,
        cfg:        &EncoderConfig<M1, M2>,
        state:      &mut EncoderState,
        indent:     &String,
        sep:        &String,
        sep_last:   &String,
        is_last:    bool,
    )
        -> Outcome<()>
    {
        if cfg.to_lines
            && cfg.comment_allowed
            && cfg.kind_scope != KindScope::Everything
            && (k.kind().is_abox() || v.kind().is_abox())
        {
            // Special case of abox annotation to end of line.
            if !k.kind().is_abox() {
                // :! A comment\n i.e. () => abox!((), "A comment")
                if let Dat::ABox(ncfg, boxdat, comment) = v {
                    if boxdat.kind() == Kind::Empty {
                        let key_str = if k.kind() == Kind::Empty {
                            fmt!("")
                        } else {
                            res!(k.recursive_encode(&cfg, state.clone()))
                        };
                        s.push_str(&fmt!("{}{}:{} {}\n",
                            indent,
                            key_str,
                            if ncfg.is_type1() {
                                cfg.comment1_start_char
                            } else {
                                cfg.comment2_start_char
                            },
                            comment,
                        ));
                    }
                }
            } else if !v.kind().is_abox() {
                // ! A comment\n i.e. abox!((), "A comment") => ()
                if let Dat::ABox(ncfg, boxdat, comment) = k {
                    if boxdat.kind() == Kind::Empty {
                        s.push_str(&fmt!("{}{} {}\n",
                            indent,
                            if ncfg.is_type1() {
                                cfg.comment1_start_char
                            } else {
                                cfg.comment2_start_char
                            },
                            comment,
                        ));
                    }
                }
            }
        } else {
            s.push_str(&fmt!("{}{}: {}{}",
                indent,
                res!(k.recursive_encode(&cfg, state.recurse())),
                res!(v.recursive_encode(&cfg, state.recurse())),
                if !is_last { &sep } else { &sep_last },
            ));
        }
        Ok(())
    }

    pub fn encode_list<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        list:       &[Dat],
        cfg:        &EncoderConfig<M1, M2>,
        mut state:  &mut EncoderState,
    )
        -> Outcome<(String, bool)>
    {
        if list.len() == 0 {
            Ok((fmt!("[]"), false))
        } else {
            let mut s = String::new();
            let last = list.len() - 1;
            let (indent0, indent, sep, sep_last) = Self::collection_encoding(
                &cfg,
                &mut state,
            );
            for (i, v) in list.iter().enumerate() {
                if cfg.to_lines
                    && cfg.comment_allowed
                    && cfg.kind_scope != KindScope::Everything
                    && v.kind().is_abox()
                {
                    // Special case of abox annotation to end of line.
                    if let Dat::ABox(ncfg, boxdat, comment) = v {
                        if boxdat.kind() == Kind::Empty {
                            s.push_str(&fmt!("{}{} {}\n",
                                indent,
                                if ncfg.is_type1() {
                                    cfg.comment1_start_char
                                } else {
                                    cfg.comment2_start_char
                                },
                                comment,
                            ));
                        }
                    }
                } else {
                    s.push_str(&fmt!("{}{}{}",
                        indent,
                        res!(v.recursive_encode(&cfg, state.recurse())),
                        if i != last { &sep } else { &sep_last },
                    ));
                }
            }
            Ok((
                fmt!("[{}{}{}]",
                    if cfg.to_lines { "\n" } else { " " },
                    s,
                    indent0,
                ),
                false,
            ))
        }
    }

    pub fn collection_encoding<
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        cfg:        &EncoderConfig<M1, M2>,
        state:      &mut EncoderState,
    )
        -> (
            String, // indent0
            String, // indent
            String, // sep
            String, // sep_last
        )
    {
        let sep = fmt!(",{}", if cfg.to_lines { "\n" } else { " " });
        let sep_last = fmt!(
            "{}{}",
            if cfg.trailing_commas { "," } else { "" },
            if cfg.to_lines { "\n" } else { "" },
        );
        let indent0 = cfg.tab.repeat(state.indenter.level());
        if cfg.to_lines {
            state.indenter.inc();
        }
        let indent = cfg.tab.repeat(state.indenter.level());
        (indent0, indent, sep, sep_last)
    }
}
