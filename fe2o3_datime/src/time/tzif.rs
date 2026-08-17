//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use std::{
    fs,
    io::{Cursor, Read},
    path::Path,
};

/// RFC 8536, versions 1 to 3.
pub struct TZifParser {
    data: Vec<u8>,
    timezone_data: Option<TZifData>,
}

#[derive(Clone, Debug)]
pub struct TZifData {
    pub version: u8, // 1, 2 or 3
    pub transition_times: Vec<i64>, // UTC seconds since the Unix epoch
    pub transition_types: Vec<u8>, // indices into local_time_types
    pub local_time_types: Vec<LocalTimeType>,
    pub abbreviations: String,
    pub leap_seconds: Vec<LeapSecond>,
    pub standard_wall_indicators: Vec<bool>,
    pub ut_local_indicators: Vec<bool>,
    pub posix_tz_string: Option<String>, // version 2 and later
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalTimeType {
    pub utc_offset: i32, // seconds east of UTC
    pub is_dst: bool,
    pub abbreviation_index: usize, // into abbreviations
}

#[derive(Clone, Debug, PartialEq)]
pub struct LeapSecond {
    pub transition_time: i64, // UTC seconds since the Unix epoch
    pub correction: i32, // cumulative, not the step
}

/// A local time may be reached twice or not at all across a DST transition.
#[derive(Clone, Debug, PartialEq)]
pub enum LocalTimeResult<T> {
    Single(T),
    Ambiguous(T, T), // the autumn fold: (standard, daylight)
    None, // the spring gap, where the local time never occurs
}

impl TZifParser {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            timezone_data: None,
        }
    }

    pub fn load_from_file<P: AsRef<Path>>(&mut self, path: P) -> Outcome<()> {
        self.data = res!(fs::read(path.as_ref()).map_err(|e| 
            err!("Failed to read TZif file {:?}: {}", path.as_ref(), e; IO, File)));
        self.parse()
    }

    pub fn load_from_bytes(&mut self, data: &[u8]) -> Outcome<()> {
        self.data = data.to_vec();
        self.parse()
    }

    pub fn timezone_data(&self) -> Option<&TZifData> {
        self.timezone_data.as_ref()
    }

    fn parse(&mut self) -> Outcome<()> {
        if self.data.len() < 44 {
            return Err(err!("TZif file too short: {} bytes", self.data.len(); Invalid, Input));
        }

        let mut cursor = Cursor::new(&self.data);
        let header = res!(self.parse_header(&mut cursor));
        
        // Parse version 1 data first (required for all versions)
        let v1_data = res!(self.parse_data_block(&mut cursor, &header, false));
        
        // For version 2+ files, parse the second data block with 64-bit timestamps
        let timezone_data = if header.version >= 2 {
            // Skip version 1 data and parse version 2+ header and data
            let v2_header = res!(self.parse_header(&mut cursor));
            let v2_data = res!(self.parse_data_block(&mut cursor, &v2_header, true));
            
            // Parse POSIX TZ string footer
            let posix_tz_string = res!(self.parse_posix_footer(&mut cursor));
            
            TZifData {
                version: header.version,
                posix_tz_string: Some(posix_tz_string),
                ..v2_data
            }
        } else {
            TZifData {
                version: header.version,
                posix_tz_string: None,
                ..v1_data
            }
        };

        self.timezone_data = Some(timezone_data);
        Ok(())
    }

    fn parse_header(&self, cursor: &mut Cursor<&Vec<u8>>) -> Outcome<TZifHeader> {
        let mut magic = [0u8; 4];
        res!(cursor.read_exact(&mut magic).map_err(|e| 
            err!("Failed to read magic number: {}", e; IO)));
        
        if &magic != b"TZif" {
            return Err(err!("Invalid TZif magic number: {:?}", magic; Invalid, Input));
        }

        let mut version_byte = [0u8; 1];
        res!(cursor.read_exact(&mut version_byte).map_err(|e| 
            err!("Failed to read version: {}", e; IO)));
        
        let version = match version_byte[0] {
            0 => 1,
            b'2' => 2,
            b'3' => 3,
            v => return Err(err!("Unsupported TZif version: {}", v; Invalid, Input)),
        };

        // Skip reserved bytes (15 bytes)
        let mut reserved = [0u8; 15];
        res!(cursor.read_exact(&mut reserved).map_err(|e| 
            err!("Failed to read reserved bytes: {}", e; IO)));

        // Read counts (6 * 4 bytes = 24 bytes)
        let tzh_utcnt = res!(read_u32_be(cursor));     // UT/local indicators
        let tzh_stdcnt = res!(read_u32_be(cursor));    // standard/wall indicators
        let tzh_leapcnt = res!(read_u32_be(cursor));   // leap second records
        let tzh_timecnt = res!(read_u32_be(cursor));   // transition times
        let tzh_typecnt = res!(read_u32_be(cursor));   // local time types
        let tzh_charcnt = res!(read_u32_be(cursor));   // abbreviation characters

        Ok(TZifHeader {
            version,
            tzh_utcnt,
            tzh_stdcnt,
            tzh_leapcnt,
            tzh_timecnt,
            tzh_typecnt,
            tzh_charcnt,
        })
    }

    fn parse_data_block(&self, cursor: &mut Cursor<&Vec<u8>>, header: &TZifHeader, is_64bit: bool) -> Outcome<TZifData> {
        // Parse transition times
        let mut transition_times = Vec::with_capacity(header.tzh_timecnt as usize);
        for _ in 0..header.tzh_timecnt {
            let time = if is_64bit {
                res!(read_i64_be(cursor))
            } else {
                res!(read_i32_be(cursor)) as i64
            };
            transition_times.push(time);
        }

        // Parse transition types
        let mut transition_types = Vec::with_capacity(header.tzh_timecnt as usize);
        for _ in 0..header.tzh_timecnt {
            let mut byte = [0u8; 1];
            res!(cursor.read_exact(&mut byte).map_err(|e| 
                err!("Failed to read transition type: {}", e; IO)));
            transition_types.push(byte[0]);
        }

        // Parse local time types
        let mut local_time_types = Vec::with_capacity(header.tzh_typecnt as usize);
        for _ in 0..header.tzh_typecnt {
            let utc_offset = res!(read_i32_be(cursor));
            
            let mut is_dst_byte = [0u8; 1];
            res!(cursor.read_exact(&mut is_dst_byte).map_err(|e| 
                err!("Failed to read DST flag: {}", e; IO)));
            let is_dst = is_dst_byte[0] != 0;
            
            let mut abbrev_index_byte = [0u8; 1];
            res!(cursor.read_exact(&mut abbrev_index_byte).map_err(|e| 
                err!("Failed to read abbreviation index: {}", e; IO)));
            let abbreviation_index = abbrev_index_byte[0] as usize;
            
            local_time_types.push(LocalTimeType {
                utc_offset,
                is_dst,
                abbreviation_index,
            });
        }

        // Parse abbreviations
        let mut abbrev_data = vec![0u8; header.tzh_charcnt as usize];
        res!(cursor.read_exact(&mut abbrev_data).map_err(|e| 
            err!("Failed to read abbreviations: {}", e; IO)));
        let abbreviations = String::from_utf8_lossy(&abbrev_data).to_string();

        // Parse leap seconds
        let mut leap_seconds = Vec::with_capacity(header.tzh_leapcnt as usize);
        for _ in 0..header.tzh_leapcnt {
            let transition_time = if is_64bit {
                res!(read_i64_be(cursor))
            } else {
                res!(read_i32_be(cursor)) as i64
            };
            let correction = res!(read_i32_be(cursor));
            leap_seconds.push(LeapSecond { transition_time, correction });
        }

        // Parse standard/wall indicators
        let mut standard_wall_indicators = Vec::with_capacity(header.tzh_stdcnt as usize);
        for _ in 0..header.tzh_stdcnt {
            let mut byte = [0u8; 1];
            res!(cursor.read_exact(&mut byte).map_err(|e| 
                err!("Failed to read standard/wall indicator: {}", e; IO)));
            standard_wall_indicators.push(byte[0] != 0);
        }

        // Parse UT/local indicators
        let mut ut_local_indicators = Vec::with_capacity(header.tzh_utcnt as usize);
        for _ in 0..header.tzh_utcnt {
            let mut byte = [0u8; 1];
            res!(cursor.read_exact(&mut byte).map_err(|e| 
                err!("Failed to read UT/local indicator: {}", e; IO)));
            ut_local_indicators.push(byte[0] != 0);
        }

        Ok(TZifData {
            version: header.version,
            transition_times,
            transition_types,
            local_time_types,
            abbreviations,
            leap_seconds,
            standard_wall_indicators,
            ut_local_indicators,
            posix_tz_string: None,
        })
    }

    fn parse_posix_footer(&self, cursor: &mut Cursor<&Vec<u8>>) -> Outcome<String> {
        // Skip newline
        let mut newline = [0u8; 1];
        res!(cursor.read_exact(&mut newline).map_err(|e| 
            err!("Failed to read newline before POSIX string: {}", e; IO)));
        
        if newline[0] != b'\n' {
            return Err(err!("Expected newline before POSIX string, got: {}", newline[0]; Invalid, Input));
        }

        // Read until final newline
        let mut posix_data = Vec::new();
        let mut byte = [0u8; 1];
        
        loop {
            match cursor.read_exact(&mut byte) {
                Ok(()) => {
                    if byte[0] == b'\n' {
                        break;
                    }
                    posix_data.push(byte[0]);
                },
                Err(_) => break, // EOF
            }
        }

        Ok(String::from_utf8_lossy(&posix_data).to_string())
    }
}

