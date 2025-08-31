use crate::prelude::*;

impl Dat {

    // Atomic Kinds ===========================
    // Logic
    pub const EMPTY_CODE:       u8 = 0x00; // 0
    pub const TRUE_CODE:        u8 = 0x01; // 1
    pub const FALSE_CODE:       u8 = 0x02; // 2
    pub const OPT_NONE_CODE:    u8 = 0x03; // 3
    //                               0x04     4
    //                               0x05     5
    //                               0x06     6
    //                               0x07     7
    //                               0x08     8
    //                               0x09     9
    // Fixed
    pub const U8_CODE:          u8 = 0x0a; // 10
    pub const U16_CODE:         u8 = 0x0b; // 11
    pub const U32_CODE:         u8 = 0x0c; // 12
    pub const U64_CODE:         u8 = 0x0d; // 13
    pub const U128_CODE:        u8 = 0x0e; // 14
    //                               0x0f     15
    pub const I8_CODE:          u8 = 0x10; // 16
    pub const I16_CODE:         u8 = 0x11; // 17
    pub const I32_CODE:         u8 = 0x12; // 18
    pub const I64_CODE:         u8 = 0x13; // 19
    pub const I128_CODE:        u8 = 0x14; // 20
    //                               0x15     21
    //                               0x16     22
    //                               0x17     23
    //                               0x18     24
    //                               0x19     25
    pub const F32_CODE:         u8 = 0x1a; // 26
    pub const F64_CODE:         u8 = 0x1b; // 27
    //                               0x1c     28
    //                               0x1d     29
    // Variable
    pub const AINT_CODE:        u8 = 0x1e; // 30
    pub const ADEC_CODE:        u8 = 0x1f; // 31
    pub const C64_CODE_START:   u8 = 0x20; // 32 -> 0 i.e. the value zero
    //                               0x21     33 -> 1 byte used to represent value
    //                               0x22     34 -> 2 bytes used to represent value
    //                               0x23     35 -> 3 bytes used to represent value
    //                               0x24     36 -> 4 bytes used to represent value
    //                               0x25     37 -> 5 bytes used to represent value
    //                               0x26     38 -> 6 bytes used to represent value
    //                               0x27     39 -> 7 bytes used to represent value
    //                               0x28     40 -> 8 bytes used to represent value
    pub const C64_CODE_END:     u8 = Self::C64_CODE_START + 8; // 0x28, 40
    pub const STR_CODE:         u8 = 0x29; // 41
    //                               0x2a     42
    //                               0x2b     43
    // Molecular Kinds ========================
    // Unitary
    pub const USR_CODE:         u8 = 0x2c; // 44
    pub const BOX_CODE:         u8 = 0x2d; // 45
    pub const OPT_SOME_CODE:    u8 = 0x2e; // 46
    //                               0x2f     47
    //                               0x30     48
    //                               0x31     49
    pub const ABOX_CODE:        u8 = 0x32; // 50
    // Heterogenous
    pub const LIST_CODE:        u8 = 0x33; // 51
    pub const TUP2_CODE:        u8 = 0x34; // 52
    pub const TUP3_CODE:        u8 = 0x35; // 53
    pub const TUP4_CODE:        u8 = 0x36; // 54
    pub const TUP5_CODE:        u8 = 0x37; // 55
    pub const TUP6_CODE:        u8 = 0x38; // 56
    pub const TUP7_CODE:        u8 = 0x39; // 57
    pub const TUP8_CODE:        u8 = 0x3a; // 58
    pub const TUP9_CODE:        u8 = 0x3b; // 59
    pub const TUP10_CODE:       u8 = 0x3c; // 60
    //                               0x3d     61
    //                               0x3e     62
    pub const MAP_CODE:         u8 = 0x3f; // 63
    pub const OMAP_CODE:        u8 = 0x40; // 64
    // Homogenous
    pub const VEK_CODE:         u8 = 0x41; // 65
    //                               0x42     66
    //                               0x43     67
    // Variable length bytes - length itself is encoded as a u8, u16, ...
    pub const BU8_CODE:         u8 = 0x44; // 68
    pub const BU16_CODE:        u8 = 0x45; // 69
    pub const BU32_CODE:        u8 = 0x46; // 70
    pub const BU64_CODE:        u8 = 0x47; // 71
    pub const BC64_CODE:        u8 = 0x48; // 72
    //                               0x49     73
    //                               0x4a     74
    //                               0x4b     75
    // Fixed length bytes
    pub const B2_CODE:          u8 = 0x4c; // 76 2 bytes (16 bits)
    pub const B3_CODE:          u8 = 0x4d; // 77 3 bytes
    pub const B4_CODE:          u8 = 0x4e; // 78 4 bytes (32 bits)
    pub const B5_CODE:          u8 = 0x4f; // 79 5 bytes
    pub const B6_CODE:          u8 = 0x50; // 80 6 bytes
    pub const B7_CODE:          u8 = 0x51; // 81 7 bytes
    pub const B8_CODE:          u8 = 0x52; // 82 8 bytes (64 bits)
    pub const B9_CODE:          u8 = 0x53; // 83 9 bytes
    pub const B10_CODE:         u8 = 0x54; // 84 10 bytes
    pub const B16_CODE:         u8 = 0x55; // 85 16 bytes (128 bits)
    pub const B32_CODE:         u8 = 0x56; // 86 32 bytes (256 bits)
    //                               0x57     87
    //                               0x58     88
    // Variable length numbers
    // Not yet implemented.
    pub const VU16_CODE:        u8 = 0x59; // 89
    pub const VU32_CODE:        u8 = 0x5a; // 90
    pub const VU64_CODE:        u8 = 0x5b; // 91
    pub const VU128_CODE:       u8 = 0x5c; // 92
    pub const VI16_CODE:        u8 = 0x5d; // 93
    pub const VI32_CODE:        u8 = 0x5e; // 94
    pub const VI64_CODE:        u8 = 0x5f; // 95
    pub const VI128_CODE:       u8 = 0x60; // 96
    // Fixed length numbers - Sequential layout
    pub const TUP2_U8_CODE:     u8 = 0x61; // 97  [u8; 2]
    pub const TUP3_U8_CODE:     u8 = 0x62; // 98  [u8; 3]
    pub const TUP4_U8_CODE:     u8 = 0x63; // 99  [u8; 4]
    pub const TUP5_U8_CODE:     u8 = 0x64; // 100 [u8; 5]
    pub const TUP6_U8_CODE:     u8 = 0x65; // 101 [u8; 6]
    pub const TUP7_U8_CODE:     u8 = 0x66; // 102 [u8; 7]
    pub const TUP8_U8_CODE:     u8 = 0x67; // 103 [u8; 8]
    pub const TUP9_U8_CODE:     u8 = 0x68; // 104 [u8; 9]
    pub const TUP10_U8_CODE:    u8 = 0x69; // 105 [u8; 10]
    //                               0x6a     106 gap
    //                               0x6b     107 gap
    //                               0x6c     108 gap
    //                               0x6d     109 gap
    //                               0x6e     110 gap
    //                               0x6f     111 gap
    
