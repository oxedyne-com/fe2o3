use oxedyne_fe2o3_core::prelude::*;

/// High-precision stopwatch.
#[derive(Clone, Debug)]
pub struct StopWatch {
	start_time: Option<std::time::Instant>,
}

impl StopWatch {
	/// Creates a new stopwatch.
	pub fn new() -> Self {
		Self { start_time: None }
	}
	
	/// Starts the stopwatch.
	pub fn start(&mut self) {
		self.start_time = Some(std::time::Instant::now());
	}
	
	/// Stops and returns elapsed nanoseconds.
	pub fn stop(&mut self) -> Outcome<u64> {
		if let Some(start) = self.start_time.take() {
			let elapsed = start.elapsed();
			Ok(elapsed.as_nanos() as u64)
		} else {
			Err(err!("Stopwatch not started"; Invalid))
		}
	}
}

/// Millisecond-precision stopwatch.
#[derive(Clone, Debug)]
pub struct StopWatchMillis {
	start_time: Option<std::time::Instant>,
}

impl StopWatchMillis {
	/// Creates a new millisecond stopwatch.
	pub fn new() -> Self {
		Self { start_time: None }
	}
	
	/// Starts the stopwatch.
	pub fn start(&mut self) {
		self.start_time = Some(std::time::Instant::now());
	}
	
	/// Stops and returns elapsed milliseconds.
	pub fn stop(&mut self) -> Outcome<u64> {
		if let Some(start) = self.start_time.take() {
			let elapsed = start.elapsed();
			Ok(elapsed.as_millis() as u64)
		} else {
			Err(err!("Stopwatch not started"; Invalid))
		}
	}
}