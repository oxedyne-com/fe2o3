/// Network Time Protocol (NTP) implementation for fe2o3_datime
/// 
/// This module provides functionality to synchronise time with NTP servers,
/// query network time, and calculate clock offset and drift.
/// 
/// NTP Protocol Reference: RFC 5905
/// 
/// # Examples
/// 
/// ```ignore
/// use oxedyne_fe2o3_datime::time::ntp::NtpClient;
/// 
/// let client = NtpClient::new("pool.ntp.org", 123)?;
/// let ntp_time = client.query_time()?;
/// println!("Network time: {}", ntp_time.network_time);
/// println!("Offset: {} ms", ntp_time.offset_millis);
/// ```

use oxedyne_fe2o3_core::prelude::*;
use std::{
    net::{UdpSocket, ToSocketAddrs},
    time::{Duration, SystemTime, UNIX_EPOCH},
    thread,
    sync::{Arc, mpsc},
};

/// NTP packet format as defined in RFC 5905
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct NtpPacket {
    /// Leap Indicator (2 bits), Version Number (3 bits), Mode (3 bits)
    li_vn_mode: u8,
    /// Stratum level of the local clock
    stratum: u8,
    /// Maximum interval between successive messages
    poll: i8,
    /// Precision of the local clock
    precision: i8,
    /// Total roundtrip delay to primary reference source
    root_delay: u32,
    /// Maximum error due to clock frequency tolerance
    root_dispersion: u32,
    /// Reference clock identifier
    ref_id: u32,
    /// Reference timestamp
    ref_timestamp: u64,
    /// Origin timestamp (T1)
    origin_timestamp: u64,
    /// Receive timestamp (T2)
    receive_timestamp: u64,
    /// Transmit timestamp (T3)
    transmit_timestamp: u64,
}

impl NtpPacket {
    /// Creates a new NTP request packet
    fn new_request() -> Self {
        NtpPacket {
            li_vn_mode: 0x1B, // Version 3, Mode 3 (client)
            stratum: 0,
            poll: 6,  // 64 seconds
            precision: -6, // ~15.6 ms precision
            root_delay: 0,
            root_dispersion: 0,
            ref_id: 0,
            ref_timestamp: 0,
            origin_timestamp: 0,
            receive_timestamp: 0,
            transmit_timestamp: system_time_to_ntp_timestamp(SystemTime::now()),
        }
    }

    /// Converts to byte array for network transmission
    fn to_bytes(&self) -> [u8; 48] {
        let mut bytes = [0u8; 48];
        bytes[0] = self.li_vn_mode;
        bytes[1] = self.stratum;
        bytes[2] = self.poll as u8;
        bytes[3] = self.precision as u8;
        bytes[4..8].copy_from_slice(&self.root_delay.to_be_bytes());
        bytes[8..12].copy_from_slice(&self.root_dispersion.to_be_bytes());
        bytes[12..16].copy_from_slice(&self.ref_id.to_be_bytes());
        bytes[16..24].copy_from_slice(&self.ref_timestamp.to_be_bytes());
        bytes[24..32].copy_from_slice(&self.origin_timestamp.to_be_bytes());
        bytes[32..40].copy_from_slice(&self.receive_timestamp.to_be_bytes());
        bytes[40..48].copy_from_slice(&self.transmit_timestamp.to_be_bytes());
        bytes
    }

    /// Creates from byte array received from network
    fn from_bytes(bytes: &[u8; 48]) -> Self {
        NtpPacket {
            li_vn_mode: bytes[0],
            stratum: bytes[1],
            poll: bytes[2] as i8,
            precision: bytes[3] as i8,
            root_delay: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            root_dispersion: u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            ref_id: u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
            ref_timestamp: u64::from_be_bytes([
                bytes[16], bytes[17], bytes[18], bytes[19],
                bytes[20], bytes[21], bytes[22], bytes[23]
            ]),
            origin_timestamp: u64::from_be_bytes([
                bytes[24], bytes[25], bytes[26], bytes[27],
                bytes[28], bytes[29], bytes[30], bytes[31]
            ]),
            receive_timestamp: u64::from_be_bytes([
                bytes[32], bytes[33], bytes[34], bytes[35],
                bytes[36], bytes[37], bytes[38], bytes[39]
            ]),
            transmit_timestamp: u64::from_be_bytes([
                bytes[40], bytes[41], bytes[42], bytes[43],
                bytes[44], bytes[45], bytes[46], bytes[47]
            ]),
        }
    }
}

/// NTP time query result containing network time and offset information
#[derive(Debug, Clone)]
pub struct NtpTimeResult {
    /// The network time from the NTP server
    pub network_time: SystemTime,
    /// Local system time when the response was received
    pub local_time: SystemTime,
    /// Clock offset in milliseconds (positive means local clock is ahead)
    pub offset_millis: i64,
    /// Round-trip delay in milliseconds
    pub delay_millis: u64,
    /// Stratum level of the NTP server
    pub stratum: u8,
    /// Reference identifier of the server
    pub reference_id: u32,
}

