#[derive(Clone, Debug, Eq, PartialEq, FromDatMap, ToDatMap)]
pub struct NetConfig {
    pub host:       String,
    pub timeout:    u64,
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            host:       "https://api.example.com".to_string(),
            timeout:    Duration::from_secs(30).as_secs(),
        }
    }
}

pub struct Plain {
    pub host: String,
}

impl Default for Plain {
    fn default() -> Self {
        Self { host: "https://plain.example.com".to_string() }
    }
}