    pub const TUP2_U16_CODE:    u8 = 0x70; // 112 [u16; 2]
    pub const TUP3_U16_CODE:    u8 = 0x71; // 113 [u16; 3]
    pub const TUP4_U16_CODE:    u8 = 0x72; // 114 [u16; 4]
    pub const TUP5_U16_CODE:    u8 = 0x73; // 115 [u16; 5]
    pub const TUP6_U16_CODE:    u8 = 0x74; // 116 [u16; 6]
    pub const TUP7_U16_CODE:    u8 = 0x75; // 117 [u16; 7]
    pub const TUP8_U16_CODE:    u8 = 0x76; // 118 [u16; 8]
    pub const TUP9_U16_CODE:    u8 = 0x77; // 119 [u16; 9]
    pub const TUP10_U16_CODE:   u8 = 0x78; // 120 [u16; 10]
    //                               0x79     121 gap
    //                               0x7a     122 gap
    //                               0x7b     123 gap
    //                               0x7c     124 gap
    //                               0x7d     125 gap
    //                               0x7e     126 gap
    //                               0x7f     127 gap
    
    pub const TUP2_U32_CODE:    u8 = 0x80; // 128 [u32; 2]
    pub const TUP3_U32_CODE:    u8 = 0x81; // 129 [u32; 3]
    pub const TUP4_U32_CODE:    u8 = 0x82; // 130 [u32; 4]
    pub const TUP5_U32_CODE:    u8 = 0x83; // 131 [u32; 5]
    pub const TUP6_U32_CODE:    u8 = 0x84; // 132 [u32; 6]
    pub const TUP7_U32_CODE:    u8 = 0x85; // 133 [u32; 7]
    pub const TUP8_U32_CODE:    u8 = 0x86; // 134 [u32; 8]
    pub const TUP9_U32_CODE:    u8 = 0x87; // 135 [u32; 9]
    pub const TUP10_U32_CODE:   u8 = 0x88; // 136 [u32; 10]
    //                               0x89     137 gap
    //                               0x8a     138 gap
    //                               0x8b     139 gap
    //                               0x8c     140 gap
    //                               0x8d     141 gap
    //                               0x8e     142 gap
    //                               0x8f     143 gap
    
