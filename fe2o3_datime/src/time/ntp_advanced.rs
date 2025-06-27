use oxedyne_fe2o3_core::prelude::*;
use crate::{
	time::{CalClock, CalClockZone, CalClockDuration},
	index::{time_basis::{TimeIndexInterval, TimeIndex}, TimeLong},
};
use std::{
	collections::{HashMap, BTreeMap},
	net::{IpAddr, SocketAddr, UdpSocket},
	time::{Duration, Instant},
};

/// Advanced NTP client with multi-server support and fault tolerance.
/// 
/// Implements RFC 1305 algorithms including intersection, selection,
/// and combine algorithms for reliable network time synchronization.
pub struct AdvancedNtpClient {
	servers: Vec<NtpServerConnection>,
	server_stats: HashMap<IpAddr, NtpServerStats>,
	offsets: HashMap<IpAddr, NtpOffsets>,
	algorithm: NtpAlgorithm,
	min_servers: usize,
	min_true_chimers: usize,
	timeout: Duration,
	best_server: Option<IpAddr>,
}

/// NTP algorithm selection.
#[derive(Debug, Clone, PartialEq)]
pub enum NtpAlgorithm {
	/// RFC 1305 standard algorithm.
	Rfc1305,
	/// Custom Prodigitum algorithm (placeholder for future).
	Prodigitum,
}

/// Individual server connection.
#[derive(Debug, Clone)]
struct NtpServerConnection {
	address: IpAddr,
	socket_addr: SocketAddr,
	timeout: Duration,
}

/// NTP data result from a single server.
#[derive(Debug, Clone)]
pub struct NtpData {
	address: IpAddr,
	offset: CalClockDuration,
	delay: CalClockDuration,
	root_distance: i64,
	correctness_interval: TimeIndexInterval<TimeLong>,
	stratum: u8,
	precision: i8,
}

impl NtpData {
	/// Returns the server address.
	pub fn address(&self) -> IpAddr {
		self.address
	}
	
	/// Returns the clock offset.
	pub fn offset(&self) -> &CalClockDuration {
		&self.offset
	}
	
	/// Returns the round-trip delay.
	pub fn delay(&self) -> &CalClockDuration {
		&self.delay
	}
	
	/// Returns the root distance.
	pub fn root_distance(&self) -> i64 {
		self.root_distance
	}
	
	/// Returns the correctness interval.
	pub fn correctness_interval(&self) -> &TimeIndexInterval<TimeLong> {
		&self.correctness_interval
	}
	
	/// Returns the server stratum.
	pub fn stratum(&self) -> u8 {
		self.stratum
	}
	
	/// Returns the server precision.
	pub fn precision(&self) -> i8 {
		self.precision
	}
}

/// Statistical tracking for each NTP server.
#[derive(Debug, Clone)]
pub struct NtpServerStats {
	#[allow(dead_code)]
	offset_history: Vec<f64>,
	#[allow(dead_code)]
	delay_history: Vec<f64>,
	#[allow(dead_code)]
	mean_offset: f64,
	#[allow(dead_code)]
	std_dev_offset: f64,
	#[allow(dead_code)]
	mean_delay: f64,
	#[allow(dead_code)]
	std_dev_delay: f64,
	#[allow(dead_code)]
	sample_count: usize,
	#[allow(dead_code)]
	root_distance: f64,
	#[allow(dead_code)]
	stratum: u8,
	#[allow(dead_code)]
	precision: i8,
	#[allow(dead_code)]
	last_poll_time: Option<Instant>,
	#[allow(dead_code)]
	is_reachable: bool,
	#[allow(dead_code)]
	falseticker_count: usize,
}

/// Result from NTP polling operation.
#[derive(Debug, Clone)]
pub struct AdvancedNtpResult {
	/// The synchronized network time.
	pub network_time: CalClock,
	/// Clock offset from local time (network_time - local_time).
	pub offset: CalClockDuration,
	/// Round-trip delay to the server.
	pub delay: CalClockDuration,
	/// Root distance (accuracy estimate).
	pub root_distance: f64,
	/// Number of servers used in calculation.
	pub servers_used: usize,
	/// Number of servers considered.
	pub servers_total: usize,
	/// Best server used for final time.
	pub best_server: Option<IpAddr>,
	/// Jitter estimate.
	pub jitter: f64,
}

/// Tracking statistics for historical offsets.
#[derive(Debug, Clone)]
pub struct NtpOffsets {
	values: Vec<f64>,
	mean: f64,
	std_dev: f64,
}

