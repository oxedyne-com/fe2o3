use oxedyne_fe2o3_core::prelude::*;

/// English ordinal numbers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OrdinalEnglish {
    First,
    Second,
    Third,
    Fourth,
    Fifth,
    Sixth,
    Seventh,
    Eighth,
    Ninth,
    Tenth,
    Eleventh,
    Twelfth,
    Thirteenth,
    Fourteenth,
    Fifteenth,
    Sixteenth,
    Seventeenth,
    Eighteenth,
    Nineteenth,
    Twentieth,
    TwentyFirst,
    TwentySecond,
    TwentyThird,
    TwentyFourth,
    TwentyFifth,
    TwentySixth,
    TwentySeventh,
    TwentyEighth,
    TwentyNinth,
    Thirtieth,
    ThirtyFirst,
}

impl OrdinalEnglish {
    pub fn from_number(n: u8) -> Outcome<Self> {
        match n {
            1 => Ok(Self::First),
            2 => Ok(Self::Second),
            3 => Ok(Self::Third),
            4 => Ok(Self::Fourth),
            5 => Ok(Self::Fifth),
            6 => Ok(Self::Sixth),
            7 => Ok(Self::Seventh),
            8 => Ok(Self::Eighth),
            9 => Ok(Self::Ninth),
            10 => Ok(Self::Tenth),
            11 => Ok(Self::Eleventh),
            12 => Ok(Self::Twelfth),
            13 => Ok(Self::Thirteenth),
            14 => Ok(Self::Fourteenth),
            15 => Ok(Self::Fifteenth),
            16 => Ok(Self::Sixteenth),
            17 => Ok(Self::Seventeenth),
            18 => Ok(Self::Eighteenth),
            19 => Ok(Self::Nineteenth),
            20 => Ok(Self::Twentieth),
            21 => Ok(Self::TwentyFirst),
            22 => Ok(Self::TwentySecond),
            23 => Ok(Self::TwentyThird),
            24 => Ok(Self::TwentyFourth),
            25 => Ok(Self::TwentyFifth),
            26 => Ok(Self::TwentySixth),
            27 => Ok(Self::TwentySeventh),
            28 => Ok(Self::TwentyEighth),
            29 => Ok(Self::TwentyNinth),
            30 => Ok(Self::Thirtieth),
            31 => Ok(Self::ThirtyFirst),
            _ => Err(err!("Ordinal {} not implemented", n; Unimplemented)),
        }
    }

    pub fn value(&self) -> u8 {
        match self {
            Self::First => 1,
            Self::Second => 2,
            Self::Third => 3,
            Self::Fourth => 4,
            Self::Fifth => 5,
            Self::Sixth => 6,
            Self::Seventh => 7,
            Self::Eighth => 8,
            Self::Ninth => 9,
            Self::Tenth => 10,
            Self::Eleventh => 11,
            Self::Twelfth => 12,
            Self::Thirteenth => 13,
            Self::Fourteenth => 14,
            Self::Fifteenth => 15,
            Self::Sixteenth => 16,
            Self::Seventeenth => 17,
            Self::Eighteenth => 18,
            Self::Nineteenth => 19,
            Self::Twentieth => 20,
            Self::TwentyFirst => 21,
            Self::TwentySecond => 22,
            Self::TwentyThird => 23,
            Self::TwentyFourth => 24,
            Self::TwentyFifth => 25,
            Self::TwentySixth => 26,
            Self::TwentySeventh => 27,
            Self::TwentyEighth => 28,
            Self::TwentyNinth => 29,
            Self::Thirtieth => 30,
            Self::ThirtyFirst => 31,
        }
    }

    /// Parse from a string name (case insensitive).
    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.to_lowercase();
        
        // Handle numeric ordinals with suffixes
        if let Some(ordinal) = Self::parse_numeric_ordinal(&name) {
            return Some(ordinal);
        }
        