    pub const TUP2_U64_CODE:    u8 = 0x90; // 144 [u64; 2]
    pub const TUP3_U64_CODE:    u8 = 0x91; // 145 [u64; 3]
    pub const TUP4_U64_CODE:    u8 = 0x92; // 146 [u64; 4]
    pub const TUP5_U64_CODE:    u8 = 0x93; // 147 [u64; 5]
    pub const TUP6_U64_CODE:    u8 = 0x94; // 148 [u64; 6]
    pub const TUP7_U64_CODE:    u8 = 0x95; // 149 [u64; 7]
    pub const TUP8_U64_CODE:    u8 = 0x96; // 150 [u64; 8]
    pub const TUP9_U64_CODE:    u8 = 0x97; // 151 [u64; 9]
    pub const TUP10_U64_CODE:   u8 = 0x98; // 152 [u64; 10]
    //                               0x99     153 gap
    //                               0x9a     154 gap
    //                               0x9b     155 gap
    //                               0x9c     156 gap
    //                               0x9d     157 gap
    //                               0x9e     158 gap
    //                               0x9f     159 gap
    
    pub const TUP2_I8_CODE:     u8 = 0xa0; // 160 [i8; 2]
    pub const TUP3_I8_CODE:     u8 = 0xa1; // 161 [i8; 3]
    pub const TUP4_I8_CODE:     u8 = 0xa2; // 162 [i8; 4]
    pub const TUP5_I8_CODE:     u8 = 0xa3; // 163 [i8; 5]
    pub const TUP6_I8_CODE:     u8 = 0xa4; // 164 [i8; 6]
    pub const TUP7_I8_CODE:     u8 = 0xa5; // 165 [i8; 7]
    pub const TUP8_I8_CODE:     u8 = 0xa6; // 166 [i8; 8]
    pub const TUP9_I8_CODE:     u8 = 0xa7; // 167 [i8; 9]
    pub const TUP10_I8_CODE:    u8 = 0xa8; // 168 [i8; 10]
    //                               0xa9     169 gap
    //                               0xaa     170 gap
    //                               0xab     171 gap
    //                               0xac     172 gap
    //                               0xad     173 gap
    //                               0xae     174 gap
    //                               0xaf     175 gap
    