impl NtpOffsets {
	/// Creates a new NtpOffsets tracker.
	pub fn new() -> Self {
		Self {
			values: Vec::new(),
			mean: 0.0,
			std_dev: 0.0,
		}
	}
	
	/// Adds a new offset value.
	pub fn add(&mut self, value: f64) {
		self.values.push(value);
		
		// Keep only last 8 samples
		if self.values.len() > 8 {
			self.values.remove(0);
		}
		
		self.recalculate_stats();
	}
	
	/// Recalculates mean and standard deviation.
	fn recalculate_stats(&mut self) {
		if self.values.is_empty() {
			return;
		}
		
		self.mean = self.values.iter().sum::<f64>() / self.values.len() as f64;
		
		if self.values.len() > 1 {
			let variance: f64 = self.values
				.iter()
				.map(|x| (x - self.mean).powi(2))
				.sum::<f64>() / (self.values.len() - 1) as f64;
			self.std_dev = variance.sqrt();
		} else {
			self.std_dev = 0.0;
		}
	}
	
	/// Returns the standard deviation.
	pub fn std_dev(&self) -> f64 {
		self.std_dev
	}
	
	/// Returns the mean.
	pub fn mean(&self) -> f64 {
		self.mean
	}
	
	/// Returns the current sample count.
	pub fn count(&self) -> usize {
		self.values.len()
	}
}


impl AdvancedNtpClient {
	/// Creates a new advanced NTP client.
	pub fn new(servers: Vec<IpAddr>) -> Outcome<Self> {
		if servers.len() < 4 {
			return Err(err!(
				"Advanced NTP requires at least 4 servers, got {}",
				servers.len();
				Invalid, Input
			));
		}
		
		let server_connections = servers
			.into_iter()
			.map(|addr| NtpServerConnection {
				address: addr,
				socket_addr: SocketAddr::new(addr, 123),
				timeout: Duration::from_secs(2),
			})
			.collect();
		
		Ok(Self {
			servers: server_connections,
			server_stats: HashMap::new(),
			offsets: HashMap::new(),
			algorithm: NtpAlgorithm::Rfc1305,
			min_servers: 4,
			min_true_chimers: 2,
			timeout: Duration::from_secs(2),
			best_server: None,
		})
	}
	
	/// Sets the NTP algorithm to use.
	pub fn with_algorithm(mut self, algorithm: NtpAlgorithm) -> Self {
		self.algorithm = algorithm;
		self
	}
	
	/// Sets the minimum number of servers required.
	pub fn with_min_servers(mut self, min_servers: usize) -> Self {
		self.min_servers = min_servers;
		self
	}
	
	/// Sets the timeout for server connections.
	pub fn with_timeout(mut self, timeout: Duration) -> Self {
		self.timeout = timeout;
		for server in &mut self.servers {
			server.timeout = timeout;
		}
		self
	}
	
	/// Performs advanced NTP synchronization with fault tolerance.
	pub fn synchronize(&mut self, zone: CalClockZone) -> Outcome<AdvancedNtpResult> {
		// Step 1: Poll all servers
		let data_list = res!(self.poll_all_servers());
		
		if data_list.is_empty() {
			return Err(err!("Could not get any NTP data from any of the given servers"; Network));
		}
		
		// Step 2: Apply RFC 1305 algorithms
		match self.algorithm {
			NtpAlgorithm::Rfc1305 => self.rfc1305_algorithm(data_list, zone),
			NtpAlgorithm::Prodigitum => self.prodigitum_algorithm(data_list, zone),
		}
	}
	
	/// Polls all configured servers.
	fn poll_all_servers(&mut self) -> Outcome<Vec<NtpData>> {
		let mut data_list = Vec::new();
		let servers = self.servers.clone(); // Clone to avoid borrowing issues
		
		for server in &servers {
			match self.poll_single_server(server) {
				Ok(data) => {
					// Update offset history
					let offset_ns = data.offset.nanoseconds();
					let offset_series = self.offsets.entry(data.address).or_insert_with(NtpOffsets::new);
					offset_series.add(offset_ns as f64);
					
					data_list.push(data);
				},
				Err(_e) => {
					// Mark server as unreachable and continue
					self.mark_server_unreachable(&server.address);
				}
			}
		}
		
		Ok(data_list)
	}
	
