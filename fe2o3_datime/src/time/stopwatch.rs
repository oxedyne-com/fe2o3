//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

#[derive(Clone, Debug)]
pub struct StopWatch {
	start_time: Option<std::time::Instant>,
}

impl StopWatch {
	pub fn new() -> Self {
		Self { start_time: None }
	}
	
	pub fn start(&mut self) {
		self.start_time = Some(std::time::Instant::now());
	}
	
	pub fn stop(&mut self) -> Outcome<u64> {
		if let Some(start) = self.start_time.take() {
			let elapsed = start.elapsed();
			Ok(elapsed.as_nanos() as u64)
		} else {
			Err(err!("Stopwatch not started"; Invalid))
		}
	}
}

#[derive(Clone, Debug)]
pub struct StopWatchMillis {
	start_time: Option<std::time::Instant>,
}

impl StopWatchMillis {
	pub fn new() -> Self {
		Self { start_time: None }
	}
	
	pub fn start(&mut self) {
		self.start_time = Some(std::time::Instant::now());
	}
	
	pub fn stop(&mut self) -> Outcome<u64> {
		if let Some(start) = self.start_time.take() {
			let elapsed = start.elapsed();
			Ok(elapsed.as_millis() as u64)
		} else {
			Err(err!("Stopwatch not started"; Invalid))
		}
	}
	
	pub fn tic(&mut self) {
		self.start();
	}
	
	pub fn toc(&self) -> u64 {
		if let Some(start) = self.start_time {
			let elapsed = start.elapsed();
			elapsed.as_millis() as u64
		} else {
			0
		}
	}
}