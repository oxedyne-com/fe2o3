/// Credit: https://stackoverflow.com/users/4498831/boiethios 
/// https://stackoverflow.com/questions/57685567/how-to-move-values-out-of-a-vector-when-the-vector-is-immediately-discarded
pub trait Extract: Default {
    fn extract(&mut self) -> Self;
}

impl<T: Default> Extract for T {
    fn extract(&mut self) -> Self {
        std::mem::replace(self, T::default())
    }
}

/// Gets current memory usage in MB.
/// 
/// Returns the current process memory usage (VmRSS) in megabytes.
/// On non-Linux systems or if unable to read memory stats, returns 0.0.
pub fn get_memory_usage_mb() -> f64 {
    // Simple approach: read from /proc/self/status on Linux.
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if line.starts_with("VmRSS:") {
                if let Some(kb_str) = line.split_whitespace().nth(1) {
                    if let Ok(kb) = kb_str.parse::<u64>() {
                        return kb as f64 / 1024.0; // Convert KB to MB.
                    }
                }
            }
        }
    }
    
    // Fallback: return 0 if we can't read memory usage.
    0.0
}