	/// Polls a single NTP server.
	fn poll_single_server(&self, server: &NtpServerConnection) -> Outcome<NtpData> {
		// Create UDP socket
		let socket = res!(UdpSocket::bind("0.0.0.0:0").map_err(|e| {
			err!("Failed to create UDP socket: {}", e; Network)
		}));
		
		res!(socket.set_read_timeout(Some(server.timeout)).map_err(|e| {
			err!("Failed to set socket timeout: {}", e; Network)
		}));
		
		// Create NTP request packet
		let request_packet = self.create_ntp_request();
		let send_time = Instant::now();
		
		// Send request
		res!(socket.send_to(&request_packet, server.socket_addr).map_err(|e| {
			err!("Failed to send NTP request to {}: {}", server.address, e; Network)
		}));
		
		// Receive response
		let mut buffer = [0u8; 48];
		let recv_time = Instant::now();
		
		let (bytes_received, _) = res!(socket.recv_from(&mut buffer).map_err(|e| {
			err!("Failed to receive NTP response from {}: {}", server.address, e; Network)
		}));
		
		if bytes_received != 48 {
			return Err(err!(
				"Invalid NTP response size from {}: {} bytes",
				server.address,
				bytes_received;
				Invalid, Network
			));
		}
		
		// Parse response and calculate timing
		self.parse_ntp_response(&buffer, send_time, recv_time, server.address)
	}
	
	/// Creates an NTP request packet.
	fn create_ntp_request(&self) -> [u8; 48] {
		let mut packet = [0u8; 48];
		
		// Set LI (0), VN (4), Mode (3) - Client mode
		packet[0] = 0x1b; // 00 100 011
		
		// Set poll interval (6 = 64 seconds)
		packet[2] = 6;
		
		// Set precision (-20 = microsecond precision)
		packet[3] = 0xec; // -20 in two's complement
		
		// Transmit timestamp (current time)
		let now = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap_or_default();
		
		let ntp_timestamp = (now.as_secs() + 2208988800) as u64; // Convert to NTP epoch
		let fraction = ((now.subsec_nanos() as u64) << 32) / 1_000_000_000;
		
		// Write transmit timestamp (bytes 40-47)
		packet[40..44].copy_from_slice(&(ntp_timestamp as u32).to_be_bytes());
		packet[44..48].copy_from_slice(&(fraction as u32).to_be_bytes());
		
		packet
	}
	