/// NTP client for querying network time servers
#[derive(Debug)]
pub struct NtpClient {
    server_addr: String,
    port: u16,
    timeout: Duration,
}

impl NtpClient {
    /// Creates a new NTP client for the specified server and port
    pub fn new(server: &str, port: u16) -> Self {
        NtpClient {
            server_addr: server.to_string(),
            port,
            timeout: Duration::from_secs(5),
        }
    }

    /// Creates an NTP client with default settings (port 123)
    pub fn default(server: &str) -> Self {
        Self::new(server, 123)
    }

    /// Sets the timeout for NTP queries
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Queries the NTP server for current time
    pub fn query_time(&self) -> Outcome<NtpTimeResult> {
        // Resolve server address
        let server_addr = format!("{}:{}", self.server_addr, self.port);
        let mut addrs = res!(server_addr.to_socket_addrs()
            .map_err(|e| err!("Failed to resolve NTP server '{}': {}", server_addr, e; Invalid, Network)));
        
        let addr = res!(addrs.next()
            .ok_or_else(|| err!("No addresses found for NTP server '{}'", server_addr; Invalid, Network)));

        // Create UDP socket
        let socket = res!(UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| err!("Failed to create UDP socket: {}", e; Invalid, Network)));

        res!(socket.set_read_timeout(Some(self.timeout))
            .map_err(|e| err!("Failed to set socket timeout: {}", e; Invalid, Network)));

        // Create and send NTP request
        let request = NtpPacket::new_request();
        let request_bytes = request.to_bytes();
        let send_time = SystemTime::now();

        res!(socket.send_to(&request_bytes, addr)
            .map_err(|e| err!("Failed to send NTP request: {}", e; Invalid, Network)));

        // Receive response
        let mut response_bytes = [0u8; 48];
        let (bytes_received, _) = res!(socket.recv_from(&mut response_bytes)
            .map_err(|e| err!("Failed to receive NTP response: {}", e; Invalid, Network, Timeout)));

        if bytes_received != 48 {
            return Err(err!("Invalid NTP response size: {} bytes", bytes_received; Invalid, Network));
        }

        let receive_time = SystemTime::now();
        let response = NtpPacket::from_bytes(&response_bytes);

        // Calculate timestamps and offset
        self.calculate_time_result(request, response, send_time, receive_time)
    }

    /// Queries multiple NTP servers and returns the best result
    pub fn query_multiple_servers(servers: &[&str], max_concurrent: usize) -> Outcome<NtpTimeResult> {
        if servers.is_empty() {
            return Err(err!("No NTP servers provided"; Invalid, Input));
        }

        let max_queries = std::cmp::min(servers.len(), max_concurrent);
        let (tx, rx) = mpsc::channel();
        
        // Launch concurrent queries
        let mut handles = Vec::new();
        for &server in servers.iter().take(max_queries) {
            let tx = tx.clone();
            let server_owned = server.to_string(); // Convert to owned string
            let handle = thread::spawn(move || {
                let client = NtpClient::default(&server_owned);
                let result = client.query_time();
                let _ = tx.send(result); // Ignore send errors
            });
            handles.push(handle);
        }
        
        // Drop the original sender to signal no more messages
        drop(tx);
        
        // Collect results with timeout
        let mut results = Vec::new();
        let timeout = Duration::from_secs(5); // 5 second timeout total
        let start_time = SystemTime::now();
        
        while let Ok(result) = rx.recv() {
            if let Ok(ntp_result) = result {
                results.push(ntp_result);
            }
            
            // Check if we have enough results or timeout
            if results.len() >= 3 || start_time.elapsed().unwrap_or(timeout) >= timeout {
                break;
            }
        }
        
        // Wait for all threads to complete (with timeout)
        for handle in handles {
            let _ = handle.join(); // Ignore join errors
        }

        if results.is_empty() {
            return Err(err!("All NTP servers failed to respond"; Invalid, Network));
        }

        // Select the best result (lowest stratum, then lowest delay)
        results.sort_by(|a, b| {
            a.stratum.cmp(&b.stratum)
                .then(a.delay_millis.cmp(&b.delay_millis))
        });

        match results.into_iter().next() {
            Some(result) => Ok(result),
            None => Err(err!("Internal error: no NTP results after filtering"; Invalid, Bug)),
        }
    }

    /// Calculates time result from NTP packet exchange
    fn calculate_time_result(
        &self,
        request: NtpPacket,
        response: NtpPacket,
        send_time: SystemTime,
        receive_time: SystemTime
    ) -> Outcome<NtpTimeResult> {
        // Convert NTP timestamps to SystemTime
        let t1 = system_time_to_ntp_timestamp(send_time);    // Origin time
        let t2 = response.receive_timestamp;                 // Receive time at server
        let t3 = response.transmit_timestamp;                // Transmit time at server
        let t4 = system_time_to_ntp_timestamp(receive_time); // Destination time

        // Calculate offset and delay using NTP algorithm
        // Offset = ((T2 - T1) + (T3 - T4)) / 2
        // Delay = (T4 - T1) - (T3 - T2)
        
        let offset_ntp = (((t2 as i64) - (t1 as i64)) + ((t3 as i64) - (t4 as i64))) / 2;
        let delay_ntp = ((t4 as i64) - (t1 as i64)) - ((t3 as i64) - (t2 as i64));

        // Convert to milliseconds
        let offset_millis = ntp_timestamp_to_millis(offset_ntp as u64) as i64;
        let delay_millis = std::cmp::max(0, ntp_timestamp_to_millis(delay_ntp as u64)) as u64;

        // Calculate network time
        let network_time = ntp_timestamp_to_system_time(t3);

        Ok(NtpTimeResult {
            network_time,
            local_time: receive_time,
            offset_millis,
            delay_millis,
            stratum: response.stratum,
            reference_id: response.ref_id,
        })
    }
}