        // Handle word ordinals
        match name.as_str() {
            "first" => Some(Self::First),
            "second" => Some(Self::Second),
            "third" => Some(Self::Third),
            "fourth" => Some(Self::Fourth),
            "fifth" => Some(Self::Fifth),
            "sixth" => Some(Self::Sixth),
            "seventh" => Some(Self::Seventh),
            "eighth" => Some(Self::Eighth),
            "ninth" => Some(Self::Ninth),
            "tenth" => Some(Self::Tenth),
            "eleventh" => Some(Self::Eleventh),
            "twelfth" => Some(Self::Twelfth),
            "thirteenth" => Some(Self::Thirteenth),
            "fourteenth" => Some(Self::Fourteenth),
            "fifteenth" => Some(Self::Fifteenth),
            "sixteenth" => Some(Self::Sixteenth),
            "seventeenth" => Some(Self::Seventeenth),
            "eighteenth" => Some(Self::Eighteenth),
            "nineteenth" => Some(Self::Nineteenth),
            "twentieth" => Some(Self::Twentieth),
            "twenty-first" | "twenty first" => Some(Self::TwentyFirst),
            "twenty-second" | "twenty second" => Some(Self::TwentySecond),
            "twenty-third" | "twenty third" => Some(Self::TwentyThird),
            "twenty-fourth" | "twenty fourth" => Some(Self::TwentyFourth),
            "twenty-fifth" | "twenty fifth" => Some(Self::TwentyFifth),
            "twenty-sixth" | "twenty sixth" => Some(Self::TwentySixth),
            "twenty-seventh" | "twenty seventh" => Some(Self::TwentySeventh),
            "twenty-eighth" | "twenty eighth" => Some(Self::TwentyEighth),
            "twenty-ninth" | "twenty ninth" => Some(Self::TwentyNinth),
            "thirtieth" => Some(Self::Thirtieth),
            "thirty-first" | "thirty first" => Some(Self::ThirtyFirst),
            _ => None,
        }
    }

    /// Parse numeric ordinals like "1st", "2nd", "3rd", "4th", etc.
    fn parse_numeric_ordinal(name: &str) -> Option<Self> {
        if name.len() < 3 {
            return None;
        }
        
        let (num_part, suffix) = name.split_at(name.len() - 2);
        
        // Check if it ends with valid ordinal suffix
        let is_valid_suffix = match suffix {
            "st" | "nd" | "rd" | "th" => true,
            _ => false,
        };
        
        if !is_valid_suffix {
            return None;
        }
        
        // Parse the numeric part
        if let Ok(num) = num_part.parse::<u8>() {
            Self::from_number(num).ok()
        } else {
            None
        }
    }
    
    /// Returns the long name (Java-compatible method).
    pub fn to_long_name(&self) -> &'static str {
        match self {
            Self::First => "FIRST",
            Self::Second => "SECOND",
            Self::Third => "THIRD",
            Self::Fourth => "FOURTH",
            Self::Fifth => "FIFTH",
            Self::Sixth => "SIXTH",
            Self::Seventh => "SEVENTH",
            Self::Eighth => "EIGHTH",
            Self::Ninth => "NINTH",
            Self::Tenth => "TENTH",
            Self::Eleventh => "ELEVENTH",
            Self::Twelfth => "TWELFTH",
            Self::Thirteenth => "THIRTEENTH",
            Self::Fourteenth => "FOURTEENTH",
            Self::Fifteenth => "FIFTEENTH",
            Self::Sixteenth => "SIXTEENTH",
            Self::Seventeenth => "SEVENTEENTH",
            Self::Eighteenth => "EIGHTEENTH",
            Self::Nineteenth => "NINETEENTH",
            Self::Twentieth => "TWENTIETH",
            Self::TwentyFirst => "TWENTYFIRST",
            Self::TwentySecond => "TWENTYSECOND",
            Self::TwentyThird => "TWENTYTHIRD",
            Self::TwentyFourth => "TWENTYFOURTH",
            Self::TwentyFifth => "TWENTYFIFTH",
            Self::TwentySixth => "TWENTYSIXTH",
            Self::TwentySeventh => "TWENTYSEVENTH",
            Self::TwentyEighth => "TWENTYEIGHTH",
            Self::TwentyNinth => "TWENTYNINTH",
            Self::Thirtieth => "THIRTIETH",
            Self::ThirtyFirst => "THIRTYFIRST",
        }
    }
    
    /// Returns the short name (Java-compatible method).
    pub fn to_short_name(&self) -> String {
        let num = self.value();
        let suffix = match num {
            1 | 21 | 31 => "ST",
            2 | 22 => "ND", 
            3 | 23 => "RD",
            _ => "TH",
        };
        format!("{}{}", num, suffix)
    }
    
    /// Java-compatible method name.
    pub fn of(&self) -> u8 {
        self.value()
    }
    
    /// Java-compatible lookup by value.
    pub fn get(val: u8) -> Option<Self> {
        Self::from_number(val).ok()
    }
    
    /// Java-compatible lookup using short name.
    pub fn get_using_short_name(name: &str) -> Option<Self> {
        let name = name.replace(" ", "").to_uppercase();
        Self::parse_numeric_ordinal(&name.to_lowercase())
    }
    
    /// Java-compatible lookup using long name.
    pub fn get_using_long_name(name: &str) -> Option<Self> {
        let name = name.replace(" ", "").to_uppercase();
        match name.as_str() {
            "FIRST" => Some(Self::First),
            "SECOND" => Some(Self::Second),
            "THIRD" => Some(Self::Third),
            "FOURTH" => Some(Self::Fourth),
            "FIFTH" => Some(Self::Fifth),
            "SIXTH" => Some(Self::Sixth),
            "SEVENTH" => Some(Self::Seventh),
            "EIGHTH" => Some(Self::Eighth),
            "NINTH" => Some(Self::Ninth),
            "TENTH" => Some(Self::Tenth),
            "ELEVENTH" => Some(Self::Eleventh),
            "TWELFTH" => Some(Self::Twelfth),
            "THIRTEENTH" => Some(Self::Thirteenth),
            "FOURTEENTH" => Some(Self::Fourteenth),
            "FIFTEENTH" => Some(Self::Fifteenth),
            "SIXTEENTH" => Some(Self::Sixteenth),
            "SEVENTEENTH" => Some(Self::Seventeenth),
            "EIGHTEENTH" => Some(Self::Eighteenth),
            "NINETEENTH" => Some(Self::Nineteenth),
            "TWENTIETH" => Some(Self::Twentieth),
            "TWENTYFIRST" => Some(Self::TwentyFirst),
            "TWENTYSECOND" => Some(Self::TwentySecond),
            "TWENTYTHIRD" => Some(Self::TwentyThird),
            "TWENTYFOURTH" => Some(Self::TwentyFourth),
            "TWENTYFIFTH" => Some(Self::TwentyFifth),
            "TWENTYSIXTH" => Some(Self::TwentySixth),
            "TWENTYSEVENTH" => Some(Self::TwentySeventh),
            "TWENTYEIGHTH" => Some(Self::TwentyEighth),
            "TWENTYNINTH" => Some(Self::TwentyNinth),
            "THIRTIETH" => Some(Self::Thirtieth),
            "THIRTYFIRST" => Some(Self::ThirtyFirst),
            _ => None,
        }
    }
    
    /// Java-compatible convenience method.
    pub fn get_using_name(name: &str) -> Option<Self> {
        Self::get_using_short_name(name)
            .or_else(|| Self::get_using_long_name(name))
    }
    
    /// Used by parser - checks if string is ordinal suffix.
    pub fn is_ordinal_suffix(s: &str) -> bool {
        let s = s.to_uppercase();
        matches!(s.as_str(), "ST" | "ND" | "RD" | "TH")
    }
}