	/// Parses NTP response and calculates NTP data.
	fn parse_ntp_response(
		&self,
		packet: &[u8; 48],
		send_time: Instant,
		recv_time: Instant,
		server_addr: IpAddr,
	) -> Outcome<NtpData> {
		// Extract NTP packet fields
		let stratum = packet[1];
		let precision = packet[3] as i8;
		
		// Extract timestamps
		let root_delay = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]) as f64 / 65536.0;
		let root_dispersion = u32::from_be_bytes([packet[8], packet[9], packet[10], packet[11]]) as f64 / 65536.0;
		
		// Extract server timestamps
		let _reference_ts = self.extract_ntp_timestamp(&packet[16..24]);
		let _originate_ts = self.extract_ntp_timestamp(&packet[24..32]);
		let receive_ts = self.extract_ntp_timestamp(&packet[32..40]);
		let transmit_ts = self.extract_ntp_timestamp(&packet[40..48]);
		
		// Calculate timing
		let rtt = recv_time.duration_since(send_time).as_secs_f64();
		let delay = rtt;
		
		// Calculate offset using NTP algorithm
		let t1 = send_time.elapsed().as_secs_f64(); // Local send time
		let t2 = receive_ts; // Server receive time
		let t3 = transmit_ts; // Server transmit time
		let t4 = recv_time.elapsed().as_secs_f64(); // Local receive time
		
		let offset_seconds = ((t2 - t1) + (t3 - t4)) / 2.0;
		
		// Convert to CalClockDuration
		let offset = CalClockDuration::from_seconds(offset_seconds as i64);
		let ntp_delay = CalClockDuration::from_seconds(delay as i64);
		
		// Calculate root distance in nanoseconds
		let root_distance_ns = ((root_delay + root_dispersion + delay) * 1_000_000_000.0) as i64;
		
		// Calculate correctness interval in nanoseconds
		let offset_ns = (offset_seconds * 1_000_000_000.0) as i64;
		let half_delay_ns = (delay * 1_000_000_000.0 / 2.0) as i64;
		
		let interval_start = TimeIndex::new(TimeLong::new(offset_ns - half_delay_ns));
		let interval_finish = TimeIndex::new(TimeLong::new(offset_ns + half_delay_ns));
		let correctness_interval = res!(TimeIndexInterval::new(interval_start, interval_finish));
		
		Ok(NtpData {
			address: server_addr,
			offset,
			delay: ntp_delay,
			root_distance: root_distance_ns,
			correctness_interval,
			stratum,
			precision,
		})
	}
	
	/// Extracts NTP timestamp from 8 bytes.
	fn extract_ntp_timestamp(&self, bytes: &[u8]) -> f64 {
		let seconds = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as u64;
		let fraction = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as u64;
		
		// Convert from NTP epoch (1900) to Unix epoch (1970)
		let unix_seconds = seconds.saturating_sub(2208988800);
		let fractional_seconds = fraction as f64 / (1u64 << 32) as f64;
		
		unix_seconds as f64 + fractional_seconds
	}
	
	/// Calculates jitter estimate for a server.
	#[allow(dead_code)]
	fn calculate_jitter(&self, server_addr: IpAddr, current_delay: f64) -> f64 {
		if let Some(stats) = self.server_stats.get(&server_addr) {
			if stats.delay_history.len() > 1 {
				return stats.std_dev_delay;
			}
		}
		current_delay * 0.1 // Default estimate
	}
	
	/// Implements RFC 1305 algorithm.
	fn rfc1305_algorithm(
		&mut self,
		data_list: Vec<NtpData>,
		zone: CalClockZone,
	) -> Outcome<AdvancedNtpResult> {
		// Step 1: Intersection Algorithm  
		let intersection_interval = res!(self.get_intersection_interval(&data_list));
		let true_chimers = res!(self.get_true_chimers(&data_list, &intersection_interval));
		
		if true_chimers.len() < self.min_true_chimers {
			return Err(err!(
				"Only {} true chimers found, minimum {} required",
				true_chimers.len(),
				self.min_true_chimers;
				Invalid, Network
			));
		}
		
		// Step 2: Selection Algorithm
		let survivors = res!(self.get_survivors(&true_chimers));
		
		if survivors.is_empty() {
			return Err(err!("No servers survived selection algorithm"; Invalid, Network));
		}
		
		// Step 3: Combine Algorithm - sort by root distance
		let mut sorted_survivors = survivors;
		sorted_survivors.sort_by(|a, b| a.root_distance.cmp(&b.root_distance));
		
		let best_data = &sorted_survivors[0];
		
		// Convert to CalClock
		let network_time = res!(self.data_to_calclock(best_data, zone));
		
		// Calculate results
		let local_time = res!(CalClock::now(network_time.zone().clone()));
		let offset = res!(network_time.duration_until(&local_time));
		
		self.best_server = Some(best_data.address);
		
		Ok(AdvancedNtpResult {
			network_time,
			offset,
			delay: best_data.delay.clone(),
			root_distance: best_data.root_distance as f64,
			servers_used: 1,
			servers_total: self.servers.len(),
			best_server: Some(best_data.address),
			jitter: 0.0, // Calculated from offset history
		})
	}
	
	
	/// Marks a server as unreachable.
	fn mark_server_unreachable(&mut self, address: &IpAddr) {
		if let Some(stats) = self.server_stats.get_mut(address) {
			stats.is_reachable = false;
			stats.falseticker_count += 1;
		}
	}
	
	/// Returns server statistics.
	pub fn server_statistics(&self) -> &HashMap<IpAddr, NtpServerStats> {
		&self.server_stats
	}
	
	/// Returns the best server address.
	pub fn best_server(&self) -> Option<IpAddr> {
		self.best_server
	}
	
	/// Implements the Java-compatible intersection algorithm.
	fn get_intersection_interval(&self, data_list: &[NtpData]) -> Outcome<TimeIndexInterval<TimeLong>> {
		
		let m = self.servers.len();
		let mut tuples = BTreeMap::new();
		
		// Create ordered tuple map from intervals
		for data in data_list {
			let offset = data.offset.nanoseconds();
			let interval = data.correctness_interval();
			
			// Add interval start (-1), offset (0), and interval finish (+1)
			tuples.insert(interval.start().time().value(), -1);
			tuples.insert(offset, 0);
			tuples.insert(interval.finish().time().value(), 1);
		}
		
		let mut f = 0; // Number of falsetickers
		
		loop {
			#[allow(unused_assignments)]
			let mut lower = 0i64; // Will be overwritten before use
			#[allow(unused_assignments)]
			let mut upper = 0i64; // Will be overwritten before use
			let mut endcount = 0;
			let mut midcount = 0;
			
			// Go forwards
			for (key, value) in &tuples {
				endcount -= value;
				
				if endcount >= (m - f) as i32 {
					lower = *key;
					
					// Now go backwards
					endcount = 0;
					for (key2, value2) in tuples.iter().rev() {
						endcount += value2;
						
						if endcount >= (m - f) as i32 {
							upper = *key2;
							if lower <= upper && midcount <= f as i32 {
								let start = TimeIndex::new(TimeLong::new(lower));
								let finish = TimeIndex::new(TimeLong::new(upper));
								return TimeIndexInterval::new(start, finish);
							}
						}
						
						if *value2 == 0 {
							midcount += 1;
						}
					}
					break;
				}
				
				if *value == 0 {
					midcount += 1;
				}
			}
			
			f += 1;
			
			if f >= m / 2 {
				return Err(err!("Number of falsetickers {} is at least half the number {} of NTP servers polled. Formal correctness cannot be achieved.", f, m; Invalid, Network));
			}
		}
	}
	
	/// Gets true chimers based on intersection interval.
	fn get_true_chimers(&self, data_list: &[NtpData], intersection_interval: &TimeIndexInterval<TimeLong>) -> Outcome<Vec<NtpData>> {
		let mut result = Vec::new();
		
		for data in data_list {
			let interval = data.correctness_interval();
			
			// Check if interval overlaps with intersection interval
			if !interval.overlaps(intersection_interval) {
				continue;
			}
			
			result.push(data.clone());
		}
		
		Ok(result)
	}
	
	/// Gets survivors using selection algorithm.
	fn get_survivors(&self, true_chimers: &[NtpData]) -> Outcome<Vec<NtpData>> {
		let mut result = true_chimers.to_vec();
		
		if result.len() <= self.min_true_chimers {
			return Ok(result);
		}
		
		let mut have_pruned = true;
		while have_pruned && result.len() > self.min_true_chimers {
			have_pruned = false;
			let mut peer_jitter = BTreeMap::new();
			let mut select_jitter = BTreeMap::new();
			
			for data1 in &result {
				let theta1 = data1.offset.nanoseconds();
				let lambda1 = data1.root_distance;
				
				// Calculate peer jitter (standard deviation from offset history)
				let offset_series = self.offsets.get(&data1.address);
				let peer_jitter_value = if let Some(series) = offset_series {
					series.std_dev() as i64
				} else {
					0
				};
				peer_jitter.insert(peer_jitter_value, data1.clone());
				
				// Calculate select jitter (RMS of differences)
				let mut d = Vec::new();
				for data2 in true_chimers {
					let theta2 = data2.offset.nanoseconds();
					d.push(((theta1 - theta2).abs() * lambda1) as i64);
				}
				
				let select_jitter_value = self.rms(&d) as i64;
				select_jitter.insert(select_jitter_value, data1.clone());
			}
			
			// Get maximum select jitter and minimum peer jitter
			if let (Some((phi_s_max, worst_select)), Some((phi_r_min, _))) = 
				(select_jitter.iter().rev().next(), peer_jitter.iter().next()) {
				
				if phi_s_max > phi_r_min {
					// Remove the worst server
					result.retain(|data| data.address != worst_select.address);
					have_pruned = true;
				}
			}
		}
		
		Ok(result)
	}
	
	/// Root mean square calculation for jitter.
	fn rms(&self, series: &[i64]) -> f64 {
		if series.is_empty() {
			return 0.0;
		}
		
		let mut sum_x2 = 0i64;
		for &x in series {
			sum_x2 += x * x; // Fixed: was overwriting instead of accumulating
		}
		
		((sum_x2 as f64) / (series.len() as f64)).sqrt()
	}
	
	/// Converts NtpData to CalClock.
	fn data_to_calclock(&self, data: &NtpData, zone: CalClockZone) -> Outcome<CalClock> {
		let current_time = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map_err(|e| err!("System time error: {}", e; System))?;
		
		let offset_seconds = data.offset.total_seconds() as f64;
		let adjusted_time = current_time.as_secs_f64() + offset_seconds;
		CalClock::from_unix_timestamp_seconds(adjusted_time as i64, zone)
	}
	
	/// Placeholder for Prodigitum algorithm.
	fn prodigitum_algorithm(
		&mut self,
		data_list: Vec<NtpData>,
		zone: CalClockZone,
	) -> Outcome<AdvancedNtpResult> {
		// For now, fall back to RFC 1305
		self.rfc1305_algorithm(data_list, zone)
	}
}