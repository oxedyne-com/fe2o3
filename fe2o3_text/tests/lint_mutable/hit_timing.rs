use std::time::Duration;
const WAIT: u64 = 30;
const RATE_TIMEOUT_SECS: u64 = 9;
pub fn wait() {
    std::thread::sleep(Duration::from_secs(5 * 60));
    let d = Duration::new(30, 0);
    let w = Duration::from_secs(WAIT);
    tokio::time::sleep(WAIT);
    let r = Duration::from_secs(RATE_TIMEOUT_SECS);
    let z = Duration::new(0, 0);
}