#[derive(Debug)]
struct TZifHeader {
    version: u8,
    tzh_utcnt: u32,    // UT/local indicators count
    tzh_stdcnt: u32,   // standard/wall indicators count  
    tzh_leapcnt: u32,  // leap second records count
    tzh_timecnt: u32,  // transition times count
    tzh_typecnt: u32,  // local time types count
    tzh_charcnt: u32,  // abbreviation characters count
}

impl TZifData {
    pub fn get_abbreviation(&self, local_time_type: &LocalTimeType) -> Outcome<&str> {
        if local_time_type.abbreviation_index >= self.abbreviations.len() {
            return Err(err!(
                "Abbreviation index {} out of bounds (len: {})", 
                local_time_type.abbreviation_index, self.abbreviations.len(); 
                Invalid, Input
            ));
        }

        let abbrev_start = local_time_type.abbreviation_index;
        let abbrev_end = self.abbreviations[abbrev_start..]
            .find('\0')
            .map(|pos| abbrev_start + pos)
            .unwrap_or(self.abbreviations.len());

        Ok(&self.abbreviations[abbrev_start..abbrev_end])
    }

    pub fn utc_to_local(&self, utc_timestamp: i64) -> LocalTimeResult<(i64, &LocalTimeType)> {
        // Find the applicable transition
        let transition_index = self.transition_times
            .binary_search(&utc_timestamp)
            .unwrap_or_else(|insert_pos| {
                if insert_pos == 0 { 0 } else { insert_pos - 1 }
            });

        if transition_index >= self.transition_types.len() {
            return LocalTimeResult::None;
        }

        let type_index = self.transition_types[transition_index] as usize;
        
        if type_index >= self.local_time_types.len() {
            return LocalTimeResult::None;
        }

        let local_time_type = &self.local_time_types[type_index];
        let local_timestamp = utc_timestamp + local_time_type.utc_offset as i64;

        LocalTimeResult::Single((local_timestamp, local_time_type))
    }