    pub const TUP2_I16_CODE:    u8 = 0xb0; // 176 [i16; 2]
    pub const TUP3_I16_CODE:    u8 = 0xb1; // 177 [i16; 3]
    pub const TUP4_I16_CODE:    u8 = 0xb2; // 178 [i16; 4]
    pub const TUP5_I16_CODE:    u8 = 0xb3; // 179 [i16; 5]
    pub const TUP6_I16_CODE:    u8 = 0xb4; // 180 [i16; 6]
    pub const TUP7_I16_CODE:    u8 = 0xb5; // 181 [i16; 7]
    pub const TUP8_I16_CODE:    u8 = 0xb6; // 182 [i16; 8]
    pub const TUP9_I16_CODE:    u8 = 0xb7; // 183 [i16; 9]
    pub const TUP10_I16_CODE:   u8 = 0xb8; // 184 [i16; 10]
    //                               0xb9     185 gap
    //                               0xba     186 gap
    //                               0xbb     187 gap
    //                               0xbc     188 gap
    //                               0xbd     189 gap
    //                               0xbe     190 gap
    //                               0xbf     191 gap
    
    pub const TUP2_I32_CODE:    u8 = 0xc0; // 192 [i32; 2]
    pub const TUP3_I32_CODE:    u8 = 0xc1; // 193 [i32; 3]
    pub const TUP4_I32_CODE:    u8 = 0xc2; // 194 [i32; 4]
    pub const TUP5_I32_CODE:    u8 = 0xc3; // 195 [i32; 5]
    pub const TUP6_I32_CODE:    u8 = 0xc4; // 196 [i32; 6]
    pub const TUP7_I32_CODE:    u8 = 0xc5; // 197 [i32; 7]
    pub const TUP8_I32_CODE:    u8 = 0xc6; // 198 [i32; 8]
    pub const TUP9_I32_CODE:    u8 = 0xc7; // 199 [i32; 9]
    pub const TUP10_I32_CODE:   u8 = 0xc8; // 200 [i32; 10]
    //                               0xc9     201 gap
    //                               0xca     202 gap
    //                               0xcb     203 gap
    //                               0xcc     204 gap
    //                               0xcd     205 gap
    //                               0xce     206 gap
    //                               0xcf     207 gap
    
    pub const TUP2_I64_CODE:    u8 = 0xd0; // 208 [i64; 2]
    pub const TUP3_I64_CODE:    u8 = 0xd1; // 209 [i64; 3]
    pub const TUP4_I64_CODE:    u8 = 0xd2; // 210 [i64; 4]
    pub const TUP5_I64_CODE:    u8 = 0xd3; // 211 [i64; 5]
    pub const TUP6_I64_CODE:    u8 = 0xd4; // 212 [i64; 6]
    pub const TUP7_I64_CODE:    u8 = 0xd5; // 213 [i64; 7]
    pub const TUP8_I64_CODE:    u8 = 0xd6; // 214 [i64; 8]
    pub const TUP9_I64_CODE:    u8 = 0xd7; // 215 [i64; 9]
    pub const TUP10_I64_CODE:   u8 = 0xd8; // 216 [i64; 10]

    pub const TUP_SERIES_START:     u8 = Self::TUP2_CODE;
    pub const BYTE_SERIES_START:    u8 = Self::B2_CODE;
    pub const TUP_U8_SERIES_START:  u8 = Self::TUP2_U8_CODE;
    pub const TUP_U16_SERIES_START: u8 = Self::TUP2_U16_CODE;
    pub const TUP_U32_SERIES_START: u8 = Self::TUP2_U32_CODE;
    pub const TUP_U64_SERIES_START: u8 = Self::TUP2_U64_CODE;
    pub const TUP_I8_SERIES_START:  u8 = Self::TUP2_I8_CODE;
    pub const TUP_I16_SERIES_START: u8 = Self::TUP2_I16_CODE;
    pub const TUP_I32_SERIES_START: u8 = Self::TUP2_I32_CODE;
    pub const TUP_I64_SERIES_START: u8 = Self::TUP2_I64_CODE;
}
