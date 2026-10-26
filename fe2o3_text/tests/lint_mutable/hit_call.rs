use std::time::Duration;
pub fn wait() {
    std::thread::sleep(Duration::from_millis(250));
    let d = Duration::from_secs(0);
    let n = 30 * 1000;
}