    pub fn local_to_utc(&self, local_timestamp: i64) -> LocalTimeResult<(i64, &LocalTimeType)> {
        // This is more complex due to DST transitions creating ambiguous or invalid times
        let mut candidates = Vec::new();

        // Check all possible timezone rules around this time
        for (i, &_transition_time) in self.transition_times.iter().enumerate() {
            if i >= self.transition_types.len() {
                continue;
            }

            let type_index = self.transition_types[i] as usize;
            if type_index >= self.local_time_types.len() {
                continue;
            }

            let local_time_type = &self.local_time_types[type_index];
            let candidate_utc = local_timestamp - local_time_type.utc_offset as i64;

            // Check if this UTC time would produce the given local time
            if let LocalTimeResult::Single((computed_local, _)) = self.utc_to_local(candidate_utc) {
                if computed_local == local_timestamp {
                    candidates.push((candidate_utc, local_time_type));
                }
            }

            // Only check transitions around the target time (within 24 hours)
            if (candidate_utc - local_timestamp).abs() > 86400 {
                continue;
            }
        }

        match candidates.len() {
            0 => LocalTimeResult::None,
            1 => LocalTimeResult::Single(candidates[0]),
            2 => LocalTimeResult::Ambiguous(candidates[0], candidates[1]),
            _ => {
                // Multiple candidates - take the first valid one
                LocalTimeResult::Single(candidates[0])
            }
        }
    }

