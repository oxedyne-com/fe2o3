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
    // Fixed length numbers
    pub const TUP2_U16_CODE:    u8 = 0x61; // 97 i.e. [u16, u16] 
    pub const TUP3_U16_CODE:    u8 = 0x62; // 98 [u16, u16, u16]
    pub const TUP4_U16_CODE:    u8 = 0x63; // 99 ...
    pub const TUP5_U16_CODE:    u8 = 0x64; // 100
    pub const TUP6_U16_CODE:    u8 = 0x65; // 101
    pub const TUP7_U16_CODE:    u8 = 0x66; // 102
    pub const TUP8_U16_CODE:    u8 = 0x67; // 103
    pub const TUP9_U16_CODE:    u8 = 0x68; // 104
    pub const TUP10_U16_CODE:   u8 = 0x69; // 105
    
    pub const TUP2_U32_CODE:    u8 = 0x6a; // 106 i.e. [u32, u32] 
    pub const TUP3_U32_CODE:    u8 = 0x6b; // 107
    pub const TUP4_U32_CODE:    u8 = 0x6c; // 108
    pub const TUP5_U32_CODE:    u8 = 0x6d; // 109
    pub const TUP6_U32_CODE:    u8 = 0x6e; // 110
    pub const TUP7_U32_CODE:    u8 = 0x6f; // 111
    pub const TUP8_U32_CODE:    u8 = 0x70; // 112
    pub const TUP9_U32_CODE:    u8 = 0x71; // 113
    pub const TUP10_U32_CODE:   u8 = 0x72; // 114
    
    pub const TUP2_U64_CODE:    u8 = 0x73; // 115 i.e. [u64, u64] 
    pub const TUP3_U64_CODE:    u8 = 0x74; // 116
    pub const TUP4_U64_CODE:    u8 = 0x75; // 117
    pub const TUP5_U64_CODE:    u8 = 0x76; // 118
    pub const TUP6_U64_CODE:    u8 = 0x77; // 119
    pub const TUP7_U64_CODE:    u8 = 0x78; // 120
    pub const TUP8_U64_CODE:    u8 = 0x79; // 121
    pub const TUP9_U64_CODE:    u8 = 0x7a; // 122
    pub const TUP10_U64_CODE:   u8 = 0x7b; // 123
    //                               0x7c     124
    //                               0x7d     125
    //                               0x7e     126
    //                               0x7f     127
    //                               0x80     128
    //                               0x81     129
    //                               0x82     130
    //                               0x83     131
    //                               0x84     132
    //                               0x85     133
    //                               
    pub const TUP2_I8_CODE:     u8 = 0x86; // 134
    pub const TUP3_I8_CODE:     u8 = 0x87; // 135
    pub const TUP4_I8_CODE:     u8 = 0x88; // 136
    pub const TUP5_I8_CODE:     u8 = 0x89; // 137
    pub const TUP6_I8_CODE:     u8 = 0x8a; // 138
    pub const TUP7_I8_CODE:     u8 = 0x8b; // 139
    pub const TUP8_I8_CODE:     u8 = 0x8c; // 140
    pub const TUP9_I8_CODE:     u8 = 0x8d; // 141
    pub const TUP10_I8_CODE:    u8 = 0x8e; // 142
    
    pub const TUP2_I16_CODE:    u8 = 0x8f; // 143
    pub const TUP3_I16_CODE:    u8 = 0x90; // 144
    pub const TUP4_I16_CODE:    u8 = 0x91; // 145
    pub const TUP5_I16_CODE:    u8 = 0x92; // 146
    pub const TUP6_I16_CODE:    u8 = 0x93; // 147
    pub const TUP7_I16_CODE:    u8 = 0x94; // 148
    pub const TUP8_I16_CODE:    u8 = 0x95; // 149
    pub const TUP9_I16_CODE:    u8 = 0x96; // 150
    pub const TUP10_I16_CODE:   u8 = 0x97; // 151
    
    pub const TUP2_I32_CODE:    u8 = 0x98; // 152
    pub const TUP3_I32_CODE:    u8 = 0x99; // 153
    pub const TUP4_I32_CODE:    u8 = 0x9a; // 154
    pub const TUP5_I32_CODE:    u8 = 0x9b; // 155
    pub const TUP6_I32_CODE:    u8 = 0x9c; // 156
    pub const TUP7_I32_CODE:    u8 = 0x9d; // 157
    pub const TUP8_I32_CODE:    u8 = 0x9e; // 158
    pub const TUP9_I32_CODE:    u8 = 0x9f; // 159
    pub const TUP10_I32_CODE:   u8 = 0xa0; // 160
    
    pub const TUP2_I64_CODE:    u8 = 0xa1; // 161
    pub const TUP3_I64_CODE:    u8 = 0xa2; // 162
    pub const TUP4_I64_CODE:    u8 = 0xa3; // 163
    pub const TUP5_I64_CODE:    u8 = 0xa4; // 164
    pub const TUP6_I64_CODE:    u8 = 0xa5; // 165
    pub const TUP7_I64_CODE:    u8 = 0xa6; // 166
    pub const TUP8_I64_CODE:    u8 = 0xa7; // 167
    pub const TUP9_I64_CODE:    u8 = 0xa8; // 168
    pub const TUP10_I64_CODE:   u8 = 0xa9; // 169

    pub const TUP_SERIES_START:     u8 = Self::TUP2_CODE;
    pub const BYTE_SERIES_START:    u8 = Self::B2_CODE;
    pub const TUP_U16_SERIES_START: u8 = Self::TUP2_U16_CODE;
    pub const TUP_U32_SERIES_START: u8 = Self::TUP2_U32_CODE;
    pub const TUP_U64_SERIES_START: u8 = Self::TUP2_U64_CODE;
}