/// Default NTP server pool for quick access
pub struct NtpPool;

impl NtpPool {
    /// Public NTP server pool
    pub const PUBLIC: &'static [&'static str] = &[
        "pool.ntp.org",
        "time.nist.gov",
        "time.google.com",
        "time.cloudflare.com",
    ];

    /// Queries the public NTP pool for current time
    pub fn query_time() -> Outcome<NtpTimeResult> {
        NtpClient::query_multiple_servers(Self::PUBLIC, 4)
    }

    /// Queries a specific subset of reliable NTP servers
    pub fn query_reliable() -> Outcome<NtpTimeResult> {
        let reliable_servers = &[
            "time.google.com",
            "time.cloudflare.com", 
            "pool.ntp.org",
        ];
        NtpClient::query_multiple_servers(reliable_servers, 3)
    }
}

/// Utility functions for NTP timestamp conversion

/// Converts SystemTime to NTP timestamp (seconds since 1900-01-01)
fn system_time_to_ntp_timestamp(time: SystemTime) -> u64 {
    const NTP_EPOCH_OFFSET: u64 = 2_208_988_800; // Seconds from 1900 to 1970
    
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            let seconds = duration.as_secs() + NTP_EPOCH_OFFSET;
            let nanos = duration.subsec_nanos() as u64;
            let fraction = (nanos * (1u64 << 32)) / 1_000_000_000;
            (seconds << 32) | fraction
        },
        Err(_) => 0, // Handle times before Unix epoch
    }
}

/// Converts NTP timestamp to SystemTime
fn ntp_timestamp_to_system_time(ntp_timestamp: u64) -> SystemTime {
    const NTP_EPOCH_OFFSET: u64 = 2_208_988_800;
    
    let seconds = (ntp_timestamp >> 32) - NTP_EPOCH_OFFSET;
    let fraction = ntp_timestamp & 0xFFFFFFFF;
    let nanos = (fraction * 1_000_000_000) >> 32;
    
    UNIX_EPOCH + Duration::new(seconds, nanos as u32)
}

/// Converts NTP timestamp to milliseconds
fn ntp_timestamp_to_millis(ntp_timestamp: u64) -> u64 {
    let seconds = ntp_timestamp >> 32;
    let fraction = ntp_timestamp & 0xFFFFFFFF;
    let millis = (fraction * 1000) >> 32;
    seconds * 1000 + millis
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ntp_packet_serialization() {
        let packet = NtpPacket::new_request();
        let bytes = packet.to_bytes();
        let deserialised = NtpPacket::from_bytes(&bytes);
        
        assert_eq!(packet.li_vn_mode, deserialised.li_vn_mode);
        assert_eq!(packet.stratum, deserialised.stratum);
        assert_eq!(packet.transmit_timestamp, deserialised.transmit_timestamp);
    }

    #[test]
    fn test_timestamp_conversion() {
        let now = SystemTime::now();
        let ntp_ts = system_time_to_ntp_timestamp(now);
        let converted_back = ntp_timestamp_to_system_time(ntp_ts);
        
        // Should be within 1 second due to precision
        let diff = now.duration_since(converted_back).unwrap_or_else(|_| {
            converted_back.duration_since(now).unwrap_or(Duration::from_secs(0))
        });
        
        assert!(diff.as_secs() <= 1);
    }

    #[ignore] // Network test - only run manually
    #[test] 
    fn test_ntp_query() -> Outcome<()> {
        let client = NtpClient::default("pool.ntp.org");
        let result = res!(client.query_time());
        
        println!("Network time: {:?}", result.network_time);
        println!("Offset: {} ms", result.offset_millis);
        println!("Delay: {} ms", result.delay_millis);
        
        // Sanity checks
        assert!(result.delay_millis < 10000); // Less than 10 seconds delay
        assert!(result.stratum > 0 && result.stratum < 16); // Valid stratum
        Ok(())
    }

    #[ignore] // Network test - only run manually
    #[test]
    fn test_ntp_pool_query() -> Outcome<()> {
        let result = res!(NtpPool::query_reliable());
        
        println!("Pool result - Offset: {} ms, Delay: {} ms, Stratum: {}", 
                 result.offset_millis, result.delay_millis, result.stratum);
        
        assert!(result.stratum > 0);
        Ok(())
    }
}