    pub fn get_offset_at_utc(&self, utc_timestamp: i64) -> Outcome<i32> {
        match self.utc_to_local(utc_timestamp) {
            LocalTimeResult::Single((_, local_time_type)) => Ok(local_time_type.utc_offset),
            _ => Err(err!("Could not determine offset for UTC timestamp {}", utc_timestamp; Invalid, Input)),
        }
    }

    pub fn is_dst_at_utc(&self, utc_timestamp: i64) -> Outcome<bool> {
        match self.utc_to_local(utc_timestamp) {
            LocalTimeResult::Single((_, local_time_type)) => Ok(local_time_type.is_dst),
            _ => Err(err!("Could not determine DST status for UTC timestamp {}", utc_timestamp; Invalid, Input)),
        }
    }
}

// Helper functions for reading big-endian values

fn read_u32_be(cursor: &mut Cursor<&Vec<u8>>) -> Outcome<u32> {
    let mut bytes = [0u8; 4];
    res!(cursor.read_exact(&mut bytes).map_err(|e| 
        err!("Failed to read u32: {}", e; IO)));
    Ok(u32::from_be_bytes(bytes))
}

fn read_i32_be(cursor: &mut Cursor<&Vec<u8>>) -> Outcome<i32> {
    let mut bytes = [0u8; 4];
    res!(cursor.read_exact(&mut bytes).map_err(|e| 
        err!("Failed to read i32: {}", e; IO)));
    Ok(i32::from_be_bytes(bytes))
}

fn read_i64_be(cursor: &mut Cursor<&Vec<u8>>) -> Outcome<i64> {
    let mut bytes = [0u8; 8];
    res!(cursor.read_exact(&mut bytes).map_err(|e| 
        err!("Failed to read i64: {}", e; IO)));
    Ok(i64::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tzif_header_parsing() {
        // Create minimal valid TZif header
        let mut data = Vec::new();
        data.extend_from_slice(b"TZif");  // Magic
        data.push(b'2');                 // Version 2
        data.extend_from_slice(&[0u8; 15]); // Reserved
        // Counts (all zero for minimal test)
        data.extend_from_slice(&[0u8; 24]); // 6 * 4 bytes of counts
        
        let mut parser = TZifParser::new();
        assert!(parser.load_from_bytes(&data).is_ok());
    }

    #[test]
    fn test_local_time_type() {
        let ltt = LocalTimeType {
            utc_offset: -18000, // EST: -5 hours
            is_dst: false,
            abbreviation_index: 0,
        };
        
        assert_eq!(ltt.utc_offset, -18000);
        assert!(!ltt.is_dst);
    }

    #[test]
    fn test_leap_second() {
        let leap = LeapSecond {
            transition_time: 78796800, // 1972-07-01
            correction: 1,
        };
        
        assert_eq!(leap.transition_time, 78796800);
        assert_eq!(leap.correction, 1);
    }
}